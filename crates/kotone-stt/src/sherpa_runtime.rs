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
    max_audio_seconds: None,
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
                hotwords: false,
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
        assert_eq!(
            STREAMING_SPEC.modeling_unit, "bpe",
            "X-ASR tokens 使用 ▁ 前缀的 SentencePiece piece；cjkchar+bpe 会把中文拆成词表中不存在的裸字"
        );
        assert_eq!(STREAMING_SPEC.bpe_vocab_file, Some("bpe.vocab"));
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
    fn bpe_vocab_gating() {
        use crate::online_transducer::gated_bpe_vocab;

        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("bpe.vocab"), b"token\t-1.5\n").unwrap();
        assert_eq!(
            gated_bpe_vocab(&STREAMING_SPEC, tmp.path()),
            Some(tmp.path().join("bpe.vocab"))
        );

        let tmp2 = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp2.path().join("bpe.vocab"),
            [
                0x0a, 0x0e, 0x0a, 0x05, b'<', b'b', b'l', b'k', b'>', 0x00, 0x00,
            ],
        )
        .unwrap();
        assert_eq!(gated_bpe_vocab(&STREAMING_SPEC, tmp2.path()), None);

        let tmp3 = tempfile::tempdir().unwrap();
        assert_eq!(gated_bpe_vocab(&STREAMING_SPEC, tmp3.path()), None);

        let tmp4 = tempfile::tempdir().unwrap();
        let mut piece = vec![0x0a, 0x05];
        piece.extend_from_slice(b"<blk>");
        piece.push(0x15);
        piece.extend_from_slice(&0.0f32.to_le_bytes());
        let mut model = vec![0x0a, piece.len() as u8];
        model.extend_from_slice(&piece);
        std::fs::write(tmp4.path().join("bpe.model"), model).unwrap();
        let got = gated_bpe_vocab(&STREAMING_SPEC, tmp4.path()).unwrap();
        assert!(got.ends_with("bpe.vocab"));
        assert!(crate::model::is_valid_bpe_vocab(&got));
    }

    #[cfg(feature = "engine-sherpa")]
    #[test]
    fn tail_padding_and_decode_cap_contract() {
        use crate::online_transducer::imp::{
            silence_tail, MAX_FINALIZE_DECODE_ROUNDS, TAIL_PADDING_MS,
        };
        assert_eq!(
            silence_tail().len(),
            TAIL_PADDING_MS * 16,
            "16kHz 每秒 16000 采样"
        );
        // 编译期合同：常量比较不要写成运行时 assert!（clippy::assertions_on_constants）
        const _: () = assert!(TAIL_PADDING_MS >= 480, "尾帧须覆盖 X-ASR 480ms lookahead");
        const _: () = assert!(
            MAX_FINALIZE_DECODE_ROUNDS >= 64 && MAX_FINALIZE_DECODE_ROUNDS <= 4096,
            "防挂死上限应在 64..=4096"
        );
    }

    /// 真机冒烟：使用已下载的 X-ASR 创建带中英混合热词的 stream。
    /// 手动运行：
    /// cargo test -p kotone-stt --features engine-sherpa
    ///   sherpa_runtime::tests::local_model_accepts_lol_hotwords -- --ignored --nocapture
    #[cfg(feature = "engine-sherpa")]
    #[test]
    #[ignore = "依赖本机已下载的 X-ASR 真模型"]
    fn local_model_accepts_lol_hotwords() {
        let e = SherpaStreamingEngine::new();
        if !e.is_ready() {
            eprintln!("X-ASR 模型未下载，跳过真机热词冒烟");
            return;
        }
        e.warmup(&SessionConfig::default()).expect("X-ASR 预热失败");
        let hotwords = std::env::var("KOTONE_TEST_HOTWORDS")
            .ok()
            .map(|value| {
                value
                    .split('|')
                    .filter(|word| !word.trim().is_empty())
                    .map(|word| word.trim().to_string())
                    .collect()
            })
            .unwrap_or_else(|| {
                vec![
                    "闪现".into(),
                    "大龙".into(),
                    "gank".into(),
                    "打野".into(),
                    "悠米".into(),
                    "璐璐".into(),
                    "残影".into(),
                    "无尽".into(),
                ]
            });
        let cfg = SessionConfig {
            language: "zh".into(),
            hotwords,
            options: serde_json::Value::Null,
            ..Default::default()
        };
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut session = e
            .start_session(&cfg, tx)
            .expect("创建带热词的 X-ASR stream 失败");
        if let Ok(wav) = std::env::var("KOTONE_TEST_HOTWORD_WAV") {
            let pcm = kotone_core::eval::read_wav(std::path::Path::new(&wav))
                .expect("读取 KOTONE_TEST_HOTWORD_WAV 失败");
            for chunk in pcm.chunks(800) {
                session.push_audio(chunk).expect("推送测试音频失败");
            }
            let transcript = session.finalize().expect("测试音频收尾失败");
            eprintln!("热词 WAV 识别结果：{}", transcript.text);
            if let Ok(expected) = std::env::var("KOTONE_TEST_EXPECTED") {
                assert!(
                    transcript.text.contains(&expected),
                    "识别结果未包含期望热词「{expected}」：{}",
                    transcript.text
                );
            }
        } else {
            session.cancel();
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
