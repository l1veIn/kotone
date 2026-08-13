//! kotone-platform-windows：Windows 平台适配器。
//!
//! - `audio`：cpal 生产音频采集（CpalBackend，实现 core 的 AudioBackend 端口）
//! - `inject`：raw windows crate 直调 Win32 SendInput（WindowsInjector / WinFocusBackend），
//!   非 Windows 编译时提供保持可编译的兜底实现（运行时明确报错）
//! - `elevation`：UIPI 提权检测与管理员重启（TokenElevation / ShellExecuteExW runas）
//! - `fullscreen`：独占全屏 Direct3D 状态探测（悬浮窗兼容性提醒）
//! - `hotkey_ll`：WH_KEYBOARD_LL + WH_MOUSE_LL 低级键鼠钩子热键源
//!   （实现 core 的 HotkeySource 端口）

pub mod audio;
pub mod elevation;
pub mod fullscreen;
#[cfg(windows)]
pub mod hotkey_ll;
pub mod inject;
pub mod playback;
pub mod wav_audio;
