//! 火山引擎一句话识别（`wss://openspeech.bytedance.com/api/v2/asr`）。
//!
//! 豆包语音产品线见 https://www.volcengine.com/docs/6561/ 。当前只接一句话识别：
//! 松手后把整段 PCM 按 5 秒一块上传。流式 ASR 是另一个产品，不走这个引擎。

use std::io::{Read, Write};
use std::time::Instant;

use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use tokio::sync::mpsc;
use tungstenite::client::IntoClientRequest;
use tungstenite::Message;

use kotone_core::stt::{
    EngineCapabilities, SessionConfig, SttEngine, SttEvent, SttSession, Transcript,
};

use crate::model::VOLCANO_ASR_STT_ID;
use crate::pcm::pcm_f32_to_s16le;

const VOLCANO_WS_URL: &str = "wss://openspeech.bytedance.com/api/v2/asr";
const VOLCANO_CLUSTER: &str = "volcengine_input_common";
const VOLCANO_WORKFLOW: &str = "audio_in,resample,partition,vad,fe,decode";

const HEADER_FULL_CLIENT: [u8; 4] = [0x11, 0x10, 0x11, 0x00];
const HEADER_AUDIO_ONLY: [u8; 4] = [0x11, 0x20, 0x11, 0x00];
const HEADER_LAST_AUDIO: [u8; 4] = [0x11, 0x22, 0x11, 0x00];

const SERVER_FULL_RESPONSE: u8 = 9;
const SERVER_ERROR_RESPONSE: u8 = 15;
const COMPRESSION_GZIP: u8 = 1;

/// 音频分帧大小：5 秒 @ 16kHz 16bit = 160000 字节。
const AUDIO_SEG_BYTES: usize = 160_000;

pub struct VolcanoAsrEngine;

impl VolcanoAsrEngine {
    fn option(cfg: &SessionConfig, key: &str) -> String {
        cfg.options
            .get(key)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string()
    }

    fn keys_complete(cfg: &serde_json::Map<String, serde_json::Value>) -> bool {
        let has = |key: &str| {
            cfg.get(key)
                .and_then(|v| v.as_str())
                .is_some_and(|s| !s.trim().is_empty())
        };
        has("appId") && has("accessToken")
    }
}

