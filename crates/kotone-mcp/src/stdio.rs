//! 子进程（stdio）服务器：起程序、放进 Job Object、管它的环境变量和 stderr，关的时候连同它启动的进程一起结束。
//!
//! - 环境变量：先清空，只放安全基线（当前进程里有的才放）和配置里写的（同名覆盖基线，Windows 上不分大小写）。
//!   基线照 Codex 的 `WINDOWS_CORE_ENV_VARS`（`codex-rs/protocol/src/shell_environment.rs`）和 Hermes Agent 的
//!   `_build_safe_env`（`tools/mcp_tool_config.py`：只传白名单，不把自己的环境整个传下去）。Key 这类东西只能经配置的 `env` 给。
//! - 程序路径：Windows 上按子进程的 PATH 和 PATHEXT 解析（`which::which_in`，照 Codex 的 `program_resolver.rs`），
//!   `npx` → `…\npx.cmd`。`.cmd` / `.bat` 由标准库经 `cmd.exe` 跑：参数转义、遇到转义不了的参数直接拒绝，
//!   都是 Rust 1.77.2 起标准库做的（CVE-2024-24576），这里不自己拼 `cmd /c`。
//! - 不弹控制台窗口（Goose 也这样）；stderr 留最后 4 KB，启动失败时拿来说明原因。
//! - 以普通权限运行（ADR 044 D2）：Kotone 以管理员身份运行时，服务器以桌面为父进程创建、继承桌面的普通权限
//!   （`kotone_platform_windows::unelevated`，`.cmd` 的转义照标准库同一套规则）；降不下来就不启动，
//!   返回“没法用普通权限启动：……”。
//! - 关闭（MCP 规范的 stdio 关闭顺序）：先关它的标准输入（rmcp 关传输时做），等它自己退出（最多 2 秒），
//!   然后结束整个 Job（它没退出、或者它启动的进程还在）。

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
#[cfg(not(windows))]
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[cfg(windows)]
use kotone_platform_windows::unelevated::{self, PipedChild as Child, PipedCommand};
use tokio::io::AsyncReadExt;
#[cfg(not(windows))]
use tokio::process::Child;
use tokio::process::{ChildStdin, ChildStdout};

use crate::McpError;

/// 关闭时等它自己退出多久。
const EXIT_GRACE: Duration = Duration::from_secs(2);
/// stderr 留多少字节。
const STDERR_TAIL: usize = 4 * 1024;

/// 子进程默认能拿到的环境变量（当前进程里有的才给）：Codex 的 `WINDOWS_CORE_ENV_VARS` 加上 Hermes Agent
/// `_SAFE_ENV_KEYS_CASE_INSENSITIVE` 多出来的几个（都是系统位置和机器信息，没有密钥）。
#[cfg(windows)]
pub(crate) const BASELINE_ENV: &[&str] = &[
    // 找程序
    "PATH",
    "PATHEXT",
    // 系统目录和外壳
    "SHELL",
    "COMSPEC",
    "SYSTEMROOT",
    "WINDIR",
    "SYSTEMDRIVE",
    // 用户
    "USERNAME",
    "USERDOMAIN",
    "USERPROFILE",
    "HOMEDRIVE",
    "HOMEPATH",
    "ALLUSERSPROFILE",
    "PUBLIC",
    // 程序目录
    "PROGRAMFILES",
    "PROGRAMFILES(X86)",
    "PROGRAMW6432",
    "PROGRAMDATA",
    "COMMONPROGRAMFILES",
    "COMMONPROGRAMFILES(X86)",
    "COMMONPROGRAMW6432",
    // 应用数据和缓存（npm、uv 的缓存在这里）
    "LOCALAPPDATA",
    "APPDATA",
    // 临时目录
    "TEMP",
    "TMP",
    "TMPDIR",
    // 机器（Python 的 platform 这类会读）
    "COMPUTERNAME",
    "NUMBER_OF_PROCESSORS",
    "OS",
    "PROCESSOR_ARCHITECTURE",
    // PowerShell
    "POWERSHELL",
    "PWSH",
];

#[cfg(not(windows))]
pub(crate) const BASELINE_ENV: &[&str] = &[
    "HOME", "LOGNAME", "PATH", "SHELL", "USER", "LANG", "LC_ALL", "TERM", "TMPDIR", "TZ",
];

fn same_name(a: &str, b: &str) -> bool {
    if cfg!(windows) {
        a.eq_ignore_ascii_case(b)
    } else {
        a == b
    }
}

