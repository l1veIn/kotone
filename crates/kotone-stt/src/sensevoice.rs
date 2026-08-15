//! 引擎 #3：sherpa-onnx SenseVoice 多语言（非流式，人工评测第三选手）。
//!
//! 非流式骨架（offline_sherpa.rs）的 SenseVoice 实例：recognizer/session 逻辑
//! 全部在骨架内，本文件只声明模型配置（configure）与语言映射。
//!
//! 非流式会话语义：push_audio 只缓冲 PCM（16kHz mono f32）→ finalize 时
//! 一次性 OfflineRecognizer 转写。识别质量口碑优于 zipformer，代价是边说边不出字。
//!
//! SenseVoice 不支持热词注入（sherpa-onnx 的 per-stream hotwords 仅流式
//! transducer 系支持），capabilities.hotwords = false。
//!
//! feature `engine-sherpa` 控制编译（与 zipformer 共享同一份 sherpa-onnx 原生
//! 依赖）：开启 = 真实实现；关闭 = 占位注册（恒 is_ready=false）。

use tokio::sync::mpsc;

use kotone_core::stt::{EngineCapabilities, SessionConfig, SttEngine, SttEvent, SttSession};

use crate::offline_sherpa::{OfflineEngine, OfflineSpec};

pub const ENGINE_ID: &str = "sherpa-onnx-sensevoice";

/// SenseVoice 实例规格
pub(crate) const SPEC: OfflineSpec = OfflineSpec {
    engine_id: ENGINE_ID,
    display_name: "sherpa-onnx SenseVoice 多语言",
    languages: &["zh", "en", "ja", "ko", "yue"],
    hotwords: false, // SenseVoice 不支持热词注入
    not_ready_hint: "SenseVoice 模型未下载。请在高级页下载",
};

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

/// SenseVoice 引擎 = 非流式骨架的 SenseVoice 实例
pub struct SenseVoiceEngine(OfflineEngine);

impl SenseVoiceEngine {
    #[cfg(feature = "engine-sherpa")]
    pub fn new() -> Self {
        Self(OfflineEngine::new(&SPEC, configure))
    }

    #[cfg(not(feature = "engine-sherpa"))]
    pub fn new() -> Self {
        Self(OfflineEngine::new(&SPEC))
    }
}

impl Default for SenseVoiceEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl SttEngine for SenseVoiceEngine {
    fn id(&self) -> &'static str {
        self.0.id()
    }

    fn display_name(&self) -> &str {
        self.0.display_name()
    }

    fn capabilities(&self) -> EngineCapabilities {
        self.0.capabilities()
    }

    fn is_ready(&self) -> bool {
        self.0.is_ready()
    }

    fn warmup(&self, cfg: &SessionConfig) -> Result<(), String> {
        self.0.warmup(cfg)
    }

    fn unload(&self) {
        self.0.unload()
    }

    fn start_session(
        &self,
        cfg: &SessionConfig,
        events: mpsc::UnboundedSender<SttEvent>,
    ) -> Result<Box<dyn SttSession>, String> {
        self.0.start_session(cfg, events)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_metadata_consistent() {
        let e = SenseVoiceEngine::new();
        assert_eq!(e.id(), "sherpa-onnx-sensevoice");
        let caps = e.capabilities();
        assert!(!caps.streaming, "SenseVoice 必须是非流式引擎");
        assert!(!caps.hotwords, "SenseVoice 不支持热词");
        assert!(caps.offline);
        for lang in ["zh", "en", "ja", "ko", "yue"] {
            assert!(caps.languages.iter().any(|l| l == lang), "缺语言 {lang}");
        }
    }

    #[cfg(feature = "engine-sherpa")]
    #[test]
    fn language_mapping() {
        assert_eq!(map_language("zh"), "zh");
        assert_eq!(map_language("yue"), "yue");
        assert_eq!(map_language("auto"), "auto");
        assert_eq!(map_language(""), "auto");
        assert_eq!(map_language("fr"), "auto");
    }

    #[test]
    fn ready_state_matches_model_files() {
        let e = SenseVoiceEngine::new();
        #[cfg(feature = "engine-sherpa")]
        {
            let id = crate::model::active_model(ENGINE_ID);
            assert_eq!(e.is_ready(), crate::model::multi_model_ready(&id));
        }
        #[cfg(not(feature = "engine-sherpa"))]
        {
            assert!(!e.is_ready(), "占位引擎恒未就绪");
            assert!(e
                .start_session(&SessionConfig::default(), mpsc::unbounded_channel().0)
                .is_err());
        }
    }
}
