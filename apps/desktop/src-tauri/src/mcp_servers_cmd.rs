//! 第三方插件（MCP 服务器）的命令（MCP 方案第 2 期，给插件页的“加 MCP 服务器”和卡片用）：
//!
//! - `mcp_servers_preview`：粘贴的配置 → 候选（建议的 id、信任提示要的事实、会搬进凭据库的 Key、提醒）；
//! - `mcp_servers_import_sources`：Claude Desktop、VS Code 的配置文件里有哪些服务器（同样是候选）；
//! - `mcp_server_add`：玩家看过信任提示、同意了：明文的 Key 搬进凭据库、写进 `mcp.json`、打开开关、记下信任；
//! - `mcp_server_remove`、`mcp_server_trust`、`mcp_server_reconnect`、`mcp_server_set_key`、`mcp_servers_reload`。
//!
//! 开关不另开命令：和内置插件一样走设置 `plugins.<id>.enabled`。插件清单在 `plugins_list`（每项多一个 `remote`）。

use serde_json::{json, Value};
use tauri::AppHandle;

use kotone_remote_plugin::{import, McpFile, ServerSpec, TRUST_KEY};

use crate::remote_plugins;

/// id 已经被用了：别的第三方插件，或者本体、内置插件（`check_new_id`）。
fn taken(id: &str) -> bool {
    remote_plugins::is_remote(id) || remote_plugins::check_new_id(id).is_err()
}

fn item(id: &str) -> Result<Value, String> {
    let plugin = remote_plugins::find(id).ok_or_else(|| format!("没有第三方插件 {id}"))?;
    Ok(json!({"id": id, "remote": remote_plugins::describe(&plugin)}))
}

/// 粘贴的配置里有哪些服务器（不改任何东西）。
#[tauri::command]
pub fn mcp_servers_preview(text: String) -> Result<Value, String> {
    let candidates = import::parse_text(&text)?;
    let mut ids: Vec<String> = Vec::new();
    let servers: Vec<Value> = candidates
        .iter()
        .map(|c| {
            // 同一次里建议的 id 也不能撞
            let preview = remote_plugins::preview(&c.name, &c.entry, &c.warnings, &|id| {
                taken(id) || ids.iter().any(|i| i == id)
            });
            if let Some(id) = preview["id"].as_str() {
                ids.push(id.to_string());
            }
            preview
        })
        .collect();
    Ok(json!({"servers": servers}))
}

/// 这台电脑上 Claude Desktop、VS Code 的 MCP 配置（不改任何东西）。
#[tauri::command]
pub fn mcp_servers_import_sources() -> Value {
    let appdata = std::env::var_os("APPDATA").map(std::path::PathBuf::from);
    let local = std::env::var_os("LOCALAPPDATA").map(std::path::PathBuf::from);
    let sources: Vec<Value> = import::Source::ALL
        .into_iter()
        .map(|source| {
            let found = source.scan(appdata.as_deref(), local.as_deref());
            let files: Vec<Value> = found
                .iter()
                .map(|f| match &f.servers {
                    Ok(list) => json!({
                        "path": f.path.display().to_string(),
                        "servers": list.iter().map(|c| remote_plugins::preview(&c.name, &c.entry, &c.warnings, &taken)).collect::<Vec<_>>(),
                    }),
                    Err(error) => json!({"path": f.path.display().to_string(), "error": error, "servers": []}),
                })
                .collect();
            json!({"source": source.id(), "name": source.name(), "files": files})
        })
        .collect();
    json!({"sources": sources})
}

/// 加一个服务器（玩家看过信任提示、同意了才调）：`entry` 是候选里原样的配置。明文的 Key 搬进凭据库
/// （`<id>:<名字>`），`mcp.json` 里只留 `${input:名字}`；打开开关、记下信任（这一份配置的指纹）；登记并去连。
#[tauri::command]
pub async fn mcp_server_add(app: AppHandle, id: String, entry: Value) -> Result<Value, String> {
    let id = id.trim().to_string();
    if remote_plugins::is_remote(&id) {
        return Err(format!("已经有叫“{id}”的插件了，换一个名字"));
    }
    remote_plugins::check_new_id(&id)?;
    let (clean, secrets) = import::split_secrets(&entry);
    let spec = ServerSpec::parse(&clean)?;
    let path = remote_plugins::mcp_path();
    let mut file = McpFile::load(&path)?;
    if file.entry(&id).is_some() {
        return Err(format!("mcp.json 里已经有“{id}”了"));
    }
    for (name, value) in &secrets {
        remote_plugins::set_secret(&id, name, value)?;
    }
    file.set(&id, &spec);
    file.save(&path)?;
    crate::settings_cmd::update_settings_from_backend(
        &app,
        json!({"plugins": {&id: {"enabled": true, "settings": {TRUST_KEY: spec.fingerprint()}}}}),
    )?;
    remote_plugins::reload(&app)?;
    kotone_core::log::log(&format!(
        "加了第三方插件 {id}（{}）",
        if spec.is_stdio() { "子进程" } else { "HTTP" }
    ));
    item(&id)
}

