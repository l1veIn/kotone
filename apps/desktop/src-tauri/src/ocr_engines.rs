//! 桌面端的 OCR 后端集合：统一模型清单里的每个 OCR 条目对应一个后端实例。
//!
//! - 本地包按 `(模型 ID, 包目录)` 缓存实例，已加载的 ONNX 会话在陪玩流程与工具服务之间复用；
//!   模型目录迁移后按新目录创建新实例，不会读到旧目录。
//! - DeepSeek 在线后端每次识别时从当前设置解析所绑定的连接（`modelConfigs` 优先，
//!   其次旧的 `toolService.ocr.deepseekConnectionId`），密钥来自凭据库。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use kotone_core::ocr::{OcrBackend, OcrCancel, OcrInput, OcrParams};
use kotone_core::settings::Settings;
use kotone_ocr::manifest::{self, ModelPackage};
use serde_json::Value;
use tauri::{AppHandle, Manager};

use crate::SharedState;

pub struct OcrEngines {
    local: Mutex<HashMap<(&'static str, PathBuf), Arc<kotone_ocr::local::LocalPpOcr>>>,
    deepseek: Arc<kotone_ocr::DeepSeekOcr>,
}

fn package_dir(root: &std::path::Path, package: &ModelPackage) -> PathBuf {
    // 韩文包保留 M2 起的隔离冒烟覆盖（KOTONE_OCR_MODEL_DIR）。
    if package.id == manifest::KOREAN_MOBILE_ID {
        return kotone_ocr::local_model_dir(root);
    }
    manifest::package_dir(root, package)
}

impl OcrEngines {
    pub fn new(app: &AppHandle) -> Self {
        let source_app = app.clone();
        let source: kotone_ocr::deepseek::ConnectionSource = Arc::new(move || {
            let Some(shared) = source_app.try_state::<SharedState>() else {
                return Err("应用状态尚未就绪".into());
            };
            let id = kotone_ocr::catalog::deepseek_connection_id(&shared.settings.read().unwrap());
            match id {
                None => Ok(None),
                Some(id) => shared.connections.resolve(&id).map(Some),
            }
        });
        Self {
            local: Mutex::new(HashMap::new()),
            deepseek: Arc::new(kotone_ocr::DeepSeekOcr::new(source)),
        }
    }

    fn local(&self, root: &std::path::Path, package: &'static ModelPackage) -> Arc<dyn OcrBackend> {
        let dir = package_dir(root, package);
        let mut cache = self.local.lock().unwrap();
        cache.retain(|(id, cached), _| *id != package.id || *cached == dir);
        cache
            .entry((package.id, dir.clone()))
            .or_insert_with(|| Arc::new(kotone_ocr::local::LocalPpOcr::with_package(dir, *package)))
            .clone()
    }

    /// 全部 OCR 后端（工具服务 `ocr.backends`），本地包目录按给定配置计算。
    pub fn all(&self, settings: &Settings) -> Vec<Arc<dyn OcrBackend>> {
        let root = kotone_stt::model::models_dir_from(settings);
        let mut out: Vec<Arc<dyn OcrBackend>> = manifest::PACKAGES
            .iter()
            .map(|p| self.local(&root, p))
            .collect();
        out.push(self.deepseek.clone());
        out
    }

    /// 清单 ID 对应的后端（陪玩流程按当前设置解析）。
    pub fn by_model(&self, settings: &Settings, model_id: &str) -> Option<Arc<dyn OcrBackend>> {
        if model_id == kotone_ocr::catalog::DEEPSEEK_MODEL_ID {
            return Some(self.deepseek.clone());
        }
        let package = manifest::package(model_id)?;
        Some(self.local(&kotone_stt::model::models_dir_from(settings), package))
    }
}

/// 一行识别结果：文字与它在图里的比例框 `[x, y, w, h]`（识别器没给框时为 None）。
pub type OcrLine = (String, Option<[f64; 4]>);

/// 用陪玩选定的 OCR 模型识别一张图：返回每行文字与它在图里的比例框 `[x, y, w, h]`（0–1）。
/// 自动化的 `ocr()` 与 `game.look` 共用。
pub fn ocr_image(
    app: &AppHandle,
    image: &kotone_tools::image::RgbaImage,
) -> Result<Vec<OcrLine>, String> {
    let settings = app
        .state::<crate::SharedState>()
        .settings
        .read()
        .unwrap()
        .clone();
    let (_, backend) = crate::companion::ocr_backend(app, &settings)?;
    let info = backend.info();
    if !info.ready {
        return Err(format!(
            "文字识别模型还不能用：{}",
            info.reason.unwrap_or(info.state)
        ));
    }
    let png = if backend.kind() == kotone_core::ocr::OcrBackendKind::Online {
        kotone_tools::image::encode_png(image).map_err(|e| e.to_string())?
    } else {
        Vec::new()
    };
    let output = backend
        .recognize(
            &OcrInput {
                width: image.width(),
                height: image.height(),
                rgba: image.pixels(),
                png: &png,
            },
            &OcrParams {
                language: info.default_language,
                options: Value::Null,
            },
            &OcrCancel::new(),
            Instant::now() + Duration::from_secs(20),
        )
        .map_err(|e| format!("文字识别失败：{e}"))?;
    let (w, h) = (f64::from(image.width()), f64::from(image.height()));
    Ok(output
        .observations
        .into_iter()
        .map(|o| {
            let rect = o
                .bbox
                .map(|b| [b.x / w, b.y / h, b.width / w, b.height / h]);
            (o.text, rect)
        })
        .collect())
}
