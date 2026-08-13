//! 通用 OpenAI-compatible 润色。去口癖、修口误、少改、保住游戏术语。

use std::sync::Arc;

use kotone_core::connection::ConnectionResolver;
use kotone_core::postprocess::{
    NetworkAccess, ProcessError, ProcessFuture, ProcessingCancelToken, ProcessingContext,
    ProcessorCategory, ProcessorConfigField, ProcessorConfigFieldKind, ProcessorDescriptor,
    ProcessorFactory, TextDocument, TextProcessor,
};
use serde_json::{json, Value};

use crate::client::{chat_completion, validate_output, ChatMessage, ChatRequest};

pub const PROCESSOR_ID: &str = "writing.openai-compat";

pub const DEFAULT_SYSTEM_PROMPT: &str = "\
只输出清理后的游戏聊天短句。
删除口癖（那个/就是/嗯/啊），修正明显识别错误。
不要改变意思，不要变正式，不要解释。
保留游戏术语原文：闪现、大龙、小龙、gank、flash、baron。";

pub struct OpenAiCompatFactory {
    pub connections: Arc<dyn ConnectionResolver>,
}

impl ProcessorFactory for OpenAiCompatFactory {
    fn descriptor(&self) -> ProcessorDescriptor {
        ProcessorDescriptor {
            id: PROCESSOR_ID.into(),
            display_name: "AI 润色".into(),
            description: "用在线大模型去掉口癖、修正口误，并尽量保持原意和游戏术语。".into(),
            category: ProcessorCategory::Writing,
            developer_only: false,
            network_access: NetworkAccess::Internet,
            config_fields: vec![
                ProcessorConfigField {
                    key: "connectionId".into(),
                    display_name: "API 连接".into(),
                    description: "使用「API 连接」里已保存的在线接口。".into(),
                    kind: ProcessorConfigFieldKind::Connection,
                    required: true,
                    file_extensions: Vec::new(),
                },
                ProcessorConfigField {
                    key: "systemPrompt".into(),
                    display_name: "系统提示（可选）".into(),
                    description: "留空则使用默认的少改、去口癖提示。".into(),
                    kind: ProcessorConfigFieldKind::Text,
                    required: false,
                    file_extensions: Vec::new(),
                },
            ],
        }
    }

    fn create(&self, config: &Value) -> Result<Arc<dyn TextProcessor>, String> {
        let connection_id = config
            .get("connectionId")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| "请选择 API 连接".to_string())?;
        let connection = self.connections.resolve(connection_id)?;
        if connection.api_key.as_deref().unwrap_or("").trim().is_empty() {
            return Err(format!("连接「{}」还没有 API key", connection.display_name));
        }
        let system_prompt = config
            .get("systemPrompt")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|prompt| !prompt.is_empty())
            .unwrap_or(DEFAULT_SYSTEM_PROMPT)
            .to_string();
        let max_tokens = config
            .get("maxTokens")
            .and_then(Value::as_u64)
            .unwrap_or(96)
            .clamp(16, 256) as u32;
        Ok(Arc::new(OpenAiCompatProcessor {
            client: reqwest::Client::new(),
            connection,
            system_prompt,
            max_tokens,
        }))
    }
}

struct OpenAiCompatProcessor {
    client: reqwest::Client,
    connection: kotone_core::connection::ResolvedConnection,
    system_prompt: String,
    max_tokens: u32,
}

impl TextProcessor for OpenAiCompatProcessor {
    fn process<'a>(
        &'a self,
        input: TextDocument,
        _context: &'a ProcessingContext,
        cancel: ProcessingCancelToken,
    ) -> ProcessFuture<'a> {
        Box::pin(async move {
            let raw = chat_completion(
                &self.client,
                ChatRequest {
                    base_url: self.connection.base_url.clone(),
                    api_key: self.connection.api_key.clone(),
                    model: self.connection.model.clone(),
                    messages: vec![
                        ChatMessage {
                            role: "system".into(),
                            content: self.system_prompt.clone(),
                        },
                        ChatMessage {
                            role: "user".into(),
                            content: input.text.clone(),
                        },
                    ],
                    temperature: 0.2,
                    max_tokens: self.max_tokens,
                    extra_body: json!({ "enable_thinking": false }),
                },
                &cancel,
            )
            .await?;
            let text = validate_output(&input.text, &raw, 2)?;
            if cancel.is_cancelled() {
                return Err(ProcessError {
                    kind: kotone_core::postprocess::ProcessErrorKind::Cancelled,
                    message: "后处理已取消".into(),
                });
            }
            Ok(TextDocument {
                source_text: input.source_text,
                text,
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kotone_core::connection::{ConnectionKind, ConnectionResolver, ResolvedConnection};
    use kotone_core::postprocess::ProcessingCancelToken;
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    struct StaticResolver(ResolvedConnection);

    impl ConnectionResolver for StaticResolver {
        fn resolve(&self, _: &str) -> Result<ResolvedConnection, String> {
            Ok(self.0.clone())
        }
    }

    fn resolved(base_url: String) -> ResolvedConnection {
        ResolvedConnection {
            id: "ds".into(),
            display_name: "通义".into(),
            kind: ConnectionKind::Remote,
            provider: "dashscope".into(),
            base_url,
            model: "qwen-turbo".into(),
            api_key: Some("sk-test".into()),
        }
    }

    #[test]
    fn create_requires_connection_and_key() {
        let factory = OpenAiCompatFactory {
            connections: Arc::new(StaticResolver(ResolvedConnection {
                api_key: None,
                ..resolved("https://example.test/v1".into())
            })),
        };
        assert!(factory.create(&json!({})).is_err());
        match factory.create(&json!({"connectionId": "ds"})) {
            Err(error) => assert!(error.contains("API key")),
            Ok(_) => panic!("missing key should fail"),
        }
    }

    #[tokio::test]
    async fn process_returns_model_content() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "choices": [{ "message": { "content": "闪现交了" } }]
            })))
            .mount(&server)
            .await;

        let factory = OpenAiCompatFactory {
            connections: Arc::new(StaticResolver(resolved(server.uri()))),
        };
        let processor = factory.create(&json!({"connectionId": "ds"})).unwrap();
        let output = processor
            .process(
                TextDocument::recognized("那个闪了"),
                &ProcessingContext::default(),
                ProcessingCancelToken::default(),
            )
            .await
            .unwrap();
        assert_eq!(output.text, "闪现交了");
    }
}
