//! 评测目标解析：模型清单在 STT 层，core 仅接收明确的引擎、模型及配置。

use kotone_core::eval::EvalTarget;
use kotone_core::settings::Settings;
use kotone_core::stt::SessionConfig;

use crate::model::{self, ModelRecipe, SHERPA_MODELS};

/// 解析一个本地目标。旧家族 ID 只选该家族的默认模型，不读取当前活动模型。
/// mock 必须显式指定；远程服务不参与语料回放，避免意外上传本地录音。
pub fn resolve_local_target(
    settings: &Settings,
    engine_id: Option<&str>,
    model_id: Option<&str>,
) -> Result<EvalTarget, String> {
    if engine_id == Some("mock-stream") {
        if model_id.is_some_and(|id| id != "mock") {
            return Err("mock-stream 只支持 --model mock".into());
        }
        return Ok(EvalTarget::new(
            "mock-stream",
            "mock",
            SessionConfig::default(),
        ));
    }
    let model_id = match model_id {
        Some(id) => id,
        None => engine_id
            .and_then(model::multi_file_default_model)
            .ok_or("请用 --model 指定本地模型，或用 --engine 指定本地引擎/模型族")?,
    };
    let manifest = SHERPA_MODELS
        .iter()
        .find(|m| m.id == model_id)
        .ok_or_else(|| format!("不支持的本地评测模型: {model_id}（在线模型不参与录档评测）"))?;
    if let Some(engine_id) = engine_id {
        if !model::model_belongs_to_engine(model_id, engine_id) {
            return Err(format!("模型 {model_id} 不属于引擎/模型族 {engine_id}"));
        }
    }

    // 明确模型后再合并其配置；不能让活动模型或另一模型族的旧选项污染目标。
    let mut snapshot = settings.clone();
    snapshot.active_model_id = model_id.into();
    let family_id = match manifest.recipe {
        ModelRecipe::ZipformerTransducer => "sherpa-onnx-x-asr-zh-en",
        ModelRecipe::SenseVoice => "sherpa-onnx-sensevoice",
        ModelRecipe::FunasrNano => "sherpa-onnx-funasr-nano",
        _ => return Err("评测仅支持本地模型".into()),
    };
    let merged = snapshot.session_options(family_id);
    // 仅保存本地后端消费的选项，避免任意遗留配置（例如服务密钥）写入评测文件。
    let mut options = serde_json::json!({});
    for key in ["threads", "language"] {
        if let Some(value) = merged.get(key) {
            options[key] = value.clone();
        }
    }
    Ok(EvalTarget::new(
        manifest.engine_id,
        model_id,
        SessionConfig {
            language: settings.language.clone(),
            // 基线评测不加入当前游戏配置中的热词，目标结果仍完整记录这一选择。
            hotwords: Vec::new(),
            hotwords_score: settings.hotwords_score,
            options,
        },
    ))
}

/// 按本地模型清单枚举全部已安装目标，同一后端的多个模型各占一个目标。
pub fn available_local_targets(settings: &Settings) -> Vec<EvalTarget> {
    local_targets_with(settings, |id| {
        cfg!(feature = "engine-sherpa") && model::multi_model_ready_from(settings, id)
    })
}

fn local_targets_with(settings: &Settings, ready: impl Fn(&str) -> bool) -> Vec<EvalTarget> {
    SHERPA_MODELS
        .iter()
        .filter(|m| ready(m.id))
        .filter_map(|m| resolve_local_target(settings, None, Some(m.id)).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_family_pins_its_model_despite_another_active_model() {
        let settings = Settings {
            active_model_id: "funasr-nano-int8-2025-12-30".into(),
            ..Settings::default()
        };
        let target = resolve_local_target(&settings, Some("sherpa-onnx-sensevoice"), None).unwrap();
        assert_eq!(target.engine_id, "sherpa-offline");
        assert_eq!(target.model_id, "sense-voice-zh-en-ja-ko-yue-2024-07-17");
        assert_eq!(target.session_config.options["model"], target.model_id);
    }

    #[test]
    fn explicit_model_checks_engine_and_rejects_remote_targets() {
        let settings = Settings::default();
        let model = "funasr-nano-int8-2025-12-30";
        assert!(resolve_local_target(&settings, Some("sherpa-offline"), Some(model)).is_ok());
        assert!(
            resolve_local_target(&settings, Some("sherpa-onnx-sensevoice"), Some(model)).is_err()
        );
        assert!(resolve_local_target(&settings, Some("remote-openai-compat"), None).is_err());
        assert!(resolve_local_target(&settings, None, Some("volcano-asr")).is_err());
    }

    #[test]
    fn target_uses_model_specific_options_without_copying_credentials() {
        let mut settings = Settings::default();
        let model = "sense-voice-zh-en-ja-ko-yue-2024-07-17";
        settings.engine_options["sherpa-onnx-sensevoice"] = serde_json::json!({"threads": 2});
        settings.model_configs.insert(
            model.into(),
            serde_json::json!({"threads": 3, "language": "en", "apiKey": "test-secret"}),
        );
        let target = resolve_local_target(&settings, None, Some(model)).unwrap();
        assert_eq!(target.session_config.options["threads"], 3);
        assert_eq!(target.session_config.options["language"], "en");
        assert!(!serde_json::to_string(&target)
            .unwrap()
            .contains("test-secret"));
    }

    #[test]
    fn catalog_enumerates_models_not_just_runtime_engines() {
        let targets = local_targets_with(&Settings::default(), |_| true);
        assert_eq!(targets.len(), SHERPA_MODELS.len());
        assert!(
            targets
                .iter()
                .filter(|t| t.engine_id == "sherpa-offline")
                .count()
                > 1
        );
        assert!(targets.iter().all(|t| t.engine_id.starts_with("sherpa-")));
        let one = local_targets_with(&Settings::default(), |id| id == SHERPA_MODELS[0].id);
        assert_eq!(one.len(), 1);
    }
}
