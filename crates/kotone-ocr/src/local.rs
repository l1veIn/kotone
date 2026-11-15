//! 本地韩文 OCR 后端：PP-OCRv5 mobile（检测 + 韩文识别），ONNX Runtime CPU 推理。
//!
//! - 模型按 [`crate::manifest::KOREAN_MOBILE`] 安装在用户模型目录；就绪检查逐文件核对
//!   大小与 SHA-256，加载时对内存中的同一份字节再次校验后才创建会话。
//! - 首次识别时冷加载（`coldStart=true`，计入 `modelLoadMs`），之后复用会话。
//! - 推理可被 [`OcrCancel`] 中断（RunOptionsSetTerminate）；期限在各阶段之间核对。
//! - 不会降级到在线后端。

// 未启用 `onnx` 时推理路径不编译，只保留就绪检查与拒绝逻辑。
#![cfg_attr(not(feature = "onnx"), allow(dead_code, unused_imports))]

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde_json::{json, Value};

use kotone_core::ocr::{
    OcrBackend, OcrBackendInfo, OcrBackendKind, OcrCancel, OcrError, OcrGranularity, OcrInput,
    OcrObservation, OcrOutput, OcrParams, OcrRect, OcrWarning,
};

use crate::install::{PackageState, Verifier};
use crate::manifest::{ModelPackage, KOREAN_MOBILE};
use crate::ppocr::{self, DetParams, Rgba};

pub const BACKEND_ID: &str = "local-ppocrv5-korean";
const INTRA_THREADS: i32 = 4;
const INTER_THREADS: i32 = 1;
/// 检测输入像素上限（超过则等比缩小，仍取 32 的倍数）。
const MAX_DET_PIXELS: u64 = 4_194_304;
/// 检测前的长边上限（RapidOCR max_side_len）。
const MAX_SIDE_LEN: u32 = 2000;
const MAX_LINES: usize = 256;

pub const CONFIDENCE_SEMANTICS: &str = "recognizer self-reported score: mean of the per-character max softmax probability over decoded characters; NOT semantic or translation accuracy; null when nothing was decoded";

#[cfg(feature = "onnx")]
struct Loaded {
    det: crate::ort::Session,
    rec: crate::ort::Session,
    dict: Vec<String>,
    classes: usize,
}

pub struct LocalKoreanOcr {
    dir: PathBuf,
    package: ModelPackage,
    verifier: Verifier,
    #[cfg(feature = "onnx")]
    loaded: Mutex<Option<Arc<Loaded>>>,
    #[cfg(not(feature = "onnx"))]
    #[allow(dead_code)]
    loaded: Mutex<Option<Arc<()>>>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct LocalOptions {
    min_confidence: Option<f32>,
    det_limit_side_len: u32,
}

fn parse_options(options: &Value) -> Result<LocalOptions, OcrError> {
    let mut out = LocalOptions {
        min_confidence: None,
        det_limit_side_len: DetParams::default().limit_side_len,
    };
    let map = match options {
        Value::Null => return Ok(out),
        Value::Object(map) => map,
        _ => {
            return Err(OcrError::UnsupportedOption {
                option: "options".into(),
                message: "options 必须是对象".into(),
            })
        }
    };
    for (key, value) in map {
        match key.as_str() {
            "minConfidence" => match value.as_f64() {
                Some(v) if (0.0..=1.0).contains(&v) => out.min_confidence = Some(v as f32),
                _ => {
                    return Err(OcrError::UnsupportedOption {
                        option: key.clone(),
                        message: "minConfidence 必须是 0..=1 的数值".into(),
                    })
                }
            },
            "detLimitSideLen" => match value.as_u64() {
                Some(v) if (32..=1536).contains(&v) => out.det_limit_side_len = v as u32,
                _ => {
                    return Err(OcrError::UnsupportedOption {
                        option: key.clone(),
                        message: "detLimitSideLen 必须是 32..=1536 的整数".into(),
                    })
                }
            },
            other => {
                return Err(OcrError::UnsupportedOption {
                    option: other.into(),
                    message: format!(
                        "{BACKEND_ID} 不支持该选项（支持 minConfidence、detLimitSideLen）"
                    ),
                })
            }
        }
    }
    Ok(out)
}

pub fn options_schema() -> Value {
    json!({
        "type": ["object", "null"],
        "additionalProperties": false,
        "properties": {
            "minConfidence": {"type": "number", "minimum": 0, "maximum": 1, "description": "丢弃识别器自报分数低于该值的行（缺省不过滤；被丢弃的行数在 warnings 中报告）"},
            "detLimitSideLen": {"type": "integer", "minimum": 32, "maximum": 1536, "default": 736, "description": "检测输入短边目标长度（RapidOCR limit_type=min）"}
        }
    })
}

/// 检测输入尺寸：长边先限制到 2000，再按短边规则放大，最后限制总像素。
pub(crate) fn det_dims(width: u32, height: u32, limit: u32) -> (u32, u32) {
    let max_side = width.max(height);
    let (mut w, mut h) = (width, height);
    if max_side > MAX_SIDE_LEN {
        let r = f64::from(MAX_SIDE_LEN) / f64::from(max_side);
        w = ((f64::from(width) * r) as u32).max(1);
        h = ((f64::from(height) * r) as u32).max(1);
    }
    let (mut dw, mut dh) = ppocr::det_size(w, h, limit);
    let pixels = u64::from(dw) * u64::from(dh);
    if pixels > MAX_DET_PIXELS {
        let r = (MAX_DET_PIXELS as f64 / pixels as f64).sqrt();
        let round32 = |v: f64| (((v / 32.0).floor() as u32) * 32).max(32);
        dw = round32(f64::from(dw) * r);
        dh = round32(f64::from(dh) * r);
    }
    (dw, dh)
}

impl LocalKoreanOcr {
    /// `dir` 为模型包目录（`<模型根>/ocr/ppocrv5-korean-mobile`）。
    pub fn new(dir: PathBuf) -> Self {
        Self::with_package(dir, KOREAN_MOBILE)
    }

