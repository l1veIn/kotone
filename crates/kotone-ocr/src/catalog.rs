//! 统一模型清单的 OCR 类别（docs/product-evolution-game-tools.md §11.2）。
//!
//! - 本地条目来自 [`crate::manifest::PACKAGES`]，就绪状态逐文件核对大小与 SHA-256。
//! - 在线条目只有 DeepSeek OCR：通过 `connection` 配置字段绑定 AI 连接（密钥在凭据库），
//!   旧配置 `toolService.ocr.deepseekConnectionId` 继续生效。
//! - 当前 OCR 模型：`settings.ocr.activeModelId`；为空时取推荐顺序里第一个已安装的本地包，
//!   不会自动改用在线模型（在线会上传截图，必须由用户明确选择）。

use std::path::{Path, PathBuf};

use kotone_core::model_catalog::{
    schema_of, CatalogBackend, CatalogFile, CatalogModel, ModelCategory, StaticConfigField,
};
use kotone_core::settings::Settings;

use crate::install::{PackageState, Verifier};
use crate::manifest::{self, ModelPackage};

/// DeepSeek 在线 OCR 的清单 ID（同时也是 `ocr.backends` 的后端 ID）。
pub const DEEPSEEK_MODEL_ID: &str = crate::deepseek::BACKEND_ID;

const DEEPSEEK_SCHEMA: &[StaticConfigField] = &[StaticConfigField {
    key: "connection",
    label: "API 连接（DeepSeek）",
    kind: "connection",
    default: "",
    options: &[],
    required: true,
}];

/// 本地包的安装目录：`<模型根>/ocr/<模型 ID>`。
pub fn package_dir(models_root: &Path, package: &ModelPackage) -> PathBuf {
    manifest::package_dir(models_root, package)
}

/// DeepSeek OCR 绑定的连接 ID：统一清单的配置优先，其次旧的 `toolService.ocr` 配置，
/// 都没有就用琴音的 DeepSeek 连接（只配一个 DeepSeek 就能用全部功能）。
pub fn deepseek_connection_id(settings: &Settings) -> Option<String> {
    settings
        .model_configs
        .get(DEEPSEEK_MODEL_ID)
        .and_then(|cfg| cfg.get("connection"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| {
            settings
                .tool_service
                .ocr
                .deepseek_connection_id
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        })
        .or_else(|| settings.deepseek_connection_id())
}

/// 清单里是否有该 OCR 模型 ID。
pub fn is_known(id: &str) -> bool {
    id == DEEPSEEK_MODEL_ID || manifest::package(id).is_some()
}

/// 清单 ID → `ocr.backends` 后端 ID。
pub fn backend_id_of(id: &str) -> Option<&'static str> {
    if id == DEEPSEEK_MODEL_ID {
        return Some(crate::deepseek::BACKEND_ID);
    }
    manifest::package(id).map(|p| p.backend_id)
}

fn package_ready(verifier: &Verifier, models_root: &Path, package: &ModelPackage) -> bool {
    verifier
        .check(&package_dir(models_root, package), package)
        .state
        == PackageState::Ready
}

/// 当前 OCR 模型：显式选择优先；为空时取推荐顺序里第一个已安装的本地包。
pub fn resolve_active(
    settings: &Settings,
    models_root: &Path,
    verifier: &Verifier,
) -> Option<String> {
    let selected = settings.ocr.active_model_id.trim();
    if !selected.is_empty() {
        return Some(selected.to_string());
    }
    manifest::PACKAGES
        .iter()
        .find(|p| package_ready(verifier, models_root, p))
        .map(|p| p.id.to_string())
}

