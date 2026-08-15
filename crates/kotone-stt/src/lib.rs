//! kotone-stt：STT 引擎适配器与模型管理。
//!
//! - 引擎实现：mock（恒在，全链路联调用）、sherpa 流式 / 非流式两个 I/O 循环
//!   （按模型 `recipe` 打开；engine-sherpa feature 关闭时为占位注册）、
//!   以及远程 OpenAI 兼容 STT；
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
pub mod remote_openai;
pub mod online_transducer;
pub mod sensevoice;
pub mod sherpa_runtime;
pub mod vad;
pub mod xasr;

use kotone_core::stt::{EngineRegistry, SttEngine};

/// 内置引擎实例列表（mock-stream 恒在；sherpa 系占位/真实依 feature）
pub fn builtin_engines() -> Vec<Box<dyn SttEngine>> {
    vec![
        Box::new(mock::MockStreamEngine),
        Box::new(sherpa_runtime::SherpaStreamingEngine::new()),
        Box::new(sherpa_runtime::SherpaOfflineEngine::new()),
        Box::new(remote_openai::RemoteOpenaiEngine),
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
        assert!(ids.contains(&"sherpa-streaming".to_string()));
        assert!(ids.contains(&"sherpa-offline".to_string()));
        assert!(ids.contains(&"remote-openai-compat".to_string()));
        assert!(
            !ids.iter().any(|id| id.starts_with("sherpa-onnx-")),
            "家族引擎不应再注册：{ids:?}"
        );
    }

    #[test]
    fn readiness_matches_environment() {
        let mut reg = EngineRegistry::new();
        register_builtin(&mut reg);
        assert!(reg.get("mock-stream").unwrap().is_ready());
        for engine in ["sherpa-streaming", "sherpa-offline"] {
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
        assert_eq!(
            reg.get("sherpa-onnx-x-asr-zh-en").unwrap().id(),
            "sherpa-streaming"
        );
        assert_eq!(
            reg.get("sherpa-onnx-sensevoice").unwrap().id(),
            "sherpa-offline"
        );
        assert_eq!(
            reg.get("sherpa-onnx-funasr-nano").unwrap().id(),
            "sherpa-offline"
        );
        assert!(reg.get("no-such-engine").is_none());
    }
}
