//! 读别的进程的命令行（ADR 035：英雄联盟客户端 `LeagueClientUx.exe` 的端口和令牌在命令行里）。
//!
//! 照 LeagueAkari（MIT）的做法：`NtQueryInformationProcess(ProcessCommandLineInformation)`，
//! 只要 `PROCESS_QUERY_LIMITED_INFORMATION`，**不用管理员**——国服客户端以管理员运行，
//! 2026-11-23 实测普通权限也读得到。不读进程内存。

/// 这个进程的完整命令行；打不开、读不到时 None。
#[cfg(windows)]
pub fn command_line(pid: u32) -> Option<String> {
    use windows::Wdk::System::Threading::{NtQueryInformationProcess, PROCESSINFOCLASS};
    use windows::Win32::Foundation::{CloseHandle, UNICODE_STRING};
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

    /// `ProcessCommandLineInformation`（Windows 8.1 起）。
    const PROCESS_COMMAND_LINE_INFORMATION: PROCESSINFOCLASS = PROCESSINFOCLASS(60);

    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut needed = 0u32;
        // 第一次只问要多大的缓冲区（返回“缓冲区太小”是预期的）。
        let _ = NtQueryInformationProcess(
            handle,
            PROCESS_COMMAND_LINE_INFORMATION,
            std::ptr::null_mut(),
            0,
            &mut needed,
        );
        let text = if needed as usize >= std::mem::size_of::<UNICODE_STRING>() {
            // 按 8 字节对齐分配：缓冲区开头是一个 UNICODE_STRING，后面跟着字符。
            let mut buf = vec![0u64; (needed as usize).div_ceil(8)];
            let status = NtQueryInformationProcess(
                handle,
                PROCESS_COMMAND_LINE_INFORMATION,
                buf.as_mut_ptr().cast(),
                needed,
                &mut needed,
            );
            if status.is_ok() {
                let us = &*(buf.as_ptr() as *const UNICODE_STRING);
                let len = usize::from(us.Length) / 2;
                (!us.Buffer.is_null() && len > 0)
                    .then(|| String::from_utf16_lossy(std::slice::from_raw_parts(us.Buffer.0, len)))
            } else {
                None
            }
        } else {
            None
        };
        let _ = CloseHandle(handle);
        text
    }
}

#[cfg(not(windows))]
pub fn command_line(_pid: u32) -> Option<String> {
    None
}

#[cfg(all(test, windows))]
mod tests {
    #[test]
    fn reads_our_own_command_line() {
        let cmd = super::command_line(std::process::id()).expect("自己的命令行");
        let exe = std::env::current_exe().unwrap();
        let stem = exe.file_stem().unwrap().to_string_lossy().to_string();
        assert!(cmd.contains(&stem), "{cmd}");
    }
}
