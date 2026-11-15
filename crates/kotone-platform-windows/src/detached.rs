//! 以“完全脱离”的方式启动进程：`bInheritHandles = FALSE`，独立进程组、无控制台。
//!
//! `std::process::Command` 在指定标准流时会以继承句柄方式创建进程，调用方的所有可继承
//! 句柄（例如 Agent 捕获 `kotone-cli` 输出用的管道）都会泄漏到被启动的长驻进程里——
//! 调用方于是永远等不到 EOF。启动 `kotone.exe` 这类长驻 GUI 进程必须不继承任何句柄。

use std::path::Path;

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::System::Threading::{
    CreateProcessW, GetExitCodeProcess, WaitForSingleObject, CREATE_NEW_PROCESS_GROUP,
    DETACHED_PROCESS, PROCESS_INFORMATION, STARTUPINFOW,
};

/// 已启动的脱离进程；只用于查询是否已退出，丢弃时关闭句柄（不结束进程）。
pub struct DetachedChild {
    process: HANDLE,
    pid: u32,
}

unsafe impl Send for DetachedChild {}

impl DetachedChild {
    pub fn id(&self) -> u32 {
        self.pid
    }

    /// 已退出则返回退出码。
    pub fn try_wait(&self) -> Option<u32> {
        unsafe {
            if WaitForSingleObject(self.process, 0) != WAIT_OBJECT_0 {
                return None;
            }
            let mut code = 0u32;
            GetExitCodeProcess(self.process, &mut code).ok()?;
            Some(code)
        }
    }
}

impl Drop for DetachedChild {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.process);
        }
    }
}

fn quote(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '"']) {
        return arg.to_string();
    }
    let mut out = String::from("\"");
    let mut backslashes = 0;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                out.push_str(&"\\".repeat(backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            _ => {
                out.push_str(&"\\".repeat(backslashes));
                out.push(c);
                backslashes = 0;
            }
        }
    }
    out.push_str(&"\\".repeat(backslashes * 2));
    out.push('"');
    out
}

/// 不继承任何句柄地启动 `exe args...`（继承当前环境变量与工作目录）。
pub fn spawn_detached(exe: &Path, args: &[&str]) -> std::io::Result<DetachedChild> {
    let exe_str = exe.to_string_lossy();
    let mut command_line = quote(&exe_str);
    for arg in args {
        command_line.push(' ');
        command_line.push_str(&quote(arg));
    }
    let mut command_line: Vec<u16> = command_line.encode_utf16().chain(Some(0)).collect();
    let application: Vec<u16> = exe_str.encode_utf16().chain(Some(0)).collect();
    let startup = STARTUPINFOW {
        cb: std::mem::size_of::<STARTUPINFOW>() as u32,
        ..Default::default()
    };
    let mut info = PROCESS_INFORMATION::default();
    unsafe {
        CreateProcessW(
            PCWSTR(application.as_ptr()),
            Some(PWSTR(command_line.as_mut_ptr())),
            None,
            None,
            false,
            DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP,
            None,
            PCWSTR::null(),
            &startup,
            &mut info,
        )
        .map_err(|e| std::io::Error::other(e.to_string()))?;
        let _ = CloseHandle(info.hThread);
    }
    Ok(DetachedChild {
        process: info.hProcess,
        pid: info.dwProcessId,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting_follows_windows_argv_rules() {
        assert_eq!(quote("--background"), "--background");
        assert_eq!(
            quote("C:\\Program Files\\Kotone\\kotone.exe"),
            "\"C:\\Program Files\\Kotone\\kotone.exe\""
        );
        assert_eq!(quote("a\"b"), "\"a\\\"b\"");
        assert_eq!(quote("dir\\"), "dir\\");
        assert_eq!(quote("with space\\"), "\"with space\\\\\"");
        assert_eq!(quote(""), "\"\"");
    }

    #[test]
    fn detached_child_reports_exit_without_inheriting_handles() {
        let cmd = std::path::PathBuf::from(
            std::env::var("ComSpec").unwrap_or_else(|_| "C:\\Windows\\System32\\cmd.exe".into()),
        );
        let child = spawn_detached(&cmd, &["/c", "exit", "7"]).unwrap();
        assert!(child.id() > 0);
        let mut code = None;
        for _ in 0..100 {
            code = child.try_wait();
            if code.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(code, Some(7));
        assert!(spawn_detached(Path::new("Z:\\missing\\nothing.exe"), &[]).is_err());
    }
}
