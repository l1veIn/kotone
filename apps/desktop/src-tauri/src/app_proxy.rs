//! 回环代理（MCP 方案 4.5，ADR 039 D5）：插件的 MCP App 在 Kotone 里显示的地方。照 goose 的
//! `crates/goose/src/acp/mcp_app_proxy.rs`（Rust 侧起本机服务、CSP 在 Rust 侧拼）和 ext-apps 的
//! `examples/basic-host`（沙盒代理页 + 里层 iframe）。
//!
//! - **每个插件一个端口**（= 一个源）：App 和 Kotone 的页面、IPC 隔开（源不同），两个插件的 App 之间也隔开
//!   （同源的 iframe 能互相读页面；MCP Apps 规范的 `domain` 也是“每个服务器一个源”）。第一次要显示这个插件的
//!   App 时才起（[`Proxies::get`]），Kotone 退出时一起关。
//! - **只认本机**：只绑 `127.0.0.1`，对方地址不是回环的不理，`Host` 头不是 `127.0.0.1:<端口>` / `localhost:<端口>`
//!   的拒绝（防 DNS 重绑定）；路径第一段是随机密钥（256 位），不对一律 404。
//! - 路径（`<密钥>` 之后）：
//!   - `GET /<密钥>/sandbox/<插件>/<页面>`：沙盒代理页（`app_sandbox.html`），宿主的 iframe 指向这里；
//!   - `GET /<密钥>/app/<插件>/<页面>`：App 的 HTML（和 `resources/read` 同一份），**CSP 在这个响应的头里**，
//!     从 `default-src 'none'` 开始，只放行资源 `_meta.ui.csp` 声明的地址，加上本代理自己的 `/assets/<插件>/`；
//!   - `GET /<密钥>/assets/<插件>/<路径>`：素材，路径过了 [`valid_asset_path`] 才转给插件的 `asset`。

use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;

use http_body_util::Full;
use hyper::body::{Bytes, Incoming};
use hyper::header::{
    HeaderName, HeaderValue, CACHE_CONTROL, CONTENT_SECURITY_POLICY, CONTENT_TYPE, HOST,
    REFERRER_POLICY, X_CONTENT_TYPE_OPTIONS,
};
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::{TokioIo, TokioTimer};
use kotone_plugin_api::resource::normalize_csp_source;
use kotone_plugin_api::{valid_asset_path, Asset, BoxFuture, Csp, Resource};
use kotone_tools::ids::SecretToken;
use tokio::net::TcpListener;
use tokio::sync::watch;

/// 沙盒代理页（MCP Apps 规范的 Sandbox proxy）。
const SANDBOX_HTML: &str = include_str!("app_sandbox.html");

/// 沙盒代理页自己的 CSP：只有它自己的内联脚本和样式，只能嵌同源的 App。
const SANDBOX_CSP: &str =
    "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; \
frame-src 'self'; base-uri 'none'; form-action 'none'; object-src 'none'";

/// 素材响应自己的 CSP：图片（含 SVG）被直接打开时也跑不了脚本、连不了外面（素材那边的建议）。
const ASSET_CSP: &str = "default-src 'none'; style-src 'unsafe-inline'; sandbox";

/// 一个插件的 App 从哪来（桌面端是 `plugin_apps::PluginSource`，测试里是假的）。
pub trait Source: Send + Sync + 'static {
    /// 插件 id。
    fn plugin(&self) -> &str;
    /// `ui://kotone/<插件>/<page>` 的资源；插件关着、页面不是 App 时给错误（代理回 404）。
    fn resource(&self, page: &str) -> Result<Resource, String>;
    /// 素材（路径已检查过）；没有给 `None`（代理回 404）。
    fn asset(&self, path: String) -> BoxFuture<'static, Option<Asset>>;
}

/// 一个插件的代理（一个端口）。丢掉时停。
pub struct Proxy {
    addr: SocketAddr,
    key: SecretToken,
    plugin: String,
    stop: watch::Sender<bool>,
}

impl Drop for Proxy {
    fn drop(&mut self) {
        let _ = self.stop.send(true);
    }
}

