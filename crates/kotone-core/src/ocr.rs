//! OCR 后端端口（M2）：本地 / 在线可插拔，统一文字观测、几何粒度、可空置信度与失败分类。
//!
//! 只定义类型与端口；本地 PP-OCRv5（ONNX Runtime）与 DeepSeek 在线实现位于 `kotone-ocr`，
//! 调度、产物与坐标映射位于 `kotone-tools`。core 不依赖 HTTP / ONNX / Windows。
//!
//! 约定：
//! - 输入是一张已解码的 RGBA8 图像（某个产物的物理像素）及其 PNG 字节；后端返回的
//!   框都在**该输入图像**的物理像素坐标中（左上原点），由服务映射回根产物。
//! - 后端没有提供几何时 `bbox=None`，不得按行数伪造坐标；整块框使用 `Block` 粒度。
//! - 后端没有提供置信度时 `confidence=None`；置信度只是识别器自报的分数，不是语义正确率。
//! - 后端不得自行降级到另一后端（尤其不得从本地静默改走在线）。
//! - 识别是阻塞调用，由调用方放到工作线程；实现应尊重 [`OcrCancel`] 与 `deadline`。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde_json::Value;

/// 后端部署形态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OcrBackendKind {
    Local,
    Online,
}

impl OcrBackendKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Online => "online",
        }
    }
}

/// 文字观测的几何粒度。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OcrGranularity {
    /// 一行文字（本地检测框）。
    Line,
    /// 一个整块区域（可能含多行，例如在线 rich layout 的版面框）。
    Block,
    /// 整页/整图文字，没有更细的几何。
    Page,
}

impl OcrGranularity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Line => "line",
            Self::Block => "block",
            Self::Page => "page",
        }
    }
}

