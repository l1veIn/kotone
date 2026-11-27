//! 在线语音合成：DeepSeek（`/v1/tools/tts`）、MiniMax（`/v1/t2a_v2`）、阿里云百炼 Qwen3-TTS-Flash
//! （`multimodal-generation`）、硅基流动与自定义接口（OpenAI 兼容 `/audio/speech`）、火山（`/api/v1/tts`）。
//!
//! - 整段合成（不流式），要 WAV（百炼给一个音频链接，再下载那个 WAV），解码后交给与 Kokoro 相同的播放与取消路径。
//! - 连接与 Key 由调用方在每次合成时解析（[`OnlineResolver`]），Key 只来自凭据库，不进缓存键、日志或错误消息；
//!   错误不回显响应体，服务商的错误码翻成一句中文（ADR 048）。
//! - 不跟随跳转：Key 不会被带到别的地址去；跳转一律当失败。百炼的音频链接只下官方的存储地址，不带 Key。
//! - 选中的声音属于其他提供方时直接报错，不换声音。
//! - 保存 Key 时“先试一次”（[`check`]、[`detect_region`]）：只用不花钱的接口（列声音、列模型）；火山和自定义
//!   接口没有这种接口，就合成一个字。

use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use kotone_core::speech::{SpeechRequest, SpeechVoice};
use kotone_core::voice_vendor::Region;
use serde_json::{json, Value};

use crate::{voice, SynthesizedSpeech};

const MAX_RESPONSE_BYTES: u64 = 20 * 1024 * 1024;
/// 百炼的音频文件最多下载这么大（一次最多 600 字，24 kHz 单声道 WAV 远小于它）。
const MAX_AUDIO_DOWNLOAD_BYTES: u64 = 16 * 1024 * 1024;
/// 百炼一次最多 600 字；按这个长度先分好段（留点余量）。
const DASHSCOPE_MAX_CHARS: usize = 500;

pub const VOLCANO_ENDPOINT: &str = "https://openspeech.bytedance.com/api/v1/tts";
pub const VOLCANO_DEFAULT_CLUSTER: &str = "volcano_tts";
pub const DEEPSEEK_ORIGIN: &str = "https://api.deepseek.com";
/// MiniMax 国内站（官方文档 2026-11 起写 `api.minimax.cn`；旧的 `api.minimaxi.com` 也还通）。
pub const MINIMAX_CN_ORIGIN: &str = "https://api.minimax.cn";
pub const MINIMAX_INTL_ORIGIN: &str = "https://api.minimax.io";
pub const MINIMAX_DEFAULT_MODEL: &str = "speech-2.8-turbo";
pub const MINIMAX_DEFAULT_VOICE: &str = "female-shaonv";
/// 阿里云百炼（北京 / 新加坡；两边的 Key 不通用）。
pub const DASHSCOPE_CN_ORIGIN: &str = "https://dashscope.aliyuncs.com";
pub const DASHSCOPE_INTL_ORIGIN: &str = "https://dashscope-intl.aliyuncs.com";
pub const DASHSCOPE_DEFAULT_MODEL: &str = "qwen3-tts-flash";
pub const DASHSCOPE_DEFAULT_VOICE: &str = "Cherry";
/// 硅基流动：OpenAI 兼容接口的预设。
pub const SILICONFLOW_BASE_URL: &str = "https://api.siliconflow.cn/v1";
pub const SILICONFLOW_DEFAULT_MODEL: &str = "FunAudioLLM/CosyVoice2-0.5B";
pub const SILICONFLOW_DEFAULT_VOICE: &str = "FunAudioLLM/CosyVoice2-0.5B:claire";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OnlineKind {
    /// 自定义（OpenAI 兼容接口）。
    OpenAi,
    Volcano,
    Deepseek,
    Minimax,
    Dashscope,
    Siliconflow,
}

impl OnlineKind {
    pub const ALL: [OnlineKind; 6] = [
        Self::Deepseek,
        Self::Minimax,
        Self::Dashscope,
        Self::Siliconflow,
        Self::Volcano,
        Self::OpenAi,
    ];

    pub fn provider(self) -> &'static str {
        match self {
            Self::OpenAi => voice::OPENAI,
            Self::Volcano => voice::VOLCANO,
            Self::Deepseek => voice::DEEPSEEK,
            Self::Minimax => voice::MINIMAX,
            Self::Dashscope => voice::DASHSCOPE,
            Self::Siliconflow => voice::SILICONFLOW,
        }
    }

    /// 给玩家看的服务商名字（错误消息里用）。
    pub fn display_name(self) -> &'static str {
        match self {
            Self::OpenAi => "自定义语音接口",
            Self::Volcano => "火山语音合成",
            Self::Deepseek => "DeepSeek",
            Self::Minimax => "MiniMax",
            Self::Dashscope => "阿里云百炼",
            Self::Siliconflow => "硅基流动",
        }
    }
}

/// 一次合成所需的配置（每次合成时解析，改设置后立即生效）。
#[derive(Clone, PartialEq, Eq)]
pub struct OnlineConfig {
    /// 自定义 / 硅基流动：接口地址（如 `https://api.siliconflow.cn/v1`）；DeepSeek / MiniMax / 百炼：站点
    /// （带不带 `/v1` 都行）；火山：完整端点。
    pub base_url: String,
    /// Key（火山是 Access Token）。
    pub api_key: String,
    /// 合成模型（火山是 cluster）。
    pub model: String,
    /// 火山：App ID；其余为空。
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

/// 站点根：去掉末尾的 `/` 和 `/v1`（连接地址常写成 `https://api.deepseek.com/v1`、`https://api.minimaxi.com/v1`）。
pub fn site_origin(base_url: &str, default: &str) -> String {
    let base = base_url.trim().trim_end_matches('/');
    let base = if base.is_empty() { default } else { base };
    base.strip_suffix("/v1")
        .unwrap_or(base)
        .trim_end_matches('/')
        .to_string()
}

/// DeepSeek 连接的地址常写成 `https://api.deepseek.com` 或 `…/v1`：语音接口在站点根下的 `/v1/tools/tts`。
pub fn deepseek_origin(base_url: &str) -> String {
    site_origin(base_url, DEEPSEEK_ORIGIN)
}

/// 这一家、这个站的默认地址（DeepSeek、MiniMax、百炼、硅基流动）。
pub fn default_origin(kind: OnlineKind, region: Region) -> &'static str {
    match (kind, region) {
        (OnlineKind::Minimax, Region::Cn) => MINIMAX_CN_ORIGIN,
        (OnlineKind::Minimax, Region::Intl) => MINIMAX_INTL_ORIGIN,
        (OnlineKind::Dashscope, Region::Cn) => DASHSCOPE_CN_ORIGIN,
        (OnlineKind::Dashscope, Region::Intl) => DASHSCOPE_INTL_ORIGIN,
        (OnlineKind::Siliconflow, _) => SILICONFLOW_BASE_URL,
        (OnlineKind::Volcano, _) => VOLCANO_ENDPOINT,
        (OnlineKind::Deepseek, _) => DEEPSEEK_ORIGIN,
        // 自定义接口没有默认地址
        (OnlineKind::OpenAi, _) => "",
    }
}

