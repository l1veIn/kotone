//! kotone-stt：STT 引擎适配器与模型管理。
//!
//! - 引擎实现：mock（恒在，全链路联调用）、whisper.cpp sidecar、sherpa-onnx（占位）；
//! - `register_builtin`：把内置引擎注入 core 的 EngineRegistry 容器
//!   （依赖方向：kotone-stt → kotone-core，core 不认识任何具体引擎）；
//! - `model`：各引擎模型的下载/校验/切换（签名就位，实现待做）。

pub mod mock;
pub mod model;
pub mod sherpa;
pub mod whisper_sidecar;

use kotone_core::stt::{EngineRegistry, SttEngine};

/// 内置引擎实例列表（mock-stream 恒在；whisper/sherpa 占位，is_ready=false）
pub fn builtin_engines() -> Vec<Box<dyn SttEngine>> {
    // 候选池引擎（funasr / cloud-asr 等）后续按 cargo feature 追加
    vec![
        Box::new(mock::MockStreamEngine),
        Box::new(whisper_sidecar::WhisperSidecarEngine),
        Box::new(sherpa::SherpaEngine),
    ]
}

/// 把全部内置引擎注入注册表容器
pub fn register_builtin(registry: &mut EngineRegistry) {
    for e in builtin_engines() {
        registry.register(e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_contains_mock_and_placeholders() {
        let mut reg = EngineRegistry::new();
        register_builtin(&mut reg);
        let ids: Vec<String> = reg.list_info().iter().map(|i| i.id.clone()).collect();
        assert!(ids.contains(&"mock-stream".to_string()));
        assert!(ids.contains(&"whisper-cpp-sidecar".to_string()));
        assert!(ids.contains(&"sherpa-onnx-zipformer-zh".to_string()));
    }

    #[test]
    fn mock_is_ready_placeholders_are_not() {
        let mut reg = EngineRegistry::new();
        register_builtin(&mut reg);
        assert!(reg.get("mock-stream").unwrap().is_ready());
        assert!(!reg.get("whisper-cpp-sidecar").unwrap().is_ready());
        assert!(!reg.get("sherpa-onnx-zipformer-zh").unwrap().is_ready());
        assert!(reg.get("no-such-engine").is_none());
    }
}
