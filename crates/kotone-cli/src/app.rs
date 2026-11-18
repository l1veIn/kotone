//! `kotone-cli app`：桌面应用生命周期（Agent 从“打开 exe”开始驱动 Kotone）。
//!
//! - `status`：是否在运行；运行时返回 `app.status` 的结果。
//! - `launch`：以 `--background` 启动 `kotone.exe`（只驻留托盘、不打开窗口、不抢焦点），
//!   等待工具服务就绪后返回实例信息；已在运行时直接返回现有实例，不唤起窗口。
//! - `quit`：调用 `app.quit`（等同托盘“退出”），等待进程退出、描述文件被删除。
//!
//! 退出码：0 成功（status：在运行）；1 未在运行（status）/ 操作失败；2 用法或配置错误；
//! 3 发现或连接失败。

use std::path::PathBuf;
use std::time::{Duration, Instant};

use clap::Subcommand;
use serde_json::{json, Value};

use kotone_core::tools::ToolHost as _;
use kotone_tools::protocol::{ErrorCode, ToolCallRequest, ToolError};

use crate::tools::{connect, print_json, EXIT_OK, EXIT_TOOL_FAILED, EXIT_UNAVAILABLE, EXIT_USAGE};

#[derive(Subcommand, Debug)]
pub enum AppCommand {
    /// Kotone 桌面端是否在运行（及其状态）
    Status {
        #[arg(long)]
        json: bool,
    },
    /// 后台启动 Kotone 桌面端并等待工具服务就绪（已在运行则直接返回）
    Launch {
        /// kotone.exe 路径（缺省：KOTONE_EXE 环境变量，其次与 kotone-cli 同目录）
        #[arg(long)]
        exe: Option<PathBuf>,
        /// 等待就绪的毫秒数
        #[arg(long, default_value_t = 30_000)]
        wait_ms: u64,
        /// 工具服务未开启时先写入 toolService.enabled=true 再启动
        #[arg(long)]
        enable_tool_service: bool,
        /// 正常启动（显示设置窗口），而不是后台启动
        #[arg(long)]
        foreground: bool,
    },
    /// 退出 Kotone 桌面端（等同托盘“退出”）并等待进程结束
    Quit {
        #[arg(long, default_value_t = 15_000)]
        wait_ms: u64,
    },
}

pub(crate) fn failure(code: ErrorCode, message: impl Into<String>) -> ToolError {
    ToolError::new(code, message)
}

/// 连接到正在运行的实例并返回 `app.status` 结果。
pub(crate) async fn running_status() -> Result<Value, ToolError> {
    let client = connect()?;
    let envelope = client
        .call(&ToolCallRequest {
            request_id: None,
            tool: "app.status".into(),
            arguments: json!({}),
            timeout_ms: Some(5_000),
        })
        .await?;
    match (envelope.ok, envelope.result, envelope.error) {
        (true, Some(result), _) => Ok(result),
        (_, _, Some(error)) => Err(error),
        _ => Err(failure(
            ErrorCode::ResponseInvalid,
            "app.status 没有返回结果",
        )),
    }
}

/// 解析 kotone.exe：显式给出（--exe 或 KOTONE_EXE）时只用它，不存在即报错——
/// 绝不悄悄改用别的可执行文件；都未给出时才看与 kotone-cli 同目录的 kotone.exe。
fn resolve_exe(explicit: Option<PathBuf>) -> Result<PathBuf, ToolError> {
    let (candidate, origin) = match explicit {
        Some(path) => (Some(path), "--exe"),
        None => match std::env::var_os("KOTONE_EXE").filter(|v| !v.is_empty()) {
            Some(path) => (Some(PathBuf::from(path)), "KOTONE_EXE"),
            None => (
                std::env::current_exe()
                    .ok()
                    .and_then(|p| p.parent().map(|d| d.join("kotone.exe"))),
                "kotone-cli 同目录",
            ),
        },
    };
    match candidate {
        Some(path) if path.is_file() => Ok(path),
        Some(path) => Err(failure(
            ErrorCode::InvalidArguments,
            format!("找不到 kotone.exe（{origin}：{}）", path.display()),
        )),
        None => Err(failure(
            ErrorCode::InvalidArguments,
            "找不到 kotone.exe：用 --exe 指定，或设置 KOTONE_EXE",
        )),
    }
}

