//! 通话（ADR 033）：按呼叫键或喊唤醒词 = 接通；通话里一直在听，玩家开口就是下一句，
//! 不用再按键、不用再喊唤醒词。
//!
//! 2026-11-26 修订：呼叫以后有三种用法（`assistant.callMode`，[`mode`]）：说一句就走（缺省）、录音笔、
//! 通话。只有选了通话，[`start`] 才接通；另外两种不接通，答完怎么办见 [`crate::follow_up`]。
//!
//! - 收音：唤醒那一路麦克风（[`crate::wake`]）在通话里改成“本机 silero 检测到开口，就把这句交给收音会话”，
//!   没开唤醒就为通话单独开这一路；识别、交给她的流程和呼叫键一样。
//! - 挂断：说“退下 / 安静 / 挂了 / 没事了 / 拜拜 / 再见”（[`crate::follow_up::is_exit_phrase`]，
//!   在 `agent::send` 里判）、通话中再按呼叫键、或 `assistant.callIdleSeconds` 秒没人说话。
//! - 空闲计时照 orb（dsh-orb-cordis，MIT）`packages/voice/src/idle.ts`：她在说、在想、在做、
//!   在等确认、玩家正在说时不计时。
//! - 只在内存里；Kotone 重启后不在通话。

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use kotone_core::assistant::CallMode;
use kotone_core::log;
use kotone_core::orchestrator::OrchestratorState;
use tauri::{AppHandle, Emitter, Manager};

use crate::SharedState;

static ACTIVE: AtomicBool = AtomicBool::new(false);
/// 每次接通 / 挂断加一：旧的空闲计时看到变了就退出。
static GENERATION: AtomicU64 = AtomicU64::new(0);
/// 最近一次“有动静”（谁在说、她在做）的时刻。
static LAST_ACTIVITY: Mutex<Option<Instant>> = Mutex::new(None);
/// 接通 / 挂断时发给界面（聊天页的呼叫按钮变成“挂断”）：`{active}`。
pub const EVENT: &str = "kotone://call";
/// 空闲计时多久看一次。
const TICK: Duration = Duration::from_millis(250);
/// 玩家开口、她压低以后最多等多久识别结果（再久就恢复音量）。
const DUCK_LIMIT: Duration = Duration::from_secs(6);
/// 收音会话已经结束、却一直没交给她：压低这么久以后恢复（会话刚开始的那几十毫秒还没进入“在听”）。
const DUCK_SETTLE: Duration = Duration::from_millis(1_500);

/// 正在通话吗。
pub fn active() -> bool {
    ACTIVE.load(Ordering::SeqCst)
}

/// 界面打开时问一下现在在不在通话。
#[tauri::command]
pub fn call_active() -> bool {
    active()
}

/// 呼叫以后怎么听（ADR 033 2026-11-26 修订）。0.1.x 旧提醒助手没有通话、也没有录音笔：按说一句就走。
pub fn mode(app: &AppHandle) -> CallMode {
    let state = app.state::<SharedState>();
    let settings = state.settings.read().unwrap();
    if settings.ui.legacy_upgrade.uses_legacy_assistant() {
        return CallMode::OneShot;
    }
    settings.assistant.effective_call_mode()
}

/// 设置里的空闲挂断秒数；0 = 不通话（一问一答）。0.1.x 旧提醒助手没有通话。
pub fn idle_seconds(app: &AppHandle) -> u32 {
    let settings = app.state::<SharedState>().settings.read().unwrap().clone();
    if settings.ui.legacy_upgrade.uses_legacy_assistant() {
        return 0;
    }
    settings.assistant.call_idle_seconds
}

/// 记一次动静（玩家开口、她开始说），空闲计时从现在重新算。
pub fn touch() {
    *LAST_ACTIVITY.lock().unwrap() = Some(Instant::now());
}

fn quiet_for() -> Duration {
    LAST_ACTIVITY
        .lock()
        .unwrap()
        .map(|at| at.elapsed())
        .unwrap_or_default()
}

