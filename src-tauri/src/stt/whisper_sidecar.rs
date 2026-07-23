//! 引擎 #1：whisper.cpp sidecar 子进程（ggml-small，首启下载）
//! finalize-only（非流式）；feature `engine-whisper-sidecar` 控制编译
//! （docs/development.md §3.3、§5.1 stt::whisper_sidecar）

#![allow(unused)]

use super::{EngineCapabilities, SessionConfig, SttEngine, SttSession, Transcript};

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
        todo!("检查 whisper-cli sidecar 与 ggml-small 模型是否就绪")
    }

    fn start_session(&self, _cfg: &SessionConfig) -> Result<Box<dyn SttSession>, String> {
        todo!("启动 sidecar 生命周期，wav → 文本")
    }
}
