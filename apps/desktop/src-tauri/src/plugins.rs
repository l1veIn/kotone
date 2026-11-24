//! 插件的宿主（ADR 038）：插件注册、开关、交给插件的本体能力（[`DesktopHost`]），以及插件工具的三个出口——
//! 她的工具清单（[`offers`]）、工具服务 / MCP（[`service_specs`]）、插件自己的页面（`plugins_cmd.rs`）。
//!
//! - 一期都是内置插件：[`registry`] 是一张静态表，编译在一起；
//! - 开关在设置 `plugins.<id>`（`enabled`、`sources.<开关>`），没写按插件清单的默认；
//! - 插件关着时：不出现在她的工具清单、工具服务里，调用被拒；后台循环由插件自己看 [`Host::enabled`] 停下；
//! - 工具只在插件里定义一次（名字、说明、参数、`visibility`、页面、要的数据开关）。

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use kotone_core::games::Observation;
use kotone_plugin_api::{BoxFuture, Host, Manifest, Offer, Plugin, ToolResult, ToolSpec};
use serde_json::{json, Map, Value};
use tauri::{AppHandle, Emitter, Manager};

/// 界面上插件的事件（`{plugin, event, payload}`）。
pub const EVENT: &str = "kotone://plugin";

/// 内置的插件（按侧栏里的顺序）。
pub fn registry() -> &'static [Arc<dyn Plugin>] {
    static PLUGINS: OnceLock<Vec<Arc<dyn Plugin>>> = OnceLock::new();
    PLUGINS.get_or_init(|| vec![Arc::new(kotone_plugin_lol::LolPlugin::new()) as Arc<dyn Plugin>])
}

#[cfg(test)]
fn find(id: &str) -> Option<&'static Arc<dyn Plugin>> {
    registry().iter().find(|p| p.manifest().id == id)
}

/// 这个工具是哪个插件的（不管开没开）。
pub fn owner(tool: &str) -> Option<&'static Arc<dyn Plugin>> {
    registry()
        .iter()
        .find(|p| p.tools().iter().any(|t| t.name == tool))
}

fn settings(app: &AppHandle) -> kotone_core::settings::Settings {
    app.state::<crate::SharedState>()
        .settings
        .read()
        .unwrap()
        .clone()
}

/// 插件开着没有。
pub fn enabled(settings: &kotone_core::settings::Settings, manifest: &Manifest) -> bool {
    settings.plugin_enabled(&manifest.id, manifest.enabled_by_default)
}

/// 插件里的某个数据开关开着没有（插件关着也算关）。
pub fn source_enabled(
    settings: &kotone_core::settings::Settings,
    manifest: &Manifest,
    source: &str,
) -> bool {
    enabled(settings, manifest)
        && manifest.source(source).is_some_and(|s| {
            settings.plugin_source_enabled(&manifest.id, source, s.enabled_by_default)
        })
}

/// 插件开着、工具要的数据开关都开着。
fn tool_available(
    settings: &kotone_core::settings::Settings,
    manifest: &Manifest,
    tool: &ToolSpec,
) -> bool {
    enabled(settings, manifest)
        && tool
            .needs
            .iter()
            .all(|s| source_enabled(settings, manifest, s))
}

/// 宿主交给插件的本体能力。每个插件一份。
pub struct DesktopHost {
    app: AppHandle,
    id: String,
    manifest: Manifest,
}

impl DesktopHost {
    pub fn for_plugin(app: &AppHandle, plugin: &Arc<dyn Plugin>) -> Arc<dyn Host> {
        Arc::new(Self {
            app: app.clone(),
            id: plugin.manifest().id.clone(),
            manifest: plugin.manifest().clone(),
        })
    }
}

impl Host for DesktopHost {
    fn plugin(&self) -> &str {
        &self.id
    }

    fn enabled(&self) -> bool {
        enabled(&settings(&self.app), &self.manifest)
    }

