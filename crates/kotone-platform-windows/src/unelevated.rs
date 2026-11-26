//! 以普通权限启动（ADR 044 D2）：Kotone 以管理员身份运行时，她打开的东西一律降回普通权限——她跑的命令
//! （`shell.run`）、第三方 MCP 服务器、打开的链接和文件夹、启动的游戏、`kotone-cli app launch` 起的进程。
//! 模型决定的命令和别人的程序拿到管理员，出了事就是整台机器的事。
//!
//! 本体只有这一个启动入口，三种用法：
//!
//! 1. **要接管道、放进作业对象的**（`shell.run`、MCP 的 stdio 服务器）：[`spawn_piped_in_job`]。
//!    以桌面外壳（[`GetShellWindow`] 那个窗口的进程，就是资源管理器）为父进程创建
//!    （`PROC_THREAD_ATTRIBUTE_PARENT_PROCESS`），新进程继承外壳的普通权限令牌。照 PowerToys 的
//!    `run_non_elevated`（MIT，<https://github.com/microsoft/PowerToys> 的 `src/common/utils/elevation.h`）和
//!    Raymond Chen 的 “How can I launch an unelevated process from my elevated process, redux”
//!    （<https://devblogs.microsoft.com/oldnewthing/20190425-00/?p=102443>）。
//!
//!    PowerToys 不接管道（`bInheritHandles = FALSE`）。换了父进程以后，**可继承的句柄是从新的父进程那里继承的**
//!    （`UpdateProcThreadAttribute` 的文档：“Attributes inherited from the specified process include handles,
//!    the device map, processor affinity, priority, quotas, the process token, and job object”），我们进程里的管道句柄
//!    子进程拿不到。所以：先把三根管道子进程那一头复制进外壳进程（可继承），再用 `PROC_THREAD_ATTRIBUTE_HANDLE_LIST`
//!    只让这三个被继承（外壳自己的可继承句柄一个也不带过去）；`STARTUPINFO` 的标准输入输出填它们在外壳里的值
//!    （继承来的句柄值不变）；建好以后马上在外壳里关掉——不关的话外壳一直拿着输出管道写的那头，我们永远读不到结尾。
//!    挂起创建、放进关掉就杀的作业对象再恢复、`CREATE_NO_WINDOW`、环境变量，和直接启动时一样。
//!
//!    命令行和 `.cmd` / `.bat` 的转义照标准库（`library/std/src/sys/args/windows.rs` 的 `append_arg`、
//!    `make_bat_command_line`，MIT / Apache-2.0）：直接启动时是标准库做的；这条路不能用标准库（它只会把句柄建在
//!    自己的进程里），所以照抄同一套规则，`npx` 这类 `.cmd` 包装照样经 `cmd.exe` 安全转义（CVE-2024-24576）。
//!
//!    没选 `CreateProcessWithTokenW` + 外壳的令牌（tauri-apps/nsis-tauri-utils 的 `RunAsUser`、Chromium 的
//!    `base/win/elevation_util.cc` 用的办法，安装包结束后打开 Kotone 就是它）：它经“Secondary Logon”服务创建进程，
//!    文档列的创建标志里没有 `CREATE_NO_WINDOW`（PowerShell 会弹黑窗口），那个服务被关掉时也用不了。
//! 2. **交给外壳打开的**（链接、文件夹、启动游戏：要文件关联、快捷方式、`steam://` 这类地址，换父进程做不了——
//!    Raymond Chen 那篇也说了它只能做 `CreateProcess` 能直接做的事）：[`open_via_desktop`]。让桌面自己去打开：
//!    `IShellWindows::FindWindowSW(桌面)` → `IShellBrowser` → `IShellView` → `IShellFolderViewDual` →
//!    `IShellDispatch2::ShellExecute`，照 Raymond Chen 的 “How can I launch an unelevated process from my elevated
//!    process and vice versa?”（<https://devblogs.microsoft.com/oldnewthing/20131118-00/?p=2643>）。
//! 3. **完全脱离的长驻进程**（`kotone-cli app launch` 起的 `kotone.exe`，要拿到 PID 看它是不是马上退出）：
//!    [`spawn_detached_under_desktop`]，同 1，但不接管道、不继承任何句柄。
//!
//! Kotone 本来就是普通权限时（[`needed`] 为 false），调用方照原来的做法直接启动（行为不变）。
//! 降不下来（找不到桌面外壳，例如资源管理器没在运行；系统调用失败）时**不启动**，返回
//! “没法用普通权限启动：……”（[`REFUSED`]），从不偷偷以管理员身份跑。UAC 整个关掉的电脑上桌面本身就是管理员
//! （Raymond Chen 那篇的说明），那时没有“普通权限”可降，新进程和桌面一样。
//! 故意以管理员身份启动的只有：`runas` 重开自己（`elevation`）、游戏语音的 VB-CABLE 驱动安装程序。

use std::collections::BTreeMap;
use std::ffi::{c_void, OsStr, OsString};
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::os::windows::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::sync::OnceLock;

use tokio::process::{ChildStderr, ChildStdin, ChildStdout};
use tokio::sync::watch;
use windows::core::{Interface, BSTR, PCWSTR, PWSTR};
use windows::Win32::Foundation::{
    DuplicateHandle, DUPLICATE_CLOSE_SOURCE, DUPLICATE_SAME_ACCESS, HANDLE, INVALID_HANDLE_VALUE,
    WAIT_OBJECT_0,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, IDispatch, IServiceProvider,
    CLSCTX_LOCAL_SERVER, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
};
use windows::Win32::System::Pipes::CreatePipe;
use windows::Win32::System::Threading::{
    CreateProcessW, DeleteProcThreadAttributeList, GetCurrentProcess, GetExitCodeProcess,
    InitializeProcThreadAttributeList, OpenProcess, RegisterWaitForSingleObject, ResumeThread,
    TerminateProcess, UnregisterWaitEx, UpdateProcThreadAttribute, WaitForSingleObject,
    CREATE_NEW_PROCESS_GROUP, CREATE_NO_WINDOW, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT,
    DETACHED_PROCESS, EXTENDED_STARTUPINFO_PRESENT, INFINITE, LPPROC_THREAD_ATTRIBUTE_LIST,
    PROCESS_ACCESS_RIGHTS, PROCESS_CREATE_PROCESS, PROCESS_CREATION_FLAGS, PROCESS_DUP_HANDLE,
    PROCESS_INFORMATION, PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROC_THREAD_ATTRIBUTE_PARENT_PROCESS,
    STARTF_USESTDHANDLES, STARTUPINFOEXW, STARTUPINFOW, WT_EXECUTEONLYONCE,
};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Shell::{
    IShellBrowser, IShellDispatch2, IShellFolderViewDual, IShellView, IShellWindows,
    SID_STopLevelBrowser, ShellWindows, CSIDL_DESKTOP, SVGIO_BACKGROUND, SWC_DESKTOP,
    SWFO_NEEDDISPATCH,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, GetShellWindow, GetWindowThreadProcessId,
};

