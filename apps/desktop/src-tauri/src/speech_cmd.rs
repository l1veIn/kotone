//! 她的声音（ADR 048）：声音设置页和 `speech.status` 用的后端——每一家的状态（Key 从哪来、怎么计费）、
//! 保存 Key 时先试一次（顺便判断国内 / 国际站）、在线声音列表（MiniMax）、试听。
//!
//! Key 只从 [`speech_key_save`] 进凭据库（`tts:<模型 ID>`），不回给界面、不进日志和错误消息。

use std::sync::Mutex;
use std::time::{Duration, Instant};

use kotone_core::assistant::TtsEngine;
use kotone_core::speech::SpeechVoice;
use kotone_core::voice_vendor::Region;
use kotone_tts::catalog::{self, Provider};
use kotone_tts::online::{self, CheckError, OnlineKind};
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

use crate::tts_online::KeySource;
use crate::SharedState;

/// 试听念的那句（固定一句，结果按声音落盘，同一个声音再听不花钱）。
pub const TRY_TEXT: &str = "你好呀，以后我就用这个声音跟你说话。";
/// MiniMax 在线声音列表缓存多久。
const VOICES_TTL: Duration = Duration::from_secs(600);

/// 一家在线声音的状态（不含 Key）。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderInfo {
    pub model_id: &'static str,
    /// 声音编号的前缀（`minimax/…`）。
    pub provider: &'static str,
    pub display_name: &'static str,
    pub description: &'static str,
    /// 怎么计费（页面上照写）。
    pub billing: &'static str,
    pub api_key_url: Option<&'static str>,
    pub default_model: &'static str,
    pub default_voice: &'static str,
    /// 能不能调语速。
    pub rate: bool,
    /// Key 分国内 / 国际站。
    pub regions: bool,
    /// 只要一个 Key。
    pub key_only: bool,
    /// 她现在用的就是这一家（含“自动”选中的）。
    pub active: bool,
    pub ready: bool,
    pub reason: Option<String>,
    pub key_source: KeySource,
    pub region: Option<Region>,
}

fn info(app: &AppHandle, provider: &'static Provider, active: TtsEngine) -> ProviderInfo {
    let status = crate::tts_online::status(app, provider.kind);
    let resolved = crate::tts_online::resolved(app, provider.kind).ok();
    ProviderInfo {
        model_id: provider.model_id,
        provider: provider.kind.provider(),
        display_name: provider.display_name,
        description: provider.description,
        billing: provider.billing,
        api_key_url: provider.api_key_url,
        default_model: provider.default_model,
        default_voice: provider.default_voice,
        rate: provider.rate,
        regions: provider.regions,
        key_only: provider.key_only,
        active: active == catalog::engine_of_kind(provider.kind),
        ready: status.ready,
        reason: status.reason,
        key_source: resolved
            .as_ref()
            .map_or(KeySource::None, |r| r.key_source.clone()),
        region: resolved.and_then(|r| r.region),
    }
}

/// 每一家在线声音的状态（服务商表的顺序）。
pub fn providers(app: &AppHandle) -> Vec<ProviderInfo> {
    let active = current_engine(app);
    catalog::PROVIDERS
        .iter()
        .map(|p| info(app, p, active))
        .collect()
}

fn current_engine(app: &AppHandle) -> TtsEngine {
    let config = app
        .state::<SharedState>()
        .settings
        .read()
        .unwrap()
        .assistant
        .clone();
    crate::assistant::effective_engine(app, &config)
}

/// 现在她用的声音：哪一家、是不是“自动”选的、怎么计费（`speech.status` 的 `engine`，ADR 048：页面要写明在用哪家、怎么计费）。
pub fn current(app: &AppHandle) -> Value {
    let config = app
        .state::<SharedState>()
        .settings
        .read()
        .unwrap()
        .assistant
        .clone();
    let engine = crate::assistant::effective_engine(app, &config);
    let auto = config.tts_is_auto();
    match catalog::kind_of(engine) {
        Some(kind) => {
            let p = info(app, catalog::provider(kind), engine);
            json!({
                "modelId": p.model_id, "auto": auto, "displayName": p.display_name, "billing": p.billing,
                "online": true, "ready": p.ready, "reason": p.reason, "keySource": p.key_source,
                "region": p.region, "rate": p.rate,
            })
        }
        None => json!({
            "modelId": catalog::model_of(engine), "auto": auto,
            "displayName": if engine == TtsEngine::Kokoro { "Kokoro 本地中文" } else { "Windows 系统语音" },
            "billing": "不花钱（在本机合成，文字不上传）",
            "online": false, "ready": true, "reason": null, "keySource": KeySource::NotNeeded,
            "region": null, "rate": true,
        }),
    }
}

