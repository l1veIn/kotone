//! 引擎 #1：whisper.cpp sidecar 子进程（ggml-small，首启下载）
//! finalize-only（非流式）；feature `engine-whisper-sidecar` 控制编译
//! （docs/development.md §3.3、§5.1 stt::whisper_sidecar）
//!
//! 当前状态：占位注册（进注册表、设置页可见），`is_ready() = false`，
//! `start_session` 返回「未实现」。真实实现待后续子代理接入 sidecar。

use tokio::sync::mpsc;

use kotone_core::stt::{EngineCapabilities, SessionConfig, SttEngine, SttEvent, SttSession};

pub struct WhisperSidecarEngine;

impl SttEngine for WhisperSidecarEngine {
    fn id(&self) -> &'static str {
        "whisper-cpp-sidecar"
    }

    fn display_name(&self) -> &str {
        "whisper.cpp (sidecar)"
    }

    fn capabilities(&self) -> EngineCapabilities {
        EngineCapabilities {
            streaming: false,
            hotwords: true, // initial_prompt 热词
            gpu: false,
            offline: true,
            languages: vec!["zh".into(), "en".into()],
        }
    }

    fn is_ready(&self) -> bool {
        // TODO(stt 子代理)：检查 whisper-cli sidecar 与 ggml-small 模型是否就绪
        false
    }

    fn start_session(
        &self,
        _cfg: &SessionConfig,
        _events: mpsc::UnboundedSender<SttEvent>,
    ) -> Result<Box<dyn SttSession>, String> {
        Err("whisper-cpp-sidecar 引擎尚未实现：sidecar 与模型管理待接入".into())
    }
}
