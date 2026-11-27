//! 收 GSI 的本机 HTTP：**只听 127.0.0.1**，只收 `POST`（带 `Content-Length` 的 JSON），令牌不对的丢掉。
//!
//! 游戏一份一份地 POST（上一份没回 2xx 之前不发下一份，超时以后下一份带完整状态），连接可能复用：一个连接里按顺序处理
//! 多个请求。不用 HTTP 框架：要的只是这么一小段，自己读头、读正文。

use std::net::{Ipv4Addr, SocketAddr};
use std::time::Duration;

use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// 默认端口（存在插件设置 `port` 里；被占了往后找）。
pub const DEFAULT_PORT: u16 = 23730;
/// 往后找几个端口。
pub const TRIES: u16 = 20;
/// 请求头最多多大。
pub const MAX_HEAD: usize = 16 * 1024;
/// 正文最多多大（观战时带 10 个人的数据也就几十 KB）。
pub const MAX_BODY: usize = 1024 * 1024;
/// 一个连接多久没动静就关。
const IDLE: Duration = Duration::from_secs(60);

/// 收到的一份怎么处理的（决定回什么）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// 收下了。
    Accepted,
    /// 令牌不对：丢掉，回 403。
    BadToken,
    /// 不是 CS2（appid 不是 730）：丢掉，回 200（不让它重试）。
    NotCs2,
}

/// 从 `port` 起找一个能在 127.0.0.1 上听的端口（最多往后 [`TRIES`] 个）。
pub async fn bind_from(port: u16) -> std::io::Result<(TcpListener, u16)> {
    let mut last = None;
    for p in port..=port.saturating_add(TRIES - 1) {
        match TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, p))).await {
            Ok(listener) => return Ok((listener, p)),
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap_or_else(|| std::io::Error::other("没有可用的端口")))
}

/// 请求头里要的几样。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Head {
    pub method: String,
    pub length: Option<usize>,
    pub chunked: bool,
    pub close: bool,
}

/// 读请求头（不含最后的空行）。
pub fn parse_head(head: &str) -> Option<Head> {
    let mut lines = head.split("\r\n");
    let first = lines.next()?;
    let mut parts = first.split(' ');
    let method = parts.next()?.to_string();
    let _path = parts.next()?;
    let version = parts.next().unwrap_or("HTTP/1.0");
    let mut out = Head {
        method,
        length: None,
        chunked: false,
        close: version == "HTTP/1.0",
    };
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match name.trim().to_ascii_lowercase().as_str() {
            "content-length" => out.length = Some(value.parse().ok()?),
            "transfer-encoding" => out.chunked = value.to_ascii_lowercase().contains("chunked"),
            "connection" => match value.to_ascii_lowercase().as_str() {
                "close" => out.close = true,
                "keep-alive" => out.close = false,
                _ => {}
            },
            _ => {}
        }
    }
    Some(out)
}

fn response(code: u16, close: bool) -> String {
    let reason = match code {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        405 => "Method Not Allowed",
        411 => "Length Required",
        413 => "Payload Too Large",
        431 => "Request Header Fields Too Large",
        _ => "Error",
    };
    format!(
        "HTTP/1.1 {code} {reason}\r\nContent-Length: 0\r\nConnection: {}\r\n\r\n",
        if close { "close" } else { "keep-alive" }
    )
}

/// 处理一个连接：一个个读请求，正文是 JSON 就交给 `handle`。
pub async fn serve<F>(mut stream: TcpStream, handle: F)
where
    F: Fn(Value) -> Verdict,
{
    let mut buf: Vec<u8> = Vec::with_capacity(4096);
    loop {
        // 读到头结束
        let head_end = loop {
            if let Some(at) = find(&buf, b"\r\n\r\n") {
                break at;
            }
            if buf.len() > MAX_HEAD {
                let _ = stream.write_all(response(431, true).as_bytes()).await;
                return;
            }
            if !read_more(&mut stream, &mut buf).await {
                return;
            }
        };
        let Some(head) = std::str::from_utf8(&buf[..head_end])
            .ok()
            .and_then(parse_head)
        else {
            let _ = stream.write_all(response(400, true).as_bytes()).await;
            return;
        };
        let refuse = if head.method != "POST" {
            Some(405)
        } else if head.chunked || head.length.is_none() {
            Some(411)
        } else if head.length.unwrap_or(0) > MAX_BODY {
            Some(413)
        } else {
            None
        };
        if let Some(code) = refuse {
            let _ = stream.write_all(response(code, true).as_bytes()).await;
            return;
        }
        let length = head.length.unwrap_or(0);
        let body_start = head_end + 4;
        while buf.len() < body_start + length {
            if !read_more(&mut stream, &mut buf).await {
                return;
            }
        }
        let code = match serde_json::from_slice::<Value>(&buf[body_start..body_start + length]) {
            Ok(value) => match handle(value) {
                Verdict::Accepted | Verdict::NotCs2 => 200,
                Verdict::BadToken => 403,
            },
            Err(_) => 400,
        };
        if stream
            .write_all(response(code, head.close).as_bytes())
            .await
            .is_err()
            || head.close
        {
            return;
        }
        buf.drain(..body_start + length);
    }
}