use crate::job::{self, Job};

/// 降不下来时错误的开头（ADR 044 D2：“没法用普通权限启动：……”）。
pub const REFUSED: &str = "没法用普通权限启动";

/// 测试用的开关：设了这个环境变量（任何值），不管 Kotone 是不是管理员都走“以桌面为父进程 / 让桌面打开”那条路。
/// 只会更严（多降一次权），所以留在正式版里也没坏处；用它把 `kotone-exec`、`kotone-mcp` 的整套测试换这条路跑一遍。
pub const FORCE_ENV: &str = "KOTONE_SPAWN_UNELEVATED";

/// `ShellExecute` 的 `SW_SHOWNORMAL`。
pub const SHOW_NORMAL: i32 = 1;

/// 现在启动东西要不要降权：Kotone 是管理员身份（或者打开了测试开关 [`FORCE_ENV`]）。
pub fn needed() -> bool {
    static ELEVATED: OnceLock<bool> = OnceLock::new();
    std::env::var_os(FORCE_ENV).is_some() || *ELEVATED.get_or_init(crate::elevation::is_elevated)
}

fn refused(reason: impl std::fmt::Display) -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!("{REFUSED}：{reason}"),
    )
}

/// `windows` 的错误 → `io::Error`：Win32 的错误码还原成原样（`NotFound` 这类判断要用）。
fn os_error(error: windows::core::Error) -> io::Error {
    let code = error.code().0 as u32;
    if code & 0xFFFF_0000 == 0x8007_0000 {
        io::Error::from_raw_os_error((code & 0xFFFF) as i32)
    } else {
        io::Error::other(error.message())
    }
}

fn raw(handle: &OwnedHandle) -> HANDLE {
    HANDLE(handle.as_raw_handle())
}

fn wide(text: &OsStr) -> Vec<u16> {
    text.encode_wide().chain(Some(0)).collect()
}

/// 桌面外壳（资源管理器）的进程 ID。
pub fn desktop_pid() -> io::Result<u32> {
    // SAFETY: 只读查询。
    let hwnd = unsafe { GetShellWindow() };
    if hwnd.0.is_null() {
        return Err(refused("找不到桌面（资源管理器没在运行）"));
    }
    let mut pid = 0u32;
    // SAFETY: hwnd 刚拿到，pid 是本地变量。
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    if pid == 0 {
        return Err(refused("找不到桌面的进程"));
    }
    Ok(pid)
}

fn open_desktop(pid: u32, access: PROCESS_ACCESS_RIGHTS) -> io::Result<OwnedHandle> {
    // SAFETY: 句柄交给 OwnedHandle 关。
    let handle = unsafe { OpenProcess(access, false, pid) }
        .map_err(|e| refused(format!("打不开桌面的进程（{}）", e.message())))?;
    Ok(unsafe { OwnedHandle::from_raw_handle(handle.0) })
}

// ---------- 1. 接管道、放进作业对象 ----------

/// 一个要接管道、放进作业对象的命令（`shell.run`、MCP 的 stdio 服务器）。标准输入、输出、错误都接管道。
#[derive(Debug, Clone)]
pub struct PipedCommand {
    program: PathBuf,
    args: Vec<String>,
    cwd: Option<PathBuf>,
    /// None：继承 Kotone 的环境变量；Some：先清空，只给这些。
    env: Option<Vec<(OsString, OsString)>>,
    /// None：看 [`needed`]；Some(true)：测试用，不管 Kotone 是不是管理员都以桌面为父进程。
    via_desktop: Option<bool>,
}

impl PipedCommand {
    pub fn new(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            cwd: None,
            env: None,
            via_desktop: None,
        }
    }

    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    pub fn current_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.cwd = Some(dir.into());
        self
    }

    /// 先清空环境变量，只给这些（同名的后面的算，Windows 上不分大小写）。
    pub fn env_clear_and_set(mut self, env: Vec<(OsString, OsString)>) -> Self {
        self.env = Some(env);
        self
    }

    /// 测试用：`true` 时不管 Kotone 是不是管理员都以桌面为父进程（看这条路本身对不对）。
    pub fn via_desktop(mut self, on: bool) -> Self {
        self.via_desktop = Some(on);
        self
    }
}

/// 起来的子进程：标准输入输出是 tokio 的类型（和以前 `tokio::process::Child` 拿到的一样）。
/// 丢掉时还没退出的话结束它（和以前的 `kill_on_drop(true)` 一样）。
pub struct PipedChild {
    pub stdin: Option<ChildStdin>,
    pub stdout: Option<ChildStdout>,
    pub stderr: Option<ChildStderr>,
    inner: Inner,
}

enum Inner {
    /// Kotone 是普通权限：照原来的 `tokio::process::Command` 起的。
    Direct(Box<tokio::process::Child>),
    /// 以桌面为父进程起的。
    Desktop(RawProcess),
}

impl PipedChild {
    pub fn id(&self) -> Option<u32> {
        match &self.inner {
            Inner::Direct(child) => child.id(),
            Inner::Desktop(process) => Some(process.pid),
        }
    }

    /// 是不是以桌面为父进程起的（降了权）。
    pub fn via_desktop(&self) -> bool {
        matches!(self.inner, Inner::Desktop(_))
    }

    /// 等它退出。可以被取消（`select!`、超时），之后再等。
    pub async fn wait(&mut self) -> io::Result<ExitStatus> {
        match &mut self.inner {
            Inner::Direct(child) => child.wait().await,
            Inner::Desktop(process) => process.wait().await,
        }
    }

    /// 结束它（只结束它本身；连同它启动的进程一起结束用作业对象）并等它退出。
    pub async fn kill(&mut self) -> io::Result<()> {
        match &mut self.inner {
            Inner::Direct(child) => child.kill().await,
            Inner::Desktop(process) => {
                process.start_kill()?;
                process.wait().await.map(|_| ())
            }
        }
    }
}

/// 起一个子进程，三根管道接好，放进新建的 Job（关掉就杀）。
///
/// - Kotone 是普通权限：照原来的做法（`tokio::process::Command` + [`job::spawn_in_job`]）。
/// - Kotone 是管理员：以桌面为父进程创建（见模块说明）；降不下来就不启动，返回 [`REFUSED`] 开头的错误。
///
/// 放不进 Job 时照样运行，返回的 Job 是 `None`（和 [`job::spawn_in_job`] 一样）。
pub fn spawn_piped_in_job(cmd: &PipedCommand) -> io::Result<(PipedChild, Option<Job>)> {
    if cmd.via_desktop.unwrap_or_else(needed) {
        spawn_under_desktop(cmd, None)
    } else {
        spawn_direct(cmd)
    }
}