/// 子进程的全部环境变量：基线（从 `parent` 里挑）+ 配置里写的。
pub(crate) fn child_env(
    parent: impl IntoIterator<Item = (OsString, OsString)>,
    configured: &BTreeMap<String, String>,
) -> Vec<(OsString, OsString)> {
    let mut env: Vec<(OsString, OsString)> = parent
        .into_iter()
        .filter(|(name, _)| {
            name.to_str()
                .is_some_and(|n| BASELINE_ENV.iter().any(|b| same_name(b, n)))
        })
        .collect();
    for (name, value) in configured {
        env.retain(|(n, _)| !n.to_str().is_some_and(|n| same_name(n, name)));
        env.push((name.into(), value.into()));
    }
    env
}

/// 在子进程的 PATH 里找程序（Windows 上认 PATHEXT 里的 `.cmd` / `.bat`……）；找不到就原样交给系统，让它报错。
pub(crate) fn resolve_program(program: &str, env: &[(OsString, OsString)], cwd: &Path) -> OsString {
    if !cfg!(windows) {
        return program.into();
    }
    let path = env
        .iter()
        .find(|(n, _)| n.to_str().is_some_and(|n| same_name(n, "PATH")))
        .map(|(_, v)| v.clone());
    match which::which_in(program, path, cwd) {
        Ok(found) => found.into_os_string(),
        Err(_) => program.into(),
    }
}

/// stderr 的最后几 KB。
#[derive(Default)]
struct Tail(Vec<u8>);

impl Tail {
    fn push(&mut self, bytes: &[u8]) {
        self.0.extend_from_slice(bytes);
        if self.0.len() > STDERR_TAIL {
            let cut = self.0.len() - STDERR_TAIL;
            self.0.drain(..cut);
        }
    }
}

/// 一个跑着的服务器进程。Drop 时（没调 [`ServerProcess::shutdown`]）Job 的句柄被关掉，里面的进程一起结束。
pub(crate) struct ServerProcess {
    child: Child,
    #[cfg(windows)]
    job: Option<kotone_platform_windows::job::Job>,
    stderr: Arc<Mutex<Tail>>,
}

impl ServerProcess {
    pub(crate) fn id(&self) -> Option<u32> {
        self.child.id()
    }

    /// 它最后在 stderr 说的话（去掉首尾空白）。
    pub(crate) fn stderr_tail(&self) -> String {
        let tail = self.stderr.lock().unwrap();
        String::from_utf8_lossy(&tail.0).trim().to_string()
    }

    /// 启动失败时的说明：退出码（等一小会儿，它可能正在退出）+ stderr 的最后几行。
    pub(crate) async fn failure_note(&mut self) -> String {
        let exited = tokio::time::timeout(Duration::from_millis(500), self.child.wait())
            .await
            .ok()
            .and_then(Result::ok);
        // 给读 stderr 的任务一点时间读完
        tokio::time::sleep(Duration::from_millis(50)).await;
        let mut note = String::new();
        if let Some(status) = exited {
            match status.code() {
                Some(code) => note.push_str(&format!("程序退出了（退出码 {code}）")),
                None => note.push_str("程序退出了"),
            }
        }
        let tail = self.stderr_tail();
        if !tail.is_empty() {
            let last: Vec<&str> = tail.lines().rev().take(5).collect();
            let last: Vec<&str> = last.into_iter().rev().collect();
            if !note.is_empty() {
                note.push('。');
            }
            note.push_str(&format!("它最后说：{}", last.join(" / ")));
        }
        note
    }

    /// 关掉：等它自己退出（调用前传输已经关了，标准输入关了），然后结束整个 Job。
    pub(crate) async fn shutdown(mut self) {
        let _ = tokio::time::timeout(EXIT_GRACE, self.child.wait()).await;
        self.kill().await;
    }

    /// 马上结束它和它启动的所有进程。
    pub(crate) async fn kill(&mut self) {
        #[cfg(windows)]
        if let Some(job) = self.job.take() {
            job.terminate();
        }
        let _ = self.child.kill().await;
    }
}

pub(crate) struct Spawned {
    pub stdout: ChildStdout,
    pub stdin: ChildStdin,
    pub process: ServerProcess,
}

