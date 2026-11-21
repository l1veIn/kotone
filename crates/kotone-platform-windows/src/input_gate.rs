//! 发送窗口内挡住玩家的按下（ADR 032 决定 1）。
//!
//! 往游戏聊天里发一条消息要分好几次 `SendInput`（开聊天框键 → 文字 → 发送键），
//! 两次之间聊天框开着，玩家这时按的键会进聊天框：进输入法组字、混进消息、变成 Shift+Enter。
//! `WindowsInjector::send` 发第一个键前打开窗口，最后一个键松开 30ms 后关上；窗口内：
//!
//! - 玩家的**按下**被低层键盘钩子吞掉，**不补发**（玩家确认：游戏是实时的，晚到的键可能是错的）；
//! - 松开一律放行，鼠标不挡，Kotone 自己的键（`LLKHF_INJECTED` + `KOTONE_INPUT_TAG`）放行；
//! - 从打开起超过 600ms 钩子自己停止吞键（只比时间戳，不依赖发送线程）；
//! - 钩子没运行、或从没见过带 Kotone 标记的键时不打开；打开后某次发送里 Kotone 发出了键、
//!   钩子却一个都没认出来（输入过滤软件清掉了注入标记），停用到下次启动，免得吞掉自己的键。
//!
//! 钩子回调只调 [`Gate::on_key`]：只读写原子变量，不等锁、不写日志、不做 IO。
//! 判定写成不碰 Win32 的方法（时间由调用方传入），单测直接构造一个 [`Gate`]。

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// 从打开起超过这么久，钩子不再吞键（玩家把延时调得很长时，超出的部分不受保护）。
pub const HARD_LIMIT_MS: u64 = 600;
/// 最后一个键松开后窗口再留这么久，让游戏处理完最后的 Enter。
pub const TAIL_MS: u64 = 30;

/// 窗口状态。生产用全局的 [`GATE`]；单测各自构造一个。
pub struct Gate {
    /// 当前窗口打开的时刻（毫秒，见 [`now_ms`]，从 1 起算）；0 = 没打开。
    open_since_ms: AtomicU64,
    /// 同时打开的窗口数（并发发送时，最后一个关上才算关）。
    depth: AtomicU32,
    /// 钩子认出的 Kotone 自己的按键事件累计数（窗口内外都数）。
    own_seen: AtomicU64,
    /// Kotone 交给 `SendInput` 并被系统接受的键盘事件累计数。
    own_submitted: AtomicU64,
    /// 窗口内吞掉的按下累计数。
    swallowed: AtomicU64,
    /// 认不出自己的键后停用，到下次启动前不再打开。
    disabled: AtomicBool,
}

/// 全局窗口状态：钩子回调和发送线程共用。
pub static GATE: Gate = Gate::new();

/// 一次 [`Gate::begin`] 的快照，交回 [`Gate::end`] 结算。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Session {
    /// 这次窗口有没有真的打开（没打开时钩子什么都不吞）。
    pub armed: bool,
    started_ms: u64,
    own_seen: u64,
    own_submitted: u64,
    swallowed: u64,
}

/// [`Gate::end`] 的结算结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Report {
    pub armed: bool,
    /// 窗口从打开到关上的毫秒数。
    pub window_ms: u64,
    /// 窗口期间吞掉的按下个数。
    pub swallowed: u64,
    /// 这次结算把挡键停用了（认不出自己的键）。
    pub disabled_now: bool,
}

impl Gate {
    pub const fn new() -> Self {
        Self {
            open_since_ms: AtomicU64::new(0),
            depth: AtomicU32::new(0),
            own_seen: AtomicU64::new(0),
            own_submitted: AtomicU64::new(0),
            swallowed: AtomicU64::new(0),
            disabled: AtomicBool::new(false),
        }
    }