    fn source_enabled(&self, source: &str) -> bool {
        source_enabled(&settings(&self.app), &self.manifest, source)
    }

    fn settings(&self) -> Value {
        settings(&self.app)
            .plugins
            .get(&self.id)
            .map(|p| Value::Object(p.settings.clone()))
            .unwrap_or_else(|| json!({}))
    }

    fn data_dir(&self) -> PathBuf {
        kotone_core::settings::kotone_dir()
            .join("plugins")
            .join(&self.id)
    }

    fn spawn(&self, task: BoxFuture<'static, ()>) {
        tauri::async_runtime::spawn(task);
    }

    fn process_ids(&self, exe: &str) -> Vec<u32> {
        kotone_platform_windows::app_audio::pids_by_name(exe)
    }

    fn process_command_line(&self, pid: u32) -> Option<String> {
        kotone_platform_windows::process_cmdline::command_line(pid)
    }

    fn game_observe(&self, game: &str, observation: Observation) {
        crate::games::observe(&self.app, game, observation);
    }

    fn game_claim(&self, game: &str, claimed: bool) {
        crate::games::claim(game, claimed);
    }

    fn game_event(
        &self,
        game: &str,
        kind: &str,
        source: &str,
        data: Map<String, Value>,
        game_time: Option<f64>,
    ) {
        crate::games::push(&self.app, game, kind, source, data, game_time);
    }

    fn emit(&self, event: &str, payload: Value) {
        let _ = self.app.emit(
            EVENT,
            json!({"plugin": self.id, "event": event, "payload": payload}),
        );
    }
}

/// 启动时：每个插件开它的后台循环（循环里自己看开关）。
pub fn init(app: &AppHandle) {
    for plugin in registry() {
        if let Err(error) = plugin.manifest().check_tools(&plugin.tools()) {
            kotone_core::log::log(&format!(
                "插件 {} 的工具不对：{error}",
                plugin.manifest().id
            ));
        }
        plugin.clone().start(DesktopHost::for_plugin(app, plugin));
    }
}

/// 开着的插件给她的东西（她的工具清单和系统提示）。一段对话开始时取一次，之后不变（ADR 024）。
pub fn offers(app: &AppHandle) -> Vec<Offer> {
    let settings = settings(app);
    registry()
        .iter()
        .filter(|p| enabled(&settings, p.manifest()))
        .map(|p| {
            let m = p.manifest();
            Offer {
                plugin: m.id.clone(),
                name: m.name.clone(),
                text: m.agent.clone(),
                tools: p
                    .tools()
                    .into_iter()
                    .filter(|t| t.for_model() && tool_available(&settings, m, t))
                    .collect(),
            }
        })
        .collect()
}

/// 工具服务 `system.capabilities` 里开着的插件的工具（给外部 Agent、MCP；带 `visibility`）。
pub fn service_specs(app: &AppHandle) -> Vec<Value> {
    let settings = settings(app);
    registry()
        .iter()
        .flat_map(|p| {
            let m = p.manifest();
            p.tools()
                .into_iter()
                .filter(|t| tool_available(&settings, m, t))
                .map(|t| t.service_spec(&m.id))
                .collect::<Vec<_>>()
        })
        .collect()
}

