//! 测试支持（仅 `cfg(test)` 或 `test-support` feature）：假窗口/捕获后端、可控时钟、
//! 无 ACL 的假宿主与最小 JSON Schema 校验器。
//!
//! 这些实现只用于自动化测试，不会被桌面端或 CLI 的正式构建使用，也不出现在 capabilities 中。

use std::collections::HashMap;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;

use kotone_core::tools::{
    CaptureBackendInfo, CaptureWarning, CapturedImage, PhysicalRect, ToolHost, WindowCapturer,
    WindowIdentity, WindowInfo, WindowObserver, WindowToolError,
};

use crate::artifacts::Clock;

/// 可手动推进的时钟。
#[derive(Debug, Default)]
pub struct ManualClock(AtomicU64);

impl ManualClock {
    pub fn at(ms: u64) -> Arc<Self> {
        Arc::new(Self(AtomicU64::new(ms)))
    }
    pub fn advance(&self, ms: u64) {
        self.0.fetch_add(ms, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn now_unix_ms(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
}

type GatePredicate = Box<dyn Fn() -> bool + Send>;

struct Gate {
    predicate: GatePredicate,
    entered: std::sync::mpsc::Sender<()>,
    release: std::sync::mpsc::Receiver<()>,
}

/// 可在指定调用处暂停的时钟：`arm` 之后，第一个满足谓词的 `now_unix_ms` 调用会通知
/// `entered` 并阻塞，直到测试发送 `release`。用于把同步提交确定性地停在插入之前。
#[derive(Default)]
pub struct GatedClock {
    ms: AtomicU64,
    gate: Mutex<Option<Gate>>,
}

impl GatedClock {
    pub fn at(ms: u64) -> Arc<Self> {
        let clock = Self::default();
        clock.ms.store(ms, Ordering::SeqCst);
        Arc::new(clock)
    }

    pub fn arm(
        &self,
        predicate: impl Fn() -> bool + Send + 'static,
    ) -> (std::sync::mpsc::Receiver<()>, std::sync::mpsc::Sender<()>) {
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        *self.gate.lock().unwrap() = Some(Gate {
            predicate: Box::new(predicate),
            entered: entered_tx,
            release: release_rx,
        });
        (entered_rx, release_tx)
    }
}

impl Clock for GatedClock {
    fn now_unix_ms(&self) -> u64 {
        let gate = {
            let mut slot = self.gate.lock().unwrap();
            if slot.as_ref().is_some_and(|g| (g.predicate)()) {
                slot.take()
            } else {
                None
            }
        };
        if let Some(gate) = gate {
            let _ = gate.entered.send(());
            let _ = gate.release.recv();
        }
        self.ms.load(Ordering::SeqCst)
    }
}

/// 假桌面中的一个窗口。
#[derive(Debug, Clone)]
pub struct FakeWindow {
    pub info: WindowInfo,
}

/// 坐标花纹：像素 (x, y) = [x mod 256, y mod 256, (x ^ y) mod 256, 255]，便于逐像素核对裁剪。
pub fn pattern_pixel(x: u32, y: u32) -> [u8; 4] {
    [x as u8, y as u8, (x ^ y) as u8, 255]
}

fn pattern(width: u32, height: u32) -> Vec<u8> {
    let mut px = Vec::with_capacity(width as usize * height as usize * 4);
    for y in 0..height {
        for x in 0..width {
            px.extend_from_slice(&pattern_pixel(x, y));
        }
    }
    px
}

/// 同时实现窗口观察与捕获端口的假桌面，语义与 GDI 前台模式的前置校验一致。
#[derive(Default)]
pub struct FakeDesktop {
    windows: Mutex<Vec<FakeWindow>>,
    capture_delay: Mutex<Duration>,
    captures: AtomicU64,
    capture_starts: AtomicU64,
    capture_threads: Mutex<Vec<std::thread::ThreadId>>,
    unavailable: Mutex<Option<String>>,
    /// 覆盖捕获像素（宽, 高, RGBA）；为 None 时使用坐标花纹。
    image: Mutex<Option<(u32, u32, Vec<u8>)>>,
}

impl FakeDesktop {
    /// 让后续捕获返回指定像素（客户区尺寸随之改为图像尺寸）。用于真实 OCR 冒烟。
    pub fn set_image(&self, handle: u64, width: u32, height: u32, rgba: Vec<u8>) {
        assert_eq!(rgba.len(), width as usize * height as usize * 4);
        for w in self.windows.lock().unwrap().iter_mut() {
            if w.info.identity.handle == handle {
                if let Some(rect) = w.info.client_rect.as_mut() {
                    rect.width = width;
                    rect.height = height;
                }
            }
        }
        *self.image.lock().unwrap() = Some((width, height, rgba));
    }

    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// 添加窗口，返回其身份。客户区位于 `(x, y)`，尺寸 `width x height`。
    #[allow(clippy::too_many_arguments)]
    pub fn add_window(
        &self,
        handle: u64,
        pid: u32,
        title: &str,
        process: &str,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
    ) -> WindowIdentity {
        let identity = WindowIdentity {
            handle,
            pid,
            process_started_at: Some(1_000 + u64::from(pid)),
        };
        let info = WindowInfo {
            identity,
            title: title.into(),
            class_name: "FakeWindowClass".into(),
            process_name: Some(process.into()),
            visible: true,
            minimized: false,
            cloaked: false,
            foreground: false,
            client_rect: Some(PhysicalRect {
                x,
                y,
                width,
                height,
            }),
            monitor_rect: Some(PhysicalRect {
                x: 0,
                y: 0,
                width: 8192,
                height: 8192,
            }),
            dpi: Some(144),
        };
        self.windows.lock().unwrap().push(FakeWindow { info });
        identity
    }

    pub fn set_foreground(&self, handle: Option<u64>) {
        for w in self.windows.lock().unwrap().iter_mut() {
            w.info.foreground = Some(w.info.identity.handle) == handle;
        }
    }

    pub fn set_minimized(&self, handle: u64, minimized: bool) {
        for w in self.windows.lock().unwrap().iter_mut() {
            if w.info.identity.handle == handle {
                w.info.minimized = minimized;
            }
        }
    }

    pub fn close(&self, handle: u64) {
        self.windows
            .lock()
            .unwrap()
            .retain(|w| w.info.identity.handle != handle);
    }

    /// 模拟 HWND 被另一个进程复用。
    pub fn reuse_handle(&self, handle: u64, new_pid: u32) {
        for w in self.windows.lock().unwrap().iter_mut() {
            if w.info.identity.handle == handle {
                w.info.identity.pid = new_pid;
                w.info.identity.process_started_at = Some(9_999);
            }
        }
    }

    pub fn set_capture_delay(&self, delay: Duration) {
        *self.capture_delay.lock().unwrap() = delay;
    }

    pub fn set_unavailable(&self, reason: Option<&str>) {
        *self.unavailable.lock().unwrap() = reason.map(str::to_string);
    }

    /// 已完成（成功产出像素）的捕获次数。
    pub fn capture_count(&self) -> u64 {
        self.captures.load(Ordering::SeqCst)
    }

    /// 捕获调用真正开始执行的次数（包括随后失败的调用）。
    pub fn capture_starts(&self) -> u64 {
        self.capture_starts.load(Ordering::SeqCst)
    }

    /// 当前线程是否执行过捕获（用于区分工作线程与提交所在的运行时线程）。
    pub fn is_capture_thread(&self) -> bool {
        self.capture_threads
            .lock()
            .unwrap()
            .contains(&std::thread::current().id())
    }

    fn find(&self, handle: u64) -> Option<WindowInfo> {
        self.windows
            .lock()
            .unwrap()
            .iter()
            .find(|w| w.info.identity.handle == handle)
            .map(|w| w.info.clone())
    }
}

fn check_identity(
    desktop: &FakeDesktop,
    identity: &WindowIdentity,
) -> Result<WindowInfo, WindowToolError> {
    let info = desktop.find(identity.handle).ok_or(WindowToolError::Gone)?;
    if info.identity.pid != identity.pid
        || info.identity.process_started_at != identity.process_started_at
    {
        return Err(WindowToolError::IdentityChanged);
    }
    Ok(info)
}

impl WindowObserver for FakeDesktop {
    fn list_windows(&self) -> Result<Vec<WindowInfo>, WindowToolError> {
        Ok(self
            .windows
            .lock()
            .unwrap()
            .iter()
            .map(|w| w.info.clone())
            .collect())
    }

    fn foreground_window(&self) -> Result<Option<WindowInfo>, WindowToolError> {
        Ok(self
            .windows
            .lock()
            .unwrap()
            .iter()
            .find(|w| w.info.foreground)
            .map(|w| w.info.clone()))
    }

    fn inspect(&self, identity: &WindowIdentity) -> Result<WindowInfo, WindowToolError> {
        check_identity(self, identity)
    }
}

impl WindowCapturer for FakeDesktop {
    fn backend_info(&self) -> CaptureBackendInfo {
        let unavailable = self.unavailable.lock().unwrap().clone();
        CaptureBackendInfo {
            method: "fake-test-capture".into(),
            available: unavailable.is_none(),
            unavailable_reason: unavailable,
            limitations: vec!["test double; not a real capture backend".into()],
        }
    }

    fn capture_foreground_client(
        &self,
        identity: &WindowIdentity,
        max_pixels: u64,
    ) -> Result<CapturedImage, WindowToolError> {
        self.capture_starts.fetch_add(1, Ordering::SeqCst);
        {
            let mut threads = self.capture_threads.lock().unwrap();
            let me = std::thread::current().id();
            if !threads.contains(&me) {
                threads.push(me);
            }
        }
        let delay = *self.capture_delay.lock().unwrap();
        if !delay.is_zero() {
            std::thread::sleep(delay);
        }
        let info = check_identity(self, identity)?;
        if info.minimized {
            return Err(WindowToolError::Minimized);
        }
        if !info.foreground {
            return Err(WindowToolError::NotForeground);
        }
        let rect = info
            .client_rect
            .ok_or_else(|| WindowToolError::InvalidGeometry("no client rect".into()))?;
        if rect.width == 0 || rect.height == 0 {
            return Err(WindowToolError::InvalidGeometry("empty".into()));
        }
        if rect.pixel_count() > max_pixels {
            return Err(WindowToolError::TooLarge {
                pixels: rect.pixel_count(),
                max_pixels,
            });
        }
        self.captures.fetch_add(1, Ordering::SeqCst);
        let rgba = match self.image.lock().unwrap().as_ref() {
            Some((w, h, px)) if *w == rect.width && *h == rect.height => px.clone(),
            _ => pattern(rect.width, rect.height),
        };
        Ok(CapturedImage {
            width: rect.width,
            height: rect.height,
            rgba,
            client_rect: rect,
            dpi: info.dpi,
            warnings: vec![CaptureWarning::new("FAKE", "test double")],
        })
    }
}

/// 可编排的假 OCR 后端：按调用顺序返回预设结果，记录调用、取消与最大并发。
///
/// `cooperative=false` 时模拟不响应取消的后端（工作线程在延迟结束前不会返回），
/// 用于验证名额直到工作线程真正结束才释放。
pub struct FakeOcr {
    id: String,
    kind: kotone_core::ocr::OcrBackendKind,
    state: Mutex<FakeOcrState>,
    calls: AtomicU64,
    cancels: AtomicU64,
    finished: AtomicU64,
    active: std::sync::atomic::AtomicUsize,
    max_active: std::sync::atomic::AtomicUsize,
}

struct FakeOcrState {
    ready: bool,
    languages: Vec<String>,
    delay: Duration,
    cooperative: bool,
    script:
        std::collections::VecDeque<Result<kotone_core::ocr::OcrOutput, kotone_core::ocr::OcrError>>,
}

/// 便捷构造：一行带框观测（输入图像像素）。
pub fn ocr_line(
    text: &str,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    confidence: Option<f32>,
) -> kotone_core::ocr::OcrObservation {
    kotone_core::ocr::OcrObservation {
        text: text.into(),
        granularity: kotone_core::ocr::OcrGranularity::Line,
        bbox: Some(kotone_core::ocr::OcrRect {
            x,
            y,
            width: w,
            height: h,
        }),
        polygon: Some([[x, y], [x + w, y], [x + w, y + h], [x, y + h]]),
        confidence,
        label: None,
        lines: vec![],
    }
}

/// 便捷构造：识别结果。
pub fn ocr_output(
    observations: Vec<kotone_core::ocr::OcrObservation>,
) -> kotone_core::ocr::OcrOutput {
    kotone_core::ocr::OcrOutput {
        observations,
        raw_text: None,
        truncated: false,
        warnings: vec![],
        model_version: "fake-model 1".into(),
        cold_start: false,
        phases: vec![],
        extra: serde_json::json!({}),
    }
}

impl FakeOcr {
    pub fn new(id: &str, kind: kotone_core::ocr::OcrBackendKind) -> Arc<Self> {
        Arc::new(Self {
            id: id.into(),
            kind,
            state: Mutex::new(FakeOcrState {
                ready: true,
                languages: vec!["ko".into()],
                delay: Duration::ZERO,
                cooperative: true,
                script: Default::default(),
            }),
            calls: AtomicU64::new(0),
            cancels: AtomicU64::new(0),
            finished: AtomicU64::new(0),
            active: Default::default(),
            max_active: Default::default(),
        })
    }

    pub fn set_ready(&self, ready: bool) {
        self.state.lock().unwrap().ready = ready;
    }

    pub fn set_languages(&self, languages: &[&str]) {
        self.state.lock().unwrap().languages = languages.iter().map(|s| s.to_string()).collect();
    }

    pub fn set_delay(&self, delay: Duration, cooperative: bool) {
        let mut state = self.state.lock().unwrap();
        state.delay = delay;
        state.cooperative = cooperative;
    }

    pub fn push(&self, result: Result<kotone_core::ocr::OcrOutput, kotone_core::ocr::OcrError>) {
        self.state.lock().unwrap().script.push_back(result);
    }

    /// 真正开始执行（越过语言/选项校验）的识别次数。
    pub fn calls(&self) -> u64 {
        self.calls.load(Ordering::SeqCst)
    }

    pub fn cancels(&self) -> u64 {
        self.cancels.load(Ordering::SeqCst)
    }

    /// 工作线程真正结束（返回）的识别次数。
    pub fn finished(&self) -> u64 {
        self.finished.load(Ordering::SeqCst)
    }

    pub fn max_active(&self) -> usize {
        self.max_active.load(Ordering::SeqCst)
    }
}

impl kotone_core::ocr::OcrBackend for FakeOcr {
    fn id(&self) -> &str {
        &self.id
    }

    fn kind(&self) -> kotone_core::ocr::OcrBackendKind {
        self.kind
    }

    fn info(&self) -> kotone_core::ocr::OcrBackendInfo {
        let state = self.state.lock().unwrap();
        kotone_core::ocr::OcrBackendInfo {
            id: self.id.clone(),
            kind: self.kind,
            display_name: format!("fake {}", self.id),
            ready: state.ready,
            state: if state.ready {
                "ready"
            } else {
                "not-installed"
            }
            .into(),
            reason: (!state.ready).then(|| "fake backend not installed".to_string()),
            languages: state.languages.clone(),
            default_language: state.languages.first().cloned().unwrap_or_default(),
            data_egress: if self.kind == kotone_core::ocr::OcrBackendKind::Online {
                "selected-image"
            } else {
                "none"
            }
            .into(),
            options_schema: serde_json::json!({"type": ["object", "null"], "additionalProperties": false, "properties": {}}),
            details: serde_json::json!({"fake": true}),
        }
    }

    fn recognize(
        &self,
        input: &kotone_core::ocr::OcrInput<'_>,
        params: &kotone_core::ocr::OcrParams,
        cancel: &kotone_core::ocr::OcrCancel,
        _deadline: std::time::Instant,
    ) -> Result<kotone_core::ocr::OcrOutput, kotone_core::ocr::OcrError> {
        use kotone_core::ocr::OcrError;
        let (ready, languages, delay, cooperative) = {
            let s = self.state.lock().unwrap();
            (s.ready, s.languages.clone(), s.delay, s.cooperative)
        };
        if !languages.contains(&params.language) {
            return Err(OcrError::UnsupportedLanguage {
                language: params.language.clone(),
                supported: languages,
            });
        }
        match &params.options {
            Value::Null => {}
            Value::Object(m) if m.is_empty() => {}
            Value::Object(m) => {
                return Err(OcrError::UnsupportedOption {
                    option: m.keys().next().cloned().unwrap_or_default(),
                    message: "fake backend has no options".into(),
                })
            }
            _ => {
                return Err(OcrError::UnsupportedOption {
                    option: "options".into(),
                    message: "must be an object".into(),
                })
            }
        }
        if !ready {
            return Err(OcrError::NotReady {
                reason: "not-installed".into(),
                message: "fake backend not installed".into(),
            });
        }
        assert!(input.width > 0 && !input.png.is_empty());
        self.calls.fetch_add(1, Ordering::SeqCst);
        let now_active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_active.fetch_max(now_active, Ordering::SeqCst);
        struct Done<'a>(&'a FakeOcr);
        impl Drop for Done<'_> {
            fn drop(&mut self) {
                self.0.active.fetch_sub(1, Ordering::SeqCst);
                self.0.finished.fetch_add(1, Ordering::SeqCst);
            }
        }
        let _done = Done(self);
        let started = std::time::Instant::now();
        while started.elapsed() < delay {
            // 协作模式只响应取消令牌（不自行检查期限），用于验证服务端的取消传递
            if cooperative && cancel.is_cancelled() {
                self.cancels.fetch_add(1, Ordering::SeqCst);
                return Err(OcrError::Cancelled);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        if cancel.is_cancelled() {
            self.cancels.fetch_add(1, Ordering::SeqCst);
        }
        self.state
            .lock()
            .unwrap()
            .script
            .pop_front()
            .unwrap_or_else(|| Ok(ocr_output(vec![])))
    }
}

/// 假宿主：普通文件读写（不设置 ACL），可配置会话与“存活进程”。
pub struct FakeHost {
    pub session: Option<u32>,
    processes: Mutex<HashMap<u32, u64>>,
    pub insecure: Mutex<bool>,
}

impl FakeHost {
    pub fn new(session: Option<u32>) -> Arc<Self> {
        let host = Self {
            session,
            processes: Mutex::new(HashMap::new()),
            insecure: Mutex::new(false),
        };
        host.processes
            .lock()
            .unwrap()
            .insert(std::process::id(), 42);
        Arc::new(host)
    }

    pub fn set_process(&self, pid: u32, started: Option<u64>) {
        let mut map = self.processes.lock().unwrap();
        match started {
            Some(s) => {
                map.insert(pid, s);
            }
            None => {
                map.remove(&pid);
            }
        }
    }
}

impl ToolHost for FakeHost {
    fn session_id(&self) -> Option<u32> {
        self.session
    }
    fn process_started_at(&self, pid: u32) -> Option<u64> {
        self.processes.lock().unwrap().get(&pid).copied()
    }
    fn prepare_private_dir(&self, dir: &Path) -> io::Result<()> {
        std::fs::create_dir_all(dir)
    }
    fn write_private_file(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        std::fs::write(path, bytes)
    }
    fn verify_private_file(&self, _path: &Path) -> io::Result<()> {
        if *self.insecure.lock().unwrap() {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "fake insecure",
            ))
        } else {
            Ok(())
        }
    }
}

/// 最小 JSON Schema 校验（type/const/enum/required/properties/additionalProperties=false/
/// items/oneOf/minimum），足以核对 capabilities 声明的 outputSchema 与真实结果一致。
pub fn validate_schema(schema: &Value, value: &Value, path: &str) -> Result<(), String> {
    if let Some(options) = schema.get("oneOf").and_then(|v| v.as_array()) {
        let matches = options
            .iter()
            .filter(|s| validate_schema(s, value, path).is_ok())
            .count();
        return if matches == 1 {
            Ok(())
        } else {
            Err(format!("{path}: oneOf matched {matches}"))
        };
    }
    if let Some(expected) = schema.get("const") {
        if expected != value {
            return Err(format!("{path}: expected const {expected}, got {value}"));
        }
    }
    if let Some(options) = schema.get("enum").and_then(|v| v.as_array()) {
        if !options.contains(value) {
            return Err(format!("{path}: {value} not in enum"));
        }
    }
    if let Some(types) = schema.get("type") {
        let allowed: Vec<&str> = match types {
            Value::String(s) => vec![s.as_str()],
            Value::Array(items) => items.iter().filter_map(|v| v.as_str()).collect(),
            _ => vec![],
        };
        let actual_ok = allowed.iter().any(|t| match *t {
            "object" => value.is_object(),
            "array" => value.is_array(),
            "string" => value.is_string(),
            "integer" => value.is_i64() || value.is_u64(),
            "number" => value.is_number(),
            "boolean" => value.is_boolean(),
            "null" => value.is_null(),
            _ => false,
        });
        if !actual_ok {
            return Err(format!("{path}: {value} is not of type {allowed:?}"));
        }
    }
    if let (Some(min), Some(n)) = (
        schema.get("minimum").and_then(|m| m.as_f64()),
        value.as_f64(),
    ) {
        if n < min {
            return Err(format!("{path}: {n} < minimum {min}"));
        }
    }
    if let Some(object) = value.as_object() {
        if let Some(required) = schema.get("required").and_then(|r| r.as_array()) {
            for key in required.iter().filter_map(|k| k.as_str()) {
                if !object.contains_key(key) {
                    return Err(format!("{path}: missing required {key}"));
                }
            }
        }
        let properties = schema.get("properties").and_then(|p| p.as_object());
        let closed = schema.get("additionalProperties") == Some(&Value::Bool(false));
        for (key, child) in object {
            match properties.and_then(|p| p.get(key)) {
                Some(child_schema) => {
                    validate_schema(child_schema, child, &format!("{path}.{key}"))?
                }
                None if closed => return Err(format!("{path}: unexpected property {key}")),
                None => {}
            }
        }
    }
    if let (Some(items), Some(array)) = (schema.get("items"), value.as_array()) {
        for (i, item) in array.iter().enumerate() {
            validate_schema(items, item, &format!("{path}[{i}]"))?;
        }
    }
    Ok(())
}

/// 解码 PNG（测试核对像素用）。
pub fn decode_png(bytes: &[u8]) -> (u32, u32, Vec<u8>) {
    let decoder = png::Decoder::new(bytes);
    let mut reader = decoder.read_info().expect("png header");
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).expect("png frame");
    buf.truncate(info.buffer_size());
    (info.width, info.height, buf)
}

