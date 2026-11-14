//! `kotone-cli tools`：本机 Agent 工具服务客户端（docs/tool-service.md）。
//!
//! 通过当前用户/会话私有的描述文件直接连接**已经在运行**的 Kotone 桌面实例；
//! 不启动 kotone.exe、不加载配置或 ASR 引擎，因此不会触发单实例回调抢焦点。
//! 会话凭证只从描述文件读取，从不出现在命令行参数或输出中。
//!
//! 退出码：0 成功；1 工具返回 ok=false；2 本地用法/文件错误；3 发现或连接失败。

use std::io::Read as _;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use clap::Subcommand;
use serde_json::{json, Value};

use kotone_tools::discovery;
use kotone_tools::protocol::{Envelope, ErrorCode, Timing, ToolCallRequest, ToolError};
use kotone_tools::ToolClient;

/// 参数文件上限（与服务端请求体上限一致）。
const MAX_ARGS_FILE_BYTES: u64 = 64 * 1024;

pub const EXIT_OK: i32 = 0;
pub const EXIT_TOOL_FAILED: i32 = 1;
pub const EXIT_USAGE: i32 = 2;
pub const EXIT_UNAVAILABLE: i32 = 3;

#[derive(Subcommand, Debug)]
pub enum ToolsCommand {
    /// 连接已运行的桌面实例并显示服务状态（ASR 停止时同样可用）
    Status {
        /// 输出完整 JSON 信封
        #[arg(long)]
        json: bool,
    },
    /// 列出实例真实实现的工具（--json 输出含 schema 的完整 capabilities）
    List {
        #[arg(long)]
        json: bool,
    },
    /// 调用工具：参数从 JSON 文件读取（`-` 表示标准输入），结果以 JSON 信封输出
    Call {
        /// 工具名，如 window.active / screen.capture / image.crop
        tool: String,
        /// 参数 JSON 文件（对象）；省略表示无参数
        #[arg(long)]
        args_file: Option<PathBuf>,
        /// 服务端执行期限（毫秒，1-30000，缺省 5000）
        #[arg(long)]
        timeout_ms: Option<u64>,
        /// 调用方关联 ID（字母数字与 - _ . :）
        #[arg(long)]
        request_id: Option<String>,
    },
    /// 产物操作
    Artifact {
        #[command(subcommand)]
        action: ArtifactCommand,
    },
}

#[derive(Subcommand, Debug)]
pub enum ArtifactCommand {
    /// 下载产物 PNG 到本地文件（文件由客户端写入；服务端不接触任何路径）
    Get {
        artifact_id: String,
        /// 输出 PNG 路径
        #[arg(long)]
        output: PathBuf,
        /// 覆盖已存在的输出文件
        #[arg(long)]
        force: bool,
    },
}

fn local_failure(error: ToolError) -> Envelope {
    Envelope::failure(String::new(), None, None, error, Timing::default())
}

fn print_json(value: &impl serde::Serialize) {
    match serde_json::to_string_pretty(value) {
        Ok(text) => println!("{text}"),
        Err(e) => eprintln!("序列化输出失败: {e}"),
    }
}

fn exit_for(envelope: &Envelope) -> i32 {
    if envelope.ok {
        EXIT_OK
    } else {
        EXIT_TOOL_FAILED
    }
}

fn unavailable(error: ToolError) -> i32 {
    print_json(&local_failure(error));
    EXIT_UNAVAILABLE
}

fn usage(code: ErrorCode, message: String) -> i32 {
    print_json(&local_failure(ToolError::new(code, message)));
    EXIT_USAGE
}

fn connect() -> Result<ToolClient, ToolError> {
    let dir = discovery::default_dir();
    ToolClient::discover(&dir, &kotone_platform_windows::tool_host::WindowsToolHost)
}

