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

    /// 英雄联盟的强度榜是 App（没打包时是占位页），这局还不是；关掉插件或“用网上的统计”就没有了；覆盖层不是资源。
    #[test]
    fn app_pages_follow_the_switches() {
        let s = Settings::default();
        let tier = read(&s, "ui://kotone/lol/tier").unwrap();
        assert_eq!(tier.mime_type, kotone_plugin_api::APP_MIME);
        assert!(tier.text.contains("kotone:page"));
        let listed = list(&s);
        let uris: Vec<&str> = listed.iter().filter_map(|r| r["uri"].as_str()).collect();
        assert!(uris.contains(&"ui://kotone/lol/tier"), "{uris:?}");
        assert!(!uris.contains(&"ui://kotone/lol/match"), "{uris:?}");
        let tier_listing = listed
            .iter()
            .find(|r| r["uri"] == "ui://kotone/lol/tier")
            .unwrap();
        assert_eq!(tier_listing["name"], "强度榜");
        assert_eq!(
            tier_listing["_meta"]["ui"]["csp"]["resourceDomains"][0],
            "https://raw.communitydragon.org"
        );

        assert!(read(&s, "ui://kotone/lol/match")
            .unwrap_err()
            .contains("还不是 App"));
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
}