/// 原来的做法（Kotone 是普通权限时）。
fn spawn_direct(cmd: &PipedCommand) -> io::Result<(PipedChild, Option<Job>)> {
    let mut command = tokio::process::Command::new(&cmd.program);
    command.args(&cmd.args);
    if let Some(env) = &cmd.env {
        command.env_clear().envs(env.iter().map(|(k, v)| (k, v)));
    }
    if let Some(dir) = &cmd.cwd {
        command.current_dir(dir);
    }
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let (mut child, job) = job::spawn_in_job(&mut command)?;
    Ok((
        PipedChild {
            stdin: child.stdin.take(),
            stdout: child.stdout.take(),
            stderr: child.stderr.take(),
            inner: Inner::Direct(Box::new(child)),
        },
        job,
    ))
}

/// 以桌面为父进程起（`desktop`：测试时给一个假的 PID 看降不下来的那条路）。
fn spawn_under_desktop(
    cmd: &PipedCommand,
    desktop: Option<u32>,
) -> io::Result<(PipedChild, Option<Job>)> {
    let desktop = match desktop {
        Some(pid) => pid,
        None => desktop_pid()?,
    };
    let program = resolve_program(&cmd.program, cmd.env.as_deref(), cmd.cwd.as_deref())?;
    let (application, mut line) = command_line(&program, &cmd.args)?;
    let env = cmd.env.as_deref().map(env_block).transpose()?;
    let cwd = cmd.cwd.as_deref().map(|d| wide(d.as_os_str()));

    let (stdin_child, stdin_ours) = pipe()?;
    let (stdout_ours, stdout_child) = pipe()?;
    let (stderr_ours, stderr_child) = pipe()?;
    let created = create_under_desktop(
        desktop,
        &application,
        &mut line,
        env.as_deref(),
        cwd.as_deref(),
        CREATE_SUSPENDED | CREATE_NO_WINDOW | CREATE_UNICODE_ENVIRONMENT,
        Some([&stdin_child, &stdout_child, &stderr_child]),
    )?;
    // 子进程那一头它已经继承了；我们手里的这份关掉，它退出时我们才读得到结尾。
    drop((stdin_child, stdout_child, stderr_child));

    let job = match Job::new() {
        Ok(job) if job.assign(created.process.as_raw_handle()).is_ok() => Some(job),
        _ => None,
    };
    let process = match RawProcess::new(created.process, created.pid) {
        Ok(process) => process,
        Err((e, handle)) => {
            // 还挂着：没法等它退出就别让它跑起来
            // SAFETY: 句柄有效。
            unsafe {
                let _ = TerminateProcess(raw(&handle), 1);
            }
            return Err(e);
        }
    };
    // SAFETY: 线程句柄是 CreateProcessW 给的。
    unsafe { ResumeThread(raw(&created.thread)) };

    let stdin = ChildStdin::from_std(std::process::ChildStdin::from(stdin_ours))?;
    let stdout = ChildStdout::from_std(std::process::ChildStdout::from(stdout_ours))?;
    let stderr = ChildStderr::from_std(std::process::ChildStderr::from(stderr_ours))?;
    Ok((
        PipedChild {
            stdin: Some(stdin),
            stdout: Some(stdout),
            stderr: Some(stderr),
            inner: Inner::Desktop(process),
        },
        job,
    ))
}

/// 一根匿名管道：(读的那头, 写的那头)，都不可继承。
fn pipe() -> io::Result<(OwnedHandle, OwnedHandle)> {
    let mut read = HANDLE::default();
    let mut write = HANDLE::default();
    // SAFETY: 两个出参是本地变量，句柄交给 OwnedHandle。
    unsafe {
        CreatePipe(&mut read, &mut write, None, 0).map_err(os_error)?;
        Ok((
            OwnedHandle::from_raw_handle(read.0),
            OwnedHandle::from_raw_handle(write.0),
        ))
    }
}

/// 复制进桌面进程的句柄（在桌面那边可继承）。Drop 时在桌面那边关掉。
struct InDesktop<'a> {
    desktop: &'a OwnedHandle,
    handles: Vec<HANDLE>,
}

impl InDesktop<'_> {
    fn put(&mut self, local: &OwnedHandle) -> io::Result<HANDLE> {
        let mut remote = HANDLE::default();
        // SAFETY: 两个进程句柄都有效；remote 是本地变量。
        unsafe {
            DuplicateHandle(
                GetCurrentProcess(),
                raw(local),
                raw(self.desktop),
                &mut remote,
                0,
                true,
                DUPLICATE_SAME_ACCESS,
            )
        }
        .map_err(|e| refused(format!("没法把管道交给桌面（{}）", e.message())))?;
        self.handles.push(remote);
        Ok(remote)
    }
}

impl Drop for InDesktop<'_> {
    fn drop(&mut self) {
        for handle in self.handles.drain(..) {
            // SAFETY: 在桌面那边关掉我们放进去的句柄（DUPLICATE_CLOSE_SOURCE，不复制）。
            unsafe {
                let _ = DuplicateHandle(
                    raw(self.desktop),
                    handle,
                    HANDLE::default(),
                    std::ptr::null_mut(),
                    0,
                    false,
                    DUPLICATE_CLOSE_SOURCE,
                );
            }
        }
    }
}

/// 进程/线程属性表（`STARTUPINFOEX` 用）。Drop 时删掉。
struct AttributeList {
    buffer: Vec<u64>,
}

impl AttributeList {
    fn new(count: u32) -> io::Result<Self> {
        let mut size = 0usize;
        // SAFETY: 第一次只问大小（按约定会返回“缓冲区不够”）。
        unsafe {
            let _ = InitializeProcThreadAttributeList(None, count, None, &mut size);
        }
        if size == 0 {
            return Err(refused("建不了进程属性表"));
        }
        let mut list = Self {
            buffer: vec![0u64; size.div_ceil(8)],
        };
        // SAFETY: 缓冲区按要求的大小分配、8 字节对齐。
        unsafe { InitializeProcThreadAttributeList(Some(list.ptr()), count, None, &mut size) }
            .map_err(|e| refused(format!("建不了进程属性表（{}）", e.message())))?;
        Ok(list)
    }

    fn ptr(&mut self) -> LPPROC_THREAD_ATTRIBUTE_LIST {
        LPPROC_THREAD_ATTRIBUTE_LIST(self.buffer.as_mut_ptr() as *mut c_void)
    }

    /// 系统只记 `value` 的指针：它指的东西要活到 `CreateProcessW` 之后。
    unsafe fn set(&mut self, attribute: u32, value: *const c_void, size: usize) -> io::Result<()> {
        UpdateProcThreadAttribute(
            self.ptr(),
            0,
            attribute as usize,
            Some(value),
            size,
            None,
            None,
        )
        .map_err(|e| refused(format!("设不了进程属性（{}）", e.message())))
    }
}