    pub fn with_package(dir: PathBuf, package: ModelPackage) -> Self {
        Self {
            dir,
            package,
            verifier: Verifier::new(),
            loaded: Mutex::new(None),
        }
    }

    pub fn package(&self) -> &ModelPackage {
        &self.package
    }

    pub fn dir(&self) -> &std::path::Path {
        &self.dir
    }

    fn runtime() -> Result<String, String> {
        #[cfg(feature = "onnx")]
        {
            crate::ort::runtime_version()
        }
        #[cfg(not(feature = "onnx"))]
        {
            Err("本构建未启用本地 ONNX 推理（kotone-ocr feature `onnx`）".into())
        }
    }

    /// 是否已加载（正在加载时不等待，按未加载报告）。
    fn is_loaded(&self) -> bool {
        self.loaded.try_lock().map(|g| g.is_some()).unwrap_or(false)
    }

    #[cfg(feature = "onnx")]
    fn load(
        &self,
        cancel: &OcrCancel,
        deadline: Instant,
    ) -> Result<(Arc<Loaded>, bool, u64), OcrError> {
        let mut slot = self.loaded.lock().unwrap();
        if let Some(loaded) = slot.as_ref() {
            return Ok((loaded.clone(), false, 0));
        }
        let started = Instant::now();
        let invalid = |message: String| OcrError::NotReady {
            reason: "invalid".into(),
            message,
        };
        let not_installed = |message: String| OcrError::NotReady {
            reason: "not-installed".into(),
            message: format!("{message}；请运行 `kotone-cli ocr install` 安装本地韩文 OCR 模型"),
        };
        let mut bytes = Vec::new();
        for role in ["det", "rec"] {
            let file = self.package.file(role).expect("manifest role");
            match self.verifier.read_verified(&self.dir, file) {
                Ok(b) => bytes.push(b),
                Err(e) if e.starts_with("缺少") => return Err(not_installed(e)),
                Err(e) => return Err(invalid(e)),
            }
            cancel.check(deadline)?;
        }
        let dict = crate::onnx_meta::character_list(&bytes[1]).map_err(invalid)?;
        let det = crate::ort::Session::from_bytes(&bytes[0], INTRA_THREADS, INTER_THREADS)
            .map_err(|e| invalid(format!("加载检测模型失败: {e}")))?;
        cancel.check(deadline)?;
        let rec = crate::ort::Session::from_bytes(&bytes[1], INTRA_THREADS, INTER_THREADS)
            .map_err(|e| invalid(format!("加载识别模型失败: {e}")))?;
        // 自检：识别输出类别数必须等于 blank + 字符表 + 空格
        let mut probe = vec![0f32; 3 * 48 * 320];
        let out = rec
            .run(&mut probe, &[1, 3, 48, 320], |_| Box::new(()))
            .map_err(|e| invalid(format!("识别模型自检失败: {e}")))?;
        let classes = *out.shape.last().unwrap_or(&0) as usize;
        if out.shape.len() != 3 || classes != dict.len() + 2 {
            return Err(invalid(format!(
                "识别模型输出形状 {:?} 与内嵌字符表（{} 个字符）不符",
                out.shape,
                dict.len()
            )));
        }
        let loaded = Arc::new(Loaded {
            det,
            rec,
            dict,
            classes,
        });
        *slot = Some(loaded.clone());
        Ok((loaded, true, started.elapsed().as_millis() as u64))
    }
}

fn terminated(cancel: &OcrCancel, deadline: Instant, message: String) -> OcrError {
    if cancel.is_cancelled() {
        OcrError::Cancelled
    } else if Instant::now() >= deadline {
        OcrError::DeadlineExceeded
    } else {
        OcrError::Engine(message)
    }
}

impl OcrBackend for LocalKoreanOcr {
    fn id(&self) -> &str {
        BACKEND_ID
    }

