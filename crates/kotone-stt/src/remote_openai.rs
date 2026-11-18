//! 远程整段识别：
//! - OpenAI 兼容：把整段音频 POST 到连接里的 /audio/transcriptions；
//! - DeepSeek：POST 到站点根下的 `/v1/tools/asr`（只有 `file` 一个字段，自动判断语种；
//!   16 位 PCM WAV、最长 120 秒、最大 8 MiB，见 api-docs.deepseek.com/guides/asr）。
//!
//! 输出是一次性 final，走离线会话循环。

use std::time::Instant;

use tokio::sync::mpsc;

use kotone_core::stt::{
    EngineCapabilities, SessionConfig, SttEngine, SttEvent, SttSession, Transcript,
};

use crate::model::{DEEPSEEK_ASR_STT_ID, REMOTE_OPENAI_ENGINE_ID, REMOTE_OPENAI_STT_ID};

/// DeepSeek 一次最多识别 120 秒；更长的只送最后 118 秒（呼叫琴音说的话不会这么长）。
const DEEPSEEK_MAX_SAMPLES: usize = 16_000 * 118;

pub struct RemoteOpenaiEngine;

/// DeepSeek 语音识别：连接（地址 + 密钥）由宿主按琴音的 DeepSeek 连接填进 options。
pub struct RemoteDeepseekEngine;

impl SttEngine for RemoteDeepseekEngine {
    fn id(&self) -> &'static str {
        DEEPSEEK_ASR_STT_ID
    }

    fn display_name(&self) -> &str {
        "DeepSeek 语音识别（在线）"
    }

    fn capabilities(&self) -> EngineCapabilities {
        EngineCapabilities {
            streaming: false,
            hotwords: false,
            gpu: false,
            offline: false,
            languages: vec!["auto".into()],
        }
    }

    fn is_ready(&self) -> bool {
        kotone_core::settings::load()
            .connections
            .iter()
            .any(|c| c.provider == "deepseek")
    }

    fn start_session(
        &self,
        cfg: &SessionConfig,
        events: mpsc::UnboundedSender<SttEvent>,
    ) -> Result<Box<dyn SttSession>, String> {
        let text = |key: &str| {
            cfg.options
                .get(key)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string()
        };
        let (base_url, api_key) = (text("baseUrl"), text("apiKey"));
        if api_key.is_empty() {
            return Err(
                "DeepSeek 语音识别需要一条已保存密钥的 DeepSeek 连接（「设置 → API 连接」）".into(),
            );
        }
        Ok(Box::new(RemoteSession {
            pcm: Vec::new(),
            events,
            cancelled: false,
            base_url,
            api_key,
            model: String::new(),
            language: String::new(),
            deepseek: true,
        }))
    }
}

impl SttEngine for RemoteOpenaiEngine {
    fn id(&self) -> &'static str {
        REMOTE_OPENAI_ENGINE_ID
    }

    fn display_name(&self) -> &str {
        "在线识别（OpenAI 兼容）"
    }

    fn capabilities(&self) -> EngineCapabilities {
        EngineCapabilities {
            streaming: false,
            hotwords: false,
            gpu: false,
            offline: false,
            languages: vec!["auto".into()],
        }
    }

    fn is_ready(&self) -> bool {
        let settings = kotone_core::settings::load();
        settings
            .model_configs
            .get(REMOTE_OPENAI_STT_ID)
            .or_else(|| settings.model_configs.get(&settings.active_model_id))
            .and_then(|cfg| cfg.get("connectionId"))
            .and_then(|v| v.as_str())
            .is_some_and(|id| !id.trim().is_empty())
    }

    fn start_session(
        &self,
        cfg: &SessionConfig,
        events: mpsc::UnboundedSender<SttEvent>,
    ) -> Result<Box<dyn SttSession>, String> {
        let base_url = cfg
            .options
            .get("baseUrl")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let api_key = cfg
            .options
            .get("apiKey")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let model = cfg
            .options
            .get("remoteModel")
            .and_then(|v| v.as_str())
            .unwrap_or("whisper-1")
            .trim()
            .to_string();
        if base_url.is_empty() || api_key.is_empty() {
            return Err("在线识别未就绪：请在模型配置里选择一条已保存密钥的 API 连接".into());
        }
        Ok(Box::new(RemoteSession {
            pcm: Vec::new(),
            events,
            cancelled: false,
            base_url,
            api_key,
            model,
            language: cfg.language.clone(),
            deepseek: false,
        }))
    }
}

struct RemoteSession {
    pcm: Vec<f32>,
    events: mpsc::UnboundedSender<SttEvent>,
    cancelled: bool,
    base_url: String,
    api_key: String,
    model: String,
    language: String,
    /// DeepSeek 的 `/v1/tools/asr`（否则 OpenAI 兼容的 /audio/transcriptions）。
    deepseek: bool,
}

impl SttSession for RemoteSession {
    fn push_audio(&mut self, pcm: &[f32]) -> Result<(), String> {
        if self.cancelled {
            return Ok(());
        }
        self.pcm.extend_from_slice(pcm);
        Ok(())
    }

