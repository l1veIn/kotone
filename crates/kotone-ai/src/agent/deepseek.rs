//! DeepSeek 连接用的 Chat Completions 客户端（ADR 026）。其余服务商仍走 rig（[`super::model`]）。
//!
//! 照 DeepSeek 官方指南「构建 Chatbot」（https://api-docs.deepseek.com/zh-cn/guides/build_a_chatbot）：
//! - 每条玩家消息前一条 `latest_reminder`（日期、星期、平台、语言），历史里的照样带回；
//! - 还没有标题时请求 `title`，拿到就不再要；
//! - `safety_check` 开着时，审核拒绝（`finish_reason: "content_filter"`）返回 [`FinishReason::Refused`]，
//!   由循环撤回已经显示的半句；
//! - 思考模式：`reasoning_content` 每一轮原样带回；不传 `temperature`（思考模式下无效）；强度用 `reasoning_effort`；
//! - 错误：最后一块带 `error`（HTTP 已经是 200）、`: keep-alive` 心跳、429 / 5xx 还没出字时重发、402 余额不足。
//!
//! rig 的 OpenAI 兼容格式表达不了 `latest_reminder` 角色，也读不到 `title` 和最后一块的 `error`，所以自己写。
//! 只做转换和流式解析；工具名编码沿用 [`NameMap`]。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use futures::StreamExt;
use kotone_core::cancel::CancellationToken;
use kotone_core::connection::{ConnectionKind, ResolvedConnection};
use serde_json::{json, Map, Value};

use super::model::NameMap;
use super::{
    render_tool, render_user, Attachment, Delta, FinishReason, Message, ModelClient, ModelReply,
    ModelRequest, TokenUsage, ToolCall,
};
use crate::transport::AiError;

/// 两个数据块之间最长等待（首个数据块同样适用）。心跳行不算新内容。
const IDLE_TIMEOUT: Duration = Duration::from_secs(60);
/// 还没出字时，429 / 5xx / 连接失败 / 最后一块报错后重发的等待（最多重发这么多次）。
const RETRY_DELAYS: [Duration; 2] = [Duration::from_secs(1), Duration::from_secs(3)];
/// `latest_reminder` 里的平台与语言（Kotone 是桌面 App，界面是中文）。
const REMINDER_TAIL: &str = ",,App,中文";

/// 这个连接是不是 DeepSeek 官方接口。
pub fn is_deepseek(connection: &ResolvedConnection) -> bool {
    connection.provider == "deepseek" || connection.base_url.contains("api.deepseek.com")
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub struct BalanceAmount {
    pub currency: String,
    #[serde(rename(deserialize = "total_balance"))]
    pub amount: String,
}

#[derive(serde::Deserialize)]
struct BalanceResponse {
    balance_infos: Vec<BalanceAmount>,
}

impl BalanceResponse {
    fn amounts(self) -> Result<Vec<BalanceAmount>, String> {
        let balances = self.balance_infos;
        let mut currencies = std::collections::HashSet::new();
        if balances.is_empty()
            || !balances.iter().all(|value| {
                matches!(value.currency.as_str(), "CNY" | "USD")
                    && currencies.insert(value.currency.as_str())
                    && value.amount.parse::<f64>().is_ok_and(f64::is_finite)
            })
        {
            return Err("DeepSeek 返回的余额格式不正确".into());
        }
        Ok(balances)
    }
}

fn official_balance_origin(base_url: &str) -> bool {
    // 对话客户端还兼容自定义网关。余额只使用配置在 DeepSeek 官方域名上的 Key。
    reqwest::Url::parse(base_url.trim()).is_ok_and(|url| {
        url.scheme() == "https"
            && url.host_str() == Some("api.deepseek.com")
            && url.port().is_none()
            && url.username().is_empty()
            && url.password().is_none()
    })
}

/// 从 DeepSeek 官方的账户余额接口取金额，保留它返回的十进制精度。
pub async fn account_balance(
    connection: &ResolvedConnection,
) -> Result<Option<Vec<BalanceAmount>>, String> {
    if connection.kind != ConnectionKind::Remote || !official_balance_origin(&connection.base_url) {
        return Ok(None);
    }
    let Some(key) = connection
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|key| !key.is_empty())
    else {
        return Ok(None);
    };
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "无法连接 DeepSeek 查询余额")?;
    let response = client
        .get("https://api.deepseek.com/user/balance")
        .bearer_auth(key)
        .send()
        .await
        .map_err(|_| "无法连接 DeepSeek 查询余额")?;
    if !response.status().is_success() {
        return Err(format!(
            "DeepSeek 余额查询失败（HTTP {}）",
            response.status().as_u16()
        ));
    }
    let payload = response
        .json::<BalanceResponse>()
        .await
        .map_err(|_| "无法读取 DeepSeek 余额")?;
    payload.amounts().map(Some)
}

