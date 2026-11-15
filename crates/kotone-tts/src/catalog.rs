//! 统一模型清单的 TTS 类别（docs/product-evolution-game-tools.md §11.3）。
//!
//! 当前语音合成模型由 `settings.assistant.speechEngine` 决定（与设置页「语音」一致）。
//! 声音编号写成 `提供方/声音`，例如 `system/Microsoft Huihui Desktop`、`kokoro/zf_001`。

use kotone_core::assistant::SpeechEngine;
use kotone_core::model_catalog::{CatalogBackend, CatalogModel, ModelCategory};

/// Windows 系统语音（SAPI），系统自带，无需下载。
pub const SYSTEM_MODEL_ID: &str = "system-speech";
/// Kokoro 中文（sherpa-onnx int8 v1.1 包，约 147 MB）。
pub const KOKORO_MODEL_ID: &str = "kokoro-zh";
/// Kokoro 包下载大小（tar.bz2）。
pub const KOKORO_ARCHIVE_BYTES: u64 = 147_031_220;

/// 清单 ID → 语音引擎。
pub fn engine_of(id: &str) -> Option<SpeechEngine> {
    match id {
        SYSTEM_MODEL_ID => Some(SpeechEngine::System),
        KOKORO_MODEL_ID => Some(SpeechEngine::Kokoro),
        _ => None,
    }
}

/// 语音引擎 → 清单 ID。
pub fn model_of(engine: SpeechEngine) -> &'static str {
    match engine {
        SpeechEngine::System => SYSTEM_MODEL_ID,
        SpeechEngine::Kokoro => KOKORO_MODEL_ID,
    }
}

pub fn is_known(id: &str) -> bool {
    engine_of(id).is_some()
}

/// 列出 TTS 类别的全部条目。`kokoro_installed`：Kokoro 包已完整安装；
/// `system_error`：系统语音不可用时的原因。
pub fn entries(
    active: SpeechEngine,
    kokoro_installed: bool,
    system_error: Option<String>,
) -> Vec<CatalogModel> {
    vec![
        CatalogModel {
            category: ModelCategory::Tts,
            id: SYSTEM_MODEL_ID.into(),
            engine_id: "system".into(),
            display_name: "Windows 系统语音".into(),
            description: "系统自带，无需下载；声音取决于系统已安装的语音包".into(),
            size_bytes: 0,
            backend: CatalogBackend::Local,
            downloaded: true,
            ready: system_error.is_none(),
            state: if system_error.is_none() {
                "ready"
            } else {
                "unavailable"
            }
            .into(),
            reason: system_error,
            active: active == SpeechEngine::System,
            recommended: false,
            languages: vec!["zh".into()],
            data_egress: "none".into(),
            config_schema: Vec::new(),
            api_key_url: None,
            license: "Windows".into(),
            files: Vec::new(),
        },
        CatalogModel {
            category: ModelCategory::Tts,
            id: KOKORO_MODEL_ID.into(),
            engine_id: "kokoro".into(),
            display_name: "Kokoro 中文（本地神经语音）".into(),
            description: "更自然的本地中文女声，约 147 MB；本机 CPU 合成，不上传文字".into(),
            size_bytes: KOKORO_ARCHIVE_BYTES,
            backend: CatalogBackend::Local,
            downloaded: kokoro_installed,
            ready: kokoro_installed,
            state: if kokoro_installed {
                "ready"
            } else {
                "not-installed"
            }
            .into(),
            reason: None,
            active: active == SpeechEngine::Kokoro,
            recommended: true,
            languages: vec!["zh".into()],
            data_egress: "none".into(),
            config_schema: Vec::new(),
            api_key_url: None,
            license: "Apache-2.0".into(),
            files: Vec::new(),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_entry_follows_the_speech_engine() {
        let list = entries(SpeechEngine::Kokoro, false, None);
        assert_eq!(list.len(), 2);
        let kokoro = list.iter().find(|m| m.id == KOKORO_MODEL_ID).unwrap();
        assert!(kokoro.active && !kokoro.ready && kokoro.state == "not-installed");
        assert_eq!(
            engine_of(model_of(SpeechEngine::System)),
            Some(SpeechEngine::System)
        );
        assert!(!is_known("edge-tts"));
    }
}
