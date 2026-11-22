//! [`ModelClient`] 的 rig-core 实现（ADR 015）：OpenAI 兼容的 `/chat/completions`，默认流式。
//!
//! rig-core 负责：各家的流式格式与工具调用拼装、推理内容（DeepSeek 等要带回的
//! `reasoning_content`）、图片编码、用量统计、按模型价格表估算费用。本文件只做转换：
//! Kotone 的 [`Message`] ↔ rig 的消息，以及错误改写。rig 是 0.x，升级时只改这里。
//!
//! - 接口地址沿用 Kotone 连接的约定（`base_url` 含 `/v1`，请求 `{base_url}/chat/completions`），
//!   所有连接用同一个“OpenAI 兼容网关”方言；DeepSeek 的推理字段由 rig 按地址识别；
//! - 工具名在接口里只能用 `[A-Za-z0-9_-]`，Kotone 的工具名带点（`automation.list`），
//!   每次请求内部建一张对照表，发出前编码、收到后还原；
//! - 错误信息不回显服务商的响应内容与带密钥的地址（rig 的错误文本会带响应体，一律改写）。

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use futures::StreamExt;
use kotone_core::cancel::CancellationToken;
use kotone_core::connection::ResolvedConnection;
use rig_core::completion::{CompletionRequest, CompletionResponse, ToolDefinition};
use rig_core::message::{
    AssistantContent, AssistantMessage, CallId, ImageMediaType, Message as RigMessage, ToolName,
    ToolResult, ToolResultContent, UserContent,
};
use rig_core::providers::openai::wire::{Chat, Dialect};
use rig_core::providers::openai::OpenAIConfig;
use rig_core::streaming::{Item, StreamEvent};
use rig_core::{Model, ProviderError};
use rig_reqwest::ReqwestClient;
use serde_json::{json, Value};

use super::{
    render_tool, render_user, Attachment, Delta, FinishReason, Message, ModelClient, ModelReply,
    ModelRequest, TokenUsage, ToolCall,
};
use crate::transport::AiError;

/// 所有 Kotone 连接共用的方言：OpenAI 兼容网关的基线行为，路径 `/chat/completions`。
const COMPATIBLE: Dialect = Dialect::gateway("openai-compatible", "https://api.openai.com/v1", "");

/// 流式回复两个数据块之间最长等待（首个数据块同样适用）。
const IDLE_TIMEOUT: Duration = Duration::from_secs(60);

/// 进程内共用的 HTTP 客户端（连接复用）。reqwest 0.13 用进程默认的 rustls 加密提供者：
/// 这里装上 ring（已经装过别的就沿用），不引入 aws-lc。
fn transport() -> ReqwestClient {
    static CLIENT: OnceLock<ReqwestClient> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            let _ = rustls::crypto::ring::default_provider().install_default();
            match reqwest13::Client::builder().build() {
                Ok(client) => ReqwestClient::from(client),
                // 建不出来时，rig 会在每次发送时报告构建错误。
                Err(_) => ReqwestClient::default(),
            }
        })
        .clone()
}

pub struct OpenAiModel {
    model: Model<Chat, ReqwestClient>,
    connection: ResolvedConnection,
    /// 关闭时发非流式请求（个别服务商的流式工具调用有问题时用）。
    pub stream: bool,
    pub temperature: f64,
    pub max_tokens: u64,
}

impl OpenAiModel {
    pub fn new(connection: ResolvedConnection) -> Self {
        let mut config =
            OpenAIConfig::with_key(&COMPATIBLE, connection.api_key.clone().unwrap_or_default());
        config.base_url = connection.base_url.trim().trim_end_matches('/').to_string();
        let model = Model::new(config.chat(connection.model.clone()), transport())
            .with_idle_timeout(Some(IDLE_TIMEOUT));
        Self {
            model,
            connection,
            stream: true,
            temperature: 0.3,
            max_tokens: 2048,
        }
    }

    /// 转成 rig 的请求（纯函数，便于测试）。返回请求与工具名对照表。
    pub fn to_rig(&self, request: &ModelRequest) -> Result<(CompletionRequest, NameMap), AiError> {
        let names = NameMap::build(request);
        let mut history: Vec<RigMessage> = Vec::new();
        // 工具带回的图片：OpenAI 兼容接口的工具结果只能是文字，而且一次工具调用的全部结果
        // 必须紧挨着。所以等这一组工具结果结束后，再补一条带图片的用户消息（说明是数据）。
        let mut pending: Vec<UserContent> = Vec::new();
        for (i, message) in request.messages.iter().enumerate() {
            if let Some(m) = convert_message(message, &names)? {
                history.push(m);
            }
            if let Message::Tool { name, images, .. } = message {
                let note = super::render_tool_images(name, images);
                if !note.is_empty() {
                    pending.push(UserContent::text(note));
                }
                for image in images {
                    if let Attachment::Image { mime, data, .. } = image {
                        if !data.is_empty() {
                            pending.push(UserContent::image_base64(
                                data.clone(),
                                media_type(mime),
                                None,
                            ));
                        }
                    }
                }
            }
            let group_ends = !matches!(request.messages.get(i + 1), Some(Message::Tool { .. }));
            if group_ends && !pending.is_empty() {
                history.push(RigMessage::User {
                    content: std::mem::take(&mut pending),
                });
            }
        }
        let prompt = history
            .pop()
            .ok_or_else(|| AiError::Failed("没有要发给 AI 的消息".into()))?;
        let mut rig = CompletionRequest::new(prompt)
            .messages(history)
            .preamble(request.system.clone())
            .temperature(self.temperature);
        rig.max_tokens = Some(self.max_tokens);
        rig.accept_unknown_finish_reasons = true;
        for tool in &request.tools {
            rig.tools.push(Arc::new(ToolDefinition::new(
                tool_name(&names.wire(&tool.name))?,
                tool.description.clone(),
                tool.parameters.clone(),
            )));
        }
        if self.connection.provider.starts_with("dashscope") {
            rig.additional_params = Some(json!({"enable_thinking": false}));
        }
        Ok((rig, names))
    }
}

