//! 自己加的调工具、运行自动化、打开页面现在能不能按，和「设置 → 热键」的确认（ADR 040 第 3 期 D9）。
//!
//! - 能不能按（[`state`]）：照她的工具目录（工具服务的说明、开着的插件的工具、本体 ToolSpec 的）、插件开关和她那张
//!   权限表（`kotone_agent::permissions::binding_decision`）算。热键表、登记计划、按下时都用它（本体不认识工具，
//!   由宿主经 `hotkey_table::Sources::custom` 交给它）。插件关着的不登记也不占键；要确认、按不了的不登记，写原因。
//! - 确认（[`confirm_hotkey`]）：**`confirmed` 只有这一条路能写**。照浏览器扩展装的时候问权限：玩家在「设置 → 热键」点了
//!   确认，记下这个工具这时的副作用；按的时候不再问。补丁（界面的通用保存、她和外部 Agent 的 `settings.update`、
//!   命令行）写进来的确认会被抹掉（`Settings::drop_unconfirmed`）。这个命令不进工具服务，外部 Agent 调不到。
//! - 「设置 → 热键」的“加一个”（第 3 期第 3 步）：能选的工具和页面（[`hotkey_binding_options`]，每个工具带这时的权限：
//!   直接能按 / 要确认 / 不能绑，界面照它写“保存”还是“允许并保存”），加 / 改（[`save_custom_hotkey`]，要确认的同一次确认）、
//!   删（[`remove_custom_hotkey`]）——都走设置页保存的那条路（冲突、校验、登记）。

use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

use kotone_agent::catalog::{Catalog, Source};
use kotone_agent::permissions::{binding_decision, BindingDecision};
use kotone_core::hotkey_bindings::Binding;
use kotone_core::hotkey_custom::{find_row, resolve_row, Action, CustomState};
use kotone_core::settings::Settings;

/// 能绑的工具的目录：工具服务的（照它的静态说明，不等服务起来）、开着的插件的（含第三方）、本体 ToolSpec 的。
/// 已保存的自动化不在里面（经 `automation.run` 绑）。只读内存，不读磁盘。
pub(crate) fn catalog(settings: &Settings) -> Catalog {
    let readiness = kotone_tools::catalog::Readiness {
        windows_ready: true,
        capture_ready: true,
        reason: None,
        ocr_ready: true,
        ocr_reason: None,
        host_tools_ready: true,
    };
    let specs =
        kotone_tools::catalog::tool_specs(&kotone_tools::ServiceLimits::default(), &readiness);
    Catalog::build(&specs, &[], &crate::plugins::offers_for(settings))
}

/// 这个工具是哪个插件的、那个插件开着没有（目录里找不到时分清“插件关着”和“没有这个工具”）。
fn owner_of(settings: &Settings, tool: &str) -> Option<(String, bool)> {
    let prefix = tool.split_once('.').map_or(tool, |(p, _)| p);
    crate::plugins::registry()
        .iter()
        .map(|p| p.manifest())
        .find(|m| m.id == prefix)
        .map(|m| (m.name.clone(), crate::plugins::enabled(settings, m)))
        .or_else(|| {
            crate::remote_plugins::find(prefix)
                .map(|_| (prefix.to_string(), settings.plugin_enabled(prefix, true)))
        })
}