impl Proxy {
    /// 在 `127.0.0.1` 的随机端口上起。
    pub async fn start(source: Arc<dyn Source>) -> std::io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let addr = listener.local_addr()?;
        let key = SecretToken::generate();
        let (stop, mut stopped) = watch::channel(false);
        let context = Arc::new(Context {
            source: source.clone(),
            key: key.clone(),
            port: addr.port(),
        });
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    changed = stopped.changed() => {
                        if changed.is_err() || *stopped.borrow() {
                            break;
                        }
                    }
                    accepted = listener.accept() => {
                        let Ok((stream, peer)) = accepted else { continue };
                        if !peer.ip().is_loopback() {
                            continue;
                        }
                        let context = context.clone();
                        tokio::spawn(async move {
                            let service = hyper::service::service_fn(move |req| {
                                let context = context.clone();
                                async move { Ok::<_, Infallible>(context.handle(req).await) }
                            });
                            let _ = hyper::server::conn::http1::Builder::new()
                                .timer(TokioTimer::new())
                                .header_read_timeout(std::time::Duration::from_secs(10))
                                .serve_connection(TokioIo::new(stream), service)
                                .await;
                        });
                    }
                }
            }
        });
        Ok(Self {
            addr,
            plugin: source.plugin().to_string(),
            key,
            stop,
        })
    }

    pub fn origin(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// 宿主的 iframe 指向这里（`page`：`ui://kotone/<插件>/<页面>` 的最后一段）。
    pub fn sandbox_url(&self, page: &str) -> String {
        format!(
            "{}/{}/sandbox/{}/{page}",
            self.origin(),
            self.key.expose(),
            self.plugin
        )
    }
}

/// 每个插件一个代理（第一次要显示它的 App 时才起）。放在 Tauri 的状态里，Kotone 退出时一起丢掉。
#[derive(Default)]
pub struct Proxies(tokio::sync::Mutex<HashMap<String, Arc<Proxy>>>);

impl Proxies {
    pub async fn get(&self, source: Arc<dyn Source>) -> Result<Arc<Proxy>, String> {
        let mut proxies = self.0.lock().await;
        if let Some(proxy) = proxies.get(source.plugin()) {
            return Ok(proxy.clone());
        }
        let plugin = source.plugin().to_string();
        let proxy = Arc::new(
            Proxy::start(source)
                .await
                .map_err(|e| format!("起不来 App 的本机代理：{e}"))?,
        );
        kotone_core::log::log(&format!(
            "app proxy: plugin {plugin} on 127.0.0.1:{}",
            proxy.addr.port()
        ));
        proxies.insert(plugin, proxy.clone());
        Ok(proxy)
    }
}

/// App 的 CSP（MCP Apps 规范“Host Behavior”：从 `default-src 'none'` 开始，只放行声明的；照 goose 的
/// `build_outer_csp`）。`assets`：本代理给这个插件的素材地址（`http://127.0.0.1:<端口>/<密钥>/assets/<插件>/`）。
/// 写法不对的声明（`*`、关键字、分号……）丢掉（[`normalize_csp_source`]）。
pub fn app_csp(csp: &Csp, assets: &str) -> String {
    let join = |domains: &[String]| -> String {
        domains
            .iter()
            .filter_map(|d| normalize_csp_source(d))
            .map(|d| format!(" {d}"))
            .collect()
    };
    let resources = join(&csp.resource_domains);
    let connect = join(&csp.connect_domains);
    let frames = join(&csp.frame_domains);
    let bases = join(&csp.base_uri_domains);
    let frame_src = if frames.is_empty() {
        "frame-src 'none'".to_string()
    } else {
        format!("frame-src{frames}")
    };
    format!(
        "default-src 'none'; \
         script-src 'unsafe-inline'{resources}; \
         style-src 'unsafe-inline'{resources}; \
         img-src data: blob: {assets}{resources}; \
         font-src data: {assets}{resources}; \
         media-src data: blob: {assets}{resources}; \
         connect-src {assets}{connect}; \
         {frame_src}; \
         object-src 'none'; \
         form-action 'none'; \
         base-uri 'self'{bases}"
    )
}

struct Context {
    source: Arc<dyn Source>,
    key: SecretToken,
    port: u16,
}