async fn read_more(stream: &mut TcpStream, buf: &mut Vec<u8>) -> bool {
    let mut chunk = [0u8; 8192];
    match tokio::time::timeout(IDLE, stream.read(&mut chunk)).await {
        Ok(Ok(n)) if n > 0 => {
            buf.extend_from_slice(&chunk[..n]);
            true
        }
        _ => false,
    }
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn heads_are_parsed() {
        let h = parse_head("POST / HTTP/1.1\r\nHost: 127.0.0.1:3000\r\nContent-Type: application/json\r\nContent-Length: 12").unwrap();
        assert_eq!(h.method, "POST");
        assert_eq!(h.length, Some(12));
        assert!(!h.close && !h.chunked);
        let h = parse_head("POST / HTTP/1.0\r\nContent-Length: 1").unwrap();
        assert!(h.close, "HTTP/1.0 默认关连接");
        let h = parse_head("POST / HTTP/1.1\r\nTransfer-Encoding: chunked\r\nConnection: close")
            .unwrap();
        assert!(h.chunked && h.close && h.length.is_none());
        assert!(parse_head("POST / HTTP/1.1\r\nContent-Length: abc").is_none());
        assert!(parse_head("").is_none());
    }

    async fn post(port: u16, body: &str, extra: &str) -> String {
        let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        let req = format!(
            "POST / HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{extra}\r\n{body}",
            body.len()
        );
        s.write_all(req.as_bytes()).await.unwrap();
        let mut out = vec![0u8; 256];
        let n = s.read(&mut out).await.unwrap();
        String::from_utf8_lossy(&out[..n]).into_owned()
    }

    /// 起一个服务：令牌是 `good`，收下的记在 `got` 里。
    async fn start() -> (u16, Arc<Mutex<Vec<Value>>>) {
        let (listener, port) = bind_from(DEFAULT_PORT + 500).await.unwrap();
        let got = Arc::new(Mutex::new(Vec::new()));
        let sink = got.clone();
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                let sink = sink.clone();
                tokio::spawn(serve(stream, move |v: Value| {
                    if !crate::gsi::token_ok(&v, "good") {
                        return Verdict::BadToken;
                    }
                    sink.lock().unwrap().push(v);
                    Verdict::Accepted
                }));
            }
        });
        (port, got)
    }

    #[tokio::test]
    async fn listens_only_on_loopback() {
        let (listener, port) = bind_from(DEFAULT_PORT + 400).await.unwrap();
        let addr = listener.local_addr().unwrap();
        assert!(addr.ip().is_loopback(), "{addr}");
        assert_eq!(addr.port(), port);
    }

    #[tokio::test]
    async fn a_taken_port_moves_to_the_next_one() {
        let (_held, port) = bind_from(DEFAULT_PORT + 300).await.unwrap();
        let (_second, next) = bind_from(port).await.unwrap();
        assert!(next > port, "{port} 被占了，应该换成后面的（拿到 {next}）");
        assert!(next < port + TRIES);
    }

    #[tokio::test]
    async fn the_token_decides_and_connections_are_reused() {
        let (port, got) = start().await;
        let ok = r#"{"provider":{"appid":730},"auth":{"token":"good"}}"#;
        let bad = r#"{"provider":{"appid":730},"auth":{"token":"bad"}}"#;
        assert!(post(port, ok, "").await.starts_with("HTTP/1.1 200"));
        assert!(post(port, bad, "").await.starts_with("HTTP/1.1 403"));
        assert!(post(port, "{not json", "")
            .await
            .starts_with("HTTP/1.1 400"));
        assert_eq!(got.lock().unwrap().len(), 1, "令牌不对的丢掉");

        // 同一个连接里两份
        let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        let one = format!(
            "POST / HTTP/1.1\r\nContent-Length: {}\r\n\r\n{ok}",
            ok.len()
        );
        s.write_all(format!("{one}{one}").as_bytes()).await.unwrap();
        let mut seen = String::new();
        while seen.matches("200 OK").count() < 2 {
            let mut chunk = [0u8; 256];
            let n = s.read(&mut chunk).await.unwrap();
            assert!(n > 0);
            seen.push_str(&String::from_utf8_lossy(&chunk[..n]));
        }
        assert_eq!(got.lock().unwrap().len(), 3);
    }

    #[tokio::test]
    async fn only_post_with_a_length_is_accepted() {
        let (port, got) = start().await;
        let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        s.write_all(b"GET / HTTP/1.1\r\n\r\n").await.unwrap();
        let mut out = [0u8; 128];
        let n = s.read(&mut out).await.unwrap();
        assert!(String::from_utf8_lossy(&out[..n]).starts_with("HTTP/1.1 405"));
        let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        s.write_all(b"POST / HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n")
            .await
            .unwrap();
        let n = s.read(&mut out).await.unwrap();
        assert!(String::from_utf8_lossy(&out[..n]).starts_with("HTTP/1.1 411"));
        let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        s.write_all(
            format!(
                "POST / HTTP/1.1\r\nContent-Length: {}\r\n\r\n",
                MAX_BODY + 1
            )
            .as_bytes(),
        )
        .await
        .unwrap();
        let n = s.read(&mut out).await.unwrap();
        assert!(String::from_utf8_lossy(&out[..n]).starts_with("HTTP/1.1 413"));
        assert!(got.lock().unwrap().is_empty());
    }
}
