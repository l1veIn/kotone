//! 在线语音合成：OpenAI 兼容 `/audio/speech`、火山语音合成 HTTP 接口（`/api/v1/tts`）与
//! DeepSeek 官方语音合成（`/v1/tools/tts`）。
//!
//! - 整段合成（不流式），要求 WAV 返回，解码后交给与 Kokoro 相同的播放与取消路径。
//! - 连接与密钥由调用方在每次合成时解析（[`OnlineResolver`]），密钥只来自凭据库，
//!   不进缓存键、日志或错误消息；错误不回显响应体。
//! - 选中的声音属于其他提供方时直接报错，不换声音。

use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use kotone_core::speech::SpeechRequest;
use serde_json::{json, Value};

use crate::{voice, SynthesizedSpeech};

const MAX_RESPONSE_BYTES: u64 = 20 * 1024 * 1024;
pub const VOLCANO_ENDPOINT: &str = "https://openspeech.bytedance.com/api/v1/tts";
pub const VOLCANO_DEFAULT_CLUSTER: &str = "volcano_tts";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnlineKind {
    OpenAi,
    Volcano,
    Deepseek,
}

impl OnlineKind {
    pub fn provider(self) -> &'static str {
        match self {
            Self::OpenAi => voice::OPENAI,
            Self::Volcano => voice::VOLCANO,
            Self::Deepseek => voice::DEEPSEEK,
        }
    }
}

/// 一次合成所需的配置（每次合成时解析，改设置后立即生效）。
#[derive(Clone, PartialEq, Eq)]
pub struct OnlineConfig {
    /// OpenAI 兼容：连接的接口地址（如 `https://api.openai.com/v1`）；火山：完整端点。
    pub base_url: String,
    /// OpenAI 兼容：API key；火山：Access Token。
    pub api_key: String,
    /// OpenAI 兼容：合成模型（如 `gpt-4o-mini-tts`）；火山：cluster。
    pub model: String,
    /// 火山：App ID；OpenAI 兼容为空。
    pub app_id: String,
    /// 未在请求里指定时使用的声音（引擎自己的声音名，可带提供方前缀）。
    pub voice: String,
}

impl std::fmt::Debug for OnlineConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OnlineConfig")
            .field("base_url", &self.base_url)
            .field("model", &self.model)
            .field("voice", &self.voice)
            .finish_non_exhaustive()
    }
}

pub type OnlineResolver = Arc<dyn Fn() -> Result<OnlineConfig, String> + Send + Sync>;

/// DeepSeek 连接的地址常写成 `https://api.deepseek.com` 或 `…/v1`：语音接口在站点根下的 `/v1/tools/tts`。
pub fn deepseek_origin(base_url: &str) -> String {
    let base = base_url.trim().trim_end_matches('/');
    let base = if base.is_empty() {
        DEEPSEEK_ORIGIN
    } else {
        base
    };
    base.strip_suffix("/v1")
        .unwrap_or(base)
        .trim_end_matches('/')
        .to_string()
}

pub const DEEPSEEK_ORIGIN: &str = "https://api.deepseek.com";

/// 语速 -10..=10 → 倍速 0.5..=2.0（与 Kokoro 一样只改时长，不改音高）。
pub fn speed_ratio(rate: i32) -> f32 {
    2.0_f32.powf(rate.clamp(-10, 10) as f32 / 10.0)
}

pub struct OnlineSynth {
    kind: OnlineKind,
    resolver: OnlineResolver,
    /// 在合成线程里首次使用时创建：阻塞客户端自带运行时，不能在异步上下文里创建或销毁。
    client: std::sync::OnceLock<reqwest::blocking::Client>,
}

impl OnlineSynth {
    pub fn new(kind: OnlineKind, resolver: OnlineResolver) -> Result<Self, String> {
        Ok(Self {
            kind,
            resolver,
            client: std::sync::OnceLock::new(),
        })
    }

