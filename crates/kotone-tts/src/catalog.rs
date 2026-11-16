//! 统一模型清单的 TTS 类别（docs/product-evolution-game-tools.md §11.3）。
//!
//! 当前语音合成模型由 `assistant.speechEngine`（系统语音 / Kokoro）与新字段 `assistant.ttsModelId`
//! （在线引擎）共同决定，见 `AssistantConfig::tts_engine`；在线引擎不写进旧字段。
//! 声音编号写成 `提供方/声音`，例如 `system/<Windows 语音 token>`、`kokoro/zf_001`、
//! `openai/alloy`、`volcano/BV700_streaming`。在线条目的配置（连接、模型、声音）存在
//! `settings.modelConfigs[<模型 ID>]`，密钥只在凭据库。Edge TTS 不接入（非官方接口）。

use kotone_core::assistant::TtsEngine;
use kotone_core::model_catalog::{
    schema_of, CatalogBackend, CatalogModel, ModelCategory, StaticConfigField,
};
use kotone_core::speech::SpeechVoice;

use crate::online::OnlineKind;

/// Windows 系统语音（SAPI），系统自带，无需下载。
pub const SYSTEM_MODEL_ID: &str = "system-speech";
/// Kokoro 中文（sherpa-onnx int8 v1.1 包，约 147 MB）。
pub const KOKORO_MODEL_ID: &str = "kokoro-zh";
/// OpenAI 兼容的在线语音合成。
pub const OPENAI_MODEL_ID: &str = "openai-compat-tts";
/// 火山语音合成（在线）。
pub const VOLCANO_MODEL_ID: &str = "volcano-tts";
/// Kokoro 包下载大小（tar.bz2）。
pub const KOKORO_ARCHIVE_BYTES: u64 = 147_031_220;

const OPENAI_SCHEMA: &[StaticConfigField] = &[
    StaticConfigField {
        key: "connection",
        label: "API 连接",
        kind: "connection",
        default: "",
        options: &[],
        required: true,
    },
    StaticConfigField {
        key: "model",
        label: "合成模型",
        kind: "string",
        default: "gpt-4o-mini-tts",
        options: &[],
        required: false,
    },
    StaticConfigField {
        key: "voice",
        label: "声音（服务商的声音名，如 alloy）",
        kind: "string",
        default: "alloy",
        options: &[],
        required: true,
    },
];

const VOLCANO_SCHEMA: &[StaticConfigField] = &[
    StaticConfigField {
        key: "appId",
        label: "App ID（留空则复用语音识别里填的火山 App ID）",
        kind: "string",
        default: "",
        options: &[],
        required: false,
    },
    StaticConfigField {
        key: "connection",
        label: "Access Token 所在的连接（留空则复用语音识别里填的 Access Token）",
        kind: "connection",
        default: "",
        options: &[],
        required: false,
    },
    StaticConfigField {
        key: "voice",
        label: "声音",
        kind: "enum",
        default: "BV700_streaming",
        options: &[
            ("BV700_streaming", "灿灿（女声）"),
            ("BV001_streaming", "通用女声"),
            ("BV002_streaming", "通用男声"),
        ],
        required: true,
    },
    StaticConfigField {
        key: "cluster",
        label: "Cluster",
        kind: "string",
        default: crate::online::VOLCANO_DEFAULT_CLUSTER,
        options: &[],
        required: false,
    },
];

/// 清单 ID → 语音引擎。
pub fn engine_of(id: &str) -> Option<TtsEngine> {
    match id {
        SYSTEM_MODEL_ID => Some(TtsEngine::System),
        KOKORO_MODEL_ID => Some(TtsEngine::Kokoro),
        OPENAI_MODEL_ID => Some(TtsEngine::Openai),
        VOLCANO_MODEL_ID => Some(TtsEngine::Volcano),
        _ => None,
    }
}

/// 语音引擎 → 清单 ID。
pub fn model_of(engine: TtsEngine) -> &'static str {
    match engine {
        TtsEngine::System => SYSTEM_MODEL_ID,
        TtsEngine::Kokoro => KOKORO_MODEL_ID,
        TtsEngine::Openai => OPENAI_MODEL_ID,
        TtsEngine::Volcano => VOLCANO_MODEL_ID,
    }
}

/// 声音编号的提供方前缀。
pub fn provider_of(engine: TtsEngine) -> &'static str {
    match engine {
        TtsEngine::System => crate::voice::SYSTEM,
        TtsEngine::Kokoro => crate::voice::KOKORO,
        TtsEngine::Openai => crate::voice::OPENAI,
        TtsEngine::Volcano => crate::voice::VOLCANO,
    }
}

pub fn is_known(id: &str) -> bool {
    engine_of(id).is_some()
}

/// 在线引擎的常用声音（OpenAI 兼容服务的声音名因服务商而异，可在配置里自填）。
pub fn online_voices(kind: OnlineKind) -> Vec<SpeechVoice> {
    let list: &[(&str, &str)] = match kind {
        OnlineKind::OpenAi => &[
            ("alloy", "alloy"),
            ("ash", "ash"),
            ("coral", "coral"),
            ("echo", "echo"),
            ("fable", "fable"),
            ("nova", "nova"),
            ("onyx", "onyx"),
            ("sage", "sage"),
            ("shimmer", "shimmer"),
        ],
        OnlineKind::Volcano => &[
            ("BV700_streaming", "灿灿（女声）"),
            ("BV001_streaming", "通用女声"),
            ("BV002_streaming", "通用男声"),
        ],
    };
    list.iter()
        .map(|(id, name)| SpeechVoice {
            id: format!("{}/{id}", kind.provider()),
            name: (*name).into(),
        })
        .collect()
}

