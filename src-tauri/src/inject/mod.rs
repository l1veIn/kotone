//! 输入注入：Windows 直调 Win32 SendInput，时序 1:1 对齐 LeagueAkari
//! （docs/development.md §3.5、§5.1、§6）
//!
//! 合规红线：只做系统标准输入合成 + 剪贴板；不读写游戏内存、不 hook 渲染。
//!
//! 结构：
//! - 本模块：`Injector` trait、`CancelToken`、`InjectError` 与发送时序编排
//!   （`send_sequence` 面向可注入的 `SendOps` 编程，单测用 mock，不真睡、不真发键）；
//! - `windows.rs`（cfg(windows)）：三个 Win32 原语的真实实现 + `WindowsInjector`。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::profile::GameProfile;

/// 注入错误
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InjectError {
    pub message: String,
    /// true = 目标进程权限高于 Kotone（UIPI），提示前端引导管理员重启（§10 R-1）
    #[serde(default)]
    pub needs_elevation: bool,
}

impl InjectError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            needs_elevation: false,
        }
    }

    /// 标记「需要提权」的错误（目标游戏以管理员运行，UIPI 会丢弃合成输入）
    pub fn with_needs_elevation(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            needs_elevation: true,
        }
    }
}

impl std::fmt::Display for InjectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for InjectError {}

/// 发送取消令牌（docs/development.md §4：发送可取消，对齐 LeagueAkari AbortController）。
/// orchestrator 在取消会话时置位；发送时序的每个 sleep 前后检查。
#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// 发送编排接口：orchestrator 只面向此 trait 编程（STT 与 inject 完全解耦）。
/// 实现负责 §6 发送时序：前台校验 → openChatKey → 文本 → sendKey（含 delay 与取消检查）。
pub trait Injector: Send + Sync {
    fn send(
        &self,
        text: &str,
        profile: &GameProfile,
        cancel: CancelToken,
    ) -> Result<(), InjectError>;
}

/// 占位实现：什么都不做直接成功。非 Windows 平台兜底（Windows 上由 `WindowsInjector` 接管）。
pub struct StubInjector;

impl Injector for StubInjector {
    fn send(
        &self,
        text: &str,
        profile: &GameProfile,
        cancel: CancelToken,
    ) -> Result<(), InjectError> {
        let _ = (text, profile);
        if cancel.is_cancelled() {
            return Err(InjectError::new("发送已取消"));
        }
        Ok(())
    }
}

// ---------- 发送时序编排（§6；与平台无关，可单测） ----------

/// 时序原语抽象：真实实现发 Win32 事件；单测注入 mock（不真睡、不真发键）。
pub trait SendOps {
    /// 单键 / 组合键 down+up（如 "Enter"、"Ctrl+V"），实现内部保证按下的键一定补 up
    fn key_down_up(&self, key: &str) -> Result<(), InjectError>;
    /// Unicode 逐字输入（encode_utf16 逐单元 down+up）
    fn send_unicode(&self, text: &str) -> Result<(), InjectError>;
    /// 读取当前剪贴板文本（用于粘贴后恢复用户原内容；读不到返回 None）
    fn clipboard_read(&self) -> Option<String>;
    /// 写入剪贴板文本（CF_UNICODETEXT）
    fn clipboard_write(&self, text: &str) -> Result<(), InjectError>;
    /// 阻塞睡眠（毫秒）
    fn sleep_ms(&self, ms: u64);
}

/// 剪贴板粘贴后、恢复原内容前的等待：给目标应用消息循环处理粘贴的时间
const CLIPBOARD_RESTORE_DELAY_MS: u32 = 100;

fn ensure_not_cancelled(cancel: &CancelToken) -> Result<(), InjectError> {
    if cancel.is_cancelled() {
        Err(InjectError::new("发送已取消"))
    } else {
        Ok(())
    }
}

