//! 按来源调一个工具：玩家设下的热键（ADR 040 第 3 期）和自动化程序里的 `tool()`（ADR 046 D1）共用这一份。
//!
//! 能不能调由调用方先判（热键 [`crate::hotkey_bind::state`]，程序 `automation::ToolRules`），这里只管经哪条路调：
//! 工具服务的经进程内的工具服务；本体的（`game_chat.send`）照“替玩家发言”的前台规矩（`Speaker::Automation`）；
//! 插件的经 `Caller::Automation`（第三方的也能调：是玩家在 Kotone 里设下的，不是转给外部 Agent）。
//! 结果整理成一样的形状（[`Reply`]）：给她的那份，和插件给的结构化结果。等多久、停下怎么办由调用方管。

use serde_json::Value;
use tauri::AppHandle;

use kotone_agent::catalog::{Source, ToolEntry};

/// 谁在调。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum By {
    /// 玩家按了热键。
    Hotkey,
    /// 自动化程序里的 `tool()`。
    Program,
}

impl By {
    /// 本体工具记下的调用方（发到游戏的记录里看得到）。
    fn caller(self) -> &'static str {
        match self {
            By::Hotkey => "hotkey",
            By::Program => "automation",
        }
    }

    fn service_not_ready(self) -> &'static str {
        match self {
            By::Hotkey => "工具服务还没准备好，过一会再按",
            By::Program => "工具服务还没准备好",
        }
    }

    fn not_bindable(self, tool: &str) -> String {
        match self {
            By::Hotkey => format!("{tool} 不能绑热键（只在和她的对话里用）"),
            By::Program => format!("{tool} 只在和她的对话里用，程序里调不了"),
        }
    }
}

/// 工具回来的东西。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Reply {
    /// 给她的那份：工具服务的 `result`（没有时是 null），插件、本体的 `content`。
    pub content: Value,
    /// 插件、本体给的结构化结果（`structuredContent`，例如 `lol.match` 给程序的那份）；工具服务的没有。
    pub structured: Option<Value>,
}

impl Reply {
    /// 给程序的那份（ADR 046 D1）：有结构化结果用它，没有用给她的那份。
    pub(crate) fn for_program(&self) -> &Value {
        self.structured.as_ref().unwrap_or(&self.content)
    }
}

impl From<kotone_plugin_api::ToolResult> for Reply {
    fn from(result: kotone_plugin_api::ToolResult) -> Self {
        Self {
            content: result.content,
            structured: result.structured_content,
        }
    }
}

/// 调 `entry` 这个工具。`Err` 是工具自己说没成（工具服务 ok = false、插件返回错误），或者这条路走不通。
pub(crate) async fn call(
    app: &AppHandle,
    entry: &ToolEntry,
    args: Value,
    by: By,
) -> Result<Reply, String> {
    let tool = entry.name.as_str();
    match &entry.source {
        Source::Service => {
            let service = crate::tool_service::service(app).ok_or(by.service_not_ready())?;
            let envelope = service
                .call(kotone_tools::protocol::ToolCallRequest {
                    request_id: None,
                    tool: tool.to_string(),
                    arguments: args,
                    timeout_ms: None,
                })
                .await;
            if envelope.ok {
                Ok(Reply {
                    content: envelope.result.unwrap_or(Value::Null),
                    structured: None,
                })
            } else {
                Err(envelope
                    .error
                    .map(|e| e.message)
                    .unwrap_or_else(|| format!("{tool} 没成")))
            }
        }
        Source::Body => crate::game_chat::body_tool(
            app,
            tool,
            &args,
            by.caller(),
            kotone_core::send_to_game::Speaker::Automation,
            None,
        )
        .await
        .map(Reply::from),
        Source::Plugin { .. } => {
            crate::plugins::call(app, tool, &args, crate::plugins::Caller::Automation)
                .await
                .map(Reply::from)
        }
        _ => Err(by.not_bindable(tool)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 程序拿到的是结构化结果（`lol.match` 的那份），没有就是给她的那份；热键弹幕那一行看给她的那份。
    #[test]
    fn programs_get_the_structured_result_when_there_is_one() {
        let mut result = kotone_plugin_api::ToolResult::content(json!("这局 12:30，3 比 5"));
        result.structured_content = Some(json!({"status": "ok", "gameTime": 750}));
        let reply = Reply::from(result);
        assert_eq!(reply.for_program()["gameTime"], 750);
        assert_eq!(reply.content, json!("这局 12:30，3 比 5"));
        let plain = Reply::from(kotone_plugin_api::ToolResult::content(
            json!({"status": "need_login"}),
        ));
        assert_eq!(plain.for_program(), &json!({"status": "need_login"}));
    }

    /// 热键的说法一点不变（「过一会再按」「不能绑热键」），程序的说法不提热键。
    #[test]
    fn hotkeys_and_programs_word_it_their_own_way() {
        assert_eq!(
            By::Hotkey.service_not_ready(),
            "工具服务还没准备好，过一会再按"
        );
        assert!(!By::Program.service_not_ready().contains("按"));
        assert_eq!(
            By::Hotkey.not_bindable("speak"),
            "speak 不能绑热键（只在和她的对话里用）"
        );
        assert!(By::Program.not_bindable("speak").contains("程序里调不了"));
        assert_eq!(
            (By::Hotkey.caller(), By::Program.caller()),
            ("hotkey", "automation")
        );
    }
}
