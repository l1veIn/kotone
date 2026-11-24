//! 第三方插件的宿主那一半（MCP 方案第 2 期；插件本身在 `kotone-remote-plugin`）：
//!
//! - **插件表加锁、运行时加入**：内置的还是编译进来的静态表（`plugins::registry`），第三方的在这里，
//!   读 `KOTONE_HOME/mcp.json` 时对一遍（配置没变的留着、连接不断；变了的、删掉的断开）。
//!   `plugins::owner`、`offers`、`list`、`call` 两张表一起看。
//! - **给第三方的宿主更窄**（[`RemoteHost`]，ADR 038“不做”、ADR 039 D3）：只有开关、自己的设置（信任记在这里）、
//!   凭据库（`<插件 id>:<名字>`）和后台任务；替玩家说话、出声、覆盖层、截图识别、网页窗口、自动卡片、
//!   改设置都用 `Host` 的默认实现（一律拒绝）。
//! - **不进工具服务**（方案第七节第 2 条，隐私）：工具服务、`kotone.exe mcp` 只列内置插件的工具和资源
//!   （`plugins::service_specs`、`plugin_apps::list` 只看静态表），外部 Agent 调它们被拒（`plugins::call`）。
//! - id 不能和内置插件、本体工具的前缀撞（`<id>.<工具>` 要唯一）：撞了的那一项当成配置写错了的卡片。

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock, RwLock};

use kotone_core::games::Observation;
use kotone_plugin_api::{BoxFuture, Host, Offer, Plugin};
use kotone_remote_plugin::{McpFile, RemotePlugin, ServerSpec};
use serde_json::{json, Map, Value};
use tauri::AppHandle;

fn table() -> &'static RwLock<Vec<Arc<RemotePlugin>>> {
    static TABLE: OnceLock<RwLock<Vec<Arc<RemotePlugin>>>> = OnceLock::new();
    TABLE.get_or_init(RwLock::default)
}

/// 现在的第三方插件（`mcp.json` 里的顺序）。
pub fn plugins() -> Vec<Arc<RemotePlugin>> {
    table().read().unwrap().clone()
}

/// 同上，当成一般的插件。
pub fn as_plugins() -> Vec<Arc<dyn Plugin>> {
    plugins()
        .into_iter()
        .map(|p| p as Arc<dyn Plugin>)
        .collect()
}

pub fn find(id: &str) -> Option<Arc<RemotePlugin>> {
    plugins().into_iter().find(|p| p.manifest().id == id)
}

pub fn is_remote(id: &str) -> bool {
    find(id).is_some()
}

/// `KOTONE_HOME/mcp.json`。
pub fn mcp_path() -> PathBuf {
    McpFile::path(&kotone_core::settings::kotone_dir())
}

fn data_dir(id: &str) -> PathBuf {
    kotone_core::settings::kotone_dir().join("plugins").join(id)
}

/// 本体和内置插件已经用了的名字：内置插件的 id、全部工具名的第一段（`settings`、`game`、`lol`、`voice_chat`……）。
fn builtin_names() -> &'static HashSet<String> {
    static NAMES: OnceLock<HashSet<String>> = OnceLock::new();
    NAMES.get_or_init(|| {
        let mut names = HashSet::new();
        let mut add = |tool: &str| {
            names.insert(tool.split('.').next().unwrap_or(tool).to_string());
        };
        for name in kotone_tools::catalog::implemented_tools() {
            add(name);
        }
        for entry in kotone_agent::catalog::Catalog::build(&[], &[], &[]).entries() {
            add(&entry.name);
        }
        for plugin in crate::plugins::registry() {
            add(&plugin.manifest().id);
            for tool in plugin.tools() {
                add(&tool.name);
            }
        }
        names
    })
}

/// 这个 id 能不能给第三方插件用（写法、保留名、和本体 / 内置插件撞不撞）。不看已有的第三方插件。
pub fn check_new_id(id: &str) -> Result<(), String> {
    kotone_remote_plugin::check_id(id)?;
    if builtin_names().contains(id) {
        return Err(format!(
            "名字“{id}”和 Kotone 里已有的插件或工具撞了，换一个"
        ));
    }
    Ok(())
}

/// 第三方插件拿到的那份宿主能力（更窄，见模块说明）。
struct RemoteHost {
    app: AppHandle,
    id: String,
}

fn settings(app: &AppHandle) -> kotone_core::settings::Settings {
    use tauri::Manager;
    app.state::<crate::SharedState>()
        .settings
        .read()
        .unwrap()
        .clone()
}

