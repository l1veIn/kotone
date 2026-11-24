//! Job Object：把一个子进程连同它再启动的进程放在一起管。
//!
//! - 新建的 Job 带 `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`：句柄关掉时（包括 Kotone 自己崩溃、被结束，系统替它关）
//!   Job 里的进程一起结束；
//! - 先挂起创建子进程，放进 Job 再恢复（[`spawn_in_job`]），免得它在放进去之前就启动了自己的子进程。
//!
//! 没有状态：每次调用自己建一个 Job，句柄归调用方。两个用户（原则 14，第二个用户出现时从 `kotone-exec` 抽到这里）：
//! `kotone-exec` 的 `shell.run`（ADR 030）、`kotone-mcp` 起的 MCP 服务器子进程（MCP 方案第 1 期）。

use std::ffi::c_void;
use std::os::windows::io::RawHandle;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
};
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows::Win32::System::Threading::{OpenThread, ResumeThread, THREAD_SUSPEND_RESUME};

pub const CREATE_SUSPENDED: u32 = 0x0000_0004;
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// 一个匿名 Job。句柄存成整数，保证跨 `.await` 时 `Send`。Drop 时关闭句柄：
/// 没调过 [`Job::release`] 的话，里面还在跑的进程一起结束。
pub struct Job(isize);

impl Job {
    /// 新建一个关闭时结束全部进程的 Job。
    pub fn new() -> Result<Self, String> {
        // SAFETY: 匿名 Job，失败返回错误。
        let handle =
            unsafe { CreateJobObjectW(None, PCWSTR::null()) }.map_err(|e| e.to_string())?;
        let job = Job(handle.0 as isize);
        job.set_limits(JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE)?;
        Ok(job)
    }

    fn handle(&self) -> HANDLE {
        HANDLE(self.0 as *mut c_void)
    }

    fn set_limits(&self, flags: JOB_OBJECT_LIMIT) -> Result<(), String> {
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = flags;
        // SAFETY: info 是正确大小的结构，句柄有效。
        unsafe {
            SetInformationJobObject(
                self.handle(),
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        }
        .map_err(|e| e.to_string())
    }

    /// 把一个进程放进来。
    pub fn assign(&self, process: RawHandle) -> Result<(), String> {
        // SAFETY: 进程句柄来自调用方刚创建的子进程。
        unsafe { AssignProcessToJobObject(self.handle(), HANDLE(process)) }
            .map_err(|e| e.to_string())
    }

    /// 结束 Job 里的所有进程（然后关闭）。
    pub fn terminate(self) {
        // SAFETY: 句柄有效；之后 Drop 关闭它。
        let _ = unsafe { TerminateJobObject(self.handle(), 1) };
    }

    /// 正常结束：不再在关闭时结束剩下的进程，然后关闭（命令里另外启动、要一直开着的程序留着）。
    pub fn release(self) {
        let _ = self.set_limits(JOB_OBJECT_LIMIT(0));
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        // SAFETY: 只关一次。
        let _ = unsafe { CloseHandle(self.handle()) };
    }
}

/// 恢复挂起创建的进程的所有线程。
pub fn resume_process(pid: u32) {
    // SAFETY: 快照与线程句柄都在这里打开和关闭。
    unsafe {
        let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) else {
            return;
        };
        let mut entry = THREADENTRY32 {
            dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
            ..Default::default()
        };
        if Thread32First(snapshot, &mut entry).is_ok() {
            loop {
                if entry.th32OwnerProcessID == pid {
                    if let Ok(thread) = OpenThread(THREAD_SUSPEND_RESUME, false, entry.th32ThreadID)
                    {
                        ResumeThread(thread);
                        let _ = CloseHandle(thread);
                    }
                }
                if Thread32Next(snapshot, &mut entry).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snapshot);
    }
}

/// 起一个子进程并放进新建的 Job：挂起创建（不弹控制台窗口）→ 放进 Job → 恢复。
///
/// 放不进 Job 时（例如建 Job 失败）照样恢复运行，返回的 Job 是 `None`，调用方只能结束子进程本身。
/// `cmd` 原来设的 `creation_flags` 会被换成 `CREATE_SUSPENDED | CREATE_NO_WINDOW`。
pub fn spawn_in_job(
    cmd: &mut tokio::process::Command,
) -> std::io::Result<(tokio::process::Child, Option<Job>)> {
    cmd.creation_flags(CREATE_SUSPENDED | CREATE_NO_WINDOW);
    let child = cmd.spawn()?;
    let job = match (Job::new().ok(), child.raw_handle()) {
        (Some(job), Some(handle)) if job.assign(handle).is_ok() => Some(job),
        _ => None,
    };
    if let Some(pid) = child.id() {
        resume_process(pid);
    }
    Ok((child, job))
}
