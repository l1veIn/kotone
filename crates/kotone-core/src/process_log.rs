//! 本地结构化流程事件：为诊断包与离线 PM4Py 分析提供最小事件日志。
//!
//! 数据只写入 `~/.kotone/diagnostics/events.jsonl`，不会自动上传。事件刻意不提供
//! 文本、音频、热词、窗口标题或文件路径字段，调用方只能填写白名单元数据。

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

const SCHEMA_VERSION: u32 = 1;
const MAX_EVENTS: usize = 20_000;
const TRIM_AT_BYTES: u64 = 4 * 1024 * 1024;

static WRITE_LOCK: Mutex<()> = Mutex::new(());
static APP_SESSION_ID: OnceLock<String> = OnceLock::new();
static EVENT_INDEX: AtomicU64 = AtomicU64::new(0);
static RECORDING: AtomicBool = AtomicBool::new(true);

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventContext {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub engine_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interaction_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elevated: Option<bool>,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventData {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_chars: Option<u64>,
    /// 短白名单细节（VK 名、overlay 原因、SendInput 计数）。不得写入识别文本。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessEvent {
    pub schema_version: u32,
    pub case_id: String,
    pub activity: String,
    pub timestamp: String,
    pub app_session_id: String,
    #[serde(default)]
    pub event_index: u64,
    #[serde(default)]
    pub context: EventContext,
    #[serde(default)]
    pub data: EventData,
}

impl ProcessEvent {
    pub fn new(case_id: impl Into<String>, activity: impl Into<String>) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            case_id: case_id.into(),
            activity: activity.into(),
            timestamp: crate::eval::utc_now_iso_millis(),
            app_session_id: app_session_id().to_string(),
            event_index: EVENT_INDEX.fetch_add(1, Ordering::Relaxed),
            context: EventContext::default(),
            data: EventData::default(),
        }
    }
}

pub fn diagnostics_dir() -> PathBuf {
    crate::settings::kotone_dir().join("diagnostics")
}

pub fn events_path() -> PathBuf {
    diagnostics_dir().join("events.jsonl")
}

pub fn app_session_id() -> &'static str {
    APP_SESSION_ID
        .get_or_init(|| format!("app-{}", crate::eval::new_session_id()))
        .as_str()
}

pub fn set_recording_enabled(enabled: bool) {
    RECORDING.store(enabled, Ordering::Relaxed);
}

pub fn recording_enabled() -> bool {
    RECORDING.load(Ordering::Relaxed)
}

/// 追加一条事件。失败返回错误给调用方决定是否降级；业务流程不应依赖该函数成功。
/// 用户关闭诊断记录时直接成功返回，不落盘。
pub fn record(event: ProcessEvent) -> Result<(), String> {
    if !recording_enabled() {
        return Ok(());
    }
    record_in(&events_path(), event)
}

/// 不依赖语音会话的诊断事件（热键/悬浮窗/注入），case id 用 app session。
pub fn record_activity(activity: impl Into<String>, data: EventData) {
    let mut event = ProcessEvent::new(app_session_id(), activity);
    event.data = data;
    let _ = record(event);
}

pub fn record_in(path: &Path, event: ProcessEvent) -> Result<(), String> {
    let _guard = WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建诊断事件目录失败: {e}"))?;
    }
    let line = serde_json::to_string(&event).map_err(|e| format!("序列化诊断事件失败: {e}"))?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| format!("打开诊断事件文件失败: {e}"))?;
    writeln!(file, "{line}").map_err(|e| format!("写入诊断事件失败: {e}"))?;
    drop(file);

    if std::fs::metadata(path)
        .map(|m| m.len() > TRIM_AT_BYTES)
        .unwrap_or(false)
    {
        trim_in(path)?;
    }
    Ok(())
}