fn tool_name(name: &str) -> Result<ToolName, AiError> {
    ToolName::new(name).map_err(|_| AiError::Failed("工具名为空".into()))
}

fn media_type(mime: &str) -> Option<ImageMediaType> {
    match mime {
        "image/png" => Some(ImageMediaType::PNG),
        "image/jpeg" | "image/jpg" => Some(ImageMediaType::JPEG),
        "image/webp" => Some(ImageMediaType::WEBP),
        "image/gif" => Some(ImageMediaType::GIF),
        _ => None,
    }
}

fn convert_message(message: &Message, names: &NameMap) -> Result<Option<RigMessage>, AiError> {
    Ok(Some(match message {
        Message::User {
            text,
            attachments,
            context,
            ..
        } => {
            let mut content = vec![UserContent::text(render_user(context, text, attachments))];
            for attachment in attachments {
                if let Attachment::Image { mime, data, .. } = attachment {
                    if !data.is_empty() {
                        content.push(UserContent::image_base64(
                            data.clone(),
                            media_type(mime),
                            None,
                        ));
                    }
                }
            }
            RigMessage::User { content }
        }
        Message::Assistant {
            text,
            tool_calls,
            reasoning,
            ..
        } => {
            let mut content = Vec::new();
            if !reasoning.is_empty() {
                content.push(AssistantContent::reasoning(reasoning.clone()));
            }
            if !text.is_empty() {
                content.push(AssistantContent::text(text.clone()));
            }
            for call in tool_calls {
                let arguments = if call.arguments.trim().is_empty() {
                    "{}".to_string()
                } else {
                    call.arguments.clone()
                };
                content.push(AssistantContent::tool_call(
                    call.id.clone(),
                    tool_name(&names.wire(&call.name))?,
                    Value::String(arguments),
                ));
            }
            if content.is_empty() {
                return Ok(None);
            }
            RigMessage::Assistant(AssistantMessage::new(content))
        }
        Message::Tool {
            call_id,
            name,
            content,
            is_error,
            untrusted,
            ..
        } => RigMessage::User {
            content: vec![UserContent::ToolResult(ToolResult {
                call: CallId::from_wire(call_id.clone()),
                name: tool_name(&names.wire(name))?,
                content: vec![ToolResultContent::text(render_tool(
                    content, *is_error, *untrusted,
                ))],
                is_error: *is_error,
            })],
        },
        Message::Notice { .. } => return Ok(None),
    }))
}

/// rig 的错误文本会带上服务商的响应体，这里只保留可以给用户看的部分。
pub fn map_error(error: &ProviderError) -> AiError {
    let status = |response: &rig_core::ProviderResponseError| {
        response
            .status
            .map(|s| {
                format!(
                    "AI 接口返回 HTTP {}，请检查连接、模型和服务商状态",
                    s.as_u16()
                )
            })
            .unwrap_or_else(|| "AI 接口返回错误".into())
    };
    AiError::Failed(match error {
        ProviderError::Http(_) => "连不上 AI 接口，请检查接口地址与网络".into(),
        ProviderError::ProviderResponse(r) => status(r),
        ProviderError::InvalidAuthentication(r) => format!("{}（Key 无效或无权限）", status(r)),
        ProviderError::Stalled(_) => "AI 响应超时".into(),
        ProviderError::Truncated => "AI 响应中断".into(),
        ProviderError::Json(_) | ProviderError::Response(_) => "AI 响应格式不正确".into(),
        ProviderError::Url(_) => "AI 接口地址无效".into(),
        ProviderError::Request(_) | ProviderError::UnsupportedOption(_) => {
            "AI 请求无法构建，请检查模型与参数".into()
        }
        _ => "AI 请求失败".into(),
    })
}

