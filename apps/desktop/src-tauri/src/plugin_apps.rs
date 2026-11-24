//! 插件的 MCP App（ADR 039 D5；对应规则见 `kotone_plugin_api::resource`）：哪些页面做成了 App、读它的 HTML、
//! 取素材。工具服务（`resources/list` / `resources/read`，经 `kotone.exe mcp` 给外部宿主）和回环代理
//! （`app_proxy.rs`，Kotone 自己显示）都走这里：插件关着、页面要的数据开关关着、页面还不是 App 时都没有。

use std::sync::Arc;

use kotone_core::settings::Settings;
use kotone_plugin_api::{Asset, BoxFuture, Plugin, Resource};
use serde_json::Value;
use tauri::{AppHandle, Manager};

use crate::plugins::{enabled, registry, source_enabled};

fn settings(app: &AppHandle) -> Settings {
    app.state::<crate::SharedState>()
        .settings
        .read()
        .unwrap()
        .clone()
}

/// 这个页面地址是哪个插件的。
pub fn owner(uri: &str) -> Option<&'static Arc<dyn Plugin>> {
    registry().iter().find(|p| p.manifest().owns_uri(uri))
}

/// 读一个 App 资源：插件和页面要的数据开关都开着、页面是 App、资源过了清单的检查。
pub fn read(settings: &Settings, uri: &str) -> Result<Resource, String> {
    let plugin = owner(uri).ok_or_else(|| format!("没有资源 {uri}"))?;
    let m = plugin.manifest();
    let page = m
        .page(uri)
        .ok_or_else(|| format!("「{}」插件没有页面 {uri}", m.name))?;
    if !enabled(settings, m) {
        return Err(format!("「{}」插件关着", m.name));
    }
    if let Some(off) = page.needs.iter().find(|s| !source_enabled(settings, m, s)) {
        let name = m.source(off).map(|s| s.name.as_str()).unwrap_or(off);
        return Err(format!("「{}」插件的“{name}”关着，{uri} 用不了", m.name));
    }
    let resource = plugin
        .resource(uri)
        .ok_or_else(|| format!("「{}」的“{}”还不是 App", m.name, page.title))?;
    m.check_resource(&resource).inspect_err(|e| {
        kotone_core::log::log(&format!("插件 {} 的资源不对：{e}", m.id));
    })?;
    Ok(resource)
}

