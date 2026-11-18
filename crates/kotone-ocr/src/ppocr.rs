//! PP-OCRv5 前后处理（与推理运行时无关的纯函数，可在没有模型的情况下测试）。
//!
//! 与 RapidOCR v3.10.0 / PaddleOCR 的默认参数对齐：
//! - 输入按 OpenCV 约定使用 **BGR** 通道顺序；
//! - 检测：短边不足 736 时放大到 736（limit_type=min），边长取 32 的倍数，
//!   归一化 (x/255-0.5)/0.5；DB 后处理 thresh=0.3、box_thresh=0.5、unclip_ratio=1.6、
//!   2×2 膨胀、fast score；
//! - 识别：高 48，批内按最大宽高比统一宽度（不小于 320），右侧补零，(x/255-0.5)/0.5；
//!   CTC 贪心解码（去重、去 blank），置信度为保留字符最大概率的均值。
//!
//! 简化（文档化的偏差）：检测框取连通域的**轴对齐**外接矩形，而不是旋转最小外接矩形，
//! 适用于水平排布的聊天文字；倾斜文字的框会偏大。

/// 检测参数。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DetParams {
    pub limit_side_len: u32,
    pub thresh: f32,
    pub box_thresh: f32,
    pub unclip_ratio: f32,
    pub max_candidates: usize,
}

impl Default for DetParams {
    fn default() -> Self {
        Self {
            limit_side_len: 736,
            thresh: 0.3,
            box_thresh: 0.5,
            unclip_ratio: 1.6,
            max_candidates: 1000,
        }
    }
}

pub const REC_HEIGHT: u32 = 48;
pub const REC_BASE_WIDTH: u32 = 320;
pub const REC_BATCH: usize = 6;
/// 单行识别输入宽度上限（防止极端长条耗尽内存）。
pub const REC_MAX_WIDTH: u32 = 3200;

/// 竖向填充参数（RapidOCR `use_vertical_padding`：min_height=30，width_height_ratio=8）。
const PAD_MIN_HEIGHT: u32 = 30;
const PAD_RATIO: f64 = 8.0;

/// 输入图像（RGBA8，紧密排列）。
#[derive(Clone, Copy)]
pub struct Rgba<'a> {
    pub width: u32,
    pub height: u32,
    pub pixels: &'a [u8],
}

impl Rgba<'_> {
    #[inline]
    fn bgr(&self, x: u32, y: u32) -> [f32; 3] {
        let i = ((y as usize) * (self.width as usize) + x as usize) * 4;
        let p = &self.pixels[i..i + 3];
        [f32::from(p[2]), f32::from(p[1]), f32::from(p[0])]
    }
}

/// 竖向黑边填充（上下各 `top` 行）；返回新缓冲与 `top`。不需要填充时返回 None。
pub fn vertical_padding(image: Rgba<'_>) -> Option<(Vec<u8>, u32, u32)> {
    let (w, h) = (image.width, image.height);
    let too_flat = f64::from(w) / f64::from(h) > PAD_RATIO;
    if h > PAD_MIN_HEIGHT && !too_flat {
        return None;
    }
    let new_h = ((f64::from(w) / PAD_RATIO) as u32).max(PAD_MIN_HEIGHT) * 2;
    let top = (new_h as i64 - h as i64).unsigned_abs() as u32 / 2;
    if top == 0 {
        return None;
    }
    let out_h = h + 2 * top;
    let row = w as usize * 4;
    let mut out = vec![0u8; row * out_h as usize];
    for y in 0..h as usize {
        let dst = (y + top as usize) * row;
        out[dst..dst + row].copy_from_slice(&image.pixels[y * row..(y + 1) * row]);
    }
    // 黑边 alpha 置 255（与 OpenCV 三通道常量填充等价；alpha 不参与推理）
    for y in (0..top as usize).chain((top + h) as usize..out_h as usize) {
        for x in 0..w as usize {
            out[y * row + x * 4 + 3] = 255;
        }
    }
    Some((out, top, out_h))
}