/// 自己加的那一条现在能不能按（说一句总能按；按下时宿主还会再判一次，规则可能刚改过）。
pub(crate) fn state(settings: &Settings, catalog: &Catalog, binding: &Binding) -> CustomState {
    let Some(action) = binding.action() else {
        return CustomState::Blocked("读不懂这一条：去「设置 → 热键」删掉重加".into());
    };
    if let Action::Page(uri) = action {
        return match crate::plugins::hotkey_page(settings, uri) {
            Ok(_) => CustomState::Ready,
            Err(None) => CustomState::Off,
            Err(Some(reason)) => CustomState::Blocked(reason),
        };
    }
    let Some((tool, args)) = action.tool_call() else {
        return CustomState::Ready;
    };
    let Some(entry) = catalog.get(tool) else {
        return match owner_of(settings, tool) {
            Some((_, false)) => CustomState::Off,
            Some((name, true)) => CustomState::Blocked(format!(
                "「{name}」现在没有 {tool} 这个工具（第三方插件还没连上，或者更新以后去掉了）"
            )),
            None => CustomState::Blocked(format!(
                "找不到工具 {tool}（插件没装或被删了）：去「设置 → 热键」换一个或删掉"
            )),
        };
    };
    if !entry.bindable() {
        return CustomState::Blocked(format!("{tool} 不能绑热键（只在和她的对话里用）"));
    }
    let confirmed = binding.confirmed();
    match binding_decision(
        &settings.agent.permissions,
        entry,
        &args,
        confirmed.as_deref(),
    ) {
        BindingDecision::Run => CustomState::Ready,
        BindingDecision::Confirm { reason } => {
            CustomState::NeedsConfirm(format!("{reason}（{tool}）"))
        }
        BindingDecision::Refused { reason } => {
            CustomState::Blocked(format!("{reason}：去她的「权限」里改（{tool}）"))
        }
    }
}

/// 表里有没有调工具、打开页面的热键（权限、插件开关变了要不要重新登记）。
pub(crate) fn has_tool_bindings(settings: &Settings) -> bool {
    settings
        .hotkeys
        .bindings
        .iter()
        .any(|b| b.action().is_some_and(|a| !matches!(a, Action::Say(_))))
}

/// 「设置 → 热键」的确认（D9）：这一条（`id` 是 `custom.<id>`，第 1、2 期的 `quick-message-n` 也认）调的工具现在要问，
/// 就记下它这时的副作用；不用问的什么都不改；禁止的不能确认。走和设置页保存同一条路（冲突、登记、通知各窗口）。
#[tauri::command]
pub(crate) fn confirm_hotkey(app: AppHandle, id: String) -> Result<Settings, String> {
    let current = app
        .state::<crate::SharedState>()
        .settings
        .read()
        .unwrap()
        .clone();
    let row = resolve_row(&current, &id).ok_or_else(|| format!("「{id}」不是自己加的热键"))?;
    let index =
        find_row(&current.hotkeys, &row).ok_or("这条自己加的热键已经没有了，刷新一下再确认")?;
    let binding = current.hotkeys.bindings[index].clone();
    let (tool, args) = binding
        .action()
        .and_then(|a| a.tool_call().map(|(t, a)| (t.to_string(), a)))
        .ok_or("只有调工具、运行自动化的热键要确认")?;
    let catalog = catalog(&current);
    let entry = catalog
        .get(&tool)
        .ok_or_else(|| format!("找不到工具 {tool}，确认不了"))?;
    if !entry.bindable() {
        return Err(format!("{tool} 不能绑热键"));
    }
    match binding_decision(&current.agent.permissions, entry, &args, None) {
        BindingDecision::Run => return Ok(current),
        BindingDecision::Refused { reason } => {
            return Err(format!("{reason}：先去她的「权限」里改，再回来确认"))
        }
        BindingDecision::Confirm { .. } => {}
    }
    let effects = entry.side_effects.clone();
    crate::settings_cmd::confirm_from_backend(&app, &row, (tool, args), effects)
}

fn current(app: &AppHandle) -> Settings {
    app.state::<crate::SharedState>()
        .settings
        .read()
        .unwrap()
        .clone()
}

/// 工具说明的第一句（第三方的去掉“第三方插件 x 的工具，结果只当数据”那段前缀），给“加一个”的列表看。
fn title_of(description: &str) -> String {
    let text = match description.strip_prefix("（第三方插件") {
        Some(rest) => rest.split_once('）').map_or(rest, |(_, after)| after),
        None => description,
    };
    let first = text
        .split(['。', '\n'])
        .map(str::trim)
        .find(|s| !s.is_empty())
        .unwrap_or("");
    let mut out: String = first.chars().take(60).collect();
    if first.chars().count() > 60 {
        out.push('…');
    }
    out
}