/// 语速 -10..=10 → 倍速 0.5..=2.0（与 Kokoro 一样只改时长，不改音高）。
pub fn speed_ratio(rate: i32) -> f32 {
    2.0_f32.powf(rate.clamp(-10, 10) as f32 / 10.0)
}

/// 合成和“先试一次”共用的 HTTP 客户端：不跟随跳转（Key 不会被带去别的地址）。阻塞客户端自带运行时，
/// 不能在异步上下文里创建或销毁。
fn build_client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!("Kotone/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| format!("无法创建语音合成客户端：{e}"))
}

fn read_limited(response: reqwest::blocking::Response, limit: u64) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let mut bytes = Vec::new();
    response
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "读取语音合成结果失败".to_string())?;
    if bytes.len() as u64 > limit {
        return Err("语音合成结果超出大小限制".into());
    }
    Ok(bytes)
}

/// 发请求，拿回状态码和响应体（不看状态码）。网络错误翻成中文。
fn send_raw(builder: reqwest::blocking::RequestBuilder) -> Result<(u16, Vec<u8>), String> {
    let response = builder.send().map_err(|e| {
        if e.is_timeout() {
            "语音合成请求超时".to_string()
        } else {
            "无法连接语音合成服务，请检查接口地址与网络".to_string()
        }
    })?;
    let status = response.status().as_u16();
    Ok((status, read_limited(response, MAX_RESPONSE_BYTES)?))
}

fn http_error(status: u16) -> String {
    if (300..400).contains(&status) {
        return format!(
            "语音合成接口要把请求转到别的地址（HTTP {status}），没跟过去：请检查接口地址"
        );
    }
    format!("语音合成接口返回 HTTP {status}，请检查连接、模型与声音")
}

/// 发请求，不是 2xx 就报错（不回显响应体）。
fn send(builder: reqwest::blocking::RequestBuilder) -> Result<Vec<u8>, String> {
    let (status, bytes) = send_raw(builder)?;
    if !(200..300).contains(&status) {
        return Err(http_error(status));
    }
    Ok(bytes)
}

/// 十六进制文本 → 字节（MiniMax 的 `data.audio`）。
fn decode_hex(text: &str) -> Option<Vec<u8>> {
    let text = text.trim();
    if text.len() % 2 != 0 {
        return None;
    }
    let digit = |b: u8| match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    };
    text.as_bytes()
        .chunks_exact(2)
        .map(|pair| Some((digit(pair[0])? << 4) | digit(pair[1])?))
        .collect()
}

/// MiniMax 的业务错误码 → 一句中文（不回显 `status_msg`）。
fn minimax_error(code: i64) -> String {
    match code {
        1004 | 2049 => "MiniMax 不认这个 Key（登录失败）：请检查 Key 有没有填对，或者它是不是另一个站（国内 / 国际）的 Key".into(),
        1008 => "MiniMax 账户余额不足".into(),
        1002 | 1039 => "MiniMax 说请求太频繁（限流了），过一会儿再试".into(),
        1001 => "MiniMax 语音合成超时".into(),
        1026 | 1027 => "MiniMax 说这段文字不能念（内容审核）".into(),
        1042 => "这句话里看不见的字符太多，MiniMax 不念".into(),
        2013 => "MiniMax 说参数不对：多半是这个声音或模型用不了".into(),
        _ => format!("MiniMax 语音合成失败（错误码 {code}）"),
    }
}

/// 读 MiniMax 的 `base_resp`：HTTP 200 里也可能装着失败。没有 `base_resp` 当成功（有的接口不带）。
fn minimax_status(value: &Value) -> Result<(), (i64, String)> {
    let Some(code) = value
        .get("base_resp")
        .and_then(|b| b.get("status_code"))
        .and_then(Value::as_i64)
    else {
        return Ok(());
    };
    if code == 0 {
        Ok(())
    } else {
        Err((code, minimax_error(code)))
    }
}

fn is_minimax_key_error(code: i64) -> bool {
    matches!(code, 1004 | 2049)
}

/// 百炼的错误（HTTP 状态 + 响应里的 `code`）→ 一句中文（不回显 `message`）。
fn dashscope_error(status: u16, body: &[u8]) -> String {
    let code = serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|v| v.get("code").and_then(Value::as_str).map(str::to_string))
        .unwrap_or_default();
    match (status, code.as_str()) {
        (401 | 403, _) | (_, "InvalidApiKey") => {
            "阿里云百炼不认这个 Key：请检查 Key 有没有填对，或者它是不是另一个地域（北京 / 新加坡）的 Key".into()
        }
        (_, "Arrearage") => "阿里云百炼账户欠费了".into(),
        (429, _) | (_, "Throttling" | "Throttling.RateQuota" | "Throttling.AllocationQuota") => {
            "阿里云百炼说请求太频繁（限流了），过一会儿再试".into()
        }
        (_, "DataInspectionFailed") => "阿里云百炼说这段文字不能念（内容审核）".into(),
        (_, "InvalidParameter") => "阿里云百炼说参数不对：多半是这个声音或模型用不了".into(),
        _ if (300..400).contains(&status) => http_error(status),
        _ => format!("阿里云百炼语音合成失败（HTTP {status}）"),
    }
}

/// 百炼的音频链接是不是官方存储（`dashscope-result-<地域>.oss-<地域>.aliyuncs.com`）。
fn is_official_audio_host(host: &str) -> bool {
    host.starts_with("dashscope-result")
        && host.ends_with(".aliyuncs.com")
        && host.contains(".oss-")
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
}