/// 输入图像物理像素中的轴对齐矩形（可为小数，例如在线归一化坐标换算）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OcrRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// 单条文字观测。
#[derive(Debug, Clone, PartialEq)]
pub struct OcrObservation {
    /// 后端返回的原文（不做语义修正；仅去掉首尾空白）。
    pub text: String,
    pub granularity: OcrGranularity,
    /// 轴对齐外接框；没有几何时为 None。
    pub bbox: Option<OcrRect>,
    /// 四点多边形（左上、右上、右下、左下）；没有时为 None。
    pub polygon: Option<[[f64; 2]; 4]>,
    /// 识别器自报分数 0..=1；没有时为 None（不得伪造 1.0）。
    pub confidence: Option<f32>,
    /// 后端给出的区域类别（如在线版面的 `text` / `title`）；没有时为 None。
    pub label: Option<String>,
    /// `Block` 粒度时块内的文本行（无逐行几何）；其他粒度为空。
    pub lines: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OcrWarning {
    pub code: String,
    pub message: String,
}

impl OcrWarning {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

/// 一次识别的结果。
#[derive(Debug, Clone, PartialEq)]
pub struct OcrOutput {
    /// 阅读顺序（自上而下、自左而右）的观测。
    pub observations: Vec<OcrObservation>,
    /// 后端原样返回的整段文字（在线后端）；本地后端为 None（由观测拼接）。
    pub raw_text: Option<String>,
    /// 输出被截断（例如在线 `finish_reason=length`）。
    pub truncated: bool,
    pub warnings: Vec<OcrWarning>,
    /// 实际使用的模型/接口版本描述。
    pub model_version: String,
    /// 本次是否包含模型冷加载。
    pub cold_start: bool,
    /// 分阶段耗时（毫秒），如 `modelLoadMs` / `detMs` / `recMs` / `uploadMs`。
    pub phases: Vec<(String, u64)>,
    /// 后端附加信息（如在线 usage）；不得包含凭证。
    pub extra: Value,
}

/// 后端自我描述与就绪状态（`ocr.backends`）。
#[derive(Debug, Clone, PartialEq)]
pub struct OcrBackendInfo {
    pub id: String,
    pub kind: OcrBackendKind,
    pub display_name: String,
    /// 能否立刻接受识别请求。
    pub ready: bool,
    /// 机器可读状态：`ready` / `not-installed` / `invalid` / `runtime-unavailable` /
    /// `not-configured` / `credentials-missing` / `credentials-error`。
    pub state: String,
    pub reason: Option<String>,
    pub languages: Vec<String>,
    pub default_language: String,
    /// 数据外发：`none` 或 `selected-image`。
    pub data_egress: String,
    /// 支持的 `options` JSON Schema（additionalProperties=false）。
    pub options_schema: Value,
    /// 其余描述：设备、运行时、模型来源/许可/版本/哈希、几何与置信度语义、安装入口等。
    pub details: Value,
}

/// 识别参数。
#[derive(Debug, Clone, PartialEq)]
pub struct OcrParams {
    pub language: String,
    /// 后端专属选项（对象）；未知键必须拒绝。
    pub options: Value,
}

/// 待识别图像（某个产物的物理像素）。
pub struct OcrInput<'a> {
    pub width: u32,
    pub height: u32,
    /// 紧密排列的 RGBA8。
    pub rgba: &'a [u8],
    /// 同一图像的 PNG 编码（在线后端上传用）。
    pub png: &'a [u8],
}

/// 识别失败分类；每类映射到协议中独立的错误码，不把网络错误伪装成“没看清”。
#[derive(Debug, Clone, PartialEq)]
pub enum OcrError {
    /// 后端未就绪：`reason` 为机器可读原因（同 [`OcrBackendInfo::state`]）。
    NotReady {
        reason: String,
        message: String,
    },
    UnsupportedLanguage {
        language: String,
        supported: Vec<String>,
    },
    UnsupportedOption {
        option: String,
        message: String,
    },
    /// 输入不适合该后端（尺寸等）。
    InvalidInput(String),
    Cancelled,
    DeadlineExceeded,
    /// 远端返回错误（HTTP 状态或 HTTP 200 内的 error 对象）。
    Remote {
        status: Option<u16>,
        error_type: Option<String>,
        message: String,
        retryable: bool,
    },
    /// 网络/传输失败（连接、读取超时）。
    Transport {
        message: String,
        retryable: bool,
    },
    /// 响应无法解析或结构不符。
    ResponseInvalid(String),
    /// 本地推理失败。
    Engine(String),
}

impl std::fmt::Display for OcrError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotReady { message, .. } => write!(f, "OCR 后端未就绪: {message}"),
            Self::UnsupportedLanguage {
                language,
                supported,
            } => write!(
                f,
                "不支持的语言 {language}（支持: {}）",
                supported.join(", ")
            ),
            Self::UnsupportedOption { option, message } => {
                write!(f, "不支持的选项 {option}: {message}")
            }
            Self::InvalidInput(m) => write!(f, "输入不适合该后端: {m}"),
            Self::Cancelled => write!(f, "识别已取消"),
            Self::DeadlineExceeded => write!(f, "识别超过截止时间"),
            Self::Remote {
                status, message, ..
            } => match status {
                Some(s) => write!(f, "在线 OCR 返回错误（HTTP {s}）: {message}"),
                None => write!(f, "在线 OCR 返回错误: {message}"),
            },
            Self::Transport { message, .. } => write!(f, "在线 OCR 传输失败: {message}"),
            Self::ResponseInvalid(m) => write!(f, "OCR 响应无效: {m}"),
            Self::Engine(m) => write!(f, "本地 OCR 推理失败: {m}"),
        }
    }
}

impl std::error::Error for OcrError {}

type CancelHook = Box<dyn Fn() + Send + Sync>;

/// 协作式取消：调用方在期限到达或服务关闭时 `cancel()`；后端可注册钩子
/// （例如 ONNX Runtime 的 RunOptionsSetTerminate）以中断正在进行的推理。
#[derive(Clone, Default)]
pub struct OcrCancel {
    inner: Arc<CancelInner>,
}

