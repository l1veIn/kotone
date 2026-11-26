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
}