/// OpenCV INTER_LINEAR 语义的双线性采样：把源图 `(sx, sy, sw, sh)` 区域缩放到 `dw × dh`，
/// 回调接收 (dx, dy, bgr)。
pub fn resize_region_bilinear(
    image: Rgba<'_>,
    region: (u32, u32, u32, u32),
    dw: u32,
    dh: u32,
    mut put: impl FnMut(u32, u32, [f32; 3]),
) {
    let (rx, ry, rw, rh) = region;
    let fx = f64::from(rw) / f64::from(dw);
    let fy = f64::from(rh) / f64::from(dh);
    let max_x = rw - 1;
    let max_y = rh - 1;
    // 预计算列索引与权重
    let cols: Vec<(u32, u32, f32)> = (0..dw)
        .map(|dx| {
            let s = ((f64::from(dx) + 0.5) * fx - 0.5).max(0.0);
            let x0 = (s.floor() as u32).min(max_x);
            let x1 = (x0 + 1).min(max_x);
            let wx = (s - f64::from(x0)).clamp(0.0, 1.0) as f32;
            (x0, x1, wx)
        })
        .collect();
    for dy in 0..dh {
        let s = ((f64::from(dy) + 0.5) * fy - 0.5).max(0.0);
        let y0 = (s.floor() as u32).min(max_y);
        let y1 = (y0 + 1).min(max_y);
        let wy = (s - f64::from(y0)).clamp(0.0, 1.0) as f32;
        for (dx, &(x0, x1, wx)) in cols.iter().enumerate() {
            let a = image.bgr(rx + x0, ry + y0);
            let b = image.bgr(rx + x1, ry + y0);
            let c = image.bgr(rx + x0, ry + y1);
            let d = image.bgr(rx + x1, ry + y1);
            let mut px = [0f32; 3];
            for k in 0..3 {
                let top = a[k] + (b[k] - a[k]) * wx;
                let bottom = c[k] + (d[k] - c[k]) * wx;
                // OpenCV 对 8 位图像输出四舍五入回 u8
                px[k] = (top + (bottom - top) * wy).round().clamp(0.0, 255.0);
            }
            put(dx as u32, dy, px);
        }
    }
}

/// 检测输入尺寸（宽, 高）。
pub fn det_size(width: u32, height: u32, limit_side_len: u32) -> (u32, u32) {
    let (w, h) = (f64::from(width), f64::from(height));
    let min_side = w.min(h);
    let ratio = if min_side < f64::from(limit_side_len) {
        f64::from(limit_side_len) / min_side
    } else {
        1.0
    };
    let rw = (w * ratio) as u32;
    let rh = (h * ratio) as u32;
    let round32 = |v: u32| (((f64::from(v) / 32.0).round() as u32) * 32).max(32);
    (round32(rw), round32(rh))
}

/// 检测预处理：返回 NCHW f32 张量（BGR）。
pub fn det_tensor(image: Rgba<'_>, dw: u32, dh: u32) -> Vec<f32> {
    let plane = (dw * dh) as usize;
    let mut out = vec![0f32; plane * 3];
    resize_region_bilinear(
        image,
        (0, 0, image.width, image.height),
        dw,
        dh,
        |x, y, px| {
            let i = (y * dw + x) as usize;
            for k in 0..3 {
                out[k * plane + i] = (px[k] / 255.0 - 0.5) / 0.5;
            }
        },
    );
    out
}

/// 检测框（填充后图像坐标，整数像素，含端点语义同 OpenCV 轮廓点）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DetBox {
    pub x0: u32,
    pub y0: u32,
    pub x1: u32,
    pub y1: u32,
    pub score: f32,
}

