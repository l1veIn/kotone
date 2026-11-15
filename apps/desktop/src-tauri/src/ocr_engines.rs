//! 桌面端的 OCR 后端集合：统一模型清单里的每个 OCR 条目对应一个后端实例。
//!
//! - 本地包按 `(模型 ID, 包目录)` 缓存实例，已加载的 ONNX 会话在陪玩流程与工具服务之间复用；
//!   模型目录迁移后按新目录创建新实例，不会读到旧目录。
//! - DeepSeek 在线后端每次识别时从当前设置解析所绑定的连接（`modelConfigs` 优先，
//!   其次旧的 `toolService.ocr.deepseekConnectionId`），密钥来自凭据库。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use kotone_core::ocr::OcrBackend;
use kotone_core::settings::Settings;
use kotone_ocr::manifest::{self, ModelPackage};
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