impl Drop for AttributeList {
    fn drop(&mut self) {
        // SAFETY: 初始化过才会有这个值。
        unsafe { DeleteProcThreadAttributeList(self.ptr()) };
    }
}

struct Created {
    process: OwnedHandle,
    thread: OwnedHandle,
    pid: u32,
}

/// 以桌面进程为父进程 `CreateProcessW`。`std_handles` 给了就接这三个（标准输入、输出、错误，我们进程里的），
/// 只继承它们；没给就不继承任何句柄。
fn create_under_desktop(
    desktop: u32,
    application: &[u16],
    command_line: &mut [u16],
    env: Option<&[u16]>,
    cwd: Option<&[u16]>,
    flags: PROCESS_CREATION_FLAGS,
    std_handles: Option<[&OwnedHandle; 3]>,
) -> io::Result<Created> {
    let access = if std_handles.is_some() {
        PROCESS_CREATE_PROCESS | PROCESS_DUP_HANDLE
    } else {
        PROCESS_CREATE_PROCESS
    };
    let desktop = open_desktop(desktop, access)?;
    let desktop_raw = raw(&desktop);
    let mut in_desktop = InDesktop {
        desktop: &desktop,
        handles: Vec::new(),
    };
    let inherited: Vec<HANDLE> = match std_handles {
        Some(handles) => handles
            .iter()
            .map(|h| in_desktop.put(h))
            .collect::<io::Result<_>>()?,
        None => Vec::new(),
    };

    let mut attributes = AttributeList::new(if inherited.is_empty() { 1 } else { 2 })?;
    // SAFETY: desktop_raw 和 inherited 都活到下面的 CreateProcessW 之后。
    unsafe {
        attributes.set(
            PROC_THREAD_ATTRIBUTE_PARENT_PROCESS,
            &desktop_raw as *const HANDLE as *const c_void,
            std::mem::size_of::<HANDLE>(),
        )?;
        if !inherited.is_empty() {
            attributes.set(
                PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
                inherited.as_ptr() as *const c_void,
                std::mem::size_of::<HANDLE>() * inherited.len(),
            )?;
        }
    }

    let mut startup = STARTUPINFOEXW {
        StartupInfo: STARTUPINFOW {
            cb: std::mem::size_of::<STARTUPINFOEXW>() as u32,
            ..Default::default()
        },
        lpAttributeList: attributes.ptr(),
    };
    if let &[input, output, error] = inherited.as_slice() {
        startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        startup.StartupInfo.hStdInput = input;
        startup.StartupInfo.hStdOutput = output;
        startup.StartupInfo.hStdError = error;
    }
    let mut info = PROCESS_INFORMATION::default();
    // SAFETY: 所有指针都指向本函数里还活着的缓冲区；命令行可写、以 0 结尾。
    let created = unsafe {
        CreateProcessW(
            PCWSTR(application.as_ptr()),
            Some(PWSTR(command_line.as_mut_ptr())),
            None,
            None,
            !inherited.is_empty(),
            flags | EXTENDED_STARTUPINFO_PRESENT,
            env.map(|block| block.as_ptr() as *const c_void),
            cwd.map_or(PCWSTR::null(), |dir| PCWSTR(dir.as_ptr())),
            &startup.StartupInfo,
            &mut info,
        )
    };
    // 不管建没建成，放进桌面的句柄马上在那边关掉。
    drop(in_desktop);
    drop(attributes);
    created.map_err(|e| {
        let error = os_error(e);
        match error.kind() {
            io::ErrorKind::NotFound | io::ErrorKind::InvalidInput => error,
            _ => refused(error),
        }
    })?;
    // SAFETY: CreateProcessW 成功时两个句柄都有效，交给 OwnedHandle。
    unsafe {
        Ok(Created {
            process: OwnedHandle::from_raw_handle(info.hProcess.0),
            thread: OwnedHandle::from_raw_handle(info.hThread.0),
            pid: info.dwProcessId,
        })
    }
}

/// 以桌面为父进程起的进程：等它退出（线程池里等，不占 tokio 的线程）、结束它。
struct RawProcess {
    handle: OwnedHandle,
    pid: u32,
    exited: watch::Receiver<bool>,
    /// `RegisterWaitForSingleObject` 的等待句柄（存成整数，保证 Send / Sync）。
    wait: isize,
    /// 回调里用的 `Box<watch::Sender<bool>>`（同上）。
    sender: usize,
}

unsafe extern "system" fn on_exit(context: *mut c_void, _timed_out: bool) {
    // SAFETY: context 是 RawProcess 持有的 Box<watch::Sender<bool>>，Drop 里先注销等待再释放它。
    let sender = unsafe { &*(context as *const watch::Sender<bool>) };
    sender.send_replace(true);
}

impl RawProcess {
    fn new(handle: OwnedHandle, pid: u32) -> Result<Self, (io::Error, OwnedHandle)> {
        let (tx, rx) = watch::channel(false);
        let sender = Box::into_raw(Box::new(tx));
        let mut wait = HANDLE::default();
        // SAFETY: 进程句柄活得比等待长（Drop 里先注销等待）。
        let registered = unsafe {
            RegisterWaitForSingleObject(
                &mut wait,
                raw(&handle),
                Some(on_exit),
                Some(sender as *const c_void),
                INFINITE,
                WT_EXECUTEONLYONCE,
            )
        };
        if let Err(e) = registered {
            // SAFETY: 没注册上，回调不会用它。
            drop(unsafe { Box::from_raw(sender) });
            return Err((os_error(e), handle));
        }
        Ok(Self {
            handle,
            pid,
            exited: rx,
            wait: wait.0 as isize,
            sender: sender as usize,
        })
    }

    fn exited_now(&self) -> bool {
        // SAFETY: 句柄有效。
        unsafe { WaitForSingleObject(raw(&self.handle), 0) == WAIT_OBJECT_0 }
    }

    fn status(&self) -> io::Result<ExitStatus> {
        let mut code = 0u32;
        // SAFETY: 句柄有效。
        unsafe { GetExitCodeProcess(raw(&self.handle), &mut code) }.map_err(os_error)?;
        Ok(ExitStatus::from_raw(code))
    }

    async fn wait(&mut self) -> io::Result<ExitStatus> {
        if !self.exited_now() {
            // 发送端在 self 里，self 活着就不会断
            let _ = self.exited.wait_for(|done| *done).await;
        }
        self.status()
    }

    fn start_kill(&self) -> io::Result<()> {
        if self.exited_now() {
            return Ok(());
        }
        // SAFETY: 句柄有效。
        unsafe { TerminateProcess(raw(&self.handle), 1) }.map_err(os_error)
    }
}