/// DB 后处理。`pred` 为 [ph, pw] 概率图；输出映射到 `dest_w × dest_h` 的图像坐标。
pub fn db_postprocess(
    pred: &[f32],
    pw: u32,
    ph: u32,
    dest_w: u32,
    dest_h: u32,
    params: &DetParams,
) -> Vec<DetBox> {
    let (w, h) = (pw as usize, ph as usize);
    debug_assert_eq!(pred.len(), w * h);
    // 二值化 + 2×2 膨胀（OpenCV 锚点 (1,1)：dst(x,y)=max(src[x-1..=x, y-1..=y])）
    let seg: Vec<bool> = pred.iter().map(|&p| p > params.thresh).collect();
    let mut mask = vec![false; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut on = seg[y * w + x];
            if !on && x > 0 {
                on = seg[y * w + x - 1];
            }
            if !on && y > 0 {
                on = seg[(y - 1) * w + x] || (x > 0 && seg[(y - 1) * w + x - 1]);
            }
            mask[y * w + x] = on;
        }
    }
    // 8 连通域（外轮廓的外接矩形）
    let mut label = vec![false; w * h];
    let mut boxes = Vec::new();
    let mut stack = Vec::new();
    let mut candidates = 0usize;
    for start in 0..w * h {
        if !mask[start] || label[start] {
            continue;
        }
        candidates += 1;
        if candidates > params.max_candidates {
            break;
        }
        let (mut minx, mut miny, mut maxx, mut maxy) = (usize::MAX, usize::MAX, 0usize, 0usize);
        label[start] = true;
        stack.push(start);
        while let Some(i) = stack.pop() {
            let (x, y) = (i % w, i / w);
            minx = minx.min(x);
            maxx = maxx.max(x);
            miny = miny.min(y);
            maxy = maxy.max(y);
            for dy in [-1i64, 0, 1] {
                for dx in [-1i64, 0, 1] {
                    let (nx, ny) = (x as i64 + dx, y as i64 + dy);
                    if nx < 0 || ny < 0 || nx >= w as i64 || ny >= h as i64 {
                        continue;
                    }
                    let j = ny as usize * w + nx as usize;
                    if mask[j] && !label[j] {
                        label[j] = true;
                        stack.push(j);
                    }
                }
            }
        }
        // 轮廓点外接矩形：宽高为点坐标之差（与 cv2.minAreaRect 对轴对齐区域一致）
        let rw = (maxx - minx) as f64;
        let rh = (maxy - miny) as f64;
        if rw.min(rh) < 3.0 {
            continue;
        }
        // fast score：外接矩形内概率均值
        let mut sum = 0f64;
        for y in miny..=maxy {
            for x in minx..=maxx {
                sum += f64::from(pred[y * w + x]);
            }
        }
        let score = (sum / ((maxx - minx + 1) * (maxy - miny + 1)) as f64) as f32;
        if score < params.box_thresh {
            continue;
        }
        // unclip：矩形外扩距离 d = area * ratio / perimeter
        let d = rw * rh * f64::from(params.unclip_ratio) / (2.0 * (rw + rh));
        let (ex0, ey0, ex1, ey1) = (
            minx as f64 - d,
            miny as f64 - d,
            maxx as f64 + d,
            maxy as f64 + d,
        );
        if (ex1 - ex0).min(ey1 - ey0) < 5.0 {
            continue;
        }
        let sx = |v: f64| {
            (v / w as f64 * f64::from(dest_w))
                .round()
                .clamp(0.0, f64::from(dest_w))
        };
        let sy = |v: f64| {
            (v / h as f64 * f64::from(dest_h))
                .round()
                .clamp(0.0, f64::from(dest_h))
        };
        // filter_det_res：裁到 [0, w-1] × [0, h-1]，宽高需 > 3
        let cx = |v: f64| v.min(f64::from(dest_w.saturating_sub(1))).max(0.0) as u32;
        let cy = |v: f64| v.min(f64::from(dest_h.saturating_sub(1))).max(0.0) as u32;
        let b = DetBox {
            x0: cx(sx(ex0)),
            y0: cy(sy(ey0)),
            x1: cx(sx(ex1)),
            y1: cy(sy(ey1)),
            score,
        };
        if b.x1 - b.x0 <= 3 || b.y1 - b.y0 <= 3 {
            continue;
        }
        boxes.push(b);
    }
    sort_boxes(&mut boxes);
    boxes
}

