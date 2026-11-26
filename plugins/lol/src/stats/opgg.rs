//! OP.GG 官方 MCP（`https://mcp-api.op.gg/mcp`，全球数据和对线指南）：英雄联盟插件的数据开关 `opgg`，**默认开**
//! （ADR 043 D9，玩家 2026-11-26 决定：“直接用”，不再等 issue 的回复）。
//!
//! 2026-11-24 替玩家在 opgginc/opgg-mcp 提了 issue #26 问能不能用（照 Glyph 的 #17），里面答应的用法照旧：
//! 按需请求（页面上把来源换成 OP.GG、她问的时候，不批量抓、不查召唤师）、最多 2 个并发（[`super::OPGG_CONCURRENCY`]，
//! 真的限住了）、磁盘缓存 24 小时、`client_app` 写 `kotone`、页面上放 OP.GG 的标志和链接（App 的来源标签）、
//! 她用的时候说出处。
//!
//! 协议：MCP Streamable HTTP，经本体的 MCP 客户端 `kotone-mcp`（MCP 方案 4.7：OP.GG 是第一个用户）。
//! 第一次调用时握手一次（`initialize` → `notifications/initialized`），之后的调用复用这个会话，一次调用一个请求。
//! 返回的是文字（一种紧凑的类声明，不是 JSON），原样给页面和她。

use std::time::Duration;

use kotone_mcp::{McpError, ServerConfig, ToolCallResult};
use serde_json::{json, Value};

pub const ENDPOINT: &str = "https://mcp-api.op.gg/mcp";
pub const CLIENT_APP: &str = "kotone";
pub const HOME: &str = "https://op.gg";
/// 出处（页面和她说的时候用）。
pub const ATTRIBUTION: &str = "OP.GG（全球数据，op.gg）";
/// 握手和每次调用最多等多久（和以前每个请求的 15 秒一样）。
const TIMEOUT: Duration = Duration::from_secs(15);

/// 英雄的英文 id → OP.GG 要的写法（`MissFortune` → `MISS_FORTUNE`）。
pub fn champion_key(alias: &str) -> String {
    let mut out = String::new();
    for (i, c) in alias.chars().enumerate() {
        if c.is_ascii_uppercase() && i > 0 {
            out.push('_');
        }
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_uppercase());
        }
    }
    out
}

/// 101 的分路 → OP.GG 的。
pub fn position(lane: &str) -> &'static str {
    match lane {
        "TOP" => "top",
        "JUNGLE" => "jungle",
        "MIDDLE" => "mid",
        "BOTTOM" => "adc",
        "SUPPORT" => "support",
        _ => "top",
    }
}

/// 对线指南（只有 OP.GG 有，ADR 043 D8“101 没有的用 OP.GG”）：选人卡片锁定以后的“对线”那块。
pub const LANE_MATCHUP_TOOL: &str = "lol_get_lane_matchup_guide";

/// `lol_get_lane_matchup_guide` 的参数（`champion_key` 过的英文 id、OP.GG 的分路）。
pub fn lane_matchup_arguments(my_champion: &str, opponent: &str, position: &str) -> Value {
    json!({"position": position, "my_champion": my_champion, "opponent_champion": opponent, "lang": "zh_CN"})
}

/// 对线指南里的几条事实（OP.GG 的原话，插件不加判断）：谁对线占优、谁单杀占优（`me` / `opponent`）、
/// 建议的打法（OP.GG 的词：`even` 等）、对这个对手的提示（OP.GG 给的原文，常常是英文）。
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LaneMatchup {
    pub lane_advantage: Option<&'static str>,
    pub solo_kill_advantage: Option<&'static str>,
    pub play_style: Option<String>,
    pub tip: Option<String>,
}

impl LaneMatchup {
    pub fn is_empty(&self) -> bool {
        self.lane_advantage.is_none()
            && self.solo_kill_advantage.is_none()
            && self.play_style.is_none()
            && self.tip.is_none()
    }
}

/// 读 `lol_get_lane_matchup_guide` 回的文字（2026-11-26 实测是一段 JSON：`my_champion` / `opponent_champion` 是
/// 中文称号，`data.lane_advantage_champion` 等写的也是称号）。读不懂时 None。
pub fn parse_lane_matchup(text: &str) -> Option<LaneMatchup> {
    let v: Value = serde_json::from_str(text.trim()).ok()?;
    let name = |key: &str| {
        v.get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or("")
            .to_string()
    };
    let (me, opponent) = (name("my_champion"), name("opponent_champion"));
    let data = v.get("data")?;
    let side = |key: &str| -> Option<&'static str> {
        let who = data.get(key)?.as_str()?.trim();
        if who.is_empty() {
            None
        } else if who == me {
            Some("me")
        } else if who == opponent {
            Some("opponent")
        } else {
            None
        }
    };
    let text_of = |key: &str| {
        data.get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_string)
    };
    let tip = text_of("opponent_champion_tip").map(|t| {
        // 太长的截一下（卡片很窄）；不截半个字
        let mut out: String = t.chars().take(400).collect();
        if t.chars().count() > 400 {
            out.push('…');
        }
        out
    });
    Some(LaneMatchup {
        lane_advantage: side("lane_advantage_champion"),
        solo_kill_advantage: side("lane_solo_kill_advantage_champion"),
        play_style: text_of("recommended_play_style"),
        tip,
    })
}