fn trim_in(path: &Path) -> Result<(), String> {
    let raw = std::fs::read_to_string(path).map_err(|e| format!("读取诊断事件失败: {e}"))?;
    let lines: Vec<&str> = raw.lines().filter(|line| !line.trim().is_empty()).collect();
    if lines.len() <= MAX_EVENTS {
        return Ok(());
    }
    let mut kept = lines[lines.len() - MAX_EVENTS..].join("\n");
    kept.push('\n');
    let tmp = path.with_extension("jsonl.tmp");
    std::fs::write(&tmp, kept).map_err(|e| format!("裁剪诊断事件失败: {e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("落盘诊断事件失败: {e}"))
}

pub fn list_recent(max: usize) -> Vec<ProcessEvent> {
    list_recent_in(&events_path(), max)
}

pub fn list_recent_in(path: &Path, max: usize) -> Vec<ProcessEvent> {
    list_since_in(path, None, max)
}

pub fn list_since(cutoff_iso: Option<&str>, max: usize) -> Vec<ProcessEvent> {
    list_since_in(&events_path(), cutoff_iso, max)
}

pub fn list_since_in(path: &Path, cutoff_iso: Option<&str>, max: usize) -> Vec<ProcessEvent> {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut events: Vec<ProcessEvent> = raw
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    if let Some(cutoff) = cutoff_iso {
        let cutoff = normalize_iso(cutoff);
        events.retain(|event| normalize_iso(&event.timestamp) >= cutoff);
    }
    if events.len() > max {
        events.drain(..events.len() - max);
    }
    events
}

pub fn clear() -> Result<(), String> {
    clear_in(&events_path())
}

pub fn clear_in(path: &Path) -> Result<(), String> {
    let _guard = WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if path.exists() {
        std::fs::remove_file(path).map_err(|e| format!("删除诊断事件失败: {e}"))?;
    }
    let tmp = path.with_extension("jsonl.tmp");
    if tmp.exists() {
        let _ = std::fs::remove_file(tmp);
    }
    Ok(())
}

pub fn normalize_iso(ts: &str) -> String {
    let ts = ts.trim();
    if ts.len() >= 24 {
        return ts.to_string();
    }
    if ts.ends_with('Z') && ts.len() == 20 {
        return format!("{}.000Z", &ts[..19]);
    }
    ts.to_string()
}

/// 导出 PM4Py 兼容 CSV。前三列使用 PM4Py 约定字段名，其余均为低敏白名单属性。
pub fn to_pm4py_csv(events: &[ProcessEvent], app_version: &str) -> String {
    let mut out = String::from(
        "case:concept:name,concept:name,time:timestamp,eventIndex,appSessionId,appVersion,engineId,modelId,profileId,interactionMode,elevated,outcome,errorCode,durationMs,audioMs,textChars,detail\n",
    );
    for event in events {
        let elevated = event
            .context
            .elevated
            .map(|v| if v { "true" } else { "false" })
            .unwrap_or("");
        let duration_ms = event
            .data
            .duration_ms
            .map(|v| v.to_string())
            .unwrap_or_default();
        let audio_ms = event
            .data
            .audio_ms
            .map(|v| v.to_string())
            .unwrap_or_default();
        let text_chars = event
            .data
            .text_chars
            .map(|v| v.to_string())
            .unwrap_or_default();
        let event_index = event.event_index.to_string();
        let fields = [
            event.case_id.as_str(),
            event.activity.as_str(),
            event.timestamp.as_str(),
            event_index.as_str(),
            event.app_session_id.as_str(),
            app_version,
            event.context.engine_id.as_deref().unwrap_or(""),
            event.context.model_id.as_deref().unwrap_or(""),
            event.context.profile_id.as_deref().unwrap_or(""),
            event.context.interaction_mode.as_deref().unwrap_or(""),
            elevated,
            event.data.outcome.as_deref().unwrap_or(""),
            event.data.error_code.as_deref().unwrap_or(""),
            duration_ms.as_str(),
            audio_ms.as_str(),
            text_chars.as_str(),
            event.data.detail.as_deref().unwrap_or(""),
        ];
        out.push_str(
            &fields
                .iter()
                .map(|value| csv_escape(value))
                .collect::<Vec<_>>()
                .join(","),
        );
        out.push('\n');
    }
    out
}

fn csv_escape(value: &str) -> String {
    if value.contains([',', '"', '\r', '\n']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_roundtrip_and_pm4py_columns() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("events.jsonl");
        let mut event = ProcessEvent::new("session-1", "capture_started");
        event.timestamp = "2026-07-28T12:00:00Z".into();
        event.context.engine_id = Some("x-asr".into());
        event.data.text_chars = Some(12);
        record_in(&path, event).unwrap();

        let events = list_recent_in(&path, 50);
        assert_eq!(events.len(), 1);
        let csv = to_pm4py_csv(&events, "0.1.1");
        assert!(csv.starts_with("case:concept:name,concept:name,time:timestamp"));
        assert!(csv.contains("session-1,capture_started,2026-07-28T12:00:00Z"));
        assert!(csv.contains(",x-asr,"));
        assert!(csv.contains(",detail"));
        assert!(!csv.contains("finalText"));
    }

    #[test]
    fn csv_escapes_dynamic_values() {
        assert_eq!(csv_escape("a,b"), "\"a,b\"");
        assert_eq!(csv_escape("a\"b"), "\"a\"\"b\"");
        assert_eq!(csv_escape("plain"), "plain");
    }

    #[test]
    fn recording_flag_skips_public_record() {
        set_recording_enabled(false);
        record(ProcessEvent::new("s", "hotkey_fired")).unwrap();
        set_recording_enabled(true);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("events.jsonl");
        record_in(&path, ProcessEvent::new("s", "hotkey_fired")).unwrap();
        assert_eq!(list_recent_in(&path, 10).len(), 1);
    }

    #[test]
    fn list_since_keeps_events_on_or_after_cutoff() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("events.jsonl");
        let mut old = ProcessEvent::new("s", "old");
        old.timestamp = "2026-01-01T00:00:00Z".into();
        let mut recent = ProcessEvent::new("s", "recent");
        recent.timestamp = "2026-09-07T12:00:00.000Z".into();
        record_in(&path, old).unwrap();
        record_in(&path, recent).unwrap();
        let kept = list_since_in(&path, Some("2026-09-01T00:00:00Z"), 50);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].activity, "recent");
    }

    #[test]
    fn clear_removes_events_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("events.jsonl");
        record_in(&path, ProcessEvent::new("s", "x")).unwrap();
        assert!(path.exists());
        clear_in(&path).unwrap();
        assert!(!path.exists());
        clear_in(&path).unwrap();
    }

    #[test]
    fn normalize_iso_pads_second_precision() {
        assert_eq!(
            normalize_iso("2026-09-07T12:00:00Z"),
            "2026-09-07T12:00:00.000Z"
        );
        assert_eq!(
            normalize_iso("2026-09-07T12:00:00.123Z"),
            "2026-09-07T12:00:00.123Z"
        );
    }
}
