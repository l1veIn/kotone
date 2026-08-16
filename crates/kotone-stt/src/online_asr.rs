//! 在线 ASR 平台引擎（火山引擎 + 科大讯飞）。
//!
//! 模型清单与配置（configSchema：App ID / Access Token / APIKey / APISecret）
//! 在 `model.rs`；本文件实现各平台的音频转写适配器。
//!
//! - 火山引擎：一句话识别（极速版，HTTP multipart）。
//! - 科大讯飞：语音听写（WebSocket），当前占位待接入。

use std::time::Instant;

use tokio::sync::mpsc;

use kotone_core::stt::{EngineCapabilities, SessionConfig, SttEngine, SttEvent, SttSession, Transcript};

use crate::model::{recipe_of, ModelRecipe, ONLINE_ASR_ENGINE_ID};

const VOLCANO_ENDPOINT: &str = "https://openspeech.bytedance.com/api/v3/auc/bigmodel/recognize/flash";
const VOLCANO_RESOURCE_ID: &str = "volc.bigasr.auc_turbo";

pub struct OnlineAsrEngine;

impl OnlineAsrEngine {
    /// 活动在线 ASR 模型的必填配置是否齐备。
    fn required_complete(
        cfg: &serde_json::Map<String, serde_json::Value>,
        recipe: ModelRecipe,
    ) -> bool {
        let has = |key: &str| {
            cfg.get(key)
                .and_then(|v| v.as_str())
                .is_some_and(|s| !s.trim().is_empty())
        };
        match recipe {
            ModelRecipe::VolcanoAsr => has("appId") && has("accessToken"),
            ModelRecipe::IflytekAsr => has("appId") && has("apiKey") && has("apiSecret"),
            _ => false,
        }
    }

    fn option<'a>(cfg: &'a SessionConfig, key: &str) -> String {
        cfg.options
            .get(key)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string()
    }
}

impl SttEngine for OnlineAsrEngine {
    fn id(&self) -> &'static str {
        ONLINE_ASR_ENGINE_ID
    }

    fn display_name(&self) -> &str {
        "在线语音识别"
    }

    fn capabilities(&self) -> EngineCapabilities {
        EngineCapabilities {
            streaming: false,
            hotwords: false,
            gpu: false,
            offline: false,
            languages: vec!["zh".into()],
        }
    }

    fn is_ready(&self) -> bool {
        let settings = kotone_core::settings::load();
        let model_id = settings.active_model_id.trim();
        if model_id.is_empty() {
            return false;
        }
        let Some(recipe) = recipe_of(model_id) else {
            return false;
        };
        if !matches!(recipe, ModelRecipe::VolcanoAsr | ModelRecipe::IflytekAsr) {
            return false;
        }
        settings
            .model_configs
            .get(model_id)
            .and_then(|cfg| cfg.as_object())
            .is_some_and(|cfg| Self::required_complete(cfg, recipe))
    }

    fn start_session(
        &self,
        cfg: &SessionConfig,
        events: mpsc::UnboundedSender<SttEvent>,
    ) -> Result<Box<dyn SttSession>, String> {
        let model_id = Self::option(cfg, "model");
        let recipe = recipe_of(&model_id).ok_or_else(|| "在线 ASR 模型不存在".to_string())?;
        match recipe {
            ModelRecipe::VolcanoAsr => {
                let app_id = Self::option(cfg, "appId");
                let access_token = Self::option(cfg, "accessToken");
                if app_id.is_empty() || access_token.is_empty() {
                    return Err("火山引擎语音识别未就绪：请填写 App ID 与 Access Token".into());
                }
                Ok(Box::new(VolcanoSession {
                    pcm: Vec::new(),
                    events,
                    cancelled: false,
                    app_id,
                    access_token,
                }))
            }
            ModelRecipe::IflytekAsr => {
                Err("科大讯飞语音听写适配器尚未接入，敬请期待".into())
            }
            _ => Err(format!("模型 {model_id} 不是在线 ASR 模型")),
        }
    }
}

struct VolcanoSession {
    pcm: Vec<f32>,
    events: mpsc::UnboundedSender<SttEvent>,
    cancelled: bool,
    app_id: String,
    access_token: String,
}

impl SttSession for VolcanoSession {
    fn push_audio(&mut self, pcm: &[f32]) -> Result<(), String> {
        if self.cancelled {
            return Ok(());
        }
        self.pcm.extend_from_slice(pcm);
        Ok(())
    }