impl SttEngine for VolcanoAsrEngine {
    fn id(&self) -> &'static str {
        VOLCANO_ASR_STT_ID
    }

    fn display_name(&self) -> &str {
        "火山引擎一句话识别"
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
        kotone_core::settings::load()
            .model_configs
            .get(VOLCANO_ASR_STT_ID)
            .and_then(|cfg| cfg.as_object())
            .is_some_and(Self::keys_complete)
    }

    fn start_session(
        &self,
        cfg: &SessionConfig,
        events: mpsc::UnboundedSender<SttEvent>,
    ) -> Result<Box<dyn SttSession>, String> {
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
        let pcm16 = pcm_f32_to_s16le(&self.pcm);
        let text =
            volcano_transcribe_ws(&self.app_id, &self.access_token, VOLCANO_CLUSTER, &pcm16)?;
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

fn volcano_transcribe_ws(
    app_id: &str,
    access_token: &str,
    cluster: &str,
    pcm16: &[u8],
) -> Result<String, String> {
    let mut request = VOLCANO_WS_URL
        .into_client_request()
        .map_err(|e| format!("火山引擎 WebSocket 地址无效：{e}"))?;
    request.headers_mut().insert(
        "Authorization",
        format!("Bearer;{access_token}")
            .parse()
            .map_err(|e| format!("无法构造认证头：{e}"))?,
    );
    let (mut socket, _) =
        tungstenite::connect(request).map_err(|e| format!("火山引擎 WebSocket 连接失败：{e}"))?;

    let handshake = serde_json::json!({
        "app": {"appid": app_id, "cluster": cluster, "token": access_token},
        "user": {"uid": app_id},
        "request": {"reqid": new_request_id(), "nbest": 1, "workflow": VOLCANO_WORKFLOW,
                    "result_type": "full", "sequence": 1},
        "audio": {"format": "raw", "codec": "raw"},
    });
    let handshake_bytes = serde_json::to_vec(&handshake).map_err(|e| e.to_string())?;
    socket
        .send(Message::Binary(build_message(
            &HEADER_FULL_CLIENT,
            &gzip_compress(&handshake_bytes),
        )))
        .map_err(|e| format!("火山引擎握手发送失败：{e}"))?;
    let mut text = String::new();
    let ack = socket
        .read()
        .map_err(|e| format!("火山引擎握手响应失败：{e}"))?;
    if let Message::Binary(data) = ack {
        let (msg_type, payload) = parse_server_frame(&data)?;
        match msg_type {
            SERVER_ERROR_RESPONSE => {
                return Err(format!(
                    "火山引擎握手被拒绝：{}",
                    String::from_utf8_lossy(&payload)
                ));
            }
            SERVER_FULL_RESPONSE => {
                text = extract_text(&payload);
            }
            _ => {}
        }
    }

    let mut sent = 0usize;
    while sent < pcm16.len() {
        let remaining = pcm16.len() - sent;
        let last = remaining <= AUDIO_SEG_BYTES;
        let take = remaining.min(AUDIO_SEG_BYTES);
        let header = if last {
            &HEADER_LAST_AUDIO
        } else {
            &HEADER_AUDIO_ONLY
        };
        socket
            .send(Message::Binary(build_message(
                header,
                &gzip_compress(&pcm16[sent..sent + take]),
            )))
            .map_err(|e| format!("火山引擎音频发送失败：{e}"))?;
        sent += take;

        let resp = socket
            .read()
            .map_err(|e| format!("火山引擎识别响应失败：{e}"))?;
        if let Message::Binary(data) = resp {
            let (msg_type, payload) = parse_server_frame(&data)?;
            match msg_type {
                SERVER_FULL_RESPONSE => {
                    text = extract_text(&payload);
                }
                SERVER_ERROR_RESPONSE => {
                    return Err(format!(
                        "火山引擎识别失败：{}",
                        String::from_utf8_lossy(&payload)
                    ));
                }
                _ => {}
            }
        }
    }
    socket.close(None).ok();
    if text.trim().is_empty() {
        return Err("火山引擎语音识别没有返回文本".into());
    }
    Ok(text.trim().to_string())
}

fn build_message(header: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut msg = Vec::with_capacity(8 + payload.len());
    msg.extend_from_slice(header);
    msg.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    msg.extend_from_slice(payload);
    msg
}

fn parse_server_frame(data: &[u8]) -> Result<(u8, Vec<u8>), String> {
    if data.len() < 8 {
        return Err("火山引擎响应帧过短".into());
    }
    let header_size = (data[0] & 0x0f) as usize;
    let msg_type = data[1] >> 4;
    let compression = data[2] & 0x0f;
    let body = &data[header_size * 4..];
    if body.len() < 4 {
        return Err("火山引擎响应帧缺少长度".into());
    }
    let size = u32::from_be_bytes([body[0], body[1], body[2], body[3]]) as usize;
    let payload = &body[4..];
    let payload = if payload.len() >= size {
        &payload[..size]
    } else {
        payload
    };
    if compression == COMPRESSION_GZIP {
        Ok((msg_type, gzip_decompress(payload)?))
    } else {
        Ok((msg_type, payload.to_vec()))
    }
}

fn extract_text(payload: &[u8]) -> String {
    let Ok(json) = serde_json::from_slice::<serde_json::Value>(payload) else {
        return String::new();
    };
    if let Some(items) = json.get("result").and_then(|r| r.as_array()) {
        let text: String = items
            .iter()
            .filter_map(|item| item.get("text").and_then(|t| t.as_str()))
            .collect();
        if !text.trim().is_empty() {
            return text;
        }
    }
    json.get("result")
        .and_then(|r| r.get("text"))
        .or_else(|| json.get("text"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

fn gzip_compress(data: &[u8]) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    if encoder.write_all(data).is_err() {
        return Vec::new();
    }
    encoder.finish().unwrap_or_default()
}

fn gzip_decompress(data: &[u8]) -> Result<Vec<u8>, String> {
    let mut decoder = GzDecoder::new(data);
    let mut out = Vec::new();
    decoder
        .read_to_end(&mut out)
        .map_err(|e| format!("火山引擎响应 gzip 解压失败：{e}"))?;
    Ok(out)
}

fn new_request_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("kotone-{nanos}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gzip_roundtrip() {
        let data = b"hello kotone".repeat(10);
        let compressed = gzip_compress(&data);
        assert!(!compressed.is_empty());
        assert_eq!(gzip_decompress(&compressed).unwrap(), data);
    }

    #[test]
    fn build_message_prepends_header_and_size() {
        let msg = build_message(&HEADER_FULL_CLIENT, b"abc");
        assert_eq!(&msg[0..4], &HEADER_FULL_CLIENT);
        assert_eq!(u32::from_be_bytes([msg[4], msg[5], msg[6], msg[7]]), 3);
        assert_eq!(&msg[8..], b"abc");
    }

    #[test]
    fn keys_complete_checks_volcano_fields() {
        let cfg: serde_json::Map<_, _> = serde_json::json!({
            "appId": "app", "accessToken": "tok"
        })
        .as_object()
        .unwrap()
        .clone();
        assert!(VolcanoAsrEngine::keys_complete(&cfg));
        let incomplete: serde_json::Map<_, _> = serde_json::json!({"appId": "app"})
            .as_object()
            .unwrap()
            .clone();
        assert!(!VolcanoAsrEngine::keys_complete(&incomplete));
    }
}