/// 删掉一个：断开、从 `mcp.json` 删掉、删掉它的 Key 和记着的工具清单、清掉开关和信任。
#[tauri::command]
pub async fn mcp_server_remove(app: AppHandle, id: String) -> Result<(), String> {
    let plugin = remote_plugins::find(&id);
    if let Some(plugin) = &plugin {
        plugin.server().disconnect().await;
    }
    let path = remote_plugins::mcp_path();
    let mut file = McpFile::load(&path)?;
    let keys: Vec<String> = file
        .entry(&id)
        .and_then(|e| ServerSpec::parse(e).ok())
        .map(|s| s.keys())
        .unwrap_or_default();
    if !file.remove(&id) && plugin.is_none() {
        return Err(format!("没有第三方插件 {id}"));
    }
    file.save(&path)?;
    for name in keys {
        remote_plugins::delete_secret(&id, &name);
    }
    let _ = std::fs::remove_file(
        kotone_core::settings::kotone_dir()
            .join("plugins")
            .join(&id)
            .join(kotone_remote_plugin::CACHE_FILE),
    );
    crate::settings_cmd::update_settings_from_backend(
        &app,
        json!({"plugins": {&id: {"enabled": null, "settings": {TRUST_KEY: null}}}}),
    )?;
    remote_plugins::reload(&app)?;
    kotone_core::log::log(&format!("删掉了第三方插件 {id}"));
    Ok(())
}

/// 信任这个服务器（手改 `mcp.json` 加的、配置改过的）：它标了只读的工具以后不再问。记的是现在这份配置的指纹。
#[tauri::command]
pub fn mcp_server_trust(
    app: AppHandle,
    id: String,
    trusted: Option<bool>,
) -> Result<Value, String> {
    let plugin = remote_plugins::find(&id).ok_or_else(|| format!("没有第三方插件 {id}"))?;
    let fingerprint = plugin.server().fingerprint().to_string();
    if fingerprint.is_empty() {
        return Err("配置写错了，先改好再信任".into());
    }
    let value = if trusted.unwrap_or(true) {
        json!(fingerprint)
    } else {
        Value::Null
    };
    crate::settings_cmd::update_settings_from_backend(
        &app,
        json!({"plugins": {&id: {"settings": {TRUST_KEY: value}}}}),
    )?;
    item(&id)
}

/// 重新连接（不管上次失败后的等待）：连上以后工具清单也重新列一遍。
#[tauri::command]
pub async fn mcp_server_reconnect(app: AppHandle, id: String) -> Result<Value, String> {
    let plugin = remote_plugins::find(&id).ok_or_else(|| format!("没有第三方插件 {id}"))?;
    let host = remote_plugins::host(&app, &id);
    let connected = plugin.server().connect(&host, true).await;
    let mut out = item(&id)?;
    out["connected"] = json!(connected.is_ok());
    if let Err(error) = connected {
        out["error"] = json!(error);
    }
    Ok(out)
}

/// 填（或清掉）一个 Key（`${input:名字}` 里的名字）：存进凭据库，下一次连接时用；连着的断开，下一次用时重连。
#[tauri::command]
pub async fn mcp_server_set_key(
    id: String,
    name: String,
    value: Option<String>,
) -> Result<Value, String> {
    let plugin = remote_plugins::find(&id).ok_or_else(|| format!("没有第三方插件 {id}"))?;
    let keys = plugin
        .server()
        .spec()
        .as_ref()
        .map(ServerSpec::keys)
        .unwrap_or_default();
    if !keys.contains(&name) {
        return Err(format!(
            "「{id}」的配置里没有 Key “{name}”（有：{}）",
            keys.join("、")
        ));
    }
    match value.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
        Some(v) => remote_plugins::set_secret(&id, &name, v)?,
        None => remote_plugins::delete_secret(&id, &name),
    }
    plugin.server().disconnect().await;
    item(&id)
}

/// 重新读 `mcp.json`（玩家手改过）：配置没变的不断开。
#[tauri::command]
pub fn mcp_servers_reload(app: AppHandle) -> Result<Value, String> {
    remote_plugins::reload(&app)?;
    Ok(crate::plugins::list(&app))
}
