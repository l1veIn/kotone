//! OpenAI 兼容的 [`ModelClient`]：`/chat/completions` + `tools`，默认流式（SSE）。
//!
//! - 服务商不支持流式、直接返回 JSON 时自动按非流式解析；
//! - 工具名在接口里只能用 `[A-Za-z0-9_-]{1,64}`，Kotone 的工具名带点（`automation.list`），
//!   每次请求内部建一张对照表，发出前编码、收到后还原；
//! - 错误信息不回显服务商的响应内容与带密钥的地址（同 [`crate::transport`]）。

use std::collections::HashMap;
use std::time::Duration;

use kotone_core::cancel::CancellationToken;
use kotone_core::connection::ResolvedConnection;
use serde_json::{json, Value};

use super::{
    render_tool, render_user, FinishReason, Message, ModelClient, ModelReply, ModelRequest,
    ToolCall,
};
use crate::transport::AiError;

const MAX_RESPONSE_BYTES: usize = 4 * 1_048_576;
const MAX_TOOL_CALLS: usize = 32;
/// 两个数据块之间最长等待（首个数据块同样适用）。
const IDLE_TIMEOUT: Duration = Duration::from_secs(60);

pub struct OpenAiModel {
    client: reqwest::Client,
    connection: ResolvedConnection,
    /// 关闭时发非流式请求（个别服务商的流式工具调用有问题时用）。
    pub stream: bool,
    pub temperature: f64,
    pub max_tokens: u32,
}

impl OpenAiModel {
    pub fn new(client: reqwest::Client, connection: ResolvedConnection) -> Self {
        Self {
            client,
            connection,
            stream: true,
            temperature: 0.3,
            max_tokens: 2048,
        }
    }

    /// 请求体（纯函数，便于测试）。返回请求体与工具名对照表。
    pub fn request_body(&self, request: &ModelRequest) -> (Value, NameMap) {
        let names = NameMap::build(request);
        let mut messages = vec![json!({"role": "system", "content": request.system})];
        for message in &request.messages {
            match message {
                Message::User {
                    text, attachments, ..
                } => messages
                    .push(json!({"role": "user", "content": render_user(text, attachments)})),
                Message::Assistant {
                    text,
                    tool_calls,
                    reasoning,
                    ..
                } => {
                    let mut m = json!({
                        "role": "assistant",
                        "content": if text.is_empty() && !tool_calls.is_empty() { Value::Null } else { json!(text) },
                    });
                    if !tool_calls.is_empty() {
                        m["tool_calls"] = tool_calls
                            .iter()
                            .map(|c| {
                                json!({
                                    "id": c.id,
                                    "type": "function",
                                    "function": {"name": names.wire(&c.name), "arguments": if c.arguments.trim().is_empty() { "{}" } else { &c.arguments }},
                                })
                            })
                            .collect();
                    }
                    if !reasoning.is_empty() {
                        m["reasoning_content"] = json!(reasoning);
                    }
                    messages.push(m);
                }
                Message::Tool {
                    call_id,
                    content,
                    is_error,
                    untrusted,
                    ..
                } => messages.push(json!({
                    "role": "tool",
                    "tool_call_id": call_id,
                    "content": render_tool(content, *is_error, *untrusted),
                })),
                Message::Notice { .. } => {}
            }
        }
        let mut body = json!({
            "model": self.connection.model,
            "messages": messages,
            "temperature": self.temperature,
            "max_tokens": self.max_tokens,
            "stream": self.stream,
        });
        if !request.tools.is_empty() {
            body["tools"] = request
                .tools
                .iter()
                .map(|t| {
                    json!({
                        "type": "function",
                        "function": {"name": names.wire(&t.name), "description": t.description, "parameters": t.parameters},
                    })
                })
                .collect();
            body["tool_choice"] = json!("auto");
        }
        if self.connection.provider.starts_with("dashscope") {
            body["enable_thinking"] = json!(false);
        }
        (body, names)
    }
}

impl ModelClient for OpenAiModel {
    async fn complete(
        &self,
        request: &ModelRequest,
        on_text: &(dyn Fn(&str) + Send + Sync),
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
            return Err(AiError::Failed("AI 连接尚未保存密钥".into()));
        }
        let (body, names) = self.request_body(request);
        let work = async {
            let response = crate::transport::send(
                &self.client,
                &self.connection.base_url,
                self.connection.api_key.as_deref(),
                &body,
            )
            .await?;
            let is_json = response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| v.contains("application/json"));
            let mut reply = if is_json {
                let bytes = read_bounded(response).await?;
                let value: Value = serde_json::from_slice(&bytes)
                    .map_err(|_| AiError::Failed("AI 响应不是有效 JSON".into()))?;
                let reply = parse_reply(&value)?;
                if !reply.text.is_empty() {
                    on_text(&reply.text);
                }
                reply
            } else {
                read_stream(response, on_text).await?
            };
            for call in &mut reply.tool_calls {
                call.name = names.local(&call.name);
            }
            Ok(reply)
        };
        tokio::select! {
            biased;
            _ = cancel.cancelled() => Err(AiError::Cancelled),
            result = work => result,
        }
    }
}

