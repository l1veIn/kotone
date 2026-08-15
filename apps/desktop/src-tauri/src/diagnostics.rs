//! 脱敏诊断包导出。
//!
//! 包内只放白名单环境信息、模型状态、历史元数据、结构化流程事件和脱敏日志。
//! 不复制 config.json / profiles / history 原文 / wav，避免识别文本、热词、音频、
//! 下载代理与本机路径意外外泄。

use std::io::Write;
use std::path::{Path, PathBuf};

use tauri::{AppHandle, Manager};
use zip::write::SimpleFileOptions;

use crate::hotkey::HotkeyManager;
use crate::runtime::RuntimeManager;
use crate::SharedState;

const PACKAGE_SCHEMA_VERSION: u32 = 2;
const MAX_PROCESS_EVENTS: usize = 20_000;
const MAX_HISTORY_RECORDS: usize = 50;
const MAX_LOG_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticExportResult {
    pub report_id: String,
    pub path: String,
    pub event_count: usize,
    pub history_count: usize,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct Manifest<'a> {
    schema_version: u32,
    report_id: &'a str,
    generated_at: String,
    app_version: &'a str,
    contains_recognition_text: bool,
    contains_audio: bool,
    contains_hotwords: bool,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct Environment {
    os_name: Option<String>,
    os_version: Option<String>,
    kernel_version: Option<String>,
    architecture: &'static str,
    elevated: bool,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeSnapshot {
    phase: String,
    restart_needed: bool,
    orchestrator_state: String,
    continuous_session: bool,
    engine_id: Option<String>,
    engine_name: Option<String>,
    model_id: Option<String>,
    interaction_mode: Option<String>,
    hotkey: String,
    hotkey_backend_preference: String,
    hotkey_registered: bool,
    hotkey_active_backend: String,
    hotkey_error_code: Option<String>,
    audio_device_name: Option<String>,
    active_profile_id: Option<String>,
    elevated: bool,
    run_as_admin_on_start: bool,
    history_mode: String,
    history_include_audio: bool,
    overlay_visibility: String,
    overlay_style: String,
    vad_compiled: bool,
    vad_model_ready: bool,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ModelSnapshot {
    id: String,
    engine_id: String,
    size_bytes: u64,
    downloaded: bool,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct HistoryMetadata {
    session_id: String,
    ts: String,
    engine_id: String,
    profile_id: Option<String>,
    audio_ms: u64,
    first_partial_ms: Option<u64>,
    finalize_latency_ms: Option<u64>,
    outcome: String,
    error_code: Option<String>,
    had_text: bool,
    text_chars: usize,
    had_audio: bool,
}

pub fn export(app: &AppHandle, requested_path: &Path) -> Result<DiagnosticExportResult, String> {
    let path = with_zip_extension(requested_path);
    let app_version = app.package_info().version.to_string();
    let report_id = format!("KT-{}", kotone_core::eval::new_session_id());
    let generated_at = kotone_core::eval::utc_now_iso();

    let shared = app.state::<SharedState>();
    let settings = shared.settings.read().unwrap().clone();
    let runtime = app
        .state::<RuntimeManager>()
        .status(&settings, &shared.engines, None);
    let hotkey = app.state::<HotkeyManager>().status();
    let audio_device_name = kotone_platform_windows::audio::list_devices()
        .into_iter()
        .find(|device| device.id == settings.audio_device_id)
        .map(|device| device.name);

    let runtime_snapshot = RuntimeSnapshot {
        phase: runtime.phase,
        restart_needed: runtime.restart_needed,
        orchestrator_state: serde_label(&shared.orchestrator.state()),
        continuous_session: shared.orchestrator.continuous_session(),
        engine_id: runtime.engine_id,
        engine_name: runtime.engine_name,
        model_id: runtime.model_id,
        interaction_mode: runtime.interaction_mode,
        hotkey: settings.hotkey.key.clone(),
        hotkey_backend_preference: serde_label(&settings.hotkey_backend),
        hotkey_registered: hotkey.registered,
        hotkey_active_backend: hotkey.backend,
        hotkey_error_code: hotkey.error.as_deref().map(classify_error),
        audio_device_name,
        active_profile_id: settings.active_profile_id.clone(),
        elevated: kotone_platform_windows::elevation::is_elevated(),
        run_as_admin_on_start: settings.run_as_admin_on_start,
        history_mode: serde_label(&settings.history.mode),
        history_include_audio: settings.history.include_audio,
        overlay_visibility: serde_label(&settings.overlay.visibility),
        overlay_style: serde_label(&settings.overlay.style),
        vad_compiled: kotone_stt::vad::compiled(),
        vad_model_ready: kotone_stt::model::vad_model_ready(),
    };

    let environment = Environment {
        os_name: sysinfo::System::name(),
        os_version: sysinfo::System::os_version(),
        kernel_version: sysinfo::System::kernel_version(),
        architecture: std::env::consts::ARCH,
        elevated: kotone_platform_windows::elevation::is_elevated(),
    };

    let models: Vec<ModelSnapshot> = kotone_stt::model::list()
        .unwrap_or_default()
        .into_iter()
        .map(|model| ModelSnapshot {
            id: model.id,
            engine_id: model.engine_id,
            size_bytes: model.size_bytes,
            downloaded: model.downloaded,
        })
        .collect();

    let history: Vec<HistoryMetadata> = kotone_core::history::list()
        .unwrap_or_default()
        .into_iter()
        .take(MAX_HISTORY_RECORDS)
        .map(|record| HistoryMetadata {
            session_id: record.session_id,
            ts: record.ts,
            engine_id: record.engine_id,
            profile_id: record.profile_id,
            audio_ms: record.audio_ms,
            first_partial_ms: record.first_partial_ms,
            finalize_latency_ms: record.finalize_latency_ms,
            outcome: serde_label(&record.outcome),
            error_code: record.error.as_deref().map(classify_error),
            had_text: !record.final_text.trim().is_empty(),
            text_chars: record.final_text.chars().count(),
            had_audio: record.audio_file.is_some(),
        })
        .collect();

    let process_events = kotone_core::process_log::list_recent(MAX_PROCESS_EVENTS);
    let events_csv = kotone_core::process_log::to_pm4py_csv(&process_events, &app_version);
    let sanitized_log = read_sanitized_log();
    let manifest = Manifest {
        schema_version: PACKAGE_SCHEMA_VERSION,
        report_id: &report_id,
        generated_at,
        app_version: &app_version,
        contains_recognition_text: false,
        contains_audio: false,
        contains_hotwords: false,
    };

    write_zip(
        &path,
        &[
            ("manifest.json", pretty_json(&manifest, "序列化诊断包清单")?),
            (
                "environment.json",
                pretty_json(&environment, "序列化环境信息")?,
            ),
            (
                "runtime.json",
                pretty_json(&runtime_snapshot, "序列化运行状态")?,
            ),
            ("models.json", pretty_json(&models, "序列化模型状态")?),
            (
                "history-metadata.json",
                pretty_json(&history, "序列化历史元数据")?,
            ),
            ("events.csv", events_csv),
            ("kotone.log", sanitized_log),
            ("README.txt", package_readme(&report_id)),
        ],
    )?;

    Ok(DiagnosticExportResult {
        report_id,
        path: path.to_string_lossy().into_owned(),
        event_count: process_events.len(),
        history_count: history.len(),
    })
}

fn write_zip(path: &Path, entries: &[(&str, String)]) -> Result<(), String> {
    let file = std::fs::File::create(path)
        .map_err(|e| format!("创建诊断包 {} 失败：{e}", path.display()))?;
    let mut zip = zip::ZipWriter::new(file);
    let options = SimpleFileOptions::default();
    for (name, content) in entries {
        zip.start_file(*name, options)
            .map_err(|e| format!("创建诊断包条目 {name} 失败：{e}"))?;
        zip.write_all(content.as_bytes())
            .map_err(|e| format!("写入诊断包条目 {name} 失败：{e}"))?;
    }
    zip.finish()
        .map_err(|e| format!("完成诊断包 {} 失败：{e}", path.display()))?;
    Ok(())
}

fn pretty_json<T: serde::Serialize>(value: &T, label: &str) -> Result<String, String> {
    serde_json::to_string_pretty(value).map_err(|e| format!("{label}失败：{e}"))
}

fn with_zip_extension(path: &Path) -> PathBuf {
    if path.extension().and_then(|s| s.to_str()) == Some("zip") {
        path.to_path_buf()
    } else {
        path.with_extension("zip")
    }
}

fn package_readme(report_id: &str) -> String {
    format!(
        "Kotone 脱敏诊断包\n\
         报告编号：{report_id}\n\n\
         本包不包含录音、识别文本或热词内容。\n\
         history-metadata.json 只保留耗时、结果、错误码和文本长度。\n\
         runtime.json 包含 VAD 编译、模型就绪和当前会话降级状态。\n\
         events.csv 可由 PM4Py 直接读取；前三列分别为 case id、activity、timestamp。\n"
    )
}

fn read_sanitized_log() -> String {
    let path = kotone_core::settings::kotone_dir().join("kotone.log");
    let Ok(bytes) = std::fs::read(path) else {
        return String::new();
    };
    let start = bytes.len().saturating_sub(MAX_LOG_BYTES);
    let raw = String::from_utf8_lossy(&bytes[start..]);
    raw.lines()
        .map(redact_log_line)
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn redact_log_line(line: &str) -> String {
    let mut redacted = redact_home(line);
    for marker in ["VAD 初始化失败", "VAD 推理失败"] {
        let Some(marker_index) = redacted.find(marker) else {
            continue;
        };
        let Some(detail_offset) = redacted[marker_index..].find(": ") else {
            continue;
        };
        redacted.truncate(marker_index + detail_offset);
        redacted.push_str(": [details redacted]");
        break;
    }
    if let Some(index) = redacted.find("state -> ") {
        let prefix = &redacted[..index];
        let state = redacted[index + "state -> ".len()..]
            .split_whitespace()
            .next()
            .unwrap_or("unknown");
        redacted = format!("{prefix}state -> {state} [payload redacted]");
    }
    redacted
}

pub(crate) fn redact_home(input: &str) -> String {
    let Some(home) = dirs::home_dir() else {
        return input.chars().take(2000).collect();
    };
    let home = home.to_string_lossy();
    let normalized_input = input.replace('\\', "/");
    let normalized_home = home.replace('\\', "/");
    normalized_input
        .replace(&normalized_home, "%USERPROFILE%")
        .chars()
        .take(2000)
        .collect()
}

fn serde_label<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string(value)
        .unwrap_or_else(|_| "\"unknown\"".into())
        .trim_matches('"')
        .to_string()
}

fn classify_error(error: &str) -> String {
    let lower = error.to_ascii_lowercase();
    // 「拦截」要最先判：SendInput 被系统拦截的消息里同时可能带「注入」字样，
    // 统一归到 INJECTION_BLOCKED，与 events.csv 的 errorCode 保持一致
    // （0.1.5 诊断包曾出现 events.csv=INJECTION_FAILED 而 history-metadata
    // =UNKNOWN_ERROR 的不一致——旧消息「SendInput 被系统拦截」不含任何关键字）
    if error.contains("拦截") || lower.contains("sendinput") || lower.contains("blocked") {
        "INJECTION_BLOCKED"
    } else if error.contains("管理员") || error.contains("提权") || lower.contains("elevation")
    {
        "ELEVATION_REQUIRED"
    } else if error.contains("热键") || lower.contains("hotkey") {
        "HOTKEY_FAILED"
    } else if error.contains("麦克风") || error.contains("录音") || lower.contains("audio") {
        "AUDIO_FAILED"
    } else if lower.contains("vad") || error.contains("静音检测") {
        "VAD_FAILED"
    } else if error.contains("模型") || error.contains("引擎") || lower.contains("model") {
        "MODEL_OR_ENGINE_FAILED"
    } else if error.contains("发送") || error.contains("注入") || lower.contains("inject") {
        "INJECTION_FAILED"
    } else {
        "UNKNOWN_ERROR"
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_state_payload_is_redacted() {
        let line = r#"[1.000] state -> sending {"payload":{"text":"秘密文本"},"state":"sending"}"#;
        let redacted = redact_log_line(line);
        assert_eq!(redacted, "[1.000] state -> sending [payload redacted]");
        assert!(!redacted.contains("秘密文本"));
    }

    #[test]
    fn vad_native_details_are_redacted_but_failure_stage_remains() {
        let line =
            r#"[1.000] VAD 初始化失败，本次会话降级为热键结束: 模型路径 D:\\Private\\silero.onnx"#;
        let redacted = redact_log_line(line);
        assert_eq!(
            redacted,
            "[1.000] VAD 初始化失败，本次会话降级为热键结束: [details redacted]"
        );
        assert!(!redacted.contains("Private"));
        assert_eq!(classify_error("VAD 初始化失败"), "VAD_FAILED");
    }

    #[test]
    fn classify_error_maps_blocked_sendinput() {
        // 0.1.5 不一致回归：SendInput 拦截消息旧分类落 UNKNOWN_ERROR，
        // 与 events.csv 的 INJECTION_FAILED 矛盾；0.1.6 统一为 INJECTION_BLOCKED
        assert_eq!(
            classify_error("SendInput 被系统拦截（0/10 成功）: 操作成功完成。"),
            "INJECTION_BLOCKED"
        );
        assert_eq!(
            classify_error("发送失败：目标窗口已关闭"),
            "INJECTION_FAILED"
        );
        assert_eq!(classify_error("未知异常"), "UNKNOWN_ERROR");
    }

    #[test]
    fn extension_is_normalized() {
        assert_eq!(
            with_zip_extension(Path::new("report")),
            PathBuf::from("report.zip")
        );
        assert_eq!(
            with_zip_extension(Path::new("report.zip")),
            PathBuf::from("report.zip")
        );
    }

    #[test]
    fn zip_contains_named_entries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("diagnostics.zip");
        write_zip(
            &path,
            &[
                ("manifest.json", "{}".into()),
                ("events.csv", "a,b\n".into()),
            ],
        )
        .unwrap();
        let file = std::fs::File::open(path).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        assert!(archive.by_name("manifest.json").is_ok());
        assert!(archive.by_name("events.csv").is_ok());
    }
}
