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

    /// 和 `other` 的交集；不相交（或只有边相接）时为 None。
    pub fn intersection(&self, other: &PhysicalRect) -> Option<PhysicalRect> {
        let left = i64::from(self.x).max(i64::from(other.x));
        let top = i64::from(self.y).max(i64::from(other.y));
        let right = (i64::from(self.x) + i64::from(self.width))
            .min(i64::from(other.x) + i64::from(other.width));
        let bottom = (i64::from(self.y) + i64::from(self.height))
            .min(i64::from(other.y) + i64::from(other.height));
        (right > left && bottom > top).then(|| PhysicalRect {
            x: left as i32,
            y: top as i32,
            width: (right - left) as u32,
            height: (bottom - top) as u32,
        })
    }
}

/// Kotone 自己截前台窗口时的取景（开口时的截图、读聊天、`game.look`、自动化）：客户区和它所在
/// 显示器（系统按面积最大的那块给出）的交集。窗口没跨屏时就是整个客户区；跨屏时只截它在主要那块
/// 屏上的部分——仍然只截这个窗口，不退化成桌面截图。外部工具服务仍走严格的
/// [`WindowCapturer::capture_foreground_client`]（跨屏、超限就拒绝）。
pub fn capture_view(client: &PhysicalRect, monitor: &PhysicalRect) -> Option<PhysicalRect> {
    client.intersection(monitor)
}

/// 超过像素上限时等比缩小后的尺寸（宽高比不变、都至少 1，且宽 × 高不超过上限）；没超就原样。
pub fn fit_size(width: u32, height: u32, max_pixels: u64) -> (u32, u32) {
    let pixels = u64::from(width) * u64::from(height);
    if pixels <= max_pixels || width == 0 || height == 0 {
        return (width, height);
    }
    let scale = (max_pixels as f64 / pixels as f64).sqrt();
    let mut w = ((f64::from(width) * scale).floor() as u32).max(1);
    let mut h = ((f64::from(height) * scale).floor() as u32).max(1);
    // 浮点误差兜底：仍超就再各减一点。
    while u64::from(w) * u64::from(h) > max_pixels && (w > 1 || h > 1) {
        if w >= h {
            w -= 1;
        } else {
            h -= 1;
        }
    }
    (w, h)
}

/// RGBA8 按面积平均缩小（照 OpenCV `INTER_AREA`：缩小时每个输出像素取它覆盖的源像素的加权平均，
/// 文字边缘不会像最近邻那样断笔、也不像 Lanczos 那样有振铃，适合缩小后再做文字识别）。
/// `out_w`/`out_h` 不大于原尺寸。逐行输出、只留一行的累加：不为整张图开中间缓冲（带鱼屏
/// 5120×2160 那样开要多占一百多 MB，玩家开着游戏时内存吃紧）。
pub fn downscale_area(rgba: &[u8], width: u32, height: u32, out_w: u32, out_h: u32) -> Vec<u8> {
    let (w, h) = (width as usize, height as usize);
    let (ow, oh) = (out_w.max(1) as usize, out_h.max(1) as usize);
    debug_assert_eq!(rgba.len(), w * h * 4);
    if ow == w && oh == h {
        return rgba.to_vec();
    }
    let xs = area_taps(w, ow);
    let ys = area_taps(h, oh);
    let mut out = vec![0u8; ow * oh * 4];
    let mut acc = vec![0f32; ow * 4];
    for (oy, y_taps) in ys.iter().enumerate() {
        acc.fill(0.0);
        for &(sy, wy) in y_taps {
            let row = &rgba[sy * w * 4..(sy + 1) * w * 4];
            for (ox, x_taps) in xs.iter().enumerate() {
                let dst = &mut acc[ox * 4..ox * 4 + 4];
                for &(sx, wx) in x_taps {
                    let weight = wx * wy;
                    for (a, &p) in dst.iter_mut().zip(&row[sx * 4..sx * 4 + 4]) {
                        *a += f32::from(p) * weight;
                    }
                }
            }
        }
        for (o, a) in out[oy * ow * 4..(oy + 1) * ow * 4].iter_mut().zip(&acc) {
            *o = a.round().clamp(0.0, 255.0) as u8;
        }
    }
    out
}

