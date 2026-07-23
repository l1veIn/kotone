//! 输入注入：Windows 直调 Win32 SendInput，时序 1:1 对齐 LeagueAkari
//! （docs/development.md §3.5、§5.1、§6）
//!
//! 合规红线：只做系统标准输入合成 + 剪贴板；不读写游戏内存、不 hook 渲染。
//!
//! 当前状态：`Injector` trait 与编排接口已定型（orchestrator 面向它编程），
//! 生产实现 `StubInjector` 仅返回 Ok(()) —— **真实 SendInput 实现由 inject 子代理完成**，
//! 实现时把 `StubInjector` 换成 `WindowsInjector`（下方三个原语 + §6 时序）即可，
//! orchestrator / IPC 不需要任何改动。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::profile::GameProfile;

/// 注入错误
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct InjectError {
    pub message: String,
}

impl InjectError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
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
/// orchestrator 在取消会话时置位；inject 实现在发送时序的每个 sleep 前后检查。
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

/// 占位实现：什么都不做直接成功。
/// TODO(inject 子代理)：替换为 WindowsInjector——
///   1. is_process_foreground(profile.process_names) 硬校验，false → InjectError「游戏不在前台」
///   2. key_down_up(open_chat_key) → sleep(pre_open_delay_ms)
///   3. prefer_clipboard_paste ? 剪贴板 + Ctrl+V : send_unicode(text)
///   4. sleep(pre_send_delay_ms) → key_down_up(send_key)
///   5. 每个 sleep 前后检查 cancel.is_cancelled()，取消则安全中止
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

// ---------- 三个 Win32 原语（inject 子代理实现，签名已定） ----------

/// Unicode 逐字输入（encode_utf16 逐单元 KEYEVENTF_UNICODE down+up）
#[allow(dead_code)] // 签名契约，由 inject 子代理实现并接入 WindowsInjector
pub fn send_unicode(_text: &str) -> Result<(), InjectError> {
    Err(InjectError::new(
        "send_unicode 未实现：等待 inject 子代理接入 SendInput",
    ))
}

/// 单键 down+up（VK + MapVirtualKey scan code）
#[allow(dead_code)]
pub fn key_down_up(_key: &str) -> Result<(), InjectError> {
    Err(InjectError::new(
        "key_down_up 未实现：等待 inject 子代理接入 SendInput",
    ))
}

/// 发送前硬性校验：目标游戏进程必须为前台进程，否则 abort
#[allow(dead_code)]
pub fn is_process_foreground(_process_names: &[String]) -> bool {
    // TODO(inject 子代理)：GetForegroundWindow + GetWindowThreadProcessId 匹配
    false
}
