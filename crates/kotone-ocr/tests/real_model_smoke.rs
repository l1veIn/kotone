//! 真实本地模型冒烟（默认忽略）：需要显式隔离的模型目录与输入图像，不会自动下载。
//!
//! ```powershell
//! kotone-cli ocr install --model-dir <隔离目录>
//! $env:KOTONE_OCR_SMOKE_MODEL_DIR = "<隔离目录>"
//! $env:KOTONE_OCR_SMOKE_IMAGE = "<png>"
//! cargo test -p kotone-ocr --features onnx --test real_model_smoke -- --ignored --nocapture
//! ```

#![cfg(feature = "onnx")]

use std::time::{Duration, Instant};

use kotone_core::ocr::{OcrBackend, OcrCancel, OcrInput, OcrParams};
use kotone_ocr::LocalKoreanOcr;

fn decode(path: &str) -> (u32, u32, Vec<u8>, Vec<u8>) {
    let bytes = std::fs::read(path).expect("读取输入图像");
    let mut decoder = png::Decoder::new(bytes.as_slice());
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder.read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).unwrap();
    buf.truncate(info.buffer_size());
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => buf
            .chunks(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        other => panic!("不支持的 PNG 颜色类型 {other:?}"),
    };
    (info.width, info.height, rgba, bytes)
}

#[test]
#[ignore = "需要已安装的真实模型与显式输入（见文件头）"]
fn real_local_korean_ocr_smoke() {
    let (Ok(dir), Ok(image)) = (
        std::env::var("KOTONE_OCR_SMOKE_MODEL_DIR"),
        std::env::var("KOTONE_OCR_SMOKE_IMAGE"),
    ) else {
        panic!("设置 KOTONE_OCR_SMOKE_MODEL_DIR 与 KOTONE_OCR_SMOKE_IMAGE");
    };
    let backend = LocalKoreanOcr::new(dir.into());
    let info = backend.info();
    assert!(info.ready, "{:?}", info.reason);
    let (w, h, rgba, png) = decode(&image);
    let input = OcrInput {
        width: w,
        height: h,
        rgba: &rgba,
        png: &png,
    };
    let params = OcrParams {
        language: "ko".into(),
        options: serde_json::Value::Null,
    };
    for round in ["cold", "warm", "warm"] {
        let started = Instant::now();
        let out = backend
            .recognize(
                &input,
                &params,
                &OcrCancel::new(),
                Instant::now() + Duration::from_secs(60),
            )
            .unwrap();
        let total = started.elapsed().as_millis();
        println!(
            "== {round}: totalMs={total} coldStart={} phases={:?}",
            out.cold_start, out.phases
        );
        for o in &out.observations {
            let b = o.bbox.unwrap();
            println!(
                "  [{:>4.0},{:>4.0} {:>4.0}x{:>3.0}] conf={:.3} {}",
                b.x,
                b.y,
                b.width,
                b.height,
                o.confidence.unwrap_or(f32::NAN),
                o.text
            );
        }
        for warning in &out.warnings {
            println!("  warning {}: {}", warning.code, warning.message);
        }
        assert!(!out.observations.is_empty());
    }
}