/// 面积平均的抽头：每个输出列 / 行覆盖的源像素与权重（边缘按覆盖比例，权重和为 1）。
fn area_taps(src: usize, dst: usize) -> Vec<Vec<(usize, f32)>> {
    let ratio = src as f64 / dst as f64;
    (0..dst)
        .map(|d| {
            let start = d as f64 * ratio;
            let end = (d as f64 + 1.0) * ratio;
            let mut taps = Vec::new();
            let mut s = start.floor() as usize;
            while (s as f64) < end && s < src {
                let lo = (s as f64).max(start);
                let hi = ((s + 1) as f64).min(end);
                if hi > lo {
                    taps.push((s, ((hi - lo) / ratio) as f32));
                }
                s += 1;
            }
            taps
        })
        .collect()
}

/// Kotone 自己用的前台截图（见 [`capture_view`]）：`image.client_rect` 仍是完整客户区，
/// `view` 是实际截下的那块屏幕区域，图像可能比它小（`scale` = 图像像素 / 屏幕物理像素，≤ 1）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewCapture {
    pub image: CapturedImage,
    pub view: PhysicalRect,
}

impl ViewCapture {
    /// 图像像素 / 屏幕物理像素（没缩小时为 1）。
    pub fn scale(&self) -> f64 {
        f64::from(self.image.width) / f64::from(self.view.width.max(1))
    }

    /// 图里的一点（图像像素，可以是小数）→ 屏幕物理像素（落在取景区域内）。
    pub fn image_to_screen(&self, px: f64, py: f64) -> (i32, i32) {
        let w = f64::from(self.image.width.max(1));
        let h = f64::from(self.image.height.max(1));
        view_ratio_to_screen(&self.view, px / w, py / h)
    }
}

