//! 外部工具服务（M1）的领域端口：窗口观察、前台可见客户区捕获与实例发现宿主。
//!
//! 这里只定义类型与端口；Windows 实现位于 `kotone-platform-windows`，
//! HTTP 服务、产物存储与协议位于 `kotone-tools`。core 不依赖 HTTP / Windows / Tauri。
//!
//! 坐标约定：所有矩形均为**物理像素**（不是 Tauri 逻辑像素），屏幕坐标原点为
//! 主显示器左上角；`client_rect` 是窗口客户区在屏幕上的物理位置。

use std::fmt;
use std::io;
use std::path::Path;

/// 窗口与其所属进程的身份。`handle` 是平台窗口句柄的整数值（Windows 为 HWND）。
///
/// HWND 可以被系统复用，因此身份同时绑定 PID 与进程创建时间；
/// 捕获前必须重新校验三者，避免把旧引用套到新窗口或新进程上。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WindowIdentity {
    pub handle: u64,
    pub pid: u32,
    /// 进程创建时间（平台原始单位；Windows 为 FILETIME 100ns）。无法查询时为 None，
    /// 此时身份只绑定句柄与 PID（较弱），由调用方在结果中如实标注。
    pub process_started_at: Option<u64>,
}

/// 物理像素矩形（屏幕或客户区坐标，由字段语境决定）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhysicalRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl PhysicalRect {
    pub fn pixel_count(&self) -> u64 {
        u64::from(self.width) * u64::from(self.height)
    }

    /// `other` 是否完整包含 `self`（边界相接视为包含）。
    pub fn is_within(&self, other: &PhysicalRect) -> bool {
        let right = i64::from(self.x) + i64::from(self.width);
        let bottom = i64::from(self.y) + i64::from(self.height);
        let other_right = i64::from(other.x) + i64::from(other.width);
        let other_bottom = i64::from(other.y) + i64::from(other.height);
        self.x >= other.x && self.y >= other.y && right <= other_right && bottom <= other_bottom
    }
}

/// 一次观察到的顶层窗口元数据。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowInfo {
    pub identity: WindowIdentity,
    pub title: String,
    pub class_name: String,
    /// 可执行文件名（不含目录，如 `League of Legends.exe`）；无权限时为 None。
    pub process_name: Option<String>,
    pub visible: bool,
    pub minimized: bool,
    /// DWM 隐藏（cloaked）的窗口虽然 visible 但不在屏幕上呈现。
    pub cloaked: bool,
    pub foreground: bool,
    /// 客户区在屏幕上的物理像素位置；最小化或无法查询时为 None。
    pub client_rect: Option<PhysicalRect>,
    /// 客户区所在显示器的物理像素范围；无法查询时为 None。
    pub monitor_rect: Option<PhysicalRect>,
    pub dpi: Option<u32>,
}

/// 捕获结果：紧密排列的 RGBA8 像素（行优先，无行填充，alpha 恒为 255）。
#[derive(Clone, PartialEq, Eq)]
pub struct CapturedImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    /// 捕获时客户区的屏幕物理坐标。
    pub client_rect: PhysicalRect,
    pub dpi: Option<u32>,
    /// 机器可读的警告码（如 `UNIFORM_PIXELS`），由后端如实给出。
    pub warnings: Vec<CaptureWarning>,
}

impl fmt::Debug for CapturedImage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CapturedImage")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("bytes", &self.rgba.len())
            .field("client_rect", &self.client_rect)
            .field("dpi", &self.dpi)
            .field("warnings", &self.warnings)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureWarning {
    pub code: String,
    pub message: String,
}

impl CaptureWarning {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

/// 捕获后端的自我描述：方法名与必须告知调用方的边界。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureBackendInfo {
    /// 稳定的方法标识，如 `gdi-screen-foreground-client`。
    pub method: String,
    /// 是否在当前平台可用（不可用时相关工具在 capabilities 里标注未就绪）。
    pub available: bool,
    pub unavailable_reason: Option<String>,
    /// 人类可读的限制说明，原样进入 capabilities 与捕获结果。
    pub limitations: Vec<String>,
}