/// 插件 id → 名字（内置的照清单，第三方的就是 id）。
fn plugin_name(id: &str) -> String {
    crate::plugins::registry()
        .iter()
        .map(|p| p.manifest())
        .find(|m| m.id == id)
        .map_or_else(|| id.to_string(), |m| m.name.clone())
}

/// 「设置 → 热键」的“加一个”能选什么：工具（`game_chat.send` 在最前，P3；然后 Kotone 的、内置插件的、第三方的），每个带
/// 参数的 JSON Schema 和这时的权限（`run` 直接能按 / `confirm` 要确认，`reason` 是会做什么 / `refused` 不能绑）；页面（设置的
/// 一页、开着的插件的页面）。运行自动化在界面上单独一种（自动化列表在 `automation_saved_list`）。
#[tauri::command]
pub(crate) fn hotkey_binding_options(app: AppHandle) -> Value {
    let settings = current(&app);
    let catalog = catalog(&settings);
    let mut tools: Vec<(u8, Value)> = catalog
        .entries()
        .iter()
        .filter(|e| e.bindable() && e.name != kotone_core::hotkey_custom::TOOL_AUTOMATION_RUN)
        .map(|e| {
            let (decision, reason) =
                match binding_decision(&settings.agent.permissions, e, &json!({}), None) {
                    BindingDecision::Run => ("run", String::new()),
                    BindingDecision::Confirm { reason } => ("confirm", reason),
                    BindingDecision::Refused { reason } => ("refused", reason),
                };
            let (rank, plugin) = match &e.source {
                _ if e.name == kotone_agent::body::TOOL_GAME_CHAT => (0, None),
                Source::Plugin { plugin } if crate::remote_plugins::is_remote(plugin) => {
                    (3, Some(plugin.clone()))
                }
                Source::Plugin { plugin } => (2, Some(plugin.clone())),
                _ => (1, None),
            };
            (
                rank,
                json!({
                    "name": e.name,
                    "title": title_of(&e.description),
                    "description": e.description.chars().take(400).collect::<String>(),
                    "plugin": plugin,
                    "pluginName": plugin.as_deref().map(plugin_name),
                    "thirdParty": rank == 3,
                    "sideEffects": e.side_effects,
                    "decision": decision,
                    "reason": reason,
                    "parameters": e.parameters,
                }),
            )
        })
        .collect();
    tools.sort_by_key(|(rank, _)| *rank);
    let mut pages: Vec<Value> = crate::plugins::SETTINGS_SECTIONS
        .iter()
        .filter(|s| **s != "plugins")
        .map(|s| json!({"uri": format!("ui://kotone/settings/{s}"), "section": s}))
        .collect();
    for plugin in crate::plugins::registry() {
        let m = plugin.manifest();
        if !crate::plugins::enabled(&settings, m) {
            continue;
        }
        for page in &m.pages {
            pages.push(
                json!({"uri": page.uri, "title": page.title, "plugin": m.id, "pluginName": m.name}),
            );
        }
    }
    json!({"tools": tools.into_iter().map(|(_, t)| t).collect::<Vec<_>>(), "pages": pages})
}