fn read_args(path: &Path) -> Result<Value, String> {
    let mut raw = Vec::new();
    if path.as_os_str() == "-" {
        std::io::stdin()
            .take(MAX_ARGS_FILE_BYTES + 1)
            .read_to_end(&mut raw)
            .map_err(|e| format!("读取标准输入失败: {e}"))?;
    } else {
        let file = std::fs::File::open(path)
            .map_err(|e| format!("无法打开参数文件 {}: {e}", path.display()))?;
        file.take(MAX_ARGS_FILE_BYTES + 1)
            .read_to_end(&mut raw)
            .map_err(|e| format!("读取参数文件失败: {e}"))?;
    }
    if raw.len() as u64 > MAX_ARGS_FILE_BYTES {
        return Err(format!("参数文件超过 {MAX_ARGS_FILE_BYTES} 字节上限"));
    }
    // 容忍 Windows 记事本写入的 UTF-8 BOM
    let text = String::from_utf8(raw).map_err(|_| "参数文件不是 UTF-8".to_string())?;
    let text = text.trim_start_matches('\u{feff}');
    let value: Value =
        serde_json::from_str(text).map_err(|e| format!("参数文件不是有效 JSON: {e}"))?;
    if !(value.is_object() || value.is_null()) {
        return Err("参数文件必须是 JSON 对象".into());
    }
    Ok(value)
}

pub async fn run(command: ToolsCommand) -> i32 {
    match command {
        ToolsCommand::Status { json } => status(json).await,
        ToolsCommand::List { json } => list(json).await,
        ToolsCommand::Call {
            tool,
            args_file,
            timeout_ms,
            request_id,
        } => call(tool, args_file, timeout_ms, request_id).await,
        ToolsCommand::Artifact {
            action:
                ArtifactCommand::Get {
                    artifact_id,
                    output,
                    force,
                },
        } => artifact_get(artifact_id, output, force).await,
    }
}

async fn status(as_json: bool) -> i32 {
    let client = match connect() {
        Ok(c) => c,
        Err(e) => return unavailable(e),
    };
    let envelope = match client.status().await {
        Ok(e) => e,
        Err(e) => return unavailable(e),
    };
    if as_json || !envelope.ok {
        print_json(&envelope);
        return exit_for(&envelope);
    }
    let r = envelope.result.clone().unwrap_or_default();
    println!("Kotone 工具服务：运行中");
    println!("  实例        {}", r["instanceId"].as_str().unwrap_or("?"));
    println!("  桌面版本    {}", r["appVersion"].as_str().unwrap_or("?"));
    println!(
        "  协议        {} v{}",
        r["protocol"].as_str().unwrap_or("?"),
        r["protocolVersion"]
    );
    println!(
        "  ASR 运行时  {}（工具不依赖 ASR）",
        r["asr"]["runtimePhase"].as_str().unwrap_or("?")
    );
    println!(
        "  捕获后端    {}{}",
        r["capture"]["method"].as_str().unwrap_or("?"),
        if r["capture"]["available"] == true {
            ""
        } else {
            "（不可用）"
        }
    );
    println!(
        "  产物        {}/{} 个，{} 字节",
        r["artifacts"]["count"], r["artifacts"]["maxArtifacts"], r["artifacts"]["totalBytes"]
    );
    EXIT_OK
}

async fn list(as_json: bool) -> i32 {
    let client = match connect() {
        Ok(c) => c,
        Err(e) => return unavailable(e),
    };
    let envelope = match client.capabilities().await {
        Ok(e) => e,
        Err(e) => return unavailable(e),
    };
    if as_json || !envelope.ok {
        print_json(&envelope);
        return exit_for(&envelope);
    }
    let r = envelope.result.clone().unwrap_or_default();
    for tool in r["tools"].as_array().into_iter().flatten() {
        println!(
            "{:<22} {:<9} {}",
            tool["name"].as_str().unwrap_or("?"),
            if tool["ready"] == true {
                "ready"
            } else {
                "not-ready"
            },
            tool["description"].as_str().unwrap_or("")
        );
    }
    EXIT_OK
}

async fn call(
    tool: String,
    args_file: Option<PathBuf>,
    timeout_ms: Option<u64>,
    request_id: Option<String>,
) -> i32 {
    let arguments = match &args_file {
        Some(path) => match read_args(path) {
            Ok(v) => v,
            Err(e) => return usage(ErrorCode::InvalidArguments, e),
        },
        None => Value::Null,
    };
    let client = match connect() {
        Ok(c) => c,
        Err(e) => return unavailable(e),
    };
    let request = ToolCallRequest {
        request_id,
        tool,
        arguments,
        timeout_ms,
    };
    match client.call(&request).await {
        Ok(envelope) => {
            print_json(&envelope);
            exit_for(&envelope)
        }
        Err(e) => unavailable(e),
    }
}

