//! 出错时说人话（原则 21）：哪一步、为什么、下一步怎么办。调用方（例如英雄联盟插件的 OP.GG）可以按种类换成自己的说法。

use std::error::Error as StdError;
use std::fmt;
use std::time::Duration;

/// 超时发生在哪一步。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// 起程序 + 握手。
    Startup,
    /// 一次请求（调工具、列工具、读资源）。
    Request,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpError {
    /// 配置不对（没写程序、地址不是 http(s)、请求头不合法、参数不是 JSON 对象……）。
    Config(String),
    /// 程序没起来，或者起来以后握手前就退出了（带它最后在 stderr 说的话）。
    Spawn(String),
    /// 连不上（网络、TLS、对方断开……）。
    Connect(String),
    /// HTTP 服务器回了错误的状态码。
    Http(u16),
    /// 等太久了。
    Timeout { stage: Stage, after: Duration },
    /// 连接已经断了（程序退出了、会话没了、调用过 `close`）。
    Closed,
    /// 服务器回了 JSON-RPC 错误。
    Server { code: i32, message: String },
    /// 服务器回的东西读不懂。
    Protocol(String),
}

impl fmt::Display for McpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            McpError::Config(m) => write!(f, "MCP 服务器的配置不对：{m}"),
            McpError::Spawn(m) => write!(f, "MCP 服务器没起来：{m}"),
            McpError::Connect(m) => write!(f, "连不上 MCP 服务器：{m}"),
            McpError::Http(status) => write!(f, "MCP 服务器返回 {}", status_text(*status)),
            McpError::Timeout { stage, after } => {
                let what = match stage {
                    Stage::Startup => "启动",
                    Stage::Request => "这次请求",
                };
                write!(f, "MCP 服务器{what}超过 {} 秒没有回应", seconds(*after))
            }
            McpError::Closed => write!(f, "和 MCP 服务器的连接断了，重新连一次试试"),
            McpError::Server { message, .. } => write!(f, "MCP 服务器报错：{message}"),
            McpError::Protocol(m) => write!(f, "MCP 服务器回的东西读不懂：{m}"),
        }
    }
}

impl StdError for McpError {}

/// `404` → `404 Not Found`（和 reqwest 的写法一样）。
pub fn status_text(status: u16) -> String {
    http::StatusCode::from_u16(status)
        .map(|s| s.to_string())
        .unwrap_or_else(|_| status.to_string())
}

/// `1.5` 秒写成 “1.5”，整秒写成 “10”。
pub(crate) fn seconds(d: Duration) -> String {
    let s = d.as_secs_f64();
    if (s - s.round()).abs() < 1e-9 {
        format!("{}", s.round() as u64)
    } else {
        format!("{s:.1}")
    }
}