/// 调一个插件的工具。`from_page`：插件自己的页面调的（只给页面用的工具也能调，只能调自己插件的）。
/// 插件关着时拒绝；数据开关关着时由插件自己回答（一段对话里她的工具清单不变，开关可能中途关了）。
pub async fn call(
    app: &AppHandle,
    tool: &str,
    arguments: &Value,
    from_page: Option<&str>,
) -> Result<ToolResult, String> {
    let plugin = owner(tool).ok_or_else(|| format!("没有叫 {tool} 的插件工具"))?;
    let manifest = plugin.manifest();
    let spec = plugin
        .tools()
        .into_iter()
        .find(|t| t.name == tool)
        .ok_or_else(|| format!("没有叫 {tool} 的插件工具"))?;
    match from_page {
        Some(page_plugin) if page_plugin != manifest.id => {
            return Err(format!("{page_plugin} 的页面不能调 {tool}"));
        }
        Some(_) if !spec.for_app() => return Err(format!("{tool} 不给页面用")),
        None if !spec.for_model() => return Err(format!("{tool} 只给插件的页面用")),
        _ => {}
    }
    if !enabled(&settings(app), manifest) {
        return Ok(ToolResult::content(json!({
            "status": "disabled",
            "text": format!("「{}」插件关着（设置 → 插件 里可以打开）", manifest.name),
        })));
    }
    plugin
        .call(DesktopHost::for_plugin(app, plugin), tool, arguments)
        .await
}

/// 界面用的插件清单：清单、开关、工具（名字、给谁、页面）。
pub fn list(app: &AppHandle) -> Value {
    let settings = settings(app);
    Value::Array(
        registry()
            .iter()
            .map(|p| {
                let m = p.manifest();
                let sources: Map<String, Value> = m
                    .sources
                    .iter()
                    .map(|s| (s.id.clone(), json!(source_enabled(&settings, m, &s.id))))
                    .collect();
                json!({
                    "manifest": m,
                    "enabled": enabled(&settings, m),
                    "sources": sources,
                    "tools": p.tools().iter().map(|t| json!({
                        "name": t.name,
                        "visibility": t.visibility,
                        "page": t.page,
                        "available": tool_available(&settings, m, t),
                    })).collect::<Vec<_>>(),
                })
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_manifests_and_tools_are_valid() {
        let mut ids = std::collections::HashSet::new();
        let mut tools = std::collections::HashSet::new();
        for p in registry() {
            let m = p.manifest();
            assert!(ids.insert(m.id.clone()), "插件 {} 重名", m.id);
            m.check_tools(&p.tools()).unwrap();
            for t in p.tools() {
                assert!(tools.insert(t.name.clone()), "工具 {} 重名", t.name);
                // 插件的工具不能和本体的工具重名（工具服务、她的清单都按名字找）
                assert!(
                    !kotone_tools::catalog::implemented_tools().contains(&t.name.as_str()),
                    "{} 和工具服务的工具重名",
                    t.name
                );
                assert!(
                    !kotone_agent::catalog::MAIN_TOOLS.contains(&t.name.as_str()),
                    "{} 和她的内置工具重名",
                    t.name
                );
            }
        }
        assert!(find("lol").is_some());
        assert_eq!(
            owner("lol.scout").map(|p| p.manifest().id.as_str()),
            Some("lol")
        );
        assert!(owner("music").is_none());
    }

    #[test]
    fn switches_follow_settings_and_manifest_defaults() {
        let lol = find("lol").unwrap().manifest();
        let mut s = kotone_core::settings::Settings::default();
        assert!(enabled(&s, lol) && source_enabled(&s, lol, "client"));
        let scout = find("lol")
            .unwrap()
            .tools()
            .into_iter()
            .find(|t| t.name == "lol.scout")
            .unwrap();
        assert!(tool_available(&s, lol, &scout));
        // 关掉数据开关：工具不出现
        s.plugins
            .entry("lol".into())
            .or_default()
            .sources
            .insert("client".into(), false);
        assert!(enabled(&s, lol) && !source_enabled(&s, lol, "client"));
        assert!(!tool_available(&s, lol, &scout));
        // 数据开关开着、插件关着：都算关
        let mut s = kotone_core::settings::Settings::default();
        s.plugins.entry("lol".into()).or_default().enabled = Some(false);
        assert!(!enabled(&s, lol) && !source_enabled(&s, lol, "client"));
        assert!(!source_enabled(
            &kotone_core::settings::Settings::default(),
            lol,
            "nope"
        ));
    }
}
