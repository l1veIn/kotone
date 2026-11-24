//! 语音输入插件在插件清单里的那一面（ADR 038 D5）：清单、她的工具 `game_chat.send`。
//!
//! 听写本身（热键 → 听一句 → 听到以后）不经插件契约：本体的“听一句”直接调 [`crate::VoiceInput`]
//! （`AfterHearing`），宿主按插件开关决定登不登记语音输入热键。这里只有她替玩家在聊天框发言：
//! 往游戏里打字用的注入器和发送时挡住玩家按键（ADR 032）全进程只有一份，在宿主（[`Host::type_in_game_chat`]）。

use std::sync::{Arc, OnceLock};

use kotone_plugin_api::{BoxFuture, Host, Manifest, Plugin, ToolResult, ToolSpec};
use serde_json::{json, Value};

pub const TOOL_SEND: &str = "game_chat.send";
pub const PAGE: &str = "ui://kotone/voice-input/home";

pub fn manifest() -> &'static Manifest {
    static MANIFEST: OnceLock<Manifest> = OnceLock::new();
    MANIFEST.get_or_init(|| {
        Manifest::parse(include_str!("../plugin.toml")).expect("plugins/voice-input/plugin.toml")
    })
}

/// 工具（名字、说明、参数照搬进插件以前她的内置工具，玩家设过的权限和她的习惯都不变）。
pub fn specs() -> Vec<ToolSpec> {
    vec![ToolSpec::new(
        TOOL_SEND,
        "替玩家把一句话打进当前游戏的聊天框（按游戏配置的开聊天框键、频道与输入方式），默认按发送键（跟随用户的“自动发送”设置）。只在玩家明确要你替他说话或发送翻译时用；写成玩家本人的口吻，不加署名和引号。发出的内容会记在「自动化 → 记录」里。",
        json!({"type": "object", "additionalProperties": false, "required": ["text"], "properties": {
            "text": {"type": "string", "minLength": 1, "maxLength": 200, "description": "单行，不含换行"},
            "send": {"type": "boolean", "description": "缺省跟随用户设置；false 只打进聊天框不发送"}
        }}),
    )
    .side_effects(&["input", "records-run"])
    // 主会话（和玩家说话的那段）也拿：打一句字在一两秒内返回（ADR 029）
    .instant()]
}

/// 要打的字：去掉首尾空白；空的、超过 200 字、含换行的不打。
pub fn chat_text(arguments: &Value) -> Result<String, String> {
    let text = arguments
        .get("text")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("");
    if text.is_empty() || text.chars().count() > 200 || text.contains(['\r', '\n']) {
        return Err("要打的字为空、超过 200 字或含有换行".into());
    }
    Ok(text.to_string())
}

#[derive(Default)]
pub struct VoiceInputPlugin;

impl VoiceInputPlugin {
    pub fn new() -> Self {
        Self
    }
}

impl Plugin for VoiceInputPlugin {
    fn manifest(&self) -> &Manifest {
        manifest()
    }

    fn tools(&self) -> Vec<ToolSpec> {
        specs()
    }

    fn call<'a>(
        &'a self,
        host: Arc<dyn Host>,
        tool: &'a str,
        arguments: &'a Value,
    ) -> BoxFuture<'a, Result<ToolResult, String>> {
        Box::pin(async move {
            match tool {
                TOOL_SEND => {
                    let text = chat_text(arguments)?;
                    let send = arguments.get("send").and_then(Value::as_bool);
                    let receipt = host.type_in_game_chat(&text, send).await?;
                    Ok(ToolResult::content(receipt))
                }
                _ => Err(format!("语音输入插件没有 {tool}")),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_and_tools_agree() {
        let m = manifest();
        assert_eq!(m.id, "voice-input");
        assert!(m.enabled_by_default, "语音输入一直都能用：默认开");
        assert_eq!(
            m.tool_prefix(),
            "game_chat",
            "搬进插件以前就叫 game_chat.send"
        );
        m.check_tools(&specs()).unwrap();
        let send = &specs()[0];
        assert!(send.for_model() && send.instant && !send.untrusted);
        assert_eq!(send.side_effects, ["input", "records-run"]);
        assert!(m.pages.iter().any(|p| p.uri == PAGE));
    }

    #[test]
    fn chat_text_is_one_short_line() {
        assert_eq!(
            chat_text(&json!({"text": "  我去下路 "})).unwrap(),
            "我去下路"
        );
        for bad in [json!({}), json!({"text": "  "}), json!({"text": "gg\nwp"})] {
            assert!(chat_text(&bad).is_err(), "{bad}");
        }
        assert!(chat_text(&json!({"text": "字".repeat(201)})).is_err());
        assert!(chat_text(&json!({"text": "字".repeat(200)})).is_ok());
    }

    #[test]
    fn messages_match_the_manifest() {
        let zh: serde_json::Map<String, Value> =
            serde_json::from_str(include_str!("../messages/zh-CN.json")).unwrap();
        let en: serde_json::Map<String, Value> =
            serde_json::from_str(include_str!("../messages/en.json")).unwrap();
        let m = manifest();
        assert_eq!(zh["plugin_voice_input_name"], m.name.as_str());
        assert_eq!(zh["plugin_voice_input_description"], m.description.as_str());
        for key in zh.keys() {
            assert!(key.starts_with("plugin_voice_input_"), "{key}");
            assert!(en.contains_key(key), "en 缺 {key}");
        }
    }
}
