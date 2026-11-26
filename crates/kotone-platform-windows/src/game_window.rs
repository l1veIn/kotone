//! 游戏场景的窗口操作（ADR 014 第 5 步）：切到前台、请求关闭、打开程序。
//!
//! - 切前台：最小化的先还原；`SetForegroundWindow` 受前台锁限制时先轻按一下 Alt
//!   （系统允许最近有输入的进程切换前台），再不行就把输入队列附加到前台线程后重试；
//!   最后核对前台确实是目标窗口。
//! - 关闭：只投递 `WM_CLOSE`（游戏自己决定是否弹确认、怎么存档），从不强制结束进程。
//! - 打开：`ShellExecuteW("open")`，支持程序、快捷方式和启动器链接（如 `steam://`）；
//!   启动路径以引号开头时，引号里是程序、后面是参数（Riot 客户端要 `--launch-product=…`）；
//!   工作目录设为程序所在文件夹（腾讯的启动器要求）；要管理员权限的程序由 Windows 弹授权。
//!   路径来自用户在游戏配置里填写的“启动路径”，或检测到的已安装游戏（`games.rs`）。
//!   Kotone 以管理员身份运行时（ADR 044 D2）让桌面替我们打开（`unelevated::open_via_desktop`），
//!   游戏和启动器跟桌面一样是普通权限起的（它们自己要管理员时照样由 Windows 弹授权）。
//!
//! 非 Windows 平台编译为明确报错的兜底实现。

#[cfg(windows)]
mod imp {
    use std::time::Duration;

    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
    use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        keybd_event, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, VK_MENU,
    };
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetAncestor, GetForegroundWindow, GetWindowThreadProcessId, IsIconic, IsWindow,
        PostMessageW, SetForegroundWindow, ShowWindow, WindowFromPoint, GA_ROOT, SW_RESTORE,
        SW_SHOWNORMAL, WM_CLOSE,
    };

    /// 屏幕物理像素 (x, y) 上最上层可点的窗口属于哪个进程（顶层窗口的进程 ID）；
    /// 鼠标穿透的窗口（如琴音的气泡）不算。查不到时为 None。
    pub fn process_at(x: i32, y: i32) -> Option<u32> {
        unsafe {
            let child = WindowFromPoint(POINT { x, y });
            if child.0.is_null() {
                return None;
            }
            let root = GetAncestor(child, GA_ROOT);
            let window = if root.0.is_null() { child } else { root };
            let mut pid = 0u32;
            GetWindowThreadProcessId(window, Some(&mut pid));
            (pid != 0).then_some(pid)
        }
    }

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
            // 带 Kotone 的标记：钩子不把这下 Alt 当成玩家输入（ADR 019）。
            let tag = crate::inject::KOTONE_INPUT_TAG;
            keybd_event(VK_MENU.0 as u8, 0, KEYBD_EVENT_FLAGS(0), tag);
            keybd_event(VK_MENU.0 as u8, 0, KEYEVENTF_KEYUP, tag);
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
        let (program, args) = crate::games::split_command(path);
        let folder = std::path::Path::new(&program)
            .parent()
            .filter(|dir| dir.is_dir());
        if crate::unelevated::needed() {
            // 交给桌面以后拿不到“打不开”的结果，先看路径在不在（`steam://` 这类地址不看）
            let is_address = program.find(':').is_some_and(|i| i > 1);
            if !is_address && !std::path::Path::new(&program).exists() {
                return Err("打开失败（找不到这个程序）：请检查游戏配置里的启动路径".into());
            }
            return crate::unelevated::open_via_desktop(
                &program,
                args.as_deref(),
                folder,
                crate::unelevated::SHOW_NORMAL,
            )
            .map_err(|e| format!("打开失败：{e}"));
        }
        let directory = folder.map(|dir| HSTRING::from(dir.as_os_str()));
        let file = HSTRING::from(program.as_str());
        let params = args.as_deref().map(HSTRING::from);
        let verb = HSTRING::from("open");
        let result = unsafe {
            ShellExecuteW(
                None,
                &verb,
                &file,
                params
                    .as_ref()
                    .map_or(PCWSTR::null(), |p| PCWSTR(p.as_ptr())),
                directory
                    .as_ref()
                    .map_or(PCWSTR::null(), |d| PCWSTR(d.as_ptr())),
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
    pub fn process_at(_x: i32, _y: i32) -> Option<u32> {
        None
    }
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

pub use imp::{close_window, focus_window, launch, process_at};
