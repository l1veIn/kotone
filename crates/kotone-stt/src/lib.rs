//! kotone-stt：STT 引擎适配器与模型管理。
//!
//! - 引擎实现：mock（恒在，全链路联调用）、sherpa-onnx X-ASR（流式，默认引擎）、
//!   SenseVoice 与 FunASR-Nano（非流式；同 engine-sherpa feature 门控，
//!   关闭时均为占位注册）；
//! - `register_builtin`：把内置引擎注入 core 的 EngineRegistry 容器
//!   （依赖方向：kotone-stt → kotone-core，core 不认识任何具体引擎）；
//! - `model`：模型清单与下载管理（ADR-003，自管理于 ~/.kotone；
//!   silero VAD 随本体分发，见 model::ensure_vad_model）；
//! - `download`：通用下载器（流式 + SHA256 校验 + 原子落盘 + 镜像回退）。

pub mod download;
pub mod funasr_nano;
pub mod mock;
pub mod model;
pub mod offline_sherpa;
pub mod online_transducer;
pub mod sensevoice;
pub mod vad;
pub mod xasr;

use kotone_core::stt::{EngineRegistry, SttEngine};

/// 内置引擎实例列表（mock-stream 恒在；sherpa 系占位/真实依 feature）
pub fn builtin_engines() -> Vec<Box<dyn SttEngine>> {
    // 候选池引擎（cloud-asr 等）后续按 cargo feature 追加
    vec![
        Box::new(mock::MockStreamEngine),
        Box::new(xasr::XAsrEngine::new()),
        Box::new(sensevoice::SenseVoiceEngine::new()),
        Box::new(funasr_nano::FunAsrNanoEngine::new()),
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
    fn registry_contains_all_builtin_engines() {
        let mut reg = EngineRegistry::new();
        register_builtin(&mut reg);
        let ids: Vec<String> = reg.list_info().iter().map(|i| i.id.clone()).collect();
        assert!(ids.contains(&"mock-stream".to_string()));
        assert!(ids.contains(&"sherpa-onnx-x-asr-zh-en".to_string()));
        assert!(ids.contains(&"sherpa-onnx-sensevoice".to_string()));
        assert!(ids.contains(&"sherpa-onnx-funasr-nano".to_string()));
    }

    #[test]
    fn readiness_matches_environment() {
        let mut reg = EngineRegistry::new();
        register_builtin(&mut reg);
        assert!(reg.get("mock-stream").unwrap().is_ready());
        // sherpa 系三引擎：同一 feature 门控、同一就绪判据（模型文件齐备）
        for engine in [
            "sherpa-onnx-x-asr-zh-en",
            "sherpa-onnx-sensevoice",
            "sherpa-onnx-funasr-nano",
        ] {
            #[cfg(feature = "engine-sherpa")]
            let expected = model::multi_model_ready(&model::active_model(engine));
            #[cfg(not(feature = "engine-sherpa"))]
            let expected = false;
            assert_eq!(
                reg.get(engine).unwrap().is_ready(),
                expected,
                "{engine} 就绪状态与环境不符"
            );
        }
        assert!(reg.get("no-such-engine").is_none());
    }
}