fn client() -> reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(15))
                .build()
                .unwrap_or_default()
        })
        .clone()
}

/// 一段还没有标题的对话：请求时带上 `title`，拿到后交给 `sink`，之后不再要。
pub struct TitleRequest {
    wanted: AtomicBool,
    sink: Box<dyn Fn(String) + Send + Sync>,
}

impl TitleRequest {
    pub fn new(sink: impl Fn(String) + Send + Sync + 'static) -> Arc<Self> {
        Arc::new(Self {
            wanted: AtomicBool::new(true),
            sink: Box::new(sink),
        })
    }

    fn wanted(&self) -> bool {
        self.wanted.load(Ordering::Relaxed)
    }

    fn deliver(&self, title: &str) {
        let title = title.trim();
        if !title.is_empty() && self.wanted.swap(false, Ordering::Relaxed) {
            (self.sink)(title.to_string());
        }
    }
}

pub struct DeepSeekModel {
    connection: ResolvedConnection,
    pub max_tokens: u64,
    /// `low` / `high` / `max`；`none` = 关掉思考；None = 服务商默认（思考开、high）。
    pub reasoning_effort: Option<String>,
    pub safety_check: bool,
    pub title: Option<Arc<TitleRequest>>,
}

impl DeepSeekModel {
    pub fn new(connection: ResolvedConnection) -> Self {
        Self {
            connection,
            max_tokens: 8192,
            reasoning_effort: None,
            safety_check: false,
            title: None,
        }
    }

    fn url(&self) -> String {
        crate::transport::completions_url(&self.connection.base_url)
    }