/// 凭据库里的名字：`<插件 id>:<名字>`（和内置插件一样）。
fn secret_id(id: &str, name: &str) -> String {
    format!("{id}:{name}")
}

pub fn secret(id: &str, name: &str) -> Option<String> {
    use kotone_core::connection::SecretStore;
    kotone_ai::secrets::KeyringSecretStore
        .get(&secret_id(id, name))
        .ok()
        .flatten()
}

pub fn set_secret(id: &str, name: &str, value: &str) -> Result<(), String> {
    use kotone_core::connection::SecretStore;
    kotone_ai::secrets::KeyringSecretStore.set(&secret_id(id, name), value)
}

pub fn delete_secret(id: &str, name: &str) {
    use kotone_core::connection::SecretStore;
    let _ = kotone_ai::secrets::KeyringSecretStore.delete(&secret_id(id, name));
}

impl Host for RemoteHost {
    fn plugin(&self) -> &str {
        &self.id
    }

    fn enabled(&self) -> bool {
        settings(&self.app).plugin_enabled(&self.id, true)
    }

    /// 第三方插件没有数据开关。
    fn source_enabled(&self, _source: &str) -> bool {
        false
    }

    fn app_version(&self) -> String {
        self.app.package_info().version.to_string()
    }

    fn settings(&self) -> Value {
        settings(&self.app)
            .plugins
            .get(&self.id)
            .map(|p| Value::Object(p.settings.clone()))
            .unwrap_or_else(|| json!({}))
    }

    fn data_dir(&self) -> PathBuf {
        data_dir(&self.id)
    }

    fn spawn(&self, task: BoxFuture<'static, ()>) {
        tauri::async_runtime::spawn(task);
    }

    /// 不给第三方看别的进程。
    fn process_ids(&self, _exe: &str) -> Vec<u32> {
        Vec::new()
    }

    fn process_command_line(&self, _pid: u32) -> Option<String> {
        None
    }

    /// 不给第三方往游戏中心报事（以后要给也是单向门，先不开）。
    fn game_observe(&self, _game: &str, _observation: Observation) {}

    fn game_claim(&self, _game: &str, _claimed: bool) {}

    fn game_event(
        &self,
        _game: &str,
        _kind: &str,
        _source: &str,
        _data: Map<String, Value>,
        _game_time: Option<f64>,
    ) {
    }

    fn secret(&self, name: &str) -> Option<String> {
        secret(&self.id, name)
    }

    fn set_secret(&self, name: &str, value: &str) -> Result<(), String> {
        set_secret(&self.id, name, value)
    }

    fn delete_secret(&self, name: &str) {
        delete_secret(&self.id, name)
    }
}

/// 第三方插件的宿主。
pub fn host(app: &AppHandle, id: &str) -> Arc<dyn Host> {
    Arc::new(RemoteHost {
        app: app.clone(),
        id: id.to_string(),
    })
}

/// 读 `mcp.json`，对一遍插件表：配置没变的留着（连接不断）；变了的、删掉的断开；新的加进来并开始（开着、
/// 还不知道有哪些工具的会去连）。文件坏了不动现在的表（报错）。
pub fn reload(app: &AppHandle) -> Result<(), String> {
    let file = McpFile::load(&mcp_path())?;
    let version = app.package_info().version.to_string();
    let old = plugins();
    let mut next: Vec<Arc<RemotePlugin>> = Vec::new();
    let mut started: Vec<Arc<RemotePlugin>> = Vec::new();
    for (id, spec) in file.servers() {
        let spec = spec.and_then(|s| check_new_id(&id).map(|()| s));
        let same = old.iter().find(|p| {
            p.manifest().id == id
                && match (p.server().spec(), &spec) {
                    (Ok(a), Ok(b)) => a == b,
                    (Err(a), Err(b)) => a == b,
                    _ => false,
                }
        });
        match same {
            Some(plugin) => next.push(plugin.clone()),
            None => {
                let plugin = Arc::new(RemotePlugin::new(&id, spec, Some(data_dir(&id)), &version));
                started.push(plugin.clone());
                next.push(plugin);
            }
        }
    }
    let gone: Vec<Arc<RemotePlugin>> = old
        .into_iter()
        .filter(|p| !next.iter().any(|n| Arc::ptr_eq(n, p)))
        .collect();
    *table().write().unwrap() = next;
    for plugin in gone {
        let server = plugin.server().clone();
        tauri::async_runtime::spawn(async move { server.disconnect().await });
    }
    for plugin in started {
        let id = plugin.manifest().id.clone();
        kotone_core::log::log(&format!("第三方插件 {id} 登记了"));
        plugin.start(host(app, &id));
    }
    Ok(())
}

