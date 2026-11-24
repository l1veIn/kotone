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
/// 打开一个页面（`ui.open`）：`{uri, params, title, plugin?}`。插件的页面在右侧栏，设置页在设置窗口。
pub const UI_OPEN_EVENT: &str = "kotone://ui-open";
/// 本体的设置页（`ui://kotone/settings/<分页>`，和界面 `advanced/types.ts` 的分页一致）。
pub const SETTINGS_SECTIONS: &[&str] = &[
    "call",
    "reply",
    "permissions",
    "usage",
    "models",
    "connections",
    "sound",
    "tuning",
    "appearance",
    "system",
    "browser",
    "plugins",
];

/// 内置的插件（按侧栏里的顺序）。
pub fn registry() -> &'static [Arc<dyn Plugin>] {
    static PLUGINS: OnceLock<Vec<Arc<dyn Plugin>>> = OnceLock::new();
    PLUGINS.get_or_init(|| {
        vec![
            Arc::new(kotone_plugin_lol::LolPlugin::new()) as Arc<dyn Plugin>,
            Arc::new(kotone_plugin_game_voice::GameVoicePlugin::new()),
            Arc::new(kotone_plugin_music::MusicPlugin::new()),
            Arc::new(kotone_plugin_interpreter::InterpreterPlugin),
        ]
    })
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

/// 在这个线程上临时用“低于正常”的优先级，离开作用域时改回来。
struct BelowNormalThread(#[cfg(windows)] i32);

impl BelowNormalThread {
    fn enter() -> Self {
        #[cfg(windows)]
        {
            use windows::Win32::System::Threading::{
                GetCurrentThread, GetThreadPriority, SetThreadPriority,
                THREAD_PRIORITY_BELOW_NORMAL,
            };
            // SAFETY: 只改当前线程的优先级。
            unsafe {
                let old = GetThreadPriority(GetCurrentThread());
                let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL);
                Self(old)
            }
        }
        #[cfg(not(windows))]
        Self()
    }
}

impl Drop for BelowNormalThread {
    fn drop(&mut self) {
        #[cfg(windows)]
        // SAFETY: 改回进来时的优先级。
        unsafe {
            use windows::Win32::System::Threading::{
                GetCurrentThread, SetThreadPriority, THREAD_PRIORITY,
            };
            let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY(self.0));
        }
    }
}

/// 宿主交给插件的本体能力。每个插件一份。
pub struct DesktopHost {
    app: AppHandle,
    id: String,
    manifest: Manifest,
}

impl DesktopHost {
    /// 凭据库里的名字：`<插件 id>:<name>`（放歌是 `music:netease`，和搬进插件以前一样）。
    fn secret_id(&self, name: &str) -> String {
        format!("{}:{name}", self.id)
    }

    /// 插件的网页登录窗口（放歌是 `music-login`，和搬进插件以前一样）。
    fn web_window_label(&self) -> String {
        format!("{}-login", self.id)
    }

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

    fn app_version(&self) -> String {
        self.app.package_info().version.to_string()
    }

    fn overlay(&self, items: Vec<kotone_plugin_api::OverlayItem>) {
        // 只认清单里登记过的覆盖层；插件关着、或者覆盖层要的数据开关关着时不显示
        let settings = settings(&self.app);
        let items: Vec<_> = items
            .into_iter()
            .filter(|item| {
                self.manifest
                    .overlays
                    .iter()
                    .find(|o| o.uri == item.uri)
                    .is_some_and(|o| {
                        enabled(&settings, &self.manifest)
                            && o.needs
                                .iter()
                                .all(|s| source_enabled(&settings, &self.manifest, s))
                    })
            })
            .collect();
        // 记一笔（插件只在变了时才发）：真机测试靠它看覆盖层要了什么，不用真的盖在玩家屏幕上
        let uris: Vec<&str> = items.iter().map(|i| i.uri.as_str()).collect();
        kotone_core::log::log(&format!(
            "插件 {} 的覆盖层：{} 项 {:?}",
            self.id,
            items.len(),
            uris
        ));
        crate::overlay_layer::set(&self.app, &self.id, items);
    }

