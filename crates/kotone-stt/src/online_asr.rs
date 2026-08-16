//! 在线 ASR 平台引擎（火山引擎 + 科大讯飞）。
//!
//! 模型清单与配置（configSchema：App ID / Access Token / APIKey / APISecret）
//! 在 `model.rs`；本文件实现各平台的音频转写适配器。
//!
//! - 火山引擎：一句话识别（WebSocket 二进制协议，`/api/v2/asr`，gzip 压缩）。
//! - 科大讯飞：语音听写（WebSocket + HMAC-SHA256 鉴权，`/v2/iat`）。

use std::io::{Read, Write};
use std::time::Instant;

use base64::Engine;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use tokio::sync::mpsc;
use tungstenite::client::IntoClientRequest;
use tungstenite::Message;

use kotone_core::stt::{
    EngineCapabilities, SessionConfig, SttEngine, SttEvent, SttSession, Transcript,
};

use crate::model::{recipe_of, ModelRecipe, ONLINE_ASR_ENGINE_ID};

type HmacSha256 = Hmac<Sha256>;

// ---------- 火山引擎一句话识别（WebSocket 二进制协议） ----------

const VOLCANO_WS_URL: &str = "wss://openspeech.bytedance.com/api/v2/asr";
const VOLCANO_CLUSTER: &str = "volcengine_input_common";
const VOLCANO_WORKFLOW: &str = "audio_in,resample,partition,vad,fe,decode";

/// 客户端请求帧头（4 字节：protocol/flags/serialization+compression/reserved）。
const HEADER_FULL_CLIENT: [u8; 4] = [0x11, 0x10, 0x11, 0x00];
const HEADER_AUDIO_ONLY: [u8; 4] = [0x11, 0x20, 0x11, 0x00];
const HEADER_LAST_AUDIO: [u8; 4] = [0x11, 0x22, 0x11, 0x00];

/// 服务端响应 message type（msg[1] >> 4）。
const SERVER_FULL_RESPONSE: u8 = 9;
const SERVER_ERROR_RESPONSE: u8 = 15;
/// msg[2] & 0x0f == 1 表示 payload 为 gzip。
const COMPRESSION_GZIP: u8 = 1;

/// 音频分帧大小：5 秒 @ 16kHz 16bit = 160000 字节。
const AUDIO_SEG_BYTES: usize = 160_000;

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

    fn option(cfg: &SessionConfig, key: &str) -> String {
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
                let app_id = Self::option(cfg, "appId");
                let api_key = Self::option(cfg, "apiKey");
                let api_secret = Self::option(cfg, "apiSecret");
                if app_id.is_empty() || api_key.is_empty() || api_secret.is_empty() {
                    return Err(
                        "科大讯飞语音听写未就绪：请填写 APPID、APIKey 与 APISecret".into(),
                    );
                }
                Ok(Box::new(IflytekSession {
                    pcm: Vec::new(),
                    events,
                    cancelled: false,
                    app_id,
                    api_key,
                    api_secret,
                }))
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
        let pcm16 = pcm_f32_to_s16le(&self.pcm);
        let text = volcano_transcribe_ws(
            &self.app_id,
            &self.access_token,
            VOLCANO_CLUSTER,
            &pcm16,
        )?;
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

/// 火山引擎一句话识别：WebSocket 二进制协议（gzip 压缩）。
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
    let (mut socket, _) = tungstenite::connect(request)
        .map_err(|e| format!("火山引擎 WebSocket 连接失败：{e}"))?;

    // 1. 握手：FullClient 帧 + gzip JSON。
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
    // 读握手响应：可能是 ACK（确认）或 FULL_RESPONSE（直接返回结果）。
    let mut text = String::new();
    let ack = socket.read().map_err(|e| format!("火山引擎握手响应失败：{e}"))?;
    if let Message::Binary(data) = ack {
        let (msg_type, payload) = parse_server_frame(&data)?;
        match msg_type {
            SERVER_ERROR_RESPONSE => {
                return Err(format!("火山引擎握手被拒绝：{}", String::from_utf8_lossy(&payload)));
            }
            SERVER_FULL_RESPONSE => {
                text = extract_text(&payload);
            }
            _ => {} // ACK 或其他
        }
    }

    // 2. 分帧发送音频（raw 16bit PCM）。
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

        // 每帧后读响应，累积结果文本。
        let resp = socket.read().map_err(|e| format!("火山引擎识别响应失败：{e}"))?;
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

/// 构造二进制帧：4 字节 header + 4 字节大端 payload 长度 + gzip payload。
fn build_message(header: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut msg = Vec::with_capacity(8 + payload.len());
    msg.extend_from_slice(header);
    msg.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    msg.extend_from_slice(payload);
    msg
}

/// 解析服务端帧，返回 (message_type, 解压后的 payload)。
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

/// 从服务端 JSON 里提取识别文本（result 为数组，逐项拼接 text）。
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

fn pcm_f32_to_s16le(pcm: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(pcm.len() * 2);
    for sample in pcm {
        let clipped = sample.clamp(-1.0, 1.0);
        let int = (clipped * 32768.0).round().clamp(-32768.0, 32767.0) as i16;
        out.extend_from_slice(&int.to_le_bytes());
    }
    out
}

fn new_request_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("kotone-{nanos}")
}

