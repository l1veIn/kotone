//! 在线语音合成的配置解析（ADR 048：她的声音在一个地方配）。
//!
//! 每一家要的东西（Key、地址、模型、声音）都在这里解析，每次播报都重新解析，改设置后立即生效：
//! - **Key**：先用声音设置里填的（凭据库 `tts:<模型 ID>`，`kotone_core::model_secrets`）；没填就用同一家
//!   API 连接的 Key（`kotone_core::voice_vendor::vendor_connection`：她正在用的那条是这一家就用它，否则第一条）；
//!   再往后是旧写法（`modelConfigs[…].connection` 选的连接、火山复用语音识别的 Access Token）。
//! - **站**（MiniMax 国内 / 国际，百炼北京 / 新加坡）：Key 来自连接就跟着连接的地址；自己填的 Key 用保存时判断出来的
//!   `modelConfigs[…].region`（[`crate::speech_cmd`]），没有就是国内。
//! - **模型、声音**：`modelConfigs[…].model / voice`，没填用服务商表里的默认值（`kotone_tts::catalog::PROVIDERS`）；
//!   自定义接口没有默认模型和声音，要自己填。
//!
//! Key 只交给合成线程，不出现在返回给界面、工具的任何字段里，也不进日志和错误消息。

use kotone_core::connection::SecretStore;
use kotone_core::settings::Settings;
use kotone_core::voice_vendor::{self, Region};
use kotone_tts::catalog::{OnlineStatus, OPENAI_MODEL_ID, VOLCANO_MODEL_ID};
use kotone_tts::online::{self, OnlineConfig, OnlineKind};
use tauri::{AppHandle, Manager};

use crate::SharedState;

fn field(settings: &Settings, model: &str, key: &str) -> String {
    settings
        .model_configs
        .get(model)
        .and_then(|cfg| cfg.get(key))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .unwrap_or("")
        .to_string()
}

fn or_default(value: String, default: &str) -> String {
    if value.is_empty() {
        default.to_string()
    } else {
        value
    }
}

/// Key 从哪来（给界面和 `speech.status` 看；不含 Key 本身）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum KeySource {
    /// 声音设置里填的（凭据库）。
    Own,
    /// 用这条 API 连接的 Key。
    #[serde(rename_all = "camelCase")]
    Connection { id: String, name: String },
    /// 火山：复用语音识别里填的 Access Token。
    SpeechRecognition,
    /// 自定义接口可以不要 Key（本机的服务）。
    NotNeeded,
    /// 还没有 Key。
    None,
}

/// 解析出来的一家：合成要用的配置，加上 Key 从哪来、哪个站。
#[derive(Debug, Clone)]
pub struct Resolved {
    pub config: OnlineConfig,
    pub key_source: KeySource,
    pub region: Option<Region>,
}

fn connection_name(settings: &Settings, id: &str) -> String {
    settings
        .connections
        .iter()
        .find(|c| c.id == id)
        .map(|c| c.display_name.clone())
        .unwrap_or_else(|| id.to_string())
}

/// 连接的地址和 Key（连接不存在是 None；没存 Key 时 Key 为空）。
fn connection_parts(
    settings: &Settings,
    secrets: &dyn SecretStore,
    id: &str,
) -> Option<(String, String)> {
    let connection = settings.connections.iter().find(|c| c.id == id)?;
    let key = secrets.get(id).ok().flatten().unwrap_or_default();
    Some((connection.base_url.clone(), key))
}

/// 声音设置里填的 Key（凭据库 `tts:<模型 ID>`）。
fn own_key(secrets: &dyn SecretStore, model: &str) -> Option<String> {
    kotone_core::model_secrets::get_from(secrets, model)
}

