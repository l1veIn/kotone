//! 工具调度：参数校验、并发/期限控制、窗口引用与产物生命周期。
//!
//! 每次调用在 tokio 阻塞线程池执行（捕获、裁剪、PNG 编码、base64 都不在异步线程或任何
//! 运行时/UI 锁上进行）。并发许可随阻塞任务一起释放，所以超时调用在后台收尾期间仍占用
//! 名额，实际并发始终有界。
//!
//! 绝对期限在三处强制：取得执行名额后、工作线程真正开始时、以及最终提交闸门内。
//! 工作线程只产出“待提交”的结果（新产物或释放请求），从不直接修改产物存储；提交闸门
//! 与关闭（close）共用存储锁，期限已过或服务已关闭的结果一律丢弃，不产生副作用。
//!
//! M2：OCR 推理另有按后端类型划分的有界名额（本地 / 在线），在进入工作线程前于期限内
//! 获取；名额与执行许可都随工作线程一起释放。每次调用带一个取消令牌：期限到达或服务
//! 关闭时取消，本地推理通过 ONNX Runtime 终止、在线请求被丢弃。OCR 结果与跟踪状态同样
//! 只在提交闸门内（各自的锁内核对关闭与期限）写入。

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine as _;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use tokio::sync::Semaphore;

use kotone_core::ocr::{OcrBackend, OcrBackendKind, OcrCancel, OcrError, OcrInput, OcrParams};
use kotone_core::tools::{
    CaptureBackendInfo, WindowCapturer, WindowIdentity, WindowInfo, WindowObserver, WindowToolError,
};

use crate::artifacts::{
    ArtifactKind, ArtifactMeta, ArtifactRecord, ArtifactStore, CaptureSource, Clock, CommitError,
    Gone, LookupError, RectJson, RegionJson, RootTransform, WarningJson, COORDINATE_SPACE,
    MIME_PNG,
};
use crate::catalog::{self, Readiness};
use crate::ids::new_id;
use crate::image::{encode_png, sha256_hex, validate_crop, RgbaImage};
use crate::limits::ServiceLimits;
use crate::ocr_results::{OcrCommitError, OcrLookupError, OcrResults, StoredOcr};
use crate::preprocess;
use crate::protocol::{
    is_valid_request_id, Envelope, ErrorCode, Timing, ToolCallRequest, ToolError,
    PATH_CAPABILITIES, PATH_STATUS, PATH_TOOLS_CALL, PROTOCOL, PROTOCOL_VERSION,
};
use crate::tracking::{
    compact, IngestInput, IngestLine, OnSourceChange, Region, SourceTag, TrackError, Tracker,
};

/// 应用级工具（生命周期、事件、Agent 请求与回复、助手提交）。子模块可访问服务私有状态。
#[path = "service_app.rs"]
mod app;
pub use app::EventReadArgs;

/// 读取 ASR 运行时相位（stopped/starting/running/stopping）。工具服务不依赖它。
pub type AsrPhaseProvider = Arc<dyn Fn() -> String + Send + Sync>;

/// 构造服务所需的依赖。
pub struct ServiceDeps {
    pub instance_id: String,
    pub app_version: String,
    pub observer: Arc<dyn WindowObserver>,
    pub capturer: Arc<dyn WindowCapturer>,
    pub asr_phase: AsrPhaseProvider,
    pub limits: ServiceLimits,
    pub clock: Arc<dyn Clock>,
    /// OCR 后端（M2）：本地与在线可插拔，按 ID 选择，互不降级。可为空。
    pub ocr_backends: Vec<Arc<dyn OcrBackend>>,
    /// 桌面宿主：运行时启停、退出、交付回复、助手提交。没有宿主时用 `NoAppHost`。
    pub host: Arc<dyn crate::host::AppHost>,
}

const MAX_WINDOW_REFS: usize = 4096;

/// 实例内的不透明窗口引用表：同一身份得到同一 ID，ID 不跨实例有效。
struct WindowRefs {
    inner: Mutex<WindowRefsInner>,
}

#[derive(Default)]
struct WindowRefsInner {
    by_id: HashMap<String, WindowIdentity>,
    by_identity: HashMap<WindowIdentity, String>,
    order: VecDeque<String>,
}

impl WindowRefs {
    fn new() -> Self {
        Self {
            inner: Mutex::new(WindowRefsInner::default()),
        }
    }

    fn id_for(&self, identity: WindowIdentity) -> String {
        let mut inner = self.inner.lock().unwrap();
        if let Some(id) = inner.by_identity.get(&identity) {
            return id.clone();
        }
        let id = new_id("win");
        inner.by_id.insert(id.clone(), identity);
        inner.by_identity.insert(identity, id.clone());
        inner.order.push_back(id.clone());
        while inner.order.len() > MAX_WINDOW_REFS {
            if let Some(old) = inner.order.pop_front() {
                if let Some(identity) = inner.by_id.remove(&old) {
                    inner.by_identity.remove(&identity);
                }
            }
        }
        id
    }

    fn resolve(&self, id: &str) -> Option<WindowIdentity> {
        self.inner.lock().unwrap().by_id.get(id).copied()
    }
}

/// 阻塞阶段产出、尚未提交到存储的产物。
struct PendingArtifact {
    kind: ArtifactKind,
    image: RgbaImage,
    png: Vec<u8>,
    sha256: String,
    ttl_ms: u64,
    lineage: Lineage,
    extra: Map<String, Value>,
}

enum Lineage {
    Capture(CaptureSource),
    /// 由父产物派生：子产物点 p 对应父产物点 `offset + p / scale`。
    Derived {
        parent: Box<ArtifactMeta>,
        offset: (f64, f64),
        scale: (f64, f64),
        derivation: Value,
    },
}

/// 识别完成、尚未提交的 OCR 结果。
struct PendingOcr {
    result: Value,
    stored: StoredOcr,
}

enum Prepared {
    Value(Value),
    Artifact(Box<PendingArtifact>),
    /// 释放请求：只在提交闸门内执行，排队超时的释放不会在之后删除产物。
    Release {
        artifact_id: String,
    },
    Ocr(Box<PendingOcr>),
    TrackIngest(Box<(IngestInput, String)>),
    TrackReset {
        session: String,
        reason: String,
    },
    /// 通用提交：闸门（未关闭、未过期）通过后在提交线程执行的短小状态变更。
    Commit(app::CommitFn),
}

/// 单次调用的执行上下文。
struct CallCtx {
    cancel: OcrCancel,
    deadline: std::time::Instant,
}

struct Outcome {
    prepared: Prepared,
    phases: BTreeMap<String, u64>,
}

type ToolResult = Result<Outcome, ToolError>;

pub struct ToolService {
    instance_id: String,
    app_version: String,
    observer: Arc<dyn WindowObserver>,
    capturer: Arc<dyn WindowCapturer>,
    asr_phase: AsrPhaseProvider,
    limits: ServiceLimits,
    store: ArtifactStore,
    windows: WindowRefs,
    calls: Arc<Semaphore>,
    ocr_backends: Vec<Arc<dyn OcrBackend>>,
    local_lane: Arc<Semaphore>,
    online_lane: Arc<Semaphore>,
    ocr_results: OcrResults,
    tracker: Tracker,
    /// 在途调用的取消令牌（关闭时全部取消）。
    inflight: Mutex<HashMap<u64, OcrCancel>>,
    next_call: std::sync::atomic::AtomicU64,
    /// 最近一次就绪检查（后端 ID → 是否就绪）；capabilities 只读这个快照，不在异步线程上做校验。
    ocr_ready: Mutex<HashMap<String, bool>>,
    host: Arc<dyn crate::host::AppHost>,
    events: Arc<crate::events::EventBus>,
    requests: crate::agent_requests::AgentRequests,
    /// 长轮询事件读取的并发上限（不占用工具执行名额）。
    event_waiters: Arc<Semaphore>,
    /// 后台交付中的 Agent 回复名额。
    reply_slots: Arc<Semaphore>,
    started_at_unix_ms: u64,
    started: Instant,
    /// 关闭信号（维护任务订阅）；权威的关闭状态在存储锁内。
    closing: tokio::sync::watch::Sender<bool>,
    maintenance_running: Arc<AtomicBool>,
    /// 测试缝：捕获返回之后、PNG 编码之前执行（模拟慢速处理）。
    #[cfg(test)]
    pub(crate) before_encode_hook: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
}

fn timeout_error(tool: &str, timeout_ms: u64, stage: &str) -> ToolError {
    ToolError::new(
        ErrorCode::Timeout,
        format!("工具 {tool} 未在 {timeout_ms}ms 内完成（{stage}）；结果已丢弃"),
    )
    .with_details(json!({"timeoutMs": timeout_ms, "stage": stage}))
}

fn closed_error() -> ToolError {
    ToolError::new(ErrorCode::ServiceClosed, "工具服务正在关闭或已关闭")
}