/// sleep 前后各检查一次取消令牌（§6：取消则安全中止）。
/// 注意：本函数被调用时不可能有「按下一半」的键——key_down_up 是原子 down/up。
fn cancellable_sleep(
    ops: &dyn SendOps,
    ms: u32,
    cancel: &CancelToken,
) -> Result<(), InjectError> {
    ensure_not_cancelled(cancel)?;
    if ms > 0 {
        ops.sleep_ms(u64::from(ms));
    }
    ensure_not_cancelled(cancel)
}

/// §6 发送时序（前台校验由各 `Injector` 实现在调用前完成）：
/// key_down_up(openChatKey) → sleep(preOpenDelayMs)
/// → preferClipboardPaste ? 剪贴板+Ctrl+V : send_unicode
/// → sleep(preSendDelayMs) → key_down_up(sendKey)。
/// 每个 sleep 前后检查取消令牌；取消则安全中止（无悬键，剪贴板尽量恢复）。
pub fn send_sequence(
    text: &str,
    profile: &GameProfile,
    cancel: &CancelToken,
    ops: &dyn SendOps,
) -> Result<(), InjectError> {
    ensure_not_cancelled(cancel)?;
    ops.key_down_up(&profile.open_chat_key)?;
    cancellable_sleep(ops, profile.pre_open_delay_ms, cancel)?;
    if profile.prefer_clipboard_paste {
        send_clipboard_path(text, profile, cancel, ops)?;
    } else {
        ops.send_unicode(text)?;
    }
    cancellable_sleep(ops, profile.pre_send_delay_ms, cancel)?;
    ops.key_down_up(&profile.send_key)?;
    Ok(())
}

/// 剪贴板备选路径：读旧值 → 写入 → sleep(prePasteDelayMs) → Ctrl+V
/// → 延迟后恢复用户原剪贴板内容。任何一步取消/失败都尽量恢复原内容。
fn send_clipboard_path(
    text: &str,
    profile: &GameProfile,
    cancel: &CancelToken,
    ops: &dyn SendOps,
) -> Result<(), InjectError> {
    let backup = ops.clipboard_read();
    let restore = |ops: &dyn SendOps| {
        if let Some(old) = backup.as_deref() {
            let _ = ops.clipboard_write(old);
        }
    };
    if let Err(e) = ops.clipboard_write(text) {
        restore(ops);
        return Err(e);
    }
    if let Err(e) = cancellable_sleep(ops, profile.pre_paste_delay_ms, cancel) {
        restore(ops);
        return Err(e);
    }
    if let Err(e) = ops.key_down_up("Ctrl+V") {
        restore(ops);
        return Err(e);
    }
    // 给目标应用处理粘贴留出时间；即使期间被取消也要先恢复剪贴板
    let _ = cancellable_sleep(ops, CLIPBOARD_RESTORE_DELAY_MS, cancel);
    restore(ops);
    Ok(())
}

// ---------- 平台实现 ----------

#[cfg(windows)]
#[path = "windows.rs"]
mod windows_impl;

#[cfg(windows)]
pub use windows_impl::{
    find_pid_by_name, foreground_pid, foreground_process_name, is_process_foreground,
    key_down_up, process_name_from_pid, send_unicode, WindowsInjector,
};

#[cfg(not(windows))]
pub use self::fallback::*;

/// 非 Windows 兜底：保持可编译，运行时返回明确错误（macOS/Linux 注入见 §3.7 平台策略）
#[cfg(not(windows))]
mod fallback {
    use super::{CancelToken, InjectError, Injector};
    use crate::profile::GameProfile;

    pub fn send_unicode(_text: &str) -> Result<(), InjectError> {
        Err(InjectError::new(
            "send_unicode 仅 Windows 实现（MVP Windows-first）",
        ))
    }

    pub fn key_down_up(_key: &str) -> Result<(), InjectError> {
        Err(InjectError::new(
            "key_down_up 仅 Windows 实现（MVP Windows-first）",
        ))
    }

    pub fn is_process_foreground(process_names: &[String]) -> bool {
        // 空列表 = 通配任意前台进程；非 Windows 无法校验，保守按通配处理
        process_names.is_empty()
    }