/// 阅读顺序：按左上 y 稳定排序，相邻 y 差 < 10 视为同一行，行内按 x。
pub fn sort_boxes(boxes: &mut [DetBox]) {
    boxes.sort_by_key(|b| b.y0);
    let mut line = 0u32;
    let mut keyed: Vec<(u32, u32, DetBox)> = Vec::with_capacity(boxes.len());
    for (i, b) in boxes.iter().enumerate() {
        if i > 0 && b.y0.saturating_sub(boxes[i - 1].y0) >= 10 {
            line += 1;
        }
        keyed.push((line, b.x0, *b));
    }
    keyed.sort_by_key(|(l, x, _)| (*l, *x));
    for (slot, (_, _, b)) in boxes.iter_mut().zip(keyed) {
        *slot = b;
    }
}

/// 从右往左书写的文字：行的划分与 [`sort_boxes`] 相同，行内按右边缘从右到左排列（阅读顺序）。
pub fn sort_boxes_rtl(boxes: &mut [DetBox]) {
    boxes.sort_by_key(|b| b.y0);
    let mut line = 0u32;
    let mut keyed: Vec<(u32, std::cmp::Reverse<u32>, DetBox)> = Vec::with_capacity(boxes.len());
    for (i, b) in boxes.iter().enumerate() {
        if i > 0 && b.y0.saturating_sub(boxes[i - 1].y0) >= 10 {
            line += 1;
        }
        keyed.push((line, std::cmp::Reverse(b.x1), *b));
    }
    keyed.sort_by_key(|(l, x, _)| (*l, *x));
    for (slot, (_, _, b)) in boxes.iter_mut().zip(keyed) {
        *slot = b;
    }
}

/// 识别裁剪区域（填充后图像坐标）：宽高取框边长（与透视裁剪对轴对齐框等价）。
pub fn crop_region(b: &DetBox) -> (u32, u32, u32, u32) {
    (b.x0, b.y0, (b.x1 - b.x0).max(1), (b.y1 - b.y0).max(1))
}

/// 一批识别输入：返回 (张量 NCHW, 批宽度)。`regions` 为填充后图像中的裁剪区域。
pub fn rec_tensor(image: Rgba<'_>, regions: &[(u32, u32, u32, u32)]) -> (Vec<f32>, u32) {
    let mut max_ratio = f64::from(REC_BASE_WIDTH) / f64::from(REC_HEIGHT);
    for &(_, _, w, h) in regions {
        max_ratio = max_ratio.max(f64::from(w) / f64::from(h));
    }
    let width = ((f64::from(REC_HEIGHT) * max_ratio) as u32).clamp(1, REC_MAX_WIDTH);
    let plane = (REC_HEIGHT * width) as usize;
    let mut out = vec![0f32; plane * 3 * regions.len()];
    for (n, &(x, y, w, h)) in regions.iter().enumerate() {
        let ratio = f64::from(w) / f64::from(h);
        let resized = ((f64::from(REC_HEIGHT) * ratio).ceil() as u32).clamp(1, width);
        let base = n * plane * 3;
        resize_region_bilinear(image, (x, y, w, h), resized, REC_HEIGHT, |dx, dy, px| {
            let i = (dy * width + dx) as usize;
            for k in 0..3 {
                out[base + k * plane + i] = (px[k] / 255.0 - 0.5) / 0.5;
            }
        });
    }
    (out, width)
}

/// 从右往左书写的文字（阿拉伯字母）：识别模型按图像从左到右输出字符，需要还原为
/// 阅读顺序。与 PaddleOCR `BaseRecLabelDecode.pred_reverse` 相同：拉丁字母、数字与常见
/// 符号的连续片段保持原样，其余字符逐个作为片段，然后整体倒序拼接。
pub fn reverse_rtl(text: &str) -> String {
    let keeps = |c: char| c.is_ascii_alphanumeric() || " :*./%+-".contains(c);
    let mut parts: Vec<String> = Vec::new();
    let mut run = String::new();
    for c in text.chars() {
        if keeps(c) {
            run.push(c);
        } else {
            if !run.is_empty() {
                parts.push(std::mem::take(&mut run));
            }
            parts.push(c.to_string());
        }
    }
    if !run.is_empty() {
        parts.push(run);
    }
    parts.reverse();
    parts.concat()
}

