//! kotone-platform-windows：Windows 平台适配器。
//!
//! - `audio`：cpal 生产音频采集（CpalBackend，实现 core 的 AudioBackend 端口）
//! - `inject`：raw windows crate 直调 Win32 SendInput（WindowsInjector / WinFocusBackend），
//!   非 Windows 编译时提供保持可编译的兜底实现（运行时明确报错）
//! - `elevation`：UIPI 提权检测与管理员重启（TokenElevation / ShellExecuteExW runas）
//! - `fullscreen`：独占全屏 Direct3D 状态探测（悬浮窗兼容性提醒）
//! - `fs_publish`：原子不覆盖的文件发布（CLI 导出产物用）
//! - `game_window`：游戏场景的窗口操作（切前台、请求关闭、打开程序）
//! - `window_tools`：工具服务的窗口观察与 GDI 前台可见客户区捕获（实现 core 的 tools 端口）
//! - `tool_host`：工具服务实例发现的会话/进程身份与私有文件 DACL
//! - `app_audio`：只录某一个程序的声音（进程回环录音，ADR 021）
//! - `voice_chat`：游戏语音（ADR 020）的 VB-CABLE 检测与安装、Windows 默认录音设备
//! - `voice_router`：游戏语音的转发与混音（真麦克风 + 琴音的语音 → CABLE Input）
//! - `hotkey_ll`：WH_KEYBOARD_LL + WH_MOUSE_LL 低级键鼠钩子热键源
//!   （实现 core 的 HotkeySource 端口）

pub mod app_audio;
pub mod game_icon;
pub mod audio;
pub mod autostart;
pub mod detached;
pub mod elevation;
pub mod fs_publish;
pub mod fullscreen;
pub mod game_window;
pub mod games;
#[cfg(windows)]
pub mod hotkey_ll;
pub mod inject;
pub mod keyboard;
pub mod playback;
pub mod speech;
pub mod tool_host;
pub mod voice_chat;
pub mod voice_router;
pub mod wav_audio;
pub mod window_tools;
