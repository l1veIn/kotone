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

/// 填充 SenseVoice 模型家族字段（骨架已设 num_threads/provider）
#[cfg(feature = "engine-sherpa")]
pub(crate) fn configure(
    cfg: &SessionConfig,
    dir: &std::path::Path,
    config: &mut sherpa_onnx::OfflineRecognizerConfig,
) {
    let f = |name: &str| dir.join(name).to_string_lossy().into_owned();
    let language = cfg
        .options
        .get("language")
        .and_then(|v| v.as_str())
        .filter(|v| !v.is_empty())
        .unwrap_or(&cfg.language);
    config.model_config.sense_voice = sherpa_onnx::OfflineSenseVoiceModelConfig {
        model: Some(f("model.int8.onnx")),
        language: Some(map_language(language)),
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
}