/// 启动（或复用）桌面实例。成功返回 `{alreadyRunning, launched, status}`。
pub(crate) async fn launch(
    exe: Option<PathBuf>,
    wait_ms: u64,
    enable_tool_service: bool,
    foreground: bool,
) -> Result<Value, (i32, ToolError)> {
    if let Ok(status) = running_status().await {
        return Ok(json!({"alreadyRunning": true, "launched": false, "status": status}));
    }
    let mut settings = kotone_core::settings::load();
    if !settings.tool_service.enabled {
        if !enable_tool_service {
            return Err((
                EXIT_USAGE,
                failure(
                    ErrorCode::InvalidArguments,
                    "工具服务未开启（toolService.enabled=false）；加 --enable-tool-service 自动开启后启动",
                ),
            ));
        }
        settings.tool_service.enabled = true;
        kotone_core::settings::save(&settings).map_err(|e| {
            (
                EXIT_TOOL_FAILED,
                failure(ErrorCode::Internal, format!("写入配置失败: {e}")),
            )
        })?;
    }
    let exe = resolve_exe(exe).map_err(|e| (EXIT_USAGE, e))?;
    // 完全脱离且不继承任何句柄：调用方（Agent / MCP 客户端）捕获本命令输出用的管道
    // 不能泄漏进长驻的桌面进程，否则调用方永远等不到 EOF
    let args: &[&str] = if foreground { &[] } else { &["--background"] };
    let child = kotone_platform_windows::detached::spawn_detached(&exe, args).map_err(|e| {
        (
            EXIT_TOOL_FAILED,
            failure(
                ErrorCode::Internal,
                format!("启动 {} 失败: {e}", exe.display()),
            ),
        )
    })?;
    let started = Instant::now();
    let deadline = started + Duration::from_millis(wait_ms);
    let mut last_error = None;
    while Instant::now() < deadline {
        if let Some(code) = child.try_wait() {
            // 单实例：第二个进程转交给已在运行的实例后会立即退出，再确认一次
            if let Ok(status) = running_status().await {
                return Ok(json!({"alreadyRunning": true, "launched": false, "status": status}));
            }
            return Err((
                EXIT_TOOL_FAILED,
                failure(
                    ErrorCode::Internal,
                    format!("kotone.exe 启动后退出（{code}）"),
                ),
            ));
        }
        match running_status().await {
            Ok(status) => {
                return Ok(json!({
                    "alreadyRunning": false,
                    "launched": true,
                    "pid": child.id(),
                    "readyAfterMs": started.elapsed().as_millis() as u64,
                    "status": status,
                }))
            }
            Err(e) => last_error = Some(e),
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    Err((
        EXIT_UNAVAILABLE,
        last_error
            .unwrap_or_else(|| failure(ErrorCode::ServiceUnreachable, "等待工具服务就绪超时")),
    ))
}

/// 请求退出并等待进程结束。
pub(crate) async fn quit(wait_ms: u64) -> Result<Value, (i32, ToolError)> {
    let client = connect().map_err(|e| (EXIT_UNAVAILABLE, e))?;
    let pid = client.descriptor().pid;
    let started_at = client.descriptor().process_started_at;
    let envelope = client
        .call(&ToolCallRequest {
            request_id: None,
            tool: "app.quit".into(),
            arguments: json!({}),
            timeout_ms: Some(5_000),
        })
        .await
        .map_err(|e| (EXIT_UNAVAILABLE, e))?;
    if !envelope.ok {
        return Err((
            EXIT_TOOL_FAILED,
            envelope
                .error
                .unwrap_or_else(|| failure(ErrorCode::Internal, "app.quit 失败")),
        ));
    }
    let host = kotone_platform_windows::tool_host::WindowsToolHost;
    let begin = Instant::now();
    while begin.elapsed() < Duration::from_millis(wait_ms) {
        let alive = host
            .process_started_at(pid)
            .is_some_and(|t| Some(t) == started_at);
        if !alive {
            return Ok(
                json!({"quit": true, "pid": pid, "afterMs": begin.elapsed().as_millis() as u64}),
            );
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    Err((
        EXIT_TOOL_FAILED,
        failure(
            ErrorCode::Timeout,
            format!("进程 {pid} 在 {wait_ms}ms 内没有退出"),
        ),
    ))
}

fn report(result: Result<Value, (i32, ToolError)>) -> i32 {
    match result {
        Ok(value) => {
            print_json(&json!({"ok": true, "result": value}));
            EXIT_OK
        }
        Err((code, error)) => {
            print_json(&json!({"ok": false, "error": error}));
            code
        }
    }
}

pub async fn run(command: AppCommand) -> i32 {
    match command {
        AppCommand::Status { json } => match running_status().await {
            Ok(status) => {
                if json {
                    print_json(&json!({"running": true, "status": status}));
                } else {
                    println!(
                        "运行中  实例 {}  版本 {}  ASR {}  启动方式 {}",
                        status["instanceId"].as_str().unwrap_or("?"),
                        status["appVersion"].as_str().unwrap_or("?"),
                        status["runtime"]["phase"].as_str().unwrap_or("?"),
                        status["app"]["launch"].as_str().unwrap_or("?"),
                    );
                }
                EXIT_OK
            }
            Err(error) => {
                if json {
                    print_json(&json!({"running": false, "reason": error}));
                } else {
                    println!("未在运行（{}：{}）", error.code, error.message);
                }
                EXIT_TOOL_FAILED
            }
        },
        AppCommand::Launch {
            exe,
            wait_ms,
            enable_tool_service,
            foreground,
        } => report(launch(exe, wait_ms, enable_tool_service, foreground).await),
        AppCommand::Quit { wait_ms } => report(quit(wait_ms).await),
    }
}
