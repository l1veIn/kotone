//! 工具调度：参数校验、并发/期限控制、窗口引用与产物生命周期。
//!
//! 每次调用在 tokio 阻塞线程池执行（捕获、裁剪、PNG 编码、base64 都不在异步线程或任何
//! 运行时/UI 锁上进行）。并发许可随阻塞任务一起释放，所以超时调用在后台收尾期间仍占用
//! 名额，实际并发始终有界。
//!
//! 绝对期限在三处强制：取得执行名额后、工作线程真正开始时、以及最终提交闸门内。
//! 工作线程只产出“待提交”的结果（新产物或释放请求），从不直接修改产物存储；提交闸门
//! 与关闭（close）共用存储锁，期限已过或服务已关闭的结果一律丢弃，不产生副作用。

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine as _;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use tokio::sync::Semaphore;

use kotone_core::tools::{
    CaptureBackendInfo, WindowCapturer, WindowIdentity, WindowInfo, WindowObserver, WindowToolError,
};

use crate::artifacts::{
    ArtifactKind, ArtifactMeta, ArtifactRecord, ArtifactStore, CaptureSource, Clock, CommitError,
    Gone, LookupError, RectJson, RootTransform, WarningJson, COORDINATE_SPACE, MIME_PNG,
};
use crate::catalog::{self, Readiness, IMPLEMENTED_TOOLS};
use crate::ids::new_id;
use crate::image::{encode_png, sha256_hex, validate_crop, CropRect, RgbaImage};
use crate::limits::ServiceLimits;
use crate::protocol::{
    is_valid_request_id, Envelope, ErrorCode, Timing, ToolCallRequest, ToolError,
    PATH_CAPABILITIES, PATH_STATUS, PATH_TOOLS_CALL, PROTOCOL, PROTOCOL_VERSION,
};

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
    Crop {
        parent: Box<ArtifactMeta>,
        rect: CropRect,
    },
}

enum Prepared {
    Value(Value),
    Artifact(Box<PendingArtifact>),
    /// 释放请求：只在提交闸门内执行，排队超时的释放不会在之后删除产物。
    Release {
        artifact_id: String,
    },
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

impl ToolService {
    pub fn new(deps: ServiceDeps) -> Arc<Self> {
        let store = ArtifactStore::new(
            deps.clock.clone(),
            deps.limits.max_artifacts,
            deps.limits.max_store_bytes,
        );
        let started_at_unix_ms = deps.clock.now_unix_ms();
        Arc::new(Self {
            instance_id: deps.instance_id,
            app_version: deps.app_version,
            observer: deps.observer,
            capturer: deps.capturer,
            asr_phase: deps.asr_phase,
            calls: Arc::new(Semaphore::new(deps.limits.max_concurrent_calls.max(1))),
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

    pub fn limits(&self) -> &ServiceLimits {
        &self.limits
    }

    /// 关闭：在存储锁内撤销提交并丢弃全部内存产物，然后停止维护任务。
    /// 返回后不会再有任何请求向存储发布产物或执行释放；仍在运行的原生工作线程可以
    /// 完成自身清理，但其结果在提交闸门被拒绝。
    pub fn close(&self) {
        self.store.close();
        self.closing.send_replace(true);
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
        Readiness {
            windows_ready: capture.available,
            capture_ready: capture.available,
            reason: capture.unavailable_reason.clone(),
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
            ],
            "capture": {
                "method": capture.method,
                "available": capture.available,
                "unavailableReason": capture.unavailable_reason,
                "limitations": capture.limitations,
            },
            "limits": self.limits.to_json(),
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
        if !IMPLEMENTED_TOOLS.contains(&tool.as_str()) {
            let error = ToolError::new(
                ErrorCode::ToolNotFound,
                format!("工具 {tool} 未在本实例实现；可用工具见 system.capabilities"),
            )
            .with_details(json!({"available": IMPLEMENTED_TOOLS}));
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
        let deadline_std = deadline.into_std();
        // 闸门 1：取得名额时期限可能已过（排队耗尽了时间）
        if std::time::Instant::now() >= deadline_std {
            drop(permit);
            let error = timeout_error(&tool, timeout_ms, "queued");
            return self.envelope_failure(request_id, Some(tool), error, started);
        }
        let queue_ms = ms_since(started);
        let service = self.clone();
        let tool_name = tool.clone();
        let arguments = request.arguments;
        let work = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            // 闸门 2：阻塞线程池可能饱和，任务真正开始时再核对期限与关闭状态
            if std::time::Instant::now() >= deadline_std {
                return Err(timeout_error(&tool_name, timeout_ms, "not-started"));
            }
            if service.store.is_closed() {
                return Err(closed_error());
            }
            service.execute(&tool_name, arguments)
        });
        let outcome = match tokio::time::timeout_at(deadline, work).await {
            Err(_) => Err(timeout_error(&tool, timeout_ms, "running")),
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
            Prepared::Artifact(pending) => *pending,
        };
        let artifact_id = new_id("art");
        let now = self.store.now();
        let (width, height) = (pending.image.width(), pending.image.height());
        let (parent_artifact_id, root_artifact_id, offset_x, offset_y, source) =
            match pending.lineage {
                Lineage::Capture(source) => (None, artifact_id.clone(), 0, 0, source),
                Lineage::Crop { parent, rect } => (
                    Some(parent.artifact_id.clone()),
                    parent.root_artifact_id.clone(),
                    parent.root_transform.offset_x + i64::from(rect.x),
                    parent.root_transform.offset_y + i64::from(rect.y),
                    parent.source.clone(),
                ),
            };
        let screen_rect = RectJson {
            x: source.client_rect_screen.x + offset_x,
            y: source.client_rect_screen.y + offset_y,
            width,
            height,
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
            root_transform: RootTransform {
                offset_x,
                offset_y,
                scale_x: 1.0,
                scale_y: 1.0,
            },
            root_rect: RectJson {
                x: offset_x,
                y: offset_y,
                width,
                height,
            },
            screen_rect,
            source,
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

    /// 阻塞线程上执行具体工具。
    fn execute(&self, tool: &str, arguments: Value) -> ToolResult {
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
                lineage: Lineage::Crop {
                    parent: Box::new(parent.meta.clone()),
                    rect,
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