/// 连 OP.GG 的配置：`clientInfo` 写 `kotone` 和 Kotone 的版本。
pub fn config(endpoint: &str, version: &str) -> ServerConfig {
    ServerConfig::http(endpoint)
        .client(CLIENT_APP, version)
        .startup_timeout(TIMEOUT)
        .tool_timeout(TIMEOUT)
}

/// `tools/call` 的参数：总带上 `client_app`（OP.GG 的说明要求）。
pub fn arguments(mut arguments: Value, version: &str) -> Value {
    if let Some(map) = arguments.as_object_mut() {
        map.insert(
            "client_app".into(),
            json!(format!("{CLIENT_APP}/{version}")),
        );
    }
    arguments
}

/// `tools/call` 的结果 → 文字（`content` 里的 text 接起来）。工具说失败了、或者是空的时 Err。
pub fn result_text(result: &ToolCallResult) -> Result<String, String> {
    let text = result.text();
    if result.is_error {
        return Err(format!("OP.GG：{}", text.replace('\n', " ")));
    }
    if text.is_empty() {
        return Err("OP.GG 返回的是空的".into());
    }
    Ok(text)
}

/// 连接 / 协议出错时的说法（和以前手写连接时一样）。
pub fn describe(error: &McpError) -> String {
    match error {
        McpError::Http(status) => format!("OP.GG 返回 {}", kotone_mcp::status_text(*status)),
        McpError::Server { message, .. } => format!("OP.GG：{message}"),
        McpError::Protocol(_) => "OP.GG 返回的不是 JSON-RPC".into(),
        McpError::Connect(m) | McpError::Config(m) | McpError::Spawn(m) => {
            format!("连不上 OP.GG：{m}")
        }
        McpError::Timeout { after, .. } => {
            format!("连不上 OP.GG：{} 秒没有回应", after.as_secs())
        }
        McpError::Closed => "连不上 OP.GG：连接断了".into(),
    }
}

/// 这种错误以后要重新连（会话没了、连接断了）。
pub fn should_reconnect(error: &McpError) -> bool {
    !matches!(error, McpError::Server { .. })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_arguments_and_results() {
        assert_eq!(champion_key("MissFortune"), "MISS_FORTUNE");
        assert_eq!(champion_key("Irelia"), "IRELIA");
        assert_eq!(champion_key("Kaisa"), "KAISA");
        assert_eq!(position("MIDDLE"), "mid");
        let args = arguments(json!({"champion": "IRELIA"}), "0.2.0");
        assert_eq!(args["client_app"], "kotone/0.2.0");
        assert_eq!(args["champion"], "IRELIA");
        let config = config(ENDPOINT, "0.2.0");
        assert_eq!(config.client_name, "kotone");
        assert_eq!(config.client_version, "0.2.0");

        // 录好的回复格式（OP.GG 回的 `result`）
        let ok: ToolCallResult = serde_json::from_value(json!({
            "content": [{"type": "text", "text": "class Irelia {}"}]
        }))
        .unwrap();
        assert_eq!(result_text(&ok).unwrap(), "class Irelia {}");
        let two: ToolCallResult = serde_json::from_value(json!({
            "content": [{"type": "text", "text": "a"}, {"type": "text", "text": "b"}]
        }))
        .unwrap();
        assert_eq!(result_text(&two).unwrap(), "a\nb");
        let tool_err: ToolCallResult = serde_json::from_value(json!({
            "isError": true, "content": [{"type": "text", "text": "no such champion"}]
        }))
        .unwrap();
        assert!(result_text(&tool_err).unwrap_err().contains("no such"));
        assert_eq!(
            result_text(&ToolCallResult::default()).unwrap_err(),
            "OP.GG 返回的是空的"
        );

        // 出错时的说法
        let rpc = McpError::Server {
            code: -32602,
            message: "bad".into(),
        };
        assert_eq!(describe(&rpc), "OP.GG：bad");
        assert!(!should_reconnect(&rpc));
        assert_eq!(describe(&McpError::Http(502)), "OP.GG 返回 502 Bad Gateway");
        assert!(describe(&McpError::Connect("dns".into())).starts_with("连不上 OP.GG："));
        assert!(should_reconnect(&McpError::Closed));
    }

    /// 对线指南（照 2026-11-26 真调的一次：亚索对劫，中路）：谁占优写成 me / opponent，提示原样留着，读不懂的是 None。
    #[test]
    fn lane_matchup_facts() {
        let text = json!({
            "lang": "zh_CN", "position": "mid", "my_champion": "疾风剑豪", "opponent_champion": "影流之主",
            "data": {
                "summary": {"id": 157},
                "opponent_champion_tip": "Do not stand in range of Zed's Living Shadow (W)!",
                "lane_solo_kill_advantage_champion": "影流之主",
                "lane_advantage_champion": "疾风剑豪",
                "recommended_play_style": "even"
            }
        })
        .to_string();
        let m = parse_lane_matchup(&text).unwrap();
        assert_eq!(m.lane_advantage, Some("me"));
        assert_eq!(m.solo_kill_advantage, Some("opponent"));
        assert_eq!(m.play_style.as_deref(), Some("even"));
        assert!(m.tip.unwrap().starts_with("Do not stand"));
        assert!(parse_lane_matchup("class X {}").is_none());
        let empty = parse_lane_matchup(&json!({"data": {}}).to_string()).unwrap();
        assert!(empty.is_empty());
        let args = lane_matchup_arguments("YASUO", "ZED", "mid");
        assert_eq!(args["opponent_champion"], "ZED");
        assert_eq!(args["lang"], "zh_CN");
    }
}