fn invalid_args(message: impl Into<String>) -> ToolError {
    ToolError::new(ErrorCode::InvalidArguments, message)
}

fn ocr_error(backend: &str, error: OcrError, tool: &str, timeout_ms: u64) -> ToolError {
    match error {
        OcrError::NotReady { reason, message } => {
            ToolError::new(ErrorCode::OcrBackendNotReady, message)
                .with_details(json!({"backend": backend, "reason": reason}))
        }
        OcrError::UnsupportedLanguage {
            language,
            supported,
        } => ToolError::new(
            ErrorCode::OcrLanguageUnsupported,
            format!("后端 {backend} 不支持语言 {language}"),
        )
        .with_details(json!({"backend": backend, "language": language, "supported": supported})),
        OcrError::UnsupportedOption { option, message } => {
            ToolError::new(ErrorCode::OcrOptionUnsupported, message)
                .with_details(json!({"backend": backend, "option": option}))
        }
        OcrError::InvalidInput(message) => invalid_args(message),
        // 取消只来自期限到达或服务关闭；关闭由调用方另行映射
        OcrError::Cancelled | OcrError::DeadlineExceeded => {
            timeout_error(tool, timeout_ms, "running")
        }
        OcrError::Remote {
            status,
            error_type,
            message,
            retryable,
        } => {
            let mut e = ToolError::new(
                ErrorCode::OcrRemoteError,
                format!("在线 OCR 返回错误: {message}"),
            )
            .with_details(
                json!({"backend": backend, "httpStatus": status, "errorType": error_type}),
            );
            e.retryable = retryable;
            e
        }
        OcrError::Transport { message, retryable } => {
            let mut e = ToolError::new(
                ErrorCode::OcrTransportError,
                format!("在线 OCR 传输失败: {message}"),
            )
            .with_details(json!({"backend": backend}));
            e.retryable = retryable;
            e
        }
        OcrError::ResponseInvalid(message) => {
            ToolError::new(ErrorCode::OcrResponseInvalid, message)
                .with_details(json!({"backend": backend}))
        }
        OcrError::Engine(message) => {
            ToolError::new(ErrorCode::OcrFailed, message).with_details(json!({"backend": backend}))
        }
    }
}

fn track_error(error: TrackError, tool: &str, timeout_ms: u64) -> ToolError {
    match error {
        TrackError::GenerationMismatch { expected, current } => ToolError::new(
            ErrorCode::ChatTrackGenerationMismatch,
            format!("会话代际已是 {current}（请求 {expected}）；请先 read visible 重新同步"),
        )
        .with_details(json!({"expected": expected, "current": current})),
        TrackError::OutOfOrder {
            frame_id,
            captured_at,
            latest_captured_at,
        } => ToolError::new(
            ErrorCode::ChatTrackOutOfOrder,
            format!("帧 {frame_id} 早于会话已接受的最新帧；未改变跟踪状态"),
        )
        .with_details(json!({
            "frameId": frame_id,
            "capturedAtUnixMs": captured_at,
            "latestCapturedAtUnixMs": latest_captured_at
        })),
        TrackError::SourceChanged {
            previous_window,
            window,
        } => ToolError::new(
            ErrorCode::ChatTrackSourceChanged,
            "观测来自另一个窗口；如为重连或新对局请 chat.track.reset，或以 onSourceChange=new-generation 提交",
        )
        .with_details(json!({"previousWindowId": previous_window, "windowId": window})),
        TrackError::SessionNotFound { session, expired } => ToolError::new(
            ErrorCode::ChatTrackSessionNotFound,
            if expired {
                format!("会话 {session} 已因空闲超时被回收")
            } else {
                format!("会话 {session} 不存在")
            },
        )
        .with_details(
            json!({"session": session, "reason": if expired { "expired" } else { "unknown" }}),
        ),
        TrackError::CursorInvalid(message) => {
            ToolError::new(ErrorCode::ChatTrackCursorInvalid, message)
        }
        TrackError::CursorStale {
            cursor_generation,
            current_generation,
            reason,
        } => ToolError::new(
            ErrorCode::ChatTrackCursorStale,
            "游标属于旧代际或已重建的会话；请用 mode=visible 重新同步并使用返回的游标",
        )
        .with_details(json!({
            "cursorGeneration": cursor_generation,
            "currentGeneration": current_generation,
            "reason": reason
        })),
        TrackError::CursorExpired {
            oldest_available_seq,
        } => ToolError::new(
            ErrorCode::ChatTrackCursorExpired,
            "游标之后的部分事件已因容量被回收；请用 mode=visible 重新同步",
        )
        .with_details(json!({"oldestAvailableSeq": oldest_available_seq})),
        TrackError::TooManyLines { lines, max } => {
            invalid_args(format!("观测行数 {lines} 超过上限 {max}"))
        }
        TrackError::Closed => closed_error(),
        TrackError::DeadlineExpired => timeout_error(tool, timeout_ms, "late-result"),
    }
}

fn valid_session_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b':'))
}

fn region_of(meta: &ArtifactMeta) -> Region {
    Region {
        x: meta.root_rect.x,
        y: meta.root_rect.y,
        width: meta.root_rect.width,
        height: meta.root_rect.height,
    }
}

fn map_to_root(meta: &ArtifactMeta, x: f64, y: f64, width: f64, height: f64) -> Region {
    let r = meta.root_transform.map_rect(x, y, width, height);
    Region {
        x: r.x,
        y: r.y,
        width: r.width,
        height: r.height,
    }
}

fn parse_args<T: for<'de> Deserialize<'de>>(arguments: Value) -> Result<T, ToolError> {
    let arguments = if arguments.is_null() {
        json!({})
    } else {
        arguments
    };
    serde_json::from_value(arguments).map_err(|e| invalid_args(format!("参数不合法: {e}")))
}

fn ms_since(start: Instant) -> u64 {
    start.elapsed().as_millis() as u64
}

pub(crate) fn window_error(error: WindowToolError) -> ToolError {
    let code = match &error {
        WindowToolError::Gone => ErrorCode::WindowGone,
        WindowToolError::IdentityChanged => ErrorCode::WindowIdentityChanged,
        WindowToolError::Minimized => ErrorCode::WindowMinimized,
        WindowToolError::NotVisible => ErrorCode::WindowNotVisible,
        WindowToolError::NotForeground => ErrorCode::WindowNotForeground,
        WindowToolError::InvalidGeometry(_) => ErrorCode::InvalidGeometry,
        WindowToolError::TooLarge { .. } => ErrorCode::ImageTooLarge,
        WindowToolError::ChangedDuringCapture(_) => ErrorCode::TargetChanged,
        WindowToolError::Unsupported(_) => ErrorCode::PlatformUnsupported,
        WindowToolError::Platform(_) => ErrorCode::CaptureFailed,
    };
    let mut message = error.to_string();
    if code == ErrorCode::WindowNotForeground {
        message.push_str("；Kotone 不会替调用方切换焦点，请让目标成为前台窗口后重试");
    }
    ToolError::new(code, message)
}

fn lookup_error(id: &str, error: LookupError) -> ToolError {
    match error {
        LookupError::NotFound => ToolError::new(
            ErrorCode::ArtifactNotFound,
            format!("未知产物 {id}（产物只在发放它的实例内有效）"),
        ),
        LookupError::Gone(Gone::Released) => {
            ToolError::new(ErrorCode::ArtifactReleased, format!("产物 {id} 已释放"))
        }
        LookupError::Gone(why) => ToolError::new(
            ErrorCode::ArtifactExpired,
            format!(
                "产物 {id} 已不可用（{}）",
                match why {
                    Gone::Expired => "TTL 到期",
                    _ => "容量回收",
                }
            ),
        )
        .with_details(json!({"reason": if why == Gone::Expired { "ttl" } else { "evicted" }})),
    }
}

fn rect_json(rect: &kotone_core::tools::PhysicalRect) -> RectJson {
    RectJson {
        x: i64::from(rect.x),
        y: i64::from(rect.y),
        width: rect.width,
        height: rect.height,
    }
}