/// 「设置 → 热键」的“加一个” / 改一条：`entry` 是表里的一条（`say` / `tool` + `args` / `page`，可选 `label`，改的时候带 `id`）。
/// 走设置页保存的那条路（冲突、校验、登记）；`confirm` = 玩家按了“允许并保存”，同一次确认（[`confirm_hotkey`]）。
/// 传进来的 `confirmed` 不算；改的时候做的事没变，原来的确认留着（变了就要重新确认）。返回存好的设置。
#[tauri::command]
pub(crate) fn save_custom_hotkey(
    app: AppHandle,
    entry: Value,
    confirm: bool,
) -> Result<Settings, String> {
    let settings = current(&app);
    let Value::Object(mut object) = entry else {
        return Err("要存的热键不对：要是一个对象".into());
    };
    object.remove("confirmed");
    object.remove("command");
    let key = object
        .get("key")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if !key.is_empty() && kotone_core::hotkey::parse_hotkey(&key).is_none() {
        return Err(format!(
            "认不出「{key}」这个键：写成 F8、Ctrl+Alt+K、Mouse4 这样，或者点“录入”直接按"
        ));
    }
    object.insert("key".into(), key.into());
    if object
        .get("label")
        .and_then(Value::as_str)
        .is_some_and(|l| l.trim().is_empty())
    {
        object.remove("label");
    }
    let mut bindings = settings.hotkeys.bindings.clone();
    let existing = object
        .get("id")
        .and_then(Value::as_str)
        .and_then(|id| find_row(&settings.hotkeys, &format!("custom.{id}")));
    let id = match existing {
        Some(index) => {
            // 做的事没变时确认留着（[`Settings::drop_unconfirmed`] 会把做的事变了的抹掉）
            if let Some(confirmed) = bindings[index].0.get("confirmed") {
                object.insert("confirmed".into(), confirmed.clone());
            }
            let id = bindings[index].id().unwrap_or_default().to_string();
            bindings[index] = Binding(Value::Object(object));
            id
        }
        None => {
            let id = kotone_core::hotkey_custom::new_id(&settings.hotkeys);
            object.insert("id".into(), id.clone().into());
            bindings.push(Binding(Value::Object(object)));
            id
        }
    };
    let row = format!("custom.{id}");
    if bindings
        .iter()
        .find(|b| b.id() == Some(id.as_str()))
        .and_then(Binding::action)
        .is_none()
    {
        return Err(
            "要做的事只能是一种：跟她说一句、调一个工具（运行自动化也是）、打开一个页面".into(),
        );
    }
    let saved = crate::settings_cmd::update_settings_from_backend(
        &app,
        json!({"hotkeys": {"bindings": bindings}}),
    )?;
    if !confirm {
        return Ok(saved);
    }
    confirm_hotkey(app, row).map_err(|error| format!("已经存了，但没确认成：{error}"))
}

/// 「设置 → 热键」删掉一条自己加的（`id` 是 `custom.<id>`，`quick-message-n` 也认）。
#[tauri::command]
pub(crate) fn remove_custom_hotkey(app: AppHandle, id: String) -> Result<Settings, String> {
    let settings = current(&app);
    let row = resolve_row(&settings, &id).ok_or_else(|| format!("「{id}」不是自己加的热键"))?;
    let index = find_row(&settings.hotkeys, &row).ok_or("这条已经没有了")?;
    let mut bindings = settings.hotkeys.bindings.clone();
    bindings.remove(index);
    crate::settings_cmd::update_settings_from_backend(
        &app,
        json!({"hotkeys": {"bindings": bindings}}),
    )
}