/// 下载地址检查：官方存储改走 https；其余的地址只有测试放行的才下（见 [`OnlineSynth::allow_downloads_from`]）。
type DownloadCheck = Arc<dyn Fn(&reqwest::Url) -> bool + Send + Sync>;

fn audio_url(raw: &str, extra: Option<&DownloadCheck>) -> Result<reqwest::Url, String> {
    let mut url =
        reqwest::Url::parse(raw.trim()).map_err(|_| "阿里云百炼返回的音频地址无效".to_string())?;
    let host = url.host_str().unwrap_or("").to_ascii_lowercase();
    if is_official_audio_host(&host) && matches!(url.scheme(), "http" | "https") {
        // 官方示例给的是 http 链接；同一个地址走 https
        if url.scheme() == "http" {
            url.set_scheme("https")
                .map_err(|_| "阿里云百炼返回的音频地址无效".to_string())?;
            let _ = url.set_port(None);
        }
        return Ok(url);
    }
    if extra.is_some_and(|allow| allow(&url)) {
        return Ok(url);
    }
    Err("阿里云百炼返回的音频地址不是官方的存储地址，没有下载".into())
}

/// 百炼一次最多 600 字：长文字先在句末、逗号处分段（每段不超过 [`DASHSCOPE_MAX_CHARS`] 个字）。
fn split_for_dashscope(text: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut count = 0;
    let mut last_break: Option<(usize, usize)> = None; // (字节位置, 字数)
    for c in text.chars() {
        current.push(c);
        count += 1;
        if "。！？；!?;\n，,、".contains(c) {
            last_break = Some((current.len(), count));
        }
        if count >= DASHSCOPE_MAX_CHARS {
            let (cut, cut_count) = last_break.unwrap_or((current.len(), count));
            let rest = current.split_off(cut);
            if !current.trim().is_empty() {
                parts.push(std::mem::take(&mut current));
            }
            current = rest;
            count -= cut_count;
            last_break = None;
        }
    }
    if !current.trim().is_empty() {
        parts.push(current);
    }
    parts
}

pub struct OnlineSynth {
    kind: OnlineKind,
    resolver: OnlineResolver,
    /// 在合成线程里首次使用时创建：阻塞客户端自带运行时，不能在异步上下文里创建或销毁。
    client: std::sync::OnceLock<reqwest::blocking::Client>,
    /// 测试用：放行本机的音频下载地址。
    extra_downloads: Option<DownloadCheck>,
}

impl OnlineSynth {
    pub fn new(kind: OnlineKind, resolver: OnlineResolver) -> Result<Self, String> {
        Ok(Self {
            kind,
            resolver,
            client: std::sync::OnceLock::new(),
            extra_downloads: None,
        })
    }

    /// 测试用：除了百炼的官方存储，也下载这些地址（本机的模拟服务）。
    #[cfg(test)]
    fn allow_downloads_from(
        mut self,
        check: impl Fn(&reqwest::Url) -> bool + Send + Sync + 'static,
    ) -> Self {
        self.extra_downloads = Some(Arc::new(check));
        self
    }