impl Drop for RawProcess {
    fn drop(&mut self) {
        let _ = self.start_kill();
        // SAFETY: INVALID_HANDLE_VALUE = 等正在跑的回调跑完再返回，之后再释放回调用的发送端。
        unsafe {
            let _ = UnregisterWaitEx(HANDLE(self.wait as *mut c_void), Some(INVALID_HANDLE_VALUE));
            drop(Box::from_raw(self.sender as *mut watch::Sender<bool>));
        }
    }
}

// ---------- 程序路径、命令行、环境变量（照标准库） ----------

fn is_batch(path: &Path) -> bool {
    path.extension()
        .and_then(OsStr::to_str)
        .is_some_and(|ext| ext.eq_ignore_ascii_case("bat") || ext.eq_ignore_ascii_case("cmd"))
}

fn env_value<'a>(env: &'a [(OsString, OsString)], name: &str) -> Option<&'a OsStr> {
    env.iter()
        .rev()
        .find(|(k, _)| k.to_str().is_some_and(|k| k.eq_ignore_ascii_case(name)))
        .map(|(_, v)| v.as_os_str())
}

fn system_root() -> PathBuf {
    PathBuf::from(std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into()))
}

/// 程序的完整路径。带目录的按工作目录解析；只有名字的在子进程的 PATH（没给环境变量就用 Kotone 的）、
/// `System32`、Windows 目录里找；没写扩展名的再试 `.exe`（标准库也这样）。找不到就是 `NotFound`。
fn resolve_program(
    program: &Path,
    env: Option<&[(OsString, OsString)]>,
    cwd: Option<&Path>,
) -> io::Result<PathBuf> {
    let candidates = |path: PathBuf| {
        let mut out = vec![path.clone()];
        if path.extension().is_none() {
            out.push(path.with_extension("exe"));
        }
        out
    };
    let bare = program.components().count() == 1 && !program.has_root();
    let tries: Vec<PathBuf> = if !bare {
        let path = if program.is_absolute() {
            program.to_path_buf()
        } else {
            let base = match cwd {
                Some(dir) => dir.to_path_buf(),
                None => std::env::current_dir()?,
            };
            base.join(program)
        };
        candidates(path)
    } else {
        let path_var = match env {
            Some(env) => env_value(env, "PATH").map(OsStr::to_os_string),
            None => std::env::var_os("PATH"),
        };
        let mut dirs: Vec<PathBuf> = path_var
            .map(|p| std::env::split_paths(&p).collect())
            .unwrap_or_default();
        dirs.push(system_root().join("System32"));
        dirs.push(system_root());
        dirs.into_iter()
            .flat_map(|dir| candidates(dir.join(program)))
            .collect()
    };
    tries.into_iter().find(|p| p.is_file()).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("找不到程序 {}", program.display()),
        )
    })
}

fn ensure_no_nuls(text: &str) -> io::Result<()> {
    if text.contains('\0') {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "参数里有 NUL 字符",
        ))
    } else {
        Ok(())
    }
}

/// 一个普通参数（标准库 `append_arg`，`Quote::Auto`）：有空格、制表符或者是空的才加引号；
/// 引号前的反斜杠翻倍再加一个，收尾引号前的反斜杠翻倍。
fn push_arg(cmd: &mut String, arg: &str) -> io::Result<()> {
    ensure_no_nuls(arg)?;
    let quote = arg.is_empty() || arg.contains([' ', '\t']);
    if quote {
        cmd.push('"');
    }
    let mut backslashes = 0usize;
    for c in arg.chars() {
        if c == '\\' {
            backslashes += 1;
        } else {
            if c == '"' {
                cmd.push_str(&"\\".repeat(backslashes + 1));
            }
            backslashes = 0;
        }
        cmd.push(c);
    }
    if quote {
        cmd.push_str(&"\\".repeat(backslashes));
        cmd.push('"');
    }
    Ok(())
}

/// 给批处理的一个参数（标准库 `append_bat_arg`）：除了字母数字和 `#$*+-./:?@\_` 都加引号；
/// `"` 写成 `""`；`%` 写成 `%%cd:~,%`（不让 cmd 展开 `%变量%`）；以 `\` 结尾的也加引号。
fn push_bat_arg(cmd: &mut String, arg: &str) -> io::Result<()> {
    ensure_no_nuls(arg)?;
    const UNQUOTED: &str = r"#$*+-./:?@\_";
    let mut quote = arg.is_empty() || arg.ends_with('\\');
    if arg.chars().any(|c| {
        (c.is_ascii() && !(c.is_ascii_alphanumeric() || UNQUOTED.contains(c))) || c.is_control()
    }) {
        quote = true;
    }
    if quote {
        cmd.push('"');
    }
    let mut backslashes = 0usize;
    for c in arg.chars() {
        if c == '\\' {
            backslashes += 1;
        } else {
            if c == '"' {
                cmd.push_str(&"\\".repeat(backslashes));
                cmd.push('"');
            } else if c == '%' || c == '\r' {
                cmd.push_str("%%cd:~,");
            }
            backslashes = 0;
        }
        cmd.push(c);
    }
    if quote {
        cmd.push_str(&"\\".repeat(backslashes));
        cmd.push('"');
    }
    Ok(())
}

/// 批处理经 `cmd.exe` 跑（标准库 `make_bat_command_line`）：`cmd.exe /e:ON /v:OFF /d /c ""脚本" 参数…"`。
/// 脚本路径里有 `"`、`^`、`%` 或以 `\` 结尾、参数里有换行的一律拒绝（`InvalidInput`）。
fn bat_command_line(script: &str, args: &[String]) -> io::Result<String> {
    let invalid = |what: &str| io::Error::new(io::ErrorKind::InvalidInput, what.to_string());
    if script.contains(['"', '^', '%']) || script.ends_with('\\') {
        return Err(invalid("批处理的路径里不能有 \"、^、%，也不能以 \\ 结尾"));
    }
    let mut cmd = String::from("cmd.exe /e:ON /v:OFF /d /c \"\"");
    cmd.push_str(script);
    cmd.push('"');
    for arg in args {
        cmd.push(' ');
        if arg.contains(['\r', '\n']) {
            return Err(invalid("batch file arguments are invalid"));
        }
        push_bat_arg(&mut cmd, arg)?;
    }
    cmd.push('"');
    Ok(cmd)
}