/// 可编排的假桌面宿主：记录退出、交付与助手提交，可设置交付延迟与失败。
#[derive(Default)]
pub struct FakeAppHost {
    quits: AtomicU64,
    started: AtomicU64,
    stopped: AtomicU64,
    deliveries: Mutex<Vec<(String, Vec<&'static str>)>>,
    cancelled_deliveries: AtomicU64,
    delay: Mutex<Duration>,
    fail: Mutex<Option<String>>,
    submitted: Mutex<Vec<String>>,
    invocations: Mutex<Vec<(String, Value)>>,
    invoke_results: Mutex<std::collections::HashMap<String, Result<Value, crate::host::HostError>>>,
    plugin_tools: Mutex<Vec<Value>>,
    /// MCP App 资源：`resources/list` 的项和读到的内容（按 uri）。
    resources: Mutex<Vec<(Value, Value)>>,
}

impl FakeAppHost {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }
    /// 开着的插件的工具（`AppHost::plugin_tools`）。
    pub fn set_plugin_tools(&self, specs: Vec<Value>) {
        *self.plugin_tools.lock().unwrap() = specs;
    }
    /// 开着的插件的 MCP App 资源（`AppHost::resources` / `read_resource`）：(列表的一项, 读到的 contents 一项)。
    pub fn set_resources(&self, resources: Vec<(Value, Value)>) {
        *self.resources.lock().unwrap() = resources;
    }
    pub fn set_delay(&self, delay: Duration) {
        *self.delay.lock().unwrap() = delay;
    }
    pub fn set_failure(&self, message: Option<&str>) {
        *self.fail.lock().unwrap() = message.map(str::to_string);
    }
    pub fn quits(&self) -> u64 {
        self.quits.load(Ordering::SeqCst)
    }
    pub fn runtime_starts(&self) -> u64 {
        self.started.load(Ordering::SeqCst)
    }
    pub fn deliveries(&self) -> Vec<(String, Vec<&'static str>)> {
        self.deliveries.lock().unwrap().clone()
    }
    /// 交付 future 在完成前被丢弃（期限 / 关闭）的次数。
    pub fn cancelled_deliveries(&self) -> u64 {
        self.cancelled_deliveries.load(Ordering::SeqCst)
    }
    pub fn submitted(&self) -> Vec<String> {
        self.submitted.lock().unwrap().clone()
    }
    /// 配置类工具调用记录（工具名, 参数）。
    pub fn invocations(&self) -> Vec<(String, Value)> {
        self.invocations.lock().unwrap().clone()
    }
    /// 设定某个配置类工具的返回；未设定时回显 `{"tool", "args"}`。
    pub fn set_invoke_result(&self, tool: &str, result: Result<Value, crate::host::HostError>) {
        self.invoke_results
            .lock()
            .unwrap()
            .insert(tool.to_string(), result);
    }
}

