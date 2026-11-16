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
    /// 读取事件（Kotone → Agent 反向通道）：user.request、agent.*、runtime.phase、app.* 等。
    /// 不带 --follow 时输出一个信封；--follow 时逐行输出事件 JSON（JSON Lines），持续长轮询。
    Events {
        /// 上次返回的游标（只在同一实例内有效）
        #[arg(long)]
        cursor: Option<String>,
        /// 无游标时的起点：oldest（缺省，取回仍保留的事件）/ latest（只要之后的新事件）
        #[arg(long)]
        from: Option<String>,
        /// 类型过滤，逗号分隔；支持前缀 agent.*
        #[arg(long)]
        types: Option<String>,
        /// 暂无新事件时最多等待的毫秒数（0-30000）
        #[arg(long, default_value_t = 0)]
        wait_ms: u64,
        /// 持续输出（每行一个事件），游标失效或服务停止时退出
        #[arg(long)]
        follow: bool,
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

pub(crate) fn local_failure(error: ToolError) -> Envelope {
    Envelope::failure(String::new(), None, None, error, Timing::default())
}

pub(crate) fn print_json(value: &impl serde::Serialize) {
    match serde_json::to_string_pretty(value) {
        Ok(text) => println!("{text}"),
        Err(e) => eprintln!("序列化输出失败: {e}"),
    }
}

/// 单行 JSON（`--follow` 的 JSON Lines 输出）。
fn print_line(value: &impl serde::Serialize) {
    match serde_json::to_string(value) {
        Ok(text) => {
            println!("{text}");
            use std::io::Write as _;
            let _ = std::io::stdout().flush();
        }
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

pub(crate) fn unavailable(error: ToolError) -> i32 {
    print_json(&local_failure(error));
    EXIT_UNAVAILABLE
}

pub(crate) fn usage(code: ErrorCode, message: String) -> i32 {
    print_json(&local_failure(ToolError::new(code, message)));
    EXIT_USAGE
}

pub(crate) fn connect() -> Result<ToolClient, ToolError> {
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
        ToolsCommand::Events {
            cursor,
            from,
            types,
            wait_ms,
            follow,
        } => events(cursor, from, types, wait_ms, follow).await,
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

/// 原子写出：先把内容完整写入同目录的独占临时文件，再一次性发布到目标路径。
///
/// - 未指定 `--force`：用不覆盖的原子改名（MoveFileExW 不带 REPLACE_EXISTING）发布，
///   目标在任何时刻已存在都会失败，不存在“检查后被他人创建再被覆盖”的竞态。
/// - 指定 `--force`：`std::fs::rename` 原子替换已存在的文件。
///
/// 只有本次调用成功独占创建的临时文件才会在写入/同步/发布失败时被删除；
/// 若临时路径已存在（独占创建失败），直接返回错误，绝不删除不属于本次调用的文件。
fn write_output(path: &Path, bytes: &[u8], force: bool) -> Result<(), String> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    if !parent.is_dir() {
        return Err(format!("输出目录不存在: {}", parent.display()));
    }
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "artifact".into());
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let temp = parent.join(format!(
        ".{name}.{}.{nanos}.{}.part",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::SeqCst)
    ));
    write_output_via_temp(path, &temp, bytes, force)
}

/// [`write_output`] 的实际实现：经由调用方给定的临时路径写出并发布。
fn write_output_via_temp(
    path: &Path,
    temp: &Path,
    bytes: &[u8],
    force: bool,
) -> Result<(), String> {
    use std::io::Write as _;

    // 独占创建失败（含临时路径已存在）：该路径不属于本次调用，不做任何清理。
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(temp)
        .map_err(|e| format!("创建临时文件失败: {e}"))?;
    // 从这里起临时文件归本次调用所有；先关闭句柄再删除或发布。
    let written = file.write_all(bytes).and_then(|()| file.sync_all());
    drop(file);
    if let Err(e) = written {
        let _ = std::fs::remove_file(temp);
        return Err(format!("写入临时文件失败: {e}"));
    }
    let published = if force {
        std::fs::rename(temp, path)
    } else {
        kotone_platform_windows::fs_publish::rename_no_replace(temp, path)
    };
    match published {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(temp);
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                Err(format!("{} 已存在（使用 --force 覆盖）", path.display()))
            } else {
                Err(format!("写入 {} 失败: {e}", path.display()))
            }
        }
    }
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

fn encode_query(pairs: &[(&str, String)]) -> String {
    fn enc(s: &str) -> String {
        s.bytes()
            .map(|b| match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'*' | b',' => {
                    (b as char).to_string()
                }
                _ => format!("%{b:02X}"),
            })
            .collect()
    }
    pairs
        .iter()
        .map(|(k, v)| format!("{k}={}", enc(v)))
        .collect::<Vec<_>>()
        .join("&")
}

