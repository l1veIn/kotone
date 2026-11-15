//! `image.preprocess`：显式预设或操作列表，生成新的不可变产物；输入产物不变。
//!
//! 只有 `scale` 改变几何：输出尺寸取 `round(输入 × factor)`，实际比例为
//! `输出 / 输入`（分别记录 X/Y），坐标按“像素边界”映射：输入点 = 输出点 / 实际比例。
//! 其余操作逐像素变换颜色，不改变几何。所有操作都是确定性的。

use serde::Deserialize;
use serde_json::{json, Value};

use crate::image::RgbaImage;

pub const MAX_OPERATIONS: usize = 8;
pub const MIN_SCALE: f64 = 0.25;
pub const MAX_SCALE: f64 = 4.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Filter {
    Nearest,
    Linear,
    Cubic,
}

impl Filter {
    fn as_str(self) -> &'static str {
        match self {
            Self::Nearest => "nearest",
            Self::Linear => "linear",
            Self::Cubic => "cubic",
        }
    }
}

/// 单个操作（JSON：`{"op": "...", ...}`，不接受额外字段）。
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase", deny_unknown_fields)]
pub enum Operation {
    Scale {
        factor: f64,
        #[serde(default = "default_filter")]
        filter: Filter,
    },
    Grayscale,
    Invert,
    #[serde(rename_all = "camelCase")]
    AutoContrast {
        #[serde(default)]
        clip_percent: f64,
    },
    Threshold {
        value: u8,
    },
}

fn default_filter() -> Filter {
    Filter::Cubic
}

impl Operation {
    pub fn to_json(&self) -> Value {
        match self {
            Self::Scale { factor, filter } => {
                json!({"op": "scale", "factor": factor, "filter": filter.as_str()})
            }
            Self::Grayscale => json!({"op": "grayscale"}),
            Self::Invert => json!({"op": "invert"}),
            Self::AutoContrast { clip_percent } => {
                json!({"op": "autoContrast", "clipPercent": clip_percent})
            }
            Self::Threshold { value } => json!({"op": "threshold", "value": value}),
        }
    }

    fn validate(&self) -> Result<(), String> {
        match self {
            Self::Scale { factor, .. }
                if !factor.is_finite() || !(MIN_SCALE..=MAX_SCALE).contains(factor) =>
            {
                Err(format!("scale.factor 必须在 {MIN_SCALE}..={MAX_SCALE}"))
            }
            Self::AutoContrast { clip_percent }
                if !clip_percent.is_finite() || !(0.0..=10.0).contains(clip_percent) =>
            {
                Err("autoContrast.clipPercent 必须在 0..=10".into())
            }
            _ => Ok(()),
        }
    }
}

/// 预设：展开为明确的操作列表（结果中返回展开后的列表）。
pub const PRESETS: &[&str] = &[
    "none",
    "upscale-2x-cubic",
    "upscale-3x-cubic",
    "grayscale",
    "gray-autocontrast-2x",
];

pub fn preset_operations(name: &str) -> Option<Vec<Operation>> {
    Some(match name {
        "none" => vec![],
        "upscale-2x-cubic" => vec![Operation::Scale {
            factor: 2.0,
            filter: Filter::Cubic,
        }],
        "upscale-3x-cubic" => vec![Operation::Scale {
            factor: 3.0,
            filter: Filter::Cubic,
        }],
        "grayscale" => vec![Operation::Grayscale],
        "gray-autocontrast-2x" => vec![
            Operation::Grayscale,
            Operation::AutoContrast { clip_percent: 1.0 },
            Operation::Scale {
                factor: 2.0,
                filter: Filter::Cubic,
            },
        ],
        _ => return None,
    })
}

/// 校验操作列表并预计算输出尺寸（不执行像素运算）。
pub fn plan(
    width: u32,
    height: u32,
    operations: &[Operation],
    max_pixels: u64,
) -> Result<(u32, u32), String> {
    if operations.len() > MAX_OPERATIONS {
        return Err(format!("operations 最多 {MAX_OPERATIONS} 个"));
    }
    let (mut w, mut h) = (width, height);
    for op in operations {
        op.validate()?;
        if let Operation::Scale { factor, .. } = op {
            let nw = (f64::from(w) * factor).round().max(1.0);
            let nh = (f64::from(h) * factor).round().max(1.0);
            if nw * nh > max_pixels as f64 {
                return Err(format!("缩放后尺寸 {nw}x{nh} 超过像素上限 {max_pixels}"));
            }
            w = nw as u32;
            h = nh as u32;
        }
    }
    Ok((w, h))
}

fn luma(p: &[u8]) -> u8 {
    // BT.601（与 OpenCV COLOR_RGB2GRAY 相同系数）
    (0.299 * f64::from(p[0]) + 0.587 * f64::from(p[1]) + 0.114 * f64::from(p[2])).round() as u8
}

