//! 截图的缩放、编号标签与编码。
//!
//! 标签样式照抄 OmniParser（代码 CC-BY-4.0，Microsoft）`util/box_annotator.py` 与 UFO²（MIT）：
//! 框用调色板颜色描边；编号贴在框旁的同色底块上，底色亮度 > 160 用黑字，否则白字；
//! 编号位置依次试“框上方左对齐、框外左侧、框外右侧、框上方右对齐”，取第一个不越界、
//! 不压住别的框与标签的位置，都不行就用第一个（夹在图内）。
//! 数字用内置的 3×5 点阵放大绘制（只有 0–9，不引入字体文件）。

use image_codec::imageops::FilterType;
use image_codec::{ImageBuffer, Rgba};

use crate::image::RgbaImage;

/// 调色板（OmniParser 经 supervision 的默认调色板风格；按编号循环取色）。
const PALETTE: [[u8; 3]; 10] = [
    [0xA3, 0x51, 0xFB],
    [0xE6, 0x19, 0x4B],
    [0xF5, 0x82, 0x31],
    [0xFF, 0xE1, 0x19],
    [0xBF, 0xEF, 0x45],
    [0x3C, 0xB4, 0x4B],
    [0x42, 0xD4, 0xF4],
    [0x43, 0x63, 0xD8],
    [0xF0, 0x32, 0xE6],
    [0x46, 0x99, 0x90],
];

/// 3×5 数字点阵（每行 3 位，高位在左）。
const DIGITS: [[u8; 5]; 10] = [
    [0b111, 0b101, 0b101, 0b101, 0b111],
    [0b010, 0b110, 0b010, 0b010, 0b111],
    [0b111, 0b001, 0b111, 0b100, 0b111],
    [0b111, 0b001, 0b111, 0b001, 0b111],
    [0b101, 0b101, 0b111, 0b001, 0b001],
    [0b111, 0b100, 0b111, 0b001, 0b111],
    [0b111, 0b100, 0b111, 0b101, 0b111],
    [0b111, 0b001, 0b001, 0b001, 0b001],
    [0b111, 0b101, 0b111, 0b101, 0b111],
    [0b111, 0b101, 0b111, 0b001, 0b111],
];
/// 点阵放大倍数、字间距与内边距（像素）。
const DOT: u32 = 2;
const GAP: u32 = 2;
const PAD: u32 = 2;
/// 框线宽。
const THICKNESS: u32 = 2;

pub fn color_for(id: u32) -> [u8; 3] {
    PALETTE[(id.saturating_sub(1) as usize) % PALETTE.len()]
}

/// 底色上用黑字还是白字（OmniParser：亮度 > 160 用黑字）。
pub fn text_color(bg: [u8; 3]) -> [u8; 3] {
    let luminance = 0.299 * f64::from(bg[0]) + 0.587 * f64::from(bg[1]) + 0.114 * f64::from(bg[2]);
    if luminance > 160.0 {
        [0, 0, 0]
    } else {
        [255, 255, 255]
    }
}

/// 编号标签的尺寸。
pub fn label_size(id: u32) -> (u32, u32) {
    let n = id.to_string().len() as u32;
    (
        PAD * 2 + n * 3 * DOT + n.saturating_sub(1) * GAP,
        PAD * 2 + 5 * DOT,
    )
}

type Rect = [i64; 4]; // x0, y0, x1, y1（右下不含）

fn overlaps(a: &Rect, b: &Rect) -> bool {
    a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3]
}

/// 给每个框选标签位置（左上角）。`boxes` 是图内像素 `[x0, y0, x1, y1]`。
pub fn place_labels(boxes: &[(u32, [u32; 4])], width: u32, height: u32) -> Vec<(i64, i64)> {
    let (w, h) = (i64::from(width), i64::from(height));
    let mut taken: Vec<Rect> = boxes
        .iter()
        .map(|(_, b)| b.map(i64::from))
        .collect::<Vec<_>>();
    let mut out = Vec::with_capacity(boxes.len());
    for (index, (id, b)) in boxes.iter().enumerate() {
        let (lw, lh) = label_size(*id);
        let (lw, lh) = (i64::from(lw), i64::from(lh));
        let [x0, y0, x1, _] = b.map(i64::from);
        let candidates = [
            (x0, y0 - lh),      // 上方左对齐
            (x0 - lw, y0),      // 框外左侧
            (x1, y0),           // 框外右侧
            (x1 - lw, y0 - lh), // 上方右对齐
        ];
        let fits = |&(x, y): &(i64, i64)| {
            let r = [x, y, x + lw, y + lh];
            x >= 0
                && y >= 0
                && x + lw <= w
                && y + lh <= h
                && !taken
                    .iter()
                    .enumerate()
                    .any(|(i, t)| i != index && overlaps(&r, t))
        };
        let (x, y) = candidates.iter().copied().find(fits).unwrap_or((
            candidates[0].0.clamp(0, (w - lw).max(0)),
            candidates[0].1.clamp(0, (h - lh).max(0)),
        ));
        taken.push([x, y, x + lw, y + lh]);
        out.push((x, y));
    }
    out
}

