//! 第三方插件的宿主那一半（MCP 方案第 2 期；插件本身在 `kotone-remote-plugin`）：
//!
//! - **插件表加锁、运行时加入**：内置的还是编译进来的静态表（`plugins::registry`），第三方的在这里，
//!   读 `KOTONE_HOME/mcp.json` 时对一遍（配置没变的留着、连接不断；变了的、删掉的断开）。
//!   `plugins::owner`、`offers`、`list`、`call` 两张表一起看。
//! - **给第三方的宿主更窄**（[`RemoteHost`]，ADR 038“不做”、ADR 039 D3）：只有开关、自己的设置（信任记在这里）、
//!   凭据库（`<插件 id>:<名字>`）和后台任务；替玩家说话、出声、覆盖层、截图识别、网页窗口、推送（ADR 043）、
//!   改设置都用 `Host` 的默认实现（一律拒绝）。
//! - **不进工具服务**（方案第七节第 2 条，隐私）：工具服务、`kotone.exe mcp` 只列内置插件的工具和资源
//!   （`plugins::service_specs`、`plugin_apps::list` 只看静态表），外部 Agent 调它们被拒（`plugins::call`）。
//! - id 不能和内置插件、本体工具的前缀撞（`<id>.<工具>` 要唯一）：撞了的、写法不对的那一项**不进插件表**
//!   （进了的话 `is_remote("lol")` 就成真，真的英雄联盟插件会拿到第三方那份窄的宿主；审查后改的，2026-11-25），
//!   另列成“有问题的配置”（[`status`]），插件页上说清原因。`mcp.json` 读不出来时也在那里说一句。
//! - 配置变了（指纹不同）或删掉了：“总是允许”的规则（`agent.permissions.rules` 里 `<id>.*`）跟着清掉，
//!   和信任一样不留给换上来的那份配置（[`forget_rules`]）。

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

/// `mcp.json` 里没加进来的项（名字和 Kotone 的撞了、不能当 id）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Problem {
    /// `mcp.json` 里的键。
    pub id: String,
    /// `taken`（和内置插件、本体的工具撞了，或者是 Kotone 自己用的名字）、`invalid`（不能当插件 id）。
    pub kind: &'static str,
    /// 后端的原话。
    pub error: String,
}

/// 上一次读 `mcp.json` 的结果：读不出来的原因（读不出来时插件表不动）、没加进来的项。
#[derive(Debug, Clone, Default)]
struct FileState {
    error: Option<String>,
    problems: Vec<Problem>,
}

fn file_state() -> &'static RwLock<FileState> {
    static STATE: OnceLock<RwLock<FileState>> = OnceLock::new();
    STATE.get_or_init(RwLock::default)
}

