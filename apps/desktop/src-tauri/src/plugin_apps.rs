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

    fn resource(&self, page: &str) -> BoxFuture<'static, Result<Resource, String>> {
        let uri = format!("ui://kotone/{}/{page}", self.plugin.manifest().id);
        let resource = read(&settings(&self.app), &uri);
        Box::pin(async move { resource })
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

/// 第三方插件（MCP 方案第 2 期）的一个 MCP App 页面：插件开着、页面是它给的（归属按“哪个服务器给的”记）。
/// 页面从服务器的 `resources/read` 取，CSP 用它声明的。`uri` 和 `slug`（代理路径里的那一段）给一个。
/// 第三方的页面只在 Kotone 里显示，不进工具服务的 `resources/list`（[`list`] 只看内置插件）。
pub async fn read_remote(
    app: &AppHandle,
    id: &str,
    uri: Option<&str>,
    slug: Option<&str>,
) -> Result<Resource, String> {
    let plugin = crate::remote_plugins::find(id).ok_or_else(|| format!("没有插件 {id}"))?;
    if !settings(app).plugin_enabled(id, true) {
        return Err(format!("「{id}」插件关着（在左侧栏的「插件」里可以打开）"));
    }
    let server = plugin.server().clone();
    let uri = match (uri, slug) {
        (Some(uri), _) => uri.to_string(),
        (None, Some(slug)) => server
            .uri_for_slug(slug)
            .ok_or_else(|| format!("「{id}」插件没有这个页面"))?,
        (None, None) => return Err("要写页面地址".into()),
    };
    let host = crate::remote_plugins::host(app, id);
    server.read_app(&host, &uri).await
}

/// 哪个开着的第三方插件给了这个 `ui://` 地址。`plugin` 写了就只看它；没写、又有两个插件给了同一个地址时不猜。
pub fn remote_owner(
    settings: &Settings,
    uri: &str,
    plugin: Option<&str>,
) -> Result<String, String> {
    let owners: Vec<String> = crate::remote_plugins::plugins()
        .iter()
        .filter(|p| plugin.map_or(true, |id| p.manifest().id == id))
        .filter(|p| settings.plugin_enabled(&p.manifest().id, true))
        .filter(|p| p.server().owns_uri(uri))
        .map(|p| p.manifest().id.clone())
        .collect();
    match owners.as_slice() {
        [one] => Ok(one.clone()),
        [] => Err(format!("没有开着的插件给了 {uri}")),
        _ => Err(format!(
            "{uri} 有好几个插件都给了（{}），要写明是哪个插件的",
            owners.join("、")
        )),
    }
}

/// 回环代理里一个第三方插件的 App 从哪来。每次按 id 找插件（配置改了会换成新的插件，代理还是同一个）；没有素材。
pub struct RemoteSource {
    app: AppHandle,
    id: String,
}

impl RemoteSource {
    pub fn new(app: &AppHandle, id: &str) -> Arc<Self> {
        Arc::new(Self {
            app: app.clone(),
            id: id.to_string(),
        })
    }
}

impl crate::app_proxy::Source for RemoteSource {
    fn plugin(&self) -> &str {
        &self.id
    }