/// 开着的插件做成 App 的页面（`resources/list` 的项，名字是页面标题）。
pub fn list(settings: &Settings) -> Vec<Value> {
    registry()
        .iter()
        .filter(|p| enabled(settings, p.manifest()))
        .flat_map(|p| {
            p.manifest()
                .pages
                .iter()
                .filter_map(|page| {
                    read(settings, &page.uri)
                        .ok()
                        .map(|r| r.listing(&page.title))
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

/// 回环代理里一个插件的 App 从哪来。
pub struct PluginSource {
    app: AppHandle,
    plugin: Arc<dyn Plugin>,
}

impl PluginSource {
    pub fn new(app: &AppHandle, plugin: &Arc<dyn Plugin>) -> Arc<Self> {
        Arc::new(Self {
            app: app.clone(),
            plugin: plugin.clone(),
        })
    }
}

impl crate::app_proxy::Source for PluginSource {
    fn plugin(&self) -> &str {
        &self.plugin.manifest().id
    }

    fn resource(&self, page: &str) -> Result<Resource, String> {
        let uri = format!("ui://kotone/{}/{page}", self.plugin.manifest().id);
        read(&settings(&self.app), &uri)
    }

    fn asset(&self, path: String) -> BoxFuture<'static, Option<Asset>> {
        if !enabled(&settings(&self.app), self.plugin.manifest()) {
            return Box::pin(async { None });
        }
        let plugin = self.plugin.clone();
        let host = crate::plugins::DesktopHost::for_plugin(&self.app, &plugin);
        Box::pin(async move { plugin.asset(host, &path).await })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 英雄联盟的强度榜是 App（没打包时是占位页），还没换过来的页面不是；关掉插件或“用网上的统计”就没有了；覆盖层不是资源。
    #[test]
    fn app_pages_follow_the_switches() {
        let s = Settings::default();
        let tier = read(&s, "ui://kotone/lol/tier").unwrap();
        assert_eq!(tier.mime_type, kotone_plugin_api::APP_MIME);
        assert!(tier.text.contains("kotone:page"));
        let listed = list(&s);
        let uris: Vec<&str> = listed.iter().filter_map(|r| r["uri"].as_str()).collect();
        assert!(uris.contains(&"ui://kotone/lol/tier"), "{uris:?}");
        // 还没换成 App 的页面（一页页换过来，挑一个现在还不是的）
        let not_app = kotone_plugin_lol::manifest()
            .pages
            .iter()
            .map(|p| p.uri.as_str())
            .find(|uri| !kotone_plugin_lol::app::APP_PAGES.contains(uri))
            .expect("还有没换成 App 的页面");
        assert!(!uris.contains(&not_app), "{uris:?}");
        let tier_listing = listed
            .iter()
            .find(|r| r["uri"] == "ui://kotone/lol/tier")
            .unwrap();
        assert_eq!(tier_listing["name"], "强度榜");
        assert_eq!(
            tier_listing["_meta"]["ui"]["csp"]["resourceDomains"][0],
            "https://raw.communitydragon.org"
        );

        assert!(read(&s, not_app).unwrap_err().contains("还不是 App"));
        assert!(read(&s, "ui://kotone/lol/champ-select").is_err(), "覆盖层");
        assert!(read(&s, "ui://kotone/nope/tier").is_err());

        let mut off = Settings::default();
        off.plugins
            .entry("lol".into())
            .or_default()
            .sources
            .insert("stats".into(), false);
        assert!(read(&off, "ui://kotone/lol/tier")
            .unwrap_err()
            .contains("关着"));
        assert!(list(&off)
            .iter()
            .all(|r| r["uri"] != "ui://kotone/lol/tier"));
        let mut disabled = Settings::default();
        disabled.plugins.entry("lol".into()).or_default().enabled = Some(false);
        assert!(read(&disabled, "ui://kotone/lol/tier").is_err());
    }

    /// 回环代理的 `/assets/lol/<路径>` 走到英雄联盟插件的素材层（`Plugin::asset` → `fetch_asset`）：
    /// 客户端没开时问 CommunityDragon（这里是本机的假服务器，`KOTONE_LOL_CDRAGON`，不连真网），
    /// 第二次走磁盘缓存；三个前缀以外的路径插件不放行，假服务器也没被问到。
    #[tokio::test]
    async fn the_proxy_takes_lol_assets_from_the_plugin() {
        use std::path::PathBuf;
        use std::sync::atomic::{AtomicUsize, Ordering};

        use kotone_plugin_api::{Host, Observation};
        use serde_json::Map;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        /// 只有素材层用得到的宿主：插件和开关都开着，没有客户端进程。
        struct TestHost(PathBuf);
        impl Host for TestHost {
            fn plugin(&self) -> &str {
                "lol"
            }
            fn enabled(&self) -> bool {
                true
            }
            fn source_enabled(&self, _: &str) -> bool {
                true
            }
            fn settings(&self) -> Value {
                Value::Null
            }
            fn data_dir(&self) -> PathBuf {
                self.0.clone()
            }
            fn spawn(&self, _: BoxFuture<'static, ()>) {}
            fn process_ids(&self, _: &str) -> Vec<u32> {
                Vec::new()
            }
            fn process_command_line(&self, _: u32) -> Option<String> {
                None
            }
            fn game_observe(&self, _: &str, _: Observation) {}
            fn game_claim(&self, _: &str, _: bool) {}
            fn game_event(&self, _: &str, _: &str, _: &str, _: Map<String, Value>, _: Option<f64>) {
            }
        }

        /// 和 `PluginSource` 一样把素材交给插件（少了读设置：这里插件一直开着）。
        struct LolSource(Arc<dyn Plugin>, Arc<dyn Host>);
        impl crate::app_proxy::Source for LolSource {
            fn plugin(&self) -> &str {
                "lol"
            }
            fn resource(&self, _: &str) -> Result<Resource, String> {
                Err("这个测试只取素材".into())
            }
            fn asset(&self, path: String) -> BoxFuture<'static, Option<Asset>> {
                let (plugin, host) = (self.0.clone(), self.1.clone());
                Box::pin(async move { plugin.asset(host, &path).await })
            }
        }

        async fn get(origin: &str, path: &str) -> (u16, String, Vec<u8>) {
            let host = origin.trim_start_matches("http://");
            let mut stream = tokio::net::TcpStream::connect(host).await.unwrap();
            let request =
                format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");
            stream.write_all(request.as_bytes()).await.unwrap();
            let mut reply = Vec::new();
            stream.read_to_end(&mut reply).await.unwrap();
            let split = reply.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
            let head = String::from_utf8_lossy(&reply[..split]).to_string();
            let status = head.split_whitespace().nth(1).unwrap().parse().unwrap();
            (
                status,
                head.to_ascii_lowercase(),
                reply[split + 4..].to_vec(),
            )
        }

        // 本机的假 CommunityDragon：只有阿狸的头像（一段 PNG 文件头，不是拳头的图）
        let png = b"\x89PNG\r\n\x1a\nfake".to_vec();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let asked = Arc::new(AtomicUsize::new(0));
        let (served, count) = (png.clone(), asked.clone());
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                count.fetch_add(1, Ordering::SeqCst);
                let mut buf = vec![0u8; 4096];
                let n = socket.read(&mut buf).await.unwrap_or(0);
                let head = String::from_utf8_lossy(&buf[..n]).to_string();
                let path = head.split_whitespace().nth(1).unwrap_or("").to_string();
                let reply = if path
                    == "/latest/plugins/rcp-be-lol-game-data/global/default/v1/champion-icons/103.png"
                {
                    let mut r = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        served.len()
                    )
                    .into_bytes();
                    r.extend_from_slice(&served);
                    r
                } else {
                    b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
                };
                let _ = socket.write_all(&reply).await;
                let _ = socket.shutdown().await;
            }
        });
        // 这个测试进程里只有这里用英雄联盟的素材层（它第一次用时读这个变量）
        std::env::set_var(
            "KOTONE_LOL_CDRAGON",
            format!("http://127.0.0.1:{port}/latest"),
        );
        let dir =
            std::env::temp_dir().join(format!("kotone-proxy-lol-assets-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let plugin: Arc<dyn Plugin> = Arc::new(kotone_plugin_lol::LolPlugin::new());
        let host: Arc<dyn Host> = Arc::new(TestHost(dir.clone()));
        let proxy = crate::app_proxy::Proxy::start(Arc::new(LolSource(plugin, host)))
            .await
            .unwrap();
        // 素材地址：沙盒页地址里的 `/sandbox/lol/<页面>` 换成 `/assets/lol/`
        let sandbox = proxy.sandbox_url("tier");
        let base = sandbox
            .strip_prefix(&proxy.origin())
            .unwrap()
            .replace("/sandbox/lol/tier", "/assets/lol/");
        let icon = format!("{base}lol-game-data/assets/v1/champion-icons/103.png");

        let (status, head, body) = get(&proxy.origin(), &icon).await;
        assert_eq!(status, 200, "{head}");
        assert!(head.contains("content-type: image/png"), "{head}");
        assert_eq!(body, png);
        assert_eq!(asked.load(Ordering::SeqCst), 1);
        // 第二次：磁盘缓存，不再问
        let (status, _, body) = get(&proxy.origin(), &icon).await;
        assert_eq!((status, body), (200, png.clone()));
        assert_eq!(asked.load(Ordering::SeqCst), 1);
        assert_eq!(dir.join("asset-cache").read_dir().unwrap().count(), 1);
        // 代理的写法检查过得了、插件不放行的（客户端的别的接口、不是图片）：404，假服务器没被问
        for refused in [
            "lol-champ-select/v1/session.png",
            "lol-game-data/assets/v1/champion-summary.json",
        ] {
            let (status, _, _) = get(&proxy.origin(), &format!("{base}{refused}")).await;
            assert_eq!(status, 404, "{refused}");
        }
        assert_eq!(asked.load(Ordering::SeqCst), 1);
        // 哪儿都没有的图：404
        let (status, _, _) = get(
            &proxy.origin(),
            &format!("{base}lol-game-data/assets/v1/champion-icons/999999.png"),
        )
        .await;
        assert_eq!(status, 404);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
