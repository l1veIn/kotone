//! 引擎 #2：sherpa-onnx 流式 Zipformer-zh（FFI），中文流式 + 低延迟
//! feature `engine-sherpa` 控制编译（docs/development.md §3.3、§5.1 stt::sherpa）
//!
//! 当前状态：占位注册（进注册表、设置页可见），`is_ready() = false`，
//! `start_session` 返回「未实现」。真实实现待后续子代理接入 FFI。

use tokio::sync::mpsc;

use super::{EngineCapabilities, SessionConfig, SttEngine, SttEvent, SttSession};

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
        // TODO(stt 子代理)：检查 sherpa-onnx zipformer-zh 模型是否就绪
        false
    }

    fn start_session(
        &self,
        _cfg: &SessionConfig,
        _events: mpsc::UnboundedSender<SttEvent>,
    ) -> Result<Box<dyn SttSession>, String> {
        Err("sherpa-onnx-zipformer-zh 引擎尚未实现：FFI 绑定与模型管理待接入".into())
    }
}