/// 接通（设置里选的不是通话时什么也不做：说一句就走、录音笔不接通）。已经在通话就只记一次动静。
/// 返回是否新接通。
pub fn start(app: &AppHandle) -> bool {
    if mode(app) != CallMode::Call {
        return false;
    }
    // 语音识别没在运行（本地识别没启动）：听不了，不接通（呼叫键那一下会提示怎么启动）
    let recognizing = app
        .try_state::<crate::runtime::RuntimeManager>()
        .is_some_and(|rt| rt.phase() == kotone_core::runtime::RuntimePhase::Running);
    if !recognizing {
        return false;
    }
    touch();
    if ACTIVE.swap(true, Ordering::SeqCst) {
        return false;
    }
    let generation = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    log::log("call: connected");
    let _ = app.emit(EVENT, serde_json::json!({"active": true}));
    // 通话要那一路一直开着的麦克风（没开唤醒时现在才开）
    crate::wake::sync(app);
    let app = app.clone();
    tauri::async_runtime::spawn(async move { watch_idle(app, generation).await });
    true
}

/// 挂断。返回是否真的在通话。
pub fn end(app: &AppHandle, reason: &str) -> bool {
    if !ACTIVE.swap(false, Ordering::SeqCst) {
        return false;
    }
    GENERATION.fetch_add(1, Ordering::SeqCst);
    log::log(&format!("call: ended ({reason})"));
    let _ = app.emit(EVENT, serde_json::json!({"active": false}));
    crate::bubble::follow_up_done(app);
    // 没开唤醒时把那一路麦克风关掉
    crate::wake::sync(app);
    true
}

/// 通话中按呼叫键（ADR 033 D1）：挂断。她正在说就停下；玩家正说到一半就把这句收完交给她
/// （按键 = “说完了”），之后不再听。
pub async fn hang_up_by_key(app: &AppHandle) {
    end(app, "呼叫键");
    let orchestrator = app.state::<SharedState>().orchestrator.clone();
    if orchestrator.state() == OrchestratorState::Listening {
        let timeout = app
            .state::<SharedState>()
            .settings
            .read()
            .unwrap()
            .assistant
            .wake
            .no_speech_timeout_ms;
        orchestrator.call_assistant(None, timeout).await;
        return;
    }
    let app = app.clone();
    let _ = tauri::async_runtime::spawn_blocking(move || {
        if crate::assistant::interrupt_speech(&app) {
            log::log("call: hung up while she was speaking");
        }
    })
    .await;
}

/// 她念完一句回答（ADR 029 §五“接着听”在通话里的样子）：通话开着就在气泡上留着这句、显示倒计时。
pub fn after_reply(app: &AppHandle, shown: &str) {
    if !active() {
        return;
    }
    touch();
    crate::bubble::follow_up(app, u64::from(idle_seconds(app)), shown);
}

/// 玩家在她说话时开口、这句识别出来以后：不算打断的原因（None = 算打断，她停下）。
/// 照 xiaozhi / Pipecat（要识别出 2–3 个字才打断）和 orb `echo.ts`（和她刚说的话比，像就是回声）。
pub fn not_an_interruption(text: &str) -> Option<&'static str> {
    backchannel_or_empty(text).or_else(|| {
        looks_like_echo(text, &crate::voice_reply::recent_text()).then_some("是她自己的回声")
    })
}

fn core_chars(text: &str) -> Vec<char> {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// 空的、只是应一声（“嗯 / 对 / 哈哈”）、只有一个字（“停 / 不 / 别”除外）。纯函数。
fn backchannel_or_empty(text: &str) -> Option<&'static str> {
    const BACKCHANNEL: &str = "嗯恩哦噢喔啊呃额唔对好哈嘿呀诶欸是";
    let core = core_chars(text);
    if core.is_empty() {
        return Some("没识别出字");
    }
    if core.len() <= 3 && core.iter().all(|c| BACKCHANNEL.contains(*c)) {
        return Some("只是应了一声");
    }
    if core.len() == 1 && !matches!(core[0], '停' | '不' | '别') {
        return Some("只有一个字");
    }
    None
}

/// 和她最近说的话很像（回声没消干净）：在她的话里找一段，按顺序对得上的字占这句的六成以上。纯函数。
fn looks_like_echo(heard: &str, recent: &str) -> bool {
    let heard = core_chars(heard);
    let recent = core_chars(recent);
    if heard.len() < 2 || recent.is_empty() {
        return false;
    }
    let window = heard.len() + 2;
    let mut best = 0;
    for start in 0..recent.len() {
        let end = (start + window).min(recent.len());
        best = best.max(lcs(&heard, &recent[start..end]));
        if best == heard.len() {
            break;
        }
    }
    best * 10 >= heard.len() * 6
}

