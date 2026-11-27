//! 收 BetterGI 的通知 Webhook（ADR 047 第 3 期）：插件在 127.0.0.1 开一个小的 HTTP 接收端，地址里带随机令牌
//! （`http://127.0.0.1:<端口>/genshin/<令牌>`），BetterGI 的“Webhook 通知”指向它。
//!
//! - 只听 127.0.0.1；只收 `POST` 到带对的令牌的地址、`Host` 是本机的（挡住网页借浏览器往本机发）、有 `Content-Length`、
//!   不超过 [`BODY_MAX`]（BetterGI 默认带截图，base64 的 JPEG 几百 KB 到几 MB）；
//! - 端口和令牌存在插件的数据目录（`webhook.json`），重启 Kotone 不变（BetterGI 的配置里写的是这个地址）；端口被占了就换一个、
//!   记下来，插件页会说 Webhook 没指向 Kotone，再点一次“一键填好”。
//! - 截图不留：只取事件名、结果和原话（[`crate::runs::Notice`]）。
//!
//! 另外有一个本机的小 GET（[`get_json`]）：BetterGI 0.67 的本机外部访问服务开着时查 `/api/v1/health`。

use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// 默认端口（BetterGI 自己的外部访问服务默认 30648，不撞它）。
pub const DEFAULT_PORT: u16 = 38648;
/// 请求体上限。
pub const BODY_MAX: usize = 32 * 1024 * 1024;
const HEAD_MAX: usize = 16 * 1024;
const READ_TIMEOUT: Duration = Duration::from_secs(20);

/// 接收端的地址：端口和令牌。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Endpoint {
    pub port: u16,
    pub token: String,
}

impl Endpoint {
    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}/genshin/{}", self.port, self.token)
    }

    pub fn path(&self) -> String {
        format!("/genshin/{}", self.token)
    }

    /// BetterGI 配置里的地址是不是指向这里（`localhost` 和 `127.0.0.1` 都算，末尾的 `/` 不算）。
    pub fn is(&self, configured: &str) -> bool {
        let normalize = |s: &str| {
            s.trim()
                .trim_end_matches('/')
                .to_ascii_lowercase()
                .replace("://localhost:", "://127.0.0.1:")
        };
        !configured.trim().is_empty() && normalize(configured) == normalize(&self.url())
    }

    /// 读数据目录里的；没有或者坏了就新起一个（默认端口、新令牌）并存下。
    pub fn load_or_create(dir: &Path) -> Result<Endpoint, String> {
        let path = dir.join("webhook.json");
        if let Some(saved) = std::fs::read_to_string(&path)
            .ok()
            .and_then(|t| serde_json::from_str::<Endpoint>(&t).ok())
            .filter(|e| e.port >= 1024 && valid_token(&e.token))
        {
            return Ok(saved);
        }
        let endpoint = Endpoint {
            port: DEFAULT_PORT,
            token: new_token()?,
        };
        endpoint.save(dir)?;
        Ok(endpoint)
    }

    pub fn save(&self, dir: &Path) -> Result<(), String> {
        std::fs::create_dir_all(dir).map_err(|e| format!("建不了插件的数据目录：{e}"))?;
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(dir.join("webhook.json"), text)
            .map_err(|e| format!("存不了 Webhook 地址：{e}"))
    }
}

fn valid_token(token: &str) -> bool {
    token.len() == 32 && token.chars().all(|c| c.is_ascii_hexdigit())
}

