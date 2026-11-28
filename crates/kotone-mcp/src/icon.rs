//! 下载服务器在 `serverInfo.icons` 里给的 `https://` 图标（ADR 049 D7 第 2 条）：第三方插件加进来的时候下载一次，
//! 由调用的一方存到本机，以后只读本机文件，不再联网取。
//!
//! 规矩（防的是别人的服务器借头像做坏事，不是防盗图）：
//!
//! - 只认 `https://`；**不跟随跳转到别的主机**（协议、主机、端口有一样不同就停，最多跳 3 次）；
//! - 不超过 [`ICON_MAX_BYTES`]：先看 `Content-Length`，再边读边数，超了马上停；
//! - 整个请求 [`ICON_TIMEOUT`] 之内要完；
//! - 只收 PNG / JPEG / WebP / SVG，**看内容的开头认**（[`sniff_icon`]），不信 `Content-Type`。SVG 只用 `<img>` 显示
//!   （里面的脚本不会跑）。
//!
//! 出错只回一句原因（不带地址：和 [`crate::McpClient`] 的错误一样，地址里可能有 Key）；下不来时界面用首字块，
//! 不打扰玩家。

use std::time::Duration;

/// 头像最大多少字节（200 KB，和界面收 `data:` 图标的上限一致）。
pub const ICON_MAX_BYTES: usize = 200 * 1024;
/// 下载头像最多等多久（连上、收完都算在里面）。
pub const ICON_TIMEOUT: Duration = Duration::from_secs(5);
/// 同一个主机里最多跳几次。
const MAX_REDIRECTS: usize = 3;

/// 下载下来的头像：内容和认出来的类型（`image/png`、`image/jpeg`、`image/webp`、`image/svg+xml`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Icon {
    pub bytes: Vec<u8>,
    pub mime: &'static str,
}

/// 看内容的开头认图片类型；不是 PNG / JPEG / WebP / SVG 的回 None。
pub fn sniff_icon(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some("image/png");
    }
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some("image/jpeg");
    }
    if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    let text = std::str::from_utf8(bytes).ok()?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text).trim_start();
    let lower = text.to_ascii_lowercase();
    (lower.starts_with('<') && lower.contains("<svg") && !lower.contains("<html"))
        .then_some("image/svg+xml")
}

/// 下载一个 `https://` 头像（规矩见模块说明）。
pub async fn fetch_icon(url: &str) -> Result<Icon, String> {
    fetch(url, Options::default()).await
}

/// 下载时的选项（测试用本机的假服务器：允许 `http://`、不走系统代理、超时短一点）。
#[derive(Debug, Clone, Copy)]
struct Options {
    allow_http: bool,
    timeout: Duration,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            allow_http: false,
            timeout: ICON_TIMEOUT,
        }
    }
}