async fn events(
    cursor: Option<String>,
    from: Option<String>,
    types: Option<String>,
    wait_ms: u64,
    follow: bool,
) -> i32 {
    let client = match connect() {
        Ok(c) => c,
        Err(e) => return unavailable(e),
    };
    let wait = if follow { 25_000 } else { wait_ms };
    let mut cursor = cursor;
    let mut from = from;
    loop {
        let mut pairs = vec![("waitMs", wait.to_string())];
        if let Some(c) = &cursor {
            pairs.push(("cursor", c.clone()));
        } else if let Some(f) = &from {
            pairs.push(("from", f.clone()));
        }
        if let Some(t) = &types {
            pairs.push(("types", t.clone()));
        }
        let envelope = match client.events(&encode_query(&pairs), wait).await {
            Ok(e) => e,
            Err(e) if follow => {
                // --follow 输出的是 JSON Lines：出错时也只输出一行
                print_line(&local_failure(e));
                return EXIT_UNAVAILABLE;
            }
            Err(e) => return unavailable(e),
        };
        if !envelope.ok && follow {
            print_line(&envelope);
            return exit_for(&envelope);
        }
        if !follow {
            print_json(&envelope);
            return exit_for(&envelope);
        }
        let result = envelope.result.unwrap_or_default();
        for event in result["events"].as_array().into_iter().flatten() {
            println!("{}", serde_json::to_string(event).unwrap_or_default());
        }
        use std::io::Write as _;
        let _ = std::io::stdout().flush();
        if result["closed"] == true {
            return EXIT_OK;
        }
        cursor = result["cursor"].as_str().map(str::to_string);
        from = None;
    }
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

    fn leftover_parts(dir: &Path) -> Vec<String> {
        std::fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".part"))
            .collect()
    }

    #[test]
    fn concurrent_exports_without_force_publish_exactly_one_file() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("race.png");
        let writers = 12;
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(writers));
        let handles: Vec<_> = (0..writers)
            .map(|i| {
                let barrier = barrier.clone();
                let out = out.clone();
                std::thread::spawn(move || {
                    let bytes = vec![i as u8; 4096 + i];
                    barrier.wait();
                    (i, write_output(&out, &bytes, false))
                })
            })
            .collect();
        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        let winners: Vec<usize> = results
            .iter()
            .filter(|(_, r)| r.is_ok())
            .map(|(i, _)| *i)
            .collect();
        assert_eq!(winners.len(), 1, "恰好一个导出成功: {results:?}");
        for (_, r) in results.iter().filter(|(_, r)| r.is_err()) {
            assert!(r.as_ref().unwrap_err().contains("已存在"), "{r:?}");
        }
        let w = winners[0];
        assert_eq!(
            std::fs::read(&out).unwrap(),
            vec![w as u8; 4096 + w],
            "内容属于唯一胜者，未被替换"
        );
        assert!(
            leftover_parts(dir.path()).is_empty(),
            "失败者的临时文件已清理"
        );
    }

    #[test]
    fn file_created_after_any_check_is_never_replaced_without_force() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("late.png");
        // 模拟另一个进程在我们写临时文件期间创建目标
        std::fs::write(&out, b"someone else").unwrap();
        assert!(write_output(&out, b"mine", false).is_err());
        assert_eq!(std::fs::read(&out).unwrap(), b"someone else");
        assert!(leftover_parts(dir.path()).is_empty());
        // --force 才允许替换
        write_output(&out, b"mine", true).unwrap();
        assert_eq!(std::fs::read(&out).unwrap(), b"mine");
        // 发布失败（目标是目录）时临时文件也被清理
        let as_dir = dir.path().join("dir.png");
        std::fs::create_dir(&as_dir).unwrap();
        assert!(write_output(&as_dir, b"x", false).is_err());
        assert!(write_output(&as_dir, b"x", true).is_err());
        assert!(as_dir.is_dir());
        assert!(leftover_parts(dir.path()).is_empty());
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

    /// 临时路径已被他人占用：独占创建失败后必须原样返回，既不删除该文件，也不发布目标。
    #[test]
    fn preexisting_temp_path_is_never_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("shot.png");
        let temp = dir.path().join(".shot.png.sentinel.part");
        std::fs::write(&temp, b"not ours").unwrap();
        for force in [false, true] {
            let err = write_output_via_temp(&out, &temp, b"mine", force).unwrap_err();
            assert!(err.contains("创建临时文件失败"), "{err}");
            assert_eq!(
                std::fs::read(&temp).unwrap(),
                b"not ours",
                "不属于本次调用的临时文件必须保留"
            );
            assert!(!out.exists(), "未获得临时文件时不得发布");
        }
        // 对照：本次调用自己创建的临时文件在发布失败时被清理
        std::fs::write(&out, b"existing").unwrap();
        let own = dir.path().join(".shot.png.own.part");
        assert!(write_output_via_temp(&out, &own, b"mine", false).is_err());
        assert!(!own.exists(), "自有临时文件在发布失败后被删除");
        assert_eq!(std::fs::read(&out).unwrap(), b"existing");
        assert_eq!(std::fs::read(&temp).unwrap(), b"not ours");
    }
}
