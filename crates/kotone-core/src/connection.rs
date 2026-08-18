//! 后处理连接目录：公开记录与解析端口。
//!
//! API key 不进本模块的 serde 结构，也不进 `config.json`。凭据由后续
//! `SecretStore` 按 connection id 另存；本阶段 `resolve` 只填公开字段。

use serde::{Deserialize, Serialize};

/// 连接的部署形态。2a 只实现 `Remote`；`Attach` / `Managed` 预留给本地。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConnectionKind {
    #[default]
    Remote,
    Attach,
    Managed,
}

/// 用户可见、可持久化的连接记录。不含密钥。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Connection {
    pub id: String,
    pub display_name: String,
    #[serde(default)]
    pub kind: ConnectionKind,
    /// 预设 id：`dashscope` / `dashscope-intl` / `groq` / `gemini` / `xai` / `openai` / `custom`
    #[serde(default)]
    pub provider: String,
    pub base_url: String,
    pub model: String,
}

impl Connection {
    pub fn validate(&self) -> Result<(), String> {
        if self.id.trim().is_empty() {
            return Err("连接 ID 不能为空".into());
        }
        if self.display_name.trim().is_empty() {
            return Err("连接名称不能为空".into());
        }
        if self.kind != ConnectionKind::Remote {
            return Err(format!(
                "连接类型「{}」尚未实现，当前只支持在线 remote",
                connection_kind_label(self.kind)
            ));
        }
        if self.base_url.trim().is_empty() {
            return Err("接口地址不能为空".into());
        }
        if self.model.trim().is_empty() {
            return Err("模型名不能为空".into());
        }
        Ok(())
    }
}

fn connection_kind_label(kind: ConnectionKind) -> &'static str {
    match kind {
        ConnectionKind::Remote => "remote",
        ConnectionKind::Attach => "attach",
        ConnectionKind::Managed => "managed",
    }
}

/// `resolve` 的结果。`api_key` 在凭据层接入前恒为 `None`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedConnection {
    pub id: String,
    pub display_name: String,
    pub kind: ConnectionKind,
    pub provider: String,
    pub base_url: String,
    pub model: String,
    pub api_key: Option<String>,
}

impl ResolvedConnection {
    pub fn from_public(connection: &Connection) -> Self {
        Self {
            id: connection.id.clone(),
            display_name: connection.display_name.clone(),
            kind: connection.kind,
            provider: connection.provider.clone(),
            base_url: connection.base_url.clone(),
            model: connection.model.clone(),
            api_key: None,
        }
    }
}

/// 按 step 里的 `connectionId` 解析一条可用连接。
///
/// 实现放在 composition root / settings；处理器 factory 只持有 `Arc<dyn …>`，
/// 不得回读可变全局设置。
pub trait ConnectionResolver: Send + Sync {
    fn resolve(&self, connection_id: &str) -> Result<ResolvedConnection, String>;
}

/// 按 connection id 存取 API key。实现不得把密钥写入 Settings / 日志。
pub trait SecretStore: Send + Sync {
    fn get(&self, connection_id: &str) -> Result<Option<String>, String>;
    fn set(&self, connection_id: &str, secret: &str) -> Result<(), String>;
    fn delete(&self, connection_id: &str) -> Result<(), String>;
}

/// 设置页「新建连接」用的厂商预设。不是处理器，也不进 pipeline JSON。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConnectionProviderPreset {
    pub id: &'static str,
    pub display_name: &'static str,
    pub default_base_url: &'static str,
    pub default_model: &'static str,
    /// 厂商控制台里创建 API key 的页面；`None` = 自定义端点，不展示跳转。
    pub api_key_url: Option<&'static str>,
}

pub const CONNECTION_PRESETS: &[ConnectionProviderPreset] = &[
    ConnectionProviderPreset {
        id: "dashscope",
        display_name: "通义千问（北京）",
        default_base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1",
        default_model: "qwen3-30b-a3b-instruct-2507",
        api_key_url: Some("https://bailian.console.aliyun.com/?apiKey=1#/api-key"),
    },
    ConnectionProviderPreset {
        id: "dashscope-intl",
        display_name: "通义千问（国际）",
        default_base_url: "https://dashscope-intl.aliyuncs.com/compatible-mode/v1",
        default_model: "qwen3-30b-a3b-instruct-2507",
        api_key_url: Some("https://modelstudio.console.alibabacloud.com/?tab=dashboard#/api-key"),
    },
    ConnectionProviderPreset {
        id: "groq",
        display_name: "Groq",
        default_base_url: "https://api.groq.com/openai/v1",
        default_model: "llama-3.1-8b-instant",
        api_key_url: Some("https://console.groq.com/keys"),
    },
    ConnectionProviderPreset {
        id: "gemini",
        display_name: "Gemini",
        default_base_url: "https://generativelanguage.googleapis.com/v1beta/openai/",
        default_model: "gemini-2.5-flash-lite",
        api_key_url: Some("https://aistudio.google.com/apikey"),
    },
    ConnectionProviderPreset {
        id: "xai",
        display_name: "xAI Grok",
        default_base_url: "https://api.x.ai/v1",
        default_model: "grok-4-fast",
        api_key_url: Some("https://console.x.ai/"),
    },
    ConnectionProviderPreset {
        id: "openai",
        display_name: "OpenAI",
        default_base_url: "https://api.openai.com/v1",
        default_model: "gpt-4.1-mini",
        api_key_url: Some("https://platform.openai.com/api-keys"),
    },
    ConnectionProviderPreset {
        id: "custom",
        display_name: "自定义 OpenAI 兼容",
        default_base_url: "",
        default_model: "",
        api_key_url: None,
    },
];

pub fn preset_by_id(id: &str) -> Option<&'static ConnectionProviderPreset> {
    CONNECTION_PRESETS.iter().find(|preset| preset.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_json_roundtrip_defaults_kind_to_remote() {
        let raw = r#"{"id":"ds","displayName":"通义","baseUrl":"https://example.test/v1","model":"qwen-turbo"}"#;
        let connection: Connection = serde_json::from_str(raw).unwrap();
        assert_eq!(connection.kind, ConnectionKind::Remote);
        assert!(connection.provider.is_empty());
        assert!(connection.validate().is_ok());
    }

    #[test]
    fn reserved_kinds_are_rejected_until_implemented() {
        let connection = Connection {
            id: "local".into(),
            display_name: "本机".into(),
            kind: ConnectionKind::Managed,
            provider: "llama-server".into(),
            base_url: "http://127.0.0.1:18790/v1".into(),
            model: "qwen".into(),
        };
        assert!(connection.validate().unwrap_err().contains("尚未实现"));
    }

    #[test]
    fn named_presets_have_https_api_key_pages() {
        for preset in CONNECTION_PRESETS {
            if preset.id == "custom" {
                assert!(preset.api_key_url.is_none());
                continue;
            }
            let url = preset.api_key_url.expect(preset.id);
            assert!(
                url.starts_with("https://"),
                "{} 的 API key 页必须是 https：{url}",
                preset.id
            );
        }
    }

    #[test]
    fn resolve_result_does_not_carry_a_key() {
        let connection = Connection {
            id: "ds".into(),
            display_name: "通义".into(),
            kind: ConnectionKind::Remote,
            provider: "dashscope".into(),
            base_url: "https://example.test/v1".into(),
            model: "qwen-turbo".into(),
        };
        let resolved = ResolvedConnection::from_public(&connection);
        assert!(resolved.api_key.is_none());
        assert_eq!(resolved.base_url, connection.base_url);
    }
}
