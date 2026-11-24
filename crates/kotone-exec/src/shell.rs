//! `shell.run`：Windows PowerShell 5.1（系统自带）执行一段命令。
//!
//! - 命令从标准输入交给 PowerShell，再 `[ScriptBlock]::Create` 执行（照 Codex 更新程序的写法
//!   `Invoke-Expression ([Console]::In.ReadToEnd())`）：不用在命令行参数里转义引号；
//!   用 `-EncodedCommand` 时报错会变成 CLIXML，模型读不懂。
//! - 输出强制 UTF-8（中文 Windows 控制台默认 GBK）；原生程序仍按代码页输出的行逐行回退解码。
//! - 退出码：原生程序的非零 `$LASTEXITCODE` 优先，否则有错误记录就是 1，否则 0。
//! - 进程放进 Job Object（`KILL_ON_JOB_CLOSE`；代码在 `kotone_platform_windows::job`，和 `kotone-mcp`
//!   共用），先挂起创建、放进去再恢复，免得它在放进去之前就启动了子进程。超时 / 取消：结束整个 Job（连同它启动的所有进程）。正常结束：先去掉
//!   `KILL_ON_JOB_CLOSE` 再关掉 Job——命令里 `Start-Process` 启动、要一直开着的程序（例如
//!   BetterGI）不会被连带结束；Kotone 自己崩溃时句柄被系统关掉，Job 里还在跑的命令一并结束。

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use kotone_core::cancel::CancellationToken;

use crate::text;
#[cfg(windows)]
use kotone_platform_windows::job;

pub const DEFAULT_TIMEOUT_MS: u64 = 120_000;
pub const MAX_TIMEOUT_MS: u64 = 600_000;
const MIN_TIMEOUT_MS: u64 = 1_000;
/// 收输出的上限：头尾各 1 MB（之后再截到 3 万字）。
const CAPTURE_HALF: usize = 1 << 20;
/// 命令结束后最多再等多久把管道里剩下的读完（`Start-Process -NoNewWindow` 启动的程序会一直拿着管道）。
const DRAIN_AFTER_EXIT: Duration = Duration::from_millis(1_000);

/// PowerShell 启动后先跑的这段：UTF-8、不显示进度条、从标准输入读命令并执行、算退出码。
const BOOTSTRAP: &str = "$ProgressPreference='SilentlyContinue'; \
try { [Console]::OutputEncoding = New-Object System.Text.UTF8Encoding $false } catch {}; \
$OutputEncoding = New-Object System.Text.UTF8Encoding $false; \
try { [Console]::InputEncoding = New-Object System.Text.UTF8Encoding $false } catch {}; \
$global:LASTEXITCODE = 0; \
$__kotone_errors = $Error.Count; \
$__kotone_script = [Console]::In.ReadToEnd(); \
. ([ScriptBlock]::Create($__kotone_script)); \
if ($global:LASTEXITCODE) { exit $global:LASTEXITCODE }; \
if ($Error.Count -gt $__kotone_errors) { exit 1 }; \
exit 0";

#[derive(Debug, Clone)]
pub struct ShellRun {
    pub exit_code: Option<i32>,
    pub output: String,
    pub truncated: bool,
    pub duration_ms: u64,
    pub timed_out: bool,
    pub cancelled: bool,
    /// 放进了 Job Object（失败时超时只能结束 PowerShell 本身）。
    pub in_job: bool,
}

pub fn clamp_timeout(ms: Option<u64>) -> Duration {
    Duration::from_millis(
        ms.unwrap_or(DEFAULT_TIMEOUT_MS)
            .clamp(MIN_TIMEOUT_MS, MAX_TIMEOUT_MS),
    )
}

pub fn powershell_path() -> PathBuf {
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
    PathBuf::from(root)
        .join("System32")
        .join("WindowsPowerShell")
        .join("v1.0")
        .join("powershell.exe")
}

#[derive(Default)]
struct Capture {
    head: Vec<u8>,
    tail: std::collections::VecDeque<u8>,
    dropped: usize,
}

impl Capture {
    fn push(&mut self, bytes: &[u8]) {
        let room = CAPTURE_HALF.saturating_sub(self.head.len());
        let (to_head, rest) = bytes.split_at(room.min(bytes.len()));
        self.head.extend_from_slice(to_head);
        self.tail.extend(rest);
        while self.tail.len() > CAPTURE_HALF {
            self.tail.pop_front();
            self.dropped += 1;
        }
    }