struct Canvas<'a> {
    rgba: &'a mut [u8],
    width: u32,
    height: u32,
}

impl Canvas<'_> {
    fn fill(&mut self, x: i64, y: i64, w: i64, h: i64, color: [u8; 3]) {
        let x0 = x.clamp(0, i64::from(self.width));
        let y0 = y.clamp(0, i64::from(self.height));
        let x1 = (x + w).clamp(0, i64::from(self.width));
        let y1 = (y + h).clamp(0, i64::from(self.height));
        for py in y0..y1 {
            for px in x0..x1 {
                let i = ((py as usize) * (self.width as usize) + px as usize) * 4;
                self.rgba[i..i + 3].copy_from_slice(&color);
                self.rgba[i + 3] = 255;
            }
        }
    }

    fn frame(&mut self, b: [u32; 4], color: [u8; 3]) {
        let [x0, y0, x1, y1] = b.map(i64::from);
        let t = i64::from(THICKNESS);
        self.fill(x0, y0, x1 - x0, t, color);
        self.fill(x0, y1 - t, x1 - x0, t, color);
        self.fill(x0, y0, t, y1 - y0, color);
        self.fill(x1 - t, y0, t, y1 - y0, color);
    }

    fn label(&mut self, id: u32, x: i64, y: i64, bg: [u8; 3]) {
        let (lw, lh) = label_size(id);
        self.fill(x, y, i64::from(lw), i64::from(lh), bg);
        let fg = text_color(bg);
        let mut cx = x + i64::from(PAD);
        for ch in id.to_string().bytes() {
            let glyph = DIGITS[usize::from(ch - b'0')];
            for (row, bits) in glyph.iter().enumerate() {
                for col in 0..3 {
                    if bits & (0b100 >> col) != 0 {
                        self.fill(
                            cx + i64::from(col * DOT),
                            y + i64::from(PAD) + row as i64 * i64::from(DOT),
                            i64::from(DOT),
                            i64::from(DOT),
                            fg,
                        );
                    }
                }
            }
            cx += i64::from(3 * DOT + GAP);
        }
    }
}

/// 在图上画框与编号：`boxes` 是 (编号, 图内像素 `[x0, y0, x1, y1]`)。
pub fn annotate(image: &RgbaImage, boxes: &[(u32, [u32; 4])]) -> RgbaImage {
    let (width, height) = (image.width(), image.height());
    let mut rgba = image.pixels().to_vec();
    let positions = place_labels(boxes, width, height);
    let mut canvas = Canvas {
        rgba: &mut rgba,
        width,
        height,
    };
    for (id, b) in boxes {
        canvas.frame(*b, color_for(*id));
    }
    for ((id, _), (x, y)) in boxes.iter().zip(positions) {
        canvas.label(*id, x, y, color_for(*id));
    }
    RgbaImage::new(width, height, rgba).expect("same size as the input")
}

/// 缩放到 `width × height`（三角滤波；尺寸相同时原样返回）。
pub fn resize(image: &RgbaImage, width: u32, height: u32) -> RgbaImage {
    if image.width() == width && image.height() == height {
        return image.clone();
    }
    let buffer: ImageBuffer<Rgba<u8>, Vec<u8>> =
        ImageBuffer::from_raw(image.width(), image.height(), image.pixels().to_vec())
            .expect("RgbaImage is a valid RGBA8 buffer");
    let out =
        image_codec::imageops::resize(&buffer, width.max(1), height.max(1), FilterType::Triangle);
    RgbaImage::new(out.width(), out.height(), out.into_raw()).expect("resize keeps RGBA8")
}

/// JPEG 编码（给模型看的截图：比 PNG 小得多）。
pub fn encode_jpeg(image: &RgbaImage, quality: u8) -> Result<Vec<u8>, String> {
    let rgb: Vec<u8> = image
        .pixels()
        .chunks_exact(4)
        .flat_map(|p| [p[0], p[1], p[2]])
        .collect();
    let mut out = Vec::new();
    image_codec::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality.clamp(1, 100))
        .encode(
            &rgb,
            image.width(),
            image.height(),
            image_codec::ExtendedColorType::Rgb8,
        )
        .map_err(|e| format!("JPEG 编码失败：{e}"))?;
    Ok(out)
}
