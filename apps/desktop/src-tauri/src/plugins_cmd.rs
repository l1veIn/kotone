//! 插件的命令（ADR 038；接口约定见 `src/lib/plugins.ts`）：插件清单和开关、插件页面调本插件的工具、
//! 打开插件的 MCP App（ADR 039 D5）。
//! 开关本身走设置（`plugins.<id>.enabled` / `sources.<开关>`），这里不另开命令。

use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

/// 全部插件：清单、开着没有、数据开关、工具（名字、给谁、页面、现在能不能用）。
#[tauri::command]
pub fn plugins_list(app: AppHandle) -> Value {
    crate::plugins::list(&app)
}

/// 插件页面调本插件的工具（只给页面用的也能调）。结果：`{content, structuredContent?, ui?}`。
#[tauri::command]
pub async fn plugin_call(
    app: AppHandle,
    plugin: String,
    tool: String,
    arguments: Option<Value>,
) -> Result<Value, String> {
    let arguments = arguments.unwrap_or_else(|| Value::Object(Default::default()));
    let result = crate::plugins::call(
        &app,
        &tool,
        &arguments,
        crate::plugins::Caller::Page(&plugin),
    )
    .await?;
    serde_json::to_value(result).map_err(|e| e.to_string())
}

/// App 读自己插件的资源（MCP Apps 的 `resources/read`，桥转过来）：`contents` 的一项。`plugin` 是宿主按 iframe
/// 绑定的插件（不是页面说的），别的插件的资源不给。
#[tauri::command]
pub fn plugin_resource(app: AppHandle, plugin: String, uri: String) -> Result<Value, String> {
    if crate::plugin_apps::owner(&uri).map(|p| p.manifest().id.as_str()) != Some(plugin.as_str()) {
        return Err(format!("{plugin} 的页面不能读 {uri}"));
    }
    let settings = app
        .state::<crate::SharedState>()
        .settings
        .read()
        .unwrap()
        .clone();
    Ok(crate::plugin_apps::read(&settings, &uri)?.content())
}

/// 打开插件的一个 MCP App（`ui://kotone/<插件>/<页面>`，ADR 039 D5）：回环代理上沙盒代理页的地址（每个插件一个源，
/// 第一次时才起代理）和资源的 `_meta.ui`（`csp`、`prefersBorder`）。前端的 App 容器把 iframe 指到 `sandboxUrl`，
/// 照 MCP Apps 规范握手；App 调工具时宿主按这里的 `plugin` 绑定调用方，不信页面传来的。
/// 插件或页面要的数据开关关着、页面还不是 App 时报错。
#[tauri::command]
pub async fn plugin_app(app: AppHandle, uri: String) -> Result<Value, String> {
    let settings = app
        .state::<crate::SharedState>()
        .settings
        .read()
        .unwrap()
        .clone();
    let resource = crate::plugin_apps::read(&settings, &uri)?;
    let plugin = crate::plugin_apps::owner(&uri).ok_or_else(|| format!("没有资源 {uri}"))?;
    let page = uri.rsplit('/').next().unwrap_or_default();
    let source = crate::plugin_apps::PluginSource::new(&app, plugin);
    let proxy = app.state::<crate::app_proxy::Proxies>().get(source).await?;
    Ok(json!({
        "plugin": plugin.manifest().id,
        "uri": uri,
        "sandboxUrl": proxy.sandbox_url(page),
        "ui": resource.ui,
    }))
}
