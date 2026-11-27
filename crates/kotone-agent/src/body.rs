//! 本体用统一的 [`ToolSpec`] 登记的工具（ADR 038“怎么定的”：本体的工具也走统一的 `ToolSpec` 登记；
//! ADR 039 D2；原则 15）。和插件的工具同一种写法：名字、说明、参数、副作用、马上返回（`instant`：主会话也拿）
//! 都写在这里，不另写名单。不属于哪个插件，关掉哪个插件都还在。
//!
//! - 琴音：[`crate::catalog::Catalog::build`] 把它们放进工具清单（[`crate::catalog::Source::Body`]），
//!   调用经 [`crate::port::BuiltinHost::body_tool`] 交给桌面，执行记录的发起方是这段对话（`agent:<对话 id>`）；
//! - 外部 Agent：桌面端把 [`service_specs`] 列进工具服务的 `system.capabilities`（MCP 也就有了），
//!   调用经工具服务交给桌面，发起方记成 `tool-service`。两条路在桌面端走同一个实现。
//!
//! 现在只有 `game_chat.send`：往游戏聊天里发话归本体，它就是“发到游戏”的入口（ADR 041 D1，
//! `kotone_core::send_to_game`），语音输入插件、自动化、旧工具 `companion.type` 都经同一个入口（ADR 039 D2）。
//! 别的本体工具还是 `catalog.rs` 里的条目和工具服务的目录，以后照这里挪。

use kotone_core::send_to_game::{
    ChannelPick, ChatRequest, Speaker, CHANNEL_ALL, CHANNEL_TEAM, MAX_LINES, MAX_LINE_CHARS,
    MAX_TEXT_CHARS,
};
use kotone_plugin_api::ToolSpec;
use serde_json::{json, Value};

/// 替玩家往游戏聊天里发一句。名字不变：玩家设过的权限、外部 Agent 都认它（ADR 039 D2 第 4 问）。
pub const TOOL_GAME_CHAT: &str = "game_chat.send";