async fn next_chunk(response: &mut reqwest::Response) -> Result<Option<Vec<u8>>, AiError> {
    match tokio::time::timeout(IDLE_TIMEOUT, response.chunk()).await {
        Err(_) => Err(AiError::Failed("AI 响应超时".into())),
        Ok(Err(_)) => Err(AiError::Failed("读取 AI 响应失败".into())),
        Ok(Ok(chunk)) => Ok(chunk.map(|c| c.to_vec())),
    }
}

async fn read_bounded(mut response: reqwest::Response) -> Result<Vec<u8>, AiError> {
    let mut bytes = Vec::new();
    while let Some(chunk) = next_chunk(&mut response).await? {
        if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err(AiError::Failed("AI 响应超出大小限制".into()));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

/// 非流式响应。
pub fn parse_reply(value: &Value) -> Result<ModelReply, AiError> {
    if value.get("error").is_some_and(|e| !e.is_null()) {
        return Err(AiError::Failed("AI 接口返回错误".into()));
    }
    let choice = value
        .pointer("/choices/0")
        .ok_or_else(|| AiError::Failed("AI 没有返回结果".into()))?;
    let message = choice
        .get("message")
        .ok_or_else(|| AiError::Failed("AI 没有返回消息".into()))?;
    let text = message
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let reasoning = message
        .get("reasoning_content")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let mut tool_calls = Vec::new();
    match message.get("tool_calls") {
        None | Some(Value::Null) => {}
        Some(Value::Array(calls)) => {
            if calls.len() > MAX_TOOL_CALLS {
                return Err(AiError::Failed("AI 一次返回的工具调用过多".into()));
            }
            for (index, call) in calls.iter().enumerate() {
                let function = call
                    .get("function")
                    .ok_or_else(|| AiError::Failed("AI 工具调用格式不正确".into()))?;
                let arguments = match function.get("arguments") {
                    Some(Value::String(s)) => s.clone(),
                    None | Some(Value::Null) => String::new(),
                    // 个别服务商直接给对象。
                    Some(other) => other.to_string(),
                };
                tool_calls.push(ToolCall {
                    id: call
                        .get("id")
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string)
                        .unwrap_or_else(|| format!("call_{index}")),
                    name: function
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    arguments,
                });
            }
        }
        Some(_) => return Err(AiError::Failed("AI 工具调用格式不正确".into())),
    }
    Ok(ModelReply {
        text,
        reasoning,
        tool_calls,
        finish: finish_reason(choice.get("finish_reason")),
    })
}

fn finish_reason(value: Option<&Value>) -> FinishReason {
    match value.and_then(Value::as_str) {
        Some("stop") => FinishReason::Stop,
        Some("tool_calls") | Some("function_call") => FinishReason::ToolCalls,
        Some("length") => FinishReason::Length,
        _ => FinishReason::Other,
    }
}

/// 流式响应的累积器（纯逻辑，便于测试）。
#[derive(Default)]
pub struct StreamState {
    text: String,
    reasoning: String,
    calls: Vec<ToolCall>,
    finish: Option<FinishReason>,
    done: bool,
    seen: bool,
    buffer: Vec<u8>,
}

impl StreamState {
    /// 喂一段原始字节；正文片段经 `on_text` 报告。
    pub fn feed(&mut self, bytes: &[u8], on_text: &dyn Fn(&str)) -> Result<(), AiError> {
        self.buffer.extend_from_slice(bytes);
        while let Some(pos) = self.buffer.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.buffer.drain(..=pos).collect();
            self.line(&line, on_text)?;
            if self.done {
                return Ok(());
            }
        }
        Ok(())
    }

    fn line(&mut self, raw: &[u8], on_text: &dyn Fn(&str)) -> Result<(), AiError> {
        let line = String::from_utf8_lossy(raw);
        let line = line.trim();
        let Some(payload) = line.strip_prefix("data:") else {
            return Ok(());
        };
        let payload = payload.trim();
        if payload == "[DONE]" {
            self.done = true;
            return Ok(());
        }
        let value: Value = serde_json::from_str(payload)
            .map_err(|_| AiError::Failed("AI 流式响应格式不正确".into()))?;
        if value.get("error").is_some_and(|e| !e.is_null()) {
            return Err(AiError::Failed("AI 接口返回错误".into()));
        }
        self.seen = true;
        let Some(choice) = value.pointer("/choices/0") else {
            return Ok(()); // 如只含 usage 的末尾块
        };
        if let Some(delta) = choice.get("delta") {
            if let Some(text) = delta.get("content").and_then(Value::as_str) {
                if !text.is_empty() {
                    self.text.push_str(text);
                    on_text(text);
                }
            }
            if let Some(text) = delta.get("reasoning_content").and_then(Value::as_str) {
                self.reasoning.push_str(text);
            }
            if let Some(Value::Array(parts)) = delta.get("tool_calls") {
                for part in parts {
                    let index = part
                        .get("index")
                        .and_then(Value::as_u64)
                        .map(|i| i as usize)
                        .unwrap_or(self.calls.len().saturating_sub(1));
                    if index >= MAX_TOOL_CALLS {
                        return Err(AiError::Failed("AI 一次返回的工具调用过多".into()));
                    }
                    while self.calls.len() <= index {
                        let n = self.calls.len();
                        self.calls.push(ToolCall {
                            id: format!("call_{n}"),
                            name: String::new(),
                            arguments: String::new(),
                        });
                    }
                    let call = &mut self.calls[index];
                    if let Some(id) = part
                        .get("id")
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())
                    {
                        call.id = id.to_string();
                    }
                    if let Some(function) = part.get("function") {
                        if let Some(name) = function.get("name").and_then(Value::as_str) {
                            call.name.push_str(name);
                        }
                        if let Some(args) = function.get("arguments").and_then(Value::as_str) {
                            call.arguments.push_str(args);
                        }
                    }
                }
            }
        }
        if choice.get("finish_reason").is_some_and(|f| !f.is_null()) {
            self.finish = Some(finish_reason(choice.get("finish_reason")));
        }
        Ok(())
    }

    pub fn finish(mut self, on_text: &dyn Fn(&str)) -> Result<ModelReply, AiError> {
        if !self.done && !self.buffer.is_empty() {
            let rest = std::mem::take(&mut self.buffer);
            self.line(&rest, on_text)?;
        }
        if !self.seen {
            return Err(AiError::Failed("AI 没有返回结果".into()));
        }
        if !self.done && self.finish.is_none() {
            return Err(AiError::Failed("AI 响应中断".into()));
        }
        Ok(ModelReply {
            text: self.text,
            reasoning: self.reasoning,
            tool_calls: self.calls,
            finish: self.finish.unwrap_or(FinishReason::Other),
        })
    }
}