/// 解析一家（不访问网络）。`Err` 是还没法合成的原因（给玩家看的一句话）。
pub fn resolve(
    settings: &Settings,
    secrets: &dyn SecretStore,
    kind: OnlineKind,
) -> Result<Resolved, String> {
    let provider = kotone_tts::catalog::provider(kind);
    let id = provider.model_id;
    let model = or_default(field(settings, id, "model"), provider.default_model);
    let voice = or_default(field(settings, id, "voice"), provider.default_voice);
    let saved_region = Region::parse(&field(settings, id, "region"));
    let config = |base_url: String, api_key: String| OnlineConfig {
        base_url,
        api_key,
        model: model.clone(),
        app_id: String::new(),
        voice: voice.clone(),
    };
    match kind {
        OnlineKind::Deepseek
        | OnlineKind::Minimax
        | OnlineKind::Dashscope
        | OnlineKind::Siliconflow => {
            if let Some(key) = own_key(secrets, id) {
                let region = saved_region.unwrap_or(Region::Cn);
                return Ok(Resolved {
                    config: config(online::default_origin(kind, region).into(), key),
                    key_source: KeySource::Own,
                    region: provider.regions.then_some(region),
                });
            }
            // 旧写法：DeepSeek 语音在模型配置里选过一条连接
            let chosen = field(settings, id, "connection");
            let engine = kotone_tts::catalog::engine_of_kind(kind);
            let connection = if chosen.is_empty() {
                voice_vendor::vendor_connection(settings, engine)
            } else {
                settings
                    .connections
                    .iter()
                    .find(|c| c.id == chosen)
                    .map(|c| {
                        let region = voice_vendor::voice_of_connection(c)
                            .map_or(Region::Cn, |(_, region)| region);
                        (c.id.clone(), region)
                    })
            };
            let Some((connection_id, region)) = connection else {
                return Err(format!(
                    "{} 还没有 Key：在声音设置里填，或者在「API 连接」里加一条{}的连接",
                    provider.display_name, provider.display_name
                ));
            };
            let (base_url, key) = connection_parts(settings, secrets, &connection_id)
                .ok_or_else(|| format!("{} 用的 API 连接已经删掉了", provider.display_name))?;
            Ok(Resolved {
                config: config(base_url, key),
                key_source: KeySource::Connection {
                    name: connection_name(settings, &connection_id),
                    id: connection_id,
                },
                region: provider.regions.then_some(region),
            })
        }
        OnlineKind::OpenAi => {
            // 自定义接口：地址自己填；旧配置选过一条连接的，照旧用那条连接的地址和 Key
            let chosen = field(settings, OPENAI_MODEL_ID, "connection");
            let legacy = (!chosen.is_empty())
                .then(|| connection_parts(settings, secrets, &chosen))
                .flatten();
            let own = own_key(secrets, OPENAI_MODEL_ID);
            let base_url = match field(settings, OPENAI_MODEL_ID, "baseUrl") {
                url if !url.is_empty() => url,
                _ => legacy
                    .as_ref()
                    .map(|(url, _)| url.clone())
                    .unwrap_or_default(),
            };
            if base_url.is_empty() {
                return Err("自定义语音接口还没有填地址".into());
            }
            let (key, key_source) = match (own, legacy) {
                (Some(key), _) => (key, KeySource::Own),
                (None, Some((_, key))) if !key.is_empty() => (
                    key,
                    KeySource::Connection {
                        name: connection_name(settings, &chosen),
                        id: chosen.clone(),
                    },
                ),
                _ => (String::new(), KeySource::NotNeeded),
            };
            Ok(Resolved {
                config: config(base_url, key),
                key_source,
                region: None,
            })
        }
        OnlineKind::Volcano => {
            // 没填 App ID / Access Token 时复用语音识别里为火山填的那一套（同一个火山语音应用，需在控制台为它
            // 开通语音合成）；语音识别的 Access Token 也在凭据库里（`asr:volcano-asr`，读配置时迁过去的）。
            let asr = kotone_stt::model::VOLCANO_ASR_STT_ID;
            let app_id = or_default(
                field(settings, VOLCANO_MODEL_ID, "appId"),
                &field(settings, asr, "appId"),
            );
            let chosen = field(settings, VOLCANO_MODEL_ID, "connection");
            let (token, key_source) = if let Some(key) = own_key(secrets, VOLCANO_MODEL_ID) {
                (key, KeySource::Own)
            } else if !chosen.is_empty() {
                let key = connection_parts(settings, secrets, &chosen)
                    .map(|(_, key)| key)
                    .unwrap_or_default();
                (
                    key,
                    KeySource::Connection {
                        name: connection_name(settings, &chosen),
                        id: chosen.clone(),
                    },
                )
            } else {
                let key = kotone_core::model_secrets::get_from(secrets, asr)
                    .unwrap_or_else(|| field(settings, asr, "accessToken"));
                let source = if key.is_empty() {
                    KeySource::None
                } else {
                    KeySource::SpeechRecognition
                };
                (key, source)
            };
            Ok(Resolved {
                config: OnlineConfig {
                    base_url: online::VOLCANO_ENDPOINT.into(),
                    api_key: token,
                    model: or_default(
                        field(settings, VOLCANO_MODEL_ID, "cluster"),
                        online::VOLCANO_DEFAULT_CLUSTER,
                    ),
                    app_id,
                    voice,
                },
                key_source,
                region: None,
            })
        }
    }
}

