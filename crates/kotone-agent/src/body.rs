//! 本体用统一的 [`ToolSpec`] 登记的工具（ADR 038“怎么定的”：本体的工具也走统一的 `ToolSpec` 登记；
//! ADR 039 D2；原则 15）。和插件的工具同一种写法：名字、说明、参数、副作用、马上返回（`instant`：主会话也拿）
//! 都写在这里，不另写名单。不属于哪个插件，关掉哪个插件都还在。
//!
//! - 琴音：[`crate::catalog::Catalog::build`] 把它们放进工具清单（[`crate::catalog::Source::Body`]），
//!   调用经 [`crate::port::BuiltinHost::body_tool`] 交给桌面，执行记录的发起方是这段对话（`agent:<对话 id>`）；
//! - 外部 Agent：桌面端把 [`service_specs`] 列进工具服务的 `system.capabilities`（MCP 也就有了），
//!   调用经工具服务交给桌面，发起方记成 `tool-service`。两条路在桌面端走同一个实现。
//!
//! 现在只有 `game_chat.send`：往游戏聊天里打字归本体，语音输入插件借用同一套打字（ADR 039 D2）。
//! 别的本体工具还是 `catalog.rs` 里的条目和工具服务的目录，以后照这里挪。

use kotone_plugin_api::ToolSpec;
use serde_json::{json, Value};

/// 替玩家往游戏聊天里发一句。名字不变：玩家设过的权限、外部 Agent 都认它（ADR 039 D2 第 4 问）。
pub const TOOL_GAME_CHAT: &str = "game_chat.send";

/// 本体用 ToolSpec 登记的全部工具。
pub fn tools() -> Vec<ToolSpec> {
    vec![ToolSpec::new(
        TOOL_GAME_CHAT,
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

/// 这个名字是不是本体用 ToolSpec 登记的工具。
pub fn owns(name: &str) -> bool {
    tools().iter().any(|t| t.name == name)
}

/// 工具服务 `system.capabilities` 里的样子（给外部 Agent、MCP）：和插件的工具一样，只是没有 `plugin`。
pub fn service_specs() -> Vec<Value> {
    tools().iter().map(ToolSpec::body_service_spec).collect()
}

/// `game_chat.send` 要打的字：去掉首尾空白；空的、超过 200 字、含换行的不打。
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 说明、参数、副作用、马上返回都和以前（她的内置工具 → 语音输入插件的工具）一样。
    #[test]
    fn game_chat_send_is_registered_like_before() {
        let tools = tools();
        assert_eq!(tools.len(), 1);
        let send = &tools[0];
        assert_eq!(send.name, "game_chat.send");
        assert!(send.for_model() && send.instant && !send.untrusted);
        assert_eq!(send.side_effects, ["input", "records-run"]);
        assert!(send.needs.is_empty() && send.page.is_none());
        assert_eq!(send.parameters["required"], json!(["text"]));
        assert!(owns("game_chat.send") && !owns("game_chat") && !owns("companion.type"));
        let spec = &service_specs()[0];
        assert_eq!(spec["name"], "game_chat.send");
        assert_eq!(spec["sideEffects"], json!(["input", "records-run"]));
        assert!(spec.get("plugin").is_none(), "本体的工具不属于哪个插件");
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
}
