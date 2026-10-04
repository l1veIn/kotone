use kotone_core::cancel::CancellationToken;
use serde_json::Value;

const MAX_RESPONSE_BYTES: usize = 1_048_576;

#[derive(Debug)]
pub enum AiError {
    Cancelled,
    Failed(String),
}

impl std::fmt::Display for AiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => f.write_str("请求已取消"),
            Self::Failed(message) => f.write_str(message),
        }
    }
}

pub fn completions_url(base_url: &str) -> String {
    format!("{}/chat/completions", base_url.trim().trim_end_matches('/'))
}

/// Request and response reads are cancellable and bounded. Never include a
/// provider response body or authenticated URL in a persisted error message.
pub async fn chat_json(
    client: &reqwest::Client,
    base_url: &str,
    api_key: Option<&str>,
    body: Value,
    cancel: &CancellationToken,
) -> Result<Value, AiError> {
    let request = async {
        let mut builder = client.post(completions_url(base_url)).json(&body);
        if let Some(key) = api_key.map(str::trim).filter(|key| !key.is_empty()) {
            builder = builder.bearer_auth(key);
        }
        let mut response = builder.send().await.map_err(|e| {
            AiError::Failed(
                if e.is_timeout() {
                    "AI 请求超时"
                } else {
                    "AI 连接失败，请检查接口地址与网络"
                }
                .into(),
            )
        })?;
        if !response.status().is_success() {
            return Err(AiError::Failed(format!(
                "AI 接口返回 HTTP {}，请检查连接、模型和服务商状态",
                response.status().as_u16()
            )));
        }
        if response
            .content_length()
            .is_some_and(|n| n > MAX_RESPONSE_BYTES as u64)
        {
            return Err(AiError::Failed("AI 响应超出大小限制".into()));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| AiError::Failed("读取 AI 响应失败".into()))?
        {
            if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
                return Err(AiError::Failed("AI 响应超出大小限制".into()));
            }
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes).map_err(|_| AiError::Failed("AI 响应不是有效 JSON".into()))
    };
    tokio::select! {
        biased;
        _ = cancel.cancelled() => Err(AiError::Cancelled),
        result = tokio::time::timeout(std::time::Duration::from_secs(30), request) =>
            result.map_err(|_| AiError::Failed("AI 请求超时".into()))?,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn cancelled_request_does_not_reach_provider() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;
        let cancel = CancellationToken::default();
        cancel.cancel();
        assert!(matches!(
            chat_json(
                &reqwest::Client::new(),
                &server.uri(),
                None,
                serde_json::json!({}),
                &cancel
            )
            .await,
            Err(AiError::Cancelled)
        ));
    }

    #[tokio::test]
    async fn provider_error_does_not_expose_response_contents() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(401).set_body_string("secret-user-transcript"))
            .mount(&server)
            .await;
        let error = chat_json(
            &reqwest::Client::new(),
            &server.uri(),
            None,
            serde_json::json!({}),
            &CancellationToken::default(),
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(error.contains("401"));
        assert!(!error.contains("secret-user-transcript"));
    }

    #[tokio::test]
    async fn oversized_or_invalid_response_is_rejected_without_echoing_content() {
        for body in [
            "x".repeat(MAX_RESPONSE_BYTES + 1),
            "secret-invalid-json".to_string(),
        ] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .respond_with(ResponseTemplate::new(200).set_body_string(body))
                .mount(&server)
                .await;
            let error = chat_json(
                &reqwest::Client::new(),
                &server.uri(),
                None,
                serde_json::json!({}),
                &CancellationToken::default(),
            )
            .await
            .unwrap_err()
            .to_string();
            assert!(error.contains("大小限制") || error.contains("有效 JSON"));
            assert!(!error.contains("secret-invalid-json"));
        }
    }
}
