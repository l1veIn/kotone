//! 桌面端的 OCR 后端集合：统一模型清单里的每个 OCR 条目对应一个后端实例。
//!
//! - 本地包按 `(模型 ID, 包目录)` 缓存实例，已加载的 ONNX 会话在陪玩流程与工具服务之间复用；
//!   模型目录迁移后按新目录创建新实例，不会读到旧目录。
//! - DeepSeek 在线后端每次识别时从当前设置解析所绑定的连接（`modelConfigs` 优先，
//!   其次旧的 `toolService.ocr.deepseekConnectionId`），密钥来自凭据库。
//! - “只用本地识别”的功能（插件的 `Host::screen_text(.., local_only = true)`，海斗三选一，ADR 043 D7）固定用本地包，
//!   和全局的选择无关（[`pick_local`]）：全局选的是本地包就用它，否则用装好的第一个本地包；一个都没有时插件问玩家要不要
//!   下载推荐的那个（[`local_status`]，下载走「设置 → 模型 → 文字识别」同一套，不改全局的选择）。

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

fn settings_of(app: &AppHandle) -> Settings {
    app.state::<crate::SharedState>()
        .settings
        .read()
        .unwrap()
        .clone()
}

/// “只用本地识别”用哪个本地包（ADR 043 D7，和全局的选择无关）：全局选的（`active`，`settings.ocr.activeModelId`）是
/// 本地包、而且装好了就用它；否则用清单顺序（推荐的在前）里装好的第一个；一个都没装好时 None。`ready` 核对一个包装好没有。
pub fn pick_local(
    active: &str,
    ready: impl Fn(&ModelPackage) -> bool,
) -> Option<&'static ModelPackage> {
    manifest::package(active.trim())
        .filter(|p| ready(p))
        .or_else(|| manifest::PACKAGES.iter().find(|p| ready(p)))
}

/// 一个本地包都没装好时推荐下载的那个（清单里标了推荐的第一个：PP-OCRv5 mobile 中英日）。
pub fn recommended_local() -> &'static ModelPackage {
    manifest::PACKAGES
        .iter()
        .find(|p| p.recommended)
        .unwrap_or(&manifest::PACKAGES[0])
}

/// [`pick_local`] 的结果 → 给插件的状态：能用（用哪个）、没有（推荐下载哪个、多大）。
pub fn local_status_with(
    active: &str,
    ready: impl Fn(&ModelPackage) -> bool,
) -> kotone_plugin_api::LocalOcr {
    match pick_local(active, ready) {
        Some(p) => kotone_plugin_api::LocalOcr::Ready {
            model: p.id.to_string(),
            name: p.display_name.to_string(),
        },
        None => {
            let p = recommended_local();
            kotone_plugin_api::LocalOcr::Missing {
                model: p.id.to_string(),
                name: p.display_name.to_string(),
                bytes: p.total_bytes(),
            }
        }
    }
}

/// 这个本地包在当前模型目录里装好了没有（逐文件核对大小和 SHA-256，按大小和修改时间缓存）。
fn local_ready(root: &std::path::Path, package: &ModelPackage) -> bool {
    crate::model_catalog::verifier()
        .check(&package_dir(root, package), package)
        .state
        == kotone_ocr::install::PackageState::Ready
}

/// 本地文字识别能不能用（插件的 `Host::local_ocr`）。
pub fn local_status(app: &AppHandle) -> kotone_plugin_api::LocalOcr {
    let settings = settings_of(app);
    let root = kotone_stt::model::models_dir_from(&settings);
    local_status_with(&settings.ocr.active_model_id, |p| local_ready(&root, p))
}

/// 固定用本地包识别一张图（插件的 `Host::screen_text(.., local_only = true)`），包按 [`pick_local`] 选。
pub fn ocr_image_local(
    app: &AppHandle,
    image: &kotone_tools::image::RgbaImage,
) -> Result<Vec<OcrLine>, String> {
    let settings = settings_of(app);
    let root = kotone_stt::model::models_dir_from(&settings);
    let package = pick_local(&settings.ocr.active_model_id, |p| local_ready(&root, p))
        .ok_or("没有装好的本地文字识别模型")?;
    let backend = app.state::<OcrEngines>().local(&root, package);
    recognize(backend.as_ref(), image)
}

/// 用陪玩选定的 OCR 模型识别一张图：返回每行文字与它在图里的比例框 `[x, y, w, h]`（0–1）。
/// 自动化的 `ocr()` 与 `game.look` 共用。
pub fn ocr_image(
    app: &AppHandle,
    image: &kotone_tools::image::RgbaImage,
) -> Result<Vec<OcrLine>, String> {
    let settings = settings_of(app);
    let (_, backend) = crate::companion::ocr_backend(app, &settings)?;
    recognize(backend.as_ref(), image)
}

fn recognize(
    backend: &dyn OcrBackend,
    image: &kotone_tools::image::RgbaImage,
) -> Result<Vec<OcrLine>, String> {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// ADR 043 D7：三选一固定用本地识别，和全局的选择无关。
    #[test]
    fn local_only_picks_a_local_package_whatever_the_global_choice() {
        let mobile = manifest::CH_MOBILE.id;
        let server = manifest::CH_SERVER.id;
        fn installed<'a>(ids: &'a [&'a str]) -> impl Fn(&ModelPackage) -> bool + 'a {
            move |p| ids.contains(&p.id)
        }
        // 全局选的是在线识别（DeepSeek）、装了一个本地包：用本地的
        let deepseek = kotone_ocr::catalog::DEEPSEEK_MODEL_ID;
        assert_eq!(
            pick_local(deepseek, installed(&[server])).map(|p| p.id),
            Some(server)
        );
        // 全局选的是装好的本地包：就用它（不换成清单里排前面的）
        assert_eq!(
            pick_local(server, installed(&[mobile, server])).map(|p| p.id),
            Some(server)
        );
        // 全局选的本地包没装好（删了、坏了）：用装好的第一个
        assert_eq!(
            pick_local(server, installed(&[mobile])).map(|p| p.id),
            Some(mobile)
        );
        // 没选：照清单顺序（推荐的在前）
        assert_eq!(
            pick_local("", installed(&[server, mobile])).map(|p| p.id),
            Some(mobile)
        );
        // 一个都没装：没有，状态里推荐下载 mobile（约 21 MB）
        assert!(pick_local(deepseek, installed(&[])).is_none());
        match local_status_with(deepseek, installed(&[])) {
            kotone_plugin_api::LocalOcr::Missing { model, bytes, .. } => {
                assert_eq!(model, mobile);
                assert_eq!(bytes, manifest::CH_MOBILE.total_bytes());
                assert!(bytes > 20_000_000 && bytes < 23_000_000, "{bytes}");
            }
            other => panic!("{other:?}"),
        }
        match local_status_with(deepseek, installed(&[mobile])) {
            kotone_plugin_api::LocalOcr::Ready { model, .. } => assert_eq!(model, mobile),
            other => panic!("{other:?}"),
        }
        assert!(recommended_local().recommended);
    }
}