/// 取景区域里的比例（0–1）→ 屏幕物理像素（不越出取景区域）。
pub fn view_ratio_to_screen(view: &PhysicalRect, x: f64, y: f64) -> (i32, i32) {
    let w = view.width.max(1) as i32;
    let h = view.height.max(1) as i32;
    let px = view.x + ((x.clamp(0.0, 1.0) * f64::from(w)) as i32).min(w - 1);
    let py = view.y + ((y.clamp(0.0, 1.0) * f64::from(h)) as i32).min(h - 1);
    (px, py)
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

    fn rect(x: i32, y: i32, width: u32, height: u32) -> PhysicalRect {
        PhysicalRect {
            x,
            y,
            width,
            height,
        }
    }

    const FOUR_K: u64 = 3840 * 2160;

    #[test]
    fn a_window_spanning_two_monitors_is_cut_to_the_main_one() {
        // 玩家两块 4K 屏并排；窗口客户区 6400×2160，大部分在左边那块（系统给出的就是左屏）。
        let left = rect(0, 0, 3840, 2160);
        let client = rect(0, 0, 6400, 2160);
        let view = capture_view(&client, &left).unwrap();
        assert_eq!(view, left);
        assert!(view.pixel_count() <= FOUR_K);
        // 窗口在右屏为主、左边露出一截：只截右屏上的部分。
        let right = rect(3840, 0, 3840, 2160);
        let client = rect(3000, 100, 4000, 1500);
        assert_eq!(
            capture_view(&client, &right).unwrap(),
            rect(3840, 100, 3160, 1500)
        );
        // 没跨屏：整个客户区。
        let inside = rect(100, 100, 1280, 720);
        assert_eq!(capture_view(&inside, &left).unwrap(), inside);
        // 不在这块屏上（或只有边相接）：没有可截的。
        assert_eq!(capture_view(&rect(3840, 0, 10, 10), &left), None);
        // 负坐标的屏（主屏左边那块）也对。
        let west = rect(-2560, 0, 2560, 1440);
        assert_eq!(
            capture_view(&rect(-100, 10, 300, 300), &west).unwrap(),
            rect(-100, 10, 100, 300)
        );
    }

    #[test]
    fn oversized_views_shrink_under_the_limit_keeping_the_aspect_ratio() {
        for (w, h) in [
            (5120, 2160),
            (6400, 2160),
            (7680, 4320),
            (3841, 2160),
            (20000, 3),
        ] {
            let (fw, fh) = fit_size(w, h, FOUR_K);
            assert!(
                u64::from(fw) * u64::from(fh) <= FOUR_K,
                "{w}x{h} -> {fw}x{fh}"
            );
            let before = f64::from(w) / f64::from(h);
            let after = f64::from(fw) / f64::from(fh);
            assert!(
                (before - after).abs() / before < 0.01 || fh < 10,
                "{w}x{h} -> {fw}x{fh}"
            );
            assert!(fw >= 1 && fh >= 1);
        }
        // 带鱼屏 5120×2160 缩到 0.87 倍左右，字还看得清。
        let (fw, _) = fit_size(5120, 2160, FOUR_K);
        assert!((4400..=4440).contains(&fw), "{fw}");
        // 不超就不动。
        assert_eq!(fit_size(3840, 2160, FOUR_K), (3840, 2160));
        assert_eq!(fit_size(1920, 1080, FOUR_K), (1920, 1080));
    }

    #[test]
    fn area_downscale_averages_and_keeps_solid_colors() {
        // 2×2 → 1×1：四个像素的平均。
        let px = [
            0, 0, 0, 255, 200, 100, 40, 255, //
            100, 50, 20, 255, 100, 50, 20, 255,
        ];
        assert_eq!(downscale_area(&px, 2, 2, 1, 1), vec![100, 50, 20, 255]);
        // 非整数倍 3 → 2：边上的源像素按覆盖比例分给两个输出像素。
        // 输出 0 覆盖 [0, 1.5)：0×1/1.5 + 90×0.5/1.5 = 30；输出 1 覆盖 [1.5, 3)：90×0.5/1.5 + 180×1/1.5 = 150。
        let row = [0, 0, 0, 255, 90, 90, 90, 255, 180, 180, 180, 255];
        assert_eq!(
            downscale_area(&row, 3, 1, 2, 1),
            vec![30, 30, 30, 255, 150, 150, 150, 255]
        );
        // 纯色缩小后还是纯色（非整数倍也一样）。
        let solid: Vec<u8> = [12u8, 34, 56, 255].repeat(7 * 5);
        let out = downscale_area(&solid, 7, 5, 3, 2);
        assert_eq!(out.len(), 3 * 2 * 4);
        assert!(out.chunks(4).all(|p| p == [12, 34, 56, 255]));
        // 尺寸不变就原样。
        assert_eq!(downscale_area(&solid, 7, 5, 7, 5), solid);
    }

    #[test]
    fn image_points_map_back_to_the_screen_through_the_view() {
        let view = rect(3840, 100, 3200, 1600);
        let capture = |w: u32, h: u32| ViewCapture {
            image: CapturedImage {
                width: w,
                height: h,
                rgba: vec![0; (w * h * 4) as usize],
                client_rect: rect(3000, 100, 4040, 1600),
                dpi: Some(144),
                warnings: Vec::new(),
            },
            view,
        };
        // 没缩小：图像素 = 屏幕像素，从取景左上角算。
        let full = capture(3200, 1600);
        assert_eq!(full.scale(), 1.0);
        assert_eq!(full.image_to_screen(0.0, 0.0), (3840, 100));
        assert_eq!(full.image_to_screen(1600.0, 800.0), (5440, 900));
        // 缩小一半：图上 (800, 400) 是取景中心。
        let half = capture(1600, 800);
        assert_eq!(half.scale(), 0.5);
        assert_eq!(half.image_to_screen(800.0, 400.0), (5440, 900));
        assert_eq!(half.image_to_screen(1599.0, 799.0), (7038, 1698));
        // 不越出取景区域。
        assert_eq!(half.image_to_screen(5000.0, -3.0), (7039, 100));
        // 比例换算和工具里一致。
        assert_eq!(view_ratio_to_screen(&view, 0.5, 0.5), (5440, 900));
    }
}
