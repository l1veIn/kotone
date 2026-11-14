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
}

impl FakeDesktop {
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
        Ok(CapturedImage {
            width: rect.width,
            height: rect.height,
            rgba: pattern(rect.width, rect.height),
            client_rect: rect,
            dpi: info.dpi,
            warnings: vec![CaptureWarning::new("FAKE", "test double")],
        })
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