// ---------- 科大讯飞语音听写（WebSocket + HMAC-SHA256 鉴权） ----------

const IFLYTEK_WS_HOST: &str = "iat-api.xfyun.cn";
const IFLYTEK_WS_PATH: &str = "/v2/iat";
/// 音频分帧大小：1280 字节（40ms @ 16kHz 16bit）。
const IFLYTEK_FRAME_BYTES: usize = 1280;

struct IflytekSession {
    pcm: Vec<f32>,
    events: mpsc::UnboundedSender<SttEvent>,
    cancelled: bool,
    app_id: String,
    api_key: String,
    api_secret: String,
}

impl SttSession for IflytekSession {
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
        let text = iflytek_transcribe_ws(&self.app_id, &self.api_key, &self.api_secret, &pcm16)?;
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

/// 构造讯飞 WebSocket 鉴权（authorization + date），HMAC-SHA256 签名后 base64。
fn iflytek_auth(api_key: &str, api_secret: &str) -> (String, String) {
    let date = chrono::Utc::now()
        .format("%a, %d %b %Y %H:%M:%S GMT")
        .to_string();
    let signature_origin = format!(
        "host: {IFLYTEK_WS_HOST}\ndate: {date}\nGET {IFLYTEK_WS_PATH} HTTP/1.1"
    );
    let mut mac = HmacSha256::new_from_slice(api_secret.as_bytes())
        .expect("HMAC 接受任意长度密钥");
    mac.update(signature_origin.as_bytes());
    let signature =
        base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes());
    let authorization_origin = format!(
        "api_key=\"{api_key}\", algorithm=\"hmac-sha256\", \
         headers=\"host date request-line\", signature=\"{signature}\""
    );
    let authorization =
        base64::engine::general_purpose::STANDARD.encode(authorization_origin.as_bytes());
    (authorization, date)
}

/// 百分号编码（RFC 3986 unreserved 之外的字节都编码），用于鉴权参数进 query。
fn url_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// 讯飞语音听写音频帧：第一帧带 common/business，末帧 status=2。
fn iflytek_audio_frame(app_id: &str, status: u8, audio_b64: &str) -> serde_json::Value {
    let data = serde_json::json!({
        "status": status,
        "format": "audio/L16;rate=16000",
        "encoding": "raw",
        "audio": audio_b64,
    });
    if status == 0 {
        serde_json::json!({
            "common": {"app_id": app_id},
            "business": {"language": "zh_cn", "domain": "iat", "accent": "mandarin", "vad_eos": 10000},
            "data": data,
        })
    } else {
        serde_json::json!({ "data": data })
    }
}

