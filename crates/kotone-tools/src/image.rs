//! 不可变 RGBA 图像、裁剪校验与 PNG 编码。
//!
//! 坐标空间 `artifact-physical-pixels`：左上为原点，单位是输入产物的物理像素。

use std::sync::Arc;

use sha2::{Digest, Sha256};

/// 不可变 RGBA8 图像（像素共享只读，裁剪总是生成新缓冲区）。
#[derive(Clone, PartialEq, Eq)]
pub struct RgbaImage {
    width: u32,
    height: u32,
    pixels: Arc<[u8]>,
}

impl std::fmt::Debug for RgbaImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "RgbaImage({}x{})", self.width, self.height)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageError {
    /// 宽或高为 0。
    Empty,
    /// 缓冲区长度与 width*height*4 不一致或乘法溢出。
    BadBuffer {
        expected: Option<u64>,
        actual: usize,
    },
    Encode(String),
}

impl std::fmt::Display for ImageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => write!(f, "图像尺寸为空"),
            Self::BadBuffer { expected, actual } => {
                write!(f, "像素缓冲区长度 {actual} 与尺寸不符（期望 {expected:?}）")
            }
            Self::Encode(e) => write!(f, "PNG 编码失败: {e}"),
        }
    }
}

impl std::error::Error for ImageError {}

impl RgbaImage {
    pub fn new(width: u32, height: u32, pixels: Vec<u8>) -> Result<Self, ImageError> {
        if width == 0 || height == 0 {
            return Err(ImageError::Empty);
        }
        let expected = u64::from(width)
            .checked_mul(u64::from(height))
            .and_then(|n| n.checked_mul(4));
        if expected != Some(pixels.len() as u64) {
            return Err(ImageError::BadBuffer {
                expected,
                actual: pixels.len(),
            });
        }
        Ok(Self {
            width,
            height,
            pixels: pixels.into(),
        })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    pub fn byte_len(&self) -> usize {
        self.pixels.len()
    }

    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let i = ((y as usize) * (self.width as usize) + x as usize) * 4;
        let px = &self.pixels[i..i + 4];
        Some([px[0], px[1], px[2], px[3]])
    }

    /// 按已校验的矩形复制出新图像；原图不变。
    pub fn crop(&self, rect: CropRect) -> RgbaImage {
        debug_assert!(rect.x + rect.width <= self.width && rect.y + rect.height <= self.height);
        let stride = self.width as usize * 4;
        let row_bytes = rect.width as usize * 4;
        let mut out = Vec::with_capacity(row_bytes * rect.height as usize);
        for row in rect.y..rect.y + rect.height {
            let start = row as usize * stride + rect.x as usize * 4;
            out.extend_from_slice(&self.pixels[start..start + row_bytes]);
        }
        RgbaImage {
            width: rect.width,
            height: rect.height,
            pixels: out.into(),
        }
    }
}

/// 已通过边界校验的裁剪矩形（输入图像的物理像素）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CropRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CropError {
    Negative,
    Empty,
    Overflow,
    OutOfBounds { image_width: u32, image_height: u32 },
}

impl CropError {
    pub fn reason(&self) -> &'static str {
        match self {
            Self::Negative => "negative-origin",
            Self::Empty => "empty-rect",
            Self::Overflow => "overflow",
            Self::OutOfBounds { .. } => "out-of-bounds",
        }
    }
}

impl std::fmt::Display for CropError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Negative => write!(f, "rect.x / rect.y 不能为负"),
            Self::Empty => write!(f, "rect.width / rect.height 必须大于 0"),
            Self::Overflow => write!(f, "rect 坐标与尺寸相加溢出"),
            Self::OutOfBounds {
                image_width,
                image_height,
            } => write!(
                f,
                "rect 超出输入产物范围（{image_width}x{image_height} 物理像素）"
            ),
        }
    }
}

/// 校验调用方给出的整数矩形；不做截断或自动修正。
pub fn validate_crop(
    image_width: u32,
    image_height: u32,
    x: i64,
    y: i64,
    width: i64,
    height: i64,
) -> Result<CropRect, CropError> {
    if x < 0 || y < 0 {
        return Err(CropError::Negative);
    }
    if width <= 0 || height <= 0 {
        return Err(CropError::Empty);
    }
    let right = x.checked_add(width).ok_or(CropError::Overflow)?;
    let bottom = y.checked_add(height).ok_or(CropError::Overflow)?;
    if right > i64::from(image_width) || bottom > i64::from(image_height) {
        return Err(CropError::OutOfBounds {
            image_width,
            image_height,
        });
    }
    // 以上已保证全部落在 u32 范围内
    Ok(CropRect {
        x: x as u32,
        y: y as u32,
        width: width as u32,
        height: height as u32,
    })
}

/// 确定性 PNG 编码（RGBA8，快速压缩）。
pub fn encode_png(image: &RgbaImage) -> Result<Vec<u8>, ImageError> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, image.width, image.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Fast);
        let mut writer = encoder
            .write_header()
            .map_err(|e| ImageError::Encode(e.to_string()))?;
        writer
            .write_image_data(&image.pixels)
            .map_err(|e| ImageError::Encode(e.to_string()))?;
        writer
            .finish()
            .map_err(|e| ImageError::Encode(e.to_string()))?;
    }
    Ok(out)
}

/// 标准 base64（带填充）编码。
pub fn encode_base64(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// 标准 base64（带或不带填充）解码。
pub fn decode_base64(text: &str) -> Result<Vec<u8>, String> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(text)
        .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(text))
        .map_err(|e| e.to_string())
}