    fn resource(&self, page: &str) -> BoxFuture<'static, Result<Resource, String>> {
        let (app, id, page) = (self.app.clone(), self.id.clone(), page.to_string());
        Box::pin(async move { read_remote(&app, &id, None, Some(&page)).await })
    }

    fn asset(&self, _path: String) -> BoxFuture<'static, Option<Asset>> {
        Box::pin(async { None })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOL_CLIENT_APPS: [&str; 3] = [
        "ui://kotone/lol/match",
        "ui://kotone/lol/player",
        "ui://kotone/lol/game",
    ];
    /// 要“用网上的统计”的：英雄、强度榜，和只在推送里出现的选英雄、文字识别模型两张卡片（ADR 043 D7）。
    const LOL_STATS_APPS: [&str; 4] = [
        "ui://kotone/lol/champion",
        "ui://kotone/lol/tier",
        "ui://kotone/lol/champ-select",
        "ui://kotone/lol/augment-ocr",
    ];

    fn listed_uris(settings: &Settings) -> Vec<String> {
        list(settings)
            .iter()
            .filter_map(|r| r["uri"].as_str().map(str::to_string))
            .collect()
    }

    fn with_source_off(source: &str) -> Settings {
        let mut s = Settings::default();
        s.plugins
            .entry("lol".into())
            .or_default()
            .sources
            .insert(source.into(), false);
        s
    }

    /// 主页只依赖插件总开关，各数据页还要检查相应的数据源。
    #[test]
    fn app_pages_follow_the_switches() {
        const HOME: &str = "ui://kotone/lol/home";
        let s = Settings::default();
        let uris = listed_uris(&s);
        for uri in [&HOME]
            .into_iter()
            .chain(&LOL_CLIENT_APPS)
            .chain(&LOL_STATS_APPS)
        {
            let r = read(&s, uri).unwrap_or_else(|e| panic!("{uri}：{e}"));
            assert_eq!(r.mime_type, kotone_plugin_api::APP_MIME, "{uri}");
            let page = uri.rsplit('/').next().unwrap();
            assert!(
                r.text
                    .contains(&format!("<meta name=\"kotone:page\" content=\"{page}\">")),
                "{uri}"
            );
            assert!(uris.iter().any(|u| u == uri), "{uri} 不在 {uris:?}");
        }
        let listed = list(&s);
        let tier_listing = listed
            .iter()
            .find(|r| r["uri"] == "ui://kotone/lol/tier")
            .unwrap();
        assert_eq!(tier_listing["name"], "强度榜");
        assert_eq!(
            tier_listing["_meta"]["ui"]["csp"]["resourceDomains"][0],
            "https://raw.communitydragon.org"
        );
        let player_listing = listed
            .iter()
            .find(|r| r["uri"] == "ui://kotone/lol/player")
            .unwrap();
        assert_eq!(player_listing["name"], "战绩");

        // 设置是宿主的页面，不是 App
        assert!(!uris.iter().any(|u| u == "ui://kotone/lol/settings"));
        assert!(read(&s, "ui://kotone/lol/settings")
            .unwrap_err()
            .contains("不是 App"));
        // 原来的海克斯评分只是覆盖层，覆盖层去掉了（ADR 043 D7）：它从来不是页面
        assert!(read(&s, "ui://kotone/lol/augment-scores").is_err());
        assert!(read(&s, "ui://kotone/nope/tier").is_err());

        for (source, gone, kept) in [
            ("stats", &LOL_STATS_APPS[..], &LOL_CLIENT_APPS[..]),
            ("client", &LOL_CLIENT_APPS[..], &LOL_STATS_APPS[..]),
        ] {
            let off = with_source_off(source);
            let uris = listed_uris(&off);
            for uri in gone {
                assert!(read(&off, uri).unwrap_err().contains("关着"), "{uri}");
                assert!(!uris.iter().any(|u| u == uri), "{source} 关着还列了 {uri}");
            }
            for uri in kept {
                assert!(read(&off, uri).is_ok(), "{source} 关着不该影响 {uri}");
            }
        }
        let mut no_sources = with_source_off("client");
        no_sources
            .plugins
            .get_mut("lol")
            .unwrap()
            .sources
            .insert("stats".into(), false);
        assert!(listed_uris(&no_sources).iter().any(|uri| uri == HOME));
        assert!(read(&no_sources, HOME).is_ok());
        let mut disabled = Settings::default();
        disabled.plugins.entry("lol".into()).or_default().enabled = Some(false);
        assert!(read(&disabled, HOME).is_err());
        assert!(read(&disabled, "ui://kotone/lol/tier").is_err());
        assert!(read(&disabled, "ui://kotone/lol/player").is_err());
    }

    /// 哪些页面是 App，三处要一样（原则 16：边界写成测试）：
    /// - Rust：插件的 `resource` 给不给（英雄联盟是 `plugins/lol/src/app.rs` 的 `APP_PAGES`）；
    /// - 前端：`plugins/<id>/ui/index.ts` 的 `views` 里标了 `app: true` 的（宿主据此用 McpAppFrame 打开它）；
    /// - 浏览器预览的模拟：`apps/desktop/src/lib/ipc/pluginsMock.ts` 的 `APP_PAGES`（连它记的数据开关和清单的 `needs`）。
    ///
    /// 2026-11-25 前端和模拟标了五页、Rust 只有两页：真 Kotone 里战绩、单局详情、英雄打开就是“页面没有”，
    /// 浏览器预览和 Playwright 却照过。
    #[test]
    fn app_pages_agree_with_the_frontend() {
        use std::collections::{BTreeMap, BTreeSet};
        use std::path::Path;

        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let read_file = |path: &Path| {
            std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}：{e}", path.display()))
        };
        // 地址 → 清单的 needs，所有内置插件做成 App 的页面
        let mut rust_apps: BTreeMap<String, Vec<String>> = BTreeMap::new();
        // 常量名 → 地址（所有插件的 ui/；同名不同值的记成 None，用到时报错）
        let mut all_constants: BTreeMap<String, Option<String>> = BTreeMap::new();
        for plugin in crate::plugins::registry() {
            let m = plugin.manifest();
            let ui = repo.join("plugins").join(&m.id).join("ui");
            let constants = ts_ui_constants(&ui);
            for (name, uri) in &constants {
                let entry = all_constants
                    .entry(name.clone())
                    .or_insert_with(|| Some(uri.clone()));
                if entry.as_deref() != Some(uri.as_str()) {
                    *entry = None;
                }
            }
            let rust: BTreeSet<String> = m
                .pages
                .iter()
                .filter(|p| plugin.resource(&p.uri).is_some())
                .map(|p| p.uri.clone())
                .collect();
            let index = ui.join("index.ts");
            let front = if index.exists() {
                ts_app_views(&read_file(&index), |name| constants.get(name).cloned())
            } else {
                BTreeSet::new()
            };
            assert_eq!(
                front,
                rust,
                "「{}」：{} 里标了 app: true 的页面和插件的 resource（Rust）对不上",
                m.name,
                index.display()
            );
            for uri in rust {
                let needs = m.page(&uri).unwrap().needs.clone();
                rust_apps.insert(uri, needs);
            }
        }
        assert!(
            rust_apps.contains_key("ui://kotone/lol/player")
                && rust_apps.contains_key("ui://kotone/lol/tier"),
            "{rust_apps:?}"
        );

        let mock_path = repo.join("apps/desktop/src/lib/ipc/pluginsMock.ts");
        let mock = ts_mock_app_pages(&read_file(&mock_path), |name| {
            match all_constants.get(name) {
                Some(Some(uri)) => Some(uri.clone()),
                Some(None) => panic!("{name} 在好几个插件里是不同的地址，模拟里别用它"),
                None => None,
            }
        });
        assert_eq!(
            mock.keys().collect::<BTreeSet<_>>(),
            rust_apps.keys().collect::<BTreeSet<_>>(),
            "pluginsMock.ts 的 APP_PAGES 和插件的 resource（Rust）对不上"
        );
        for (uri, source) in &mock {
            assert_eq!(
                source.as_slice(),
                rust_apps[uri].as_slice(),
                "{uri}：pluginsMock.ts 记的数据开关和清单的 needs 对不上"
            );
        }
    }

    /// 去掉整行注释（`//`、`/*`、` *` 开头的行；`ui://` 里的 `//` 不算）。
    fn ts_strip_comment_lines(src: &str) -> String {
        src.lines()
            .filter(|line| {
                let t = line.trim_start();
                !(t.starts_with("//") || t.starts_with("/*") || t.starts_with('*'))
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// 从 `at` 起的字符串字面量（`"…"` 或 `'…'`）的内容。
    fn ts_string_at(src: &str) -> Option<String> {
        let quote = src.chars().next().filter(|c| *c == '"' || *c == '\'')?;
        let rest = &src[1..];
        rest.find(quote).map(|end| rest[..end].to_string())
    }

    /// `dir` 下（含子目录，不进 node_modules）`.ts` 文件里的 `export const NAME = "ui://…"`。
    fn ts_ui_constants(dir: &std::path::Path) -> std::collections::BTreeMap<String, String> {
        let mut out = std::collections::BTreeMap::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if path.file_name().is_some_and(|n| n != "node_modules") {
                        stack.push(path);
                    }
                    continue;
                }
                if path.extension() != Some(std::ffi::OsStr::new("ts")) {
                    continue;
                }
                let text = std::fs::read_to_string(&path).unwrap_or_default();
                for line in text.lines() {
                    let Some(rest) = line.trim_start().strip_prefix("export const ") else {
                        continue;
                    };
                    let Some((name, value)) = rest.split_once('=') else {
                        continue;
                    };
                    let name = name.split(':').next().unwrap().trim();
                    if let Some(uri) = ts_string_at(value.trim_start()) {
                        if uri.starts_with("ui://") {
                            out.insert(name.to_string(), uri);
                        }
                    }
                }
            }
        }
        out
    }

    /// 对象字面量里一项的键：`[NAME]`（查常量）或 `"ui://…"`。`before` 是 `{` 前面的文字（以 `:` 结尾）。
    fn ts_key_before(before: &str, constant: &impl Fn(&str) -> Option<String>) -> String {
        let key = before
            .trim_end()
            .strip_suffix(':')
            .unwrap_or_else(|| panic!("看不懂这一项的键：…{}", tail(before)))
            .trim_end();
        if let Some(inner) = key.strip_suffix(']') {
            let name = inner[inner.rfind('[').expect("[NAME]") + 1..].trim();
            return constant(name).unwrap_or_else(|| {
                panic!("找不到常量 {name}（要是 export const {name} = \"ui://…\"）")
            });
        }
        if let Some(inner) = key.strip_suffix('"').or_else(|| key.strip_suffix('\'')) {
            let start = inner.rfind(['"', '\'']).expect("字符串键");
            return inner[start + 1..].to_string();
        }
        panic!("看不懂这一项的键：…{}", tail(key))
    }

    fn tail(s: &str) -> &str {
        let start = s.char_indices().rev().nth(60).map_or(0, |(i, _)| i);
        &s[start..]
    }

    /// `ui/index.ts` 里标了 `app: true` 的页面（`[PAGE_X]: { app: true, … }`，可以跨行）。
    fn ts_app_views(
        src: &str,
        constant: impl Fn(&str) -> Option<String>,
    ) -> std::collections::BTreeSet<String> {
        let src = ts_strip_comment_lines(src);
        let mut out = std::collections::BTreeSet::new();
        for (at, _) in src.match_indices("app:") {
            let after = src[at + 4..].trim_start();
            let word_before = src[..at]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '$');
            if word_before || !after.starts_with("true") {
                continue;
            }
            // 往回找包着它的 `{`
            let mut depth = 0usize;
            let brace = src[..at]
                .char_indices()
                .rev()
                .find(|&(_, c)| match c {
                    '}' => {
                        depth += 1;
                        false
                    }
                    '{' if depth == 0 => true,
                    '{' => {
                        depth -= 1;
                        false
                    }
                    _ => false,
                })
                .map(|(i, _)| i)
                .expect("app: true 外面没有 {");
            out.insert(ts_key_before(&src[..brace], &constant));
        }
        out
    }

    /// 模拟资源的地址和所需的数据开关。没有额外开关的页面记作 `null`。
    fn ts_mock_app_pages(
        src: &str,
        constant: impl Fn(&str) -> Option<String>,
    ) -> std::collections::BTreeMap<String, Option<String>> {
        let src = ts_strip_comment_lines(src);
        let at = src
            .find("const APP_PAGES")
            .expect("pluginsMock.ts 里没有 APP_PAGES");
        let open = at + src[at..].find('{').expect("APP_PAGES = {");
        let close = open + src[open..].find('}').expect("APP_PAGES 的 }");
        let mut out = std::collections::BTreeMap::new();
        for item in src[open + 1..close].split(',') {
            let item = item.trim();
            if item.is_empty() {
                continue;
            }
            let colon = if item.starts_with('[') {
                item.find("]:").map(|i| i + 1)
            } else {
                item.find("\":").or_else(|| item.find("':")).map(|i| i + 1)
            }
            .unwrap_or_else(|| panic!("看不懂 APP_PAGES 的这一项：{item}"));
            let uri = ts_key_before(&item[..=colon], &constant);
            let value = item[colon + 1..].trim();
            let source = match value {
                "null" => None,
                _ => Some(
                    ts_string_at(value).unwrap_or_else(|| panic!("看不懂 APP_PAGES 的值：{item}")),
                ),
            };
            out.insert(uri, source);
        }
        out
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
            fn resource(&self, _: &str) -> BoxFuture<'static, Result<Resource, String>> {
                Box::pin(async { Err("这个测试只取素材".into()) })
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
