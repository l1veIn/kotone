//! 引擎 #6：Qwen3-ASR 0.6B（非流式，conv_frontend + encoder + LLM decoder，
//! Apache 2.0 许可）。
//!
//! 非流式骨架（offline_sherpa.rs）的 Qwen3-ASR 实例。数值参数沿用 crate
//! Default（max_total_len=512、max_new_tokens=128、temperature=1e-6、
//! top_p=0.8、seed=42，与官方默认一致）；tokenizer 传 tokenizer 目录。
//!
//! 热词：模型级 hotwords 字段（recognizer 首建时绑定，之后修改需「重启生效」，
//! 与各 sherpa 引擎同一约定）；capabilities.hotwords = true。
//!
//! feature `engine-sherpa` 控制编译（与 zipformer 共享同一份 sherpa-onnx 原生
//! 依赖）：开启 = 真实实现；关闭 = 占位注册（恒 is_ready=false）。

use tokio::sync::mpsc;

use kotone_core::stt::{EngineCapabilities, SessionConfig, SttEngine, SttEvent, SttSession};

use crate::offline_sherpa::{OfflineEngine, OfflineSpec};

pub const ENGINE_ID: &str = "sherpa-onnx-qwen3-asr";

/// Qwen3-ASR 实例规格。语言：官方声明共 52 种，此处列核心子集
/// （中英日韩粤，覆盖主场景；其余语言模型本身仍支持，只是不在 UI 声明）
pub(crate) const SPEC: OfflineSpec = OfflineSpec {
    engine_id: ENGINE_ID,
    display_name: "Qwen3-ASR 0.6B 多语言",
    languages: &["zh", "en", "ja", "ko", "yue"],
    hotwords: true, // 模型级 hotwords 字段
    not_ready_hint:
        "Qwen3-ASR 模型未下载。请在设置页下载，或运行 kotone-cli download qwen3-asr",
};

/// 填充 Qwen3-ASR 模型家族字段（骨架已设 num_threads/provider）
#[cfg(feature = "engine-sherpa")]
fn configure(
    cfg: &SessionConfig,
    dir: &std::path::Path,
    config: &mut sherpa_onnx::OfflineRecognizerConfig,
) {
    let f = |name: &str| dir.join(name).to_string_lossy().into_owned();
    config.model_config.qwen3_asr = sherpa_onnx::OfflineQwen3ASRModelConfig {
        conv_frontend: Some(f("conv_frontend.onnx")),
        encoder: Some(f("encoder.int8.onnx")),
        decoder: Some(f("decoder.int8.onnx")),
        // tokenizer 传目录（官方 CLI 样例：--qwen3-asr-tokenizer=<dir>/tokenizer）
        tokenizer: Some(f("tokenizer")),
        // 模型级热词：recognizer 首建绑定，修改需重启生效；
        // 空格连接为 upstream 惯例格式，实际分隔语义待真机验证
        hotwords: crate::offline_sherpa::imp::join_hotwords(&cfg.hotwords),
        // 数值参数沿用 crate Default（= 官方默认）：
        // max_total_len=512 / max_new_tokens=128 / temperature=1e-6 / top_p=0.8 / seed=42
        ..Default::default()
    };
}

/// Qwen3-ASR 引擎 = 非流式骨架的 Qwen3-ASR 实例
pub struct Qwen3AsrEngine(OfflineEngine);

impl Qwen3AsrEngine {
    #[cfg(feature = "engine-sherpa")]
    pub fn new() -> Self {
        Self(OfflineEngine::new(&SPEC, configure))
    }

    #[cfg(not(feature = "engine-sherpa"))]
    pub fn new() -> Self {
        Self(OfflineEngine::new(&SPEC))
    }
}

impl SttEngine for Qwen3AsrEngine {
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
        let e = Qwen3AsrEngine::new();
        assert_eq!(e.id(), "sherpa-onnx-qwen3-asr");
        let caps = e.capabilities();
        assert!(!caps.streaming, "Qwen3-ASR 必须是非流式引擎");
        assert!(caps.hotwords, "Qwen3-ASR 支持模型级热词");
        assert!(caps.offline);
        for lang in ["zh", "en", "ja", "ko", "yue"] {
            assert!(caps.languages.iter().any(|l| l == lang), "缺语言 {lang}");
        }
    }

    #[test]
    fn ready_state_matches_model_files() {
        let e = Qwen3AsrEngine::new();
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
