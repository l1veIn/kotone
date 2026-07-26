//! 引擎 #2：sherpa-onnx 流式 Zipformer 中英双语（ADR-004）。
//!
//! 在线 transducer 骨架（online_transducer.rs）的 zipformer 实例：recognizer/
//! session 逻辑全部在骨架内，本文件只声明模型文件名与元数据。
//!
//! feature `engine-sherpa` 控制编译：开启 = 真实实现；关闭 = 占位注册
//! （恒 is_ready=false，默认构建零原生依赖）。

use tokio::sync::mpsc;

use kotone_core::stt::{EngineCapabilities, SessionConfig, SttEngine, SttEvent, SttSession};

use crate::online_transducer::{OnlineTransducerEngine, OnlineTransducerSpec};

pub const ENGINE_ID: &str = "sherpa-onnx-zipformer-zh";

/// zipformer 中英双语实例规格（tokens.txt 固定名，骨架内拼接）
pub(crate) const SPEC: OnlineTransducerSpec = OnlineTransducerSpec {
    engine_id: ENGINE_ID,
    display_name: "sherpa-onnx Zipformer 中文流式",
    languages: &["zh", "en"],
    encoder_file: "encoder-epoch-99-avg-1.int8.onnx",
    decoder_file: "decoder-epoch-99-avg-1.onnx",
    joiner_file: "joiner-epoch-99-avg-1.int8.onnx",
    bpe_vocab_file: None, // cjkchar 建模单元，无 bpe.model
    not_ready_hint: "sherpa 模型未下载。请在设置页下载，或运行 kotone-cli download zipformer",
};

/// sherpa 流式引擎 = 在线 transducer 骨架的 zipformer 实例
pub struct SherpaEngine(OnlineTransducerEngine);

impl SherpaEngine {
    pub fn new() -> Self {
        Self(OnlineTransducerEngine::from_spec(&SPEC))
    }
}

impl SttEngine for SherpaEngine {
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
        let e = SherpaEngine::new();
        assert_eq!(e.id(), "sherpa-onnx-zipformer-zh");
        let caps = e.capabilities();
        assert!(caps.streaming, "sherpa 必须是流式引擎");
        assert!(caps.hotwords);
        assert!(caps.offline);
        assert!(caps.languages.iter().any(|l| l == "zh"));
    }

    #[cfg(feature = "engine-sherpa")]
    #[test]
    fn hotwords_format_one_phrase_per_line() {
        assert_eq!(
            crate::online_transducer::imp::format_hotwords(&[
                "闪现".into(),
                "大龙".into(),
                "gank".into()
            ]),
            "闪现\n大龙\ngank"
        );
        assert_eq!(crate::online_transducer::imp::format_hotwords(&[]), "");
    }

    /// partial 变化检测逻辑（不依赖原生库，用同构状态机验证）
    #[test]
    fn partial_emit_only_on_change() {
        struct ChangeDetector {
            last: String,
            emitted: Vec<String>,
        }
        impl ChangeDetector {
            fn emit_if_changed(&mut self, text: &str) {
                if !text.is_empty() && text != self.last {
                    self.last = text.to_string();
                    self.emitted.push(text.to_string());
                }
            }
        }
        let mut d = ChangeDetector {
            last: String::new(),
            emitted: Vec::new(),
        };
        d.emit_if_changed(""); // 空文本不发
        d.emit_if_changed("对面");
        d.emit_if_changed("对面"); // 重复不发
        d.emit_if_changed("对面打野");
        d.emit_if_changed("对面打野在下路");
        assert_eq!(d.emitted, vec!["对面", "对面打野", "对面打野在下路"]);
    }

    #[test]
    fn ready_state_matches_model_files() {
        let e = SherpaEngine::new();
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
