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
    bpe_vocab_file: Some("bpe.vocab"), // cjkchar+bpe；文本词表（可由 bpe.model 导出）
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
        assert_eq!(SPEC.bpe_vocab_file, Some("bpe.vocab"));
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

    /// P0 回归：bpe_vocab 门控——只有「文本 vocab 存在且格式合格」才放行
    /// （C++ Validate 要求文件存在、创建时立即解析，二进制直接 exit）。
    /// 二进制 bpe.model 场景下 resolve 应现场导出文本 vocab 兜底。
    #[test]
    fn bpe_vocab_gating() {
        use crate::online_transducer::gated_bpe_vocab;

        // 合格文本 vocab → Some
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("bpe.vocab"), b"token\t-1.5\n").unwrap();
        assert_eq!(
            gated_bpe_vocab(&SPEC, tmp.path()),
            Some(tmp.path().join("bpe.vocab"))
        );

        // vocab 是二进制（P0 事故文件）且无 bpe.model 兜底 → None（不传 C 侧）
        let tmp2 = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp2.path().join("bpe.vocab"),
            [0x0a, 0x0e, 0x0a, 0x05, b'<', b'b', b'l', b'k', b'>', 0x00, 0x00],
        )
        .unwrap();
        assert_eq!(gated_bpe_vocab(&SPEC, tmp2.path()), None);

        // vocab 缺失 + 空目录 → None
        let tmp3 = tempfile::tempdir().unwrap();
        assert_eq!(gated_bpe_vocab(&SPEC, tmp3.path()), None);

        // 只有 bpe.model（真实用户目录形态）→ 现场导出文本 vocab 并放行
        let tmp4 = tempfile::tempdir().unwrap();
        let mut piece = vec![0x0a, 0x05];
        piece.extend_from_slice(b"<blk>");
        piece.push(0x15);
        piece.extend_from_slice(&0.0f32.to_le_bytes());
        let mut model = vec![0x0a, piece.len() as u8];
        model.extend_from_slice(&piece);
        std::fs::write(tmp4.path().join("bpe.model"), model).unwrap();
        let got = gated_bpe_vocab(&SPEC, tmp4.path()).unwrap();
        assert!(got.ends_with("bpe.vocab"));
        assert!(crate::model::is_valid_bpe_vocab(&got));
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
