//! SenseVoice 离线配方：只负责填充 sherpa-onnx OfflineRecognizer 的家族字段。
//! 会话循环在 `sherpa_runtime::SherpaOfflineEngine`。

#[cfg(feature = "engine-sherpa")]
use kotone_core::stt::SessionConfig;

/// SessionConfig.language → SenseVoice language（支持 zh/en/ja/ko/yue/auto；
/// 其它值兜底 auto 自动判别）
#[cfg(feature = "engine-sherpa")]
pub(crate) fn map_language(lang: &str) -> String {
    match lang {
        "zh" | "en" | "ja" | "ko" | "yue" => lang.to_string(),
        _ => "auto".to_string(),
    }
}

/// 识别器用的语言：模型配置里玩家选的（「设置 → 模型 → 语音识别 → SenseVoice」的“识别语言”），没选就是
/// `auto`——和界面上显示的缺省一致。2026-11-25 以前没选时用的是全局的 `settings.language`（缺省 `zh`），
/// 界面写“自动”、实际按中文认；中、英、日、韩游戏短句实测 `auto` 对中文一字不差、对韩语好得多
/// （`tests/interpreter_sensevoice.rs`，ADR 039 实施记录“同传用本体的识别”），所以缺省改成 `auto`，
/// 同传和玩家自己就能共用一份。
#[cfg(feature = "engine-sherpa")]
pub(crate) fn language(cfg: &SessionConfig) -> String {
    let chosen = cfg
        .options
        .get("language")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .unwrap_or("");
    map_language(chosen)
}

/// 填充 SenseVoice 模型家族字段（骨架已设 num_threads/provider）
#[cfg(feature = "engine-sherpa")]
pub(crate) fn configure(
    cfg: &SessionConfig,
    dir: &std::path::Path,
    config: &mut sherpa_onnx::OfflineRecognizerConfig,
) {
    let f = |name: &str| dir.join(name).to_string_lossy().into_owned();
    config.model_config.sense_voice = sherpa_onnx::OfflineSenseVoiceModelConfig {
        model: Some(f("model.int8.onnx")),
        language: Some(language(cfg)),
        use_itn: true, // 逆文本正则（数字/标点落形），聊天场景更自然
    };
    config.model_config.tokens = Some(f("tokens.txt"));
}

#[cfg(all(test, feature = "engine-sherpa"))]
mod tests {
    use super::*;

    #[test]
    fn language_mapping() {
        assert_eq!(map_language("zh"), "zh");
        assert_eq!(map_language("yue"), "yue");
        assert_eq!(map_language("auto"), "auto");
        assert_eq!(map_language(""), "auto");
        assert_eq!(map_language("fr"), "auto");
    }

    #[test]
    fn language_is_the_chosen_one_or_auto() {
        let cfg = |options: serde_json::Value| SessionConfig {
            // 全局的 settings.language（缺省 zh）不再决定 SenseVoice 的语言
            language: "zh".into(),
            options,
            ..Default::default()
        };
        assert_eq!(language(&cfg(serde_json::json!({}))), "auto");
        assert_eq!(language(&cfg(serde_json::json!({"language": ""}))), "auto");
        assert_eq!(language(&cfg(serde_json::json!({"language": "ko"}))), "ko");
        assert_eq!(language(&cfg(serde_json::json!({"language": "zh"}))), "zh");
    }
}