/// 页面的写法（和 `Manifest::owns_uri` 一样）。
fn valid_page(page: &str) -> bool {
    !page.is_empty()
        && page.len() <= 64
        && page
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn host_ok(host: Option<&HeaderValue>, port: u16) -> bool {
    let Some(host) = host.and_then(|h| h.to_str().ok()) else {
        return false;
    };
    [format!("127.0.0.1:{port}"), format!("localhost:{port}")]
        .iter()
        .any(|expected| expected.eq_ignore_ascii_case(host))
}

fn plain(status: StatusCode, text: &'static str) -> Response<Full<Bytes>> {
    respond(status, "text/plain; charset=utf-8", Bytes::from(text), &[])
}

fn respond(
    status: StatusCode,
    content_type: &str,
    body: Bytes,
    extra: &[(HeaderName, String)],
) -> Response<Full<Bytes>> {
    let mut response = Response::new(Full::new(body));
    *response.status_mut() = status;
    let headers = response.headers_mut();
    if let Ok(value) = HeaderValue::from_str(content_type) {
        headers.insert(CONTENT_TYPE, value);
    }
    headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    headers.insert(REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    for (name, value) in extra {
        if let Ok(value) = HeaderValue::from_str(value) {
            headers.insert(name.clone(), value);
        }
    }
    response
}

impl Context {
    async fn handle(&self, req: Request<Incoming>) -> Response<Full<Bytes>> {
        if !host_ok(req.headers().get(HOST), self.port) {
            return plain(StatusCode::FORBIDDEN, "forbidden");
        }
        if req.method() != Method::GET && req.method() != Method::HEAD {
            return plain(StatusCode::METHOD_NOT_ALLOWED, "method not allowed");
        }
        // 不解码：合法的路径里没有要编码的字符；`%2e%2e` 这类直接过不了检查
        let path = req.uri().path().to_string();
        let mut parts = path.strip_prefix('/').unwrap_or("").splitn(4, '/');
        let (Some(key), Some(kind), Some(plugin), Some(rest)) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return plain(StatusCode::NOT_FOUND, "not found");
        };
        if !self.key.matches(key) || plugin != self.source.plugin() {
            return plain(StatusCode::NOT_FOUND, "not found");
        }
        match kind {
            "sandbox" if valid_page(rest) => respond(
                StatusCode::OK,
                "text/html; charset=utf-8",
                Bytes::from_static(SANDBOX_HTML.as_bytes()),
                &[(CONTENT_SECURITY_POLICY, SANDBOX_CSP.to_string())],
            ),
            "app" if valid_page(rest) => match self.source.resource(rest) {
                Ok(resource) => {
                    let assets = format!(
                        "http://127.0.0.1:{}/{}/assets/{plugin}/",
                        self.port,
                        self.key.expose()
                    );
                    respond(
                        StatusCode::OK,
                        "text/html; charset=utf-8",
                        Bytes::from(resource.text),
                        &[(CONTENT_SECURITY_POLICY, app_csp(&resource.ui.csp, &assets))],
                    )
                }
                Err(_) => plain(StatusCode::NOT_FOUND, "not found"),
            },
            "assets" if valid_asset_path(rest) => {
                match self.source.asset(rest.to_string()).await {
                    Some(asset) => {
                        let mut response = respond(
                            StatusCode::OK,
                            &asset.mime_type,
                            Bytes::from(asset.bytes),
                            &[(CONTENT_SECURITY_POLICY, ASSET_CSP.to_string())],
                        );
                        // 地址里有每次启动都换的密钥：在这一次运行里放心缓存
                        response.headers_mut().insert(
                            CACHE_CONTROL,
                            HeaderValue::from_static("private, max-age=3600"),
                        );
                        response
                    }
                    None => plain(StatusCode::NOT_FOUND, "not found"),
                }
            }
            _ => plain(StatusCode::NOT_FOUND, "not found"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    const HTML: &str = "<!doctype html><html><head></head><body>强度榜</body></html>";

    #[derive(Default)]
    struct FakeSource {
        asset_calls: AtomicUsize,
    }

    impl Source for FakeSource {
        fn plugin(&self) -> &str {
            "lol"
        }
        fn resource(&self, page: &str) -> Result<Resource, String> {
            if page != "tier" {
                return Err("不是 App".into());
            }
            Ok(Resource::app("ui://kotone/lol/tier", HTML).csp(Csp {
                resource_domains: vec!["https://raw.communitydragon.org".into()],
                ..Csp::default()
            }))
        }
        fn asset(&self, path: String) -> BoxFuture<'static, Option<Asset>> {
            self.asset_calls.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move {
                (path == "lol-game-data/assets/v1/champion-icons/1.png").then(|| Asset {
                    mime_type: "image/png".into(),
                    bytes: vec![0x89, b'P', b'N', b'G'],
                })
            })
        }
    }

    struct Reply {
        status: u16,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    }

    impl Reply {
        fn header(&self, name: &str) -> Option<&str> {
            self.headers
                .iter()
                .find(|(n, _)| n.eq_ignore_ascii_case(name))
                .map(|(_, v)| v.as_str())
        }
    }

    /// 发一个最简单的 HTTP/1.1 请求（可以写任意的 Host 头）。
    async fn request(addr: SocketAddr, method: &str, path: &str, host: &str) -> Reply {
        let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        let req = format!("{method} {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");
        stream.write_all(req.as_bytes()).await.unwrap();
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await.unwrap();
        let split = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
        let head = String::from_utf8_lossy(&raw[..split]).to_string();
        let mut lines = head.split("\r\n");
        let status = lines
            .next()
            .unwrap()
            .split(' ')
            .nth(1)
            .unwrap()
            .parse()
            .unwrap();
        let headers = lines
            .filter_map(|l| l.split_once(": "))
            .map(|(n, v)| (n.to_string(), v.to_string()))
            .collect();
        Reply {
            status,
            headers,
            body: raw[split + 4..].to_vec(),
        }
    }

    #[test]
    fn the_app_csp_starts_from_none_and_allows_only_what_was_declared() {
        let assets = "http://127.0.0.1:5000/k/assets/lol/";
        let csp = app_csp(
            &Csp {
                resource_domains: vec![
                    "https://raw.communitydragon.org/latest/x.png".into(),
                    "*".into(),
                ],
                connect_domains: vec!["'unsafe-eval'".into(), "wss://live.example.com".into()],
                ..Csp::default()
            },
            assets,
        );
        assert!(csp.starts_with("default-src 'none';"), "{csp}");
        assert!(csp.contains(&format!(
            "img-src data: blob: {assets} https://raw.communitydragon.org;"
        )));
        assert!(csp.contains("script-src 'unsafe-inline' https://raw.communitydragon.org;"));
        assert!(csp.contains(&format!("connect-src {assets} wss://live.example.com;")));
        assert!(csp.contains("frame-src 'none';"));
        assert!(csp.contains("object-src 'none'") && csp.contains("form-action 'none'"));
        assert!(csp.ends_with("base-uri 'self'"));
        // 写法不对的丢掉，不会放进去
        assert!(
            !csp.contains(" * ") && !csp.contains("unsafe-eval"),
            "{csp}"
        );
        // 什么都没声明：只有自己的素材，外面一个都不放
        let bare = app_csp(&Csp::default(), assets);
        assert!(bare.contains(&format!("img-src data: blob: {assets};")));
        assert!(!bare.contains("https://"), "{bare}");
        let framed = app_csp(
            &Csp {
                frame_domains: vec!["https://www.youtube.com".into()],
                base_uri_domains: vec!["https://cdn.example.com".into()],
                ..Csp::default()
            },
            assets,
        );
        assert!(framed.contains("frame-src https://www.youtube.com;"));
        assert!(framed.ends_with("base-uri 'self' https://cdn.example.com"));
    }

    #[test]
    fn only_loopback_hosts_and_valid_pages() {
        let port = 5000;
        let ok = |h: &str| host_ok(Some(&HeaderValue::from_str(h).unwrap()), port);
        assert!(ok("127.0.0.1:5000") && ok("localhost:5000"));
        assert!(!ok("127.0.0.1:5001") && !ok("evil.example.com:5000") && !ok("127.0.0.1"));
        assert!(!host_ok(None, port));
        assert!(valid_page("tier") && valid_page("champ-select"));
        assert!(!valid_page("") && !valid_page("Tier") && !valid_page("a/b") && !valid_page(".."));
    }

    #[tokio::test]
    async fn serves_the_sandbox_the_app_and_assets_only_with_the_key() {
        let source = Arc::new(FakeSource::default());
        let proxy = Proxy::start(source.clone()).await.unwrap();
        let addr = proxy.addr;
        let host = format!("127.0.0.1:{}", addr.port());
        let key = proxy.key.expose().to_string();
        assert_eq!(
            proxy.sandbox_url("tier"),
            format!("http://{host}/{key}/sandbox/lol/tier")
        );

        // 沙盒代理页
        let sandbox = request(addr, "GET", &format!("/{key}/sandbox/lol/tier"), &host).await;
        assert_eq!(sandbox.status, 200);
        assert_eq!(sandbox.header("content-security-policy"), Some(SANDBOX_CSP));
        assert!(String::from_utf8_lossy(&sandbox.body).contains("sandbox-proxy-ready"));

        // App：HTML 原样，CSP 在头里
        let app = request(addr, "GET", &format!("/{key}/app/lol/tier"), &host).await;
        assert_eq!(app.status, 200);
        assert_eq!(String::from_utf8_lossy(&app.body), HTML);
        let csp = app.header("content-security-policy").unwrap();
        assert!(csp.starts_with("default-src 'none'"), "{csp}");
        assert!(csp.contains("https://raw.communitydragon.org"));
        assert!(csp.contains(&format!("http://{host}/{key}/assets/lol/")));
        assert_eq!(app.header("referrer-policy"), Some("no-referrer"));
        assert_eq!(app.header("x-content-type-options"), Some("nosniff"));
        // 不是 App 的页面
        let not_app = request(addr, "GET", &format!("/{key}/app/lol/match"), &host).await;
        assert_eq!(not_app.status, 404);

        // 素材
        let icon = format!("/{key}/assets/lol/lol-game-data/assets/v1/champion-icons/1.png");
        let asset = request(addr, "GET", &icon, &host).await;
        assert_eq!(asset.status, 200);
        assert_eq!(asset.header("content-type"), Some("image/png"));
        assert_eq!(asset.header("content-security-policy"), Some(ASSET_CSP));
        assert_eq!(asset.header("x-content-type-options"), Some("nosniff"));
        assert_eq!(asset.body, [0x89, b'P', b'N', b'G']);
        let missing = request(addr, "GET", &format!("/{key}/assets/lol/x/2.png"), &host).await;
        assert_eq!(missing.status, 404);
        let calls = source.asset_calls.load(Ordering::SeqCst);
        assert_eq!(calls, 2);
        // 路径不对的不转给插件
        for bad in ["../x.png", "a/../../b", "%2e%2e/x", "a//b", "a/b?x=1"] {
            let reply = request(addr, "GET", &format!("/{key}/assets/lol/{bad}"), &host).await;
            assert_eq!(reply.status, 404, "{bad}");
        }
        assert_eq!(
            source.asset_calls.load(Ordering::SeqCst),
            calls + 1,
            "只有 a/b?x=1 的路径部分合法"
        );

        // 没密钥、密钥不对、别的插件：一律 404
        for path in [
            "/sandbox/lol/tier".to_string(),
            format!("/{}/sandbox/lol/tier", &key[1..]),
            "/0000/app/lol/tier".to_string(),
            format!("/{key}/app/music/tier"),
            format!("/{key}"),
            "/".to_string(),
        ] {
            assert_eq!(
                request(addr, "GET", &path, &host).await.status,
                404,
                "{path}"
            );
        }
        // 不是本机的 Host（DNS 重绑定）、不是 GET
        let rebound = request(
            addr,
            "GET",
            &format!("/{key}/app/lol/tier"),
            "evil.example.com",
        )
        .await;
        assert_eq!(rebound.status, 403);
        let post = request(addr, "POST", &format!("/{key}/app/lol/tier"), &host).await;
        assert_eq!(post.status, 405);

        // 丢掉代理：端口关掉
        drop(proxy);
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(tokio::net::TcpStream::connect(addr).await.is_err());
    }

    #[tokio::test]
    async fn one_proxy_per_plugin() {
        let proxies = Proxies::default();
        let a = proxies.get(Arc::new(FakeSource::default())).await.unwrap();
        let b = proxies.get(Arc::new(FakeSource::default())).await.unwrap();
        assert!(Arc::ptr_eq(&a, &b));
        assert!(a.origin().starts_with("http://127.0.0.1:"));
    }
}