/// 解析一次合成所需的配置（含 Key，只交给合成线程使用）。
pub fn config(app: &AppHandle, kind: OnlineKind) -> Result<OnlineConfig, String> {
    let state = app.state::<SharedState>();
    let settings = state.settings.read().unwrap().clone();
    resolve(&settings, state.secrets.as_ref(), kind).map(|r| r.config)
}

/// 解析结果（含 Key；只在后端用，不往外给）。
pub fn resolved(app: &AppHandle, kind: OnlineKind) -> Result<Resolved, String> {
    let state = app.state::<SharedState>();
    let settings = state.settings.read().unwrap().clone();
    resolve(&settings, state.secrets.as_ref(), kind)
}

/// 配置齐不齐（不访问网络；只看设置和凭据库）：没齐时是原因。
fn missing(kind: OnlineKind, resolved: &Resolved) -> Option<String> {
    let config = &resolved.config;
    let name = kotone_tts::catalog::provider(kind).display_name;
    if config.api_key.trim().is_empty() && kind != OnlineKind::OpenAi {
        return Some(match kind {
            OnlineKind::Volcano => "缺少 Access Token：请填写，或先在语音识别里填写火山配置".into(),
            _ => match &resolved.key_source {
                KeySource::Connection {
                    name: connection, ..
                } => {
                    format!("API 连接「{connection}」还没有保存 Key")
                }
                _ => format!("{name} 还没有 Key"),
            },
        });
    }
    if kind == OnlineKind::Volcano && config.app_id.trim().is_empty() {
        return Some("缺少 App ID：请填写，或先在语音识别里填写火山配置".into());
    }
    if kind == OnlineKind::OpenAi {
        if config.model.trim().is_empty() {
            return Some("自定义语音接口还没有填模型名".into());
        }
        if config.voice.trim().is_empty() {
            return Some("自定义语音接口还没有填声音名".into());
        }
    }
    None
}

/// 清单里显示的就绪状态（不访问网络；只看配置是否齐全、凭据库里有没有 Key）。
pub fn status(app: &AppHandle, kind: OnlineKind) -> OnlineStatus {
    let state = app.state::<SharedState>();
    let settings = state.settings.read().unwrap().clone();
    status_of(&settings, state.secrets.as_ref(), kind)
}

pub fn status_of(settings: &Settings, secrets: &dyn SecretStore, kind: OnlineKind) -> OnlineStatus {
    let reason = match resolve(settings, secrets, kind) {
        Err(reason) => Some(reason),
        Ok(resolved) => missing(kind, &resolved),
    };
    OnlineStatus {
        ready: reason.is_none(),
        reason,
    }
}

/// “自动”跟着她用的模型走（ADR 048）：她的 API 连接是 DeepSeek / MiniMax / 阿里云百炼、这一家有 Key，就用那家的
/// 声音；不是就 None（用系统语音）。不访问网络。
pub fn her_voice_of(
    settings: &Settings,
    secrets: &dyn SecretStore,
) -> Option<kotone_core::assistant::TtsEngine> {
    let (engine, _, _) = voice_vendor::her_voice(settings)?;
    let kind = kotone_tts::catalog::kind_of(engine)?;
    status_of(settings, secrets, kind).ready.then_some(engine)
}

pub fn her_voice(app: &AppHandle) -> Option<kotone_core::assistant::TtsEngine> {
    let state = app.state::<SharedState>();
    let settings = state.settings.read().unwrap().clone();
    her_voice_of(&settings, state.secrets.as_ref())
}

