//! 远程 OpenAI 兼容识别：把整段音频 POST 到连接里的 /audio/transcriptions。
//! 输出是一次性 final，走离线会话循环。

use std::time::Instant;

use tokio::sync::mpsc;

use kotone_core::stt::{
    EngineCapabilities, SessionConfig, SttEngine, SttEvent, SttSession, Transcript,
};

use crate::model::{REMOTE_OPENAI_ENGINE_ID, REMOTE_OPENAI_STT_ID};

pub struct RemoteOpenaiEngine;

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
        let wav = encode_wav_s16le(&self.pcm);
        let text = transcribe(
            &self.base_url,
            &self.api_key,
            &self.model,
            &self.language,
            &wav,
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
    }
}

fn transcribe(
    base_url: &str,
    api_key: &str,
    model: &str,
    language: &str,
    wav: &[u8],
) -> Result<String, String> {
    let endpoint = format!(
        "{}/audio/transcriptions",
        base_url.trim_end_matches('/')
    );
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

}
