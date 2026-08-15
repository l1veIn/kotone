//! sherpa I/O 运行时：只注册流式 / 非流式两个循环，按模型 `recipe` 打开。
//!
//! 旧的家族引擎 id（X-ASR / SenseVoice / FunASR-Nano）仍可通过
//! [`kotone_core::stt::canonical_stt_engine`] 映射到这里。

use tokio::sync::mpsc;

use kotone_core::stt::{EngineCapabilities, SessionConfig, SttEngine, SttEvent, SttSession};

use crate::model::{recipe_of, ModelRecipe, SHERPA_OFFLINE_ENGINE_ID, SHERPA_STREAMING_ENGINE_ID};
use crate::offline_sherpa::{OfflineEngine, OfflineSpec};
use crate::online_transducer::{OnlineTransducerEngine, OnlineTransducerSpec};

/// 当前唯一的流式配方（zipformer transducer / X-ASR）。加新流式家族时在此扩表。
pub(crate) const STREAMING_SPEC: OnlineTransducerSpec = OnlineTransducerSpec {
    engine_id: SHERPA_STREAMING_ENGINE_ID,
    display_name: "sherpa 流式",
    languages: &["zh", "en"],
    encoder_file: "encoder.int8.onnx",
    decoder_file: "decoder.onnx",
    joiner_file: "joiner.int8.onnx",
    modeling_unit: "bpe",
    bpe_vocab_file: Some("bpe.vocab"),
    not_ready_hint: "流式模型未下载。请在高级页下载",
};

const OFFLINE_SPEC: OfflineSpec = OfflineSpec {
    engine_id: SHERPA_OFFLINE_ENGINE_ID,
    display_name: "sherpa 非流式",
    languages: &["zh", "en", "ja", "ko", "yue"],
    hotwords: true,
    not_ready_hint: "非流式模型未下载。请在高级页下载",
};

/// 按活动模型 recipe 填充 OfflineRecognizer 家族字段。
#[cfg(feature = "engine-sherpa")]
fn configure_offline(
    cfg: &SessionConfig,
    dir: &std::path::Path,
    config: &mut sherpa_onnx::OfflineRecognizerConfig,
) {
    let model_id = crate::model::model_id_from_cfg(cfg, SHERPA_OFFLINE_ENGINE_ID);
    match recipe_of(&model_id) {
        Some(ModelRecipe::SenseVoice) => crate::sensevoice::configure(cfg, dir, config),
        Some(ModelRecipe::FunasrNano) => crate::funasr_nano::configure(cfg, dir, config),
        other => {
            kotone_core::log::log(&format!(
                "sherpa-offline: 模型 {model_id} 的配方 {other:?} 不是离线配方，跳过家族字段"
            ));
        }
    }
}

/// 流式 I/O 循环（zipformer transducer）。
pub struct SherpaStreamingEngine(OnlineTransducerEngine);

impl SherpaStreamingEngine {
    pub fn new() -> Self {
        Self(OnlineTransducerEngine::from_spec(&STREAMING_SPEC))
    }
}

impl Default for SherpaStreamingEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl SttEngine for SherpaStreamingEngine {
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

/// 非流式 I/O 循环（SenseVoice / FunASR-Nano，按 recipe 分发）。
pub struct SherpaOfflineEngine(OfflineEngine);

impl SherpaOfflineEngine {
    #[cfg(feature = "engine-sherpa")]
    pub fn new() -> Self {
        Self(OfflineEngine::new(&OFFLINE_SPEC, configure_offline))
    }

    #[cfg(not(feature = "engine-sherpa"))]
    pub fn new() -> Self {
        Self(OfflineEngine::new(&OFFLINE_SPEC))
    }
}

impl Default for SherpaOfflineEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl SttEngine for SherpaOfflineEngine {
    fn id(&self) -> &'static str {
        self.0.id()
    }

    fn display_name(&self) -> &str {
        self.0.display_name()
    }

    fn capabilities(&self) -> EngineCapabilities {
        let id = crate::model::active_model(SHERPA_OFFLINE_ENGINE_ID);
        match recipe_of(&id) {
            Some(ModelRecipe::FunasrNano) => EngineCapabilities {
                streaming: false,
                hotwords: true,
                gpu: false,
                offline: true,
                languages: ["zh", "en", "ja"]
                    .iter()
                    .map(|s| (*s).to_string())
                    .collect(),
            },
            _ => EngineCapabilities {
                streaming: false,
                hotwords: false,
                gpu: false,
                offline: true,
                languages: ["zh", "en", "ja", "ko", "yue"]
                    .iter()
                    .map(|s| (*s).to_string())
                    .collect(),
            },
        }
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
    fn streaming_runtime_metadata() {
        let e = SherpaStreamingEngine::new();
        assert_eq!(e.id(), SHERPA_STREAMING_ENGINE_ID);
        assert!(e.capabilities().streaming);
        assert!(e.capabilities().hotwords);
        assert!(e.capabilities().offline);
    }

    #[test]
    fn offline_runtime_metadata() {
        let e = SherpaOfflineEngine::new();
        assert_eq!(e.id(), SHERPA_OFFLINE_ENGINE_ID);
        assert!(!e.capabilities().streaming);
        assert!(e.capabilities().offline);
    }

    #[test]
    fn streaming_spec_matches_xasr_files() {
        let m = crate::model::SHERPA_MODELS
            .iter()
            .find(|m| m.recipe == ModelRecipe::ZipformerTransducer)
            .expect("缺少 zipformer 流式模型");
        let names: Vec<_> = m.files.iter().map(|f| f.name).collect();
        for need in [
            STREAMING_SPEC.encoder_file,
            STREAMING_SPEC.decoder_file,
            STREAMING_SPEC.joiner_file,
        ] {
            assert!(names.contains(&need), "清单缺少 {need}");
        }
    }

    #[test]
    fn ready_state_matches_active_model_files() {
        let streaming = SherpaStreamingEngine::new();
        let offline = SherpaOfflineEngine::new();
        #[cfg(feature = "engine-sherpa")]
        {
            let sid = crate::model::active_model(SHERPA_STREAMING_ENGINE_ID);
            assert_eq!(streaming.is_ready(), crate::model::multi_model_ready(&sid));
            let oid = crate::model::active_model(SHERPA_OFFLINE_ENGINE_ID);
            assert_eq!(offline.is_ready(), crate::model::multi_model_ready(&oid));
        }
        #[cfg(not(feature = "engine-sherpa"))]
        {
            assert!(!streaming.is_ready());
            assert!(!offline.is_ready());
            assert!(streaming
                .start_session(&SessionConfig::default(), mpsc::unbounded_channel().0)
                .is_err());
            assert!(offline
                .start_session(&SessionConfig::default(), mpsc::unbounded_channel().0)
                .is_err());
        }
    }
}