/// 起程序。
pub(crate) fn spawn(
    command: &str,
    args: &[String],
    env: &BTreeMap<String, String>,
    cwd: Option<&Path>,
) -> Result<Spawned, McpError> {
    if command.trim().is_empty() {
        return Err(McpError::Config("没写要运行的程序（command）".into()));
    }
    let env = child_env(std::env::vars_os(), env);
    let dir: PathBuf = match cwd {
        Some(dir) => dir.to_path_buf(),
        None => std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
    };
    let program = resolve_program(command, &env, &dir);

    #[cfg(windows)]
    let (mut child, job) = {
        let mut cmd = PipedCommand::new(PathBuf::from(&program))
            .args(args.iter().cloned())
            .env_clear_and_set(env);
        if cwd.is_some() {
            cmd = cmd.current_dir(&dir);
        }
        unelevated::spawn_piped_in_job(&cmd).map_err(|e| spawn_error(command, &program, e))?
    };
    #[cfg(not(windows))]
    let mut child = {
        let mut cmd = tokio::process::Command::new(&program);
        cmd.args(args)
            .env_clear()
            .envs(env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if cwd.is_some() {
            cmd.current_dir(&dir);
        }
        cmd.spawn().map_err(|e| spawn_error(command, &program, e))?
    };

    let stdout = child.stdout.take();
    let stdin = child.stdin.take();
    let stderr = Arc::new(Mutex::new(Tail::default()));
    if let Some(mut pipe) = child.stderr.take() {
        let tail = stderr.clone();
        tokio::spawn(async move {
            let mut buf = [0u8; 4096];
            while let Ok(n) = pipe.read(&mut buf).await {
                if n == 0 {
                    break;
                }
                tail.lock().unwrap().push(&buf[..n]);
            }
        });
    }
    let process = ServerProcess {
        child,
        #[cfg(windows)]
        job,
        stderr,
    };
    match (stdout, stdin) {
        (Some(stdout), Some(stdin)) => Ok(Spawned {
            stdout,
            stdin,
            process,
        }),
        _ => Err(McpError::Spawn("拿不到程序的标准输入输出".into())),
    }
}

fn spawn_error(command: &str, program: &OsString, error: std::io::Error) -> McpError {
    let shown = program.to_string_lossy();
    match error.kind() {
        std::io::ErrorKind::NotFound => McpError::Spawn(format!(
            "找不到程序“{command}”。装好它，或者在配置里写完整路径"
        )),
        std::io::ErrorKind::InvalidInput if is_batch(&shown) => McpError::Spawn(format!(
            "“{shown}”是批处理文件，参数里有没法安全转义的字符：{error}"
        )),
        _ => McpError::Spawn(format!("运行“{shown}”失败：{error}")),
    }
}

fn is_batch(program: &str) -> bool {
    let lower = program.to_ascii_lowercase();
    lower.ends_with(".cmd") || lower.ends_with(".bat")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os(pairs: &[(&str, &str)]) -> Vec<(OsString, OsString)> {
        pairs
            .iter()
            .map(|(a, b)| (OsString::from(a), OsString::from(b)))
            .collect()
    }

    #[test]
    fn env_is_baseline_plus_configured() {
        let parent = os(&[
            ("Path", "C:\\Windows"),
            ("SystemRoot", "C:\\Windows"),
            ("TEMP", "C:\\t"),
            ("DEEPSEEK_API_KEY", "secret"),
            ("KOTONE_HOME", "C:\\k"),
        ]);
        let mut configured = BTreeMap::new();
        configured.insert("TOKEN".to_string(), "abc".to_string());
        configured.insert("TEMP".to_string(), "D:\\t".to_string());
        let env = child_env(parent, &configured);
        let get = |name: &str| {
            env.iter()
                .find(|(n, _)| n.to_str().unwrap().eq_ignore_ascii_case(name))
                .map(|(_, v)| v.to_str().unwrap().to_string())
        };
        assert_eq!(get("TOKEN").as_deref(), Some("abc"));
        assert_eq!(get("TEMP").as_deref(), Some("D:\\t"), "配置里写的覆盖基线");
        assert_eq!(get("DEEPSEEK_API_KEY"), None, "Kotone 自己的环境不传下去");
        assert_eq!(get("KOTONE_HOME"), None);
        if cfg!(windows) {
            assert_eq!(
                get("PATH").as_deref(),
                Some("C:\\Windows"),
                "Windows 上不分大小写"
            );
            assert!(get("SYSTEMROOT").is_some());
        }
        assert_eq!(
            env.iter()
                .filter(|(n, _)| n.to_str().unwrap().eq_ignore_ascii_case("TEMP"))
                .count(),
            1
        );
    }

    #[cfg(windows)]
    #[test]
    fn names_resolve_through_pathext() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("fake-npx.cmd"), "@echo off\r\n").unwrap();
        let env = os(&[("PATH", dir.path().to_str().unwrap())]);
        let found = resolve_program("fake-npx", &env, dir.path());
        assert!(
            found
                .to_string_lossy()
                .to_ascii_lowercase()
                .ends_with("fake-npx.cmd"),
            "{found:?}"
        );
        // 找不到就原样交给系统
        assert_eq!(
            resolve_program("no-such-program-kotone", &env, dir.path()),
            "no-such-program-kotone"
        );
    }
}