/// CTC 贪心解码一条序列。`probs` 为 [T, C]，`dict` 为模型字符表（不含 blank/空格）。
/// 类别 0 为 blank，1..=dict.len() 为字符，dict.len()+1 为空格。
/// 返回 (文本, 保留字符的平均最大概率；无字符时 None)。
pub fn ctc_decode(probs: &[f32], classes: usize, dict: &[String]) -> (String, Option<f32>) {
    let steps = probs.len() / classes.max(1);
    let mut text = String::new();
    let mut confs: Vec<f32> = Vec::new();
    let mut prev: Option<usize> = None;
    for t in 0..steps {
        let row = &probs[t * classes..(t + 1) * classes];
        let (idx, p) =
            row.iter().enumerate().fold(
                (0usize, f32::MIN),
                |acc, (i, &v)| if v > acc.1 { (i, v) } else { acc },
            );
        let duplicate = prev == Some(idx);
        prev = Some(idx);
        if duplicate || idx == 0 {
            continue;
        }
        let ch = if idx <= dict.len() {
            dict[idx - 1].as_str()
        } else if idx == dict.len() + 1 {
            " "
        } else {
            continue;
        };
        text.push_str(ch);
        confs.push(p);
    }
    let conf = if confs.is_empty() {
        None
    } else {
        Some(confs.iter().sum::<f32>() / confs.len() as f32)
    };
    (text, conf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn right_to_left_text_is_restored_to_reading_order() {
        // 模型按图像从左到右输出“التنين”的字符：نينتلا
        assert_eq!(reverse_rtl("نينتلا"), "التنين");
        // 数字与拉丁字母片段保持内部顺序
        assert_eq!(reverse_rtl("ab12 ن"), "نab12 ");
        assert_eq!(reverse_rtl(""), "");
    }

    fn rgba(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
        let mut px = Vec::new();
        for y in 0..h {
            for x in 0..w {
                px.extend_from_slice(&f(x, y));
            }
        }
        px
    }

    #[test]
    fn det_size_matches_rapidocr_limit_min_rule() {
        // 650×200 聊天裁剪：短边 200 → 736，比例 3.68
        assert_eq!(det_size(650, 200, 736), (2400, 736));
        // 已足够大的图不缩放，只取 32 倍数
        assert_eq!(det_size(1950, 800, 736), (1952, 800));
        assert_eq!(det_size(10, 10, 736), (736, 736));
    }

    #[test]
    fn bilinear_identity_and_channel_order_is_bgr() {
        let px = rgba(4, 3, |x, y| [x as u8 * 10, y as u8 * 20, 7, 255]);
        let img = Rgba {
            width: 4,
            height: 3,
            pixels: &px,
        };
        let mut seen = Vec::new();
        resize_region_bilinear(img, (1, 1, 2, 2), 2, 2, |x, y, p| seen.push((x, y, p)));
        assert_eq!(seen[0], (0, 0, [7.0, 20.0, 10.0]));
        assert_eq!(seen[3], (1, 1, [7.0, 40.0, 20.0]));
        // 2 倍放大：中点插值
        let mut up = Vec::new();
        resize_region_bilinear(img, (0, 0, 2, 1), 4, 1, |_, _, p| up.push(p[2]));
        assert_eq!(up, vec![0.0, 3.0, 8.0, 10.0]); // OpenCV: 0, 2.5→3(round), 7.5→8, 10
    }

    #[test]
    fn vertical_padding_rule() {
        let px = rgba(400, 20, |_, _| [1, 2, 3, 255]);
        let img = Rgba {
            width: 400,
            height: 20,
            pixels: &px,
        };
        let (out, top, h) = vertical_padding(img).unwrap();
        // new_h = max(400/8, 30)*2 = 100；top = (100-20)/2 = 40
        assert_eq!((top, h), (40, 100));
        assert_eq!(&out[0..4], &[0, 0, 0, 255]);
        let row = 400 * 4;
        assert_eq!(&out[40 * row..40 * row + 4], &[1, 2, 3, 255]);
        let tall = rgba(100, 60, |_, _| [0, 0, 0, 255]);
        assert!(vertical_padding(Rgba {
            width: 100,
            height: 60,
            pixels: &tall
        })
        .is_none());
    }

    #[test]
    fn db_postprocess_finds_lines_in_reading_order() {
        // 概率图 100×40：两条横向“文字行”（y 5..12 与 y 25..32）
        let (w, h) = (100u32, 40u32);
        let mut pred = vec![0f32; (w * h) as usize];
        for (y0, y1, x0, x1) in [(25, 32, 10, 60), (5, 12, 20, 90)] {
            for y in y0..=y1 {
                for x in x0..=x1 {
                    pred[(y * w + x) as usize] = 0.9;
                }
            }
        }
        // 噪点：低于 box_thresh 的小块与过小的块
        pred[(38 * w + 2) as usize] = 0.4;
        let boxes = db_postprocess(&pred, w, h, 200, 80, &DetParams::default());
        assert_eq!(boxes.len(), 2, "{boxes:?}");
        assert!(boxes[0].y0 < boxes[1].y0, "自上而下");
        // 映射到 2 倍目标尺寸并外扩
        assert!(boxes[0].x0 < 40 && boxes[0].x1 > 180);
        assert!(boxes[0].score > 0.5);
    }

    #[test]
    fn sort_groups_same_row_by_x() {
        let mk = |x0, y0| DetBox {
            x0,
            y0,
            x1: x0 + 10,
            y1: y0 + 10,
            score: 1.0,
        };
        let mut boxes = vec![mk(50, 104), mk(5, 100), mk(0, 50), mk(30, 103)];
        sort_boxes(&mut boxes);
        let order: Vec<u32> = boxes.iter().map(|b| b.x0).collect();
        assert_eq!(order, vec![0, 5, 30, 50]);
    }

    #[test]
    fn ctc_decode_dedups_blanks_and_space() {
        let dict: Vec<String> = ["가", "나"].iter().map(|s| s.to_string()).collect();
        let c = dict.len() + 2; // blank + 2 chars + space
        let mut probs = Vec::new();
        for idx in [1usize, 1, 0, 1, 3, 2, 2, 0] {
            let mut row = vec![0.01f32; c];
            row[idx] = 0.9;
            probs.extend(row);
        }
        let (text, conf) = ctc_decode(&probs, c, &dict);
        assert_eq!(text, "가가 나");
        assert!((conf.unwrap() - 0.9).abs() < 1e-6);
        let blank: Vec<f32> = (0..c * 3)
            .map(|i| if i % c == 0 { 0.9 } else { 0.0 })
            .collect();
        assert_eq!(ctc_decode(&blank, c, &dict), (String::new(), None));
    }

    #[test]
    fn rec_tensor_pads_to_batch_width() {
        let px = rgba(200, 48, |_, _| [255, 255, 255, 255]);
        let img = Rgba {
            width: 200,
            height: 48,
            pixels: &px,
        };
        let (t, width) = rec_tensor(img, &[(0, 0, 200, 48), (0, 0, 48, 48)]);
        assert_eq!(width, 320);
        assert_eq!(t.len(), 2 * 3 * 48 * 320);
        let plane = 48 * 320;
        // 第二张：48 宽内为白色 (1.0)，之后补零
        let base = 3 * plane;
        assert_eq!(t[base], 1.0);
        assert_eq!(t[base + 47], 1.0);
        assert_eq!(t[base + 48], 0.0);
    }
}
