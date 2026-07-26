//! 引擎 #4：X-ASR 流式中英标点（zipformer2r transducer，sherpa-onnx 2026-06 发布）。
//!
//! 在线 transducer 骨架（online_transducer.rs）的 X-ASR 实例。与 zipformer 的
//! 差异仅在模型文件名与建模单元：X-ASR 为 cjkchar+bpe（官方导出附带
//! bpe.model），骨架据此设 modeling_unit + bpe_vocab。model_type 不显式设置
//! ——encoder.int8.onnx 元数据自带 model_type=zipformer2r，C 侧自动探测。
//!
//! 模型发布形态特殊：仅 k2-fsa GitHub releases 整包 tar.bz2（无逐文件镜像），
//! 下载走 model.rs 的 archive 整包解压路线。
//!
//! feature `engine-sherpa` 控制编译（与 zipformer 共享同一份 sherpa-onnx 原生
//! 依赖）：开启 = 真实实现；关闭 = 占位注册（恒 is_ready=false）。

use tokio::sync::mpsc;

use kotone_core::stt::{EngineCapabilities, SessionConfig, SttEngine, SttEvent, SttSession};

use crate::online_transducer::{OnlineTransducerEngine, OnlineTransducerSpec};

pub const ENGINE_ID: &str = "sherpa-onnx-x-asr-zh-en";

/// X-ASR 流式实例规格（tokens.txt 固定名，骨架内拼接）
pub(crate) const SPEC: OnlineTransducerSpec = OnlineTransducerSpec {
    engine_id: ENGINE_ID,
    display_name: "X-ASR 流式中英标点",
    languages: &["zh", "en"],
    encoder_file: "encoder.int8.onnx",
    decoder_file: "decoder.onnx",
    joiner_file: "joiner.int8.onnx",
    bpe_vocab_file: Some("bpe.model"), // cjkchar+bpe 建模单元
    not_ready_hint:
        "X-ASR 模型未下载。请在设置页下载，或运行 kotone-cli download x-asr",
};

/// X-ASR 流式引擎 = 在线 transducer 骨架的 X-ASR 实例
pub struct XAsrEngine(OnlineTransducerEngine);

impl XAsrEngine {
    pub fn new() -> Self {
        Self(OnlineTransducerEngine::from_spec(&SPEC))
    }
}

impl SttEngine for XAsrEngine {
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

    fn warmup(&self) -> Result<(), String> {
        self.0.warmup()
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
        let e = XAsrEngine::new();
        assert_eq!(e.id(), "sherpa-onnx-x-asr-zh-en");
        let caps = e.capabilities();
        assert!(caps.streaming, "X-ASR 必须是流式引擎");
        assert!(caps.hotwords);
        assert!(caps.offline);
        for lang in ["zh", "en"] {
            assert!(caps.languages.iter().any(|l| l == lang), "缺语言 {lang}");
        }
    }

    #[test]
    fn spec_uses_cjkchar_bpe() {
        assert_eq!(SPEC.bpe_vocab_file, Some("bpe.model"));
        // X-ASR 与 zipformer 清单条目各归各的引擎
        let m = crate::model::SHERPA_MODELS
            .iter()
            .find(|m| m.engine_id == ENGINE_ID)
            .expect("X-ASR 模型清单缺失");
        let names: Vec<_> = m.files.iter().map(|f| f.name).collect();
        for need in [SPEC.encoder_file, SPEC.decoder_file, SPEC.joiner_file] {
            assert!(names.contains(&need), "清单缺少 {need}");
        }
    }

    #[test]
    fn ready_state_matches_model_files() {
        let e = XAsrEngine::new();
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