/// (lpApplicationName, 命令行)，都以 0 结尾。程序名总是加引号（标准库 `make_command_line`）。
fn command_line(program: &Path, args: &[String]) -> io::Result<(Vec<u16>, Vec<u16>)> {
    let shown = program.to_string_lossy();
    let shown = shown.strip_prefix(r"\\?\").unwrap_or(&shown).to_string();
    if is_batch(program) {
        let cmd_exe = system_root().join("System32").join("cmd.exe");
        let line = bat_command_line(&shown, args)?;
        return Ok((
            wide(cmd_exe.as_os_str()),
            line.encode_utf16().chain(Some(0)).collect(),
        ));
    }
    let mut line = format!("\"{shown}\"");
    for arg in args {
        line.push(' ');
        push_arg(&mut line, arg)?;
    }
    Ok((
        wide(OsStr::new(&shown)),
        line.encode_utf16().chain(Some(0)).collect(),
    ))
}

/// 环境变量块：`名=值\0…\0`，按名字排序（不分大小写），同名的后面的算。
fn env_block(env: &[(OsString, OsString)]) -> io::Result<Vec<u16>> {
    let mut sorted: BTreeMap<String, (&OsStr, &OsStr)> = BTreeMap::new();
    for (name, value) in env {
        let check = |s: &OsStr| {
            if s.encode_wide().any(|c| c == 0) {
                Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "环境变量里有 NUL 字符",
                ))
            } else {
                Ok(())
            }
        };
        check(name)?;
        check(value)?;
        sorted.insert(
            name.to_string_lossy().to_uppercase(),
            (name.as_os_str(), value.as_os_str()),
        );
    }
    let mut block = Vec::new();
    if sorted.is_empty() {
        block.push(0);
    }
    for (name, value) in sorted.values() {
        block.extend(name.encode_wide());
        block.push('=' as u16);
        block.extend(value.encode_wide());
        block.push(0);
    }
    block.push(0);
    Ok(block)
}

// ---------- 3. 完全脱离的长驻进程 ----------

/// 以桌面为父进程、完全脱离地启动 `exe args…`（不继承任何句柄、独立进程组、没有控制台；继承 Kotone 的环境变量和
/// 工作目录）。返回 (进程句柄, PID)。`kotone-cli app launch` 被管理员身份的程序调起时用（`detached.rs`）。
pub fn spawn_detached_under_desktop(exe: &Path, args: &[&str]) -> io::Result<(OwnedHandle, u32)> {
    let desktop = desktop_pid()?;
    let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    let (application, mut line) = command_line(exe, &args)?;
    let created = create_under_desktop(
        desktop,
        &application,
        &mut line,
        None,
        None,
        DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP,
        None,
    )?;
    Ok((created.process, created.pid))
}

// ---------- 2. 让桌面替我们打开 ----------

/// 让桌面替我们打开（`IShellDispatch2::ShellExecute`，见模块说明）：链接、文件夹、程序、快捷方式、`steam://` 这类地址。
/// 打开的东西是桌面起的，跟桌面一样是普通权限。参数和 `ShellExecuteW` 一样（动作固定是 `open`；`show` 是
/// `SW_SHOWNORMAL` 这类）。这是异步的：成功只说明桌面接了，打不开时由桌面自己提示，所以调用方先查路径在不在。
/// 在单独的线程里初始化 COM 再调用，不管调用方的线程是什么套间。
pub fn open_via_desktop(
    file: &str,
    args: Option<&str>,
    dir: Option<&Path>,
    show: i32,
) -> io::Result<()> {
    // 没有桌面就别去 COM（ShellWindows 是资源管理器提供的）
    let desktop = desktop_pid()?;
    // 前台权限交给桌面，它打开的浏览器、文件夹、游戏才能到前面来（不然只在任务栏上闪）
    // SAFETY: 只是授权，失败不影响打开。
    unsafe {
        let _ = AllowSetForegroundWindow(desktop);
    }
    let file = file.to_string();
    let args = args.unwrap_or_default().to_string();
    let dir = dir
        .map(|d| d.to_string_lossy().into_owned())
        .unwrap_or_default();
    std::thread::Builder::new()
        .name("kotone-open-via-desktop".into())
        .spawn(move || {
            // SAFETY: 这个线程自己初始化、自己反初始化 COM。
            unsafe {
                let init = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
                let result = desktop_shell_execute(&file, &args, &dir, show);
                if init.is_ok() {
                    CoUninitialize();
                }
                result
            }
        })
        .map_err(|e| refused(format!("开不了线程（{e}）")))?
        .join()
        .map_err(|_| refused("打开的线程意外结束"))?
        .map_err(|e| refused(format!("桌面没有接（{}）", e.message())))
}