/// 原子写出：同目录临时文件 + rename，失败不留下半个文件。
fn write_output(path: &Path, bytes: &[u8], force: bool) -> Result<(), String> {
    if path.exists() && !force {
        return Err(format!("{} 已存在（使用 --force 覆盖）", path.display()));
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    if !parent.is_dir() {
        return Err(format!("输出目录不存在: {}", parent.display()));
    }
    let temp = parent.join(format!(
        ".{}.{}.part",
        path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "artifact".into()),
        std::process::id()
    ));
    let result = std::fs::write(&temp, bytes).and_then(|()| std::fs::rename(&temp, path));
    if let Err(e) = result {
        let _ = std::fs::remove_file(&temp);
        return Err(format!("写入 {} 失败: {e}", path.display()));
    }
    Ok(())
}

async fn artifact_get(artifact_id: String, output: PathBuf, force: bool) -> i32 {
    if output.exists() && !force {
        return usage(
            ErrorCode::InvalidArguments,
            format!("{} 已存在（使用 --force 覆盖）", output.display()),
        );
    }
    let client = match connect() {
        Ok(c) => c,
        Err(e) => return unavailable(e),
    };
    let request = ToolCallRequest {
        request_id: None,
        tool: "artifact.get".into(),
        arguments: json!({"artifactId": artifact_id, "includeContent": true}),
        timeout_ms: Some(30_000),
    };
    let mut envelope = match client.call(&request).await {
        Ok(e) => e,
        Err(e) => return unavailable(e),
    };
    if !envelope.ok {
        print_json(&envelope);
        return EXIT_TOOL_FAILED;
    }
    let Some(result) = envelope.result.as_mut() else {
        return unavailable(ToolError::new(ErrorCode::ResponseInvalid, "缺少 result"));
    };
    let data = result["content"]["data"].as_str().unwrap_or_default();
    let bytes = match base64::engine::general_purpose::STANDARD.decode(data) {
        Ok(b) => b,
        Err(e) => {
            return unavailable(ToolError::new(
                ErrorCode::ResponseInvalid,
                format!("PNG 内容不是有效 base64: {e}"),
            ))
        }
    };
    let expected_sha = result["sha256"].as_str().unwrap_or_default();
    if kotone_tools::image::sha256_hex(&bytes) != expected_sha
        || result["byteLength"].as_u64() != Some(bytes.len() as u64)
    {
        return unavailable(ToolError::new(
            ErrorCode::ResponseInvalid,
            "PNG 内容与元数据中的长度/SHA-256 不一致，未写入文件",
        ));
    }
    if let Err(e) = write_output(&output, &bytes, force) {
        return usage(ErrorCode::InvalidArguments, e);
    }
    if let Some(object) = result.as_object_mut() {
        object.remove("content");
        object.insert(
            "output".into(),
            json!({"path": output.display().to_string(), "bytesWritten": bytes.len()}),
        );
    }
    print_json(&envelope);
    EXIT_OK
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args_file_validation() {
        let dir = tempfile::tempdir().unwrap();
        let ok = dir.path().join("ok.json");
        std::fs::write(&ok, "\u{feff}{\"windowId\":\"win_1\"}").unwrap();
        assert_eq!(read_args(&ok).unwrap()["windowId"], "win_1");
        let array = dir.path().join("array.json");
        std::fs::write(&array, "[1]").unwrap();
        assert!(read_args(&array).is_err());
        let big = dir.path().join("big.json");
        std::fs::write(&big, format!("{{\"a\":\"{}\"}}", "x".repeat(70_000))).unwrap();
        assert!(read_args(&big).unwrap_err().contains("上限"));
        assert!(read_args(&dir.path().join("missing.json")).is_err());
    }

    #[test]
    fn output_is_not_overwritten_without_force() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("a.png");
        write_output(&out, b"one", false).unwrap();
        assert!(write_output(&out, b"two", false).is_err());
        assert_eq!(std::fs::read(&out).unwrap(), b"one");
        write_output(&out, b"two", true).unwrap();
        assert_eq!(std::fs::read(&out).unwrap(), b"two");
        assert!(write_output(&dir.path().join("no/such/dir.png"), b"x", false).is_err());
    }
}