    fn kind(&self) -> OcrBackendKind {
        OcrBackendKind::Local
    }

    fn info(&self) -> OcrBackendInfo {
        let runtime = Self::runtime();
        let status = self.verifier.check(&self.dir, &self.package);
        let (ready, state, reason) = match (&runtime, &status.state) {
            (Err(e), _) => (false, "runtime-unavailable".to_string(), Some(e.clone())),
            (Ok(_), PackageState::Ready) => (true, "ready".to_string(), None),
            (Ok(_), PackageState::NotInstalled) => (
                false,
                "not-installed".to_string(),
                Some("本地韩文 OCR 模型未安装；运行 `kotone-cli ocr install`".to_string()),
            ),
            (Ok(_), PackageState::Invalid(m)) => (
                false,
                "invalid".to_string(),
                Some(format!(
                    "模型文件校验失败：{m}；运行 `kotone-cli ocr install` 重新安装"
                )),
            ),
        };
        let files: Vec<Value> = self
            .package
            .files
            .iter()
            .zip(status.files.iter())
            .map(|(f, s)| {
                json!({
                    "name": f.name, "role": f.role, "url": f.url, "sha256": f.sha256,
                    "bytes": f.bytes, "present": s.present, "sizeOk": s.size_ok, "sha256Ok": s.sha256_ok
                })
            })
            .collect();
        OcrBackendInfo {
            id: BACKEND_ID.into(),
            kind: OcrBackendKind::Local,
            display_name: self.package.display_name.into(),
            ready,
            state,
            reason,
            languages: self
                .package
                .languages
                .iter()
                .map(|s| s.to_string())
                .collect(),
            default_language: "ko".into(),
            data_egress: "none".into(),
            options_schema: options_schema(),
            details: json!({
                "engine": "PP-OCRv5 DB detector + CTC recognizer, ONNX Runtime C API",
                "device": "cpu",
                "threads": {"intraOp": INTRA_THREADS, "interOp": INTER_THREADS},
                "runtime": {
                    "name": "onnxruntime",
                    "version": runtime.as_ref().ok(),
                    "linkage": "statically linked from the sherpa-onnx v1.13.5 prebuilt package (shared with ASR); no separate DLL",
                    "error": runtime.as_ref().err(),
                },
                "model": {
                    "id": self.package.id,
                    "version": self.package.version,
                    "license": self.package.license,
                    "licenseUrl": self.package.license_url,
                    "upstream": self.package.upstream,
                    "distribution": self.package.distribution,
                    "location": format!("<models>/ocr/{}", self.package.id),
                    "installed": status.state == PackageState::Ready,
                    "files": files,
                },
                "loaded": self.is_loaded(),
                "granularity": ["line"],
                "geometry": "axis-aligned line boxes from the DB detector, in input-artifact physical pixels",
                "confidence": CONFIDENCE_SEMANTICS,
                "install": "kotone-cli ocr install",
                "fallback": "none: this backend never falls back to an online backend",
                "knownLimits": [
                    "detector boxes are axis-aligned (horizontal text); rotated text gets loose boxes",
                    "text direction classifier is not used",
                    "Korean recognizer; Latin letters and digits are recognised only as far as the Korean model's character set covers them"
                ]
            }),
        }
    }