/// 启动时：读 `mcp.json`、登记第三方插件。
pub fn init(app: &AppHandle) {
    if let Err(error) = reload(app) {
        kotone_core::log::log(&format!("第三方插件没登记上：{error}"));
    }
}

/// 设置保存了：每个第三方插件对一下开关和信任。
pub fn settings_changed(app: &AppHandle) {
    for plugin in plugins() {
        let id = plugin.manifest().id.clone();
        plugin.settings_changed(host(app, &id));
    }
}

/// 给她的（开着的第三方插件）：工具全部加起来不超过上限时主会话也拿（第 1.5 期：慢了自动转后台），
/// 超过了都只给后台、后台按关键词找（`tools.find`）。只给页面用的不进她的清单。
pub fn offers(
    settings: &kotone_core::settings::Settings,
    plugins: &[Arc<dyn Plugin>],
) -> Vec<Offer> {
    let mut offers: Vec<Offer> = plugins
        .iter()
        .filter(|p| settings.plugin_enabled(&p.manifest().id, true))
        .map(|p| {
            let m = p.manifest();
            Offer {
                plugin: m.id.clone(),
                name: m.name.clone(),
                text: m.agent.clone(),
                tools: p.tools().into_iter().filter(|t| t.for_model()).collect(),
            }
        })
        .collect();
    let count: usize = offers.iter().map(|o| o.tools.len()).sum();
    let main = kotone_remote_plugin::fits_main(count);
    for tool in offers.iter_mut().flat_map(|o| o.tools.iter_mut()) {
        tool.instant = main;
    }
    offers
}

/// 插件卡片要的（`plugins_list` 每项的 `remote`）：怎么连、要哪些 Key（填了没有）、信任没有、连接状态、
/// 服务器说的自己（剥掉看不见的字符）、它的 MCP App 页面。
pub fn describe(plugin: &RemotePlugin) -> Value {
    let id = &plugin.manifest().id;
    let server = plugin.server();
    let mut out = server.describe();
    match server.spec() {
        Ok(spec) => {
            out["config"] = spec.trust_facts();
            out["keys"] = Value::Array(
                spec.keys()
                    .into_iter()
                    .map(|name| json!({"name": name, "filled": secret(id, &name).is_some()}))
                    .collect(),
            );
        }
        Err(error) => {
            out["configError"] = json!(error);
            out["keys"] = json!([]);
        }
    }
    out["fingerprint"] = json!(server.fingerprint());
    out
}

/// 给界面看的一个候选（粘贴的、导入的）：建议的 id、信任提示要的事实、会搬进凭据库的 Key 的名字、提醒。
pub fn preview(
    name: &str,
    entry: &Value,
    warnings: &[String],
    taken: &dyn Fn(&str) -> bool,
) -> Value {
    let name = if name.trim().is_empty() {
        kotone_remote_plugin::import::suggest_name(entry)
    } else {
        name.to_string()
    };
    let id = kotone_remote_plugin::import::suggest_id(&name, &|id| {
        taken(id) || check_new_id(id).is_err()
    });
    let (clean, secrets) = kotone_remote_plugin::import::split_secrets(entry);
    let spec = ServerSpec::parse(&clean);
    let mut out = json!({
        "name": name,
        "id": id,
        "entry": entry,
        "warnings": warnings,
        "movedKeys": secrets.iter().map(|(k, _)| k).collect::<Vec<_>>(),
    });
    match spec {
        Ok(spec) => {
            out["facts"] = spec.trust_facts();
            // 已经加过同一份配置（不管叫什么名字）
            out["alreadyAdded"] = json!(plugins()
                .iter()
                .find(|p| p.server().spec().as_ref().is_ok_and(|s| *s == spec))
                .map(|p| p.manifest().id.clone()));
        }
        Err(error) => out["error"] = json!(error),
    }
    out
}