/// 窗口观察/捕获的失败分类。每一类映射到协议里独立的错误码，
/// 不允许把“窗口失效”伪装成“图像没看清”。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WindowToolError {
    /// 句柄已失效（窗口关闭）。
    Gone,
    /// 句柄仍有效但 PID 或进程创建时间不同（句柄被复用 / 进程已更换）。
    IdentityChanged,
    Minimized,
    NotVisible,
    /// 目标不是前台窗口；GDI 前台模式拒绝捕获，且不会替调用方抢焦点。
    NotForeground,
    /// 客户区为空、跨显示器或超出显示器范围等。
    InvalidGeometry(String),
    TooLarge {
        pixels: u64,
        max_pixels: u64,
    },
    /// 捕获期间目标失去前台、移动、缩放或最小化，结果已丢弃。
    ChangedDuringCapture(String),
    Unsupported(String),
    Platform(String),
}

impl fmt::Display for WindowToolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Gone => write!(f, "目标窗口已关闭"),
            Self::IdentityChanged => write!(f, "窗口句柄已被其他窗口或进程复用"),
            Self::Minimized => write!(f, "目标窗口已最小化"),
            Self::NotVisible => write!(f, "目标窗口不可见"),
            Self::NotForeground => write!(f, "目标窗口不是前台窗口"),
            Self::InvalidGeometry(reason) => write!(f, "窗口几何无效: {reason}"),
            Self::TooLarge { pixels, max_pixels } => {
                write!(f, "图像像素数 {pixels} 超过上限 {max_pixels}")
            }
            Self::ChangedDuringCapture(reason) => write!(f, "捕获期间目标发生变化: {reason}"),
            Self::Unsupported(reason) => write!(f, "当前平台不支持: {reason}"),
            Self::Platform(reason) => write!(f, "平台调用失败: {reason}"),
        }
    }
}

impl std::error::Error for WindowToolError {}

/// 窗口观察端口：只读，不改变焦点、不显示或移动任何窗口。
pub trait WindowObserver: Send + Sync {
    /// 枚举可见的顶层窗口。
    fn list_windows(&self) -> Result<Vec<WindowInfo>, WindowToolError>;
    /// 当前前台窗口；没有前台窗口时为 Ok(None)。
    fn foreground_window(&self) -> Result<Option<WindowInfo>, WindowToolError>;
    /// 重新校验身份并返回最新元数据（`Gone` / `IdentityChanged` 表示引用失效）。
    fn inspect(&self, identity: &WindowIdentity) -> Result<WindowInfo, WindowToolError>;
}

/// 前台可见客户区捕获端口。实现必须在每次调用中重新校验身份与前台状态，
/// 不允许退化为截取整个桌面，并在成功或失败时释放全部原生资源。
pub trait WindowCapturer: Send + Sync {
    fn backend_info(&self) -> CaptureBackendInfo;
    fn capture_foreground_client(
        &self,
        identity: &WindowIdentity,
        max_pixels: u64,
    ) -> Result<CapturedImage, WindowToolError>;
}

/// 服务实例发现所需的宿主能力：会话/进程身份与仅当前用户可访问的文件。
pub trait ToolHost: Send + Sync {
    /// 当前进程所在的登录会话 ID（Windows 会话）。
    fn session_id(&self) -> Option<u32>;
    /// 指定进程的创建时间；进程不存在或无法查询时为 None。
    fn process_started_at(&self, pid: u32) -> Option<u64>;
    /// 创建目录并限制为当前用户（及 SYSTEM）访问。
    fn prepare_private_dir(&self, dir: &Path) -> io::Result<()>;
    /// 原子写入仅当前用户可访问的文件。
    fn write_private_file(&self, path: &Path, bytes: &[u8]) -> io::Result<()>;
    /// 校验文件属于当前用户且未向其他账户授予访问权限。
    fn verify_private_file(&self, path: &Path) -> io::Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_containment_uses_wide_arithmetic() {
        let monitor = PhysicalRect {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
        };
        let inside = PhysicalRect {
            x: 10,
            y: 10,
            width: 1910,
            height: 1070,
        };
        assert!(inside.is_within(&monitor));
        let outside = PhysicalRect { x: 11, ..inside };
        assert!(!outside.is_within(&monitor));
        let negative = PhysicalRect {
            x: -1,
            y: 0,
            width: 10,
            height: 10,
        };
        assert!(!negative.is_within(&monitor));
        let huge = PhysicalRect {
            x: i32::MAX,
            y: 0,
            width: u32::MAX,
            height: 1,
        };
        assert!(!huge.is_within(&monitor));
        assert_eq!(huge.pixel_count(), u64::from(u32::MAX));
    }
}
