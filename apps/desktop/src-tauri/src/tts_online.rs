//! 在线语音合成的配置解析（统一模型清单 TTS 类别的在线条目）。
//!
//! - OpenAI 兼容：`modelConfigs["openai-compat-tts"]` 的 `connection`（地址 + 凭据库里的 key）、
//!   `model`、`voice`。
//! - 火山：`modelConfigs["volcano-tts"]` 的 `appId`、`connection`（Access Token 存在该连接的
//!   凭据里）、`voice`、`cluster`；`appId` / `connection` 留空时复用语音识别里为火山填的
//!   App ID / Access Token（同一个火山语音应用，需在控制台为它开通语音合成）。
//! - DeepSeek：`modelConfigs["deepseek-tts"]` 的 `connection`（留空 = 琴音正在用的 DeepSeek 连接，
//!   再退一步用第一条 DeepSeek 连接）、`voice`（默认海星 `stella`）。
//! - 每次播报都重新解析，改设置后立即生效；密钥不出现在返回给界面的任何字段里。

use kotone_core::settings::Settings;
use kotone_tts::catalog::{OnlineStatus, DEEPSEEK_MODEL_ID, OPENAI_MODEL_ID, VOLCANO_MODEL_ID};
use kotone_tts::online::{OnlineConfig, OnlineKind, VOLCANO_ENDPOINT};
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

fn is_deepseek(connection: &kotone_core::connection::Connection) -> bool {
    connection.provider == "deepseek" || connection.base_url.contains("api.deepseek.com")
}

/// DeepSeek 语音用哪条连接：配置里指定的 → 琴音正在用的（若是 DeepSeek）→ 第一条 DeepSeek 连接。
fn deepseek_connection_id(settings: &Settings) -> Option<String> {
    let chosen = field(settings, DEEPSEEK_MODEL_ID, "connection");
    if !chosen.is_empty() {
        return Some(chosen);
    }
    let agent = settings.assistant.connection_id.trim();
    settings
        .connections
        .iter()
        .find(|c| c.id == agent && is_deepseek(c))
        .or_else(|| settings.connections.iter().find(|c| is_deepseek(c)))
        .map(|c| c.id.clone())
}

/// “自动”语音能不能用 DeepSeek：有 DeepSeek 连接且凭据库里有密钥（不访问网络）。
pub fn deepseek_ready(app: &AppHandle) -> bool {
    status(app, OnlineKind::Deepseek).ready
}

/// 解析一次合成所需的配置（含密钥，只交给合成线程使用）。
pub fn config(app: &AppHandle, kind: OnlineKind) -> Result<OnlineConfig, String> {
    let state = app.state::<SharedState>();
    let settings = state.settings.read().unwrap().clone();
    match kind {
        OnlineKind::OpenAi => {
            let id = field(&settings, OPENAI_MODEL_ID, "connection");
            if id.is_empty() {
                return Err("在线语音合成还没有选择 API 连接（模型 → 语音合成）".into());
            }
            let connection = state.connections.resolve(&id)?;
            Ok(OnlineConfig {
                base_url: connection.base_url,
                api_key: connection.api_key.unwrap_or_default(),
                model: or_default(
                    field(&settings, OPENAI_MODEL_ID, "model"),
                    "gpt-4o-mini-tts",
                ),
                app_id: String::new(),
                voice: or_default(field(&settings, OPENAI_MODEL_ID, "voice"), "alloy"),
            })
        }
        OnlineKind::Deepseek => {
            let id = deepseek_connection_id(&settings)
                .ok_or("还没有 DeepSeek 连接：请在「高级 → API 连接」里添加 DeepSeek")?;
            let connection = state.connections.resolve(&id)?;
            Ok(OnlineConfig {
                base_url: connection.base_url,
                api_key: connection.api_key.unwrap_or_default(),
                model: String::new(),
                app_id: String::new(),
                voice: or_default(field(&settings, DEEPSEEK_MODEL_ID, "voice"), "stella"),
            })
        }
        OnlineKind::Volcano => {
            let asr = kotone_stt::model::VOLCANO_ASR_STT_ID;
            let app_id = or_default(
                field(&settings, VOLCANO_MODEL_ID, "appId"),
                &field(&settings, asr, "appId"),
            );
            let id = field(&settings, VOLCANO_MODEL_ID, "connection");
            let token = if id.is_empty() {
                field(&settings, asr, "accessToken")
            } else {
                state.connections.resolve(&id)?.api_key.unwrap_or_default()
            };
            Ok(OnlineConfig {
                base_url: VOLCANO_ENDPOINT.into(),
                api_key: token,
                model: or_default(
                    field(&settings, VOLCANO_MODEL_ID, "cluster"),
                    kotone_tts::online::VOLCANO_DEFAULT_CLUSTER,
                ),
                app_id,
                voice: or_default(
                    field(&settings, VOLCANO_MODEL_ID, "voice"),
                    "BV700_streaming",
                ),
            })
        }
    }
}

/// 清单里显示的就绪状态（不访问网络；只看配置是否齐全、凭据库里有没有 key）。
pub fn status(app: &AppHandle, kind: OnlineKind) -> OnlineStatus {
    match config(app, kind) {
        Err(reason) => OnlineStatus {
            ready: false,
            reason: Some(reason),
        },
        Ok(config) => {
            let missing = if config.api_key.trim().is_empty() {
                Some(match kind {
                    OnlineKind::OpenAi => "所选连接尚未保存 API key",
                    OnlineKind::Deepseek => "DeepSeek 连接尚未保存 API key",
                    OnlineKind::Volcano => {
                        "缺少 Access Token：请选择连接，或先在语音识别里填写火山配置"
                    }
                })
            } else if kind == OnlineKind::Volcano && config.app_id.is_empty() {
                Some("缺少 App ID：请填写，或先在语音识别里填写火山配置")
            } else {
                None
            };
            OnlineStatus {
                ready: missing.is_none(),
                reason: missing.map(str::to_string),
            }
        }
    }
}