/// 测试用：一个记着 `count` 个给她的工具（`t0`…）和一个只给页面的工具的第三方插件（连不上的地址，不会去连）。
#[cfg(test)]
pub(crate) fn test_plugin(id: &str, count: usize) -> Arc<RemotePlugin> {
    let tools: Vec<Value> = (0..count)
        .map(|i| json!({"name": format!("t{i}"), "inputSchema": {"type": "object"}}))
        .chain(std::iter::once(
            json!({"name": "page_only", "inputSchema": {},
            "_meta": {"ui": {"visibility": ["app"]}}}),
        ))
        .collect();
    let spec = ServerSpec::parse(&json!({"url": "http://127.0.0.1:9/mcp"})).unwrap();
    let dir =
        std::env::temp_dir().join(format!("kotone-remote-offers-{id}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let known = kotone_remote_plugin::Known {
        fingerprint: spec.fingerprint(),
        tools: tools
            .into_iter()
            .map(|t| serde_json::from_value(t).unwrap())
            .collect(),
        ..Default::default()
    };
    std::fs::write(
        dir.join(kotone_remote_plugin::CACHE_FILE),
        serde_json::to_string(&known).unwrap(),
    )
    .unwrap();
    let plugin = Arc::new(RemotePlugin::new(id, Ok(spec), Some(dir.clone()), "test"));
    let _ = std::fs::remove_dir_all(&dir);
    plugin
}

#[cfg(test)]
pub(crate) fn insert_for_test(plugin: Arc<RemotePlugin>) {
    table().write().unwrap().push(plugin);
}

#[cfg(test)]
pub(crate) fn remove_for_test(id: &str) {
    table().write().unwrap().retain(|p| p.manifest().id != id);
}

#[cfg(test)]
mod tests {
    use super::*;
    use kotone_remote_plugin::tools;

    fn remote(id: &str, count: usize) -> Arc<dyn Plugin> {
        test_plugin(id, count)
    }

    /// 她的清单：只给页面的不进；全部第三方工具不超过上限时主会话也拿，超过了都只给后台；关着的不给。
    #[test]
    fn offers_follow_the_cap_and_the_switch() {
        let mut settings = kotone_core::settings::Settings::default();
        let few = [remote("aa", 5), remote("bb", 10)];
        let offers = offers(&settings, &few);
        assert_eq!(offers.len(), 2);
        assert!(offers
            .iter()
            .all(|o| o.tools.iter().all(|t| t.for_model() && t.instant)));
        assert_eq!(offers[0].tools.len(), 5, "只给页面的不进她的清单");
        let many = [remote("aa", 5), remote("bb", tools::MAX_MAIN_TOOLS)];
        let offers_many = super::offers(&settings, &many);
        assert!(
            offers_many
                .iter()
                .all(|o| o.tools.iter().all(|t| !t.instant)),
            "超过上限都只给后台"
        );
        settings.plugins.entry("bb".into()).or_default().enabled = Some(false);
        let offers_off = super::offers(&settings, &many);
        assert_eq!(offers_off.len(), 1);
        assert!(
            offers_off[0].tools.iter().all(|t| t.instant),
            "关掉的不算数"
        );
    }

    #[test]
    fn ids_cannot_take_kotone_names() {
        for taken in [
            "lol",
            "music",
            "game",
            "settings",
            "browser",
            "voice-input",
            "reply",
            "memory",
            "web",
        ] {
            assert!(check_new_id(taken).is_err(), "{taken}");
        }
        assert!(check_new_id("everything").is_ok());
        assert!(check_new_id("Everything").is_err());
    }

    #[test]
    fn previews_suggest_ids_and_move_keys() {
        let entry = json!({"command": "npx", "args": ["-y", "@modelcontextprotocol/server-everything"],
                           "env": {"API_TOKEN": "t-123"}});
        let p = preview("", &entry, &[], &|id| id == "everything");
        assert_eq!(p["name"], "everything");
        assert_eq!(p["id"], "everything-2");
        assert_eq!(p["movedKeys"], json!(["API_TOKEN"]));
        assert_eq!(
            p["facts"]["package"],
            "@modelcontextprotocol/server-everything"
        );
        assert_eq!(p["facts"]["keys"], json!(["API_TOKEN"]));
        assert!(
            !p["facts"].to_string().contains("t-123"),
            "信任提示里不出现 Key"
        );
        let lol = preview(
            "LoL",
            &json!({"url": "https://x.example.com/mcp"}),
            &[],
            &|_| false,
        );
        assert_eq!(lol["id"], "lol-2", "和内置插件撞名");
        let bad = preview(
            "x",
            &json!({"type": "sse", "url": "https://x/sse"}),
            &[],
            &|_| false,
        );
        assert!(bad["error"].as_str().unwrap().contains("SSE"));
    }
}