fn provider_of_id(model_id: &str) -> Result<&'static Provider, String> {
    catalog::provider_by_id(model_id.trim()).ok_or_else(|| format!("不认识的声音服务商 {model_id}"))
}

/// 保存 Key / 先试一次的结果（不含 Key）。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckOutcome {
    /// 试通了。
    pub ok: bool,
    /// Key 存进凭据库了（Key 被拒时不存；网络这类原因没试通时照样存，免得白填）。
    pub saved: bool,
    /// 给玩家看的一句话。
    pub message: String,
    pub region: Option<Region>,
}

static VOICES: Mutex<Option<(String, Instant, Vec<SpeechVoice>)>> = Mutex::new(None);

fn forget_voices() {
    *VOICES.lock().unwrap() = None;
}

/// 这一家的声音：MiniMax 有 Key 时在线拉（含复刻的声音，缓存 10 分钟），拉不到或别的家用内置列表。
pub async fn voices_for(app: &AppHandle, kind: OnlineKind) -> Vec<SpeechVoice> {
    let builtin = catalog::online_voices(kind);
    if kind != OnlineKind::Minimax {
        return builtin;
    }
    let Ok(resolved) = crate::tts_online::resolved(app, kind) else {
        return builtin;
    };
    if resolved.config.api_key.trim().is_empty() {
        return builtin;
    }
    // 缓存按地址 + Key 从哪来区分（不含 Key 本身）；保存、清掉 Key 时清空
    let cache_key = format!("{}|{:?}", resolved.config.base_url, resolved.key_source);
    if let Some((key, at, voices)) = VOICES.lock().unwrap().as_ref() {
        if *key == cache_key && at.elapsed() < VOICES_TTL {
            return voices.clone();
        }
    }
    let config = resolved.config.clone();
    let fetched = tokio::task::spawn_blocking(move || {
        online::minimax_voices(&config.base_url, &config.api_key)
    })
    .await;
    match fetched {
        Ok(Ok(voices)) if !voices.is_empty() => {
            *VOICES.lock().unwrap() = Some((cache_key, Instant::now(), voices.clone()));
            voices
        }
        Ok(Err(error)) => {
            kotone_core::log::log(&format!(
                "speech: MiniMax 声音列表没拉到：{}",
                error.message()
            ));
            builtin
        }
        _ => builtin,
    }
}

/// 保存这一家的 Key：先试一次（MiniMax、百炼顺便判断国内 / 国际站，记进 `modelConfigs[…].region`），
/// Key 被拒就不存；试通了或者只是网络没通，就存进凭据库。
#[tauri::command]
pub async fn speech_key_save(
    app: AppHandle,
    model_id: String,
    key: String,
) -> Result<CheckOutcome, String> {
    let provider = provider_of_id(&model_id)?;
    let name = kotone_core::model_secrets::name_of(provider.model_id).ok_or("这一家不用填 Key")?;
    let key = key.trim().to_string();
    if key.is_empty() {
        return Err("Key 是空的".into());
    }
    if key.len() > 4096 || key.chars().any(char::is_control) {
        return Err("Key 里有不该有的字符，请重新复制".into());
    }
    let kind = provider.kind;
    let checked = {
        let key = key.clone();
        let app = app.clone();
        tauri::async_runtime::spawn_blocking(move || -> Result<Option<Region>, CheckError> {
            if provider.regions {
                return online::detect_region(kind, &key, &online::official_origins(kind))
                    .map(Some);
            }
            // 用这一家现在的地址等设置，换上新 Key 试
            let mut config = match crate::tts_online::resolved(&app, kind) {
                Ok(resolved) => resolved.config,
                Err(reason) => return Err(CheckError::Other(reason)),
            };
            if matches!(kind, OnlineKind::Deepseek | OnlineKind::Siliconflow) {
                // 自己填的 Key 是官方站的，不是哪条连接（代理）的
                config.base_url = online::default_origin(kind, Region::Cn).into();
            }
            config.api_key = key;
            online::check(kind, &config).map(|()| None)
        })
        .await
        .map_err(|_| "试 Key 的时候出错了".to_string())?
    };
    let state = app.state::<SharedState>();
    let (ok, message, region) = match checked {
        Ok(region) => (true, "可以用了".to_string(), region),
        Err(CheckError::Key(message)) => {
            return Ok(CheckOutcome {
                ok: false,
                saved: false,
                message,
                region: None,
            })
        }
        Err(CheckError::Other(message)) => (
            false,
            format!("Key 存好了，可是这次没试通：{message}"),
            None,
        ),
    };
    state.secrets.set(&name, &key)?;
    kotone_core::log::log(&format!("speech: 存好了 {name} 的 Key"));
    if let Some(region) = region {
        let mut config = state
            .settings
            .read()
            .unwrap()
            .model_configs
            .get(provider.model_id)
            .cloned()
            .filter(Value::is_object)
            .unwrap_or_else(|| json!({}));
        config["region"] = json!(region.as_str());
        crate::settings_cmd::update_settings_from_backend(
            &app,
            json!({"modelConfigs": {provider.model_id: config}}),
        )?;
    }
    forget_voices();
    Ok(CheckOutcome {
        ok,
        saved: true,
        message,
        region,
    })
}