fn new_token() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes).map_err(|e| format!("取不到随机数：{e}"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// 先试记下的端口，被占了换一个系统给的（并记下）。
pub async fn bind(dir: &Path) -> Result<(TcpListener, Endpoint), String> {
    let mut endpoint = Endpoint::load_or_create(dir)?;
    match TcpListener::bind(("127.0.0.1", endpoint.port)).await {
        Ok(listener) => Ok((listener, endpoint)),
        Err(_) => {
            let listener = TcpListener::bind(("127.0.0.1", 0))
                .await
                .map_err(|e| format!("开不了本机的接收端：{e}"))?;
            endpoint.port = listener.local_addr().map_err(|e| e.to_string())?.port();
            endpoint.save(dir)?;
            Ok((listener, endpoint))
        }
    }
}

/// 一个请求的头。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Head {
    pub method: String,
    pub path: String,
    pub host: String,
    pub content_length: Option<usize>,
    pub chunked: bool,
}

pub fn parse_head(text: &str) -> Option<Head> {
    let mut lines = text.split("\r\n");
    let mut first = lines.next()?.split(' ');
    let method = first.next()?.to_string();
    let path = first.next()?.to_string();
    if !first.next()?.starts_with("HTTP/1.") {
        return None;
    }
    let mut head = Head {
        method,
        path,
        host: String::new(),
        content_length: None,
        chunked: false,
    };
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match name.trim().to_ascii_lowercase().as_str() {
            "host" => head.host = value.to_ascii_lowercase(),
            "content-length" => head.content_length = value.parse().ok(),
            "transfer-encoding" => head.chunked = value.to_ascii_lowercase().contains("chunked"),
            _ => {}
        }
    }
    Some(head)
}

/// 这个请求该怎么回（状态码）；`None` = 收下、交给调用方处理请求体。
pub fn check_head(head: &Head, endpoint: &Endpoint) -> Option<u16> {
    let host_ok = [
        format!("127.0.0.1:{}", endpoint.port),
        format!("localhost:{}", endpoint.port),
    ]
    .contains(&head.host);
    if !host_ok {
        return Some(403);
    }
    if head.path.split('?').next() != Some(endpoint.path().as_str()) {
        return Some(404);
    }
    if head.method != "POST" {
        return Some(405);
    }
    match head.content_length {
        _ if head.chunked => Some(411),
        None => Some(411),
        Some(len) if len > BODY_MAX => Some(413),
        Some(_) => None,
    }
}

fn reason(code: u16) -> &'static str {
    match code {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        411 => "Length Required",
        413 => "Payload Too Large",
        _ => "Error",
    }
}

async fn respond(stream: &mut TcpStream, code: u16) {
    let body = if code == 200 {
        "{\"ok\":true}"
    } else {
        "{\"ok\":false}"
    };
    let text = format!(
        "HTTP/1.1 {code} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        reason(code),
        body.len()
    );
    let _ = stream.write_all(text.as_bytes()).await;
    let _ = stream.shutdown().await;
}

/// 读一个请求：头（到空行）和请求体（照 `Content-Length`）。不合格的直接回状态码。
async fn read_request(stream: &mut TcpStream, endpoint: &Endpoint) -> Result<Vec<u8>, u16> {
    let mut buf = Vec::with_capacity(4096);
    let mut chunk = [0u8; 8192];
    let head_end = loop {
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break pos;
        }
        if buf.len() > HEAD_MAX {
            return Err(400);
        }
        let n = stream.read(&mut chunk).await.map_err(|_| 400u16)?;
        if n == 0 {
            return Err(400);
        }
        buf.extend_from_slice(&chunk[..n]);
    };
    let head = parse_head(&String::from_utf8_lossy(&buf[..head_end])).ok_or(400u16)?;
    if let Some(code) = check_head(&head, endpoint) {
        return Err(code);
    }
    let len = head.content_length.unwrap_or(0);
    let mut body = buf.split_off(head_end + 4);
    while body.len() < len {
        let n = stream.read(&mut chunk).await.map_err(|_| 400u16)?;
        if n == 0 {
            return Err(400);
        }
        body.extend_from_slice(&chunk[..n]);
    }
    body.truncate(len);
    Ok(body)
}