    fn client(&self) -> Result<&reqwest::blocking::Client, String> {
        if let Some(client) = self.client.get() {
            return Ok(client);
        }
        let client = reqwest::blocking::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .user_agent(concat!("Kotone/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| format!("无法创建语音合成客户端：{e}"))?;
        Ok(self.client.get_or_init(|| client))
    }

    fn resolve(&self, request: &SpeechRequest) -> Result<(OnlineConfig, String), String> {
        let config = (self.resolver)()?;
        let chosen = request
            .voice_id
            .as_deref()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or(&config.voice);
        let native = voice::native(self.kind.provider(), chosen)?
            .trim()
            .to_string();
        if native.is_empty() {
            return Err("请先在模型设置里为在线语音选择声音".into());
        }
        if config.api_key.trim().is_empty() {
            return Err(match self.kind {
                OnlineKind::OpenAi => "在线语音合成的 API 连接尚未保存密钥".into(),
                OnlineKind::Volcano => "火山语音合成缺少 Access Token".into(),
                OnlineKind::Deepseek => "DeepSeek 连接尚未保存 API key".into(),
            });
        }
        Ok((config, native))
    }

    /// 缓存键：提供方 + 声音 + 模型 + 端点 + 语速 + 文本（不含密钥）。
    pub fn cache_key(&self, request: &SpeechRequest) -> Result<String, String> {
        let (config, native) = self.resolve(request)?;
        Ok(format!(
            "{}/{native}\u{1}{}\u{1}{}\u{1}{}\u{1}{}\u{1}{}",
            self.kind.provider(),
            config.model,
            config.base_url,
            config.app_id,
            request.rate,
            request.text
        ))
    }

    pub fn synthesize(&self, request: &SpeechRequest) -> Result<SynthesizedSpeech, String> {
        request.validate()?;
        let (config, native) = self.resolve(request)?;
        let (bytes, provider) = match self.kind {
            OnlineKind::OpenAi => (self.openai(&config, &native, request)?, "openai"),
            OnlineKind::Volcano => (self.volcano(&config, &native, request)?, "volcano"),
            OnlineKind::Deepseek => (self.deepseek(&config, &native, request)?, "deepseek"),
        };
        // 用量账本（按字计费）：真的请求了才记，缓存命中走不到这里。
        kotone_core::usage::record_tts(provider, &config.model, &request.text);
        crate::wav::decode(&bytes)
    }

    fn read_limited(response: reqwest::blocking::Response) -> Result<Vec<u8>, String> {
        use std::io::Read;
        let mut bytes = Vec::new();
        response
            .take(MAX_RESPONSE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "读取语音合成结果失败".to_string())?;
        if bytes.len() as u64 > MAX_RESPONSE_BYTES {
            return Err("语音合成结果超出大小限制".into());
        }
        Ok(bytes)
    }

    fn send(&self, builder: reqwest::blocking::RequestBuilder) -> Result<Vec<u8>, String> {
        let response = builder.send().map_err(|e| {
            if e.is_timeout() {
                "语音合成请求超时".to_string()
            } else {
                "无法连接语音合成服务，请检查接口地址与网络".to_string()
            }
        })?;
        if !response.status().is_success() {
            return Err(format!(
                "语音合成接口返回 HTTP {}，请检查连接、模型与声音",
                response.status().as_u16()
            ));
        }
        Self::read_limited(response)
    }

    fn openai(
        &self,
        config: &OnlineConfig,
        voice: &str,
        request: &SpeechRequest,
    ) -> Result<Vec<u8>, String> {
        let url = format!(
            "{}/audio/speech",
            config.base_url.trim().trim_end_matches('/')
        );
        let model = if config.model.trim().is_empty() {
            "gpt-4o-mini-tts"
        } else {
            config.model.trim()
        };
        let body = json!({
            "model": model,
            "input": request.text,
            "voice": voice,
            "response_format": "wav",
            "speed": speed_ratio(request.rate),
        });
        self.send(
            self.client()?
                .post(url)
                .bearer_auth(config.api_key.trim())
                .json(&body),
        )
    }

    /// DeepSeek 官方语音合成：`POST <站点>/v1/tools/tts`，`{text, voice_id, format: wav}`。
    /// 接口没有语速参数（语速设置对它不生效）；按字符计费，汉字算 2 个字符。
    fn deepseek(
        &self,
        config: &OnlineConfig,
        voice: &str,
        request: &SpeechRequest,
    ) -> Result<Vec<u8>, String> {
        let url = format!("{}/v1/tools/tts", deepseek_origin(&config.base_url));
        let body = json!({
            "text": request.text,
            "voice_id": voice,
            "format": "wav",
            "filter_markdown": true,
        });
        self.send(
            self.client()?
                .post(url)
                .bearer_auth(config.api_key.trim())
                .json(&body),
        )
    }

    fn volcano(
        &self,
        config: &OnlineConfig,
        voice: &str,
        request: &SpeechRequest,
    ) -> Result<Vec<u8>, String> {
        if config.app_id.trim().is_empty() {
            return Err("火山语音合成缺少 App ID".into());
        }
        let cluster = if config.model.trim().is_empty() {
            VOLCANO_DEFAULT_CLUSTER
        } else {
            config.model.trim()
        };
        let endpoint = if config.base_url.trim().is_empty() {
            VOLCANO_ENDPOINT
        } else {
            config.base_url.trim()
        };
        let body = json!({
            "app": {"appid": config.app_id.trim(), "token": config.api_key.trim(), "cluster": cluster},
            "user": {"uid": "kotone"},
            "audio": {
                "voice_type": voice,
                "encoding": "wav",
                "rate": 24000,
                "speed_ratio": speed_ratio(request.rate),
            },
            "request": {
                "reqid": request_id(),
                "text": request.text,
                "text_type": "plain",
                "operation": "query",
            }
        });
        let bytes = self.send(
            self.client()?
                .post(endpoint)
                .header("Authorization", format!("Bearer;{}", config.api_key.trim()))
                .json(&body),
        )?;
        let value: Value = serde_json::from_slice(&bytes)
            .map_err(|_| "火山语音合成返回的不是有效 JSON".to_string())?;
        let code = value.get("code").and_then(Value::as_i64).unwrap_or(-1);
        if code != 3000 {
            let message: String = value
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("")
                .chars()
                .take(120)
                .collect();
            return Err(format!("火山语音合成失败（code {code}）：{message}"));
        }
        let data = value
            .get("data")
            .and_then(Value::as_str)
            .ok_or("火山语音合成没有返回音频")?;
        base64::engine::general_purpose::STANDARD
            .decode(data)
            .map_err(|_| "火山语音合成返回的音频无法解码".to_string())
    }
}

fn request_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("kotone-{now:x}-{}", COUNTER.fetch_add(1, Ordering::Relaxed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn request(voice: Option<&str>) -> SpeechRequest {
        SpeechRequest {
            text: "队友说先别打龙".into(),
            voice_id: voice.map(str::to_string),
            volume: 80,
            rate: 0,
        }
    }

    fn resolver(base_url: String, voice: &str) -> OnlineResolver {
        let voice = voice.to_string();
        Arc::new(move || {
            Ok(OnlineConfig {
                base_url: base_url.clone(),
                api_key: "secret-key".into(),
                model: String::new(),
                app_id: "app-1".into(),
                voice: voice.clone(),
            })
        })
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn openai_compatible_request_and_wav_decoding() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/audio/speech"))
            .and(header("authorization", "Bearer secret-key"))
            .respond_with(
                ResponseTemplate::new(200).set_body_bytes(crate::wav::encode_pcm16(
                    &[0, 100, -100],
                    24_000,
                    None,
                )),
            )
            .expect(1)
            .mount(&server)
            .await;
        let synth = OnlineSynth::new(
            OnlineKind::OpenAi,
            resolver(format!("{}/v1/", server.uri()), "alloy"),
        )
        .unwrap();
        let speech = tokio::task::spawn_blocking(move || synth.synthesize(&request(None)))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(speech.samples.len(), 3);
        let body: Value =
            serde_json::from_slice(&server.received_requests().await.unwrap()[0].body).unwrap();
        assert_eq!(body["voice"], "alloy");
        assert_eq!(body["response_format"], "wav");
        assert_eq!(body["model"], "gpt-4o-mini-tts");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn volcano_request_reuses_app_credentials_and_reports_service_errors() {
        let server = MockServer::start().await;
        let audio = base64::engine::general_purpose::STANDARD.encode(crate::wav::encode_pcm16(
            &[1, 2],
            24_000,
            None,
        ));
        Mock::given(method("POST"))
            .and(header("authorization", "Bearer;secret-key"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"code": 3000, "message": "Success", "data": audio})),
            )
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"code": 3001, "message": "invalid voice_type"})),
            )
            .mount(&server)
            .await;
        let synth = Arc::new(
            OnlineSynth::new(
                OnlineKind::Volcano,
                resolver(server.uri(), "volcano/BV700_streaming"),
            )
            .unwrap(),
        );
        let first = synth.clone();
        let speech = tokio::task::spawn_blocking(move || first.synthesize(&request(None)))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(speech.samples.len(), 2);
        let body: Value =
            serde_json::from_slice(&server.received_requests().await.unwrap()[0].body).unwrap();
        assert_eq!(body["app"]["appid"], "app-1");
        assert_eq!(body["app"]["cluster"], "volcano_tts");
        assert_eq!(body["audio"]["voice_type"], "BV700_streaming");
        let second = synth.clone();
        let error = tokio::task::spawn_blocking(move || second.synthesize(&request(None)))
            .await
            .unwrap()
            .unwrap_err();
        assert!(error.contains("3001"), "{error}");
        // 选中其他提供方的声音：报错而不是换声音
        let third = synth.clone();
        let error =
            tokio::task::spawn_blocking(move || third.synthesize(&request(Some("openai/alloy"))))
                .await
                .unwrap()
                .unwrap_err();
        assert!(error.contains("不符"));
        let key = synth.cache_key(&request(None)).unwrap();
        assert!(key.starts_with("volcano/BV700_streaming") && !key.contains("secret-key"));
        // 阻塞客户端只能在非异步线程销毁（与桌面端在合成线程里持有它一致）
        tokio::task::spawn_blocking(move || drop(synth))
            .await
            .unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn deepseek_request_uses_the_site_root_and_the_connection_key() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/tools/tts"))
            .and(header("authorization", "Bearer secret-key"))
            .respond_with(
                ResponseTemplate::new(200).set_body_bytes(crate::wav::encode_pcm16(
                    &[5, -5, 5, -5],
                    24_000,
                    None,
                )),
            )
            .expect(1)
            .mount(&server)
            .await;
        // 连接地址写成 …/v1 也能找到站点根下的语音接口
        let synth = OnlineSynth::new(
            OnlineKind::Deepseek,
            resolver(format!("{}/v1", server.uri()), "deepseek/stella"),
        )
        .unwrap();
        let speech = tokio::task::spawn_blocking(move || {
            let result = synth.synthesize(&request(None));
            drop(synth);
            result
        })
        .await
        .unwrap()
        .unwrap();
        assert_eq!(speech.samples.len(), 4);
        let body: Value =
            serde_json::from_slice(&server.received_requests().await.unwrap()[0].body).unwrap();
        assert_eq!(body["text"], "队友说先别打龙");
        assert_eq!(body["voice_id"], "stella");
        assert_eq!(body["format"], "wav");
        assert!(body.get("speed").is_none(), "接口没有语速参数");
    }

    #[test]
    fn deepseek_origin_strips_the_api_version() {
        assert_eq!(
            deepseek_origin("https://api.deepseek.com/v1"),
            "https://api.deepseek.com"
        );
        assert_eq!(
            deepseek_origin("https://api.deepseek.com/"),
            "https://api.deepseek.com"
        );
        assert_eq!(deepseek_origin(""), DEEPSEEK_ORIGIN);
    }

    #[test]
    fn speed_ratio_is_bounded_and_monotonic() {
        assert!((speed_ratio(0) - 1.0).abs() < 1e-6);
        assert!((speed_ratio(10) - 2.0).abs() < 1e-6);
        assert!((speed_ratio(-10) - 0.5).abs() < 1e-6);
        assert!(speed_ratio(-1) < speed_ratio(1));
    }
}
