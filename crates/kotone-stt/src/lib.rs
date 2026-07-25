//! kotone-stt：STT 引擎适配器与模型管理。
//!
//! - 引擎实现：mock（恒在，全链路联调用）、whisper.cpp sidecar（真实引擎 #1）、
//!   sherpa-onnx（占位）；
//! - `register_builtin`：把内置引擎注入 core 的 EngineRegistry 容器
//!   （依赖方向：kotone-stt → kotone-core，core 不认识任何具体引擎）；
//! - `model`：模型/whisper-cli 运行时清单与下载管理（ADR-003，自管理于 ~/.kotone）；
//! - `download`：通用下载器（流式 + SHA256 校验 + 原子落盘）。

pub mod download;
pub mod mock;
pub mod model;
pub mod sherpa;
pub mod vad;
pub mod whisper_sidecar;

use kotone_core::stt::{EngineRegistry, SttEngine};

/// 内置引擎实例列表（mock-stream 恒在；whisper 真实引擎；sherpa 占位）
pub fn builtin_engines() -> Vec<Box<dyn SttEngine>> {
    // 候选池引擎（funasr / cloud-asr 等）后续按 cargo feature 追加
    vec![
        Box::new(mock::MockStreamEngine),
        Box::new(whisper_sidecar::WhisperSidecarEngine),
        Box::new(sherpa::SherpaEngine::new()),
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
        assert!(ids.contains(&"whisper-cpp-sidecar".to_string()));
        assert!(ids.contains(&"sherpa-onnx-zipformer-zh".to_string()));
    }

    #[test]
    fn readiness_matches_environment() {
        let mut reg = EngineRegistry::new();
        register_builtin(&mut reg);
        assert!(reg.get("mock-stream").unwrap().is_ready());
        // whisper 就绪与否取决于 ~/.kotone 下 bin+模型的真实存在情况
        let whisper_expected = model::bin_installed()
            && model::model_path(&model::active_model("whisper-cpp-sidecar"))
                .is_some_and(|p| p.exists());
        assert_eq!(
            reg.get("whisper-cpp-sidecar").unwrap().is_ready(),
            whisper_expected
        );
        // sherpa：feature 关闭恒未就绪；开启时取决于模型文件齐备情况
        #[cfg(feature = "engine-sherpa")]
        let sherpa_expected = model::multi_model_ready(&model::active_model("sherpa-onnx-zipformer-zh"));
        #[cfg(not(feature = "engine-sherpa"))]
        let sherpa_expected = false;
        assert_eq!(
            reg.get("sherpa-onnx-zipformer-zh").unwrap().is_ready(),
            sherpa_expected
        );
        assert!(reg.get("no-such-engine").is_none());
    }
}