#[derive(Default)]
struct CancelInner {
    cancelled: AtomicBool,
    hooks: Mutex<Vec<(u64, CancelHook)>>,
    next: std::sync::atomic::AtomicU64,
}

/// 钩子注册守卫：析构时注销。
pub struct CancelRegistration {
    inner: Arc<CancelInner>,
    id: u64,
}

impl Drop for CancelRegistration {
    fn drop(&mut self) {
        self.inner
            .hooks
            .lock()
            .unwrap()
            .retain(|(id, _)| *id != self.id);
    }
}

impl OcrCancel {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        if !self.inner.cancelled.swap(true, Ordering::SeqCst) {
            let hooks = self.inner.hooks.lock().unwrap();
            for (_, hook) in hooks.iter() {
                hook();
            }
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::SeqCst)
    }

    /// 注册取消钩子；若已取消则立即执行。钩子必须短小、线程安全。
    pub fn on_cancel(&self, hook: impl Fn() + Send + Sync + 'static) -> CancelRegistration {
        let id = self.inner.next.fetch_add(1, Ordering::SeqCst);
        {
            let mut hooks = self.inner.hooks.lock().unwrap();
            hooks.push((id, Box::new(hook)));
        }
        if self.is_cancelled() {
            let hooks = self.inner.hooks.lock().unwrap();
            if let Some((_, hook)) = hooks.iter().find(|(i, _)| *i == id) {
                hook();
            }
        }
        CancelRegistration {
            inner: self.inner.clone(),
            id,
        }
    }

    /// 取消或期限已到时返回对应错误。
    pub fn check(&self, deadline: Instant) -> Result<(), OcrError> {
        if self.is_cancelled() {
            return Err(OcrError::Cancelled);
        }
        if Instant::now() >= deadline {
            return Err(OcrError::DeadlineExceeded);
        }
        Ok(())
    }
}

/// OCR 后端端口。
pub trait OcrBackend: Send + Sync {
    fn id(&self) -> &str;
    fn kind(&self) -> OcrBackendKind;
    /// 就绪检查（可能做文件哈希校验，调用方应在工作线程调用）。
    fn info(&self) -> OcrBackendInfo;
    /// 阻塞识别。必须拒绝不支持的语言与选项，不得降级到其他后端。
    fn recognize(
        &self,
        input: &OcrInput<'_>,
        params: &OcrParams,
        cancel: &OcrCancel,
        deadline: Instant,
    ) -> Result<OcrOutput, OcrError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn cancel_runs_hooks_once_and_late_registration_fires_immediately() {
        let cancel = OcrCancel::new();
        let hits = Arc::new(AtomicUsize::new(0));
        let h = hits.clone();
        let reg = cancel.on_cancel(move || {
            h.fetch_add(1, Ordering::SeqCst);
        });
        cancel.cancel();
        cancel.cancel();
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        drop(reg);
        let h = hits.clone();
        let _late = cancel.on_cancel(move || {
            h.fetch_add(10, Ordering::SeqCst);
        });
        assert_eq!(hits.load(Ordering::SeqCst), 11);
        assert_eq!(
            cancel.check(Instant::now() + std::time::Duration::from_secs(5)),
            Err(OcrError::Cancelled)
        );
    }

    #[test]
    fn dropped_registration_is_not_called() {
        let cancel = OcrCancel::new();
        let hits = Arc::new(AtomicUsize::new(0));
        let h = hits.clone();
        drop(cancel.on_cancel(move || {
            h.fetch_add(1, Ordering::SeqCst);
        }));
        cancel.cancel();
        assert_eq!(hits.load(Ordering::SeqCst), 0);
        let fresh = OcrCancel::new();
        assert_eq!(fresh.check(Instant::now()), Err(OcrError::DeadlineExceeded));
    }
}
