//! OpenAI-compatible chat completions 薄客户端。

use kotone_core::postprocess::{ProcessError, ProcessingCancelToken};
use serde::Serialize;
use serde_json::{json, Value};

#[derive(Debug, Clone, Serialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

pub struct ChatRequest {
    pub base_url: String,
    pub api_key: Option<String>,
    pub model: String,
    pub messages: Vec<ChatMessage>,
    pub temperature: f32,
    pub max_tokens: u32,
    pub extra_body: Value,
}

pub use kotone_ai::transport::completions_url;

pub fn validate_output(
    source: &str,
    output: &str,
    max_ratio: usize,
) -> Result<String, ProcessError> {
    let text = output.trim();
    if text.is_empty() {
        return Err(ProcessError {
            kind: kotone_core::postprocess::ProcessErrorKind::InvalidOutput,
            message: "模型返回了空文本".into(),
        });
    }
    let limit = source.chars().count().saturating_mul(max_ratio).max(64);
    if text.chars().count() > limit {
        return Err(ProcessError {
            kind: kotone_core::postprocess::ProcessErrorKind::InvalidOutput,
            message: format!("模型输出过长（超过 {limit} 字），已拒绝"),
        });
    }
    Ok(text.to_string())
}

pub async fn chat_completion(
    client: &reqwest::Client,
    request: ChatRequest,
    cancel: &ProcessingCancelToken,
) -> Result<String, ProcessError> {
    let mut body = json!({
        "model": request.model,
        "messages": request.messages,
        "temperature": request.temperature,
        "max_tokens": request.max_tokens,
    });
    if let Value::Object(extra) = request.extra_body {
        if let Value::Object(map) = &mut body {
            for (key, value) in extra {
                map.insert(key, value);
            }
        }
    }

    let parsed = kotone_ai::transport::chat_json(
        client,
        &request.base_url,
        request.api_key.as_deref(),
        body,
        cancel,
    )
    .await
    .map_err(|error| match error {
        kotone_ai::transport::AiError::Cancelled => cancelled(),
        kotone_ai::transport::AiError::Failed(message) => ProcessError::failed(message),
    })?;
    Ok(parsed
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string())
}

fn cancelled() -> ProcessError {
    ProcessError {
        kind: kotone_core::postprocess::ProcessErrorKind::Cancelled,
        message: "后处理已取消".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completions_url_strips_trailing_slash() {
        assert_eq!(
            completions_url("https://api.example/v1/"),
            "https://api.example/v1/chat/completions"
        );
    }

    #[test]
    fn validate_output_rejects_empty_and_runaway() {
        assert!(validate_output("闪了", "   ", 2).is_err());
        assert!(validate_output("闪了", &"扩写说明书".repeat(40), 2).is_err());
        assert_eq!(validate_output("闪了", "闪现交了", 2).unwrap(), "闪现交了");
    }
}
