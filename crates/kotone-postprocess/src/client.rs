//! OpenAI-compatible chat completions 薄客户端。

use kotone_core::postprocess::{ProcessError, ProcessingCancelToken};
use serde::{Deserialize, Serialize};
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

#[derive(Debug, Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatResponseMessage,
}

#[derive(Debug, Deserialize)]
struct ChatResponseMessage {
    content: Option<String>,
}

pub fn completions_url(base_url: &str) -> String {
    format!("{}/chat/completions", base_url.trim().trim_end_matches('/'))
}

pub fn validate_output(source: &str, output: &str, max_ratio: usize) -> Result<String, ProcessError> {
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
    if cancel.is_cancelled() {
        return Err(cancelled());
    }
    let url = completions_url(&request.base_url);
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

    let mut builder = client.post(url).json(&body);
    if let Some(api_key) = request
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|key| !key.is_empty())
    {
        builder = builder.bearer_auth(api_key);
    }

    let response = tokio::select! {
        _ = cancel.cancelled() => return Err(cancelled()),
        result = builder.send() => result.map_err(|error| ProcessError::failed(format!("请求失败：{error}")))?,
    };
    let status = response.status();
    let bytes = tokio::select! {
        _ = cancel.cancelled() => return Err(cancelled()),
        result = response.bytes() => result.map_err(|error| ProcessError::failed(format!("读取响应失败：{error}")))?,
    };
    if !status.is_success() {
        let snippet = String::from_utf8_lossy(&bytes);
        let snippet = snippet.chars().take(180).collect::<String>();
        return Err(ProcessError::failed(format!(
            "接口返回 {status}：{snippet}"
        )));
    }
    let parsed: ChatResponse = serde_json::from_slice(&bytes)
        .map_err(|error| ProcessError::failed(format!("响应无法解析：{error}")))?;
    let content = parsed
        .choices
        .into_iter()
        .next()
        .and_then(|choice| choice.message.content)
        .unwrap_or_default();
    Ok(content)
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