/// 接收端：一直收，直到 `alive()` 变成 false（每秒看一次）。每个请求体交给 `on_body`，它回 true 就是收下了（200），
/// false 是看不懂（400）。
pub async fn serve<A, F>(listener: TcpListener, endpoint: Endpoint, alive: A, on_body: F)
where
    A: Fn() -> bool + Send + Sync + 'static,
    F: Fn(Vec<u8>) -> bool + Send + Sync + 'static,
{
    let on_body = std::sync::Arc::new(on_body);
    loop {
        let accepted = tokio::time::timeout(Duration::from_secs(1), listener.accept()).await;
        if !alive() {
            break;
        }
        let Ok(accepted) = accepted else { continue };
        let (mut stream, peer): (TcpStream, SocketAddr) = match accepted {
            Ok(pair) => pair,
            Err(_) => {
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        if !peer.ip().is_loopback() {
            continue;
        }
        let endpoint = endpoint.clone();
        let on_body = on_body.clone();
        tokio::spawn(async move {
            let code = match tokio::time::timeout(
                READ_TIMEOUT,
                read_request(&mut stream, &endpoint),
            )
            .await
            {
                Err(_) => 408,
                Ok(Err(code)) => code,
                Ok(Ok(body)) => {
                    if on_body(body) {
                        200
                    } else {
                        400
                    }
                }
            };
            respond(&mut stream, code).await;
        });
    }
}

/// 本机的一个 GET（`127.0.0.1:<端口><路径>`，带 Bearer 令牌），回 200 时的 JSON。很快超时（1 秒）。
pub async fn get_json(port: u16, path: &str, token: &str) -> Option<Value> {
    let work = async {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).await.ok()?;
        let auth = if token.is_empty() {
            String::new()
        } else {
            format!("Authorization: Bearer {token}\r\n")
        };
        let request = format!(
            "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n{auth}Accept: application/json\r\nConnection: close\r\n\r\n"
        );
        stream.write_all(request.as_bytes()).await.ok()?;
        let mut bytes = Vec::new();
        stream.take(256 * 1024).read_to_end(&mut bytes).await.ok()?;
        let text = String::from_utf8_lossy(&bytes);
        let (head, body) = text.split_once("\r\n\r\n")?;
        if !head.starts_with("HTTP/1.1 200") && !head.starts_with("HTTP/1.0 200") {
            return None;
        }
        // 只认 Content-Length 的回答（ASP.NET 的小 JSON 是这样）；分块的先去掉块头试一次
        serde_json::from_str(body.trim()).ok().or_else(|| {
            body.split("\r\n")
                .nth(1)
                .and_then(|b| serde_json::from_str(b).ok())
        })
    };
    tokio::time::timeout(Duration::from_secs(1), work)
        .await
        .ok()
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    #[test]
    fn the_endpoint_is_kept_and_compared_loosely() {
        let dir = tempfile::tempdir().unwrap();
        let a = Endpoint::load_or_create(dir.path()).unwrap();
        assert_eq!(a.port, DEFAULT_PORT);
        assert!(valid_token(&a.token));
        assert_eq!(Endpoint::load_or_create(dir.path()).unwrap(), a, "重启不变");
        assert!(a.is(&a.url()));
        assert!(a.is(&format!("{}/", a.url())));
        assert!(a.is(&a.url().replace("127.0.0.1", "localhost")));
        assert!(!a.is(""));
        assert!(!a.is("http://127.0.0.1:38648/genshin/0000"));
        // 坏了的文件：新起一个
        std::fs::write(
            dir.path().join("webhook.json"),
            "{\"port\":1,\"token\":\"x\"}",
        )
        .unwrap();
        let b = Endpoint::load_or_create(dir.path()).unwrap();
        assert_ne!(b.token, "x");
    }

    #[test]
    fn heads_are_checked_before_reading_the_body() {
        let e = Endpoint {
            port: 40000,
            token: "0123456789abcdef0123456789abcdef".into(),
        };
        let head = |method: &str, path: &str, host: &str, extra: &str| {
            parse_head(&format!(
                "{method} {path} HTTP/1.1\r\nHost: {host}\r\n{extra}"
            ))
            .unwrap()
        };
        let path = e.path();
        let ok = head(
            "POST",
            &path,
            "127.0.0.1:40000",
            "Content-Length: 10\r\nContent-Type: application/json",
        );
        assert_eq!(ok.content_length, Some(10));
        assert_eq!(check_head(&ok, &e), None);
        assert_eq!(
            check_head(
                &head("POST", &path, "LOCALHOST:40000", "Content-Length: 1"),
                &e
            ),
            None
        );
        assert_eq!(
            check_head(
                &head("POST", &path, "evil.example:40000", "Content-Length: 1"),
                &e
            ),
            Some(403)
        );
        assert_eq!(
            check_head(
                &head(
                    "POST",
                    "/genshin/wrong",
                    "127.0.0.1:40000",
                    "Content-Length: 1"
                ),
                &e
            ),
            Some(404)
        );
        assert_eq!(
            check_head(&head("GET", &path, "127.0.0.1:40000", ""), &e),
            Some(405)
        );
        assert_eq!(
            check_head(
                &head(
                    "POST",
                    &path,
                    "127.0.0.1:40000",
                    "Transfer-Encoding: chunked"
                ),
                &e
            ),
            Some(411)
        );
        assert_eq!(
            check_head(&head("POST", &path, "127.0.0.1:40000", ""), &e),
            Some(411)
        );
        assert_eq!(
            check_head(
                &head(
                    "POST",
                    &path,
                    "127.0.0.1:40000",
                    &format!("Content-Length: {}", BODY_MAX + 1)
                ),
                &e
            ),
            Some(413)
        );
        assert!(parse_head("garbage").is_none());
    }

    async fn post(port: u16, path: &str, host: &str, body: &str) -> String {
        let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        let req = format!(
            "POST {path} HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        s.write_all(req.as_bytes()).await.unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).await.unwrap();
        out
    }

    /// 真的开一个接收端：对的令牌收下、错的不收；`alive` 变成 false 就关掉。
    #[tokio::test]
    async fn the_receiver_takes_only_posts_with_the_token() {
        let dir = tempfile::tempdir().unwrap();
        // 默认端口可能被别的测试占着：直接用系统给的
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let endpoint = Endpoint {
            port,
            token: Endpoint::load_or_create(dir.path()).unwrap().token,
        };
        let got = Arc::new(Mutex::new(Vec::<String>::new()));
        let alive = Arc::new(AtomicBool::new(true));
        let (g, a) = (got.clone(), alive.clone());
        let server = tokio::spawn(serve(
            listener,
            endpoint.clone(),
            move || a.load(Ordering::SeqCst),
            move |body| {
                let text = String::from_utf8(body).unwrap_or_default();
                let ok = text.starts_with('{');
                g.lock().unwrap().push(text);
                ok
            },
        ));
        let host = format!("127.0.0.1:{port}");
        let ok = post(port, &endpoint.path(), &host, r#"{"event":"notify.test"}"#).await;
        assert!(ok.starts_with("HTTP/1.1 200"), "{ok}");
        let bad = post(port, &endpoint.path(), &host, "nope").await;
        assert!(bad.starts_with("HTTP/1.1 400"), "{bad}");
        let wrong = post(
            port,
            "/genshin/ffffffffffffffffffffffffffffffff",
            &host,
            "{}",
        )
        .await;
        assert!(wrong.starts_with("HTTP/1.1 404"), "{wrong}");
        assert_eq!(
            got.lock().unwrap().as_slice(),
            [r#"{"event":"notify.test"}"#.to_string(), "nope".to_string()]
        );
        alive.store(false, Ordering::SeqCst);
        tokio::time::timeout(Duration::from_secs(5), server)
            .await
            .unwrap()
            .unwrap();
    }

    /// 本机的 GET：200 的 JSON 读得回来，别的状态码、连不上都是 None。
    #[tokio::test]
    async fn get_json_reads_a_local_answer() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            for code in ["200 OK", "401 Unauthorized"] {
                let (mut s, _) = listener.accept().await.unwrap();
                let mut buf = [0u8; 1024];
                let n = s.read(&mut buf).await.unwrap();
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                assert!(req.contains("Authorization: Bearer t"));
                let body = r#"{"status":"Healthy","processId":1}"#;
                let resp = format!(
                    "HTTP/1.1 {code}\r\nContent-Length: {}\r\n\r\n{body}",
                    body.len()
                );
                s.write_all(resp.as_bytes()).await.unwrap();
            }
        });
        let v = get_json(port, "/api/v1/health", "t").await.unwrap();
        assert_eq!(v["status"], "Healthy");
        assert!(get_json(port, "/api/v1/health", "t").await.is_none());
        // 没人听的端口：绑一个不 listen 的 socket（docs/development.md）
        let unused = tokio::net::TcpSocket::new_v4().unwrap();
        unused.bind("127.0.0.1:0".parse().unwrap()).unwrap();
        let closed = unused.local_addr().unwrap().port();
        assert!(get_json(closed, "/", "").await.is_none());
        drop(unused);
    }
}