/// 一串错误（含 source 链）接成一句。
///
/// **不带地址**：reqwest 的错误会写上 “for url (…)”，地址里可能有 Key（`?token=…`），而这句话会进插件卡片、
/// 经“没连上”发给模型。链上每个 reqwest 错误的地址都去掉（照 `reqwest::Error::without_url`；这里拿到的是借用，
/// 只能按它的写法去字），别处还提到这个地址的换成只有协议、主机和端口的写法。
pub(crate) fn chain(error: &(dyn StdError + 'static)) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut urls: Vec<reqwest::Url> = Vec::new();
    let mut current = Some(error);
    while let Some(e) = current {
        if let Some(url) = e
            .downcast_ref::<reqwest::Error>()
            .and_then(reqwest::Error::url)
        {
            urls.push(url.clone());
        }
        let text = e.to_string();
        if !parts.iter().any(|p| p.contains(&text)) {
            parts.push(text);
        }
        current = e.source();
    }
    let mut out = parts.join("：");
    for url in urls {
        out = out
            .replace(&format!(" for url ({url})"), "")
            .replace(url.as_str(), &url.origin().ascii_serialization());
    }
    out
}

type HttpError = rmcp::transport::streamable_http_client::StreamableHttpError<reqwest::Error>;

/// 传输层的错误：找出 HTTP 状态码，否则当作连不上。
pub(crate) fn from_transport(error: &(dyn StdError + 'static)) -> McpError {
    let mut current: Option<&(dyn StdError + 'static)> = Some(error);
    while let Some(e) = current {
        if let Some(http) = e.downcast_ref::<HttpError>() {
            match http {
                HttpError::HttpStatus(s) => return McpError::Http(s.status),
                HttpError::SessionExpired => return McpError::Closed,
                HttpError::TransportChannelClosed => return McpError::Closed,
                // reqwest 的错误没挂在 source 链上：直接从它说起（带上“拒绝连接”这类原因）
                HttpError::Client(r) => {
                    return match r.status() {
                        Some(status) => McpError::Http(status.as_u16()),
                        None => McpError::Connect(chain(r)),
                    }
                }
                _ => {}
            }
        }
        if let Some(r) = e.downcast_ref::<reqwest::Error>() {
            if let Some(status) = r.status() {
                return McpError::Http(status.as_u16());
            }
        }
        current = e.source();
    }
    McpError::Connect(chain(error))
}

impl From<rmcp::ServiceError> for McpError {
    fn from(error: rmcp::ServiceError) -> Self {
        use rmcp::ServiceError as E;
        match error {
            E::McpError(e) => McpError::Server {
                code: e.code.0,
                message: e.message.into_owned(),
            },
            E::TransportSend(e) => match from_transport(e.error.as_ref()) {
                McpError::Connect(m) if m.is_empty() => McpError::Closed,
                other => other,
            },
            E::TransportClosed => McpError::Closed,
            E::Timeout { timeout } => McpError::Timeout {
                stage: Stage::Request,
                after: timeout,
            },
            other => McpError::Protocol(other.to_string()),
        }
    }
}

impl From<rmcp::service::ClientInitializeError> for McpError {
    fn from(error: rmcp::service::ClientInitializeError) -> Self {
        use rmcp::service::ClientInitializeError as E;
        match error {
            E::JsonRpcError(e) => McpError::Server {
                code: e.code.0,
                message: e.message.into_owned(),
            },
            E::TransportError { error, .. } => from_transport(error.error.as_ref()),
            E::ConnectionClosed(_) => McpError::Closed,
            E::Cancelled => McpError::Closed,
            other => McpError::Protocol(other.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_are_plain() {
        assert_eq!(status_text(404), "404 Not Found");
        assert_eq!(
            McpError::Http(502).to_string(),
            "MCP 服务器返回 502 Bad Gateway"
        );
        assert_eq!(
            McpError::Timeout {
                stage: Stage::Request,
                after: Duration::from_millis(1500)
            }
            .to_string(),
            "MCP 服务器这次请求超过 1.5 秒没有回应"
        );
        assert_eq!(seconds(Duration::from_secs(10)), "10");
        let io = std::io::Error::other("拒绝连接");
        assert!(chain(&io).contains("拒绝连接"));
    }

    /// reqwest 连不上时的错误带着 “for url (…)”：地址（和里面的 Key）不进这句话。
    #[tokio::test]
    async fn connect_errors_do_not_carry_the_url() {
        let port = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.local_addr().unwrap().port()
        };
        let url = format!("http://127.0.0.1:{port}/mcp?token=sk-secret-123");
        let _ = rustls::crypto::ring::default_provider().install_default();
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let error = client.get(&url).send().await.unwrap_err();
        assert!(
            error.to_string().contains("sk-secret-123"),
            "reqwest 自己会带上地址"
        );
        let text = chain(&error);
        assert!(
            !text.contains("sk-secret-123") && !text.contains("/mcp"),
            "{text}"
        );
        assert!(!text.is_empty());
    }
}