async fn read_stream(
    mut response: reqwest::Response,
    on_text: &(dyn Fn(&str) + Send + Sync),
) -> Result<ModelReply, AiError> {
    let mut state = StreamState::default();
    let mut total = 0usize;
    while let Some(chunk) = next_chunk(&mut response).await? {
        total = total.saturating_add(chunk.len());
        if total > MAX_RESPONSE_BYTES {
            return Err(AiError::Failed("AI 响应超出大小限制".into()));
        }
        state.feed(&chunk, on_text)?;
        if state.done {
            break;
        }
    }
    state.finish(on_text)
}

/// Kotone 工具名 ↔ 接口里的函数名。
#[derive(Debug, Default)]
pub struct NameMap {
    to_wire: HashMap<String, String>,
    to_local: HashMap<String, String>,
}

impl NameMap {
    fn build(request: &ModelRequest) -> Self {
        let mut map = Self::default();
        for tool in &request.tools {
            map.insert(&tool.name);
        }
        for message in &request.messages {
            if let Message::Assistant { tool_calls, .. } = message {
                for call in tool_calls {
                    map.insert(&call.name);
                }
            }
        }
        map
    }

    fn insert(&mut self, name: &str) {
        if self.to_wire.contains_key(name) {
            return;
        }
        let base: String = name
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .take(60)
            .collect();
        let base = if base.is_empty() { "tool".into() } else { base };
        let mut wire = base.clone();
        let mut n = 2;
        while self.to_local.contains_key(&wire) {
            wire = format!("{base}_{n}");
            n += 1;
        }
        self.to_wire.insert(name.to_string(), wire.clone());
        self.to_local.insert(wire, name.to_string());
    }

    pub fn wire(&self, name: &str) -> String {
        self.to_wire
            .get(name)
            .cloned()
            .unwrap_or_else(|| name.to_string())
    }

    /// 未知的函数名原样返回（循环会把它当作不存在的工具拒绝）。
    pub fn local(&self, wire: &str) -> String {
        self.to_local
            .get(wire)
            .cloned()
            .unwrap_or_else(|| wire.to_string())
    }
}