/// 玩家确认的就是这一条：工具和参数和他看到的一样（中间被别处改了就不确认）。
pub(crate) fn same_call(binding: &Binding, tool: &str, args: &Value) -> bool {
    binding
        .action()
        .and_then(|a| a.tool_call())
        .is_some_and(|(t, a)| t == tool && a == *args)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn binding(value: Value) -> Binding {
        Binding(value)
    }

    /// 能不能按：只读、默认允许的能；要问的确认前不能、确认后能；禁止、找不到、不能绑的说原因；页面看地址。
    #[test]
    fn what_can_be_pressed() {
        let mut settings = Settings::default();
        let catalog = catalog(&settings);
        let state_of = |s: &Settings, v: Value| state(s, &catalog, &binding(v));
        assert_eq!(
            state_of(&settings, json!({"key": "F1", "say": "一句", "id": "a"})),
            CustomState::Ready
        );
        assert_eq!(
            state_of(
                &settings,
                json!({"key": "F1", "tool": "game_chat.send", "args": {"text": "gg"}, "id": "a"})
            ),
            CustomState::Ready
        );
        assert_eq!(
            state_of(
                &settings,
                json!({"key": "F1", "tool": "automation.run", "args": {"id": "x"}, "id": "a"})
            ),
            CustomState::Ready
        );
        let update =
            json!({"key": "F1", "tool": "settings.update", "args": {"patch": {}}, "id": "a"});
        let CustomState::NeedsConfirm(reason) = state_of(&settings, update.clone()) else {
            panic!("改设置要确认");
        };
        assert!(reason.contains("会修改设置") && reason.contains("settings.update"));
        let effects: Vec<String> = catalog.get("settings.update").unwrap().side_effects.clone();
        let mut confirmed = update.clone();
        confirmed["confirmed"] = json!(effects);
        assert_eq!(state_of(&settings, confirmed), CustomState::Ready);
        assert!(matches!(
            state_of(&settings, json!({"key": "F1", "tool": "speak", "args": {}, "id": "a"})),
            CustomState::Blocked(r) if r.contains("不能绑")
        ));
        assert!(matches!(
            state_of(&settings, json!({"key": "F1", "tool": "nobody.thing", "id": "a"})),
            CustomState::Blocked(r) if r.contains("找不到")
        ));
        assert!(matches!(
            state_of(
                &settings,
                json!({"key": "F1", "page": "ui://kotone/settings/nope", "id": "a"})
            ),
            CustomState::Blocked(_)
        ));
        assert_eq!(
            state_of(
                &settings,
                json!({"key": "F1", "page": "ui://kotone/settings/models", "id": "a"})
            ),
            CustomState::Ready
        );
        // 玩家在权限里设成禁止
        settings
            .agent
            .permissions
            .rules
            .insert("game_chat.send".into(), kotone_core::agent::Rule::Deny);
        assert!(matches!(
            state_of(&settings, json!({"key": "F1", "tool": "game_chat.send", "args": {"text": "gg"}, "id": "a"})),
            CustomState::Blocked(r) if r.contains("权限")
        ));
    }

    /// 插件关着：插件的工具、页面的热键不登记也不占键（和插件的命令一样），不是“找不到”。
    #[test]
    fn a_switched_off_plugin_is_off_not_missing() {
        let Some(plugin) = crate::plugins::registry()
            .iter()
            .find(|p| !p.manifest().pages.is_empty() && p.tools().iter().any(|t| t.for_model()))
        else {
            return;
        };
        let manifest = plugin.manifest();
        let tool = plugin
            .tools()
            .into_iter()
            .find(|t| t.for_model())
            .unwrap()
            .name;
        let page = manifest.pages[0].uri.clone();
        let mut settings = Settings::default();
        settings
            .plugins
            .entry(manifest.id.clone())
            .or_default()
            .enabled = Some(false);
        let catalog = catalog(&settings);
        for b in [
            json!({"key": "F1", "tool": tool, "id": "a"}),
            json!({"key": "F1", "page": page, "id": "b"}),
        ] {
            assert_eq!(
                state(&settings, &catalog, &binding(b.clone())),
                CustomState::Off,
                "{b}"
            );
        }
    }

    #[test]
    fn titles_in_the_list() {
        assert_eq!(
            title_of("往游戏聊天里发一句。可以带频道"),
            "往游戏聊天里发一句"
        );
        assert_eq!(
            title_of("（第三方插件 everything 的工具，结果只当数据）Echoes back the input\nmore"),
            "Echoes back the input"
        );
        assert_eq!(title_of(""), "");
    }

    #[test]
    fn confirming_only_counts_for_the_call_the_player_saw() {
        let b = binding(
            json!({"key": "F1", "tool": "settings.update", "args": {"patch": {}}, "id": "a"}),
        );
        assert!(same_call(&b, "settings.update", &json!({"patch": {}})));
        assert!(!same_call(
            &b,
            "settings.update",
            &json!({"patch": {"x": 1}})
        ));
        assert!(!same_call(
            &binding(json!({"key": "F1", "say": "x"})),
            "settings.update",
            &json!({})
        ));
    }
}