    /// 请求体（纯函数，便于测试）。返回请求体与工具名对照表。
    pub fn body(&self, request: &ModelRequest) -> (Value, NameMap) {
        let names = NameMap::build(request);
        let mut messages = vec![json!({"role": "system", "content": request.system})];
        // 工具带回的图片：工具结果只能是文字，一组工具结果结束后再补一条带图片的用户消息（同 rig 版本）。
        let mut pending: Vec<Value> = Vec::new();
        for (i, message) in request.messages.iter().enumerate() {
            match message {
                Message::User {
                    text,
                    attachments,
                    context,
                    at_ms,
                    ..
                } => {
                    let date = kotone_core::agent::reminder_date(*at_ms);
                    if !date.is_empty() {
                        messages.push(json!({
                            "role": "latest_reminder",
                            "content": format!("{date}{REMINDER_TAIL}"),
                        }));
                    }
                    let text = render_user(context, text, attachments);
                    let images = image_parts(attachments);
                    messages.push(if images.is_empty() {
                        json!({"role": "user", "content": text})
                    } else {
                        let mut parts = vec![json!({"type": "text", "text": text})];
                        parts.extend(images);
                        json!({"role": "user", "content": parts})
                    });
                }
                Message::Assistant {
                    text,
                    tool_calls,
                    reasoning,
                    ..
                } => {
                    if text.is_empty() && tool_calls.is_empty() && reasoning.is_empty() {
                        continue;
                    }
                    let mut m = Map::new();
                    m.insert("role".into(), json!("assistant"));
                    m.insert("content".into(), json!(text));
                    // 开了工具时每一轮的推理都要带回，否则 400（官方「思考模式」）。
                    m.insert("reasoning_content".into(), json!(reasoning));
                    if !tool_calls.is_empty() {
                        let calls: Vec<Value> = tool_calls
                            .iter()
                            .map(|c| {
                                let arguments = if c.arguments.trim().is_empty() {
                                    "{}".to_string()
                                } else {
                                    c.arguments.clone()
                                };
                                json!({"id": c.id, "type": "function",
                                       "function": {"name": names.wire(&c.name), "arguments": arguments}})
                            })
                            .collect();
                        m.insert("tool_calls".into(), Value::Array(calls));
                    }
                    messages.push(Value::Object(m));
                }
                Message::Tool {
                    call_id,
                    name,
                    content,
                    is_error,
                    untrusted,
                    images,
                    ..
                } => {
                    messages.push(json!({
                        "role": "tool",
                        "tool_call_id": call_id,
                        "content": render_tool(content, *is_error, *untrusted),
                    }));
                    let note = super::render_tool_images(name, images);
                    if !note.is_empty() {
                        pending.push(json!({"type": "text", "text": note}));
                    }
                    pending.extend(image_parts(images));
                }
                Message::Notice { .. } => {}
            }
            let group_ends = !matches!(request.messages.get(i + 1), Some(Message::Tool { .. }));
            if group_ends && !pending.is_empty() {
                messages.push(json!({"role": "user", "content": std::mem::take(&mut pending)}));
            }
        }
        let mut body = json!({
            "model": self.connection.model,
            "messages": messages,
            "stream": true,
            "max_tokens": self.max_tokens,
        });
        if !request.tools.is_empty() {
            body["tools"] = Value::Array(
                request
                    .tools
                    .iter()
                    .map(|t| {
                        json!({"type": "function", "function": {
                            "name": names.wire(&t.name),
                            "description": t.description,
                            "parameters": t.parameters,
                        }})
                    })
                    .collect(),
            );
        }
        match self.reasoning_effort.as_deref() {
            Some("none") => body["thinking"] = json!({"type": "disabled"}),
            Some(effort) if !effort.is_empty() => body["reasoning_effort"] = json!(effort),
            _ => {}
        }
        if self.safety_check {
            body["safety_check"] = json!({"level": "high"});
        }
        if self.title.as_ref().is_some_and(|t| t.wanted()) {
            body["title"] = json!({"enabled": true});
        }
        (body, names)
    }