    fn finalize(self: Box<Self>) -> Result<Transcript, String> {
        if self.cancelled || self.pcm.is_empty() {
            return Ok(Transcript {
                text: String::new(),
                latency_ms: 0,
            });
        }
        let started = Instant::now();
        let wav = encode_wav_s16le(&self.pcm);
        let text = volcano_transcribe(&self.app_id, &self.access_token, &wav)?;
        let latency_ms = started.elapsed().as_millis() as u32;
        let _ = self.events.send(SttEvent::Final {
            text: text.clone(),
            latency_ms,
        });
        Ok(Transcript { text, latency_ms })
    }

    fn cancel(&mut self) {
        self.cancelled = true;
        self.pcm.clear();
    }
}

/// 火山引擎一句话识别（极速版 flash）：JSON body + base64 音频。
fn volcano_transcribe(app_id: &str, access_token: &str, wav: &[u8]) -> Result<String, String> {
    use base64::Engine;
    let audio_b64 = base64::engine::general_purpose::STANDARD.encode(wav);
    let payload = serde_json::json!({
        "user": {"uid": app_id},
        "audio": {"format": "wav", "data": audio_b64},
        "request": {"model_name": "bigmodel", "show_utterances": true, "enable_itn": true},
    });

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(45))
        .build()
        .map_err(|e| format!("无法创建识别客户端：{e}"))?;
    let response = client
        .post(VOLCANO_ENDPOINT)
        .header("X-Api-App-Key", app_id)
        .header("X-Api-Access-Key", access_token)
        .header("X-Api-Resource-Id", VOLCANO_RESOURCE_ID)
        .header("X-Api-Request-Id", new_request_id())
        .header("X-Api-Sequence", "-1")
        .json(&payload)
        .send()
        .map_err(|e| format!("火山引擎语音识别请求失败：{e}"))?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().unwrap_or_default();
        return Err(format!("火山引擎语音识别失败（HTTP {status}）：{body}"));
    }
    // 业务状态码在响应 Header（X-Api-Status-Code）；20000003 = 静音音频。
    let status_code = response
        .headers()
        .get("X-Api-Status-Code")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if status_code == "20000003" {
        return Ok(String::new());
    }
    if !status_code.is_empty() && status_code != "20000000" {
        let status_owned = status_code.to_string();
        let msg = response
            .headers()
            .get("X-Api-Message")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        let body = response.text().unwrap_or_default();
        kotone_core::log::log(&format!(
            "volcano asr error: status={status_owned} msg={msg} endpoint={VOLCANO_ENDPOINT} resource={VOLCANO_RESOURCE_ID} app={app_id} body={body}"
        ));
        return Err(format!("火山引擎语音识别失败：[{status_owned}] {msg}"));
    }
    let json: serde_json::Value = response
        .json()
        .map_err(|e| format!("火山引擎返回无法解析：{e}"))?;
    let result = json.get("result");
    let text = result
        .and_then(|r| r.get("utterances"))
        .and_then(|u| u.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            result
                .and_then(|r| r.get("text"))
                .and_then(|t| t.as_str())
                .map(str::to_string)
        })
        .or_else(|| json.get("text").and_then(|t| t.as_str()).map(str::to_string));
    text.as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "火山引擎语音识别没有返回文本".into())
}

fn new_request_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("kotone-{nanos}")
}

fn encode_wav_s16le(pcm: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(44 + pcm.len() * 2);
    let data_len = (pcm.len() * 2) as u32;
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&16000u32.to_le_bytes());
    out.extend_from_slice(&32000u32.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for sample in pcm {
        let clipped = sample.clamp(-1.0, 1.0);
        let int = (clipped * i16::MAX as f32) as i16;
        out.extend_from_slice(&int.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_header_is_wellformed() {
        let wav = encode_wav_s16le(&[0.0, 0.5, -0.5]);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(wav.len(), 44 + 6);
    }

    #[test]
    fn required_complete_checks_volcano_fields() {
        let cfg: serde_json::Map<_, _> = serde_json::json!({
            "appId": "app", "accessToken": "tok"
        })
        .as_object()
        .unwrap()
        .clone();
        assert!(OnlineAsrEngine::required_complete(&cfg, ModelRecipe::VolcanoAsr));
        let incomplete: serde_json::Map<_, _> = serde_json::json!({"appId": "app"})
            .as_object()
            .unwrap()
            .clone();
        assert!(!OnlineAsrEngine::required_complete(&incomplete, ModelRecipe::VolcanoAsr));
    }
}