/// 解码 PNG 为 RGBA8（灰度、RGB、带透明、调色板、16 位都转换成 8 位 RGBA）。
/// `max_side` 限制宽高，防止解码超大图片。
pub fn decode_png(bytes: &[u8], max_side: u32) -> Result<RgbaImage, String> {
    let mut decoder = png::Decoder::new(bytes);
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder
        .read_info()
        .map_err(|e| format!("PNG 无法读取：{e}"))?;
    let (width, height) = (reader.info().width, reader.info().height);
    if width == 0 || height == 0 || width > max_side || height > max_side {
        return Err(format!(
            "图片尺寸 {width}×{height} 超出范围（最大 {max_side}×{max_side}）"
        ));
    }
    let mut buf = vec![0; reader.output_buffer_size()];
    let frame = reader
        .next_frame(&mut buf)
        .map_err(|e| format!("PNG 无法解码：{e}"))?;
    let data = &buf[..frame.buffer_size()];
    let rgba: Vec<u8> = match frame.color_type {
        png::ColorType::Rgba => data.to_vec(),
        png::ColorType::Rgb => data
            .chunks_exact(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => data
            .chunks_exact(2)
            .flat_map(|p| [p[0], p[0], p[0], p[1]])
            .collect(),
        png::ColorType::Grayscale => data.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return Err("PNG 调色板未能展开".into()),
    };
    RgbaImage::new(width, height, rgba).map_err(|e| e.to_string())
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// 测试夹具：每个像素编码自身坐标（R=x, G=y, B=x^y, A=255）。
#[cfg(test)]
pub(crate) fn coordinate_fixture(width: u32, height: u32) -> RgbaImage {
    let mut px = Vec::new();
    for y in 0..height {
        for x in 0..width {
            px.extend_from_slice(&[x as u8, y as u8, (x ^ y) as u8, 255]);
        }
    }
    RgbaImage::new(width, height, px).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_round_trips_and_oversized_images_are_refused() {
        let image = RgbaImage::new(3, 2, (0..24).map(|i| i as u8 * 10).collect()).unwrap();
        let png = encode_png(&image).unwrap();
        let back = decode_png(&png, 16).unwrap();
        assert_eq!((back.width(), back.height()), (3, 2));
        assert_eq!(back.pixels(), image.pixels());
        assert!(decode_png(&png, 2).unwrap_err().contains("超出范围"));
        assert!(decode_png(b"not a png", 16).is_err());
    }

    #[test]
    fn new_rejects_bad_buffers() {
        assert_eq!(RgbaImage::new(0, 1, vec![]), Err(ImageError::Empty));
        assert!(matches!(
            RgbaImage::new(2, 2, vec![0; 15]),
            Err(ImageError::BadBuffer { .. })
        ));
        assert!(matches!(
            RgbaImage::new(u32::MAX, u32::MAX, vec![0; 4]),
            Err(ImageError::BadBuffer { .. })
        ));
    }

    #[test]
    fn crop_copies_exact_pixels_and_preserves_original() {
        let source = coordinate_fixture(16, 10);
        let before = source.pixels().to_vec();
        let rect = validate_crop(16, 10, 3, 2, 5, 4).unwrap();
        let crop = source.crop(rect);
        assert_eq!((crop.width(), crop.height()), (5, 4));
        for y in 0..4 {
            for x in 0..5 {
                let (sx, sy) = (x + 3, y + 2);
                assert_eq!(
                    crop.pixel(x, y).unwrap(),
                    [sx as u8, sy as u8, (sx ^ sy) as u8, 255]
                );
            }
        }
        assert_eq!(source.pixels(), &before[..], "原图不可被修改");
        // 整图与单像素边界
        let full = source.crop(validate_crop(16, 10, 0, 0, 16, 10).unwrap());
        assert_eq!(full.pixels(), source.pixels());
        let corner = source.crop(validate_crop(16, 10, 15, 9, 1, 1).unwrap());
        assert_eq!(corner.pixel(0, 0), Some([15, 9, 15 ^ 9, 255]));
    }

    #[test]
    fn crop_validation_rejects_invalid_rects() {
        assert_eq!(validate_crop(10, 10, -1, 0, 1, 1), Err(CropError::Negative));
        assert_eq!(validate_crop(10, 10, 0, -5, 1, 1), Err(CropError::Negative));
        assert_eq!(validate_crop(10, 10, 0, 0, 0, 1), Err(CropError::Empty));
        assert_eq!(validate_crop(10, 10, 0, 0, 1, -1), Err(CropError::Empty));
        assert!(matches!(
            validate_crop(10, 10, 5, 0, 6, 1),
            Err(CropError::OutOfBounds { .. })
        ));
        assert!(matches!(
            validate_crop(10, 10, 10, 0, 1, 1),
            Err(CropError::OutOfBounds { .. })
        ));
        assert_eq!(
            validate_crop(10, 10, i64::MAX, 0, 1, 1),
            Err(CropError::Overflow)
        );
        assert_eq!(
            validate_crop(10, 10, 0, 1, 1, i64::MAX),
            Err(CropError::Overflow)
        );
        assert!(matches!(
            validate_crop(10, 10, 0, 0, i64::from(u32::MAX) + 1, 1),
            Err(CropError::OutOfBounds { .. })
        ));
    }

    #[test]
    fn png_round_trip_and_hash_is_deterministic() {
        let image = coordinate_fixture(7, 3);
        let a = encode_png(&image).unwrap();
        let b = encode_png(&image).unwrap();
        assert_eq!(a, b);
        assert_eq!(sha256_hex(&a), sha256_hex(&b));
        assert_eq!(sha256_hex(&a).len(), 64);
        let (w, h, px) = crate::testing::decode_png(&a);
        assert_eq!((w, h), (7, 3));
        assert_eq!(px, image.pixels());
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
