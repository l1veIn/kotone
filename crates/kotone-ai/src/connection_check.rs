//! 测试连接（设置页「API 连接」的“测试连接”、工具 `companion.check`）：照 DeepSeek Harness，先读一次模型列表
//! （不花 token），服务商没有列表接口时发一次很短的对话。
//!
//! 以前和旧的单次陪玩规划放在一起（`kotone_ai::companion`）；那条流程退掉以后（ADR 053）只剩这一件事。

use kotone_core::cancel::CancellationToken;
use kotone_core::connection::ResolvedConnection;
use serde_json::json;

/// 测试结果。
#[derive(Debug, Clone, PartialEq)]
pub struct SelfCheck {
    pub passed: bool,
    pub message: String,
}

/// 进程内共用的 HTTP 客户端（连接复用；超时与大小限制由 `transport` 负责）。
fn client() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(reqwest::Client::new)
}

/// 测试连接：照 DeepSeek Harness，先读服务商的模型列表（`GET /models`，不花 token），确认地址、密钥能用，
/// 所填模型在列表里；服务商没有列表接口时，改发一次很短的对话请求（一次付费请求）确认连通。
/// 不判断模型会不会调用工具：现在的主流大模型都会，这项判定在用户实测里只会误报。
/// 连接不上、密钥无效时返回 Err。
pub async fn self_check(
    connection: &ResolvedConnection,
    cancel: &CancellationToken,
) -> Result<SelfCheck, String> {
    if connection
        .api_key
        .as_deref()
        .unwrap_or("")
        .trim()
        .is_empty()
    {
        return Ok(SelfCheck {
            passed: false,
            message: "该连接尚未保存 API key".into(),
        });
    }
    match crate::model_listing::list_models(connection, cancel).await? {
        crate::model_listing::Listing::Models(models) => {
            let model = connection.model.trim();
            if !models.is_empty() && !models.iter().any(|m| m == model) {
                return Ok(SelfCheck {
                    passed: false,
                    message: format!("连得上，但服务商的模型列表里没有“{model}”，请换一个模型"),
                });
            }
            return Ok(SelfCheck {
                passed: true,
                message: "连接正常".into(),
            });
        }
        crate::model_listing::Listing::Unsupported => {}
    }
    // 没有列表接口：发一次很短的对话，回了就算连得上（不看它说了什么）
    let mut body = json!({
        "model": connection.model,
        "messages": [{"role": "user", "content": "这是连接测试，请回复“好的”。"}],
        "temperature": 0.2,
        "max_tokens": 16
    });
    if connection.provider.starts_with("dashscope") {
        body["enable_thinking"] = json!(false);
    }
    crate::transport::chat_json(
        client(),
        &connection.base_url,
        connection.api_key.as_deref(),
        body,
        cancel,
    )
    .await
    .map_err(|e| e.to_string())?;
    Ok(SelfCheck {
        passed: true,
        message: "连接正常".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn connection(base_url: String) -> ResolvedConnection {
        ResolvedConnection {
            id: "c".into(),
            display_name: "test".into(),
            kind: kotone_core::connection::ConnectionKind::Remote,
            provider: "deepseek".into(),
            base_url,
            model: "test-model".into(),
            api_key: Some("k".into()),
        }
    }

    #[tokio::test]
    async fn connection_test_reads_the_model_list_without_chatting() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/models"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"data": [{"id": "test-model"}, {"id": "other"}]})),
            )
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(500))
            .expect(0)
            .mount(&server)
            .await;
        let ok = self_check(&connection(server.uri()), &CancellationToken::default())
            .await
            .unwrap();
        assert!(ok.passed, "{}", ok.message);
        let mut wrong = connection(server.uri());
        wrong.model = "no-such-model".into();
        let missing = self_check(&wrong, &CancellationToken::default())
            .await
            .unwrap();
        assert!(!missing.passed);
        assert!(missing.message.contains("no-such-model"));
    }

    #[tokio::test]
    async fn connection_test_only_needs_a_reply() {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        // 没有列表接口：只回了一句话就算通过；只发一次对话，不带工具
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "choices": [{"message": {"role": "assistant", "content": "好的"}, "finish_reason": "stop"}]
            })))
            .mount(&server)
            .await;
        let check = self_check(&connection(server.uri()), &CancellationToken::default())
            .await
            .unwrap();
        assert!(check.passed, "{}", check.message);
        let posts: Vec<_> = server
            .received_requests()
            .await
            .unwrap()
            .into_iter()
            .filter(|r| r.method.as_str() == "POST")
            .collect();
        assert_eq!(posts.len(), 1);
        let body: Value = serde_json::from_slice(&posts[0].body).unwrap();
        assert!(body.get("tools").is_none());
        // 连不上：报错
        let down = self_check(
            &connection("http://127.0.0.1:9".into()),
            &CancellationToken::default(),
        )
        .await;
        assert!(down.is_err());
    }
}
