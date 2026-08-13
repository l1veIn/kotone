//! Qwen-MT 在线翻译。同一 HTTP 客户端，额外发送 translation_options。

use std::sync::Arc;

use kotone_core::connection::ConnectionResolver;
use kotone_core::postprocess::{
    NetworkAccess, ProcessError, ProcessFuture, ProcessingCancelToken, ProcessingContext,
    ProcessorCategory, ProcessorConfigField, ProcessorConfigFieldKind, ProcessorDescriptor,
    ProcessorFactory, TextDocument, TextProcessor,
};
use serde_json::{json, Value};

use crate::client::{chat_completion, validate_output, ChatMessage, ChatRequest};

pub const PROCESSOR_ID: &str = "translation.qwen-mt";

pub struct QwenMtFactory {
    pub connections: Arc<dyn ConnectionResolver>,
}

impl ProcessorFactory for QwenMtFactory {
    fn descriptor(&self) -> ProcessorDescriptor {
        ProcessorDescriptor {
            id: PROCESSOR_ID.into(),
            display_name: "翻译（Qwen-MT）".into(),
            description: "用通义 Qwen-MT 把识别文本译成目标语言，并尽量保住游戏术语。".into(),
            category: ProcessorCategory::Translation,
            developer_only: false,
            network_access: NetworkAccess::Internet,
            config_fields: vec![
                ProcessorConfigField {
                    key: "connectionId".into(),
                    display_name: "API 连接".into(),
                    description: "请使用通义兼容端点；模型建议 qwen-mt-lite。".into(),
                    kind: ProcessorConfigFieldKind::Connection,
                    required: true,
                    file_extensions: Vec::new(),
                },
                ProcessorConfigField {
                    key: "targetLang".into(),
                    display_name: "目标语言".into(),
                    description: "例如 English、Japanese、Korean。".into(),
                    kind: ProcessorConfigFieldKind::Text,
                    required: true,
                    file_extensions: Vec::new(),
                },
                ProcessorConfigField {
                    key: "sourceLang".into(),
                    display_name: "源语言（可选）".into(),
                    description: "留空则 auto。指定中文可写 Chinese。".into(),
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
        let mut connection = self.connections.resolve(connection_id)?;
        if connection.api_key.as_deref().unwrap_or("").trim().is_empty() {
            return Err(format!("连接「{}」还没有 API key", connection.display_name));
        }
        if let Some(model) = config
            .get("model")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|model| !model.is_empty())
        {
            connection.model = model.to_string();
        } else if !connection.model.to_ascii_lowercase().contains("qwen-mt") {
            connection.model = "qwen-mt-lite".into();
        }
        let target_lang = config
            .get("targetLang")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|lang| !lang.is_empty())
            .ok_or_else(|| "请填写目标语言".to_string())?
            .to_string();
        let source_lang = config
            .get("sourceLang")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|lang| !lang.is_empty())
            .unwrap_or("auto")
            .to_string();
        let terms = config.get("terms").cloned().unwrap_or(Value::Array(vec![]));
        Ok(Arc::new(QwenMtProcessor {
            client: reqwest::Client::new(),
            connection,
            source_lang,
            target_lang,
            terms,
        }))
    }
}

struct QwenMtProcessor {
    client: reqwest::Client,
    connection: kotone_core::connection::ResolvedConnection,
    source_lang: String,
    target_lang: String,
    terms: Value,
}

impl TextProcessor for QwenMtProcessor {
    fn process<'a>(
        &'a self,
        input: TextDocument,
        _context: &'a ProcessingContext,
        cancel: ProcessingCancelToken,
    ) -> ProcessFuture<'a> {
        Box::pin(async move {
            let mut translation_options = json!({
                "source_lang": self.source_lang,
                "target_lang": self.target_lang,
            });
            if let Value::Array(terms) = &self.terms {
                if !terms.is_empty() {
                    translation_options
                        .as_object_mut()
                        .unwrap()
                        .insert("terms".into(), Value::Array(terms.clone()));
                }
            }
            let raw = chat_completion(
                &self.client,
                ChatRequest {
                    base_url: self.connection.base_url.clone(),
                    api_key: self.connection.api_key.clone(),
                    model: self.connection.model.clone(),
                    messages: vec![ChatMessage {
                        role: "user".into(),
                        content: input.text.clone(),
                    }],
                    temperature: 0.1,
                    max_tokens: 128,
                    extra_body: json!({ "translation_options": translation_options }),
                },
                &cancel,
            )
            .await?;
            let text = validate_output(&input.text, &raw, 6)?;
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
    use wiremock::matchers::{body_partial_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    struct StaticResolver(ResolvedConnection);

    impl ConnectionResolver for StaticResolver {
        fn resolve(&self, _: &str) -> Result<ResolvedConnection, String> {
            Ok(self.0.clone())
        }
    }

    #[tokio::test]
    async fn process_sends_translation_options() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .and(body_partial_json(json!({
                "model": "qwen-mt-lite",
                "translation_options": { "source_lang": "auto", "target_lang": "English" }
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "choices": [{ "message": { "content": "Flash is down" } }]
            })))
            .mount(&server)
            .await;

        let factory = QwenMtFactory {
            connections: Arc::new(StaticResolver(ResolvedConnection {
                id: "ds".into(),
                display_name: "通义".into(),
                kind: ConnectionKind::Remote,
                provider: "dashscope".into(),
                base_url: server.uri(),
                model: "qwen-turbo".into(),
                api_key: Some("sk-test".into()),
            })),
        };
        let processor = factory
            .create(&json!({"connectionId": "ds", "targetLang": "English"}))
            .unwrap();
        let output = processor
            .process(
                TextDocument::recognized("闪现交了"),
                &ProcessingContext::default(),
                ProcessingCancelToken::default(),
            )
            .await
            .unwrap();
        assert_eq!(output.text, "Flash is down");
    }
}