/// 科大讯飞语音听写：WebSocket 文本 JSON，音频 base64 分帧发送。
fn iflytek_transcribe_ws(
    app_id: &str,
    api_key: &str,
    api_secret: &str,
    pcm16: &[u8],
) -> Result<String, String> {
    let (authorization, date) = iflytek_auth(api_key, api_secret);
    // authorization 与 date 含 base64/逗号/空格，必须做百分号编码才能放进 query。
    let url = format!(
        "wss://{IFLYTEK_WS_HOST}{IFLYTEK_WS_PATH}?authorization={}&date={}&host={IFLYTEK_WS_HOST}",
        url_encode(&authorization),
        url_encode(&date),
    );
    let (mut socket, _) = match tungstenite::connect(url) {
        Ok(connected) => connected,
        Err(e) => {
            let detail = match &e {
                tungstenite::Error::Http(resp) => {
                    let body = resp
                        .body()
                        .as_deref()
                        .map(|b| String::from_utf8_lossy(b).to_string())
                        .unwrap_or_default();
                    format!("HTTP {}: {body}", resp.status())
                }
                _ => e.to_string(),
            };
            kotone_core::log::log(&format!("iflytek asr connect error: {detail}"));
            return Err(format!("科大讯飞 WebSocket 连接失败：{detail}"));
        }
    };

    let mut text = String::new();
    let mut sent = 0usize;
    while sent < pcm16.len() {
        let remaining = pcm16.len() - sent;
        let last = remaining <= IFLYTEK_FRAME_BYTES;
        let take = remaining.min(IFLYTEK_FRAME_BYTES);
        let status = if sent == 0 { 0 } else if last { 2 } else { 1 };
        let audio_b64 =
            base64::engine::general_purpose::STANDARD.encode(&pcm16[sent..sent + take]);
        let frame = iflytek_audio_frame(app_id, status, &audio_b64);
        socket
            .send(Message::Text(frame.to_string()))
            .map_err(|e| format!("科大讯飞音频发送失败：{e}"))?;
        sent += take;

        let resp = socket.read().map_err(|e| format!("科大讯飞识别响应失败：{e}"))?;
        if let Message::Text(t) = resp {
            let extracted = iflytek_extract_text(&t);
            if !extracted.is_empty() {
                text = extracted;
            }
        }
    }
    socket.close(None).ok();
    if text.trim().is_empty() {
        return Err("科大讯飞语音听写没有返回文本".into());
    }
    Ok(text.trim().to_string())
}

/// 从讯飞响应 JSON 里提取 data.result.ws[].cw[].w 拼接文本。
fn iflytek_extract_text(payload: &str) -> String {
    let Ok(json) = serde_json::from_str::<serde_json::Value>(payload) else {
        return String::new();
    };
    let mut out = String::new();
    if let Some(ws) = json.pointer("/data/result/ws").and_then(|v| v.as_array()) {
        for item in ws {
            if let Some(cw) = item.get("cw").and_then(|v| v.as_array()) {
                for w in cw {
                    if let Some(t) = w.get("w").and_then(|v| v.as_str()) {
                        out.push_str(t);
                    }
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pcm_conversion_clamps_and_packs_s16le() {
        let bytes = pcm_f32_to_s16le(&[0.0, 1.0, -1.0, 2.0]);
        assert_eq!(bytes.len(), 8);
        assert_eq!(i16::from_le_bytes([bytes[0], bytes[1]]), 0);
        assert_eq!(i16::from_le_bytes([bytes[2], bytes[3]]), i16::MAX);
        assert_eq!(i16::from_le_bytes([bytes[4], bytes[5]]), i16::MIN);
        assert_eq!(i16::from_le_bytes([bytes[6], bytes[7]]), i16::MAX);
    }

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
