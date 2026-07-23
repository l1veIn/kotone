//! 输入注入：Windows 直调 Win32 SendInput，时序 1:1 对齐 LeagueAkari
//! （docs/development.md §3.5、§5.1、§6）
//!
//! 合规红线：只做系统标准输入合成 + 剪贴板；不读写游戏内存、不 hook 渲染。

/// 注入错误
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct InjectError {
    pub message: String,
}

impl std::fmt::Display for InjectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for InjectError {}

/// Unicode 逐字输入（encode_utf16 逐单元 KEYEVENTF_UNICODE down+up）
pub fn send_unicode(_text: &str) -> Result<(), InjectError> {
    todo!("raw windows crate 直调 SendInput")
}

/// 单键 down+up（VK + MapVirtualKey scan code）
pub fn key_down_up(_key: &str) -> Result<(), InjectError> {
    todo!("Enter / Ctrl+V 等按键合成")
}

/// 发送前硬性校验：目标游戏进程必须为前台进程，否则 abort
pub fn is_process_foreground(_process_names: &[String]) -> bool {
    todo!("GetForegroundWindow + GetWindowThreadProcessId 匹配")
}
