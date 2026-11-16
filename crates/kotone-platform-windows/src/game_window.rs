//! 游戏场景的窗口操作（ADR 014 第 5 步）：切到前台、请求关闭、打开程序。
//!
//! - 切前台：最小化的先还原；`SetForegroundWindow` 受前台锁限制时先轻按一下 Alt
//!   （系统允许最近有输入的进程切换前台），再不行就把输入队列附加到前台线程后重试；
//!   最后核对前台确实是目标窗口。
//! - 关闭：只投递 `WM_CLOSE`（游戏自己决定是否弹确认、怎么存档），从不强制结束进程。
//! - 打开：`ShellExecuteW("open")`，支持程序、快捷方式和启动器链接（如 `steam://`）；
//!   路径只来自用户在游戏配置里填写的“启动路径”。
//!
//! 非 Windows 平台编译为明确报错的兜底实现。

#[cfg(windows)]
mod imp {
    use std::time::Duration;

    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        keybd_event, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, VK_MENU,
    };
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindowThreadProcessId, IsIconic, IsWindow, PostMessageW,
        SetForegroundWindow, ShowWindow, SW_RESTORE, SW_SHOWNORMAL, WM_CLOSE,
    };

    fn hwnd(handle: u64) -> HWND {
        HWND(handle as usize as *mut std::ffi::c_void)
    }

    fn is_foreground(target: HWND) -> bool {
        unsafe { GetForegroundWindow() == target }
    }

    pub fn focus_window(handle: u64) -> Result<(), String> {
        let target = hwnd(handle);
        unsafe {
            if !IsWindow(Some(target)).as_bool() {
                return Err("窗口已经关闭".into());
            }
            if IsIconic(target).as_bool() {
                let _ = ShowWindow(target, SW_RESTORE);
            }
            if SetForegroundWindow(target).as_bool() && is_foreground(target) {
                return Ok(());
            }
            // 前台锁：轻按一下 Alt，让本进程算“最近有输入”，再试。
            keybd_event(VK_MENU.0 as u8, 0, KEYBD_EVENT_FLAGS(0), 0);
            keybd_event(VK_MENU.0 as u8, 0, KEYEVENTF_KEYUP, 0);
            if SetForegroundWindow(target).as_bool() && is_foreground(target) {
                return Ok(());
            }
            // 仍不行：附加到前台线程与目标线程的输入队列后重试。
            let current = GetCurrentThreadId();
            let fg = GetForegroundWindow();
            let fg_tid = if fg.0.is_null() {
                0
            } else {
                GetWindowThreadProcessId(fg, None)
            };
            let target_tid = GetWindowThreadProcessId(target, None);
            let attached_fg = fg_tid != 0
                && fg_tid != current
                && AttachThreadInput(current, fg_tid, true).as_bool();
            let attached_target = target_tid != 0
                && target_tid != current
                && AttachThreadInput(current, target_tid, true).as_bool();
            let _ = SetForegroundWindow(target);
            if attached_fg {
                let _ = AttachThreadInput(current, fg_tid, false);
            }
            if attached_target {
                let _ = AttachThreadInput(current, target_tid, false);
            }
        }
        // 切换是异步完成的：给一点时间再核对。
        for _ in 0..10 {
            if is_foreground(target) {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(30));
        }
        Err("系统没有允许切换前台窗口（可能被全屏程序或安全软件阻止），请手动切到游戏".into())
    }

    pub fn close_window(handle: u64) -> Result<(), String> {
        let target = hwnd(handle);
        unsafe {
            if !IsWindow(Some(target)).as_bool() {
                return Err("窗口已经关闭".into());
            }
            PostMessageW(Some(target), WM_CLOSE, WPARAM(0), LPARAM(0))
                .map_err(|e| format!("请求关闭窗口失败：{e}"))
        }
    }

    pub fn launch(path: &str) -> Result<(), String> {
        let path = path.trim();
        if path.is_empty() {
            return Err("启动路径为空".into());
        }
        let file = HSTRING::from(path);
        let verb = HSTRING::from("open");
        let result = unsafe {
            ShellExecuteW(
                None,
                &verb,
                &file,
                PCWSTR::null(),
                PCWSTR::null(),
                SW_SHOWNORMAL,
            )
        };
        // 返回值大于 32 表示成功（ShellExecute 的约定）。
        if result.0 as isize > 32 {
            Ok(())
        } else {
            Err(format!(
                "打开失败（代码 {}）：请检查游戏配置里的启动路径",
                result.0 as isize
            ))
        }
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn focus_window(_handle: u64) -> Result<(), String> {
        Err("只支持 Windows".into())
    }
    pub fn close_window(_handle: u64) -> Result<(), String> {
        Err("只支持 Windows".into())
    }
    pub fn launch(_path: &str) -> Result<(), String> {
        Err("只支持 Windows".into())
    }
}

pub use imp::{close_window, focus_window, launch};