/// 清掉声音设置里为这一家填的 Key（同一家 API 连接的 Key 不动；之后照样会用它）。
#[tauri::command]
pub fn speech_key_clear(app: AppHandle, model_id: String) -> Result<(), String> {
    let provider = provider_of_id(&model_id)?;
    let name = kotone_core::model_secrets::name_of(provider.model_id).ok_or("这一家不用填 Key")?;
    app.state::<SharedState>().secrets.delete(&name)?;
    forget_voices();
    Ok(())
}

/// 用现在的设置试一下这一家能不能用（不花钱的接口；火山和自定义接口合成一个字）。
pub async fn check(app: &AppHandle, model_id: &str) -> Result<CheckOutcome, String> {
    let provider = provider_of_id(model_id)?;
    let kind = provider.kind;
    let app = app.clone();
    let outcome = tauri::async_runtime::spawn_blocking(move || {
        let resolved = crate::tts_online::resolved(&app, kind).map_err(CheckError::Other)?;
        online::check(kind, &resolved.config).map(|()| resolved.region)
    })
    .await
    .map_err(|_| "试的时候出错了".to_string())?;
    Ok(match outcome {
        Ok(region) => CheckOutcome {
            ok: true,
            saved: false,
            message: "可以用了".into(),
            region,
        },
        Err(error) => CheckOutcome {
            ok: false,
            saved: false,
            message: error.message().to_string(),
            region: None,
        },
    })
}

#[tauri::command]
pub async fn speech_check(app: AppHandle, model_id: String) -> Result<CheckOutcome, String> {
    check(&app, &model_id).await
}

#[tauri::command]
pub fn speech_providers(app: AppHandle) -> Vec<ProviderInfo> {
    providers(&app)
}

/// 这一家的声音（不管现在选的是哪家）：Kokoro、系统语音和在线的几家。
#[tauri::command]
pub async fn speech_voices(app: AppHandle, model_id: String) -> Result<Vec<SpeechVoice>, String> {
    match model_id.trim() {
        catalog::KOKORO_MODEL_ID => Ok(kotone_tts::kokoro_voices()),
        catalog::SYSTEM_MODEL_ID => Err("系统语音的声音用 list_speech_voices 读".into()),
        id => Ok(voices_for(&app, provider_of_id(id)?.kind).await),
    }
}

/// 声音编号（`提供方/声音`）→ 引擎。
fn engine_of_voice(voice_id: &str) -> Option<TtsEngine> {
    let (prefix, rest) = voice_id.trim().split_once('/')?;
    if rest.trim().is_empty() {
        return None;
    }
    match prefix {
        kotone_tts::voice::SYSTEM => Some(TtsEngine::System),
        kotone_tts::voice::KOKORO => Some(TtsEngine::Kokoro),
        prefix => OnlineKind::ALL
            .into_iter()
            .find(|kind| kind.provider() == prefix)
            .map(catalog::engine_of_kind),
    }
}

/// 试听一个声音（`提供方/声音`）：用它念一句固定的话。在线的结果按声音落盘，同一个声音再听不花钱。
#[tauri::command]
pub async fn speech_try_voice(app: AppHandle, voice_id: String) -> Result<(), String> {
    let engine = engine_of_voice(&voice_id).ok_or("声音编号要写成「提供方/声音」")?;
    kotone_tts::cache::disk::also(TRY_TEXT);
    crate::assistant::try_voice(&app, engine, voice_id.trim(), TRY_TEXT).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voice_ids_pick_their_engine() {
        assert_eq!(
            engine_of_voice("minimax/female-shaonv"),
            Some(TtsEngine::Minimax)
        );
        assert_eq!(
            engine_of_voice("siliconflow/FunAudioLLM/CosyVoice2-0.5B:claire"),
            Some(TtsEngine::Siliconflow)
        );
        assert_eq!(
            engine_of_voice("dashscope/Cherry"),
            Some(TtsEngine::Dashscope)
        );
        assert_eq!(engine_of_voice("kokoro/zf_001"), Some(TtsEngine::Kokoro));
        assert_eq!(engine_of_voice("system/abc"), Some(TtsEngine::System));
        assert_eq!(engine_of_voice("female-shaonv"), None);
        assert_eq!(engine_of_voice("edge/xiaoxiao"), None);
        assert_eq!(engine_of_voice("minimax/"), None);
    }
}