/// 插件页要的：`mcp.json` 在哪、读不出来的原因、没加进来的项（`mcp_servers_status`）。
pub fn status() -> Value {
    let state = file_state().read().unwrap().clone();
    json!({
        "path": mcp_path().display().to_string(),
        "fileError": state.error,
        "problems": state.problems,
    })
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

/// 内置插件的 id（编译进来的静态表）。
fn builtin(id: &str) -> bool {
    crate::plugins::registry()
        .iter()
        .any(|p| p.manifest().id == id)
}

/// 这个 id 是第三方插件。**先看内置表**：内置插件的 id 永远不算第三方（插件表本来就不收撞名的，这里再挡一道）。
pub fn is_remote(id: &str) -> bool {
    !builtin(id) && find(id).is_some()
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

/// 能加进插件表的一项：id 和读出来的配置（配置写错了也算，是一张说清哪里不对的卡片）。
type Entry = (String, Result<ServerSpec, String>);

/// `mcp.json` 的每一项：能加进来的和没加进来的（名字和 Kotone 的撞了、不能当 id）。
fn split_entries(file: &McpFile) -> (Vec<Entry>, Vec<Problem>) {
    let mut entries = Vec::new();
    let mut problems = Vec::new();
    for id in file.ids() {
        if let Err(error) = check_new_id(&id) {
            let kind = if kotone_plugin_api::valid_id(&id) {
                "taken"
            } else {
                "invalid"
            };
            problems.push(Problem { id, kind, error });
            continue;
        }
        let spec = file
            .entry(&id)
            .ok_or_else(|| "没有这一项".to_string())
            .and_then(ServerSpec::parse);
        entries.push((id, spec));
    }
    (entries, problems)
}

/// 读 `mcp.json`，对一遍插件表：配置没变的留着（连接不断）；变了的、删掉的断开；新的加进来并开始（开着、
/// 还不知道有哪些工具的会去连）。名字和 Kotone 撞了、不能当 id 的不进表（[`status`] 里说原因）。
/// 文件坏了不动现在的表（报错，[`status`] 里也有）。
pub fn reload(app: &AppHandle) -> Result<(), String> {
    let file = match McpFile::load(&mcp_path()) {
        Ok(file) => file,
        Err(error) => {
            file_state().write().unwrap().error = Some(error.clone());
            return Err(error);
        }
    };
    let (entries, problems) = split_entries(&file);
    for problem in &problems {
        kotone_core::log::log(&format!(
            "mcp.json 里的「{}」没加进来：{}",
            problem.id, problem.error
        ));
    }
    *file_state().write().unwrap() = FileState {
        error: None,
        problems,
    };
    let version = app.package_info().version.to_string();
    let old = plugins();
    let mut next: Vec<Arc<RemotePlugin>> = Vec::new();
    let mut started: Vec<Arc<RemotePlugin>> = Vec::new();
    for (id, spec) in entries {
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
    // 配置和上次连过的不一样了（这次运行里改的，或者 Kotone 关着时改的、删了又加了同名的）：
    // “总是允许”的规则不留给新的那份配置
    for plugin in &started {
        if let Some(before) = plugin.server().replaced_fingerprint() {
            let id = &plugin.manifest().id;
            kotone_core::log::log(&format!(
                "第三方插件 {id} 的配置变了（{before} → {}），清掉它的“总是允许”",
                plugin.server().fingerprint()
            ));
            forget_rules(app, id);
            let _ = std::fs::remove_file(data_dir(id).join(kotone_remote_plugin::CACHE_FILE));
        }
    }
    for plugin in started {
        let id = plugin.manifest().id.clone();
        kotone_core::log::log(&format!("第三方插件 {id} 登记了"));
        plugin.start(host(app, &id));
    }
    Ok(())
}

/// 权限里这个第三方插件的工具的规则（`<id>.<工具>`，“总是允许这个工具”记在这里）和绑热键时的确认（ADR 040 D9）
/// 全部清掉：删掉插件、配置变了的时候（信任记的是指纹，配置一变就失效；规则和确认也一样，不留给换上来的那份配置）。
/// 只对真的第三方插件用：id 和内置插件、本体工具的前缀不会撞（`check_new_id`），清不到别人的规则。
pub fn forget_rules(app: &AppHandle, id: &str) {
    use tauri::Manager;
    let prefix = format!("{id}.");
    let current = settings(app);
    let has_rule = current
        .agent
        .permissions
        .rules
        .keys()
        .any(|tool| tool.starts_with(&prefix));
    // 绑热键时的确认（ADR 040 D9）也一样作废：配置换了，确认的就不是现在这个服务器的工具了
    let has_confirmation = current.clone().forget_confirmations(&prefix);
    if !has_rule && !has_confirmation {
        return;
    }
    let updated = app
        .state::<crate::SharedState>()
        .settings_repository
        .update(|next| {
            next.agent
                .permissions
                .rules
                .retain(|tool, _| !tool.starts_with(&prefix));
            next.forget_confirmations(&prefix);
            Ok(())
        });
    match updated {
        Ok(next) => {
            // 这些热键从“能按”变成“要先确认”：重新算谁登记（它们随应用登记）
            if has_confirmation {
                if let Some(manager) = app.try_state::<crate::hotkey::HotkeyManager>() {
                    manager.apply_app_hotkeys(&next);
                }
            }
            crate::tool_config::notify_settings_changed(app)
        }
        Err(error) => {
            kotone_core::log::log(&format!("第三方插件 {id} 的“总是允许”没清掉：{error}"))
        }
    }
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
    // 一页工具列表（没有 MCP App 的第三方插件，点卡片看到的就是它）：服务器说的名字、说明（剥过字、限长）、
    // 标没标只读、给谁
    let known = server.known();
    out["toolList"] = Value::Array(
        known
            .tools
            .iter()
            .filter(|t| kotone_remote_plugin::tools::valid_tool_name(&t.name))
            .map(|t| {
                let (visibility, page) = kotone_remote_plugin::tools::ui_meta(t);
                json!({
                    "name": kotone_remote_plugin::tools::kotone_name(id, &t.name),
                    "title": t.title.as_deref().map(|s| kotone_remote_plugin::tools::short_text(s, 60)),
                    "description": t.description.as_deref().map(|s| kotone_remote_plugin::tools::short_text(s, 300)),
                    "readOnly": kotone_remote_plugin::tools::read_only(t),
                    "visibility": visibility,
                    "page": page,
                })
            })
            .collect(),
    );
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

/// 测试用：一个记着 `count` 个给她的工具（`t0`…）和一个只给页面的工具（它的页面是 `ui://<id>/app`）的
/// 第三方插件（连不上的地址，不会去连）。
#[cfg(test)]
pub(crate) fn test_plugin(id: &str, count: usize) -> Arc<RemotePlugin> {
    let tools: Vec<Value> = (0..count)
        .map(|i| json!({"name": format!("t{i}"), "inputSchema": {"type": "object"}}))
        .chain(std::iter::once(
            json!({"name": "page_only", "inputSchema": {},
            "_meta": {"ui": {"visibility": ["app"], "resourceUri": format!("ui://{id}/app")}}}),
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
            "saved",
        ] {
            assert!(check_new_id(taken).is_err(), "{taken}");
        }
        assert!(check_new_id("everything").is_ok());
        assert!(check_new_id("Everything").is_err());
    }

    /// 审查后改的（2026-11-25）：`mcp.json` 里和内置插件撞名（手抄 Claude Desktop 的配置，键名正好是 `lol` /
    /// `music`）、不能当 id 的项不进插件表，另列成“有问题的配置”；配置写错了但名字没问题的照旧是一张卡片。
    #[test]
    fn colliding_entries_stay_out_of_the_plugin_table() {
        let file = McpFile::parse(
            r#"{"mcpServers": {
                "lol": {"command": "npx", "args": ["-y", "lol-mcp"]},
                "music": {"url": "https://music.example.com/mcp"},
                "saved": {"command": "x"},
                "Bad_Name": {"command": "x"},
                "everything": {"command": "npx", "args": ["-y", "@modelcontextprotocol/server-everything"]},
                "old": {"type": "sse", "url": "https://x.example.com/sse"}
            }}"#,
        )
        .unwrap();
        let (entries, problems) = split_entries(&file);
        let ids: Vec<&str> = entries.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids, ["everything", "old"]);
        assert!(
            entries[0].1.is_ok() && entries[1].1.is_err(),
            "写错了的还是一张卡片"
        );
        let by_id: std::collections::BTreeMap<&str, &Problem> =
            problems.iter().map(|p| (p.id.as_str(), p)).collect();
        assert_eq!(by_id.len(), 4);
        for id in ["lol", "music", "saved"] {
            assert_eq!(by_id[id].kind, "taken", "{id}");
            assert!(by_id[id].error.contains("换一个"), "{}", by_id[id].error);
        }
        assert_eq!(by_id["Bad_Name"].kind, "invalid");
        // 内置插件的 id 永远不算第三方
        assert!(!is_remote("lol") && !is_remote("music"));
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
