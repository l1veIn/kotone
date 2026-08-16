//! FunASR-Nano 离线配方：只负责填充 sherpa-onnx OfflineRecognizer 的家族字段。
//! 会话循环在 `sherpa_runtime::SherpaOfflineEngine`。
//!
//! 官方默认值取自 sherpa-onnx 文档 CLI 样例
//! （k2-fsa.github.io/sherpa/onnx/funasr-nano）。热词会挤占 max_total_len
//! （约 512 token），这里禁用。

/// 填充 FunASR-Nano 模型家族字段（骨架已设 num_threads/provider）
#[cfg(feature = "engine-sherpa")]
pub(crate) fn configure(
    _cfg: &kotone_core::stt::SessionConfig,
    dir: &std::path::Path,
    config: &mut sherpa_onnx::OfflineRecognizerConfig,
) {
    let f = |name: &str| dir.join(name).to_string_lossy().into_owned();
    config.model_config.funasr_nano = sherpa_onnx::OfflineFunASRNanoModelConfig {
        encoder_adaptor: Some(f("encoder_adaptor.int8.onnx")),
        llm: Some(f("llm.int8.onnx")),
        embedding: Some(f("embedding.int8.onnx")),
        tokenizer: Some(f("Qwen3-0.6B")),
        system_prompt: Some("You are a helpful assistant.".into()),
        user_prompt: Some("语音转写：".into()),
        max_new_tokens: 512,
        temperature: 1e-6,
        top_p: 0.8,
        seed: 42,
        language: None,
        itn: 1,
        hotwords: None,
    };
}