    /// 钩子回调里的判定（键盘事件，已经过 Esc 停止、探测、热键匹配且都没吞）。
    /// `down` = 按下（含系统自动重复的按下），`ours` = 带 Kotone 标记的注入键。
    /// 返回 true = 吞掉。只读写原子变量。
    pub fn on_key(&self, down: bool, ours: bool, now_ms: u64) -> bool {
        if ours {
            self.own_seen.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        if !down {
            return false;
        }
        let since = self.open_since_ms.load(Ordering::SeqCst);
        if since == 0 || now_ms.saturating_sub(since) > HARD_LIMIT_MS {
            return false;
        }
        self.swallowed.fetch_add(1, Ordering::Relaxed);
        true
    }

    /// Kotone 的 `SendInput` 被系统接受了 `count` 个键盘事件（发送线程调用）。
    pub fn note_submitted(&self, count: u64) {
        self.own_submitted.fetch_add(count, Ordering::Relaxed);
    }

    /// 发第一个键之前调用。`hooks_running` = 低层键盘钩子在运行。
    pub fn begin(&self, hooks_running: bool, now_ms: u64) -> Session {
        let armed = hooks_running
            && !self.disabled.load(Ordering::SeqCst)
            && self.own_seen.load(Ordering::Relaxed) > 0;
        let session = Session {
            armed,
            started_ms: now_ms,
            own_seen: self.own_seen.load(Ordering::Relaxed),
            own_submitted: self.own_submitted.load(Ordering::Relaxed),
            swallowed: self.swallowed.load(Ordering::Relaxed),
        };
        if armed {
            self.depth.fetch_add(1, Ordering::SeqCst);
            self.open_since_ms.store(now_ms.max(1), Ordering::SeqCst);
        }
        session
    }

    /// 窗口关上（最后一个键松开、尾巴等完之后）。任何出口都要调用，见 [`SendWindow`]。
    pub fn end(&self, session: Session, now_ms: u64) -> Report {
        if session.armed && self.depth.fetch_sub(1, Ordering::SeqCst) == 1 {
            self.open_since_ms.store(0, Ordering::SeqCst);
        }
        let submitted = self.own_submitted.load(Ordering::Relaxed) - session.own_submitted;
        let seen = self.own_seen.load(Ordering::Relaxed) - session.own_seen;
        let disabled_now = session.armed
            && submitted > 0
            && seen == 0
            && !self.disabled.swap(true, Ordering::SeqCst);
        Report {
            armed: session.armed,
            window_ms: now_ms.saturating_sub(session.started_ms),
            swallowed: self.swallowed.load(Ordering::Relaxed) - session.swallowed,
            disabled_now,
        }
    }

    /// 这次发送到现在 Kotone 交出过键盘事件（没交出就不用等尾巴）。
    fn submitted_since(&self, session: &Session) -> bool {
        self.own_submitted.load(Ordering::Relaxed) > session.own_submitted
    }
}

impl Default for Gate {
    fn default() -> Self {
        Self::new()
    }
}

static CLOCK_BASE: OnceLock<Instant> = OnceLock::new();

/// 单调毫秒时钟（进程内第一次调用时起算，从 1 开始）。发送线程用。
pub fn now_ms() -> u64 {
    CLOCK_BASE.get_or_init(Instant::now).elapsed().as_millis() as u64 + 1
}

/// 钩子回调用的时钟：不初始化（`get_or_init` 在并发初始化时会等），还没起算时返回 0。
/// 窗口打开前 [`SendWindow::open`] 已经起算过，所以窗口开着时这里总有值。
pub fn hook_now_ms() -> u64 {
    CLOCK_BASE
        .get()
        .map_or(0, |base| base.elapsed().as_millis() as u64 + 1)
}

/// 发送窗口的 RAII 守卫：创建时打开，drop 时（成功、失败、取消都会走到）
/// 等完尾巴再关上，吞过键就记一条 `inject_gate`。
pub struct SendWindow {
    session: Session,
}

impl SendWindow {
    pub fn open() -> Self {
        Self {
            session: GATE.begin(crate::hotkey_ll::hooks_running(), now_ms()),
        }
    }
}

impl Drop for SendWindow {
    fn drop(&mut self) {
        if self.session.armed && GATE.submitted_since(&self.session) {
            std::thread::sleep(Duration::from_millis(TAIL_MS));
        }
        let report = GATE.end(self.session, now_ms());
        if report.disabled_now {
            kotone_core::log::log(
                "inject_gate: no Kotone-tagged key was seen by the hook during a send \
                 (an input filter may strip the tag); key gating disabled until restart",
            );
        }
        if report.swallowed > 0 {
            kotone_core::process_log::record_activity(
                "inject_gate",
                kotone_core::process_log::EventData {
                    detail: Some(format!(
                        "swallowed={} windowMs={}",
                        report.swallowed, report.window_ms
                    )),
                    outcome: Some("ok".into()),
                    ..Default::default()
                },
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 见过一次 Kotone 自己的键、钩子在跑：后续发送会打开窗口。
    fn primed() -> Gate {
        let gate = Gate::new();
        assert!(!gate.on_key(true, true, 1));
        gate
    }

    #[test]
    fn player_downs_inside_the_window_are_swallowed_and_ups_pass() {
        let gate = primed();
        let s = gate.begin(true, 100);
        assert!(s.armed);
        assert!(gate.on_key(true, false, 120), "letter down is swallowed");
        assert!(
            gate.on_key(true, false, 130),
            "auto-repeat down is swallowed too"
        );
        assert!(!gate.on_key(false, false, 140), "up passes");
        gate.note_submitted(4);
        assert!(!gate.on_key(true, true, 150));
        let r = gate.end(s, 400);
        assert_eq!(r.swallowed, 2);
        assert_eq!(r.window_ms, 300);
        assert!(!r.disabled_now);
        assert!(
            !gate.on_key(true, false, 410),
            "closed window passes everything"
        );
    }

    #[test]
    fn own_keys_pass_and_other_programs_injected_keys_are_the_players() {
        // 调用方把“带 LLKHF_INJECTED 但不是 Kotone 标记”的键当成 ours = false 传进来
        let gate = primed();
        let s = gate.begin(true, 10);
        assert!(!gate.on_key(true, true, 20), "Kotone's own down passes");
        assert!(!gate.on_key(false, true, 30), "Kotone's own up passes");
        assert!(
            gate.on_key(true, false, 40),
            "a macro tool's injected down is swallowed"
        );
        gate.note_submitted(2);
        gate.end(s, 50);
    }

    #[test]
    fn modifier_downs_are_swallowed_like_any_other_key() {
        // 判定不看键码：Shift / Ctrl / Alt / Win 的按下同样被吞、不补发，松开放行
        let gate = primed();
        let s = gate.begin(true, 0);
        assert!(gate.on_key(true, false, 5)); // Shift down
        assert!(!gate.on_key(false, false, 300)); // Shift up after the window would also pass
        gate.note_submitted(1);
        gate.on_key(true, true, 6);
        assert_eq!(gate.end(s, 10).swallowed, 1);
    }

    #[test]
    fn hard_limit_stops_swallowing_after_600ms() {
        let gate = primed();
        let s = gate.begin(true, 1_000);
        assert!(gate.on_key(true, false, 1_000 + HARD_LIMIT_MS));
        assert!(!gate.on_key(true, false, 1_000 + HARD_LIMIT_MS + 1));
        gate.note_submitted(1);
        gate.on_key(true, true, 1_700);
        assert_eq!(gate.end(s, 1_800).swallowed, 1);
    }

    #[test]
    fn not_armed_until_the_hook_has_seen_a_kotone_key() {
        let gate = Gate::new();
        let s = gate.begin(true, 10);
        assert!(!s.armed, "first message after start is not protected");
        assert!(!gate.on_key(true, false, 20));
        gate.note_submitted(2);
        assert!(!gate.on_key(true, true, 30)); // the hook sees this send's keys
        let r = gate.end(s, 40);
        assert!(!r.armed && !r.disabled_now);
        assert!(gate.begin(true, 50).armed, "the next send is protected");
    }

    #[test]
    fn not_armed_when_the_hook_is_not_running() {
        let gate = primed();
        let s = gate.begin(false, 10);
        assert!(!s.armed);
        assert!(!gate.on_key(true, false, 20));
        gate.note_submitted(2);
        assert!(!gate.end(s, 30).disabled_now);
    }

    #[test]
    fn a_send_whose_own_keys_were_not_recognised_disables_the_gate() {
        let gate = primed();
        let s = gate.begin(true, 10);
        gate.note_submitted(4); // Kotone sent keys, but the hook saw none with the tag
        assert!(gate.on_key(true, false, 20)); // ...so this may have been Kotone's own key
        let r = gate.end(s, 300);
        assert!(r.disabled_now);
        assert!(!gate.begin(true, 400).armed, "stays off until restart");
        let again = gate.end(gate.begin(true, 500), 600);
        assert!(!again.disabled_now, "reported once");
    }

    #[test]
    fn a_send_that_submitted_nothing_does_not_disable_the_gate() {
        // 例如前台是管理员程序，SendInput 前就报错：没发出键，谈不上“没认出来”
        let gate = primed();
        let s = gate.begin(true, 10);
        let r = gate.end(s, 20);
        assert!(r.armed && !r.disabled_now);
        assert!(gate.begin(true, 30).armed);
    }

    #[test]
    fn overlapping_windows_close_with_the_last_one() {
        let gate = primed();
        let a = gate.begin(true, 10);
        let b = gate.begin(true, 20);
        gate.note_submitted(2);
        gate.on_key(true, true, 25);
        gate.end(a, 30);
        assert!(gate.on_key(true, false, 40), "b is still open");
        gate.end(b, 50);
        assert!(!gate.on_key(true, false, 60));
    }

    #[test]
    fn clock_is_monotonic_and_never_zero() {
        let a = now_ms();
        let b = hook_now_ms();
        assert!(a >= 1 && b >= a);
    }
}
