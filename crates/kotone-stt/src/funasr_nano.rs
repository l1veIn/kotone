//! 引擎 #5：FunASR-Nano（非流式，encoder_adaptor + Qwen3-0.6B LLM + embedding）。
//!
//! 非流式骨架（offline_sherpa.rs）的 FunASR-Nano 实例。官方默认值取自
//! sherpa-onnx 文档的 CLI 样例（k2-fsa.github.io/sherpa/onnx/funasr-nano）：
//! system_prompt="You are a helpful assistant."、user_prompt="语音转写："、
//! max_new_tokens=512、temperature=1e-6、top_p=0.8、seed=42、itn=1、
//! language 留空（auto）；tokenizer 传 Qwen3-0.6B 目录而非单文件。
//!
//! 热词：模型级 hotwords 字段（recognizer 首建时绑定，之后修改需「重启生效」，
//! 与各 sherpa 引擎同一约定）；capabilities.hotwords = true。
//!
//! feature `engine-sherpa` 控制编译（与 zipformer 共享同一份 sherpa-onnx 原生
//! 依赖）：开启 = 真实实现；关闭 = 占位注册（恒 is_ready=false）。

use tokio::sync::mpsc;

use kotone_core::stt::{EngineCapabilities, SessionConfig, SttEngine, SttEvent, SttSession};

use crate::offline_sherpa::{OfflineEngine, OfflineSpec};

pub const ENGINE_ID: &str = "sherpa-onnx-funasr-nano";

/// FunASR-Nano 实例规格
pub(crate) const SPEC: OfflineSpec = OfflineSpec {
    engine_id: ENGINE_ID,
    display_name: "FunASR-Nano 中英日",
    languages: &["zh", "en", "ja"],
    hotwords: false, // FunASR-Nano 的热词会挤占 max_total_len（约 512 token），
    // LOL 百条热词必然回退，故禁用并在 UI 提示不支持热词
    not_ready_hint: "FunASR-Nano 模型未下载。请在高级页下载",
    // FunASR-Nano 的时长上限由 offline_sherpa 按 recipe 动态施加（FUNASR_MAX_AUDIO_SECONDS）
    max_audio_seconds: None,
};

/// 填充 FunASR-Nano 模型家族字段（骨架已设 num_threads/provider）
#[cfg(feature = "engine-sherpa")]
pub(crate) fn configure(
    _cfg: &SessionConfig,
    dir: &std::path::Path,
    config: &mut sherpa_onnx::OfflineRecognizerConfig,
) {
    let f = |name: &str| dir.join(name).to_string_lossy().into_owned();
    config.model_config.funasr_nano = sherpa_onnx::OfflineFunASRNanoModelConfig {
        encoder_adaptor: Some(f("encoder_adaptor.int8.onnx")),
        llm: Some(f("llm.int8.onnx")),
        embedding: Some(f("embedding.int8.onnx")),
        // tokenizer 传目录（官方 CLI 样例：--funasr-nano-tokenizer=<dir>/Qwen3-0.6B）
        tokenizer: Some(f("Qwen3-0.6B")),
        system_prompt: Some("You are a helpful assistant.".into()),
        user_prompt: Some("语音转写：".into()),
        max_new_tokens: 512,
        temperature: 1e-6,
        top_p: 0.8,
        seed: 42,
        language: None, // 空 = auto 自动判别
        itn: 1,         // 逆文本正则（数字/标点落形），官方默认 itn=True
        // 禁用热词：LOL 百条热词挤占 max_total_len 会导致回退丢字
        hotwords: None,
    };
}

/// FunASR-Nano 引擎 = 非流式骨架的 FunASR-Nano 实例
pub struct FunAsrNanoEngine(OfflineEngine);

impl FunAsrNanoEngine {
    #[cfg(feature = "engine-sherpa")]
    pub fn new() -> Self {
        Self(OfflineEngine::new(&SPEC, configure))
    }

    #[cfg(not(feature = "engine-sherpa"))]
    pub fn new() -> Self {
        Self(OfflineEngine::new(&SPEC))
    }
}

impl Default for FunAsrNanoEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl SttEngine for FunAsrNanoEngine {
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
        let e = FunAsrNanoEngine::new();
        assert_eq!(e.id(), "sherpa-onnx-funasr-nano");
        let caps = e.capabilities();
        assert!(!caps.streaming, "FunASR-Nano 必须是非流式引擎");
        assert!(!caps.hotwords, "FunASR-Nano 热词会挤占 max_total_len，已禁用");
        assert!(caps.offline);
        for lang in ["zh", "en", "ja"] {
            assert!(caps.languages.iter().any(|l| l == lang), "缺语言 {lang}");
        }
    }

    #[test]
    fn ready_state_matches_model_files() {
        let e = FunAsrNanoEngine::new();
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