/// 列出 OCR 类别的全部条目。`deepseek_has_key`：所绑定连接是否已保存密钥（由调用方查凭据库）。
pub fn entries(
    settings: &Settings,
    models_root: &Path,
    verifier: &Verifier,
    deepseek_has_key: Option<bool>,
) -> Vec<CatalogModel> {
    let active = resolve_active(settings, models_root, verifier);
    let mut out: Vec<CatalogModel> = manifest::PACKAGES
        .iter()
        .map(|p| {
            let status = verifier.check(&package_dir(models_root, p), p);
            let (ready, state, reason) = match &status.state {
                PackageState::Ready => (true, "ready", None),
                PackageState::NotInstalled => (false, "not-installed", None),
                PackageState::Invalid(m) => {
                    (false, "invalid", Some(format!("模型文件校验失败：{m}")))
                }
            };
            CatalogModel {
                category: ModelCategory::Ocr,
                id: p.id.into(),
                engine_id: p.backend_id.into(),
                display_name: p.display_name.into(),
                description: p.description.into(),
                size_bytes: p.total_bytes(),
                backend: CatalogBackend::Local,
                downloaded: ready,
                ready,
                state: state.into(),
                reason,
                active: active.as_deref() == Some(p.id),
                recommended: p.recommended,
                languages: p.languages.iter().map(|s| s.to_string()).collect(),
                data_egress: "none".into(),
                config_schema: Vec::new(),
                api_key_url: None,
                license: p.license.into(),
                files: p
                    .files
                    .iter()
                    .map(|f| CatalogFile {
                        name: f.name.into(),
                        url: f.url.into(),
                        sha256: f.sha256.into(),
                        size_bytes: f.bytes,
                    })
                    .collect(),
            }
        })
        .collect();
    let connection = deepseek_connection_id(settings);
    let (ready, state, reason) = match (&connection, deepseek_has_key) {
        (None, _) => (
            false,
            "not-configured",
            Some("请选择一条 DeepSeek API 连接".to_string()),
        ),
        (Some(_), Some(false)) => (
            false,
            "not-configured",
            Some("所选连接尚未保存 API key".to_string()),
        ),
        (Some(_), _) => (true, "ready", None),
    };
    out.push(CatalogModel {
        category: ModelCategory::Ocr,
        id: DEEPSEEK_MODEL_ID.into(),
        engine_id: crate::deepseek::BACKEND_ID.into(),
        display_name: "DeepSeek OCR（在线）".into(),
        description: "在线识别，多语种、版面更稳；每次识别都会把截图上传到 DeepSeek".into(),
        size_bytes: 0,
        backend: CatalogBackend::Remote,
        downloaded: true,
        ready,
        state: state.into(),
        reason,
        active: active.as_deref() == Some(DEEPSEEK_MODEL_ID),
        recommended: false,
        languages: vec!["auto".into()],
        data_egress: "selected-image".into(),
        config_schema: schema_of(DEEPSEEK_SCHEMA),
        api_key_url: Some("https://platform.deepseek.com/api_keys".into()),
        license: "DeepSeek API terms".into(),
        files: Vec::new(),
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_selection_never_picks_the_online_model_and_explicit_choice_wins() {
        let root = tempfile::tempdir().unwrap();
        let verifier = Verifier::new();
        let mut settings = Settings::default();
        settings.tool_service.ocr.deepseek_connection_id = Some("ds".into());
        assert_eq!(resolve_active(&settings, root.path(), &verifier), None);
        let list = entries(&settings, root.path(), &verifier, Some(true));
        let online = list.iter().find(|m| m.id == DEEPSEEK_MODEL_ID).unwrap();
        assert!(online.ready && !online.active);
        assert_eq!(online.data_egress, "selected-image");
        assert!(list.iter().all(|m| m.category == ModelCategory::Ocr));
        settings.ocr.active_model_id = DEEPSEEK_MODEL_ID.into();
        assert_eq!(
            resolve_active(&settings, root.path(), &verifier).as_deref(),
            Some(DEEPSEEK_MODEL_ID)
        );
    }

    #[test]
    fn unified_connection_field_overrides_the_legacy_tool_service_setting() {
        let mut settings = Settings::default();
        assert_eq!(deepseek_connection_id(&settings), None);
        settings.tool_service.ocr.deepseek_connection_id = Some("old".into());
        assert_eq!(deepseek_connection_id(&settings).as_deref(), Some("old"));
        settings.model_configs.insert(
            DEEPSEEK_MODEL_ID.into(),
            serde_json::json!({"connection": "new"}),
        );
        assert_eq!(deepseek_connection_id(&settings).as_deref(), Some("new"));
        let list = entries(&settings, Path::new("."), &Verifier::new(), Some(false));
        let online = list.iter().find(|m| m.id == DEEPSEEK_MODEL_ID).unwrap();
        assert_eq!(online.state, "not-configured");
    }
}