/// 读配置时把旧的隐含默认值写明（ADR 048 去掉了自定义接口写死的默认模型和声音）：以前选了“OpenAI 兼容”、
/// 没填模型 / 声音的，照以前的效果写成 `gpt-4o-mini-tts` / `alloy`，旧设置照样能用；新配的要自己填。
/// 返回改了没有（改了要存盘）。
pub fn migrate_settings(settings: &mut Settings) -> bool {
    let chosen = settings.assistant.tts_model_id.trim() == OPENAI_MODEL_ID;
    let configured = settings.model_configs.contains_key(OPENAI_MODEL_ID);
    if !chosen && !configured {
        return false;
    }
    let entry = settings
        .model_configs
        .entry(OPENAI_MODEL_ID.to_string())
        .or_insert_with(|| serde_json::json!({}));
    let Some(map) = entry.as_object_mut() else {
        return false;
    };
    let mut changed = false;
    for (key, old_default) in [("model", "gpt-4o-mini-tts"), ("voice", "alloy")] {
        let empty =
            !matches!(map.get(key).and_then(|v| v.as_str()), Some(v) if !v.trim().is_empty());
        if empty {
            map.insert(key.into(), serde_json::json!(old_default));
            changed = true;
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use kotone_ai::secrets::MemorySecretStore;
    use kotone_core::connection::{Connection, ConnectionKind};

    fn connection(id: &str, provider: &str, base_url: &str) -> Connection {
        Connection {
            id: id.into(),
            display_name: format!("连接 {id}"),
            kind: ConnectionKind::Remote,
            provider: provider.into(),
            base_url: base_url.into(),
            model: "m".into(),
        }
    }

    #[test]
    fn own_key_wins_then_the_same_vendor_connection() {
        let secrets = MemorySecretStore::default();
        let mut s = Settings {
            connections: vec![connection("mm", "minimax", "https://api.minimax.io/v1")],
            ..Settings::default()
        };
        secrets.set("mm", "connection-key").unwrap();
        // 没填自己的：用 MiniMax 连接的 Key 和地址（国际站）
        let r = resolve(&s, &secrets, OnlineKind::Minimax).unwrap();
        assert_eq!(r.config.api_key, "connection-key");
        assert_eq!(r.config.base_url, "https://api.minimax.io/v1");
        assert_eq!(r.region, Some(Region::Intl));
        assert_eq!(
            r.key_source,
            KeySource::Connection {
                id: "mm".into(),
                name: "连接 mm".into()
            }
        );
        assert_eq!(r.config.model, online::MINIMAX_DEFAULT_MODEL);
        assert_eq!(r.config.voice, online::MINIMAX_DEFAULT_VOICE);
        // 声音设置里填了：用自己的，站是保存时判断出来的
        secrets.set("tts:minimax-tts", "own-key").unwrap();
        s.model_configs.insert(
            "minimax-tts".into(),
            serde_json::json!({"region": "cn", "voice": "female-yujie", "model": "speech-2.8-hd"}),
        );
        let r = resolve(&s, &secrets, OnlineKind::Minimax).unwrap();
        assert_eq!(r.config.api_key, "own-key");
        assert_eq!(r.config.base_url, online::MINIMAX_CN_ORIGIN);
        assert_eq!(r.key_source, KeySource::Own);
        assert_eq!(r.config.voice, "female-yujie");
        assert_eq!(r.config.model, "speech-2.8-hd");
        // 调试输出里没有 Key
        assert!(!format!("{r:?}").contains("own-key"));
    }

    #[test]
    fn missing_keys_say_where_to_put_them() {
        let secrets = MemorySecretStore::default();
        let s = Settings::default();
        for kind in [
            OnlineKind::Minimax,
            OnlineKind::Dashscope,
            OnlineKind::Siliconflow,
        ] {
            let status = status_of(&s, &secrets, kind);
            assert!(!status.ready);
            assert!(status.reason.unwrap().contains("还没有 Key"), "{kind:?}");
        }
        // 连接在，可是没存 Key
        let s = Settings {
            connections: vec![connection(
                "bl",
                "dashscope",
                "https://dashscope.aliyuncs.com/compatible-mode/v1",
            )],
            ..Settings::default()
        };
        let status = status_of(&s, &secrets, OnlineKind::Dashscope);
        assert!(status.reason.unwrap().contains("「连接 bl」"));
        secrets.set("bl", "k").unwrap();
        assert!(status_of(&s, &secrets, OnlineKind::Dashscope).ready);
    }

    #[test]
    fn auto_follows_her_model_only_when_that_vendor_has_a_key() {
        let secrets = MemorySecretStore::default();
        let mut s = Settings {
            connections: vec![
                connection("ds", "deepseek", "https://api.deepseek.com"),
                connection("mm", "minimax-cn", "https://api.minimaxi.com/v1"),
                connection("kimi", "moonshot-cn", "https://api.moonshot.cn/v1"),
            ],
            ..Settings::default()
        };
        s.assistant.connection_id = "mm".into();
        assert_eq!(her_voice_of(&s, &secrets), None, "MiniMax 连接还没有 Key");
        secrets.set("mm", "k").unwrap();
        assert_eq!(
            her_voice_of(&s, &secrets),
            Some(kotone_core::assistant::TtsEngine::Minimax)
        );
        // 她用 Kimi：那家没有声音，用系统语音（即使别的连接有 Key）
        secrets.set("ds", "k").unwrap();
        s.assistant.connection_id = "kimi".into();
        assert_eq!(her_voice_of(&s, &secrets), None);
        s.assistant.connection_id = "ds".into();
        assert_eq!(
            her_voice_of(&s, &secrets),
            Some(kotone_core::assistant::TtsEngine::Deepseek)
        );
    }

    #[test]
    fn custom_interface_needs_its_own_model_and_voice_but_old_settings_keep_working() {
        let secrets = MemorySecretStore::default();
        let mut s = Settings::default();
        s.model_configs.insert(
            OPENAI_MODEL_ID.into(),
            serde_json::json!({"baseUrl": "http://127.0.0.1:8880/v1"}),
        );
        let status = status_of(&s, &secrets, OnlineKind::OpenAi);
        assert!(status.reason.unwrap().contains("模型名"));
        // 本机的服务可以不要 Key
        s.model_configs.insert(
            OPENAI_MODEL_ID.into(),
            serde_json::json!({"baseUrl": "http://127.0.0.1:8880/v1", "model": "kokoro", "voice": "zf_001"}),
        );
        assert!(status_of(&s, &secrets, OnlineKind::OpenAi).ready);
        // 旧配置：选了 OpenAI 兼容、绑了连接、没填模型和声音 → 迁移写明以前的默认值
        let mut old = Settings {
            connections: vec![connection("oa", "openai", "https://api.openai.com/v1")],
            ..Settings::default()
        };
        old.assistant.tts_model_id = OPENAI_MODEL_ID.into();
        old.model_configs.insert(
            OPENAI_MODEL_ID.into(),
            serde_json::json!({"connection": "oa"}),
        );
        secrets.set("oa", "k").unwrap();
        assert!(migrate_settings(&mut old));
        assert!(!migrate_settings(&mut old));
        let r = resolve(&old, &secrets, OnlineKind::OpenAi).unwrap();
        assert_eq!(r.config.model, "gpt-4o-mini-tts");
        assert_eq!(r.config.voice, "alloy");
        assert_eq!(r.config.base_url, "https://api.openai.com/v1");
        assert!(status_of(&old, &secrets, OnlineKind::OpenAi).ready);
        // 没用过自定义接口的配置不动
        let mut fresh = Settings::default();
        assert!(!migrate_settings(&mut fresh));
        assert!(!fresh.model_configs.contains_key(OPENAI_MODEL_ID));
    }

    #[test]
    fn volcano_reuses_the_speech_recognition_token_from_the_credential_store() {
        let secrets = MemorySecretStore::default();
        let mut s = Settings::default();
        s.model_configs.insert(
            kotone_stt::model::VOLCANO_ASR_STT_ID.into(),
            serde_json::json!({"appId": "app-1"}),
        );
        assert!(!status_of(&s, &secrets, OnlineKind::Volcano).ready);
        secrets.set("asr:volcano-asr", "asr-token").unwrap();
        let r = resolve(&s, &secrets, OnlineKind::Volcano).unwrap();
        assert_eq!(r.config.api_key, "asr-token");
        assert_eq!(r.config.app_id, "app-1");
        assert_eq!(r.key_source, KeySource::SpeechRecognition);
        secrets.set("tts:volcano-tts", "own-token").unwrap();
        let r = resolve(&s, &secrets, OnlineKind::Volcano).unwrap();
        assert_eq!(r.config.api_key, "own-token");
        assert_eq!(r.key_source, KeySource::Own);
    }
}