fn map_pixels(image: &RgbaImage, f: impl Fn(&[u8]) -> [u8; 4]) -> RgbaImage {
    let mut out = Vec::with_capacity(image.pixels().len());
    for p in image.pixels().chunks_exact(4) {
        out.extend_from_slice(&f(p));
    }
    RgbaImage::new(image.width(), image.height(), out).expect("same dims")
}

fn cubic_weight(x: f64) -> f64 {
    // OpenCV INTER_CUBIC：a = -0.75
    const A: f64 = -0.75;
    let x = x.abs();
    if x <= 1.0 {
        ((A + 2.0) * x - (A + 3.0)) * x * x + 1.0
    } else if x < 2.0 {
        ((A * x - 5.0 * A) * x + 8.0 * A) * x - 4.0 * A
    } else {
        0.0
    }
}

/// 一维采样表：每个输出坐标对应的 (源索引, 权重) 列表。
fn taps(src: u32, dst: u32, filter: Filter) -> Vec<Vec<(usize, f64)>> {
    let scale = f64::from(src) / f64::from(dst);
    let max = src as i64 - 1;
    (0..dst)
        .map(|d| match filter {
            Filter::Nearest => {
                let s = ((f64::from(d) * scale).floor() as i64).clamp(0, max);
                vec![(s as usize, 1.0)]
            }
            Filter::Linear => {
                let s = ((f64::from(d) + 0.5) * scale - 0.5).max(0.0);
                let i0 = (s.floor() as i64).min(max);
                let i1 = (i0 + 1).min(max);
                let w = (s - i0 as f64).clamp(0.0, 1.0);
                vec![(i0 as usize, 1.0 - w), (i1 as usize, w)]
            }
            Filter::Cubic => {
                let s = (f64::from(d) + 0.5) * scale - 0.5;
                let i = s.floor() as i64;
                let t = s - i as f64;
                (-1..=2)
                    .map(|k| {
                        let idx = (i + k).clamp(0, max) as usize;
                        (idx, cubic_weight(t - k as f64))
                    })
                    .collect()
            }
        })
        .collect()
}