    /// 发一次请求、读完整个流。`emitted` 记下是否已经流出过文字（出过字就不再重发）。
    async fn attempt(
        &self,
        body: &Value,
        names: &NameMap,
        on_delta: &(dyn Fn(Delta<'_>) + Send + Sync),
        emitted: &AtomicBool,
    ) -> Result<(ModelReply, Option<String>), Attempt> {
        let key = self.connection.api_key.clone().unwrap_or_default();
        let response = client()
            .post(self.url())
            .bearer_auth(key.trim())
            .json(body)
            .send()
            .await
            .map_err(|_| Attempt::Retry("连不上 DeepSeek，请检查网络".into()))?;
        let status = response.status().as_u16();
        if status != 200 {
            let text = response.text().await.unwrap_or_default();
            return Err(http_error(status, &text));
        }
        let mut stream = response.bytes_stream();
        let mut parser = StreamParser::default();
        let mut buffer: Vec<u8> = Vec::new();
        let mut last_data = tokio::time::Instant::now();
        loop {
            let remaining = IDLE_TIMEOUT.saturating_sub(last_data.elapsed());
            let next = match tokio::time::timeout(remaining, stream.next()).await {
                Ok(next) => next,
                Err(_) => return Err(Attempt::fail_or_retry(emitted, "AI 响应超时")),
            };
            let Some(chunk) = next else { break };
            let chunk = chunk.map_err(|_| Attempt::fail_or_retry(emitted, "AI 响应中断"))?;
            buffer.extend_from_slice(&chunk);
            while let Some(end) = buffer.iter().position(|&b| b == b'\n') {
                let line: Vec<u8> = buffer.drain(..=end).collect();
                let line = String::from_utf8_lossy(&line);
                let line = line.trim_end_matches(['\r', '\n']);
                // 心跳（`: keep-alive`）与空行不算新内容。
                let Some(data) = line.strip_prefix("data:") else {
                    continue;
                };
                last_data = tokio::time::Instant::now();
                match parser.feed(data.trim(), names, on_delta, emitted) {
                    Ok(true) => break,
                    Ok(false) => {}
                    Err(message) => return Err(Attempt::fail_or_retry(emitted, &message)),
                }
            }
            if parser.done {
                break;
            }
        }
        if !parser.done && parser.finish.is_none() {
            return Err(Attempt::fail_or_retry(emitted, "AI 响应中断"));
        }
        let title = parser.title.take();
        Ok((parser.into_reply(names, &self.connection), title))
    }
}

/// 一次尝试的失败：还没出字、可以重发的，和不该重发的。
enum Attempt {
    Retry(String),
    Fail(String),
}

impl Attempt {
    fn fail_or_retry(emitted: &AtomicBool, message: &str) -> Self {
        if emitted.load(Ordering::Relaxed) {
            Self::Fail(message.into())
        } else {
            Self::Retry(message.into())
        }
    }
}

/// HTTP 错误换成给玩家看的话（不回显服务商的响应体）。
fn http_error(status: u16, body: &str) -> Attempt {
    match status {
        400 if body.contains("Content Exists Risk") => Attempt::Fail(
            "DeepSeek 内容审核没通过：这段对话里有不允许的内容，换个说法或开一段新对话".into(),
        ),
        400 | 422 => Attempt::Fail(format!("AI 请求格式不正确（HTTP {status}）")),
        401 | 403 => Attempt::Fail(format!("DeepSeek Key 无效或无权限（HTTP {status}）")),
        402 => Attempt::Fail("DeepSeek 账户余额不足，请先充值".into()),
        429 => Attempt::Retry("DeepSeek 请求太频繁，请稍后再试".into()),
        500..=599 => Attempt::Retry(format!(
            "DeepSeek 服务暂时不可用（HTTP {status}），请稍后再试"
        )),
        _ => Attempt::Fail(format!("AI 接口返回 HTTP {status}")),
    }
}

fn image_parts(attachments: &[Attachment]) -> Vec<Value> {
    attachments
        .iter()
        .filter_map(|a| match a {
            Attachment::Image { mime, data, .. } if !data.is_empty() => Some(json!({
                "type": "image_url",
                "image_url": {"url": format!("data:{mime};base64,{data}")},
            })),
            _ => None,
        })
        .collect()
}

/// 流式数据块的累积。
#[derive(Default)]
struct StreamParser {
    text: String,
    reasoning: String,
    /// 按 `index` 拼接的工具调用：(id, 名字, 参数)。
    calls: Vec<(String, String, String)>,
    finish: Option<String>,
    usage: Option<Value>,
    title: Option<String>,
    done: bool,
}

impl StreamParser {
    /// 处理一行 `data:`。返回 `Ok(true)` = 流结束（`[DONE]`）；`Err` = 最后一块报错。
    fn feed(
        &mut self,
        data: &str,
        names: &NameMap,
        on_delta: &(dyn Fn(Delta<'_>) + Send + Sync),
        emitted: &AtomicBool,
    ) -> Result<bool, String> {
        if data == "[DONE]" {
            self.done = true;
            return Ok(true);
        }
        let Ok(chunk) = serde_json::from_str::<Value>(data) else {
            return Ok(false);
        };
        // HTTP 已经是 200、生成中途失败：最后一块只有 error（和 usage），没有 [DONE]。
        if chunk.get("error").is_some_and(|e| !e.is_null()) {
            return Err("AI 生成中途失败，请重试".into());
        }
        if let Some(usage) = chunk.get("usage").filter(|u| !u.is_null()) {
            self.usage = Some(usage.clone());
        }
        let Some(choice) = chunk.pointer("/choices/0") else {
            return Ok(false);
        };
        if let Some(delta) = choice.get("delta") {
            if let Some(t) = delta.get("reasoning_content").and_then(Value::as_str) {
                if !t.is_empty() {
                    self.reasoning.push_str(t);
                    on_delta(Delta::Thought(t));
                }
            }
            if let Some(t) = delta.get("content").and_then(Value::as_str) {
                if !t.is_empty() {
                    self.text.push_str(t);
                    emitted.store(true, Ordering::Relaxed);
                    on_delta(Delta::Text(t));
                }
            }
            if let Some(t) = delta.get("title").and_then(Value::as_str) {
                self.title = Some(t.to_string());
            }
            if let Some(calls) = delta.get("tool_calls").and_then(Value::as_array) {
                for call in calls {
                    let index = call.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                    while self.calls.len() <= index {
                        self.calls.push(Default::default());
                    }
                    let slot = &mut self.calls[index];
                    if let Some(id) = call.get("id").and_then(Value::as_str) {
                        slot.0.push_str(id);
                    }
                    if let Some(name) = call.pointer("/function/name").and_then(Value::as_str) {
                        slot.1.push_str(name);
                    }
                    if let Some(args) = call.pointer("/function/arguments").and_then(Value::as_str)
                    {
                        slot.2.push_str(args);
                        if !args.is_empty() && !slot.1.is_empty() {
                            // 参数一段一段到：交出去好边生成边念 `speak`（ADR 029 第八节）。
                            let name = names.local(&slot.1);
                            on_delta(Delta::ToolArgs {
                                index,
                                id: &slot.0,
                                name: &name,
                                chunk: args,
                            });
                        }
                    }
                }
            }
        }
        if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
            self.finish = Some(reason.to_string());
        }
        Ok(false)
    }

    fn into_reply(self, names: &NameMap, connection: &ResolvedConnection) -> ModelReply {
        let finish = match self.finish.as_deref() {
            Some("stop") => FinishReason::Stop,
            Some("tool_calls") => FinishReason::ToolCalls,
            Some("length") => FinishReason::Length,
            Some("content_filter") => FinishReason::Refused,
            _ => FinishReason::Other,
        };
        let tool_calls = self
            .calls
            .into_iter()
            .filter(|(_, name, _)| !name.is_empty())
            .map(|(id, name, arguments)| ToolCall {
                id,
                name: names.local(&name),
                arguments,
            })
            .collect();
        let usage = self.usage.map(|u| {
            let n = |p: &str| u.pointer(p).and_then(Value::as_u64);
            let input = n("/prompt_tokens");
            let cached =
                n("/prompt_cache_hit_tokens").or(n("/prompt_tokens_details/cached_tokens"));
            let output = n("/completion_tokens");
            let price = kotone_core::usage::price_for(
                &connection.provider,
                &connection.model,
                super::now_ms(),
            );
            TokenUsage {
                input_tokens: input,
                output_tokens: output,
                cached_input_tokens: cached,
                reasoning_tokens: n("/completion_tokens_details/reasoning_tokens"),
                cost_usd: None,
                cost: price
                    .as_ref()
                    .and_then(|p| kotone_core::usage::estimate(p, input, cached, output)),
                currency: price.map(|p| p.currency.to_string()),
            }
        });
        ModelReply {
            text: self.text,
            reasoning: self.reasoning,
            tool_calls,
            finish,
            usage,
        }
    }
}

impl ModelClient for DeepSeekModel {
    async fn complete(
        &self,
        request: &ModelRequest,
        on_delta: &(dyn Fn(Delta<'_>) + Send + Sync),
        cancel: &CancellationToken,
    ) -> Result<ModelReply, AiError> {
        if self
            .connection
            .api_key
            .as_deref()
            .unwrap_or("")
            .trim()
            .is_empty()
        {
            return Err(AiError::Failed("API 连接还没有保存 Key".into()));
        }
        let (body, names) = self.body(request);
        let emitted = AtomicBool::new(false);
        let work = async {
            let mut delays = RETRY_DELAYS.iter();
            loop {
                match self.attempt(&body, &names, on_delta, &emitted).await {
                    Ok((reply, title)) => {
                        if let (Some(t), Some(request)) = (title, &self.title) {
                            request.deliver(&t);
                        }
                        return Ok(reply);
                    }
                    Err(Attempt::Fail(message)) => return Err(AiError::Failed(message)),
                    Err(Attempt::Retry(message)) => match delays.next() {
                        Some(delay) => tokio::time::sleep(*delay).await,
                        None => return Err(AiError::Failed(message)),
                    },
                }
            }
        };
        tokio::select! {
            biased;
            _ = cancel.cancelled() => Err(AiError::Cancelled),
            result = work => result,
        }
    }
}

#[cfg(test)]
mod balance_tests {
    use super::*;

    #[test]
    fn account_balance_accepts_only_the_official_https_origin() {
        for base_url in [
            "https://api.deepseek.com",
            "https://api.deepseek.com/v1/",
            "https://api.deepseek.com/anthropic",
        ] {
            assert!(official_balance_origin(base_url), "{base_url}");
        }
        for base_url in [
            "http://api.deepseek.com/v1",
            "https://api.deepseek.com:8443/v1",
            "https://api.deepseek.com.example.com/v1",
            "https://example.com/api.deepseek.com/v1",
            "https://api.deepseek.com@example.com/v1",
            "https://example.com@api.deepseek.com/v1",
        ] {
            assert!(!official_balance_origin(base_url), "{base_url}");
        }
    }

    #[test]
    fn account_balance_preserves_zero_and_decimal_precision() {
        let payload: BalanceResponse = serde_json::from_value(json!({
            "is_available": false,
            "balance_infos": [
                {"currency": "CNY", "total_balance": "0.00", "granted_balance": "0.00"},
                {"currency": "USD", "total_balance": "0.012345678901234567"}
            ]
        }))
        .unwrap();
        assert_eq!(
            serde_json::to_value(payload.amounts().unwrap()).unwrap(),
            json!([
                {"currency": "CNY", "amount": "0.00"},
                {"currency": "USD", "amount": "0.012345678901234567"}
            ])
        );
    }

    #[test]
    fn account_balance_rejects_missing_invalid_and_duplicate_amounts() {
        for balances in [
            json!([]),
            json!([{"currency": "CNY", "total_balance": "NaN"}]),
            json!([{"currency": "EUR", "total_balance": "0.00"}]),
            json!([
                {"currency": "USD", "total_balance": "1.00"},
                {"currency": "USD", "total_balance": "2.00"}
            ]),
        ] {
            let payload: BalanceResponse =
                serde_json::from_value(json!({"balance_infos": balances})).unwrap();
            assert!(payload.amounts().is_err(), "{balances}");
        }
    }
}

#[cfg(test)]
mod stream_tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn tool_call_arguments_are_streamed_with_the_local_name() {
        let names = NameMap::default();
        let emitted = AtomicBool::new(false);
        let seen: Mutex<Vec<(usize, String, String, String)>> = Mutex::default();
        let on_delta = |d: Delta<'_>| {
            if let Delta::ToolArgs {
                index,
                id,
                name,
                chunk,
            } = d
            {
                seen.lock()
                    .unwrap()
                    .push((index, id.into(), name.into(), chunk.into()));
            }
        };
        let mut parser = StreamParser::default();
        for line in [
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c1","function":{"name":"speak","arguments":""}}]}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"text\":\"好"}}]}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"的\"}"}}]}}]}"#,
            r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
        ] {
            parser.feed(line, &names, &on_delta, &emitted).unwrap();
        }
        let seen = seen.into_inner().unwrap();
        assert_eq!(seen.len(), 2, "空的参数片段不交出去");
        assert_eq!(
            seen[0],
            (0, "c1".into(), "speak".into(), r#"{"text":"好"#.into())
        );
        assert_eq!(seen[1].3, r#"的"}"#);
        assert!(!emitted.load(Ordering::Relaxed), "工具参数不影响出错重发");
    }
}
