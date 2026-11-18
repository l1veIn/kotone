//! `game.look` 的纯逻辑：截图缩放（照 Anthropic computer use 文档）、文字元素编号清单
//! （照 UFO² / OmniParser：`{id, text, box}`），以及“截图像素 → 窗口比例”的换算。

/// 截图长边默认上限（像素）。
pub const DEFAULT_LONG_EDGE: u32 = 1280;
/// 截图总像素默认上限（约 1.15 百万像素，Anthropic 文档的建议值附近）。
pub const DEFAULT_MAX_PIXELS: u64 = 1_150_000;
/// 清单里最多几个元素（从上到下、从左到右）。
pub const MAX_ELEMENTS: usize = 80;

/// 缩放结果：模型看到的图片尺寸与缩放比（`图片像素 = 窗口像素 × factor`）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scale {
    pub width: u32,
    pub height: u32,
    pub factor: f64,
}

/// 缩放比 `min(1, L / 长边, sqrt(P / (w·h)))`（Anthropic 文档），不放大。
pub fn scale_for(width: u32, height: u32, long_edge: u32, max_pixels: u64) -> Scale {
    let (w, h) = (f64::from(width.max(1)), f64::from(height.max(1)));
    let factor = 1f64
        .min(f64::from(long_edge.max(1)) / w.max(h))
        .min((max_pixels.max(1) as f64 / (w * h)).sqrt());
    Scale {
        width: ((w * factor).round() as u32).max(1),
        height: ((h * factor).round() as u32).max(1),
        factor,
    }
}

/// 截图上的一个元素：编号从 1 开始；`bbox` 是**整张截图坐标系**里的 `[x0, y0, x1, y1]`。
#[derive(Debug, Clone, PartialEq)]
pub struct Element {
    pub id: u32,
    pub text: String,
    pub bbox: [u32; 4],
}

impl Element {
    pub fn center(&self) -> (f64, f64) {
        (
            f64::from(self.bbox[0] + self.bbox[2]) / 2.0,
            f64::from(self.bbox[1] + self.bbox[3]) / 2.0,
        )
    }
}

/// 一行识别出的文字：`ratio` 是窗口比例 `[x, y, w, h]`（0–1）。
#[derive(Debug, Clone, PartialEq)]
pub struct TextBox {
    pub text: String,
    pub ratio: [f64; 4],
}

/// 文字行 → 编号清单（截图像素坐标）。空文字丢弃；按从上到下、从左到右编号
/// （同一行的判定：纵向中心相差不到半个字高）；最多 [`MAX_ELEMENTS`] 个。
pub fn elements_from_text(lines: &[TextBox], image_w: u32, image_h: u32) -> Vec<Element> {
    let (w, h) = (f64::from(image_w), f64::from(image_h));
    let mut boxes: Vec<(String, [u32; 4])> = lines
        .iter()
        .filter(|l| !l.text.trim().is_empty())
        .map(|l| {
            let [x, y, bw, bh] = l.ratio;
            // 去掉浮点误差（0.6 × 1000 = 600.000…1）再取整，避免框多出一像素。
            let floor = |v: f64| (v + 1e-6).floor() as u32;
            let ceil = |v: f64| (v - 1e-6).ceil() as u32;
            let x0 = floor(x.clamp(0.0, 1.0) * w);
            let y0 = floor(y.clamp(0.0, 1.0) * h);
            let x1 = ceil((x + bw).clamp(0.0, 1.0) * w).max(x0 + 1);
            let y1 = ceil((y + bh).clamp(0.0, 1.0) * h).max(y0 + 1);
            (
                l.text.trim().to_string(),
                [
                    x0.min(image_w - 1),
                    y0.min(image_h - 1),
                    x1.min(image_w),
                    y1.min(image_h),
                ],
            )
        })
        .collect();
    boxes.sort_by(|a, b| {
        let (ay, by) = (a.1[1] + a.1[3], b.1[1] + b.1[3]);
        let line = (a.1[3] - a.1[1]).min(b.1[3] - b.1[1]).max(2);
        if ay.abs_diff(by) < line {
            a.1[0].cmp(&b.1[0])
        } else {
            ay.cmp(&by)
        }
    });
    boxes
        .into_iter()
        .take(MAX_ELEMENTS)
        .enumerate()
        .map(|(i, (text, bbox))| Element {
            id: i as u32 + 1,
            text,
            bbox,
        })
        .collect()
}

/// 一次 `game.look` 的结果（模型下一步 `game.act` 用它的坐标系）。
#[derive(Debug, Clone, PartialEq)]
pub struct Observation {
    pub id: String,
    /// 截图时客户区的物理像素尺寸（尺寸变了，坐标与编号作废）。
    pub client_width: u32,
    pub client_height: u32,
    /// 整张截图（缩放后）的尺寸：坐标都在这个坐标系里，`zoom` 也一样。
    pub image_width: u32,
    pub image_height: u32,
    pub elements: Vec<Element>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CoordError {
    OutOfImage {
        x: f64,
        y: f64,
        width: u32,
        height: u32,
    },
    UnknownElement(u32),
}

impl std::fmt::Display for CoordError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OutOfImage {
                x,
                y,
                width,
                height,
            } => write!(f, "坐标 [{x}, {y}] 超出截图（{width}×{height}）"),
            Self::UnknownElement(id) => write!(f, "截图上没有编号 {id}"),
        }
    }
}

impl Observation {
    /// 截图像素 → 窗口比例（0–1，取像素中心）。
    pub fn to_ratio(&self, x: f64, y: f64) -> Result<(f64, f64), CoordError> {
        if !(0.0..f64::from(self.image_width)).contains(&x)
            || !(0.0..f64::from(self.image_height)).contains(&y)
        {
            return Err(CoordError::OutOfImage {
                x,
                y,
                width: self.image_width,
                height: self.image_height,
            });
        }
        Ok((
            (x.floor() + 0.5) / f64::from(self.image_width),
            (y.floor() + 0.5) / f64::from(self.image_height),
        ))
    }

    pub fn element(&self, id: u32) -> Result<&Element, CoordError> {
        self.elements
            .iter()
            .find(|e| e.id == id)
            .ok_or(CoordError::UnknownElement(id))
    }

    /// 目标（坐标或编号）→ 窗口比例。
    pub fn resolve(&self, target: super::actions::Target) -> Result<(f64, f64), CoordError> {
        match target {
            super::actions::Target::Coordinate(x, y) => self.to_ratio(x, y),
            super::actions::Target::Element(id) => {
                let (x, y) = self.element(id)?.center();
                self.to_ratio(x, y)
            }
        }
    }
}

/// `zoom` 区域（整张截图坐标）校验：返回窗口比例 `[x, y, w, h]`。
pub fn zoom_region(obs_w: u32, obs_h: u32, zoom: [f64; 4]) -> Result<[f64; 4], String> {
    let [x0, y0, x1, y1] = zoom;
    let (w, h) = (f64::from(obs_w), f64::from(obs_h));
    if !(zoom.iter().all(|v| v.is_finite())
        && 0.0 <= x0
        && x0 < x1
        && x1 <= w
        && 0.0 <= y0
        && y0 < y1
        && y1 <= h)
    {
        return Err(format!(
            "zoom 必须是截图（{obs_w}×{obs_h}）内的 [x0, y0, x1, y1]，且 x0 < x1、y0 < y1"
        ));
    }
    if x1 - x0 < 8.0 || y1 - y0 < 8.0 {
        return Err("zoom 区域太小（至少 8×8 像素）".into());
    }
    Ok([x0 / w, y0 / h, (x1 - x0) / w, (y1 - y0) / h])
}
