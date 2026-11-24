//! Streamable HTTP：rmcp 的 reqwest 客户端，加密用 ring（和 kotone-ai 一样，不引入 aws-lc）。
//!
//! 一个连接一个会话（`Mcp-Session-Id`），之后的请求都带着它；服务器说会话过期（404）时 rmcp 自己重新握手一次
//! （`reinit_on_expired_session`，默认开）。不给 reqwest 设总超时：服务器往回推消息的 GET 流是长连接，
//! 每次请求的超时由 [`crate::McpClient`] 管。

use std::collections::{BTreeMap, HashMap};
use std::time::Duration;

use http::{HeaderName, HeaderValue};
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::StreamableHttpClientTransport;

use crate::McpError;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

pub(crate) fn transport(
    url: &str,
    headers: &BTreeMap<String, String>,
    user_agent: &str,
) -> Result<StreamableHttpClientTransport<reqwest::Client>, McpError> {
    let lower = url.trim().to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://")) {
        return Err(McpError::Config(format!(
            "地址要以 http:// 或 https:// 开头：{url}"
        )));
    }
    let mut custom = HashMap::new();
    for (name, value) in headers {
        let header = HeaderName::from_bytes(name.trim().as_bytes())
            .map_err(|_| McpError::Config(format!("请求头的名字不合法：{name}")))?;
        let value = HeaderValue::from_str(value.trim())
            .map_err(|_| McpError::Config(format!("请求头 {name} 的值不合法")))?;
        custom.insert(header, value);
    }
    // reqwest 0.13 用进程默认的 rustls 加密提供者：装上 ring（已经装过别的就沿用）。
    let _ = rustls::crypto::ring::default_provider().install_default();
    let client = reqwest::Client::builder()
        .user_agent(user_agent)
        .connect_timeout(CONNECT_TIMEOUT)
        .build()
        .map_err(|e| McpError::Connect(crate::error::chain(&e)))?;
    let config = StreamableHttpClientTransportConfig::with_uri(url.trim().to_string())
        .custom_headers(custom);
    Ok(StreamableHttpClientTransport::with_client(client, config))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn bad_urls_and_headers_are_config_errors() {
        let none = BTreeMap::new();
        assert!(matches!(
            transport("ftp://x", &none, "kotone"),
            Err(McpError::Config(_))
        ));
        let mut bad = BTreeMap::new();
        bad.insert("bad header".to_string(), "v".to_string());
        assert!(matches!(
            transport("https://example.com/mcp", &bad, "kotone"),
            Err(McpError::Config(_))
        ));
    }
}