/// 本体用 ToolSpec 登记的全部工具。
pub fn tools() -> Vec<ToolSpec> {
    vec![ToolSpec::new(
        TOOL_GAME_CHAT,
        &format!(
            "替玩家把话发到当前游戏的聊天（按前台那个游戏的游戏配置：开聊天框键、频道与时序），默认发到队伍频道、按发送键（跟随用户的“自动发送”设置）。只在玩家明确要你替他说话或发送翻译时用；写成玩家本人的口吻，不加署名和引号。要发几条（例如战绩一人一条）就写成几行，一行发一条（最多 {MAX_LINES} 行，每行最多 {MAX_LINE_CHARS} 字）。英雄联盟客户端在前台时（选英雄、房间里）经英雄联盟插件走客户端的聊天接口发，不按键，几行连成一条，那里只有队伍频道（自定义对局两边都看得到）、不能只打进聊天框不发。结果说走了哪条路（route：keys 按键 / console 控制台 / client 客户端的聊天接口）、发到哪个频道、发了几行；没发出去会说原因（例如前台不是游戏、修饰键一直按着、正在发别的），不要自己重发同一句。"
        ),
        json!({"type": "object", "additionalProperties": false, "required": ["text"], "properties": {
            "text": {"type": "string", "minLength": 1, "maxLength": MAX_TEXT_CHARS, "description": format!("要发的话；多行时一行发一条（最多 {MAX_LINES} 行，每行最多 {MAX_LINE_CHARS} 字）")},
            "channel": {"type": "string", "enum": [CHANNEL_TEAM, CHANNEL_ALL], "description": "team = 队伍（缺省），all = 所有人（对面也看得到）；游戏配置里没有“所有人”频道时不发"},
            "send": {"type": "boolean", "description": "缺省跟随用户设置；false 只打进聊天框不发送（只能一行）"}
        }}),
    )
    // 不再写执行记录（2026-11-27 玩家：那不是自动化），副作用只有按键 / 发话
    .side_effects(&["input"])
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

/// `game_chat.send` 的参数 → 发到游戏的入口的请求。分行、行数、每行字数在入口里查（只有一处规矩）；
/// `auto_send` = 玩家的“自动发送”设置（`send` 缺省时用）。截止时间、取消由调用方补。
pub fn chat_request(
    arguments: &Value,
    auto_send: bool,
    speaker: Speaker,
) -> Result<ChatRequest, String> {
    let text = arguments.get("text").and_then(Value::as_str).unwrap_or("");
    if text.trim().is_empty() {
        return Err("要发的话是空的".into());
    }
    if text.chars().count() > MAX_TEXT_CHARS {
        return Err(format!("要发的话超过 {MAX_TEXT_CHARS} 字"));
    }
    let channel = match arguments.get("channel") {
        None | Some(Value::Null) => None,
        Some(Value::String(id)) if id == CHANNEL_TEAM || id == CHANNEL_ALL => Some(ChannelPick {
            id: id.clone(),
            for_game: None,
        }),
        Some(other) => {
            return Err(format!(
                "channel 只能是 team（队伍）或 all（所有人），不是 {other}"
            ))
        }
    };
    let send = match arguments.get("send") {
        None | Some(Value::Null) => auto_send,
        Some(Value::Bool(send)) => *send,
        Some(other) => return Err(format!("send 只能是 true 或 false，不是 {other}")),
    };
    let mut request = ChatRequest::new(text, speaker);
    request.channel = channel;
    request.send = send;
    Ok(request)
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
        assert_eq!(send.side_effects, ["input"]);
        assert!(send.needs.is_empty() && send.page.is_none());
        assert_eq!(send.parameters["required"], json!(["text"]));
        assert!(owns("game_chat.send") && !owns("game_chat") && !owns("companion.type"));
        let spec = &service_specs()[0];
        assert_eq!(spec["name"], "game_chat.send");
        assert_eq!(spec["sideEffects"], json!(["input"]));
        assert!(spec.get("plugin").is_none(), "本体的工具不属于哪个插件");
    }

    #[test]
    fn game_chat_send_takes_a_channel_and_several_lines() {
        let send = &tools()[0];
        let props = &send.parameters["properties"];
        assert_eq!(props["channel"]["enum"], json!(["team", "all"]));
        assert_eq!(props["text"]["maxLength"], json!(2000));
        assert!(send.description.contains("一行发一条"));
        // ADR 041 第 2 期：说明里讲清选英雄、房间里走客户端的聊天接口，和 route 的三个值
        assert!(
            send.description.contains("客户端的聊天接口") && send.description.contains("client")
        );
        // 加的都是可选参数：只给 text 的旧调用照旧
        assert_eq!(send.parameters["required"], json!(["text"]));
    }

    #[test]
    fn chat_request_reads_text_channel_and_send() {
        let r = chat_request(
            &json!({"text": "亚索 2/0/1\n劫 0/2/0"}),
            true,
            Speaker::Companion,
        )
        .unwrap();
        assert_eq!(r.text, "亚索 2/0/1\n劫 0/2/0", "分行交给入口");
        assert!(r.send && r.channel.is_none());
        assert_eq!(r.speaker, Speaker::Companion);
        let r = chat_request(
            &json!({"text": "gg", "channel": "all", "send": false}),
            true,
            Speaker::Agent,
        )
        .unwrap();
        assert_eq!(r.channel.unwrap().id, "all");
        assert!(!r.send);
        // send 缺省跟随玩家设置
        assert!(
            !chat_request(&json!({"text": "gg"}), false, Speaker::Agent)
                .unwrap()
                .send
        );
        for bad in [
            json!({}),
            json!({"text": "  \n "}),
            json!({"text": "gg", "channel": "party"}),
            json!({"text": "gg", "send": "yes"}),
        ] {
            assert!(chat_request(&bad, true, Speaker::Agent).is_err(), "{bad}");
        }
        assert!(chat_request(&json!({"text": "字".repeat(2001)}), true, Speaker::Agent).is_err());
    }
}