    fn screen_text(
        &self,
        process: &str,
        region: [f64; 4],
        local_only: bool,
    ) -> BoxFuture<'static, Result<Vec<kotone_plugin_api::ScreenText>, String>> {
        let app = self.app.clone();
        let process = process.to_string();
        Box::pin(async move {
            tauri::async_runtime::spawn_blocking(move || {
                // 照 aramhex：识别线程用低于正常的优先级（不用最低：低配机器上会饿死）
                let _priority = BelowNormalThread::enter();
                crate::computer_use::screen_text(&app, &process, region, local_only)
            })
            .await
            .map_err(|e| e.to_string())?
        })
    }

    fn emit(&self, event: &str, payload: Value) {
        let _ = self.app.emit(
            EVENT,
            json!({"plugin": self.id, "event": event, "payload": payload}),
        );
    }

    fn companion_enabled(&self) -> bool {
        !settings(&self.app)
            .ui
            .legacy_upgrade
            .uses_legacy_assistant()
    }

    fn update_settings(&self, patch: Value) -> Result<(), String> {
        // 和玩家在界面上保存一样走一遍（通知各窗口、插件的 settings_changed）
        let id = self.id.as_str();
        crate::settings_cmd::update_settings_from_backend(
            &self.app,
            json!({"plugins": {id: {"settings": patch}}}),
        )
        .map(|_| ())
    }

    fn can_synthesize(&self) -> bool {
        let assistant = settings(&self.app).assistant;
        crate::assistant::effective_engine(&self.app, &assistant)
            != kotone_core::assistant::TtsEngine::System
    }

    fn synthesize(&self, text: &str) -> BoxFuture<'static, Result<(Vec<f32>, u32), String>> {
        let app = self.app.clone();
        let text = text.to_string();
        Box::pin(async move {
            tauri::async_runtime::spawn_blocking(move || {
                let (speaker, request) = crate::assistant::speaker_and_request(&app, &text)?;
                speaker.synthesize(request)
            })
            .await
            .map_err(|e| e.to_string())?
        })
    }

    fn speak(&self, text: &str) -> BoxFuture<'static, Result<(), String>> {
        let app = self.app.clone();
        let text = text.to_string();
        Box::pin(async move {
            tauri::async_runtime::spawn_blocking(move || {
                let (speaker, request) = crate::assistant::speaker_and_request(&app, &text)?;
                speaker.speak(request)
            })
            .await
            .map_err(|e| e.to_string())?
        })
    }

    fn player_says(&self, text: &str) -> Result<(), String> {
        // 和在聊天窗口里打字一样（来源 chat：算玩家亲口说的，回复写在聊天里）
        crate::agent::send(
            &self.app,
            crate::agent::Outgoing {
                text: text.to_string(),
                source: "chat".into(),
                attachments: Vec::new(),
            },
        )
        .map(|_| ())
    }

    fn listening(&self) -> Vec<String> {
        crate::voice_listen::active(&self.app)
            .into_iter()
            .map(|(_, app)| app)
            .collect()
    }

    fn she_is_audible(&self) -> bool {
        crate::voice_reply::active()
            || crate::assistant::speaking_now()
            || crate::assistant::output_busy(&self.app)
    }

    fn secret(&self, name: &str) -> Option<String> {
        use kotone_core::connection::SecretStore;
        kotone_ai::secrets::KeyringSecretStore
            .get(&self.secret_id(name))
            .ok()
            .flatten()
    }

    fn set_secret(&self, name: &str, value: &str) -> Result<(), String> {
        use kotone_core::connection::SecretStore;
        kotone_ai::secrets::KeyringSecretStore
            .set(&self.secret_id(name), value)
            .map_err(|e| e.to_string())
    }

    fn delete_secret(&self, name: &str) {
        use kotone_core::connection::SecretStore;
        let _ = kotone_ai::secrets::KeyringSecretStore.delete(&self.secret_id(name));
    }

    fn open_web_window(&self, url: &str, title: &str) -> Result<(), String> {
        let label = self.web_window_label();
        if let Some(window) = self.app.get_webview_window(&label) {
            let _ = window.unminimize();
            let _ = window.show();
            let _ = window.set_focus();
            return Ok(());
        }
        let url = url.parse().map_err(|e| format!("网址不对：{e}"))?;
        // 无痕（WebView2 InPrivate，和主窗口同一个浏览器进程）：网页数据关窗口就没了，登录凭据只在插件那边。
        // 用单独的数据目录会另起一个浏览器进程，真机测试的调试端口会被它占走。
        tauri::WebviewWindowBuilder::new(&self.app, &label, tauri::WebviewUrl::External(url))
            .title(title)
            .inner_size(1000.0, 720.0)
            .center()
            .incognito(true)
            .build()
            .map_err(|e| format!("打不开网页窗口：{e}"))?;
        kotone_core::log::log(&format!("插件 {} 的网页窗口建好了（无痕）", self.id));
        Ok(())
    }

    fn web_window_open(&self) -> bool {
        self.app
            .get_webview_window(&self.web_window_label())
            .is_some()
    }

    fn web_window_cookies(
        &self,
        url: &str,
    ) -> BoxFuture<'static, Result<Vec<(String, String)>, String>> {
        let window = self.app.get_webview_window(&self.web_window_label());
        let url = url.parse::<url::Url>();
        Box::pin(async move {
            let window = window.ok_or("网页窗口没了")?;
            let url = url.map_err(|e| format!("{e}"))?;
            // 读 Cookie 在 Windows 上不能在主线程 / 同步回调里做（会卡死）；要主线程配合，万一卡住 5 秒后不等了
            let read = tauri::async_runtime::spawn_blocking(move || window.cookies_for_url(url));
            let cookies = tokio::time::timeout(std::time::Duration::from_secs(5), read)
                .await
                .map_err(|_| "读 Cookie 超过 5 秒没回来".to_string())?
                .map_err(|e| format!("读 Cookie 的线程出错：{e}"))?
                .map_err(|e| format!("读 Cookie 失败：{e}"))?;
            Ok(cookies
                .iter()
                .map(|c| (c.name().to_string(), c.value().to_string()))
                .collect())
        })
    }

    fn close_web_window(&self) {
        if let Some(window) = self.app.get_webview_window(&self.web_window_label()) {
            let _ = window.destroy();
        }
    }

    fn download(
        &self,
        url: &str,
        dest: PathBuf,
        sha256: Option<&str>,
    ) -> BoxFuture<'static, Result<(), String>> {
        let url = url.to_string();
        let sha256 = sha256.map(str::to_string);
        Box::pin(async move {
            tauri::async_runtime::spawn_blocking(move || {
                let _guard = kotone_stt::download::begin_download()?;
                kotone_stt::download::download_file(&url, &dest, sha256.as_deref(), &|_, _| {})
            })
            .await
            .map_err(|e| e.to_string())?
        })
    }
}