fn window_json(id: &str, info: &WindowInfo) -> Value {
    json!({
        "windowId": id,
        "title": info.title,
        "className": info.class_name,
        "processName": info.process_name,
        "pid": info.identity.pid,
        "isForeground": info.foreground,
        "isVisible": info.visible,
        "isMinimized": info.minimized,
        "isCloaked": info.cloaked,
        "clientRect": info.client_rect.as_ref().map(rect_json),
        "monitorRect": info.monitor_rect.as_ref().map(rect_json),
        "dpi": info.dpi,
        "scale": info.dpi.map(|d| f64::from(d) / 96.0),
        "coordinateSpace": "screen-physical-pixels",
        "identityBinding": if info.identity.process_started_at.is_some() {
            "handle+pid+process-start-time"
        } else {
            "handle+pid"
        },
    })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CapabilitiesArgs {
    protocol_version: Option<u32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NoArgs {}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WindowListArgs {
    process_name: Option<String>,
    title_contains: Option<String>,
    #[serde(default)]
    include_untitled: bool,
    limit: Option<usize>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CaptureArgs {
    window_id: String,
    ttl_ms: Option<u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RectArgs {
    x: i64,
    y: i64,
    width: i64,
    height: i64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CropArgs {
    artifact_id: String,
    rect: RectArgs,
    ttl_ms: Option<u64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ArtifactGetArgs {
    artifact_id: String,
    #[serde(default = "default_true")]
    include_content: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ArtifactReleaseArgs {
    artifact_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PreprocessArgs {
    artifact_id: String,
    preset: Option<String>,
    operations: Option<Vec<preprocess::Operation>>,
    ttl_ms: Option<u64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RecognizeArgs {
    artifact_id: String,
    backend: String,
    language: Option<String>,
    #[serde(default)]
    options: Value,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ExternalLine {
    text: String,
    #[serde(rename = "box")]
    bbox: Option<RectF>,
    confidence: Option<f64>,
}

#[derive(Deserialize, Clone, Copy)]
#[serde(deny_unknown_fields)]
struct RectF {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
enum Observation {
    #[serde(rename_all = "camelCase")]
    OcrResult { ocr_result_id: String },
    #[serde(rename_all = "camelCase")]
    External {
        artifact_id: String,
        producer: String,
        lines: Vec<ExternalLine>,
    },
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct IngestArgs {
    session: String,
    generation: Option<u64>,
    observation: Observation,
    on_source_change: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReadArgs {
    session: String,
    mode: String,
    cursor: Option<String>,
    limit: Option<usize>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ResetArgs {
    session: String,
    reason: String,
}

/// `chat.track.reset` 接受的原因。
pub const RESET_REASONS: &[&str] = &[
    "new-match",
    "reconnect",
    "roi-changed",
    "window-changed",
    "manual",
];

impl ToolService {
    pub fn new(deps: ServiceDeps) -> Arc<Self> {
        let store = ArtifactStore::new(
            deps.clock.clone(),
            deps.limits.max_artifacts,
            deps.limits.max_store_bytes,
        );
        let started_at_unix_ms = deps.clock.now_unix_ms();
        let tracker = Tracker::new(deps.limits.track.clone());
        let ocr_results = OcrResults::new(deps.limits.max_ocr_results);
        Arc::new(Self {
            instance_id: deps.instance_id,
            app_version: deps.app_version,
            observer: deps.observer,
            capturer: deps.capturer,
            asr_phase: deps.asr_phase,
            calls: Arc::new(Semaphore::new(deps.limits.max_concurrent_calls.max(1))),
            local_lane: Arc::new(Semaphore::new(deps.limits.local_ocr_concurrency.max(1))),
            online_lane: Arc::new(Semaphore::new(deps.limits.online_ocr_concurrency.max(1))),
            ocr_backends: deps.ocr_backends,
            ocr_results,
            tracker,
            inflight: Mutex::new(HashMap::new()),
            next_call: std::sync::atomic::AtomicU64::new(1),
            ocr_ready: Mutex::new(HashMap::new()),
            host: deps.host,
            events: Arc::new(crate::events::EventBus::new(deps.limits.max_events)),
            requests: crate::agent_requests::AgentRequests::new(
                deps.limits.agent_request_ttl_ms,
                deps.limits.max_agent_requests,
            ),
            event_waiters: Arc::new(Semaphore::new(deps.limits.max_event_waiters.max(1))),
            reply_slots: Arc::new(Semaphore::new(deps.limits.max_pending_replies.max(1))),
            limits: deps.limits,
            store,
            windows: WindowRefs::new(),
            started_at_unix_ms,
            started: Instant::now(),
            closing: tokio::sync::watch::channel(false).0,
            maintenance_running: Arc::new(AtomicBool::new(false)),
            #[cfg(test)]
            before_encode_hook: Mutex::new(None),
        })
    }

    pub fn instance_id(&self) -> &str {
        &self.instance_id
    }

    pub fn app_version(&self) -> &str {
        &self.app_version
    }

    /// 服务时钟的当前 Unix 毫秒（测试里是手动时钟）。
    pub fn now_unix_ms(&self) -> u64 {
        self.store.clock().now_unix_ms()
    }

    pub fn limits(&self) -> &ServiceLimits {
        &self.limits
    }

    /// 关闭：在存储锁内撤销提交并丢弃全部内存产物，然后停止维护任务。
    /// 返回后不会再有任何请求向存储发布产物或执行释放；仍在运行的原生工作线程可以
    /// 完成自身清理，但其结果在提交闸门被拒绝。
    pub fn close(&self) {
        // 通知订阅者服务即将停止，然后关闭总线（唤醒长轮询）
        self.publish_event("app.stopping", json!({"instanceId": self.instance_id}));
        self.events.close();
        self.requests.close();
        self.store.close();
        self.ocr_results.close();
        self.tracker.close();
        // 取消在途推理/在线请求；它们的结果在提交闸门被拒绝
        let cancels: Vec<OcrCancel> = self.inflight.lock().unwrap().values().cloned().collect();
        for cancel in cancels {
            cancel.cancel();
        }
        self.closing.send_replace(true);
    }

    /// 刷新 OCR 后端就绪快照（可能做文件哈希与凭据读取；只在阻塞线程调用）。
    pub fn refresh_ocr_readiness(&self) -> Vec<kotone_core::ocr::OcrBackendInfo> {
        let infos: Vec<_> = self.ocr_backends.iter().map(|b| b.info()).collect();
        let mut ready = self.ocr_ready.lock().unwrap();
        for info in &infos {
            ready.insert(info.id.clone(), info.ready);
        }
        infos
    }

    /// 测试观测：驻留的 OCR 结果数与跟踪会话数。
    #[doc(hidden)]
    pub fn resident_ocr_and_sessions(&self) -> (usize, usize) {
        (self.ocr_results.len(), self.tracker.session_count())
    }

    pub fn is_closed(&self) -> bool {
        self.store.is_closed()
    }

    /// 测试观测：驻留内存的产物数/字节（不触发到期清理）。
    #[doc(hidden)]
    pub fn resident_artifacts(&self) -> (usize, u64) {
        self.store.resident_without_purge()
    }

    /// 维护任务是否仍在运行。
    pub fn maintenance_running(&self) -> bool {
        self.maintenance_running.load(Ordering::SeqCst)
    }

    /// 启动有界的空闲维护：按 `limits.maintenance_interval` 回收到期产物（不依赖读请求）。
    /// 任务只持有弱引用，服务关闭或被释放时退出。
    pub fn spawn_maintenance(self: &Arc<Self>) -> tokio::task::JoinHandle<()> {
        let weak = Arc::downgrade(self);
        let mut stop = self.closing.subscribe();
        let period = self
            .limits
            .maintenance_interval
            .max(Duration::from_millis(1));
        // 复位守卫在 spawn 之前构造并被 future 捕获：即使任务在首次 poll 之前就被
        // abort / 运行时 shutdown 丢弃，守卫也会随 future 一起析构并复位标志。
        struct Running(Arc<AtomicBool>);
        impl Drop for Running {
            fn drop(&mut self) {
                self.0.store(false, Ordering::SeqCst);
            }
        }
        self.maintenance_running.store(true, Ordering::SeqCst);
        let running = Running(self.maintenance_running.clone());
        tokio::spawn(async move {
            let _running = running;
            let mut ticker = tokio::time::interval_at(tokio::time::Instant::now() + period, period);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                if *stop.borrow() {
                    break;
                }
                tokio::select! {
                    _ = ticker.tick() => {
                        let Some(service) = weak.upgrade() else { break };
                        if service.store.is_closed() {
                            break;
                        }
                        service.store.purge_expired();
                        let now = service.store.now();
                        service.ocr_results.purge(now);
                        service.tracker.purge_idle(now);
                        service.expire_agent_requests(now);
                    }
                    changed = stop.changed() => {
                        if changed.is_err() {
                            break;
                        }
                    }
                }
            }
        })
    }

    fn capture_info(&self) -> CaptureBackendInfo {
        self.capturer.backend_info()
    }

    fn readiness(&self, capture: &CaptureBackendInfo) -> Readiness {
        let ready = self.ocr_ready.lock().unwrap();
        let any_ready = self
            .ocr_backends
            .iter()
            .any(|b| ready.get(b.id()).copied().unwrap_or(false));
        let checked = self.ocr_backends.iter().all(|b| ready.contains_key(b.id()));
        Readiness {
            windows_ready: capture.available,
            capture_ready: capture.available,
            reason: capture.unavailable_reason.clone(),
            host_tools_ready: self.host.supports_invoke(),
            ocr_ready: any_ready,
            ocr_reason: if any_ready {
                None
            } else if self.ocr_backends.is_empty() {
                Some("本实例没有配置 OCR 后端".into())
            } else if !checked {
                Some("尚未完成后端就绪检查；调用 ocr.backends 查看".into())
            } else {
                Some("没有就绪的 OCR 后端；调用 ocr.backends 查看原因（本地模型需 `kotone-cli ocr install`）".into())
            },
        }
    }

    pub fn capabilities(&self) -> Value {
        let capture = self.capture_info();
        let readiness = self.readiness(&capture);
        json!({
            "protocol": PROTOCOL,
            "protocolVersion": PROTOCOL_VERSION,
            "instanceId": self.instance_id,
            "appVersion": self.app_version,
            "platform": std::env::consts::OS,
            "endpoints": [
                format!("GET {PATH_CAPABILITIES}"),
                format!("GET {PATH_STATUS}"),
                format!("POST {PATH_TOOLS_CALL}"),
                format!("GET {}", crate::protocol::PATH_EVENTS),
            ],
            "capture": {
                "method": capture.method,
                "available": capture.available,
                "unavailableReason": capture.unavailable_reason,
                "limitations": capture.limitations,
            },
            "limits": self.limits.to_json(),
            "ocr": {
                "backends": self.ocr_backends.iter().map(|b| {
                    let ready = self.ocr_ready.lock().unwrap().get(b.id()).copied();
                    json!({
                        "id": b.id(),
                        "kind": b.kind().as_str(),
                        "dataEgress": if b.kind() == OcrBackendKind::Online { "selected-image" } else { "none" },
                        "lastKnownReady": ready,
                    })
                }).collect::<Vec<_>>(),
                "fallback": "none",
            },
            "tools": catalog::tool_specs(&self.limits, &readiness),
        })
    }

    pub fn status(&self) -> Value {
        let capture = self.capture_info();
        let phase = (self.asr_phase)();
        let phase = match phase.as_str() {
            "stopped" | "starting" | "running" | "stopping" => phase,
            _ => "unknown".to_string(),
        };
        json!({
            "instanceId": self.instance_id,
            "appVersion": self.app_version,
            "protocol": PROTOCOL,
            "protocolVersion": PROTOCOL_VERSION,
            "platform": std::env::consts::OS,
            "service": {
                "state": "running",
                "startedAtUnixMs": self.started_at_unix_ms,
                "uptimeMs": self.started.elapsed().as_millis() as u64,
            },
            "asr": {"runtimePhase": phase, "requiredForTools": false},
            "capture": {"method": capture.method, "available": capture.available},
            "artifacts": self.store.stats(),
            "ocrResults": self.ocr_results.len(),
            "chatTrackSessions": self.tracker.session_count(),
            "events": {"headSeq": self.events.head(), "retained": self.events.len()},
            "agentRequests": self.requests.len(),
            "activeCalls": self.limits.max_concurrent_calls.max(1) - self.calls.available_permits(),
        })
    }

    fn envelope_failure(
        &self,
        request_id: String,
        tool: Option<String>,
        error: ToolError,
        started: Instant,
    ) -> Envelope {
        Envelope::failure(
            request_id,
            Some(self.instance_id.clone()),
            tool,
            error,
            Timing {
                total_ms: ms_since(started),
                ..Default::default()
            },
        )
    }

    /// 执行一次工具调用并返回信封（从不 panic 到调用方）。
    pub async fn call(self: &Arc<Self>, request: ToolCallRequest) -> Envelope {
        let started = Instant::now();
        let request_id = match &request.request_id {
            None => new_id("req"),
            Some(id) if is_valid_request_id(id) => id.clone(),
            Some(_) => {
                return self.envelope_failure(
                    new_id("req"),
                    Some(request.tool),
                    ToolError::new(
                        ErrorCode::InvalidRequest,
                        "requestId 只能包含 1-128 个字母、数字或 - _ . :",
                    ),
                    started,
                );
            }
        };
        let tool = request.tool;
        if self.store.is_closed() {
            return self.envelope_failure(request_id, Some(tool), closed_error(), started);
        }
        let implemented = catalog::implemented_tools();
        if !implemented.contains(&tool.as_str()) {
            let error = ToolError::new(
                ErrorCode::ToolNotFound,
                format!("工具 {tool} 未在本实例实现；可用工具见 system.capabilities"),
            )
            .with_details(json!({"available": implemented}));
            return self.envelope_failure(request_id, Some(tool), error, started);
        }
        let timeout_ms = match request.timeout_ms {
            None => self.limits.default_timeout_ms,
            Some(ms) if ms >= 1 && ms <= self.limits.max_timeout_ms => ms,
            Some(ms) => {
                let error = invalid_args(format!(
                    "timeoutMs={ms} 超出允许范围 1-{}",
                    self.limits.max_timeout_ms
                ));
                return self.envelope_failure(request_id, Some(tool), error, started);
            }
        };
        let deadline = tokio::time::Instant::now() + Duration::from_millis(timeout_ms);

        // 事件读取（可长轮询）是纯异步内存操作：不占工具执行名额，另有等待者上限
        if tool == catalog::TOOL_EVENT_READ {
            return self
                .event_read_envelope(request_id, request.arguments, timeout_ms, started)
                .await;
        }

        let permit =
            match tokio::time::timeout_at(deadline, self.calls.clone().acquire_owned()).await {
                Ok(Ok(permit)) => permit,
                _ => {
                    let error = ToolError::new(
                        ErrorCode::Busy,
                        format!(
                            "并发调用已满（上限 {}），在期限内未获得执行名额",
                            self.limits.max_concurrent_calls
                        ),
                    );
                    return self.envelope_failure(request_id, Some(tool), error, started);
                }
            };
        // OCR 名额：按所选后端类型在期限内获取（未知后端不占名额，由工作线程拒绝）
        let lane = if tool == catalog::TOOL_OCR_RECOGNIZE {
            request
                .arguments
                .get("backend")
                .and_then(Value::as_str)
                .and_then(|id| self.ocr_backends.iter().find(|b| b.id() == id))
                .map(|b| match b.kind() {
                    OcrBackendKind::Local => ("local", self.local_lane.clone()),
                    OcrBackendKind::Online => ("online", self.online_lane.clone()),
                })
        } else {
            None
        };
        let lane_permit = match lane {
            None => None,
            Some((name, semaphore)) => {
                match tokio::time::timeout_at(deadline, semaphore.acquire_owned()).await {
                    Ok(Ok(p)) => Some(p),
                    _ => {
                        drop(permit);
                        let error = ToolError::new(
                            ErrorCode::Busy,
                            format!("{name} OCR 名额已满，在期限内未获得执行名额"),
                        )
                        .with_details(json!({"lane": name}));
                        return self.envelope_failure(request_id, Some(tool), error, started);
                    }
                }
            }
        };
        let deadline_std = deadline.into_std();
        // 闸门 1：取得名额时期限可能已过（排队耗尽了时间）
        if std::time::Instant::now() >= deadline_std {
            drop(permit);
            drop(lane_permit);
            let error = timeout_error(&tool, timeout_ms, "queued");
            return self.envelope_failure(request_id, Some(tool), error, started);
        }
        let queue_ms = ms_since(started);
        let service = self.clone();
        let tool_name = tool.clone();
        let arguments = request.arguments;
        let cancel = OcrCancel::new();
        let call_id = self
            .next_call
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.inflight
            .lock()
            .unwrap()
            .insert(call_id, cancel.clone());
        // 关闭可能恰好发生在登记之前：登记后再核对一次
        if self.store.is_closed() {
            cancel.cancel();
        }
        let worker_cancel = cancel.clone();
        let work = tokio::task::spawn_blocking(move || {
            // 名额与许可随工作线程一起释放：超时的调用在真正结束前仍占名额
            let _permit = permit;
            let _lane_permit = lane_permit;
            // 取消登记随工作线程结束注销（包括 panic 展开）
            struct Inflight(Arc<ToolService>, u64);
            impl Drop for Inflight {
                fn drop(&mut self) {
                    self.0.inflight.lock().unwrap().remove(&self.1);
                }
            }
            let _inflight = Inflight(service.clone(), call_id);
            (|| {
                // 闸门 2：阻塞线程池可能饱和，任务真正开始时再核对期限与关闭状态
                if std::time::Instant::now() >= deadline_std {
                    return Err(timeout_error(&tool_name, timeout_ms, "not-started"));
                }
                if service.store.is_closed() || worker_cancel.is_cancelled() {
                    return Err(closed_error());
                }
                let ctx = CallCtx {
                    cancel: worker_cancel,
                    deadline: deadline_std,
                };
                service.execute(&tool_name, arguments, &ctx, timeout_ms)
            })()
        });
        let outcome = match tokio::time::timeout_at(deadline, work).await {
            Err(_) => {
                // 期限到达：通知工作线程尽快停止（本地推理终止、在线请求丢弃）
                cancel.cancel();
                Err(timeout_error(&tool, timeout_ms, "running"))
            }
            Ok(Err(join)) => Err(ToolError::new(
                ErrorCode::Internal,
                if join.is_panic() {
                    "工具执行异常终止".to_string()
                } else {
                    "工具执行被取消".to_string()
                },
            )),
            Ok(Ok(result)) => result,
        };
        // 闸门 3：timeout_at 在计时器与内部 future 同时就绪时会返回内部结果，
        // 因此提交闸门在存储锁内重新核对绝对期限。
        let outcome =
            outcome.and_then(|outcome| self.commit(outcome, deadline_std, &tool, timeout_ms));
        // 服务在执行期间关闭：工作线程的取消/超时错误统一报告为 SERVICE_CLOSED
        let outcome = match outcome {
            Err(e) if self.store.is_closed() && e.code != ErrorCode::ServiceClosed.as_str() => {
                Err(closed_error())
            }
            other => other,
        };
        match outcome {
            Ok((result, mut phases)) => {
                phases.insert("queueMs".into(), queue_ms);
                Envelope::success(
                    request_id,
                    Some(self.instance_id.clone()),
                    Some(tool),
                    result,
                    Timing {
                        total_ms: ms_since(started),
                        phases,
                    },
                )
            }
            Err(error) => self.envelope_failure(request_id, Some(tool), error, started),
        }
    }

    /// 提交闸门：把待提交结果写入存储并生成结果 JSON（只做内存操作）。
    /// 存储变更与“未关闭、未过期”检查在同一把存储锁内完成。
    fn commit(
        &self,
        outcome: Outcome,
        deadline: std::time::Instant,
        tool: &str,
        timeout_ms: u64,
    ) -> Result<(Value, BTreeMap<String, u64>), ToolError> {
        let gate_error = |error: CommitError| match error {
            CommitError::Closed => closed_error(),
            CommitError::DeadlineExpired => timeout_error(tool, timeout_ms, "late-result"),
            CommitError::TooLarge { bytes, max_bytes } => ToolError::new(
                ErrorCode::ImageTooLarge,
                format!("产物占用 {bytes} 字节，超过存储上限 {max_bytes}"),
            ),
            CommitError::Lookup(error) => {
                ToolError::new(ErrorCode::ArtifactNotFound, format!("{error:?}"))
            }
        };
        let Outcome { prepared, phases } = outcome;
        let pending = match prepared {
            Prepared::Value(value) => {
                if self.store.is_closed() {
                    return Err(closed_error());
                }
                if std::time::Instant::now() >= deadline {
                    return Err(timeout_error(tool, timeout_ms, "late-result"));
                }
                return Ok((value, phases));
            }
            Prepared::Release { artifact_id } => {
                let outcome = self
                    .store
                    .release_gated(&artifact_id, Some(deadline))
                    .map_err(|error| match error {
                        CommitError::Lookup(lookup) => lookup_error(&artifact_id, lookup),
                        other => gate_error(other),
                    })?;
                return Ok((
                    json!({"artifactId": artifact_id, "status": outcome.as_str()}),
                    phases,
                ));
            }
            Prepared::Ocr(pending) => {
                let PendingOcr { mut result, stored } = *pending;
                let id = stored.id.clone();
                let expires = stored.expires_at_unix_ms;
                self.ocr_results
                    .insert_gated(stored, self.store.now(), deadline)
                    .map_err(|e| match e {
                        OcrCommitError::Closed => closed_error(),
                        OcrCommitError::DeadlineExpired => {
                            timeout_error(tool, timeout_ms, "late-result")
                        }
                    })?;
                result["ocrResultId"] = json!(id);
                result["ocrResultExpiresAtUnixMs"] = json!(expires);
                return Ok((result, phases));
            }
            Prepared::TrackIngest(input) => {
                let (input, hash) = *input;
                let now = self.store.now();
                let value = self
                    .tracker
                    .ingest(input, hash, now, || self.track_gate(deadline))
                    .map_err(|e| track_error(e, tool, timeout_ms))?;
                return Ok((value, phases));
            }
            Prepared::TrackReset { session, reason } => {
                let now = self.store.now();
                let value = self
                    .tracker
                    .reset(&session, &reason, now, || self.track_gate(deadline))
                    .map_err(|e| track_error(e, tool, timeout_ms))?;
                return Ok((value, phases));
            }
            Prepared::Commit(commit) => {
                if self.store.is_closed() {
                    return Err(closed_error());
                }
                if std::time::Instant::now() >= deadline {
                    return Err(timeout_error(tool, timeout_ms, "late-result"));
                }
                return Ok((commit(self)?, phases));
            }
            Prepared::Artifact(pending) => *pending,
        };
        let artifact_id = new_id("art");
        let now = self.store.now();
        let (width, height) = (pending.image.width(), pending.image.height());
        let (parent_artifact_id, root_artifact_id, root_transform, source, derivation) =
            match pending.lineage {
                Lineage::Capture(source) => (
                    None,
                    artifact_id.clone(),
                    RootTransform::IDENTITY,
                    source,
                    Value::Null,
                ),
                Lineage::Derived {
                    parent,
                    offset,
                    scale,
                    derivation,
                } => (
                    Some(parent.artifact_id.clone()),
                    parent.root_artifact_id.clone(),
                    parent.root_transform.compose(offset, scale),
                    parent.source.clone(),
                    derivation,
                ),
            };
        let root_rect = root_transform.map_rect(0.0, 0.0, f64::from(width), f64::from(height));
        // 根产物即采集时的客户区：屏幕位置 = 客户区屏幕原点 + 根坐标
        let screen_rect = RegionJson {
            x: source.client_rect_screen.x as f64 + root_rect.x,
            y: source.client_rect_screen.y as f64 + root_rect.y,
            width: root_rect.width,
            height: root_rect.height,
        };
        let png: Arc<[u8]> = pending.png.into();
        let meta = ArtifactMeta {
            artifact_id,
            kind: pending.kind,
            mime_type: MIME_PNG,
            width,
            height,
            byte_length: png.len(),
            sha256: pending.sha256,
            created_at_unix_ms: now,
            expires_at_unix_ms: now.saturating_add(pending.ttl_ms),
            ttl_ms: pending.ttl_ms,
            parent_artifact_id,
            root_artifact_id,
            coordinate_space: COORDINATE_SPACE,
            root_transform,
            root_rect,
            screen_rect,
            source,
            derivation,
        };
        let mut result = serde_json::to_value(&meta)
            .map_err(|e| ToolError::new(ErrorCode::Internal, format!("序列化产物失败: {e}")))?;
        let evicted = self
            .store
            .insert_gated(
                ArtifactRecord {
                    meta,
                    image: pending.image,
                    png,
                },
                Some(deadline),
            )
            .map_err(gate_error)?;
        if let Some(object) = result.as_object_mut() {
            for (k, v) in pending.extra {
                object.insert(k, v);
            }
            object.insert("evictedArtifactIds".into(), json!(evicted));
        }
        Ok((result, phases))
    }

    fn value(value: Value) -> ToolResult {
        Ok(Outcome {
            prepared: Prepared::Value(value),
            phases: BTreeMap::new(),
        })
    }

    fn ttl(&self, requested: Option<u64>) -> Result<u64, ToolError> {
        match requested {
            None => Ok(self.limits.default_ttl_ms),
            Some(ms) if (self.limits.min_ttl_ms..=self.limits.max_ttl_ms).contains(&ms) => Ok(ms),
            Some(ms) => Err(invalid_args(format!(
                "ttlMs={ms} 超出允许范围 {}-{}",
                self.limits.min_ttl_ms, self.limits.max_ttl_ms
            ))),
        }
    }

    /// 跟踪状态的提交闸门：在跟踪器锁内调用，核对服务关闭与绝对期限。
    fn track_gate(&self, deadline: std::time::Instant) -> Result<(), TrackError> {
        if self.store.is_closed() {
            Err(TrackError::Closed)
        } else if std::time::Instant::now() >= deadline {
            Err(TrackError::DeadlineExpired)
        } else {
            Ok(())
        }
    }

    /// 阻塞线程上执行具体工具。
    fn execute(&self, tool: &str, arguments: Value, ctx: &CallCtx, timeout_ms: u64) -> ToolResult {
        match tool {
            catalog::TOOL_SYSTEM_CAPABILITIES => {
                let args: CapabilitiesArgs = parse_args(arguments)?;
                if let Some(version) = args.protocol_version {
                    if version != PROTOCOL_VERSION {
                        return Err(ToolError::new(
                            ErrorCode::UnsupportedProtocolVersion,
                            format!("本实例只实现协议版本 {PROTOCOL_VERSION}（请求 {version}）"),
                        ));
                    }
                }
                Self::value(self.capabilities())
            }
            catalog::TOOL_SYSTEM_STATUS => {
                let _: NoArgs = parse_args(arguments)?;
                Self::value(self.status())
            }
            catalog::TOOL_WINDOW_LIST => self.window_list(parse_args(arguments)?),
            catalog::TOOL_WINDOW_ACTIVE => {
                let _: NoArgs = parse_args(arguments)?;
                let active = self.observer.foreground_window().map_err(window_error)?;
                Self::value(match active {
                    Some(info) => {
                        let id = self.windows.id_for(info.identity);
                        json!({"window": window_json(&id, &info), "reason": null})
                    }
                    None => json!({"window": null, "reason": "NO_FOREGROUND_WINDOW"}),
                })
            }
            catalog::TOOL_SCREEN_CAPTURE => self.capture(parse_args(arguments)?),
            catalog::TOOL_IMAGE_CROP => self.crop(parse_args(arguments)?),
            catalog::TOOL_ARTIFACT_GET => self.artifact_get(parse_args(arguments)?),
            catalog::TOOL_ARTIFACT_RELEASE => {
                // 工作线程只解析参数；真正的释放在提交闸门内执行
                let args: ArtifactReleaseArgs = parse_args(arguments)?;
                Ok(Outcome {
                    prepared: Prepared::Release {
                        artifact_id: args.artifact_id,
                    },
                    phases: BTreeMap::new(),
                })
            }
            catalog::TOOL_APP_STATUS
            | catalog::TOOL_APP_QUIT
            | catalog::TOOL_RUNTIME_STATUS
            | catalog::TOOL_RUNTIME_START
            | catalog::TOOL_RUNTIME_STOP
            | catalog::TOOL_AGENT_REQUESTS
            | catalog::TOOL_AGENT_REQUEST_CLAIM
            | catalog::TOOL_AGENT_REPLY
            | catalog::TOOL_ASSISTANT_SUBMIT => self.execute_app(tool, arguments, ctx, timeout_ms),
            t if catalog::HOST_TOOLS.contains(&t) => {
                self.execute_host(tool, arguments, ctx, timeout_ms)
            }
            catalog::TOOL_IMAGE_PREPROCESS => self.preprocess(parse_args(arguments)?),
            catalog::TOOL_OCR_BACKENDS => {
                let _: NoArgs = parse_args(arguments)?;
                let infos = self.refresh_ocr_readiness();
                Self::value(json!({
                    "backends": infos.iter().map(backend_json).collect::<Vec<_>>(),
                    "fallback": "none",
                }))
            }
            catalog::TOOL_OCR_RECOGNIZE => self.recognize(parse_args(arguments)?, ctx, timeout_ms),
            catalog::TOOL_CHAT_TRACK_INGEST => self.track_ingest(parse_args(arguments)?),
            catalog::TOOL_CHAT_TRACK_READ => {
                self.track_read(parse_args(arguments)?, tool, timeout_ms)
            }
            catalog::TOOL_CHAT_TRACK_RESET => {
                let args: ResetArgs = parse_args(arguments)?;
                if !valid_session_name(&args.session) {
                    return Err(invalid_args(SESSION_RULE));
                }
                if !RESET_REASONS.contains(&args.reason.as_str()) {
                    return Err(invalid_args(format!(
                        "reason 只接受 {}",
                        RESET_REASONS.join(" / ")
                    )));
                }
                Ok(Outcome {
                    prepared: Prepared::TrackReset {
                        session: args.session,
                        reason: args.reason,
                    },
                    phases: BTreeMap::new(),
                })
            }
            other => Err(ToolError::new(
                ErrorCode::ToolNotFound,
                format!("工具 {other} 未实现"),
            )),
        }
    }

    fn window_list(&self, args: WindowListArgs) -> ToolResult {
        let limit = args.limit.unwrap_or(64);
        if limit == 0 || limit > self.limits.max_window_list {
            return Err(invalid_args(format!(
                "limit 必须在 1-{} 之间",
                self.limits.max_window_list
            )));
        }
        let process = args.process_name.map(|s| s.to_lowercase());
        let title = args.title_contains.map(|s| s.to_lowercase());
        let windows = self.observer.list_windows().map_err(window_error)?;
        let matched: Vec<WindowInfo> = windows
            .into_iter()
            .filter(|w| args.include_untitled || !w.title.trim().is_empty())
            .filter(|w| match &process {
                Some(p) => w.process_name.as_deref().map(str::to_lowercase).as_ref() == Some(p),
                None => true,
            })
            .filter(|w| match &title {
                Some(t) => w.title.to_lowercase().contains(t),
                None => true,
            })
            .collect();
        let total = matched.len();
        let items: Vec<Value> = matched
            .iter()
            .take(limit)
            .map(|info| window_json(&self.windows.id_for(info.identity), info))
            .collect();
        Self::value(json!({"windows": items, "matched": total, "truncated": total > limit}))
    }

    fn resolve_window(&self, window_id: &str) -> Result<WindowIdentity, ToolError> {
        self.windows.resolve(window_id).ok_or_else(|| {
            ToolError::new(
                ErrorCode::WindowNotFound,
                format!("未知窗口引用 {window_id}；请先调用 window.list 或 window.active"),
            )
        })
    }

    fn capture(&self, args: CaptureArgs) -> ToolResult {
        let ttl_ms = self.ttl(args.ttl_ms)?;
        let identity = self.resolve_window(&args.window_id)?;
        let backend = self.capturer.backend_info();
        if !backend.available {
            return Err(ToolError::new(
                ErrorCode::PlatformUnsupported,
                backend
                    .unavailable_reason
                    .unwrap_or_else(|| "捕获后端不可用".into()),
            ));
        }
        let info = self.observer.inspect(&identity).map_err(window_error)?;
        let capture_start = Instant::now();
        let captured = self
            .capturer
            .capture_foreground_client(&identity, self.limits.max_image_pixels)
            .map_err(window_error)?;
        // 采集时刻取自后端返回的边界，早于像素校验、PNG 编码与哈希
        let captured_at_unix_ms = self.store.now();
        let capture_ms = ms_since(capture_start);
        if u64::from(captured.width) * u64::from(captured.height) > self.limits.max_image_pixels {
            return Err(ToolError::new(
                ErrorCode::ImageTooLarge,
                "捕获后端返回的图像超过像素上限",
            ));
        }
        let image =
            RgbaImage::new(captured.width, captured.height, captured.rgba).map_err(|e| {
                ToolError::new(
                    ErrorCode::CaptureFailed,
                    format!("捕获后端返回无效像素: {e}"),
                )
            })?;
        #[cfg(test)]
        {
            let hook = self.before_encode_hook.lock().unwrap().clone();
            if let Some(hook) = hook {
                hook();
            }
        }
        let encode_start = Instant::now();
        let png =
            encode_png(&image).map_err(|e| ToolError::new(ErrorCode::Internal, e.to_string()))?;
        let sha256 = sha256_hex(&png);
        let encode_ms = ms_since(encode_start);
        let window_id = args.window_id;
        let source = CaptureSource {
            frame_id: new_id("frm"),
            window_id: window_id.clone(),
            capture_method: backend.method,
            client_rect_screen: rect_json(&captured.client_rect),
            dpi: captured.dpi,
            captured_at_unix_ms,
            process_name: info.process_name.clone(),
            warnings: captured
                .warnings
                .into_iter()
                .map(|w| WarningJson {
                    code: w.code,
                    message: w.message,
                })
                .collect(),
        };
        let mut extra = Map::new();
        extra.insert("window".into(), window_json(&window_id, &info));
        let mut phases = BTreeMap::new();
        phases.insert("captureMs".into(), capture_ms);
        phases.insert("encodeMs".into(), encode_ms);
        Ok(Outcome {
            prepared: Prepared::Artifact(Box::new(PendingArtifact {
                kind: ArtifactKind::Capture,
                image,
                png,
                sha256,
                ttl_ms,
                lineage: Lineage::Capture(source),
                extra,
            })),
            phases,
        })
    }

    fn crop(&self, args: CropArgs) -> ToolResult {
        let ttl_ms = self.ttl(args.ttl_ms)?;
        let parent = self
            .store
            .get(&args.artifact_id)
            .map_err(|e| lookup_error(&args.artifact_id, e))?;
        let (w, h) = (parent.image.width(), parent.image.height());
        let rect = validate_crop(
            w,
            h,
            args.rect.x,
            args.rect.y,
            args.rect.width,
            args.rect.height,
        )
        .map_err(|e| {
            invalid_args(e.to_string()).with_details(json!({
                "reason": e.reason(),
                "artifactWidth": w,
                "artifactHeight": h,
            }))
        })?;
        let crop_start = Instant::now();
        let image = parent.image.crop(rect);
        let crop_ms = ms_since(crop_start);
        let encode_start = Instant::now();
        let png =
            encode_png(&image).map_err(|e| ToolError::new(ErrorCode::Internal, e.to_string()))?;
        let sha256 = sha256_hex(&png);
        let encode_ms = ms_since(encode_start);
        let mut phases = BTreeMap::new();
        phases.insert("cropMs".into(), crop_ms);
        phases.insert("encodeMs".into(), encode_ms);
        Ok(Outcome {
            prepared: Prepared::Artifact(Box::new(PendingArtifact {
                kind: ArtifactKind::Crop,
                image,
                png,
                sha256,
                ttl_ms,
                lineage: Lineage::Derived {
                    parent: Box::new(parent.meta.clone()),
                    offset: (f64::from(rect.x), f64::from(rect.y)),
                    scale: (1.0, 1.0),
                    derivation: json!({"type": "crop", "rect": {
                        "x": rect.x, "y": rect.y, "width": rect.width, "height": rect.height
                    }}),
                },
                extra: Map::new(),
            })),
            phases,
        })
    }

    fn artifact_get(&self, args: ArtifactGetArgs) -> ToolResult {
        let record = self
            .store
            .get(&args.artifact_id)
            .map_err(|e| lookup_error(&args.artifact_id, e))?;
        let mut result = serde_json::to_value(&record.meta)
            .map_err(|e| ToolError::new(ErrorCode::Internal, e.to_string()))?;
        let mut phases = BTreeMap::new();
        if args.include_content {
            let start = Instant::now();
            let data = base64::engine::general_purpose::STANDARD.encode(&record.png);
            phases.insert("encodeMs".into(), ms_since(start));
            result["content"] = json!({"encoding": "base64", "mimeType": MIME_PNG, "data": data});
        }
        Ok(Outcome {
            prepared: Prepared::Value(result),
            phases,
        })
    }
}

const SESSION_RULE: &str = "session 只能包含 1-64 个字母、数字或 - _ . :";

fn backend_json(info: &kotone_core::ocr::OcrBackendInfo) -> Value {
    json!({
        "id": info.id,
        "kind": info.kind.as_str(),
        "displayName": info.display_name,
        "ready": info.ready,
        "state": info.state,
        "reason": info.reason,
        "languages": info.languages,
        "defaultLanguage": info.default_language,
        "dataEgress": info.data_egress,
        "optionsSchema": info.options_schema,
        "details": info.details,
    })
}

fn region_value(r: &Region) -> Value {
    r.to_json()
}

/// M2 工具（预处理、OCR、聊天跟踪）的工作线程实现。
impl ToolService {
    fn preprocess(&self, args: PreprocessArgs) -> ToolResult {
        let ttl_ms = self.ttl(args.ttl_ms)?;
        let (preset, operations) = match (args.preset, args.operations) {
            (Some(_), Some(_)) => return Err(invalid_args("preset 与 operations 只能二选一")),
            (None, None) => {
                return Err(invalid_args(
                    "需要 preset 或 operations（不处理请用 preset=none）",
                ))
            }
            (Some(name), None) => match preprocess::preset_operations(&name) {
                Some(ops) => (Some(name), ops),
                None => {
                    return Err(invalid_args(format!(
                        "未知预设 {name}（支持 {}）",
                        preprocess::PRESETS.join(" / ")
                    )))
                }
            },
            (None, Some(ops)) => {
                if ops.is_empty() {
                    return Err(invalid_args(
                        "operations 不能为空（不处理请用 preset=none）",
                    ));
                }
                (None, ops)
            }
        };
        let parent = self
            .store
            .get(&args.artifact_id)
            .map_err(|e| lookup_error(&args.artifact_id, e))?;
        let ops_json: Vec<Value> = operations.iter().map(|o| o.to_json()).collect();
        if operations.is_empty() {
            // preset=none：不生成新产物，原样返回输入产物元数据
            let mut result = serde_json::to_value(&parent.meta)
                .map_err(|e| ToolError::new(ErrorCode::Internal, e.to_string()))?;
            result["created"] = json!(false);
            result["preprocess"] =
                json!({"preset": preset, "operations": ops_json, "scaleX": 1.0, "scaleY": 1.0});
            result["evictedArtifactIds"] = json!([]);
            return Self::value(result);
        }
        let (w, h) = (parent.image.width(), parent.image.height());
        preprocess::plan(w, h, &operations, self.limits.max_image_pixels).map_err(|e| {
            invalid_args(e).with_details(json!({"artifactWidth": w, "artifactHeight": h}))
        })?;
        let start = Instant::now();
        let (image, sx, sy) = preprocess::apply(&parent.image, &operations);
        let preprocess_ms = ms_since(start);
        let encode_start = Instant::now();
        let png =
            encode_png(&image).map_err(|e| ToolError::new(ErrorCode::Internal, e.to_string()))?;
        let sha256 = sha256_hex(&png);
        let encode_ms = ms_since(encode_start);
        let record = json!({"preset": preset, "operations": ops_json, "scaleX": sx, "scaleY": sy});
        let mut extra = Map::new();
        extra.insert("created".into(), json!(true));
        extra.insert("preprocess".into(), record.clone());
        let mut derivation = record;
        derivation["type"] = json!("preprocess");
        let mut phases = BTreeMap::new();
        phases.insert("preprocessMs".into(), preprocess_ms);
        phases.insert("encodeMs".into(), encode_ms);
        Ok(Outcome {
            prepared: Prepared::Artifact(Box::new(PendingArtifact {
                kind: ArtifactKind::Preprocess,
                image,
                png,
                sha256,
                ttl_ms,
                lineage: Lineage::Derived {
                    parent: Box::new(parent.meta.clone()),
                    offset: (0.0, 0.0),
                    scale: (sx, sy),
                    derivation,
                },
                extra,
            })),
            phases,
        })
    }

    fn recognize(&self, args: RecognizeArgs, ctx: &CallCtx, timeout_ms: u64) -> ToolResult {
        let tool = catalog::TOOL_OCR_RECOGNIZE;
        let backend = self
            .ocr_backends
            .iter()
            .find(|b| b.id() == args.backend)
            .cloned()
            .ok_or_else(|| {
                let available: Vec<String> = self
                    .ocr_backends
                    .iter()
                    .map(|b| b.id().to_string())
                    .collect();
                ToolError::new(
                    ErrorCode::OcrBackendNotFound,
                    format!("未知 OCR 后端 {}（见 ocr.backends）", args.backend),
                )
                .with_details(json!({"available": available}))
            })?;
        let record = self
            .store
            .get(&args.artifact_id)
            .map_err(|e| lookup_error(&args.artifact_id, e))?;
        let kind = backend.kind();
        let language = args
            .language
            .unwrap_or_else(|| backend.info().default_language);
        let params = OcrParams {
            language: language.clone(),
            options: args.options.clone(),
        };
        let input = OcrInput {
            width: record.image.width(),
            height: record.image.height(),
            rgba: record.image.pixels(),
            png: &record.png,
        };
        let started = Instant::now();
        let output = backend
            .recognize(&input, &params, &ctx.cancel, ctx.deadline)
            .map_err(|e| {
                if matches!(e, OcrError::NotReady { .. }) {
                    // 就绪快照随失败更新，capabilities 不继续报告过时的“就绪”
                    self.ocr_ready
                        .lock()
                        .unwrap()
                        .insert(backend.id().to_string(), false);
                }
                if self.store.is_closed() {
                    closed_error()
                } else {
                    ocr_error(backend.id(), e, tool, timeout_ms)
                }
            })?;
        let ocr_ms = ms_since(started);
        self.ocr_ready
            .lock()
            .unwrap()
            .insert(backend.id().to_string(), true);
        let meta = &record.meta;
        let mut lines = Vec::new();
        let mut observations = Vec::new();
        for (index, o) in output.observations.iter().enumerate() {
            let root_box = o
                .bbox
                .map(|b| map_to_root(meta, b.x, b.y, b.width, b.height));
            let artifact_box = o.bbox.map(|b| Region {
                x: b.x,
                y: b.y,
                width: b.width,
                height: b.height,
            });
            let polygon = o.polygon.map(|p| {
                p.iter()
                    .map(|[x, y]| json!([compact(*x), compact(*y)]))
                    .collect::<Vec<_>>()
            });
            observations.push(json!({
                "index": index,
                "text": o.text,
                "granularity": o.granularity.as_str(),
                "geometry": if o.bbox.is_some() { "box" } else { "none" },
                "box": artifact_box.as_ref().map(region_value),
                "rootBox": root_box.as_ref().map(region_value),
                "polygon": polygon,
                "confidence": o.confidence,
                "label": o.label,
                "lines": o.lines,
            }));
            match o.granularity {
                kotone_core::ocr::OcrGranularity::Line => lines.push(IngestLine {
                    text: o.text.clone(),
                    root_box,
                    confidence: o.confidence,
                    block_box: None,
                }),
                _ => {
                    // 块/整页：块内各行没有逐行几何，只记录所在块的根坐标框
                    let block_lines: Vec<String> = if o.lines.is_empty() {
                        o.text
                            .lines()
                            .map(str::trim)
                            .filter(|l| !l.is_empty())
                            .map(str::to_string)
                            .collect()
                    } else {
                        o.lines.clone()
                    };
                    for text in block_lines {
                        lines.push(IngestLine {
                            text,
                            root_box: None,
                            confidence: o.confidence,
                            block_box: root_box,
                        });
                    }
                }
            }
        }
        let text = output.raw_text.clone().unwrap_or_else(|| {
            output
                .observations
                .iter()
                .map(|o| o.text.as_str())
                .collect::<Vec<_>>()
                .join("\n")
        });
        let empty = output.observations.is_empty();
        let mut warnings: Vec<Value> = output
            .warnings
            .iter()
            .map(|w| json!({"code": w.code, "message": w.message}))
            .collect();
        if empty && !warnings.iter().any(|w| w["code"] == "OCR_EMPTY_RESULT") {
            warnings.push(json!({
                "code": "OCR_EMPTY_RESULT",
                "message": "识别成功但没有文字；这不等于画面中确定没有聊天"
            }));
        }
        let now = self.store.now();
        let stored = StoredOcr {
            id: new_id("ocr"),
            expires_at_unix_ms: now.saturating_add(self.limits.ocr_result_ttl_ms),
            artifact_id: meta.artifact_id.clone(),
            frame_id: meta.source.frame_id.clone(),
            window_id: meta.source.window_id.clone(),
            captured_at_unix_ms: meta.source.captured_at_unix_ms,
            roi: region_of(meta),
            frame_size: (
                meta.source.client_rect_screen.width,
                meta.source.client_rect_screen.height,
            ),
            backend_id: backend.id().to_string(),
            lines,
        };
        let egress = if kind == OcrBackendKind::Online {
            "selected-image"
        } else {
            "none"
        };
        let result = json!({
            "artifactId": meta.artifact_id,
            "rootArtifactId": meta.root_artifact_id,
            "frameId": meta.source.frame_id,
            "windowId": meta.source.window_id,
            "capturedAtUnixMs": meta.source.captured_at_unix_ms,
            "backend": {
                "id": backend.id(),
                "kind": kind.as_str(),
                "dataEgress": egress,
                "modelVersion": output.model_version,
            },
            "language": language,
            "options": args.options,
            "text": text,
            "observations": observations,
            "coordinateSpace": COORDINATE_SPACE,
            "rootTransform": meta.root_transform,
            "geometryAvailable": output.observations.iter().any(|o| o.bbox.is_some()),
            "confidenceAvailable": output.observations.iter().any(|o| o.confidence.is_some()),
            "empty": empty,
            "truncated": output.truncated,
            "coldStart": output.cold_start,
            "warnings": warnings,
            "backendExtra": output.extra,
        });
        let mut phases: BTreeMap<String, u64> = output.phases.into_iter().collect();
        phases.insert("ocrMs".into(), ocr_ms);
        Ok(Outcome {
            prepared: Prepared::Ocr(Box::new(PendingOcr { result, stored })),
            phases,
        })
    }

    fn track_ingest(&self, args: IngestArgs) -> ToolResult {
        if !valid_session_name(&args.session) {
            return Err(invalid_args(SESSION_RULE));
        }
        let on_source_change = match args.on_source_change.as_deref() {
            None | Some("reject") => OnSourceChange::Reject,
            Some("new-generation") => OnSourceChange::NewGeneration,
            Some(other) => {
                return Err(invalid_args(format!(
                    "onSourceChange 只接受 reject / new-generation（收到 {other}）"
                )))
            }
        };
        let now = self.store.now();
        let (frame_id, window_id, captured_at, roi, frame_size, source, lines) = match args
            .observation
        {
            Observation::OcrResult { ocr_result_id } => {
                let stored = self.ocr_results.get(&ocr_result_id, now).map_err(|e| match e {
                        OcrLookupError::NotFound => ToolError::new(
                            ErrorCode::OcrResultNotFound,
                            format!("未知 OCR 结果 {ocr_result_id}（只接受本实例 ocr.recognize 返回的 ID）"),
                        ),
                        OcrLookupError::Expired => ToolError::new(
                            ErrorCode::OcrResultExpired,
                            format!("OCR 结果 {ocr_result_id} 已过期或被回收"),
                        ),
                    })?;
                (
                    stored.frame_id.clone(),
                    stored.window_id.clone(),
                    stored.captured_at_unix_ms,
                    stored.roi,
                    stored.frame_size,
                    SourceTag {
                        kind: "ocr".into(),
                        producer: stored.backend_id.clone(),
                        ocr_result_id: Some(stored.id.clone()),
                        artifact_id: stored.artifact_id.clone(),
                    },
                    stored.lines.clone(),
                )
            }
            Observation::External {
                artifact_id,
                producer,
                lines,
            } => {
                let producer = producer.trim().to_string();
                if producer.is_empty() || producer.chars().count() > 64 {
                    return Err(invalid_args(
                        "producer 必须是 1-64 个字符的来源说明（如 agent/model 名）",
                    ));
                }
                if lines.len() > self.limits.track.max_lines {
                    return Err(invalid_args(format!(
                        "观测行数 {} 超过上限 {}",
                        lines.len(),
                        self.limits.track.max_lines
                    )));
                }
                let record = self
                    .store
                    .get(&artifact_id)
                    .map_err(|e| lookup_error(&artifact_id, e))?;
                let meta = &record.meta;
                let (w, h) = (f64::from(meta.width), f64::from(meta.height));
                let mut out = Vec::with_capacity(lines.len());
                for (i, line) in lines.into_iter().enumerate() {
                    if let Some(c) = line.confidence {
                        if !(0.0..=1.0).contains(&c) {
                            return Err(invalid_args(format!(
                                "lines[{i}].confidence 必须在 0..=1"
                            )));
                        }
                    }
                    let root_box = match line.bbox {
                        None => None,
                        Some(b) => {
                            let valid = [b.x, b.y, b.width, b.height].iter().all(|v| v.is_finite())
                                && b.x >= 0.0
                                && b.y >= 0.0
                                && b.width > 0.0
                                && b.height > 0.0
                                && b.x + b.width <= w
                                && b.y + b.height <= h;
                            if !valid {
                                return Err(invalid_args(format!(
                                    "lines[{i}].box 必须位于产物 {}x{} 物理像素内且宽高为正",
                                    meta.width, meta.height
                                )));
                            }
                            Some(map_to_root(meta, b.x, b.y, b.width, b.height))
                        }
                    };
                    out.push(IngestLine {
                        text: line.text.trim().to_string(),
                        root_box,
                        confidence: line.confidence.map(|c| c as f32),
                        block_box: None,
                    });
                }
                out.retain(|l| !l.text.is_empty());
                (
                    meta.source.frame_id.clone(),
                    meta.source.window_id.clone(),
                    meta.source.captured_at_unix_ms,
                    region_of(meta),
                    (
                        meta.source.client_rect_screen.width,
                        meta.source.client_rect_screen.height,
                    ),
                    SourceTag {
                        kind: "external-vision".into(),
                        producer,
                        ocr_result_id: None,
                        artifact_id: meta.artifact_id.clone(),
                    },
                    out,
                )
            }
        };
        if lines.len() > self.limits.track.max_lines {
            return Err(invalid_args(format!(
                "观测行数 {} 超过上限 {}",
                lines.len(),
                self.limits.track.max_lines
            )));
        }
        // 内容指纹：决定同帧重复提交是否幂等
        let fingerprint = json!({
            "kind": source.kind,
            "producer": source.producer,
            "roi": region_value(&roi),
            "lines": lines.iter().map(|l| json!([l.text, l.root_box.map(|b| region_value(&b))])).collect::<Vec<_>>(),
        });
        let hash = sha256_hex(fingerprint.to_string().as_bytes());
        let input = IngestInput {
            session: args.session,
            expected_generation: args.generation,
            frame_id,
            window_id,
            captured_at_unix_ms: captured_at,
            roi,
            frame_size,
            source,
            lines,
            on_source_change,
        };
        Ok(Outcome {
            prepared: Prepared::TrackIngest(Box::new((input, hash))),
            phases: BTreeMap::new(),
        })
    }

    fn track_read(&self, args: ReadArgs, tool: &str, timeout_ms: u64) -> ToolResult {
        if !valid_session_name(&args.session) {
            return Err(invalid_args(SESSION_RULE));
        }
        if args.mode != "visible" && args.mode != "newSince" {
            return Err(invalid_args("mode 只能是 visible 或 newSince"));
        }
        if args.mode == "visible" && args.cursor.is_some() {
            return Err(invalid_args("mode=visible 不接受 cursor"));
        }
        let limit = args.limit.unwrap_or(100);
        if limit == 0 || limit > 500 {
            return Err(invalid_args("limit 必须在 1-500 之间"));
        }
        let value = self
            .tracker
            .read(
                &args.session,
                &args.mode,
                args.cursor.as_deref(),
                limit,
                self.store.now(),
            )
            .map_err(|e| track_error(e, tool, timeout_ms))?;
        Self::value(value)
    }
}