impl crate::host::AppHost for FakeAppHost {
    fn app_info(&self) -> Value {
        serde_json::json!({"host": "fake", "launch": "test", "assistantRoute": "agent"})
    }
    fn runtime_status(&self) -> Value {
        let running = self.started.load(Ordering::SeqCst) > self.stopped.load(Ordering::SeqCst);
        serde_json::json!({"phase": if running { "running" } else { "stopped" }})
    }
    fn runtime_start(&self) -> crate::host::HostFuture<'_, Value> {
        Box::pin(async move {
            if let Some(e) = self.fail.lock().unwrap().clone() {
                return Err(e);
            }
            self.started.fetch_add(1, Ordering::SeqCst);
            Ok(self.runtime_status())
        })
    }
    fn runtime_stop(&self) -> crate::host::HostFuture<'_, Value> {
        Box::pin(async move {
            self.stopped
                .store(self.started.load(Ordering::SeqCst), Ordering::SeqCst);
            Ok(self.runtime_status())
        })
    }
    fn request_quit(&self) -> Result<(), String> {
        self.quits.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    fn deliver(
        &self,
        text: String,
        outputs: Vec<crate::host::ReplyOutput>,
    ) -> crate::host::HostFuture<'_, Value> {
        struct Pending<'a>(&'a AtomicU64, bool);
        impl Drop for Pending<'_> {
            fn drop(&mut self) {
                if !self.1 {
                    self.0.fetch_add(1, Ordering::SeqCst);
                }
            }
        }
        Box::pin(async move {
            let mut pending = Pending(&self.cancelled_deliveries, false);
            let delay = *self.delay.lock().unwrap();
            tokio::time::sleep(delay).await;
            if let Some(e) = self.fail.lock().unwrap().clone() {
                pending.1 = true;
                return Err(e);
            }
            let outputs: Vec<&'static str> = if outputs.is_empty() {
                vec!["speech", "danmaku"]
            } else {
                outputs.iter().map(|o| o.as_str()).collect()
            };
            self.deliveries
                .lock()
                .unwrap()
                .push((text, outputs.clone()));
            pending.1 = true;
            Ok(serde_json::json!({"delivered": outputs, "errors": []}))
        })
    }
    fn assistant_submit(&self, text: String) -> crate::host::HostFuture<'_, Value> {
        Box::pin(async move {
            self.submitted.lock().unwrap().push(text);
            Ok(serde_json::json!({"route": "agent", "message": "已转交 Agent"}))
        })
    }
    fn supports_invoke(&self) -> bool {
        true
    }
    fn plugin_tools(&self) -> Vec<Value> {
        self.plugin_tools.lock().unwrap().clone()
    }
    fn resources(&self) -> Vec<Value> {
        let resources = self.resources.lock().unwrap();
        resources
            .iter()
            .map(|(listing, _)| listing.clone())
            .collect()
    }
    fn read_resource(&self, uri: &str) -> crate::host::HostInvokeFuture<'_> {
        let found = self
            .resources
            .lock()
            .unwrap()
            .iter()
            .find(|(listing, _)| listing["uri"] == uri)
            .map(|(_, content)| content.clone());
        let uri = uri.to_string();
        Box::pin(async move {
            found.ok_or(crate::host::HostError::InvalidArguments(format!(
                "没有资源 {uri}"
            )))
        })
    }
    fn invoke(&self, tool: &str, args: Value) -> crate::host::HostInvokeFuture<'_> {
        let tool = tool.to_string();
        Box::pin(async move {
            let delay = *self.delay.lock().unwrap();
            tokio::time::sleep(delay).await;
            self.invocations
                .lock()
                .unwrap()
                .push((tool.clone(), args.clone()));
            match self.invoke_results.lock().unwrap().get(&tool) {
                Some(result) => result.clone(),
                None => Ok(serde_json::json!({"tool": tool, "args": args})),
            }
        })
    }
}