/// 设置保存了（运行时状态推送点，设置保存的最后一步也经过这里）：告诉每个插件，跟着设置启停的东西自己对一下。
pub fn settings_changed(app: &AppHandle) {
    for plugin in registry() {
        plugin.settings_changed(DesktopHost::for_plugin(app, plugin));
    }
}

/// 玩家让她停下：每个插件停掉正在替她做的事（例如游戏语音里还没说完的话）。
pub fn stop_all() {
    for plugin in registry() {
        plugin.stop();
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

/// 谁在调插件的工具。
#[derive(Debug, Clone, Copy)]
pub enum Caller<'a> {
    /// 她（她的工具清单里只有给她的，`visibility` 含 `model`）。
    Her,
    /// 外部 Agent 经工具服务 / MCP（清单里两种都列出来，带 `visibility`，都能调）。
    Service,
    /// 插件自己的页面（只能调自己插件的、给页面的工具）。
    Page(&'a str),
}

/// 调一个插件的工具。
/// 插件关着时拒绝；数据开关关着时由插件自己回答（一段对话里她的工具清单不变，开关可能中途关了）。
pub async fn call(
    app: &AppHandle,
    tool: &str,
    arguments: &Value,
    caller: Caller<'_>,
) -> Result<ToolResult, String> {
    let plugin = owner(tool).ok_or_else(|| format!("没有叫 {tool} 的插件工具"))?;
    let manifest = plugin.manifest();
    let spec = plugin
        .tools()
        .into_iter()
        .find(|t| t.name == tool)
        .ok_or_else(|| format!("没有叫 {tool} 的插件工具"))?;
    match caller {
        Caller::Page(page_plugin) if page_plugin != manifest.id => {
            return Err(format!("{page_plugin} 的页面不能调 {tool}"));
        }
        Caller::Page(_) if !spec.for_app() => return Err(format!("{tool} 不给页面用")),
        Caller::Her if !spec.for_model() => return Err(format!("{tool} 只给插件的页面用")),
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

/// `ui.open`（ADR 038 D5）：检查页面地址，再让界面打开它。不把 Kotone 窗口拉到前面（玩家可能在游戏里）。
pub fn open_page(app: &AppHandle, args: &Value) -> Result<Value, String> {
    let uri = args
        .get("page")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|u| !u.is_empty())
        .ok_or("要写 page（页面地址，如 ui://kotone/lol/match）")?;
    let params = args.get("params").cloned().unwrap_or(Value::Null);
    if !params.is_null() && !params.is_object() {
        return Err("params 要是对象".into());
    }
    let (title, plugin) = resolve_page(&settings(app), uri)?;
    let _ = app.emit(
        UI_OPEN_EVENT,
        json!({"uri": uri, "params": params, "title": title, "plugin": plugin}),
    );
    Ok(json!({"opened": true, "uri": uri, "title": title,
        "note": "在 Kotone 窗口里打开了（插件的页面在右侧栏）；玩家在游戏里时窗口不会自己跳出来"}))
}

/// 页面地址 → (标题, 插件 id)。设置页的插件 id 是 None。
fn resolve_page(
    settings: &kotone_core::settings::Settings,
    uri: &str,
) -> Result<(String, Option<String>), String> {
    if let Some(section) = uri.strip_prefix("ui://kotone/settings/") {
        return if SETTINGS_SECTIONS.contains(&section) {
            Ok((section.to_string(), None))
        } else {
            Err(format!(
                "没有设置页 {section}（有：{}）",
                SETTINGS_SECTIONS.join("、")
            ))
        };
    }
    let plugin = registry()
        .iter()
        .find(|p| p.manifest().owns_uri(uri))
        .ok_or_else(|| format!("没有这个页面：{uri}"))?;
    let m = plugin.manifest();
    let page = m
        .pages
        .iter()
        .find(|p| p.uri == uri)
        .ok_or_else(|| format!("「{}」插件没有页面 {uri}", m.name))?;
    if !enabled(settings, m) {
        return Err(format!("「{}」插件关着（设置 → 插件 里可以打开）", m.name));
    }
    if let Some(off) = page.needs.iter().find(|s| !source_enabled(settings, m, s)) {
        let name = m.source(off).map(|s| s.name.as_str()).unwrap_or(off);
        return Err(format!("「{}」插件的“{name}”关着，这个页面用不了", m.name));
    }
    Ok((page.title.clone(), Some(m.id.clone())))
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
    fn game_voice_is_a_plugin_and_keeps_its_tool_names() {
        let gv = find("game-voice").expect("游戏语音插件");
        assert!(!gv.manifest().enabled_by_default);
        let names: Vec<String> = gv.tools().into_iter().map(|t| t.name).collect();
        assert!(
            names.iter().all(|n| n.starts_with("voice_chat.")),
            "{names:?}"
        );
        assert_eq!(owner("voice_chat.say").unwrap().manifest().id, "game-voice");
    }

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
        // 放歌搬进了插件（ADR 038 第 4 步）
        assert_eq!(
            owner("music").map(|p| p.manifest().id.as_str()),
            Some("music")
        );
        assert!(owner("browser.open").is_none());
    }

    #[test]
    fn ui_open_checks_the_page() {
        let s = kotone_core::settings::Settings::default();
        assert_eq!(
            resolve_page(&s, "ui://kotone/settings/plugins").unwrap(),
            ("plugins".to_string(), None)
        );
        // 设置里的“浏览器”一节（ADR 038：浏览器留在本体，开关在设置里）
        assert_eq!(
            resolve_page(&s, "ui://kotone/settings/browser").unwrap(),
            ("browser".to_string(), None)
        );
        assert!(resolve_page(&s, "ui://kotone/settings/nope").is_err());
        assert_eq!(
            resolve_page(&s, "ui://kotone/lol/settings")
                .unwrap()
                .1
                .as_deref(),
            Some("lol")
        );
        assert!(resolve_page(&s, "ui://kotone/lol/nope").is_err());
        assert!(resolve_page(&s, "ui://kotone/nope/x").is_err());
        assert!(resolve_page(&s, "https://example.com").is_err());
        let mut off = kotone_core::settings::Settings::default();
        off.plugins.entry("lol".into()).or_default().enabled = Some(false);
        assert!(resolve_page(&off, "ui://kotone/lol/settings")
            .unwrap_err()
            .contains("关着"));
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