fn resize(image: &RgbaImage, dw: u32, dh: u32, filter: Filter) -> RgbaImage {
    let (sw, sh) = (image.width(), image.height());
    let xs = taps(sw, dw, filter);
    let ys = taps(sh, dh, filter);
    let src = image.pixels();
    // 先水平（f64 中间结果），再垂直
    let mut mid = vec![0f64; dw as usize * sh as usize * 4];
    for y in 0..sh as usize {
        for (x, tap) in xs.iter().enumerate() {
            for c in 0..4 {
                let mut acc = 0.0;
                for &(sx, w) in tap {
                    acc += w * f64::from(src[(y * sw as usize + sx) * 4 + c]);
                }
                mid[(y * dw as usize + x) * 4 + c] = acc;
            }
        }
    }
    let mut out = vec![0u8; dw as usize * dh as usize * 4];
    for (y, tap) in ys.iter().enumerate() {
        for x in 0..dw as usize {
            for c in 0..4 {
                let mut acc = 0.0;
                for &(sy, w) in tap {
                    acc += w * mid[(sy * dw as usize + x) * 4 + c];
                }
                out[(y * dw as usize + x) * 4 + c] = acc.round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    RgbaImage::new(dw, dh, out).expect("resize dims")
}

/// 执行操作列表。返回新图像与累计实际比例 (sx, sy)：输出像素 / 输入像素。
pub fn apply(image: &RgbaImage, operations: &[Operation]) -> (RgbaImage, f64, f64) {
    let mut current = image.clone();
    let (mut sx, mut sy) = (1.0f64, 1.0f64);
    for op in operations {
        current = match *op {
            Operation::Scale { factor, filter } => {
                let w = (f64::from(current.width()) * factor).round().max(1.0) as u32;
                let h = (f64::from(current.height()) * factor).round().max(1.0) as u32;
                sx *= f64::from(w) / f64::from(current.width());
                sy *= f64::from(h) / f64::from(current.height());
                resize(&current, w, h, filter)
            }
            Operation::Grayscale => map_pixels(&current, |p| {
                let l = luma(p);
                [l, l, l, p[3]]
            }),
            Operation::Invert => {
                map_pixels(&current, |p| [255 - p[0], 255 - p[1], 255 - p[2], p[3]])
            }
            Operation::Threshold { value } => map_pixels(&current, |p| {
                let v = if luma(p) >= value { 255 } else { 0 };
                [v, v, v, p[3]]
            }),
            Operation::AutoContrast { clip_percent } => {
                let mut hist = [0u64; 256];
                for p in current.pixels().chunks_exact(4) {
                    hist[luma(p) as usize] += 1;
                }
                let total: u64 = hist.iter().sum();
                let clip = (total as f64 * clip_percent / 100.0) as u64;
                let (mut lo, mut acc) = (0usize, 0u64);
                while lo < 255 && acc + hist[lo] <= clip {
                    acc += hist[lo];
                    lo += 1;
                }
                let (mut hi, mut acc) = (255usize, 0u64);
                while hi > 0 && acc + hist[hi] <= clip {
                    acc += hist[hi];
                    hi -= 1;
                }
                if hi <= lo {
                    current.clone()
                } else {
                    let (lo, span) = (lo as f64, (hi - lo) as f64);
                    let stretch = |v: u8| {
                        ((f64::from(v) - lo) * 255.0 / span)
                            .round()
                            .clamp(0.0, 255.0) as u8
                    };
                    map_pixels(&current, |p| {
                        [stretch(p[0]), stretch(p[1]), stretch(p[2]), p[3]]
                    })
                }
            }
        };
    }
    (current, sx, sy)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 4]) -> RgbaImage {
        let mut px = Vec::new();
        for y in 0..h {
            for x in 0..w {
                px.extend_from_slice(&f(x, y));
            }
        }
        RgbaImage::new(w, h, px).unwrap()
    }

    #[test]
    fn operations_parse_strictly() {
        let ops: Vec<Operation> = serde_json::from_value(json!([
            {"op": "scale", "factor": 3},
            {"op": "grayscale"},
            {"op": "autoContrast", "clipPercent": 0.5},
            {"op": "threshold", "value": 128},
            {"op": "invert"}
        ]))
        .unwrap();
        assert_eq!(
            ops[0],
            Operation::Scale {
                factor: 3.0,
                filter: Filter::Cubic
            }
        );
        for bad in [
            json!({"op": "scale", "factor": 2, "path": "C:/x"}),
            json!({"op": "blur"}),
            json!({"op": "threshold", "value": 300}),
        ] {
            assert!(serde_json::from_value::<Operation>(bad).is_err());
        }
        assert!(plan(
            10,
            10,
            &[Operation::Scale {
                factor: 5.0,
                filter: Filter::Linear
            }],
            1 << 24
        )
        .is_err());
        assert!(plan(
            4000,
            4000,
            &[Operation::Scale {
                factor: 2.0,
                filter: Filter::Linear
            }],
            16_777_216
        )
        .is_err());
        assert_eq!(
            plan(
                650,
                200,
                &preset_operations("upscale-3x-cubic").unwrap(),
                1 << 24
            )
            .unwrap(),
            (1950, 600)
        );
        assert!(preset_operations("magic").is_none());
    }

    #[test]
    fn nearest_upscale_replicates_pixels_and_reports_exact_ratio() {
        let img = solid(3, 2, |x, y| [x as u8 * 50, y as u8 * 100, 0, 255]);
        let (out, sx, sy) = apply(
            &img,
            &[Operation::Scale {
                factor: 2.0,
                filter: Filter::Nearest,
            }],
        );
        assert_eq!((out.width(), out.height(), sx, sy), (6, 4, 2.0, 2.0));
        for y in 0..4 {
            for x in 0..6 {
                assert_eq!(out.pixel(x, y), img.pixel(x / 2, y / 2));
            }
        }
        // 非整数输出：比例按实际尺寸
        let (out, sx, _) = apply(
            &img,
            &[Operation::Scale {
                factor: 1.5,
                filter: Filter::Linear,
            }],
        );
        assert_eq!(out.width(), 5); // round(4.5) = 5（Rust round 远离零）
        assert!((sx - 5.0 / 3.0).abs() < 1e-12);
    }

    #[test]
    fn cubic_and_linear_preserve_flat_regions_and_input_is_untouched() {
        let img = solid(8, 8, |_, _| [40, 80, 120, 255]);
        let before = img.pixels().to_vec();
        for filter in [Filter::Linear, Filter::Cubic] {
            let (out, _, _) = apply(
                &img,
                &[Operation::Scale {
                    factor: 3.0,
                    filter,
                }],
            );
            assert!(out
                .pixels()
                .chunks_exact(4)
                .all(|p| p == [40, 80, 120, 255]));
        }
        assert_eq!(img.pixels(), &before[..]);
    }

    #[test]
    fn color_operations() {
        let img = solid(2, 1, |x, _| {
            if x == 0 {
                [100, 100, 100, 255]
            } else {
                [200, 50, 0, 255]
            }
        });
        let (g, _, _) = apply(&img, &[Operation::Grayscale]);
        assert_eq!(g.pixel(1, 0), Some([89, 89, 89, 255]));
        let (inv, _, _) = apply(&img, &[Operation::Invert]);
        assert_eq!(inv.pixel(0, 0), Some([155, 155, 155, 255]));
        let (t, _, _) = apply(&img, &[Operation::Threshold { value: 95 }]);
        assert_eq!(t.pixel(0, 0), Some([255, 255, 255, 255]));
        assert_eq!(t.pixel(1, 0), Some([0, 0, 0, 255]));
        let (ac, _, _) = apply(
            &img,
            &[
                Operation::Grayscale,
                Operation::AutoContrast { clip_percent: 0.0 },
            ],
        );
        assert_eq!(ac.pixel(1, 0), Some([0, 0, 0, 255]));
        assert_eq!(ac.pixel(0, 0), Some([255, 255, 255, 255]));
    }
}