    fn finalize(self: Box<Self>) -> Result<Transcript, String> {
        if self.cancelled {
            return Ok(Transcript {
                text: String::new(),
                latency_ms: 0,
            });
        }
        let started = Instant::now();
        let text = if self.deepseek {
            let start = self.pcm.len().saturating_sub(DEEPSEEK_MAX_SAMPLES);
            let text = transcribe_deepseek(
                &self.base_url,
                &self.api_key,
                &encode_wav_s16le(&self.pcm[start..]),
            )?;
            // 用量账本：DeepSeek 按音频时长计费（16 kHz）。
            kotone_core::usage::record_asr(
                "deepseek",
                "deepseek-asr",
                (self.pcm.len() - start) as u64 / 16,
            );
            text
        } else {
            let text = transcribe(
                &self.base_url,
                &self.api_key,
                &self.model,
                &self.language,
                &encode_wav_s16le(&self.pcm),
            )?;
            kotone_core::usage::record_asr(
                "openai-compatible",
                &self.model,
                self.pcm.len() as u64 / 16,
            );
            text
        };
        let latency_ms = started.elapsed().as_millis() as u32;
        let _ = self.events.send(SttEvent::Final {
            text: text.clone(),
            latency_ms,
        });
        Ok(Transcript { text, latency_ms })
    }

    fn cancel(&mut self) {
        self.cancelled = true;
    }
}

fn transcribe(
    base_url: &str,
    api_key: &str,
    model: &str,
    language: &str,
    wav: &[u8],
) -> Result<String, String> {
    let endpoint = format!("{}/audio/transcriptions", base_url.trim_end_matches('/'));
    let boundary = "----kotoneSpeechBoundary";
    let mut body = Vec::new();
    fn push_field(body: &mut Vec<u8>, boundary: &str, name: &str, value: &str) {
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        body.extend_from_slice(
            format!("Content-Disposition: form-data; name=\"{name}\"\r\n\r\n").as_bytes(),
        );
        body.extend_from_slice(value.as_bytes());
        body.extend_from_slice(b"\r\n");
    }
    push_field(&mut body, boundary, "model", model);
    if !language.is_empty() && language != "auto" {
        push_field(&mut body, boundary, "language", language);
    }
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(
        b"Content-Disposition: form-data; name=\"file\"; filename=\"speech.wav\"\r\n",
    );
    body.extend_from_slice(b"Content-Type: audio/wav\r\n\r\n");
    body.extend_from_slice(wav);
    body.extend_from_slice(b"\r\n");
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(45))
        .build()
        .map_err(|e| format!("无法创建识别客户端：{e}"))?;
    let response = client
        .post(endpoint)
        .bearer_auth(api_key)
        .header(
            "content-type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(body)
        .send()
        .map_err(|e| format!("在线识别请求失败：{e}"))?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().unwrap_or_default();
        return Err(format!("在线识别失败（HTTP {status}）：{body}"));
    }
    let body: serde_json::Value = response
        .json()
        .map_err(|e| format!("在线识别返回无法解析：{e}"))?;
    body.get("text")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "在线识别没有返回文本".into())
}

/// DeepSeek 连接的地址常写成 `https://api.deepseek.com` 或 `…/v1`：识别接口在站点根下。
pub fn deepseek_asr_endpoint(base_url: &str) -> String {
    let base = base_url.trim().trim_end_matches('/');
    let base = if base.is_empty() {
        "https://api.deepseek.com"
    } else {
        base
    };
    let origin = base
        .strip_suffix("/v1")
        .unwrap_or(base)
        .trim_end_matches('/');
    format!("{origin}/v1/tools/asr")
}

fn transcribe_deepseek(base_url: &str, api_key: &str, wav: &[u8]) -> Result<String, String> {
    let boundary = "----kotoneSpeechBoundary";
    let mut body = Vec::new();
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(
        b"Content-Disposition: form-data; name=\"file\"; filename=\"speech.wav\"\r\n",
    );
    body.extend_from_slice(b"Content-Type: audio/wav\r\n\r\n");
    body.extend_from_slice(wav);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(45))
        .build()
        .map_err(|e| format!("无法创建识别客户端：{e}"))?;
    let response = client
        .post(deepseek_asr_endpoint(base_url))
        .bearer_auth(api_key)
        .header(
            "content-type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(body)
        .send()
        .map_err(|e| format!("DeepSeek 语音识别请求失败：{e}"))?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().unwrap_or_default();
        let hint = match status.as_u16() {
            401 => "密钥不对",
            413 => "这段话太长了",
            429 => "同时识别的太多了，稍后再试",
            503 | 504 => "服务忙或超时，稍后再试",
            _ => "",
        };
        return Err(format!(
            "DeepSeek 语音识别失败（HTTP {status}{}）：{}",
            if hint.is_empty() {
                String::new()
            } else {
                format!("，{hint}")
            },
            body.chars().take(200).collect::<String>()
        ));
    }
    let body: serde_json::Value = response
        .json()
        .map_err(|e| format!("DeepSeek 语音识别返回无法解析：{e}"))?;
    Ok(body
        .get("text")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .unwrap_or_default()
        .to_string())
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
    fn deepseek_endpoint_is_under_the_site_root() {
        for base in [
            "https://api.deepseek.com",
            "https://api.deepseek.com/v1",
            "https://api.deepseek.com/v1/",
            "",
        ] {
            assert_eq!(
                deepseek_asr_endpoint(base),
                "https://api.deepseek.com/v1/tools/asr",
                "{base}"
            );
        }
        assert_eq!(
            deepseek_asr_endpoint("http://127.0.0.1:18765/v1"),
            "http://127.0.0.1:18765/v1/tools/asr"
        );
    }
}
