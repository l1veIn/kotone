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
///
/// 连接现在主要给内置琴音 Agent 用，默认模型取各家的主流大模型（能稳定调用工具）。
/// 清单参考 DeepSeek Harness 使用的 pi-ai 模型目录（Apache-2.0，见 THIRD_PARTY_NOTICES.md），
/// 只收 OpenAI 兼容接口：需要 OAuth 或账号专属地址的平台用「自定义」。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConnectionProviderPreset {
    pub id: &'static str,
    pub display_name: &'static str,
    /// 分组：`recommended` / `china` / `global` / `aggregator` / `custom`。
    pub group: &'static str,
    /// 一句话说明。
    pub description: &'static str,
    pub default_base_url: &'static str,
    pub default_model: &'static str,
    /// 可选的模型（第一个一般就是默认模型；用户也可以手填）。
    pub models: &'static [&'static str],
    /// 厂商控制台里创建 API key 的页面；`None` = 不确定或自定义端点，不展示跳转。
    pub api_key_url: Option<&'static str>,
}

/// 预设的分组（界面按这个顺序展示）。
pub const PRESET_GROUPS: &[&str] = &["recommended", "china", "global", "aggregator", "custom"];

pub const CONNECTION_PRESETS: &[ConnectionProviderPreset] = &[
    ConnectionProviderPreset {
        id: "deepseek",
        display_name: "DeepSeek",
        group: "recommended",
        description: "深度求索官方接口，工具调用稳定，支持看图（V4.1 Flash）",
        default_base_url: "https://api.deepseek.com/v1",
        default_model: "deepseek-flash",
        models: &["deepseek-flash", "deepseek-v4-pro"],
        api_key_url: Some("https://platform.deepseek.com/api_keys"),
    },
    ConnectionProviderPreset {
        id: "dashscope",
        display_name: "通义千问（北京）",
        group: "recommended",
        description: "阿里云百炼，国内访问快",
        default_base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1",
        default_model: "qwen-plus",
        models: &["qwen-plus", "qwen-max", "qwen-flash"],
        api_key_url: Some("https://bailian.console.aliyun.com/?apiKey=1#/api-key"),
    },
    ConnectionProviderPreset {
        id: "moonshot-cn",
        display_name: "月之暗面 Kimi（国内）",
        group: "recommended",
        description: "Kimi 开放平台，长上下文，支持看图",
        default_base_url: "https://api.moonshot.cn/v1",
        default_model: "kimi-k3",
        models: &["kimi-k3", "kimi-k2.6", "kimi-k2.7-code"],
        api_key_url: Some("https://platform.moonshot.cn/console/api-keys"),
    },
    ConnectionProviderPreset {
        id: "zhipu",
        display_name: "智谱 GLM（国内）",
        group: "recommended",
        description: "智谱开放平台按量计费接口",
        default_base_url: "https://open.bigmodel.cn/api/paas/v4",
        default_model: "glm-5.3",
        models: &["glm-5.3", "glm-5.3-flash", "glm-5.2"],
        api_key_url: Some("https://open.bigmodel.cn/usercenter/apikeys"),
    },
    ConnectionProviderPreset {
        id: "minimax-cn",
        display_name: "MiniMax（国内）",
        group: "china",
        description: "MiniMax 开放平台（OpenAI 兼容接口）",
        default_base_url: "https://api.minimaxi.com/v1",
        default_model: "MiniMax-M3",
        models: &["MiniMax-M3", "MiniMax-M2.7", "MiniMax-M2.7-highspeed"],
        api_key_url: Some(
            "https://platform.minimaxi.com/user-center/basic-information/interface-key",
        ),
    },
    ConnectionProviderPreset {
        id: "xiaomi",
        display_name: "小米 MiMo",
        group: "china",
        description: "小米 MiMo 开放平台",
        default_base_url: "https://api.xiaomimimo.com/v1",
        default_model: "mimo-v2.6-pro",
        models: &["mimo-v2.6-pro", "mimo-v2.6-flash", "mimo-v2.5"],
        api_key_url: None,
    },
    ConnectionProviderPreset {
        id: "zhipu-coding",
        display_name: "智谱 GLM Coding Plan",
        group: "china",
        description: "智谱编程套餐专用接口（需订阅套餐）",
        default_base_url: "https://open.bigmodel.cn/api/coding/paas/v4",
        default_model: "glm-5.3",
        models: &["glm-5.3", "glm-5.3-flash", "glm-4.6v"],
        api_key_url: Some("https://open.bigmodel.cn/usercenter/apikeys"),
    },
    ConnectionProviderPreset {
        id: "kimi-coding",
        display_name: "Kimi For Coding",
        group: "china",
        description: "Kimi 编程套餐专用接口（需订阅套餐）",
        default_base_url: "https://api.kimi.com/coding/v1",
        default_model: "kimi-for-coding",
        models: &["kimi-for-coding", "k3"],
        api_key_url: None,
    },
    ConnectionProviderPreset {
        id: "qwen-token-plan-cn",
        display_name: "百炼 Token Plan（国内）",
        group: "china",
        description: "阿里云百炼 Token Plan 套餐接口，可用多家模型",
        default_base_url: "https://token-plan.cn-beijing.maas.aliyuncs.com/compatible-mode/v1",
        default_model: "deepseek-v4.1-flash",
        models: &[
            "deepseek-v4.1-flash",
            "glm-5.3",
            "kimi-k2.6",
            "deepseek-v4-pro",
        ],
        api_key_url: Some("https://bailian.console.aliyun.com/?apiKey=1#/api-key"),
    },
    ConnectionProviderPreset {
        id: "ant-ling",
        display_name: "蚂蚁百灵 Ling",
        group: "china",
        description: "蚂蚁百灵大模型",
        default_base_url: "https://api.ant-ling.com/v1",
        default_model: "Ling-2.6-1T",
        models: &["Ling-2.6-1T", "Ring-2.6-1T", "Ling-2.6-flash"],
        api_key_url: None,
    },
    ConnectionProviderPreset {
        id: "openai",
        display_name: "OpenAI",
        group: "global",
        description: "OpenAI 官方接口",
        default_base_url: "https://api.openai.com/v1",
        default_model: "gpt-5",
        models: &["gpt-5", "gpt-5-mini", "gpt-4.1"],
        api_key_url: Some("https://platform.openai.com/api-keys"),
    },
    ConnectionProviderPreset {
        id: "anthropic",
        display_name: "Anthropic Claude",
        group: "global",
        description: "Claude（经 Anthropic 的 OpenAI 兼容接口）",
        default_base_url: "https://api.anthropic.com/v1",
        default_model: "claude-sonnet-4-5",
        models: &["claude-sonnet-4-5", "claude-opus-5-5", "claude-haiku-5-5"],
        api_key_url: Some("https://console.anthropic.com/settings/keys"),
    },
    ConnectionProviderPreset {
        id: "gemini",
        display_name: "Google Gemini",
        group: "global",
        description: "Gemini（经 Google 的 OpenAI 兼容接口）",
        default_base_url: "https://generativelanguage.googleapis.com/v1beta/openai/",
        default_model: "gemini-3.5-flash",
        models: &[
            "gemini-3.5-flash",
            "gemini-3.1-pro-preview",
            "gemini-2.5-pro",
        ],
        api_key_url: Some("https://aistudio.google.com/apikey"),
    },
    ConnectionProviderPreset {
        id: "xai",
        display_name: "xAI Grok",
        group: "global",
        description: "xAI 官方接口",
        default_base_url: "https://api.x.ai/v1",
        default_model: "grok-4.7",
        models: &["grok-4.7", "grok-4.6", "grok-4.3"],
        api_key_url: Some("https://console.x.ai/"),
    },
    ConnectionProviderPreset {
        id: "moonshot",
        display_name: "Kimi（国际）",
        group: "global",
        description: "Moonshot AI 国际站",
        default_base_url: "https://api.moonshot.ai/v1",
        default_model: "kimi-k3",
        models: &["kimi-k3", "kimi-k2.6", "kimi-k2.7-code"],
        api_key_url: Some("https://platform.moonshot.ai/console/api-keys"),
    },
    ConnectionProviderPreset {
        id: "zai",
        display_name: "Z.AI GLM（国际）",
        group: "global",
        description: "智谱国际站按量计费接口",
        default_base_url: "https://api.z.ai/api/paas/v4",
        default_model: "glm-5.3",
        models: &["glm-5.3", "glm-5.3-flash", "glm-5.2"],
        api_key_url: Some("https://z.ai/manage-apikey/apikey-list"),
    },
    ConnectionProviderPreset {
        id: "minimax",
        display_name: "MiniMax（国际）",
        group: "global",
        description: "MiniMax 国际站（OpenAI 兼容接口）",
        default_base_url: "https://api.minimax.io/v1",
        default_model: "MiniMax-M3",
        models: &["MiniMax-M3", "MiniMax-M2.7"],
        api_key_url: None,
    },
    ConnectionProviderPreset {
        id: "dashscope-intl",
        display_name: "通义千问（国际）",
        group: "global",
        description: "阿里云 Model Studio 国际站",
        default_base_url: "https://dashscope-intl.aliyuncs.com/compatible-mode/v1",
        default_model: "qwen-plus",
        models: &["qwen-plus", "qwen-max", "qwen-flash"],
        api_key_url: Some("https://modelstudio.console.alibabacloud.com/?tab=dashboard#/api-key"),
    },
    ConnectionProviderPreset {
        id: "mistral",
        display_name: "Mistral",
        group: "global",
        description: "Mistral 官方接口",
        default_base_url: "https://api.mistral.ai/v1",
        default_model: "mistral-medium-latest",
        models: &[
            "mistral-medium-latest",
            "magistral-medium-latest",
            "ministral-14b-2512",
        ],
        api_key_url: Some("https://console.mistral.ai/api-keys"),
    },
    ConnectionProviderPreset {
        id: "groq",
        display_name: "Groq",
        group: "global",
        description: "Groq 高速推理",
        default_base_url: "https://api.groq.com/openai/v1",
        default_model: "openai/gpt-oss-120b",
        models: &[
            "openai/gpt-oss-120b",
            "qwen/qwen3.8-27b",
            "llama-3.3-70b-versatile",
        ],
        api_key_url: Some("https://console.groq.com/keys"),
    },
    ConnectionProviderPreset {
        id: "cerebras",
        display_name: "Cerebras",
        group: "global",
        description: "Cerebras 高速推理",
        default_base_url: "https://api.cerebras.ai/v1",
        default_model: "gpt-oss-120b",
        models: &["gpt-oss-120b", "qwen-3.8-27b"],
        api_key_url: Some("https://cloud.cerebras.ai/"),
    },
    ConnectionProviderPreset {
        id: "openrouter",
        display_name: "OpenRouter",
        group: "aggregator",
        description: "一个 Key 用多家模型",
        default_base_url: "https://openrouter.ai/api/v1",
        default_model: "anthropic/claude-sonnet-4.5",
        models: &[
            "anthropic/claude-sonnet-4.5",
            "anthropic/claude-opus-5.5",
            "anthropic/claude-haiku-5.5",
        ],
        api_key_url: Some("https://openrouter.ai/keys"),
    },
    ConnectionProviderPreset {
        id: "together",
        display_name: "Together AI",
        group: "aggregator",
        description: "开源模型推理平台",
        default_base_url: "https://api.together.ai/v1",
        default_model: "deepseek-ai/DeepSeek-V4.1-Flash",
        models: &[
            "deepseek-ai/DeepSeek-V4.1-Flash",
            "moonshotai/Kimi-K3",
            "MiniMaxAI/MiniMax-M3",
            "Qwen/Qwen3.6-Plus",
        ],
        api_key_url: Some("https://api.together.ai/settings/api-keys"),
    },
    ConnectionProviderPreset {
        id: "fireworks",
        display_name: "Fireworks AI",
        group: "aggregator",
        description: "开源模型推理平台",
        default_base_url: "https://api.fireworks.ai/inference/v1",
        default_model: "accounts/fireworks/models/kimi-k3",
        models: &[
            "accounts/fireworks/models/kimi-k3",
            "accounts/fireworks/models/glm-5p3",
            "accounts/fireworks/routers/glm-latest",
        ],
        api_key_url: Some("https://fireworks.ai/account/api-keys"),
    },
    ConnectionProviderPreset {
        id: "nvidia",
        display_name: "NVIDIA NIM",
        group: "aggregator",
        description: "NVIDIA 托管的模型接口",
        default_base_url: "https://integrate.api.nvidia.com/v1",
        default_model: "deepseek-ai/deepseek-v4.1-flash",
        models: &[
            "deepseek-ai/deepseek-v4.1-flash",
            "moonshotai/kimi-k2.6",
            "nvidia/nemotron-3-ultra-550b-a55b",
        ],
        api_key_url: Some("https://build.nvidia.com/"),
    },
    ConnectionProviderPreset {
        id: "huggingface",
        display_name: "Hugging Face",
        group: "aggregator",
        description: "Hugging Face 推理路由",
        default_base_url: "https://router.huggingface.co/v1",
        default_model: "deepseek-ai/DeepSeek-V4.1-Flash",
        models: &[
            "deepseek-ai/DeepSeek-V4.1-Flash",
            "deepseek-ai/DeepSeek-V4-Pro",
        ],
        api_key_url: Some("https://huggingface.co/settings/tokens"),
    },
    ConnectionProviderPreset {
        id: "baseten",
        display_name: "Baseten",
        group: "aggregator",
        description: "开源模型推理平台",
        default_base_url: "https://inference.baseten.co/v1",
        default_model: "deepseek-ai/DeepSeek-V4.1-Flash",
        models: &[
            "deepseek-ai/DeepSeek-V4.1-Flash",
            "moonshotai/Kimi-K3",
            "deepseek-ai/DeepSeek-V4-Pro",
        ],
        api_key_url: None,
    },
    ConnectionProviderPreset {
        id: "vercel-ai-gateway",
        display_name: "Vercel AI Gateway",
        group: "aggregator",
        description: "Vercel 的多模型网关",
        default_base_url: "https://ai-gateway.vercel.sh/v1",
        default_model: "anthropic/claude-sonnet-4.5",
        models: &["anthropic/claude-sonnet-4.5"],
        api_key_url: None,
    },
    ConnectionProviderPreset {
        id: "custom",
        display_name: "自定义 OpenAI 兼容",
        group: "custom",
        description: "任何兼容 OpenAI /chat/completions 的接口",
        default_base_url: "",
        default_model: "",
        models: &[],
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
    fn presets_are_well_formed() {
        let mut ids = std::collections::HashSet::new();
        for preset in CONNECTION_PRESETS {
            assert!(ids.insert(preset.id), "重复的预设 {}", preset.id);
            assert!(
                PRESET_GROUPS.contains(&preset.group),
                "{} 的分组无效",
                preset.id
            );
            if let Some(url) = preset.api_key_url {
                assert!(
                    url.starts_with("https://"),
                    "{} 的 API key 页必须是 https：{url}",
                    preset.id
                );
            }
            if preset.id == "custom" {
                assert!(preset.api_key_url.is_none() && preset.default_base_url.is_empty());
                continue;
            }
            assert!(
                preset.default_base_url.starts_with("https://"),
                "{}",
                preset.id
            );
            assert!(!preset.default_model.is_empty(), "{}", preset.id);
            assert!(
                preset.models.contains(&preset.default_model),
                "{} 的默认模型要在列表里",
                preset.id
            );
            assert!(!preset.description.is_empty(), "{}", preset.id);
        }
        // 已发布版本存过的预设标识必须还在（配置里的 provider 指向它们）。
        for id in [
            "dashscope",
            "dashscope-intl",
            "groq",
            "gemini",
            "xai",
            "openai",
            "custom",
        ] {
            assert!(preset_by_id(id).is_some(), "{id}");
        }
        assert!(preset_by_id("deepseek").is_some());
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
