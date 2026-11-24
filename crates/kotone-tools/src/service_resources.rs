//! MCP App 资源（ADR 039 D5；MCP 方案第 3、5 期）：开着的插件做成独立 HTML 的页面。
//!
//! - 列表放在 capabilities 的 `resources` 里（和工具同一次取到：`kotone.exe mcp` 拿它决定工具的
//!   `_meta.ui.resourceUri` 带不带）；
//! - 读一个：`GET /v1/resources/read?uri=…`，结果是 MCP `resources/read` 的样子（`{contents: [...]}`）。
//!
//! 都转给桌面宿主（[`crate::host::AppHost::resources`] / [`crate::host::AppHost::read_resource`]）；
//! 没有桌面宿主（纯 CLI 测试）时没有资源。读资源不占工具执行名额（内容编在插件里，马上返回），
//! 但有期限：宿主 10 秒不回就报超时。

use super::*;

use crate::host::HostError;

/// 读一个资源最多等多久。
const READ_TIMEOUT: Duration = Duration::from_secs(10);

impl ToolService {
    /// 开着的插件做成 MCP App 的页面（`resources/list` 的项）。
    pub fn resources(&self) -> Vec<Value> {
        if self.host.supports_invoke() {
            self.host.resources()
        } else {
            Vec::new()
        }
    }

    /// `GET /v1/resources/read?uri=…`：结果 `{contents: [{uri, mimeType, text, _meta}]}`。
    pub async fn read_resource_http(self: &Arc<Self>, query: &str, request_id: String) -> Envelope {
        let started = Instant::now();
        if self.store.is_closed() {
            return self.envelope_failure(request_id, None, closed_error(), started);
        }
        let uri = match resource_uri(query) {
            Ok(uri) => uri,
            Err(e) => return self.envelope_failure(request_id, None, invalid_args(e), started),
        };
        if !self.host.supports_invoke() {
            return self.envelope_failure(request_id, None, not_found(&uri), started);
        }
        let read = tokio::time::timeout(READ_TIMEOUT, self.host.read_resource(&uri)).await;
        let result = match read {
            Err(_) => Err(ToolError::new(
                ErrorCode::Timeout,
                format!("读资源 {uri} 超过 {} 秒没回来", READ_TIMEOUT.as_secs()),
            )),
            Ok(Err(HostError::InvalidArguments(message))) => {
                Err(ToolError::new(ErrorCode::ResourceNotFound, message))
            }
            Ok(Err(HostError::Busy(message))) => Err(ToolError::new(ErrorCode::Busy, message)),
            Ok(Err(HostError::Failed(message))) => {
                Err(ToolError::new(ErrorCode::HostActionFailed, message))
            }
            Ok(Ok(content)) => Ok(json!({ "contents": [content] })),
        };
        match result {
            Ok(value) => Envelope::success(
                request_id,
                Some(self.instance_id.clone()),
                None,
                value,
                Timing {
                    total_ms: ms_since(started),
                    ..Default::default()
                },
            ),
            Err(error) => self.envelope_failure(request_id, None, error, started),
        }
    }
}

fn not_found(uri: &str) -> ToolError {
    ToolError::new(ErrorCode::ResourceNotFound, format!("没有资源 {uri}"))
}

/// 查询串里的 `uri`（只认这一个参数）：要是 `ui://` 地址，不超过 300 个字符。
fn resource_uri(query: &str) -> Result<String, String> {
    let mut uri = None;
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        match k {
            "uri" => uri = Some(super::app::percent_decode(v)?),
            other => return Err(format!("未知查询参数 {other}")),
        }
    }
    let uri = uri.ok_or("要写 uri（例如 ui://kotone/lol/tier）")?;
    if !uri.starts_with("ui://") || uri.len() > 300 || uri.chars().any(char::is_whitespace) {
        return Err(format!("uri 要是 ui:// 地址：{uri}"));
    }
    Ok(uri)
}

#[cfg(test)]
mod tests {
    use super::resource_uri;

    #[test]
    fn uri_comes_from_the_query() {
        assert_eq!(
            resource_uri("uri=ui%3A%2F%2Fkotone%2Flol%2Ftier").unwrap(),
            "ui://kotone/lol/tier"
        );
        assert_eq!(
            resource_uri("uri=ui://kotone/lol/tier").unwrap(),
            "ui://kotone/lol/tier"
        );
        assert!(resource_uri("").is_err());
        assert!(resource_uri("uri=https%3A%2F%2Fexample.com").is_err());
        assert!(resource_uri("uri=ui://a&x=1").is_err());
        assert!(resource_uri("uri=ui%3A%2F%2Fa%20b").is_err());
    }
}
