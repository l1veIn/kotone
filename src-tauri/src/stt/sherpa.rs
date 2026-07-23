//! 引擎 #2：sherpa-onnx 流式 Zipformer-zh（FFI），中文流式 + 低延迟
//! feature `engine-sherpa` 控制编译（docs/development.md §3.3、§5.1 stt::sherpa）

#![allow(unused)]

use super::{EngineCapabilities, SessionConfig, SttEngine, SttSession};

pub struct SherpaEngine;

impl SttEngine for SherpaEngine {
    fn id(&self) -> &'static str {
        "sherpa-onnx-zipformer-zh"
    }

    fn display_name(&self) -> &str {
        "sherpa-onnx Zipformer 中文流式"
    }

    fn capabilities(&self) -> EngineCapabilities {
        EngineCapabilities {
            streaming: true,
            hotwords: true,
            gpu: false,
            offline: true,
            languages: vec!["zh".into()],
        }
    }

    fn is_ready(&self) -> bool {
        todo!("检查 sherpa-onnx 模型是否就绪")
    }

    fn start_session(&self, _cfg: &SessionConfig) -> Result<Box<dyn SttSession>, String> {
        todo!("FFI 流式 session，partial 回调 → SttEvent")
    }
}