    pub fn foreground_process_name() -> Option<String> {
        None
    }

    pub fn foreground_pid() -> Option<u32> {
        None
    }

    pub fn process_name_from_pid(_pid: u32) -> Option<String> {
        None
    }

    pub fn find_pid_by_name(_name: &str) -> Option<u32> {
        None
    }

    pub struct WindowsInjector;

    impl Injector for WindowsInjector {
        fn send(
            &self,
            _text: &str,
            _profile: &GameProfile,
            cancel: CancelToken,
        ) -> Result<(), InjectError> {
            if cancel.is_cancelled() {
                return Err(InjectError::new("发送已取消"));
            }
            Err(InjectError::new(
                "当前平台暂不支持输入注入（MVP Windows-first）",
            ))
        }
    }
}

// ---------- 时序编排单测（mock ops：不真睡、不真发键） ----------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// 记录全部操作顺序的 mock；可配置「在第 N 次 sleep 时触发取消」
    struct MockOps {
        log: Mutex<Vec<String>>,
        clipboard: Mutex<Option<String>>,
        cancel: CancelToken,
        /// 第几次 sleep_ms 调用时置位取消令牌（1-based）；None = 不触发
        cancel_on_sleep: Mutex<Option<usize>>,
        sleep_calls: Mutex<usize>,
    }

    impl MockOps {
        fn new() -> Self {
            Self {
                log: Mutex::new(Vec::new()),
                clipboard: Mutex::new(None),
                cancel: CancelToken::default(),
                cancel_on_sleep: Mutex::new(None),
                sleep_calls: Mutex::new(0),
            }
        }
        fn cancel_on(self, nth_sleep: usize) -> Self {
            *self.cancel_on_sleep.lock().unwrap() = Some(nth_sleep);
            self
        }
        fn log(&self) -> Vec<String> {
            self.log.lock().unwrap().clone()
        }
    }

    impl SendOps for MockOps {
        fn key_down_up(&self, key: &str) -> Result<(), InjectError> {
            self.log.lock().unwrap().push(format!("key:{key}"));
            Ok(())
        }
        fn send_unicode(&self, text: &str) -> Result<(), InjectError> {
            self.log.lock().unwrap().push(format!("unicode:{text}"));
            Ok(())
        }
        fn clipboard_read(&self) -> Option<String> {
            self.clipboard.lock().unwrap().clone()
        }
        fn clipboard_write(&self, text: &str) -> Result<(), InjectError> {
            *self.clipboard.lock().unwrap() = Some(text.to_string());
            self.log.lock().unwrap().push(format!("clip_write:{text}"));
            Ok(())
        }
        fn sleep_ms(&self, ms: u64) {
            let mut calls = self.sleep_calls.lock().unwrap();
            *calls += 1;
            let nth = *calls;
            drop(calls);
            self.log.lock().unwrap().push(format!("sleep:{ms}"));
            if *self.cancel_on_sleep.lock().unwrap() == Some(nth) {
                self.cancel.cancel();
            }
        }
    }

    fn test_profile() -> GameProfile {
        GameProfile::builtin_generic()
    }

    #[test]
    fn unicode_path_order_and_delays() {
        let ops = MockOps::new();
        let cancel = CancelToken::default();
        send_sequence("对面打野在下路", &test_profile(), &cancel, &ops).unwrap();
        assert_eq!(
            ops.log(),
            vec![
                "key:Enter",
                "sleep:20",
                "unicode:对面打野在下路",
                "sleep:20",
                "key:Enter",
            ]
        );
    }

    #[test]
    fn custom_delays_are_respected() {
        let ops = MockOps::new();
        let cancel = CancelToken::default();
        let mut profile = test_profile();
        profile.pre_open_delay_ms = 55;
        profile.pre_send_delay_ms = 77;
        send_sequence("hi", &profile, &cancel, &ops).unwrap();
        assert_eq!(
            ops.log(),
            vec!["key:Enter", "sleep:55", "unicode:hi", "sleep:77", "key:Enter"]
        );
    }

    #[test]
    fn clipboard_path_writes_pastes_and_restores() {
        let ops = MockOps::new();
        *ops.clipboard.lock().unwrap() = Some("用户原文".into());
        let cancel = CancelToken::default();
        let mut profile = test_profile();
        profile.prefer_clipboard_paste = true;
        send_sequence("粘贴内容", &profile, &cancel, &ops).unwrap();
        assert_eq!(
            ops.log(),
            vec![
                "key:Enter",
                "sleep:20",
                "clip_write:粘贴内容",
                "sleep:20", // prePasteDelayMs
                "key:Ctrl+V",
                "sleep:100", // 粘贴处理后恢复前的等待
                "clip_write:用户原文",
                "sleep:20",
                "key:Enter",
            ]
        );
        assert_eq!(
            ops.clipboard.lock().unwrap().as_deref(),
            Some("用户原文"),
            "发送后应恢复用户原剪贴板内容"
        );
    }

    #[test]
    fn cancel_before_start_aborts_immediately() {
        let ops = MockOps::new();
        ops.cancel.cancel();
        let cancel = ops.cancel.clone();
        let err = send_sequence("hi", &test_profile(), &cancel, &ops).unwrap_err();
        assert!(err.message.contains("取消"));
        assert!(ops.log().is_empty(), "取消后不应发出任何按键");
    }

    #[test]
    fn cancel_during_pre_open_sleep_stops_before_typing() {
        let ops = MockOps::new().cancel_on(1); // preOpenDelay 期间取消
        let cancel = ops.cancel.clone();
        let err = send_sequence("hi", &test_profile(), &cancel, &ops).unwrap_err();
        assert!(err.message.contains("取消"));
        // 开聊键已完整 down+up（原子，无悬键），但文本未发出
        assert_eq!(ops.log(), vec!["key:Enter", "sleep:20"]);
    }

    #[test]
    fn cancel_during_pre_send_sleep_stops_before_send_key() {
        let ops = MockOps::new().cancel_on(2); // preSendDelay 期间取消
        let cancel = ops.cancel.clone();
        let err = send_sequence("hi", &test_profile(), &cancel, &ops).unwrap_err();
        assert!(err.message.contains("取消"));
        // 文本已上屏但发送键未按 → 游戏聊天框里留着草稿，用户可手动处理
        assert_eq!(
            ops.log(),
            vec!["key:Enter", "sleep:20", "unicode:hi", "sleep:20"]
        );
    }

    #[test]
    fn cancel_during_clipboard_paste_still_restores_clipboard() {
        let ops = MockOps::new().cancel_on(2); // prePasteDelay 期间取消
        *ops.clipboard.lock().unwrap() = Some("用户原文".into());
        let cancel = ops.cancel.clone();
        let mut profile = test_profile();
        profile.prefer_clipboard_paste = true;
        let err = send_sequence("粘贴内容", &profile, &cancel, &ops).unwrap_err();
        assert!(err.message.contains("取消"));
        // Ctrl+V 未发出，且剪贴板已恢复
        assert_eq!(
            ops.log(),
            vec![
                "key:Enter",
                "sleep:20",
                "clip_write:粘贴内容",
                "sleep:20",
                "clip_write:用户原文",
            ]
        );
        assert_eq!(ops.clipboard.lock().unwrap().as_deref(), Some("用户原文"));
    }

    #[test]
    fn zero_delays_still_check_cancel_but_skip_sleep() {
        let ops = MockOps::new();
        let cancel = CancelToken::default();
        let mut profile = test_profile();
        profile.pre_open_delay_ms = 0;
        profile.pre_send_delay_ms = 0;
        send_sequence("hi", &profile, &cancel, &ops).unwrap();
        assert_eq!(ops.log(), vec!["key:Enter", "unicode:hi", "key:Enter"]);
    }
}