    fn recognize(
        &self,
        input: &OcrInput<'_>,
        params: &OcrParams,
        cancel: &OcrCancel,
        deadline: Instant,
    ) -> Result<OcrOutput, OcrError> {
        if !self.package.languages.contains(&params.language.as_str()) {
            return Err(OcrError::UnsupportedLanguage {
                language: params.language.clone(),
                supported: self
                    .package
                    .languages
                    .iter()
                    .map(|s| s.to_string())
                    .collect(),
            });
        }
        let options = parse_options(&params.options)?;
        if input.width == 0 || input.height == 0 {
            return Err(OcrError::InvalidInput("空图像".into()));
        }
        if let Err(e) = Self::runtime() {
            return Err(OcrError::NotReady {
                reason: "runtime-unavailable".into(),
                message: e,
            });
        }
        cancel.check(deadline)?;
        #[cfg(feature = "onnx")]
        {
            self.recognize_onnx(input, options, cancel, deadline)
        }
        #[cfg(not(feature = "onnx"))]
        {
            let _ = options;
            Err(OcrError::NotReady {
                reason: "runtime-unavailable".into(),
                message: "本构建未启用本地 ONNX 推理".into(),
            })
        }
    }
}

#[cfg(feature = "onnx")]
impl LocalKoreanOcr {
    fn recognize_onnx(
        &self,
        input: &OcrInput<'_>,
        options: LocalOptions,
        cancel: &OcrCancel,
        deadline: Instant,
    ) -> Result<OcrOutput, OcrError> {
        let mut phases = Vec::new();
        let (model, cold, load_ms) = self.load(cancel, deadline)?;
        if cold {
            phases.push(("modelLoadMs".to_string(), load_ms));
        }
        cancel.check(deadline)?;
        let original = Rgba {
            width: input.width,
            height: input.height,
            pixels: input.rgba,
        };
        let padded = ppocr::vertical_padding(original);
        let (image, top) = match &padded {
            Some((buf, top, h)) => (
                Rgba {
                    width: input.width,
                    height: *h,
                    pixels: buf,
                },
                *top,
            ),
            None => (original, 0),
        };
        // 检测
        let det_started = Instant::now();
        let det_params = DetParams {
            limit_side_len: options.det_limit_side_len,
            ..DetParams::default()
        };
        let (dw, dh) = det_dims(image.width, image.height, det_params.limit_side_len);
        let mut tensor = ppocr::det_tensor(image, dw, dh);
        let out = model
            .det
            .run(&mut tensor, &[1, 3, i64::from(dh), i64::from(dw)], |t| {
                Box::new(cancel.on_cancel(move || t.terminate()))
            })
            .map_err(|e| terminated(cancel, deadline, format!("检测推理失败: {e}")))?;
        drop(tensor);
        if out.shape.len() != 4 || out.shape[2] as u32 != dh || out.shape[3] as u32 != dw {
            return Err(OcrError::Engine(format!(
                "检测输出形状异常: {:?}",
                out.shape
            )));
        }
        let mut boxes =
            ppocr::db_postprocess(&out.data, dw, dh, image.width, image.height, &det_params);
        phases.push((
            "detMs".to_string(),
            det_started.elapsed().as_millis() as u64,
        ));
        let mut warnings = Vec::new();
        if boxes.len() > MAX_LINES {
            warnings.push(OcrWarning::new(
                "OCR_LINES_TRUNCATED",
                format!("检测到 {} 行，只识别前 {MAX_LINES} 行", boxes.len()),
            ));
            boxes.truncate(MAX_LINES);
        }
        cancel.check(deadline)?;
        // 识别：按宽高比排序分批（与 RapidOCR 一致），结果按原顺序还原
        let rec_started = Instant::now();
        let mut regions: Vec<(u32, u32, u32, u32)> = boxes.iter().map(ppocr::crop_region).collect();
        // 竖排（高/宽 ≥ 1.5）的框 RapidOCR 会旋转 90°；聊天为横排，这里不旋转而是标注
        let vertical = regions
            .iter()
            .filter(|r| r.3 as f64 / r.2 as f64 >= 1.5)
            .count();
        if vertical > 0 {
            warnings.push(OcrWarning::new(
                "OCR_VERTICAL_BOXES",
                format!("{vertical} 个检测框为竖向，按横排识别，结果可能无意义"),
            ));
        }
        let mut order: Vec<usize> = (0..regions.len()).collect();
        order.sort_by(|&a, &b| {
            let ra = f64::from(regions[a].2) / f64::from(regions[a].3);
            let rb = f64::from(regions[b].2) / f64::from(regions[b].3);
            ra.partial_cmp(&rb).unwrap_or(std::cmp::Ordering::Equal)
        });
        let mut decoded: Vec<(String, Option<f32>)> = vec![(String::new(), None); regions.len()];
        for chunk in order.chunks(ppocr::REC_BATCH) {
            cancel.check(deadline)?;
            let batch: Vec<(u32, u32, u32, u32)> = chunk.iter().map(|&i| regions[i]).collect();
            let (mut t, width) = ppocr::rec_tensor(image, &batch);
            let out = model
                .rec
                .run(
                    &mut t,
                    &[
                        batch.len() as i64,
                        3,
                        i64::from(ppocr::REC_HEIGHT),
                        i64::from(width),
                    ],
                    |term| Box::new(cancel.on_cancel(move || term.terminate())),
                )
                .map_err(|e| terminated(cancel, deadline, format!("识别推理失败: {e}")))?;
            if out.shape.len() != 3
                || out.shape[0] as usize != batch.len()
                || out.shape[2] as usize != model.classes
            {
                return Err(OcrError::Engine(format!(
                    "识别输出形状异常: {:?}",
                    out.shape
                )));
            }
            let per = out.data.len() / batch.len();
            for (n, &i) in chunk.iter().enumerate() {
                decoded[i] = ppocr::ctc_decode(
                    &out.data[n * per..(n + 1) * per],
                    model.classes,
                    &model.dict,
                );
            }
        }
        phases.push((
            "recMs".to_string(),
            rec_started.elapsed().as_millis() as u64,
        ));
        regions.clear();
        let mut observations = Vec::new();
        let mut empty = 0usize;
        let mut filtered = 0usize;
        for (b, (text, conf)) in boxes.iter().zip(decoded) {
            let text = text.trim().to_string();
            if text.is_empty() {
                empty += 1;
                continue;
            }
            if let (Some(min), Some(c)) = (options.min_confidence, conf) {
                if c < min {
                    filtered += 1;
                    continue;
                }
            }
            // 去掉竖向填充，裁到原图范围
            let y0 = f64::from(b.y0.saturating_sub(top)).min(f64::from(input.height));
            let y1 = f64::from(b.y1.saturating_sub(top)).min(f64::from(input.height));
            let x0 = f64::from(b.x0).min(f64::from(input.width));
            let x1 = f64::from(b.x1).min(f64::from(input.width));
            let rect = OcrRect {
                x: x0,
                y: y0,
                width: (x1 - x0).max(0.0),
                height: (y1 - y0).max(0.0),
            };
            observations.push(OcrObservation {
                text,
                granularity: OcrGranularity::Line,
                bbox: Some(rect),
                polygon: Some([[x0, y0], [x1, y0], [x1, y1], [x0, y1]]),
                confidence: conf,
                label: None,
                lines: Vec::new(),
            });
        }
        if empty > 0 {
            warnings.push(OcrWarning::new(
                "OCR_EMPTY_LINES_DROPPED",
                format!("{empty} 个检测框没有解码出文字，已省略"),
            ));
        }
        if filtered > 0 {
            warnings.push(OcrWarning::new(
                "OCR_LOW_CONFIDENCE_DROPPED",
                format!("{filtered} 行低于 minConfidence，已省略"),
            ));
        }
        Ok(OcrOutput {
            observations,
            raw_text: None,
            truncated: false,
            warnings,
            model_version: self.package.version.to_string(),
            cold_start: cold,
            phases,
            extra: json!({"detInput": {"width": dw, "height": dh}, "verticalPaddingTop": top}),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_are_validated_and_unknown_rejected() {
        assert_eq!(parse_options(&Value::Null).unwrap().det_limit_side_len, 736);
        let ok = parse_options(&json!({"minConfidence": 0.4, "detLimitSideLen": 960})).unwrap();
        assert_eq!(ok.det_limit_side_len, 960);
        for bad in [
            json!({"minConfidence": 2}),
            json!({"detLimitSideLen": 10}),
            json!({"purpose": "text_recognition"}),
            json!([1]),
        ] {
            assert!(matches!(
                parse_options(&bad),
                Err(OcrError::UnsupportedOption { .. })
            ));
        }
    }

    #[test]
    fn det_dims_bounded() {
        assert_eq!(det_dims(650, 200, 736), (2400, 736));
        let (w, h) = det_dims(4000, 200, 736);
        assert!(u64::from(w) * u64::from(h) <= MAX_DET_PIXELS);
        assert_eq!(w % 32, 0);
        assert_eq!(h % 32, 0);
    }

    #[test]
    fn missing_models_report_not_installed_and_never_fall_back() {
        let dir = tempfile::tempdir().unwrap();
        let backend = LocalKoreanOcr::new(dir.path().to_path_buf());
        let info = backend.info();
        assert!(!info.ready);
        assert_eq!(info.data_egress, "none");
        assert!(
            info.state == "not-installed" || info.state == "runtime-unavailable",
            "{}",
            info.state
        );
        let px = vec![255u8; 4 * 4 * 4];
        let input = OcrInput {
            width: 4,
            height: 4,
            rgba: &px,
            png: &[],
        };
        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        let err = backend
            .recognize(
                &input,
                &OcrParams {
                    language: "ko".into(),
                    options: Value::Null,
                },
                &OcrCancel::new(),
                deadline,
            )
            .unwrap_err();
        assert!(matches!(err, OcrError::NotReady { .. }), "{err:?}");
        // 语言与选项在加载前就被拒绝
        let err = backend
            .recognize(
                &input,
                &OcrParams {
                    language: "ja".into(),
                    options: Value::Null,
                },
                &OcrCancel::new(),
                deadline,
            )
            .unwrap_err();
        assert!(matches!(err, OcrError::UnsupportedLanguage { .. }));
    }

    #[cfg(feature = "onnx")]
    #[test]
    fn corrupt_model_is_rejected_before_session_creation() {
        let dir = tempfile::tempdir().unwrap();
        let det = vec![7u8; 64];
        let rec = vec![9u8; 64];
        std::fs::write(dir.path().join("det.onnx"), &det).unwrap();
        std::fs::write(dir.path().join("rec.onnx"), vec![8u8; 64]).unwrap();
        let package = crate::install::tests::fake_package(&det, &rec);
        let backend = LocalKoreanOcr::with_package(dir.path().to_path_buf(), package);
        assert_eq!(backend.info().state, "invalid");
        let err = backend
            .load(
                &OcrCancel::new(),
                Instant::now() + std::time::Duration::from_secs(5),
            )
            .err()
            .unwrap();
        assert!(matches!(err, OcrError::NotReady { ref reason, .. } if reason == "invalid"));
        // 哈希正确但内容不是有效模型：加载报告 invalid，而不是 panic
        std::fs::write(dir.path().join("rec.onnx"), &rec).unwrap();
        let backend = LocalKoreanOcr::with_package(dir.path().to_path_buf(), package);
        assert_eq!(backend.info().state, "ready");
        let err = backend
            .load(
                &OcrCancel::new(),
                Instant::now() + std::time::Duration::from_secs(5),
            )
            .err()
            .unwrap();
        assert!(
            matches!(err, OcrError::NotReady { ref reason, .. } if reason == "invalid"),
            "{err:?}"
        );
        assert!(crate::ort::runtime_version().unwrap().starts_with("1."));
    }
}