    fn finish(self) -> (String, bool) {
        let head = text::decode_output(&self.head);
        let tail_bytes: Vec<u8> = self.tail.into_iter().collect();
        let tail = text::decode_output(&tail_bytes);
        let joined = if self.dropped > 0 {
            format!("{head}\n…（中间省略约 {} 字节）…\n{tail}", self.dropped)
        } else {
            format!("{head}{tail}")
        };
        let joined = joined.replace("\r\n", "\n");
        let (out, cut) = text::truncate_middle(&joined, text::OUTPUT_LIMIT_CHARS);
        (out, cut || self.dropped > 0)
    }
}

enum Ended {
    Exited(Option<i32>),
    TimedOut,
    Cancelled,
}

/// 执行一段 PowerShell。`cwd` 必须是已存在的目录。
pub async fn run(
    script: &str,
    cwd: &Path,
    timeout: Duration,
    cancel: &CancellationToken,
) -> Result<ShellRun, String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let started = Instant::now();
    let mut cmd = tokio::process::Command::new(powershell_path());
    cmd.args([
        "-NoLogo",
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-Command",
        BOOTSTRAP,
    ])
    .current_dir(cwd)
    .stdin(std::process::Stdio::piped())
    .stdout(std::process::Stdio::piped())
    .stderr(std::process::Stdio::piped())
    .kill_on_drop(true);
    #[cfg(windows)]
    let (mut child, job) =
        job::spawn_in_job(&mut cmd).map_err(|e| format!("启动 PowerShell 失败：{e}"))?;
    #[cfg(not(windows))]
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("启动 PowerShell 失败：{e}"))?;

    #[cfg(windows)]
    let in_job = job.is_some();
    #[cfg(not(windows))]
    let in_job = false;

    let capture = Arc::new(Mutex::new(Capture::default()));
    let mut readers = Vec::new();
    if let Some(mut out) = child.stdout.take() {
        let cap = capture.clone();
        readers.push(tokio::spawn(async move {
            let mut buf = [0u8; 8192];
            while let Ok(n) = out.read(&mut buf).await {
                if n == 0 {
                    break;
                }
                cap.lock().unwrap().push(&buf[..n]);
            }
        }));
    }
    if let Some(mut err) = child.stderr.take() {
        let cap = capture.clone();
        readers.push(tokio::spawn(async move {
            let mut buf = [0u8; 8192];
            while let Ok(n) = err.read(&mut buf).await {
                if n == 0 {
                    break;
                }
                cap.lock().unwrap().push(&buf[..n]);
            }
        }));
    }
    if let Some(mut stdin) = child.stdin.take() {
        let bytes = script.as_bytes().to_vec();
        tokio::spawn(async move {
            let _ = stdin.write_all(&bytes).await;
            let _ = stdin.shutdown().await;
        });
    }

    let ended = tokio::select! {
        status = child.wait() => Ended::Exited(status.ok().and_then(|s| s.code())),
        _ = tokio::time::sleep(timeout) => Ended::TimedOut,
        _ = cancel.cancelled() => Ended::Cancelled,
    };
    let (exit_code, timed_out, cancelled) = match ended {
        Ended::Exited(code) => {
            #[cfg(windows)]
            if let Some(job) = job {
                job.release();
            }
            (code, false, false)
        }
        Ended::TimedOut | Ended::Cancelled => {
            #[cfg(windows)]
            if let Some(job) = job {
                job.terminate();
            }
            let _ = child.kill().await;
            (
                None,
                matches!(ended, Ended::TimedOut),
                matches!(ended, Ended::Cancelled),
            )
        }
    };
    for reader in readers {
        let abort = reader.abort_handle();
        if tokio::time::timeout(DRAIN_AFTER_EXIT, reader)
            .await
            .is_err()
        {
            abort.abort();
        }
    }
    let capture = std::mem::take(&mut *capture.lock().unwrap());
    let (output, truncated) = capture.finish();
    Ok(ShellRun {
        exit_code,
        output,
        truncated,
        duration_ms: started.elapsed().as_millis() as u64,
        timed_out,
        cancelled,
        in_job,
    })
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    fn cwd() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    async fn ps(script: &str, timeout_ms: u64) -> ShellRun {
        let dir = cwd();
        run(
            script,
            dir.path(),
            Duration::from_millis(timeout_ms),
            &CancellationToken::default(),
        )
        .await
        .unwrap()
    }

    fn alive(pid: u32) -> bool {
        let out = std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).contains(&format!("\"{pid}\""))
    }

    fn kill(pid: u32) {
        let _ = std::process::Command::new("taskkill")
            .args(["/F", "/PID", &pid.to_string()])
            .output();
    }

    fn pid_in(output: &str) -> u32 {
        output
            .lines()
            .find_map(|l| l.trim().strip_prefix("PID="))
            .and_then(|p| p.trim().parse().ok())
            .unwrap_or_else(|| panic!("no PID in {output:?}"))
    }

    #[tokio::test]
    async fn chinese_output_and_exit_codes() {
        let r = ps("Write-Output '中文输出'; cmd /c echo 原生中文", 30_000).await;
        assert!(r.in_job);
        assert_eq!(r.exit_code, Some(0), "{r:?}");
        assert!(r.output.contains("中文输出"), "{r:?}");
        assert!(r.output.contains("原生中文"), "{r:?}");

        assert_eq!(ps("cmd /c exit 3", 30_000).await.exit_code, Some(3));
        assert_eq!(ps("exit 5", 30_000).await.exit_code, Some(5));
        let r = ps("Get-Item C:\\kotone-does-not-exist-123", 30_000).await;
        assert_eq!(r.exit_code, Some(1));
        assert!(r.output.contains("kotone-does-not-exist-123"), "{r:?}");
        assert!(!r.output.contains("CLIXML"));
        let r = ps("Write-Output \"q'uote\" 'a\"b'", 30_000).await;
        assert!(
            r.output.contains("q'uote") && r.output.contains("a\"b"),
            "{r:?}"
        );
    }

    #[tokio::test]
    async fn timeout_ends_the_command_and_its_children() {
        let r = ps(
            "$p = Start-Process ping -ArgumentList '-n','30','127.0.0.1' -PassThru -WindowStyle Hidden; \
             Write-Output \"PID=$($p.Id)\"; Wait-Process -Id $p.Id",
            3_000,
        )
        .await;
        assert!(r.timed_out, "{r:?}");
        let pid = pid_in(&r.output);
        let mut gone = false;
        for _ in 0..20 {
            if !alive(pid) {
                gone = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        kill(pid);
        assert!(gone, "ping {pid} survived the timeout");
    }

    #[tokio::test]
    async fn programs_left_running_survive_a_normal_finish() {
        let r = ps(
            "$p = Start-Process ping -ArgumentList '-n','20','127.0.0.1' -PassThru -WindowStyle Hidden; \
             Write-Output \"PID=$($p.Id)\"",
            30_000,
        )
        .await;
        assert_eq!(r.exit_code, Some(0), "{r:?}");
        let pid = pid_in(&r.output);
        tokio::time::sleep(Duration::from_millis(300)).await;
        let still = alive(pid);
        kill(pid);
        assert!(still, "ping {pid} was killed after a normal finish");
    }

    #[tokio::test]
    async fn cancel_stops_right_away() {
        let dir = cwd();
        let cancel = CancellationToken::default();
        let c = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(1_500)).await;
            c.cancel();
        });
        let started = Instant::now();
        let r = run(
            "Start-Sleep -Seconds 30",
            dir.path(),
            Duration::from_secs(60),
            &cancel,
        )
        .await
        .unwrap();
        assert!(r.cancelled);
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[tokio::test]
    async fn long_output_is_cut_in_the_middle() {
        let r = ps("1..20000 | ForEach-Object { \"line $_\" }", 60_000).await;
        assert!(r.truncated);
        assert!(r.output.contains("line 1\n"));
        assert!(r.output.contains("line 20000"));
        assert!(r.output.contains("中间省略"));
        assert!(r.output.chars().count() < text::OUTPUT_LIMIT_CHARS + 100);
    }

    #[tokio::test]
    async fn runs_in_the_given_directory() {
        let dir = cwd();
        let r = run(
            "(Get-Location).Path",
            dir.path(),
            Duration::from_secs(30),
            &CancellationToken::default(),
        )
        .await
        .unwrap();
        let name = dir
            .path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();
        assert!(r.output.contains(&name), "{r:?}");
    }
}
