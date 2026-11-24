//! 对外的类型：协议原样的字段（规范 2025-11-25 的 `Tool`、`Resource`、`CallToolResult`……），子对象留成 JSON。
//! 从 rmcp 的类型转过来时走一遍 JSON（`serde_json::to_value` → `from_value`），所以 rmcp 改字段名、加字段都碰不到调用方。

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::McpError;

/// `tools/list` 的一项。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tool {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// 参数的 JSON Schema。
    #[serde(default)]
    pub input_schema: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<Value>,
    /// `readOnlyHint` 这些（规范：注解默认不可信，只改善体验，不当安全边界）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotations: Option<Value>,
    /// `_meta`（MCP Apps 的 `_meta.ui.resourceUri`、`visibility` 在这里）。
    #[serde(rename = "_meta", default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<Value>,
}

/// `resources/list` 的一项。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Resource {
    pub uri: String,
    #[serde(default)]
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[serde(rename = "_meta", default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<Value>,
}

/// `resources/read` 读回来的一份内容：文字（`text`）或二进制（`blob`，base64）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceContents {
    pub uri: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blob: Option<String>,
    /// MCP Apps 的 `_meta.ui`（CSP 要连的域名、权限……）在这里。
    #[serde(rename = "_meta", default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<Value>,
}

/// `tools/call` 的结果。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallResult {
    /// 内容块（`{"type": "text", "text": …}`、图片、资源链接……）。
    #[serde(default)]
    pub content: Vec<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured_content: Option<Value>,
    /// 工具自己说失败了（不是协议错误）。
    #[serde(default)]
    pub is_error: bool,
    #[serde(rename = "_meta", default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<Value>,
}

impl ToolCallResult {
    /// 所有文字块接起来（一块一行）。
    pub fn text(&self) -> String {
        self.content
            .iter()
            .filter(|c| c.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|c| c.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// 握手时服务器说的自己。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServerInfo {
    pub name: String,
    pub version: String,
    /// 给人看的名字（`serverInfo.title`，2025-06-18 起）。
    pub title: Option<String>,
    /// 一句话说明（`serverInfo.description`，2025-11-25 起）。
    pub description: Option<String>,
    /// 图标（`serverInfo.icons`，2025-11-25 起）：协议原样的 `{src, mimeType?, sizes?, theme?}`。
    pub icons: Vec<Value>,
    /// 主页（`serverInfo.websiteUrl`）。
    pub website_url: Option<String>,
    /// 服务器给模型的说明（`instructions`）。
    pub instructions: Option<String>,
    /// 谈好的协议版本（例如 `2025-11-25`）。
    pub protocol_version: String,
}

/// rmcp 的类型 → 这里的类型（经 JSON）。
pub(crate) fn convert<T: DeserializeOwned>(value: &impl Serialize) -> Result<T, McpError> {
    serde_json::to_value(value)
        .and_then(serde_json::from_value)
        .map_err(|e| McpError::Protocol(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn wire_fields() {
        let tool: Tool = serde_json::from_value(json!({
            "name": "lol_get_champion_analysis",
            "description": "d",
            "inputSchema": {"type": "object"},
            "annotations": {"readOnlyHint": true},
            "_meta": {"ui": {"resourceUri": "ui://x/y"}}
        }))
        .unwrap();
        assert_eq!(tool.meta.unwrap()["ui"]["resourceUri"], "ui://x/y");
        let result: ToolCallResult = serde_json::from_value(json!({
            "content": [{"type": "text", "text": "a"}, {"type": "image", "data": "", "mimeType": "image/png"}, {"type": "text", "text": "b"}],
            "isError": true
        }))
        .unwrap();
        assert!(result.is_error);
        assert_eq!(result.text(), "a\nb");
        let empty: ToolCallResult = serde_json::from_value(json!({})).unwrap();
        assert!(!empty.is_error && empty.text().is_empty());
    }
}