/// 在线条目的就绪状态（由桌面端按设置与凭据库判断）。
#[derive(Debug, Clone, Default)]
pub struct OnlineStatus {
    pub ready: bool,
    pub reason: Option<String>,
}

/// 列表所需的外部状态。
pub struct CatalogState {
    pub active: TtsEngine,
    pub kokoro_installed: bool,
    pub system_error: Option<String>,
    pub openai: OnlineStatus,
    pub volcano: OnlineStatus,
}

/// 在线条目的固定描述。
struct OnlineSpec {
    id: &'static str,
    engine_id: &'static str,
    display_name: &'static str,
    description: &'static str,
    schema: &'static [StaticConfigField],
    api_key_url: &'static str,
}

fn online_entry(spec: &OnlineSpec, status: &OnlineStatus, active: bool) -> CatalogModel {
    CatalogModel {
        category: ModelCategory::Tts,
        id: spec.id.into(),
        engine_id: spec.engine_id.into(),
        display_name: spec.display_name.into(),
        description: spec.description.into(),
        size_bytes: 0,
        backend: CatalogBackend::Remote,
        downloaded: true,
        ready: status.ready,
        state: if status.ready {
            "ready"
        } else {
            "not-configured"
        }
        .into(),
        reason: status.reason.clone(),
        active,
        recommended: false,
        languages: vec!["zh".into()],
        data_egress: "text".into(),
        config_schema: schema_of(spec.schema),
        api_key_url: Some(spec.api_key_url.into()),
        license: "服务商条款".into(),
        files: Vec::new(),
    }
}

/// 列出 TTS 类别的全部条目。
pub fn entries(state: &CatalogState) -> Vec<CatalogModel> {
    let active = state.active;
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
            ready: state.system_error.is_none(),
            state: if state.system_error.is_none() {
                "ready"
            } else {
                "unavailable"
            }
            .into(),
            reason: state.system_error.clone(),
            active: active == TtsEngine::System,
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
            downloaded: state.kokoro_installed,
            ready: state.kokoro_installed,
            state: if state.kokoro_installed {
                "ready"
            } else {
                "not-installed"
            }
            .into(),
            reason: None,
            active: active == TtsEngine::Kokoro,
            recommended: true,
            languages: vec!["zh".into()],
            data_egress: "none".into(),
            config_schema: Vec::new(),
            api_key_url: None,
            license: "Apache-2.0".into(),
            files: Vec::new(),
        },
        online_entry(
            &OnlineSpec {
                id: OPENAI_MODEL_ID,
                engine_id: "openai",
                display_name: "OpenAI 兼容语音合成（在线）",
                description: "调用 /audio/speech 接口（OpenAI 或兼容服务）；每次播报都会把文字发给服务商",
                schema: OPENAI_SCHEMA,
                api_key_url: "https://platform.openai.com/api-keys",
            },
            &state.openai,
            active == TtsEngine::Openai,
        ),
        online_entry(
            &OnlineSpec {
                id: VOLCANO_MODEL_ID,
                engine_id: "volcano",
                display_name: "火山语音合成（在线）",
                description: "豆包语音 HTTP 接口；可复用语音识别里的火山 App ID 与 Access Token（需在控制台为该应用开通语音合成）；每次播报都会把文字发给火山",
                schema: VOLCANO_SCHEMA,
                api_key_url: "https://console.volcengine.com/speech/app",
            },
            &state.volcano,
            active == TtsEngine::Volcano,
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_entry_follows_the_speech_engine_and_online_entries_send_text() {
        let list = entries(&CatalogState {
            active: TtsEngine::Kokoro,
            kokoro_installed: false,
            system_error: None,
            openai: OnlineStatus::default(),
            volcano: OnlineStatus {
                ready: true,
                reason: None,
            },
        });
        assert_eq!(list.len(), 4);
        let kokoro = list.iter().find(|m| m.id == KOKORO_MODEL_ID).unwrap();
        assert!(kokoro.active && !kokoro.ready && kokoro.state == "not-installed");
        let openai = list.iter().find(|m| m.id == OPENAI_MODEL_ID).unwrap();
        assert_eq!(openai.state, "not-configured");
        assert_eq!(openai.data_egress, "text");
        assert!(
            list.iter()
                .find(|m| m.id == VOLCANO_MODEL_ID)
                .unwrap()
                .ready
        );
        for engine in [
            TtsEngine::System,
            TtsEngine::Kokoro,
            TtsEngine::Openai,
            TtsEngine::Volcano,
        ] {
            assert_eq!(engine_of(model_of(engine)), Some(engine));
        }
        assert!(!is_known("edge-tts"));
        assert!(online_voices(OnlineKind::Volcano)[0]
            .id
            .starts_with("volcano/"));
    }
}