async fn fetch(url: &str, options: Options) -> Result<Icon, String> {
    let start = reqwest::Url::parse(url.trim()).map_err(|_| "图标的地址不对".to_string())?;
    let scheme_ok = start.scheme() == "https" || (options.allow_http && start.scheme() == "http");
    if !scheme_ok {
        return Err("图标只认 https:// 的地址".into());
    }
    let origin = start.origin();
    let policy = reqwest::redirect::Policy::custom(move |attempt| {
        if attempt.previous().len() > MAX_REDIRECTS {
            attempt.error("图标的地址跳转太多次")
        } else if attempt.url().origin() == origin {
            attempt.follow()
        } else {
            attempt.error("图标的地址跳到了别的主机")
        }
    });
    // reqwest 0.13 用进程默认的 rustls 加密提供者：装上 ring（已经装过别的就沿用），和 HTTP 连接一样
    let _ = rustls::crypto::ring::default_provider().install_default();
    let mut builder = reqwest::Client::builder()
        .user_agent("Kotone")
        .redirect(policy)
        .connect_timeout(options.timeout)
        .timeout(options.timeout);
    if options.allow_http {
        builder = builder.no_proxy();
    }
    let client = builder.build().map_err(|e| crate::error::chain(&e))?;
    let failed = |e: reqwest::Error| format!("图标下不来：{}", crate::error::chain(&e));
    let mut response = client.get(start).send().await.map_err(failed)?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!(
            "图标下不来：HTTP {}",
            crate::status_text(status.as_u16())
        ));
    }
    let too_big = || format!("图标超过 {} KB", ICON_MAX_BYTES / 1024);
    if response
        .content_length()
        .is_some_and(|n| n > ICON_MAX_BYTES as u64)
    {
        return Err(too_big());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(failed)? {
        if bytes.len() + chunk.len() > ICON_MAX_BYTES {
            return Err(too_big());
        }
        bytes.extend_from_slice(&chunk);
    }
    let mime =
        sniff_icon(&bytes).ok_or_else(|| "图标不是 PNG / JPEG / WebP / SVG 图片".to_string())?;
    Ok(Icon { bytes, mime })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";

    /// 本机的假 HTTP 服务器：每个连接读完请求头，按路径回 `reply(路径)` 给的整段响应，然后关掉连接。
    /// 回 None 的路径不回话（测超时）。返回 `http://127.0.0.1:<端口>`。
    async fn serve(
        reply: impl Fn(&str, u16) -> Option<Vec<u8>> + Send + Sync + 'static,
    ) -> (String, u16) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let reply = Arc::new(reply);
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                let reply = reply.clone();
                tokio::spawn(async move {
                    let mut request = Vec::new();
                    let mut buf = [0u8; 1024];
                    while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                        match socket.read(&mut buf).await {
                            Ok(0) | Err(_) => return,
                            Ok(n) => request.extend_from_slice(&buf[..n]),
                        }
                    }
                    let text = String::from_utf8_lossy(&request);
                    let path = text.split_whitespace().nth(1).unwrap_or("/").to_string();
                    match reply(&path, port) {
                        Some(out) => {
                            let _ = socket.write_all(&out).await;
                            let _ = socket.shutdown().await;
                        }
                        // 不回话，连接一直开着
                        None => tokio::time::sleep(Duration::from_secs(30)).await,
                    }
                });
            }
        });
        (format!("http://127.0.0.1:{port}"), port)
    }

    fn ok(content_type: &str, body: &[u8], length: bool) -> Vec<u8> {
        let mut head =
            format!("HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nConnection: close\r\n");
        if length {
            head.push_str(&format!("Content-Length: {}\r\n", body.len()));
        }
        head.push_str("\r\n");
        let mut out = head.into_bytes();
        out.extend_from_slice(body);
        out
    }

    fn redirect(to: &str) -> Vec<u8> {
        format!("HTTP/1.1 302 Found\r\nLocation: {to}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .into_bytes()
    }

    fn test_options() -> Options {
        Options {
            allow_http: true,
            timeout: Duration::from_millis(1500),
        }
    }

    #[test]
    fn only_png_jpeg_webp_and_svg_are_icons() {
        assert_eq!(sniff_icon(PNG), Some("image/png"));
        assert_eq!(
            sniff_icon(&[0xFF, 0xD8, 0xFF, 0xE0, 0, 0]),
            Some("image/jpeg")
        );
        assert_eq!(sniff_icon(b"RIFF\x10\0\0\0WEBPVP8 "), Some("image/webp"));
        assert_eq!(
            sniff_icon(b"\xEF\xBB\xBF  <?xml version=\"1.0\"?>\n<svg xmlns=\"http://www.w3.org/2000/svg\"/>"),
            Some("image/svg+xml")
        );
        assert_eq!(
            sniff_icon(b"<svg viewBox=\"0 0 1 1\"></svg>"),
            Some("image/svg+xml")
        );
        for bad in [
            &b"<!doctype html><html><body><svg></svg></body></html>"[..],
            b"GIF89a....",
            b"hello",
            b"",
            b"RIFF\x10\0\0\0WAVEfmt ",
        ] {
            assert_eq!(sniff_icon(bad), None, "{}", String::from_utf8_lossy(bad));
        }
    }

    #[tokio::test]
    async fn https_only() {
        assert!(fetch_icon("http://127.0.0.1:9/icon.png")
            .await
            .unwrap_err()
            .contains("https://"));
        assert!(fetch_icon("file:///c:/icon.png").await.is_err());
        assert!(fetch_icon("data:image/png;base64,AAAA").await.is_err());
        assert!(fetch_icon("不是地址").await.is_err());
    }

    #[tokio::test]
    async fn the_type_is_read_from_the_content() {
        let (base, _) = serve(|path, _| match path {
            "/icon.png" => Some(ok("application/octet-stream", PNG, true)),
            "/icon.svg" => Some(ok(
                "text/plain",
                b"<svg xmlns=\"http://www.w3.org/2000/svg\"></svg>",
                false,
            )),
            "/page.html" => Some(ok("image/png", b"<!doctype html><html></html>", true)),
            _ => Some(
                b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    .to_vec(),
            ),
        })
        .await;
        let png = fetch(&format!("{base}/icon.png"), test_options())
            .await
            .unwrap();
        assert_eq!((png.mime, png.bytes.as_slice()), ("image/png", PNG));
        let svg = fetch(&format!("{base}/icon.svg"), test_options())
            .await
            .unwrap();
        assert_eq!(svg.mime, "image/svg+xml");
        let html = fetch(&format!("{base}/page.html"), test_options())
            .await
            .unwrap_err();
        assert!(html.contains("不是 PNG"), "{html}");
        let missing = fetch(&format!("{base}/gone.png"), test_options())
            .await
            .unwrap_err();
        assert!(missing.contains("404"), "{missing}");
    }

    #[tokio::test]
    async fn icons_over_200_kb_are_refused() {
        let mut big = PNG.to_vec();
        big.resize(ICON_MAX_BYTES + 1, 0);
        let mut exact = PNG.to_vec();
        exact.resize(ICON_MAX_BYTES, 0);
        let (base, _) = serve(move |path, _| match path {
            // 说了长度的：先看长度就拒
            "/big-length.png" => Some(ok("image/png", &big, true)),
            // 没说长度（读到连接关掉）：边读边数
            "/big-stream.png" => Some(ok("image/png", &big, false)),
            _ => Some(ok("image/png", &exact, false)),
        })
        .await;
        for path in ["big-length.png", "big-stream.png"] {
            let error = fetch(&format!("{base}/{path}"), test_options())
                .await
                .unwrap_err();
            assert!(error.contains("200 KB"), "{path}: {error}");
        }
        let exact = fetch(&format!("{base}/exact.png"), test_options())
            .await
            .unwrap();
        assert_eq!(exact.bytes.len(), ICON_MAX_BYTES);
    }

    #[tokio::test]
    async fn redirects_stay_on_the_same_host() {
        let (base, _) = serve(|path, port| match path {
            "/same" => Some(redirect("/real.png")),
            // 同一台机器换个名字也算别的主机（主机名不同）
            "/other" => Some(redirect(&format!("http://localhost:{port}/real.png"))),
            "/loop" => Some(redirect("/loop")),
            _ => Some(ok("image/png", PNG, true)),
        })
        .await;
        let same = fetch(&format!("{base}/same"), test_options())
            .await
            .unwrap();
        assert_eq!(same.mime, "image/png");
        let other = fetch(&format!("{base}/other"), test_options())
            .await
            .unwrap_err();
        assert!(other.contains("别的主机"), "{other}");
        let looped = fetch(&format!("{base}/loop"), test_options())
            .await
            .unwrap_err();
        assert!(looped.contains("跳转太多"), "{looped}");
    }

    #[tokio::test]
    async fn a_silent_server_times_out() {
        let (base, _) = serve(|_, _| None).await;
        let started = std::time::Instant::now();
        let error = fetch(&format!("{base}/icon.png"), test_options())
            .await
            .unwrap_err();
        assert!(error.starts_with("图标下不来"), "{error}");
        assert!(started.elapsed() < Duration::from_secs(5));
    }
}