/// 最长公共子序列的长度。
fn lcs(a: &[char], b: &[char]) -> usize {
    let mut row = vec![0usize; b.len() + 1];
    for &x in a {
        let mut diagonal = 0;
        for (j, &y) in b.iter().enumerate() {
            let above = row[j + 1];
            row[j + 1] = if x == y {
                diagonal + 1
            } else {
                above.max(row[j])
            };
            diagonal = above;
        }
    }
    row[b.len()]
}

/// 现在算不算“有人在忙”（不计空闲）：她在说、在想、在做、在等确认，或玩家正在说。
fn busy(app: &AppHandle) -> bool {
    let session = !matches!(
        app.state::<SharedState>().orchestrator.state(),
        OrchestratorState::Idle | OrchestratorState::Success | OrchestratorState::Error
    );
    session
        || crate::agent::is_running(app)
        || crate::assistant::output_busy(app)
        || crate::voice_reply::active()
        || crate::voice_ask::is_pending(app)
        || crate::agent::has_spoken_asks(app)
}

async fn watch_idle(app: AppHandle, generation: u64) {
    loop {
        tokio::time::sleep(TICK).await;
        if GENERATION.load(Ordering::SeqCst) != generation || !active() {
            return;
        }
        // 设置里改成了说一句就走 / 录音笔（或旧写法的“不通话”）：挂断
        if mode(&app) != CallMode::Call {
            end(&app, "设置里不再是通话");
            return;
        }
        let idle = idle_seconds(&app);
        // 玩家在她说话时开口、她压低着：那句话没识别出字（收音会话结束了却没交给她）或太久没结果，
        // 就恢复音量（识别出来的那条路在 agent::send 里已经收尾了）。
        if let Some(ducked) = crate::voice_reply::ducked_for() {
            let session = !matches!(
                app.state::<SharedState>().orchestrator.state(),
                OrchestratorState::Idle | OrchestratorState::Success | OrchestratorState::Error
            );
            if ducked >= DUCK_LIMIT || (!session && ducked >= DUCK_SETTLE) {
                crate::voice_reply::unduck("没识别出话");
            }
        }
        if busy(&app) {
            touch();
            continue;
        }
        if quiet_for() >= Duration::from_secs(u64::from(idle)) {
            end(&app, "没人说话");
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backchannels_and_noise_do_not_interrupt_her() {
        for text in ["", "。", "嗯", "嗯嗯", "对对对", "哈哈", "哦", "好", "啊"] {
            assert!(backchannel_or_empty(text).is_some(), "{text:?}");
        }
        for text in [
            "停",
            "不",
            "别",
            "等等",
            "好了好了",
            "琴音",
            "对面打野来了",
            "是吗",
        ] {
            assert_eq!(backchannel_or_empty(text), None, "{text:?}");
        }
    }

    #[test]
    fn her_own_words_heard_back_are_echo() {
        let recent = "先出狂战士胫甲和无尽之刃，详细的我写在聊天里了。";
        // 回声：她说过的一段（识别可能错一两个字）
        assert!(looks_like_echo("无尽之刃", recent));
        assert!(looks_like_echo("先出狂战士近甲", recent));
        assert!(looks_like_echo("写在聊天里", recent));
        // 玩家自己的话
        assert!(!looks_like_echo("对面打野在下路", recent));
        assert!(!looks_like_echo("那出什么鞋", recent));
        assert!(!looks_like_echo("无尽之刃多少钱啊", recent));
        // 太短不判；她最近没说话不判
        assert!(!looks_like_echo("刃", recent));
        assert!(!looks_like_echo("无尽之刃", ""));
    }

    #[test]
    fn longest_common_subsequence() {
        let a: Vec<char> = "ABCBDAB".chars().collect();
        let b: Vec<char> = "BDCABA".chars().collect();
        assert_eq!(lcs(&a, &b), 4);
        assert_eq!(lcs(&a, &[]), 0);
    }
}
