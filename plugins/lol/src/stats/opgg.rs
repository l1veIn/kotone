//! OP.GG 官方 MCP（`https://mcp-api.op.gg/mcp`，全球数据和对线指南）：英雄联盟插件的数据开关 `opgg`，**默认关**。
//!
//! 2026-11-24 替玩家在 opgginc/opgg-mcp 提了 issue #26 问能不能用（照 Glyph 的 #17）：说好的用法是
//! 按需请求（打开英雄页、她问的时候，不批量抓、不查召唤师）、最多 2 个并发、磁盘缓存 24 小时、
//! `client_app` 写 `kotone`、页面上放 OP.GG 的标志和链接、她用的时候说出处。OP.GG 回复同意前开关保持默认关。
//!
//! 协议：MCP Streamable HTTP（JSON-RPC：`initialize` → `notifications/initialized` → `tools/call`，
//! 会话 id 在 `Mcp-Session-Id` 头里）。返回的是文字（一种紧凑的类声明，不是 JSON），原样给页面和她。

use serde_json::{json, Value};

pub const ENDPOINT: &str = "https://mcp-api.op.gg/mcp";
pub const CLIENT_APP: &str = "kotone";
pub const HOME: &str = "https://op.gg";
/// 出处（页面和她说的时候用）。
pub const ATTRIBUTION: &str = "OP.GG（全球数据，op.gg）";

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

pub fn initialize_body(version: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
        "protocolVersion": "2025-06-18",
        "capabilities": {},
        "clientInfo": {"name": CLIENT_APP, "version": version},
    }})
}

pub fn initialized_body() -> Value {
    json!({"jsonrpc": "2.0", "method": "notifications/initialized"})
}

/// `tools/call`：参数里总带上 `client_app`（OP.GG 的说明要求）。
pub fn call_body(id: u64, tool: &str, mut arguments: Value, version: &str) -> Value {
    if let Some(map) = arguments.as_object_mut() {
        map.insert(
            "client_app".into(),
            json!(format!("{CLIENT_APP}/{version}")),
        );
    }
    json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {"name": tool, "arguments": arguments}})
}

/// 回复可能是 JSON，也可能是 SSE（`data: {...}`）：取出 JSON-RPC 的那一条。
pub fn parse_reply(content_type: &str, body: &str) -> Option<Value> {
    if content_type.contains("text/event-stream") {
        body.lines()
            .filter_map(|l| l.strip_prefix("data:"))
            .filter_map(|d| serde_json::from_str::<Value>(d.trim()).ok())
            .find(|v| v.get("result").is_some() || v.get("error").is_some())
    } else {
        serde_json::from_str(body).ok()
    }
}

/// `tools/call` 的结果 → 文字（`content` 里的 text 接起来）。出错时 Err。
pub fn result_text(reply: &Value) -> Result<String, String> {
    if let Some(e) = reply.get("error") {
        return Err(format!(
            "OP.GG：{}",
            e.get("message").and_then(Value::as_str).unwrap_or("出错了")
        ));
    }
    let result = reply.get("result").ok_or("OP.GG 没有返回结果")?;
    let text: Vec<&str> = result
        .get("content")
        .and_then(Value::as_array)
        .map(|c| {
            c.iter()
                .filter(|x| x.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|x| x.get("text").and_then(Value::as_str))
                .collect()
        })
        .unwrap_or_default();
    if result.get("isError").and_then(Value::as_bool) == Some(true) {
        return Err(format!("OP.GG：{}", text.join(" ")));
    }
    if text.is_empty() {
        return Err("OP.GG 返回的是空的".into());
    }
    Ok(text.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_bodies_and_replies() {
        assert_eq!(champion_key("MissFortune"), "MISS_FORTUNE");
        assert_eq!(champion_key("Irelia"), "IRELIA");
        assert_eq!(champion_key("Kaisa"), "KAISA");
        assert_eq!(position("MIDDLE"), "mid");
        let body = call_body(
            3,
            "lol_get_champion_analysis",
            json!({"champion": "IRELIA"}),
            "0.2.0",
        );
        assert_eq!(body["params"]["arguments"]["client_app"], "kotone/0.2.0");
        assert_eq!(
            initialize_body("0.2.0")["params"]["clientInfo"]["name"],
            "kotone"
        );

        let ok = parse_reply(
            "text/event-stream",
            "event: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":3,\"result\":{\"content\":[{\"type\":\"text\",\"text\":\"class Irelia {}\"}]}}\n\n",
        )
        .unwrap();
        assert_eq!(result_text(&ok).unwrap(), "class Irelia {}");
        let json_reply = parse_reply("application/json", r#"{"jsonrpc":"2.0","id":3,"result":{"content":[{"type":"text","text":"a"},{"type":"text","text":"b"}]}}"#).unwrap();
        assert_eq!(result_text(&json_reply).unwrap(), "a\nb");
        let err = parse_reply(
            "application/json",
            r#"{"jsonrpc":"2.0","id":3,"error":{"code":-32602,"message":"bad"}}"#,
        )
        .unwrap();
        assert!(result_text(&err).unwrap_err().contains("bad"));
        let tool_err = json!({"result": {"isError": true, "content": [{"type": "text", "text": "no such champion"}]}});
        assert!(result_text(&tool_err).unwrap_err().contains("no such"));
    }
}