/// Raymond Chen 的 `ShellExecuteFromExplorer`：桌面窗口 → 它的视图 → `Shell.Application` → `ShellExecute`。
unsafe fn desktop_shell_execute(
    file: &str,
    args: &str,
    dir: &str,
    show: i32,
) -> windows::core::Result<()> {
    let windows: IShellWindows = CoCreateInstance(&ShellWindows, None, CLSCTX_LOCAL_SERVER)?;
    let location = VARIANT::from(CSIDL_DESKTOP as i32);
    let root = VARIANT::default();
    let mut hwnd = 0i32;
    let dispatch: IDispatch =
        windows.FindWindowSW(&location, &root, SWC_DESKTOP, &mut hwnd, SWFO_NEEDDISPATCH)?;
    let provider: IServiceProvider = dispatch.cast()?;
    let browser: IShellBrowser = provider.QueryService(&SID_STopLevelBrowser)?;
    let view: IShellView = browser.QueryActiveShellView()?;
    let background: IDispatch = view.GetItemObject(SVGIO_BACKGROUND)?;
    let folder: IShellFolderViewDual = background.cast()?;
    let shell: IShellDispatch2 = folder.Application()?.cast()?;
    shell.ShellExecute(
        &BSTR::from(file),
        &VARIANT::from(args),
        &VARIANT::from(dir),
        &VARIANT::from("open"),
        &VARIANT::from(show),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::elevation::{process_integrity, INTEGRITY_MEDIUM};
    use std::time::{Duration, Instant};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn cmd_exe() -> PathBuf {
        system_root().join("System32").join("cmd.exe")
    }

    /// 没有桌面（CI 的服务会话里）就跳过“以桌面为父进程”的测试。
    fn desktop_or_skip() -> Option<u32> {
        match desktop_pid() {
            Ok(pid) => Some(pid),
            Err(e) => {
                eprintln!("[unelevated test] 跳过：{e}");
                None
            }
        }
    }

    /// 以桌面为父进程起的子进程拿的是桌面外壳的令牌：完整性级别和桌面一样，桌面是普通权限（平常的电脑）时就是普通
    /// （Medium）。GitHub 的 Windows 机器上用户是管理员、关了 UAC，桌面本身就是 High，子进程照样是 High（没有更低的令牌可拿）。
    fn assert_desktop_integrity(level: Option<u32>, desktop: u32) {
        let desktop_level = process_integrity(desktop).expect("读得到桌面的完整性级别");
        let level = level.expect("读得到完整性级别");
        assert_eq!(
            level, desktop_level,
            "完整性级别 {level:#x}，桌面是 {desktop_level:#x}"
        );
        if desktop_level == INTEGRITY_MEDIUM {
            assert_eq!(level, INTEGRITY_MEDIUM, "降到了普通权限");
        } else {
            eprintln!("[unelevated test] 桌面本身是 {desktop_level:#x}（关了 UAC 的管理员），只比对和桌面一样");
        }
    }

    fn parent_of(pid: u32) -> Option<u32> {
        crate::inject::process_list()
            .into_iter()
            .find(|(p, _, _)| *p == pid)
            .map(|(_, parent, _)| parent)
    }

    fn alive(pid: u32) -> bool {
        crate::inject::process_list()
            .iter()
            .any(|(p, _, _)| *p == pid)
    }

    async fn read_all(child: &mut PipedChild) -> (String, String) {
        let mut out = child.stdout.take().unwrap();
        let mut err = child.stderr.take().unwrap();
        let (mut o, mut e) = (Vec::new(), Vec::new());
        let (a, b) = tokio::join!(out.read_to_end(&mut o), err.read_to_end(&mut e));
        a.unwrap();
        b.unwrap();
        (
            String::from_utf8_lossy(&o).into_owned(),
            String::from_utf8_lossy(&e).into_owned(),
        )
    }

    #[test]
    fn args_are_quoted_like_the_standard_library() {
        let mut s = String::new();
        for arg in [
            "plain",
            "with space",
            "",
            "a\"b",
            "dir\\",
            "sp ace\\",
            "q\\\"x",
        ] {
            s.push(' ');
            push_arg(&mut s, arg).unwrap();
        }
        assert_eq!(s, r#" plain "with space" "" a\"b dir\ "sp ace\\" q\\\"x"#);
        assert!(push_arg(&mut String::new(), "nul\0").is_err());
    }

    #[test]
    fn batch_args_are_escaped_like_the_standard_library() {
        let line = bat_command_line(
            r"C:\tools\npx.cmd",
            &[
                "-y".into(),
                "@scope/pkg".into(),
                "a b".into(),
                "%PATH%".into(),
                "q\"x".into(),
                "&calc".into(),
                "end\\".into(),
            ],
        )
        .unwrap();
        assert_eq!(
            line,
            r#"cmd.exe /e:ON /v:OFF /d /c ""C:\tools\npx.cmd" -y @scope/pkg "a b" "%%cd:~,%PATH%%cd:~,%" "q""x" "&calc" "end\\"""#
        );
        assert_eq!(
            bat_command_line(r"C:\x.cmd", &["a\nb".into()])
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(
            bat_command_line(r"C:\100%.cmd", &[]).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
    }

    #[test]
    fn env_block_is_sorted_case_insensitive_and_last_wins() {
        let env = vec![
            (OsString::from("b"), OsString::from("2")),
            (OsString::from("A"), OsString::from("1")),
            (OsString::from("B"), OsString::from("3")),
        ];
        let block = String::from_utf16(&env_block(&env).unwrap()).unwrap();
        assert_eq!(block, "A=1\0B=3\0\0");
        assert_eq!(env_block(&[]).unwrap(), vec![0, 0]);
    }

    #[test]
    fn programs_resolve_like_before() {
        // 只有名字：在 PATH / System32 里找，补 .exe
        let found = resolve_program(Path::new("cmd"), None, None).unwrap();
        assert!(found
            .to_string_lossy()
            .to_ascii_lowercase()
            .ends_with("cmd.exe"));
        // 给了环境变量就用它的 PATH
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("fake-tool.cmd"), "@echo off\r\n").unwrap();
        let env = vec![(
            OsString::from("Path"),
            dir.path().as_os_str().to_os_string(),
        )];
        assert!(resolve_program(Path::new("fake-tool.cmd"), Some(&env), None).is_ok());
        // 找不到 = NotFound（MCP 报“找不到程序”）
        assert_eq!(
            resolve_program(Path::new("no-such-kotone-tool"), Some(&env), None)
                .unwrap_err()
                .kind(),
            io::ErrorKind::NotFound
        );
    }

    /// Kotone 是普通权限时照原来的做法直接启动：父进程是自己。
    #[tokio::test]
    async fn unelevated_kotone_spawns_directly_as_before() {
        if needed() {
            eprintln!("[unelevated test] 跳过：本身是管理员或开了 {FORCE_ENV}");
            return;
        }
        let cmd = PipedCommand::new(cmd_exe()).args(["/d", "/c", "echo direct"]);
        let (mut child, job) = spawn_piped_in_job(&cmd).unwrap();
        assert!(!child.via_desktop());
        assert!(job.is_some());
        let pid = child.id().unwrap();
        assert_eq!(parent_of(pid), Some(std::process::id()));
        let (out, _) = read_all(&mut child).await;
        assert!(out.contains("direct"), "{out:?}");
        assert_eq!(child.wait().await.unwrap().code(), Some(0));
    }

    /// 强制走“以桌面为父进程”：管道进出、退出码、环境变量、工作目录、父进程是桌面、完整性级别和桌面一样（平常是普通 Medium）。
    #[tokio::test]
    async fn desktop_parent_keeps_pipes_exit_code_env_and_runs_at_medium() {
        let Some(desktop) = desktop_or_skip() else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let mut env: Vec<(OsString, OsString)> = std::env::vars_os()
            .filter(|(k, _)| {
                ["SYSTEMROOT", "PATH", "COMSPEC"]
                    .iter()
                    .any(|n| k.to_str().is_some_and(|k| k.eq_ignore_ascii_case(n)))
            })
            .collect();
        env.push(("KOTONE_UNELEVATED_TEST".into(), "value-123".into()));
        // sort 把标准输入吐到标准输出（等到标准输入关掉才退出，所以先看父进程和完整性级别）；
        // 然后看环境变量、工作目录、标准错误，最后退出码 7
        let cmd = PipedCommand::new(cmd_exe())
            .args([
                "/d",
                "/c",
                "sort & echo ENV=%KOTONE_UNELEVATED_TEST% & cd & echo to-stderr 1>&2 & exit 7",
            ])
            .current_dir(dir.path())
            .env_clear_and_set(env)
            .via_desktop(true);
        let (mut child, job) = spawn_piped_in_job(&cmd).unwrap();
        assert!(child.via_desktop());
        assert!(job.is_some(), "放进作业对象");
        let pid = child.id().unwrap();
        assert_eq!(parent_of(pid), Some(desktop), "父进程是桌面");
        assert_desktop_integrity(process_integrity(pid), desktop);

        let mut stdin = child.stdin.take().unwrap();
        stdin.write_all(b"xyz from stdin\r\n").await.unwrap();
        stdin.shutdown().await.unwrap();
        drop(stdin);
        let (out, err) = read_all(&mut child).await;
        assert!(out.contains("xyz from stdin"), "{out:?}");
        assert!(out.contains("ENV=value-123"), "环境变量给到了：{out:?}");
        let name = dir
            .path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();
        assert!(out.contains(&name), "工作目录：{out:?}");
        assert!(err.contains("to-stderr"), "{err:?}");
        assert_eq!(child.wait().await.unwrap().code(), Some(7));
        drop(job);
    }

    /// 强制走“以桌面为父进程”：结束作业对象连同它启动的进程一起结束；丢掉 PipedChild 也会结束它。
    #[tokio::test]
    async fn desktop_parent_child_is_killed_with_its_job() {
        if desktop_or_skip().is_none() {
            return;
        }
        let cmd = PipedCommand::new(cmd_exe())
            .args(["/d", "/c", "ping -n 30 127.0.0.1 >nul"])
            .via_desktop(true);
        let (mut child, job) = spawn_piped_in_job(&cmd).unwrap();
        let pid = child.id().unwrap();
        // 等 cmd 把 ping 起来
        let started = Instant::now();
        let ping = loop {
            if let Some((p, _, _)) = crate::inject::process_list()
                .into_iter()
                .find(|(_, parent, name)| *parent == pid && name.eq_ignore_ascii_case("PING.EXE"))
            {
                break p;
            }
            assert!(started.elapsed() < Duration::from_secs(10), "ping 没起来");
            tokio::time::sleep(Duration::from_millis(50)).await;
        };
        job.unwrap().terminate();
        let status = tokio::time::timeout(Duration::from_secs(5), child.wait())
            .await
            .expect("作业对象结束后 wait 返回")
            .unwrap();
        assert_eq!(status.code(), Some(1));
        for _ in 0..50 {
            if !alive(ping) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(!alive(ping), "ping {ping} 跟着作业对象结束");

        // 丢掉：还在跑的结束（以前的 kill_on_drop）。`set /p` 等标准输入，标准输入先拿出来留着
        let cmd = PipedCommand::new(cmd_exe())
            .args(["/d", "/c", "set /p line="])
            .via_desktop(true);
        let (mut child, job) = spawn_piped_in_job(&cmd).unwrap();
        let pid = child.id().unwrap();
        let stdin = child.stdin.take();
        if let Some(job) = job {
            job.release();
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(alive(pid));
        drop(child);
        for _ in 0..50 {
            if !alive(pid) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(!alive(pid), "丢掉以后 {pid} 结束了");
        drop(stdin);
    }

    /// 强制走“以桌面为父进程”：`.cmd` 包装经 cmd.exe，参数原样到达。
    #[tokio::test]
    async fn desktop_parent_runs_batch_files_with_escaped_args() {
        if desktop_or_skip().is_none() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("echo args.cmd");
        std::fs::write(&script, "@echo off\r\necho [%1] [%2] [%3]\r\n").unwrap();
        let cmd = PipedCommand::new(&script)
            .args(["a b", "100%", "x&y"])
            .via_desktop(true);
        let (mut child, _job) = spawn_piped_in_job(&cmd).unwrap();
        let (out, err) = read_all(&mut child).await;
        assert!(
            out.contains(r#"["a b"] ["100%"] ["x&y"]"#),
            "{out:?} {err:?}"
        );
        assert_eq!(child.wait().await.unwrap().code(), Some(0));
    }

    /// 降不下来（桌面进程打不开）就不启动，错误以“没法用普通权限启动”开头；找不到程序还是 NotFound。
    #[tokio::test]
    async fn refuses_instead_of_running_elevated() {
        let cmd = PipedCommand::new(cmd_exe()).args(["/c", "exit 0"]);
        let Err(error) = spawn_under_desktop(&cmd, Some(0)) else {
            panic!("PID 0 不该能当父进程");
        };
        assert!(error.to_string().starts_with(REFUSED), "{error}");
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        let missing = PipedCommand::new("no-such-kotone-program").via_desktop(true);
        assert_eq!(
            spawn_under_desktop(&missing, Some(0))
                .err()
                .map(|e| e.kind()),
            Some(io::ErrorKind::NotFound)
        );
    }

    /// 完全脱离地以桌面为父进程启动：父进程是桌面、权限和桌面一样（平常是普通）、拿得到 PID。
    #[test]
    fn detached_under_desktop_is_unelevated() {
        let Some(desktop) = desktop_or_skip() else {
            return;
        };
        let (process, pid) =
            spawn_detached_under_desktop(&cmd_exe(), &["/d", "/c", "ping -n 3 127.0.0.1 >nul"])
                .unwrap();
        assert_eq!(parent_of(pid), Some(desktop));
        assert_desktop_integrity(process_integrity(pid), desktop);
        // SAFETY: 句柄有效。
        unsafe {
            let _ = TerminateProcess(raw(&process), 0);
        }
    }

    /// 让桌面打开：起一个没有窗口的 wscript（JScript 只睡几秒），看它的父进程是桌面、完整性级别和桌面一样（平常是普通）。
    /// 不弹窗、不抢焦点；`//E:JScript` 指定引擎（.js 常被关联给编辑器）；这台电脑没有 wscript 就跳过。
    #[test]
    fn desktop_shell_execute_starts_things_unelevated() {
        let Some(desktop) = desktop_or_skip() else {
            return;
        };
        let wscript = system_root().join("System32").join("wscript.exe");
        if !wscript.is_file() {
            eprintln!("[unelevated test] 跳过：没有 wscript.exe");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("kotone-sleep.js");
        std::fs::write(&script, "WScript.Sleep(8000);\r\n").unwrap();
        let marker = format!("kotone-unelevated-{}", std::process::id());
        let args = format!("//B //Nologo //E:JScript \"{}\" {marker}", script.display());
        open_via_desktop(&wscript.to_string_lossy(), Some(&args), Some(dir.path()), 0).unwrap();
        let started = Instant::now();
        let pid = loop {
            let found = crate::inject::process_list()
                .into_iter()
                .filter(|(_, _, name)| name.eq_ignore_ascii_case("wscript.exe"))
                .find(|(pid, _, _)| {
                    crate::process_cmdline::command_line(*pid)
                        .is_some_and(|line| line.contains(&marker))
                });
            if let Some((pid, _, _)) = found {
                break pid;
            }
            assert!(
                started.elapsed() < Duration::from_secs(10),
                "桌面没有起 wscript"
            );
            std::thread::sleep(Duration::from_millis(100));
        };
        let parent = parent_of(pid);
        let level = process_integrity(pid);
        let _ = std::process::Command::new("taskkill")
            .args(["/F", "/PID", &pid.to_string()])
            .output();
        assert_eq!(parent, Some(desktop), "是桌面起的");
        assert_desktop_integrity(level, desktop);
    }
}