fn convert_reply(
    response: &CompletionResponse,
    names: &NameMap,
    connection: &ResolvedConnection,
) -> Result<ModelReply, AiError> {
    if response.error.is_some() {
        return Err(AiError::Failed("AI 接口返回错误".into()));
    }
    let tool_calls = response
        .tool_calls()
        .map(|call| ToolCall {
            id: call.id.wire().into_owned(),
            name: names.local(call.function.name.as_str()),
            arguments: call.function.invalid_arguments.clone().unwrap_or_else(|| {
                serde_json::to_string(&call.function.arguments).unwrap_or_default()
            }),
        })
        .collect();
    use rig_core::completion::FinishReason as Rig;
    let finish = match response.finish_reason() {
        Some(Rig::Stop) => FinishReason::Stop,
        Some(Rig::ToolCalls) => FinishReason::ToolCalls,
        Some(Rig::Length) => FinishReason::Length,
        _ => FinishReason::Other,
    };
    let usage = &response.usage;
    // DeepSeek 在 usage 里报 `prompt_cache_hit_tokens`（另有 `prompt_tokens_details.cached_tokens`），
    // rig 的 OpenAI 兼容解析两者都认；原始字段也再看一眼，以官方字段为准。
    let cached = response
        .raw
        .pointer("/usage/prompt_cache_hit_tokens")
        .and_then(Value::as_u64)
        .or(usage.cached_input_tokens);
    let price =
        kotone_core::usage::price_for(&connection.provider, &connection.model, super::now_ms());
    Ok(ModelReply {
        text: response.text(),
        reasoning: response.reasoning(),
        tool_calls,
        finish,
        usage: Some(TokenUsage {
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            cached_input_tokens: cached,
            reasoning_tokens: usage.reasoning_tokens,
            // 价格表缺了某一部分时 total 只是下限，这种情况不报，免得低估。
            cost_usd: usage
                .cost
                .as_ref()
                .filter(|c| c.is_complete())
                .map(|c| c.total),
            cost: price.as_ref().and_then(|p| {
                kotone_core::usage::estimate(p, usage.input_tokens, cached, usage.output_tokens)
            }),
            currency: price.map(|p| p.currency.to_string()),
        }),
    })
}

impl ModelClient for OpenAiModel {
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
        let (rig, names) = self.to_rig(request)?;
        let work = async {
            let response = if self.stream {
                let mut stream = self.model.stream(rig).map_err(|e| map_error(&e))?;
                while let Some(item) = stream.next().await {
                    match item.map_err(|e| map_error(&e))? {
                        Item::Event(StreamEvent::Text { text, .. }) => on_delta(Delta::Text(&text)),
                        Item::Event(StreamEvent::Reasoning { text, .. }) => {
                            on_delta(Delta::Thought(&text))
                        }
                        _ => {}
                    }
                }
                stream.finish().await.map_err(|e| map_error(&e))?
            } else {
                let response = self.model.call(rig).await.map_err(|e| map_error(&e))?;
                let text = response.text();
                if !text.is_empty() {
                    on_delta(Delta::Text(&text));
                }
                response
            };
            convert_reply(&response, &names, &self.connection)
        };
        tokio::select! {
            biased;
            _ = cancel.cancelled() => Err(AiError::Cancelled),
            result = work => result,
        }
    }
}

/// 内置 Agent 用的模型：DeepSeek 官方接口走自己的客户端（ADR 026），其余经 rig（ADR 015）。
pub enum AgentModel {
    Rig(Box<OpenAiModel>),
    DeepSeek(super::deepseek::DeepSeekModel),
}

impl AgentModel {
    pub fn for_connection(connection: ResolvedConnection) -> Self {
        if super::deepseek::is_deepseek(&connection) {
            Self::DeepSeek(super::deepseek::DeepSeekModel::new(connection))
        } else {
            Self::Rig(Box::new(OpenAiModel::new(connection)))
        }
    }

    pub fn set_max_tokens(&mut self, max_tokens: u64) {
        match self {
            Self::Rig(m) => m.max_tokens = max_tokens,
            Self::DeepSeek(m) => m.max_tokens = max_tokens,
        }
    }

    /// DeepSeek 专有的选项；别的服务商返回 None。
    pub fn deepseek(&mut self) -> Option<&mut super::deepseek::DeepSeekModel> {
        match self {
            Self::DeepSeek(m) => Some(m),
            Self::Rig(_) => None,
        }
    }
}

impl ModelClient for AgentModel {
    async fn complete(
        &self,
        request: &ModelRequest,
        on_delta: &(dyn Fn(Delta<'_>) + Send + Sync),
        cancel: &CancellationToken,
    ) -> Result<ModelReply, AiError> {
        match self {
            Self::Rig(m) => m.complete(request, on_delta, cancel).await,
            Self::DeepSeek(m) => m.complete(request, on_delta, cancel).await,
        }
    }
}

/// Kotone 工具名 ↔ 接口里的函数名。
#[derive(Debug, Default)]
pub struct NameMap {
    to_wire: HashMap<String, String>,
    to_local: HashMap<String, String>,
}

impl NameMap {
    pub(super) fn build(request: &ModelRequest) -> Self {
        let mut map = Self::default();
        for tool in &request.tools {
            map.insert(&tool.name);
        }
        for message in &request.messages {
            match message {
                Message::Assistant { tool_calls, .. } => {
                    for call in tool_calls {
                        map.insert(&call.name);
                    }
                }
                Message::Tool { name, .. } => map.insert(name),
                _ => {}
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