    fn client(&self) -> Result<&reqwest::blocking::Client, String> {
        if let Some(client) = self.client.get() {
            return Ok(client);
        }
        let client = build_client()?;
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
            return Err("请先为她的声音选一个声音".into());
        }
        // 自定义接口（比如本机的语音服务）可以没有 Key；别的都要
        if config.api_key.trim().is_empty() && self.kind != OnlineKind::OpenAi {
            return Err(match self.kind {
                OnlineKind::Volcano => "火山语音合成缺少 Access Token".into(),
                OnlineKind::Deepseek => "DeepSeek 连接还没有保存 Key".into(),
                kind => format!(
                    "{} 还没有 Key：在声音设置里填，或者在「API 连接」里加一条{}的连接",
                    kind.display_name(),
                    kind.display_name()
                ),
            });
        }
        Ok((config, native))
    }

    /// 缓存键：提供方 + 声音 + 模型 + 端点 + 语速 + 文本（不含 Key）。
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
        let speech = match self.kind {
            OnlineKind::OpenAi | OnlineKind::Siliconflow => {
                crate::wav::decode(&self.openai(&config, &native, request)?)?
            }
            OnlineKind::Volcano => crate::wav::decode(&self.volcano(&config, &native, request)?)?,
            OnlineKind::Deepseek => crate::wav::decode(&self.deepseek(&config, &native, request)?)?,
            OnlineKind::Minimax => crate::wav::decode(&self.minimax(&config, &native, request)?)?,
            OnlineKind::Dashscope => self.dashscope(&config, &native, request)?,
        };
        let provider = self.kind.provider();
        // 用量账本（按字计费）：真的请求了才记，缓存命中走不到这里。
        kotone_core::usage::record_tts(provider, &config.model, &request.text);
        kotone_core::log::log(&format!(
            "speech: synthesized {} chars with {provider}/{native}",
            request.text.chars().count()
        ));
        Ok(speech)
    }

    /// OpenAI 兼容 `/audio/speech`：自定义接口和硅基流动。
    fn openai(
        &self,
        config: &OnlineConfig,
        voice: &str,
        request: &SpeechRequest,
    ) -> Result<Vec<u8>, String> {
        let siliconflow = self.kind == OnlineKind::Siliconflow;
        let base = if config.base_url.trim().is_empty() && siliconflow {
            SILICONFLOW_BASE_URL
        } else {
            config.base_url.trim()
        };
        if base.is_empty() {
            return Err("自定义语音接口还没有填地址".into());
        }
        let model = match config.model.trim() {
            "" if siliconflow => SILICONFLOW_DEFAULT_MODEL,
            "" => return Err("自定义语音接口还没有填模型名".into()),
            model => model,
        };
        // 硅基流动的系统声音写成 `<模型>:<声音>`；只填了声音名（claire）就补上模型
        let voice = if siliconflow && !voice.contains(':') {
            format!("{model}:{voice}")
        } else {
            voice.to_string()
        };
        let mut body = json!({
            "model": model,
            "input": request.text,
            "voice": voice,
            "response_format": "wav",
            "speed": speed_ratio(request.rate),
        });
        if siliconflow {
            body["sample_rate"] = json!(24_000);
        }
        let mut builder = self
            .client()?
            .post(format!("{}/audio/speech", base.trim_end_matches('/')))
            .json(&body);
        if !config.api_key.trim().is_empty() {
            builder = builder.bearer_auth(config.api_key.trim());
        }
        send(builder)
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
        send(
            self.client()?
                .post(url)
                .bearer_auth(config.api_key.trim())
                .json(&body),
        )
    }

    /// MiniMax 同步语音合成：`POST <站点>/v1/t2a_v2`，非流式、WAV、hex。HTTP 200 里也可能是失败，
    /// 先看 `base_resp.status_code`，再看 `data.status == 2`（合成完了），最后解 hex。
    fn minimax(
        &self,
        config: &OnlineConfig,
        voice: &str,
        request: &SpeechRequest,
    ) -> Result<Vec<u8>, String> {
        let url = format!(
            "{}/v1/t2a_v2",
            site_origin(&config.base_url, MINIMAX_CN_ORIGIN)
        );
        let model = match config.model.trim() {
            "" => MINIMAX_DEFAULT_MODEL,
            model => model,
        };
        let body = json!({
            "model": model,
            "text": request.text,
            "stream": false,
            "voice_setting": {
                "voice_id": voice,
                "speed": speed_ratio(request.rate),
                "vol": 1,
                "pitch": 0,
            },
            "audio_setting": {"sample_rate": 24_000, "format": "wav", "channel": 1},
            "output_format": "hex",
        });
        let (status, bytes) = send_raw(
            self.client()?
                .post(url)
                .bearer_auth(config.api_key.trim())
                .json(&body),
        )?;
        let value: Option<Value> = serde_json::from_slice(&bytes).ok();
        if let Some(value) = &value {
            minimax_status(value).map_err(|(_, message)| message)?;
        }
        if !(200..300).contains(&status) {
            return Err(http_error(status));
        }
        let value = value.ok_or("MiniMax 返回的不是有效 JSON")?;
        let data = value.get("data").ok_or("MiniMax 没有返回音频")?;
        if data.get("status").and_then(Value::as_i64) != Some(2) {
            return Err("MiniMax 没有返回合成完的音频".into());
        }
        let audio = data
            .get("audio")
            .and_then(Value::as_str)
            .filter(|a| !a.is_empty())
            .ok_or("MiniMax 没有返回音频")?;
        decode_hex(audio).ok_or_else(|| "MiniMax 返回的音频无法解码".into())
    }

    /// 阿里云百炼 Qwen3-TTS-Flash：`POST <站点>/api/v1/services/aigc/multimodal-generation/generation`。
    /// 非流式返回一个 WAV 链接（24 小时有效）：只从官方存储下载，不带 Key、不跟随跳转、限大小。
    /// 一次最多 600 字，长文字先分段再拼起来。接口没有语速参数。
    fn dashscope(
        &self,
        config: &OnlineConfig,
        voice: &str,
        request: &SpeechRequest,
    ) -> Result<SynthesizedSpeech, String> {
        let mut out: Option<SynthesizedSpeech> = None;
        for part in split_for_dashscope(&request.text) {
            let speech = crate::wav::decode(&self.dashscope_once(config, voice, &part)?)?;
            match &mut out {
                None => out = Some(speech),
                Some(all) if all.sample_rate == speech.sample_rate => {
                    all.samples.extend(speech.samples)
                }
                Some(_) => return Err("阿里云百炼返回的几段音频采样率不一样".into()),
            }
        }
        out.ok_or_else(|| "没有要念的字".into())
    }

    fn dashscope_once(
        &self,
        config: &OnlineConfig,
        voice: &str,
        text: &str,
    ) -> Result<Vec<u8>, String> {
        let url = format!(
            "{}/api/v1/services/aigc/multimodal-generation/generation",
            site_origin(&config.base_url, DASHSCOPE_CN_ORIGIN)
        );
        let model = match config.model.trim() {
            "" => DASHSCOPE_DEFAULT_MODEL,
            model => model,
        };
        let body = json!({
            "model": model,
            "input": {"text": text, "voice": voice, "language_type": "Chinese"},
        });
        let (status, bytes) = send_raw(
            self.client()?
                .post(url)
                .bearer_auth(config.api_key.trim())
                .json(&body),
        )?;
        if !(200..300).contains(&status) {
            return Err(dashscope_error(status, &bytes));
        }
        let value: Value = serde_json::from_slice(&bytes)
            .map_err(|_| "阿里云百炼返回的不是有效 JSON".to_string())?;
        if value
            .get("code")
            .and_then(Value::as_str)
            .is_some_and(|c| !c.is_empty())
        {
            return Err(dashscope_error(status, &bytes));
        }
        let audio = value
            .get("output")
            .and_then(|o| o.get("audio"))
            .ok_or("阿里云百炼没有返回音频")?;
        if let Some(data) = audio
            .get("data")
            .and_then(Value::as_str)
            .filter(|d| !d.is_empty())
        {
            return base64::engine::general_purpose::STANDARD
                .decode(data)
                .map_err(|_| "阿里云百炼返回的音频无法解码".to_string());
        }
        let link = audio
            .get("url")
            .and_then(Value::as_str)
            .filter(|u| !u.is_empty())
            .ok_or("阿里云百炼没有返回音频")?;
        let url = audio_url(link, self.extra_downloads.as_ref())?;
        // 下载音频不带 Key（链接自己带了签名）
        let response = self.client()?.get(url).send().map_err(|e| {
            if e.is_timeout() {
                "下载阿里云百炼的音频超时".to_string()
            } else {
                "下载阿里云百炼的音频失败".to_string()
            }
        })?;
        if !response.status().is_success() {
            return Err(format!(
                "下载阿里云百炼的音频失败（HTTP {}）",
                response.status().as_u16()
            ));
        }
        read_limited(response, MAX_AUDIO_DOWNLOAD_BYTES)
            .map_err(|_| "阿里云百炼的音频太大，没有下载".to_string())
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
        let bytes = send(
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

// ---------------------------------------------------------------------------------------------
// 保存 Key 时“先试一次”（ADR 048）

/// “先试一次”没过的原因。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckError {
    /// Key 被拒了（换一个站可能就行）。
    Key(String),
    /// 别的原因（网络、限流、余额……），给玩家看的一句话。
    Other(String),
}

impl CheckError {
    pub fn message(&self) -> &str {
        match self {
            Self::Key(message) | Self::Other(message) => message,
        }
    }
}

impl From<String> for CheckError {
    fn from(message: String) -> Self {
        Self::Other(message)
    }
}

/// MiniMax 的声音（`POST <站点>/v1/get_voice`，`voice_type: all`）：你复刻 / 设计的声音在前，中文的系统声音其次，
/// 其他语言最后。不花钱，也用来试 Key。阻塞，不能在异步上下文里调。
pub fn minimax_voices(base_url: &str, api_key: &str) -> Result<Vec<SpeechVoice>, CheckError> {
    if api_key.trim().is_empty() {
        return Err(CheckError::Key("MiniMax 还没有 Key".into()));
    }
    let url = format!("{}/v1/get_voice", site_origin(base_url, MINIMAX_CN_ORIGIN));
    let (status, bytes) = send_raw(
        build_client()?
            .post(url)
            .bearer_auth(api_key.trim())
            .json(&json!({"voice_type": "all"})),
    )?;
    let value: Option<Value> = serde_json::from_slice(&bytes).ok();
    if let Some(value) = &value {
        if let Err((code, message)) = minimax_status(value) {
            return Err(if is_minimax_key_error(code) {
                CheckError::Key(message)
            } else {
                CheckError::Other(message)
            });
        }
    }
    if status == 401 || status == 403 {
        return Err(CheckError::Key(minimax_error(1004)));
    }
    if !(200..300).contains(&status) {
        return Err(CheckError::Other(http_error(status)));
    }
    let value = value.ok_or_else(|| CheckError::Other("MiniMax 返回的不是有效 JSON".into()))?;
    Ok(parse_minimax_voices(&value))
}

fn parse_minimax_voices(value: &Value) -> Vec<SpeechVoice> {
    let entries = |key: &str| -> Vec<(String, String)> {
        value
            .get(key)
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| {
                        let id = item.get("voice_id")?.as_str()?.trim();
                        if id.is_empty() || id.len() > 256 || id.chars().any(char::is_control) {
                            return None;
                        }
                        let name = item
                            .get("voice_name")
                            .and_then(Value::as_str)
                            .map(str::trim)
                            .filter(|n| !n.is_empty())
                            .unwrap_or(id);
                        Some((id.to_string(), name.chars().take(80).collect()))
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    let mut own: Vec<SpeechVoice> = Vec::new();
    for (key, label) in [("voice_cloning", "复刻"), ("voice_generation", "设计")] {
        for (id, _) in entries(key) {
            own.push(SpeechVoice {
                id: format!("{}/{id}", voice::MINIMAX),
                name: format!("我的声音（{label}）· {id}"),
            });
        }
    }
    let system = entries("system_voice");
    let chinese = |id: &str| {
        crate::catalog::is_minimax_mandarin(id)
            || id.starts_with("Chinese")
            || id.starts_with("Cantonese")
    };
    let to_voice = |(id, name): &(String, String)| SpeechVoice {
        id: format!("{}/{id}", voice::MINIMAX),
        name: name.clone(),
    };
    own.extend(system.iter().filter(|(id, _)| chinese(id)).map(to_voice));
    own.extend(system.iter().filter(|(id, _)| !chinese(id)).map(to_voice));
    own.truncate(2_000);
    own
}

/// 用不花钱的接口试一下这个 Key（MiniMax 列声音；百炼、硅基流动、DeepSeek 列模型）；火山和自定义接口没有
/// 这种接口，就合成一个字（花不到一分钱）。阻塞，不能在异步上下文里调。
pub fn check(kind: OnlineKind, config: &OnlineConfig) -> Result<(), CheckError> {
    match kind {
        OnlineKind::Minimax => minimax_voices(&config.base_url, &config.api_key).map(|_| ()),
        OnlineKind::Dashscope => list_models(
            kind,
            &format!(
                "{}/compatible-mode/v1/models",
                site_origin(&config.base_url, DASHSCOPE_CN_ORIGIN)
            ),
            &config.api_key,
        ),
        OnlineKind::Siliconflow => list_models(
            kind,
            &format!(
                "{}/models",
                if config.base_url.trim().is_empty() {
                    SILICONFLOW_BASE_URL
                } else {
                    config.base_url.trim().trim_end_matches('/')
                }
            ),
            &config.api_key,
        ),
        OnlineKind::Deepseek => list_models(
            kind,
            &format!("{}/models", deepseek_origin(&config.base_url)),
            &config.api_key,
        ),
        OnlineKind::Volcano | OnlineKind::OpenAi => {
            let fixed = config.clone();
            let synth = OnlineSynth::new(kind, Arc::new(move || Ok(fixed.clone())))?;
            synth
                .synthesize(&SpeechRequest {
                    text: "好".into(),
                    voice_id: None,
                    volume: 0,
                    rate: 0,
                })
                .map(|_| ())
                .map_err(CheckError::Other)
        }
    }
}

fn list_models(kind: OnlineKind, url: &str, api_key: &str) -> Result<(), CheckError> {
    if api_key.trim().is_empty() {
        return Err(CheckError::Key(format!(
            "{} 还没有 Key",
            kind.display_name()
        )));
    }
    let (status, bytes) = send_raw(build_client()?.get(url).bearer_auth(api_key.trim()))?;
    match status {
        200..=299 => Ok(()),
        401 | 403 => Err(CheckError::Key(match kind {
            OnlineKind::Dashscope => dashscope_error(401, &bytes),
            kind => format!(
                "{} 不认这个 Key：请检查 Key 有没有填对",
                kind.display_name()
            ),
        })),
        _ if kind == OnlineKind::Dashscope => {
            Err(CheckError::Other(dashscope_error(status, &bytes)))
        }
        _ => Err(CheckError::Other(http_error(status))),
    }
}

/// 自己填的 Key 是哪个站的（MiniMax 国内 / 国际，百炼北京 / 新加坡）：先试国内，Key 被拒再试国际。
/// `origins` 是 (站, 地址)，测试里换成本机的模拟服务。返回能用的站；两边都不认就报第一边的原因。
pub fn detect_region(
    kind: OnlineKind,
    api_key: &str,
    origins: &[(Region, String)],
) -> Result<Region, CheckError> {
    let mut first: Option<CheckError> = None;
    for (region, origin) in origins {
        let config = OnlineConfig {
            base_url: origin.clone(),
            api_key: api_key.to_string(),
            model: String::new(),
            app_id: String::new(),
            voice: String::new(),
        };
        match check(kind, &config) {
            Ok(()) => return Ok(*region),
            Err(CheckError::Key(message)) => {
                first.get_or_insert(CheckError::Key(message));
            }
            Err(other) => return Err(other),
        }
    }
    Err(first.unwrap_or_else(|| CheckError::Other("没有可以试的地址".into())))
}

/// [`detect_region`] 用的官方地址：先国内，再国际。
pub fn official_origins(kind: OnlineKind) -> Vec<(Region, String)> {
    [Region::Cn, Region::Intl]
        .into_iter()
        .map(|region| (region, default_origin(kind, region).to_string()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_partial_json, header, method, path};
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
        resolver_with(base_url, voice, "")
    }

    fn resolver_with(base_url: String, voice: &str, model: &str) -> OnlineResolver {
        let voice = voice.to_string();
        let model = model.to_string();
        Arc::new(move || {
            Ok(OnlineConfig {
                base_url: base_url.clone(),
                api_key: "secret-key".into(),
                model: model.clone(),
                app_id: "app-1".into(),
                voice: voice.clone(),
            })
        })
    }

    fn wav() -> Vec<u8> {
        crate::wav::encode_pcm16(&[0, 100, -100], 24_000, None)
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// 阻塞客户端只能在非异步线程用和销毁（与桌面端在合成线程里持有它一致）。
    async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
        tokio::task::spawn_blocking(f).await.unwrap()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn custom_openai_compatible_request_and_wav_decoding() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/audio/speech"))
            .and(header("authorization", "Bearer secret-key"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(wav()))
            .expect(1)
            .mount(&server)
            .await;
        let base = format!("{}/v1/", server.uri());
        let speech = blocking(move || {
            let synth =
                OnlineSynth::new(OnlineKind::OpenAi, resolver_with(base, "alloy", "my-tts"))
                    .unwrap();
            synth.synthesize(&request(None))
        })
        .await
        .unwrap();
        assert_eq!(speech.samples.len(), 3);
        let body: Value =
            serde_json::from_slice(&server.received_requests().await.unwrap()[0].body).unwrap();
        assert_eq!(body["voice"], "alloy");
        assert_eq!(body["response_format"], "wav");
        assert_eq!(body["model"], "my-tts");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn custom_interface_has_no_default_model() {
        let server = MockServer::start().await;
        let base = format!("{}/v1", server.uri());
        let error = blocking(move || {
            OnlineSynth::new(OnlineKind::OpenAi, resolver(base, "alloy"))
                .unwrap()
                .synthesize(&request(None))
        })
        .await
        .unwrap_err();
        assert!(error.contains("模型名"), "{error}");
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn siliconflow_preset_completes_the_voice_and_asks_for_wav() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/audio/speech"))
            .and(header("authorization", "Bearer secret-key"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(wav()))
            .mount(&server)
            .await;
        let base = format!("{}/v1", server.uri());
        let speech = blocking(move || {
            OnlineSynth::new(
                OnlineKind::Siliconflow,
                resolver(base, "siliconflow/claire"),
            )
            .unwrap()
            .synthesize(&request(None))
        })
        .await
        .unwrap();
        assert_eq!(speech.samples.len(), 3);
        let body: Value =
            serde_json::from_slice(&server.received_requests().await.unwrap()[0].body).unwrap();
        assert_eq!(body["model"], SILICONFLOW_DEFAULT_MODEL);
        assert_eq!(body["voice"], "FunAudioLLM/CosyVoice2-0.5B:claire");
        assert_eq!(body["response_format"], "wav");
        assert_eq!(body["sample_rate"], 24_000);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn minimax_checks_base_resp_then_decodes_hex_wav() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/t2a_v2"))
            .and(header("authorization", "Bearer secret-key"))
            .and(body_partial_json(json!({
                "model": MINIMAX_DEFAULT_MODEL,
                "stream": false,
                "output_format": "hex",
                "voice_setting": {"voice_id": "female-shaonv"},
                "audio_setting": {"format": "wav", "sample_rate": 24_000, "channel": 1}
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": {"audio": hex(&wav()), "status": 2},
                "extra_info": {"usage_characters": 14},
                "base_resp": {"status_code": 0, "status_msg": "success"}
            })))
            .expect(1)
            .mount(&server)
            .await;
        // 连接地址写成 …/v1 也能找到站点根下的接口
        let base = format!("{}/v1", server.uri());
        let speech = blocking(move || {
            OnlineSynth::new(OnlineKind::Minimax, resolver(base, "minimax/female-shaonv"))
                .unwrap()
                .synthesize(&request(None))
        })
        .await
        .unwrap();
        assert_eq!(speech.samples.len(), 3);
        assert_eq!(speech.sample_rate, 24_000);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn minimax_errors_inside_http_200_become_plain_chinese_without_the_key() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "base_resp": {"status_code": 1004, "status_msg": "login fail: secret-key leaked here"}
            })))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "base_resp": {"status_code": 1008, "status_msg": "insufficient balance"}
            })))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": {"audio": "", "status": 1},
                "base_resp": {"status_code": 0}
            })))
            .mount(&server)
            .await;
        let base = server.uri();
        let (auth, balance, unfinished) = blocking(move || {
            let synth =
                OnlineSynth::new(OnlineKind::Minimax, resolver(base, "female-shaonv")).unwrap();
            (
                synth.synthesize(&request(None)).unwrap_err(),
                synth.synthesize(&request(None)).unwrap_err(),
                synth.synthesize(&request(None)).unwrap_err(),
            )
        })
        .await;
        assert!(auth.contains("不认这个 Key"), "{auth}");
        assert!(
            !auth.contains("secret-key") && !auth.contains("login fail"),
            "{auth}"
        );
        assert!(balance.contains("余额不足"), "{balance}");
        assert!(unfinished.contains("合成完"), "{unfinished}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn oversized_responses_and_redirects_are_refused() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/big/v1/t2a_v2"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![
                b'0';
                (MAX_RESPONSE_BYTES + 10)
                    as usize
            ]))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/moved/v1/t2a_v2"))
            .respond_with(
                ResponseTemplate::new(302).insert_header("location", "https://evil.example/steal"),
            )
            .mount(&server)
            .await;
        let (big, moved) = (
            format!("{}/big", server.uri()),
            format!("{}/moved", server.uri()),
        );
        let (big, moved) = blocking(move || {
            let big = OnlineSynth::new(OnlineKind::Minimax, resolver(big, "female-shaonv"))
                .unwrap()
                .synthesize(&request(None))
                .unwrap_err();
            let moved = OnlineSynth::new(OnlineKind::Minimax, resolver(moved, "female-shaonv"))
                .unwrap()
                .synthesize(&request(None))
                .unwrap_err();
            (big, moved)
        })
        .await;
        assert!(big.contains("大小限制"), "{big}");
        assert!(
            moved.contains("302") && moved.contains("没跟过去"),
            "{moved}"
        );
        // 只请求了这两次：没跟着跳转去别的地址
        assert_eq!(server.received_requests().await.unwrap().len(), 2);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn minimax_voices_list_own_voices_first_then_chinese() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/get_voice"))
            .and(body_partial_json(json!({"voice_type": "all"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "system_voice": [
                    {"voice_id": "English_Graceful_Lady", "voice_name": "Graceful Lady"},
                    {"voice_id": "female-shaonv", "voice_name": "少女音色"},
                    {"voice_id": "Chinese (Mandarin)_Warm_Girl", "voice_name": "温暖少女"}
                ],
                "voice_cloning": [{"voice_id": "my-voice-01"}],
                "base_resp": {"status_code": 0}
            })))
            .mount(&server)
            .await;
        let base = server.uri();
        let voices = blocking(move || minimax_voices(&base, "secret-key"))
            .await
            .unwrap();
        let ids: Vec<_> = voices.iter().map(|v| v.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "minimax/my-voice-01",
                "minimax/female-shaonv",
                "minimax/Chinese (Mandarin)_Warm_Girl",
                "minimax/English_Graceful_Lady"
            ]
        );
        assert_eq!(voices[1].name, "少女音色");
        assert!(voices[0].name.contains("复刻"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn region_detection_retries_the_international_site_when_the_key_is_refused() {
        let cn = MockServer::start().await;
        let intl = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/get_voice"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "base_resp": {"status_code": 1004, "status_msg": "login fail"}
            })))
            .expect(1)
            .mount(&cn)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/get_voice"))
            .and(header("authorization", "Bearer intl-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "system_voice": [], "base_resp": {"status_code": 0}
            })))
            .expect(1)
            .mount(&intl)
            .await;
        let origins = vec![(Region::Cn, cn.uri()), (Region::Intl, intl.uri())];
        let found = blocking(move || detect_region(OnlineKind::Minimax, "intl-key", &origins))
            .await
            .unwrap();
        assert_eq!(found, Region::Intl);
        // 两边都不认：报 Key 不对；限流这种不是 Key 的问题，不再试另一边
        let refused = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "base_resp": {"status_code": 1004}
            })))
            .mount(&refused)
            .await;
        let busy = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "base_resp": {"status_code": 1002}
            })))
            .mount(&busy)
            .await;
        let both = vec![(Region::Cn, refused.uri()), (Region::Intl, refused.uri())];
        let limited = vec![(Region::Cn, busy.uri()), (Region::Intl, refused.uri())];
        let (both, limited) = blocking(move || {
            (
                detect_region(OnlineKind::Minimax, "k", &both).unwrap_err(),
                detect_region(OnlineKind::Minimax, "k", &limited).unwrap_err(),
            )
        })
        .await;
        assert!(matches!(both, CheckError::Key(_)), "{both:?}");
        assert!(
            matches!(limited, CheckError::Other(ref m) if m.contains("限流")),
            "{limited:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn dashscope_downloads_the_wav_without_the_key_and_only_from_allowed_hosts() {
        let server = MockServer::start().await;
        let audio_url = format!("{}/audio/abc.wav?Expires=1&Signature=x", server.uri());
        Mock::given(method("POST"))
            .and(path(
                "/api/v1/services/aigc/multimodal-generation/generation",
            ))
            .and(header("authorization", "Bearer secret-key"))
            .and(body_partial_json(json!({
                "model": DASHSCOPE_DEFAULT_MODEL,
                "input": {"voice": "Cherry", "language_type": "Chinese"}
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "output": {"finish_reason": "stop", "audio": {"url": audio_url, "data": ""}},
                "usage": {"characters": 14}
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/audio/abc.wav"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(wav()))
            .mount(&server)
            .await;
        let base = server.uri();
        let allowed_base = base.clone();
        let (speech, refused) = blocking(move || {
            let port = reqwest::Url::parse(&allowed_base).unwrap().port();
            let allowed = OnlineSynth::new(
                OnlineKind::Dashscope,
                resolver(allowed_base, "dashscope/Cherry"),
            )
            .unwrap()
            .allow_downloads_from(move |url| {
                url.host_str() == Some("127.0.0.1") && url.port() == port
            });
            let speech = allowed.synthesize(&request(None));
            // 没放行的地址（不是官方存储）不下载
            let strict = OnlineSynth::new(OnlineKind::Dashscope, resolver(base, "Cherry")).unwrap();
            (speech, strict.synthesize(&request(None)).unwrap_err())
        })
        .await;
        assert_eq!(speech.unwrap().samples.len(), 3);
        assert!(refused.contains("不是官方"), "{refused}");
        let requests = server.received_requests().await.unwrap();
        let downloads: Vec<_> = requests
            .iter()
            .filter(|r| r.method.as_str() == "GET")
            .collect();
        assert_eq!(downloads.len(), 1, "没放行的那次不该去下载");
        assert!(
            downloads[0].headers.get("authorization").is_none(),
            "下载音频不带 Key"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn dashscope_errors_and_oversized_audio_are_reported_plainly() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(
                "/bad/api/v1/services/aigc/multimodal-generation/generation",
            ))
            .respond_with(ResponseTemplate::new(401).set_body_json(json!({
                "code": "InvalidApiKey", "message": "Invalid API-key provided: secret-key"
            })))
            .mount(&server)
            .await;
        let big_url = format!("{}/audio/big.wav", server.uri());
        Mock::given(method("POST"))
            .and(path(
                "/big/api/v1/services/aigc/multimodal-generation/generation",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "output": {"finish_reason": "stop", "audio": {"url": big_url}}
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/audio/big.wav"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![
                0u8;
                (MAX_AUDIO_DOWNLOAD_BYTES + 10)
                    as usize
            ]))
            .mount(&server)
            .await;
        let (bad, big) = (
            format!("{}/bad", server.uri()),
            format!("{}/big", server.uri()),
        );
        let (bad, big) = blocking(move || {
            let bad = OnlineSynth::new(OnlineKind::Dashscope, resolver(bad, "Cherry"))
                .unwrap()
                .synthesize(&request(None))
                .unwrap_err();
            let big = OnlineSynth::new(OnlineKind::Dashscope, resolver(big, "Cherry"))
                .unwrap()
                .allow_downloads_from(|url| url.host_str() == Some("127.0.0.1"))
                .synthesize(&request(None))
                .unwrap_err();
            (bad, big)
        })
        .await;
        assert!(
            bad.contains("不认这个 Key") && !bad.contains("secret-key"),
            "{bad}"
        );
        assert!(big.contains("太大"), "{big}");
    }

    #[test]
    fn official_audio_hosts_are_upgraded_to_https_and_others_refused() {
        let url = audio_url(
            "http://dashscope-result-bj.oss-cn-beijing.aliyuncs.com/1d/ab/x.wav?Expires=1&Signature=s",
            None,
        )
        .unwrap();
        assert_eq!(url.scheme(), "https");
        assert_eq!(
            url.host_str(),
            Some("dashscope-result-bj.oss-cn-beijing.aliyuncs.com")
        );
        for bad in [
            "https://evil.example/x.wav",
            "https://dashscope-result-bj.oss-cn-beijing.aliyuncs.com.evil.example/x.wav",
            "https://dashscope-result.evil.com/x.aliyuncs.com",
            "file:///C:/x.wav",
        ] {
            assert!(audio_url(bad, None).is_err(), "{bad}");
        }
    }

    #[test]
    fn long_text_is_split_for_dashscope_at_punctuation() {
        let sentence = "这是一句话，".repeat(120); // 720 字
        let parts = split_for_dashscope(&sentence);
        assert!(parts.len() >= 2);
        assert!(parts
            .iter()
            .all(|p| p.chars().count() <= DASHSCOPE_MAX_CHARS));
        assert_eq!(parts.concat(), sentence);
        assert!(parts[0].ends_with('，'));
        assert_eq!(split_for_dashscope("短句"), vec!["短句".to_string()]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn volcano_request_reuses_app_credentials_and_reports_service_errors() {
        let server = MockServer::start().await;
        let audio = base64::engine::general_purpose::STANDARD.encode(wav());
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
        let speech = blocking(move || first.synthesize(&request(None)))
            .await
            .unwrap();
        assert_eq!(speech.samples.len(), 3);
        let body: Value =
            serde_json::from_slice(&server.received_requests().await.unwrap()[0].body).unwrap();
        assert_eq!(body["app"]["appid"], "app-1");
        assert_eq!(body["app"]["cluster"], "volcano_tts");
        assert_eq!(body["audio"]["voice_type"], "BV700_streaming");
        let second = synth.clone();
        let error = blocking(move || second.synthesize(&request(None)))
            .await
            .unwrap_err();
        assert!(error.contains("3001"), "{error}");
        // 选中其他提供方的声音：报错而不是换声音
        let third = synth.clone();
        let error = blocking(move || third.synthesize(&request(Some("openai/alloy"))))
            .await
            .unwrap_err();
        assert!(error.contains("不符"));
        let key = synth.cache_key(&request(None)).unwrap();
        assert!(key.starts_with("volcano/BV700_streaming") && !key.contains("secret-key"));
        blocking(move || drop(synth)).await;
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
        let base = format!("{}/v1", server.uri());
        let speech = blocking(move || {
            OnlineSynth::new(OnlineKind::Deepseek, resolver(base, "deepseek/stella"))
                .unwrap()
                .synthesize(&request(None))
        })
        .await
        .unwrap();
        assert_eq!(speech.samples.len(), 4);
        let body: Value =
            serde_json::from_slice(&server.received_requests().await.unwrap()[0].body).unwrap();
        assert_eq!(body["text"], "队友说先别打龙");
        assert_eq!(body["voice_id"], "stella");
        assert_eq!(body["format"], "wav");
        assert!(body.get("speed").is_none(), "接口没有语速参数");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn checks_use_free_listing_endpoints() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/compatible-mode/v1/models"))
            .and(header("authorization", "Bearer good"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": []})))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/compatible-mode/v1/models"))
            .respond_with(
                ResponseTemplate::new(401).set_body_json(json!({"code": "InvalidApiKey"})),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .and(header("authorization", "Bearer good"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": []})))
            .mount(&server)
            .await;
        let base = server.uri();
        let (ok, refused, silicon) = blocking(move || {
            let config = |key: &str, base_url: String| OnlineConfig {
                base_url,
                api_key: key.into(),
                model: String::new(),
                app_id: String::new(),
                voice: String::new(),
            };
            (
                check(OnlineKind::Dashscope, &config("good", base.clone())),
                check(OnlineKind::Dashscope, &config("bad", base.clone())),
                check(
                    OnlineKind::Siliconflow,
                    &config("good", format!("{base}/v1")),
                ),
            )
        })
        .await;
        assert_eq!(ok, Ok(()));
        assert!(matches!(refused, Err(CheckError::Key(_))), "{refused:?}");
        assert_eq!(silicon, Ok(()));
        // 列模型不花钱：一次合成都没发
        assert!(server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|r| r.method.as_str() == "GET"));
    }

    #[test]
    fn hex_decoding_rejects_bad_input() {
        assert_eq!(decode_hex("00ff7A").unwrap(), vec![0, 255, 122]);
        assert!(decode_hex("abc").is_none());
        assert!(decode_hex("zz").is_none());
    }

    #[test]
    fn site_origins_strip_the_api_version() {
        assert_eq!(
            deepseek_origin("https://api.deepseek.com/v1"),
            "https://api.deepseek.com"
        );
        assert_eq!(
            deepseek_origin("https://api.deepseek.com/"),
            "https://api.deepseek.com"
        );
        assert_eq!(deepseek_origin(""), DEEPSEEK_ORIGIN);
        assert_eq!(
            site_origin("https://api.minimaxi.com/v1", MINIMAX_CN_ORIGIN),
            "https://api.minimaxi.com"
        );
        assert_eq!(site_origin("", MINIMAX_CN_ORIGIN), MINIMAX_CN_ORIGIN);
    }

    #[test]
    fn speed_ratio_is_bounded_and_monotonic() {
        assert!((speed_ratio(0) - 1.0).abs() < 1e-6);
        assert!((speed_ratio(10) - 2.0).abs() < 1e-6);
        assert!((speed_ratio(-10) - 0.5).abs() < 1e-6);
        assert!(speed_ratio(-1) < speed_ratio(1));
    }
}
