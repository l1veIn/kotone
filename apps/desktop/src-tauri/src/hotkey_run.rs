//! 按下自己加的热键（ADR 040 第 3 期，D5、D9）：热键表里不绑命令、写着“要做什么”的条目（[`kotone_core::hotkey_custom`]）。
//!
//! - 跟她说一句（快捷指令）：交给她并接通通话（[`crate::companion::say`]）。要语音识别：随识别登记，识别没在跑时
//!   只看不吞、按了气泡提示（第 2 期主会话定的第 2 条），所以经 `dispatch_runtime_hotkey`（运行时的入场票）到这里。
//! - 运行自动化、调工具：不用听，随应用登记，识别没在跑也照样按（P4），在 `route_named_hotkey` 里直接到这里。按的时候
//!   **不问**（绑的时候问过了，D9），但照现在的权限、插件开关再判一次（[`crate::hotkey_bind::state`]，规则可能刚改过）。
//!   运行自动化跑那个已保存的自动化（发起方 `hotkey`）；工具服务的经进程内的工具服务，插件的经 `Caller::Hotkey`（第三方的
//!   也能调），`game_chat.send` 照“替玩家发言”的前台规矩（前台要是游戏，`Speaker::Automation`）。调工具的成了弹幕一行
//!   工具回的结果、不出声（P2）；运行自动化的成了不出声也不弹（2026-11-27 玩家：播报、弹幕、发到聊天都是程序里的函数，
//!   要不要通知由程序自己决定），只记下结果。没成都照快捷指令以前的做法说一句原因（原则 21：程序根本没跑起来，它没法自己说）。
//!   同一条上一次还没做完时再按，这次不做（不排队、不补发，ADR 032）。
//! - 打开页面：打开 Kotone 窗口到这一页，窗口拿到前面（玩家自己按的）；游戏独占全屏时不拿、放在后面闪任务栏，用语音
//!   说一句（P1，[`crate::attention::show_for_player`]）。第三方插件的页面打开左侧栏「插件」里那个插件的页面。
//!
//! - 插件登记的调工具的命令（英雄联盟的一键喊话，[`press_command`]）：和调工具一样，随应用登记、照工具的权限再判一次。
//!
//! 每条最近一次的结果记在内存里（[`last_run`]），`hotkeys.status` 的 `bindings[].lastRun` 给她和外部 Agent 看。

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};

use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};

use kotone_agent::catalog::Source;
use kotone_core::hotkey_bindings::Binding;
use kotone_core::hotkey_custom::{find_row, Action, CustomState, ROW_PREFIX};
use kotone_core::hotkey_table::LastRun;
use kotone_core::reminders::DeliveryKind;
use kotone_core::settings::Settings;

fn settings(app: &AppHandle) -> Settings {
    app.state::<crate::SharedState>()
        .settings
        .read()
        .unwrap()
        .clone()
}

/// 正在跑的（同一条再按不叠加）。
fn running() -> &'static Mutex<HashSet<String>> {
    static RUNNING: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    RUNNING.get_or_init(Default::default)
}

fn last_runs() -> &'static Mutex<HashMap<String, LastRun>> {
    static LAST: OnceLock<Mutex<HashMap<String, LastRun>>> = OnceLock::new();
    LAST.get_or_init(Default::default)
}

/// 这一条（行 id `custom.<id>`，插件预设是命令名）最近一次按下的结果；这次启动以后没按过为 None。
pub(crate) fn last_run(id: &str) -> Option<LastRun> {
    last_runs().lock().unwrap().get(id).cloned()
}

fn record(id: &str, ok: bool, message: &str) {
    last_runs().lock().unwrap().insert(
        id.to_string(),
        LastRun {
            at_ms: kotone_core::automation::now_ms(),
            ok,
            message: message.to_string(),
        },
    );
}

/// 登记 id 是不是自己加的那一条（`custom.<id>`）。
pub(crate) fn is_custom(id: &str) -> bool {
    id.starts_with(ROW_PREFIX)
}

/// 这一条要不要语音识别（跟她说一句要；别的不要）。表里找不到的当不要（按下时说清楚）。
pub(crate) fn needs_listening(app: &AppHandle, id: &str) -> bool {
    let s = settings(app);
    find_row(&s.hotkeys, id).and_then(|index| s.hotkeys.bindings[index].action().map(|a| a.kind()))
        == Some("say")
}

/// 这一条叫什么（说给玩家听的话里用）：玩家起的名字，没起就用键。
fn name_of(label: &str, key: &str) -> String {
    match (label.trim(), key.trim()) {
        ("", "") => "自己加的热键".to_string(),
        ("", key) => format!("热键 {key} "),
        (label, _) => format!("「{label}」"),
    }
}

/// 工具回的东西里给玩家看的一句：字就是它的第一行，对象找 `text` / `note` / `message` / `summary`；最多 60 字。
fn summary(value: &Value) -> Option<String> {
    let text = match value {
        Value::String(text) => text.as_str(),
        Value::Object(object) => ["text", "note", "message", "summary", "receipt"]
            .iter()
            .find_map(|k| object.get(*k).and_then(Value::as_str))?,
        _ => return None,
    };
    let line = text.lines().map(str::trim).find(|l| !l.is_empty())?;
    let mut out: String = line.chars().take(60).collect();
    if line.chars().count() > 60 {
        out.push('…');
    }
    Some(out)
}

/// 按下了自己加的 `id` 那一条。
pub(crate) fn press(app: &AppHandle, id: &str) {
    let s = settings(app);
    let Some(index) = find_row(&s.hotkeys, id) else {
        kotone_core::log::log(&format!("hotkey {id}: 表里已经没有这一条了"));
        return;
    };
    let binding = s.hotkeys.bindings[index].clone();
    let name = name_of(binding.label(), binding.key());
    match binding.action() {
        Some(Action::Say(text)) => crate::companion::say(app, text.to_string()),
        Some(Action::Page(uri)) => open_page(app, id, &name, uri),
        Some(Action::Automation { .. } | Action::Tool { .. }) => run_tool(app, id, name, binding),
        None => kotone_core::log::log(&format!("hotkey {id}: 读不懂这一条")),
    }
}

/// 按下了插件登记的调工具的命令（`<插件>.<命令>`，英雄联盟的“喊对面闪现”，ADR 041 第 3 期）：和自己加的“调一个工具”
/// 同一条路（照这个工具现在的权限再判一次、不叠加、成了弹幕一行、没成说原因）。不是这种命令返回 false。
pub(crate) fn press_command(app: &AppHandle, command: &str) -> bool {
    let Some((name, binding)) = crate::hotkey_bind::tool_command(command) else {
        return false;
    };
    run_tool(app, command, format!("「{name}」"), binding);
    true
}

/// 调工具 / 运行自动化：后台跑，上一次没做完不叠加，做完记结果；调工具的成了弹一行结果，运行自动化的成了不出声也不弹
/// （要不要通知由那个程序自己决定）；都是没成才说原因。
fn run_tool(app: &AppHandle, id: &str, name: String, binding: Binding) {
    if !running().lock().unwrap().insert(id.to_string()) {
        kotone_core::log::log(&format!("hotkey {id}: 上一次还没做完，这次不做"));
        return;
    }
    let automation = matches!(binding.action(), Some(Action::Automation { .. }));
    let app = app.clone();
    let id = id.to_string();
    tauri::async_runtime::spawn(async move {
        let result = call(&app, &binding).await;
        running().lock().unwrap().remove(&id);
        match result {
            Ok(line) => {
                kotone_core::log::log(&format!("hotkey {id}: ok: {line}"));
                record(&id, true, &line);
                // 运行自动化：程序跑起来了就够了，播报、弹幕、发到聊天都是程序里的事（2026-11-27 玩家）
                if automation {
                    return;
                }
                // 调工具：弹幕一行、不出声（P2），那是工具回的结果，查询类的没有它就什么都看不到
                let _ = crate::assistant::deliver_text(
                    &app,
                    format!("{}：{line}", name.trim_end()),
                    vec![DeliveryKind::Danmaku],
                )
                .await;
            }
            Err(error) => fail(&app, &id, &name, &error),
        }
    });
}

/// 没成：记下、照快捷指令以前的做法说一句（原因和下一步）。
fn fail(app: &AppHandle, id: &str, name: &str, error: &str) {
    kotone_core::log::log(&format!("hotkey {id}: failed: {error}"));
    record(id, false, error);
    crate::companion::tell(app, format!("{name}没成：{error}"));
}

/// 调这一条的工具，返回给玩家看的一句。先照现在的权限、插件开关再判一次（按的时候不问，D9）。
async fn call(app: &AppHandle, binding: &Binding) -> Result<String, String> {
    let settings = settings(app);
    let action = binding.action().ok_or("读不懂这一条")?;
    let (tool, args) = action.tool_call().ok_or("这一条不是调工具")?;
    let catalog = crate::hotkey_bind::catalog(&settings);
    match crate::hotkey_bind::state(&settings, &catalog, binding) {
        CustomState::Ready => {}
        CustomState::Off => return Err("它用的插件关着（在左侧栏的「插件」里可以打开）".into()),
        // 插件登记的预设没有确认这一步（条目里没有 id）
        CustomState::NeedsConfirm(reason) if binding.id().is_none() => {
            return Err(format!(
                "{reason}：热键上没法每次问，去她的「权限」里改成允许"
            ))
        }
        CustomState::NeedsConfirm(reason) => {
            return Err(format!("要先在「设置 → 热键」确认一次：{reason}"))
        }
        CustomState::Blocked(reason) => return Err(reason),
    }
    if let Action::Automation { id, args } = action {
        let args = args.cloned().unwrap_or_default();
        let value = crate::automation::run_saved(
            app,
            "hotkey",
            id,
            &args,
            kotone_core::automation::now_ms(),
        )?;
        return Ok(summary(&value).unwrap_or_else(|| format!("开始运行自动化 {id}")));
    }
    let entry = catalog
        .get(tool)
        .ok_or_else(|| format!("找不到工具 {tool}"))?;
    let done = || format!("{tool} 做完了");
    match &entry.source {
        Source::Service => {
            let service =
                crate::tool_service::service(app).ok_or("工具服务还没准备好，过一会再按")?;
            let envelope = service
                .call(kotone_tools::protocol::ToolCallRequest {
                    request_id: None,
                    tool: tool.to_string(),
                    arguments: args,
                    timeout_ms: None,
                })
                .await;
            if envelope.ok {
                Ok(envelope
                    .result
                    .as_ref()
                    .and_then(summary)
                    .unwrap_or_else(done))
            } else {
                Err(envelope
                    .error
                    .map(|e| e.message)
                    .unwrap_or_else(|| format!("{tool} 没成")))
            }
        }
        Source::Body => {
            let result = crate::game_chat::body_tool(
                app,
                tool,
                &args,
                "hotkey",
                kotone_core::send_to_game::Speaker::Automation,
                None,
            )
            .await?;
            Ok(summary(&result.content).unwrap_or_else(done))
        }
        Source::Plugin { .. } => {
            let result =
                crate::plugins::call(app, tool, &args, crate::plugins::Caller::Hotkey).await?;
            Ok(summary(&result.content).unwrap_or_else(done))
        }
        _ => Err(format!("{tool} 不能绑热键（只在和她的对话里用）")),
    }
}

/// 打开页面（P1）：窗口拿到前面（游戏独占全屏时不拿，用语音说一句），页面交给界面（标着是玩家自己要看的）。
fn open_page(app: &AppHandle, id: &str, name: &str, uri: &str) {
    let page = match crate::plugins::hotkey_page(&settings(app), uri) {
        Ok(page) => page,
        Err(None) => {
            return fail(
                app,
                id,
                name,
                "它的插件关着（在左侧栏的「插件」里可以打开）",
            )
        }
        Err(Some(reason)) => return fail(app, id, name, &reason),
    };
    let payload = match &page {
        crate::plugins::HotkeyPage::Kotone { uri, title, plugin } => {
            json!({"uri": uri, "params": null, "title": title, "plugin": plugin, "byPlayer": true})
        }
        crate::plugins::HotkeyPage::ThirdParty { plugin } => {
            json!({"uri": uri, "plugin": plugin, "thirdParty": true, "byPlayer": true})
        }
    };
    let front = app
        .get_webview_window("main")
        .map(|window| crate::attention::show_for_player(&window));
    let _ = app.emit(crate::plugins::UI_OPEN_EVENT, payload);
    if front == Some(false) {
        let hint = "页面在 Kotone 窗口里打开了；游戏是独占全屏，没把窗口拿到前面（游戏改成无边框窗口就能直接看到）";
        record(id, true, hint);
        let app = app.clone();
        let text = format!("{}：{hint}", name.trim_end());
        tauri::async_runtime::spawn(async move {
            let _ = crate::assistant::deliver_text(&app, text, vec![DeliveryKind::Voice]).await;
        });
    } else {
        record(id, true, "打开了");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_in_what_she_says() {
        assert_eq!(name_of("连发", "F8"), "「连发」");
        assert_eq!(format!("{}没成", name_of(" ", "F8")), "热键 F8 没成");
        assert_eq!(name_of("", ""), "自己加的热键");
        assert!(
            is_custom("custom.q1") && !is_custom("kotone.call") && !is_custom("quick-message-1")
        );
    }

    /// 调工具的热键弹幕里那一句（也是 lastRun 的 message）：工具回的第一行；对象里找 text / note / message；太长截掉。
    #[test]
    fn the_line_the_player_sees() {
        assert_eq!(
            summary(&json!("已发到队伍频道\n第二行")).as_deref(),
            Some("已发到队伍频道")
        );
        assert_eq!(
            summary(&json!({"status": "started", "note": "开始了"})).as_deref(),
            Some("开始了")
        );
        assert_eq!(summary(&json!({"status": "started"})), None);
        assert_eq!(
            summary(&json!("字".repeat(70))).unwrap().chars().count(),
            61
        );
    }

    /// 同一条上一次没做完：这次不做。
    #[test]
    fn a_second_press_while_running_is_dropped() {
        assert!(running()
            .lock()
            .unwrap()
            .insert("custom.test-running".into()));
        assert!(!running()
            .lock()
            .unwrap()
            .insert("custom.test-running".into()));
        running().lock().unwrap().remove("custom.test-running");
        record("custom.test-last", false, "原因");
        let last = last_run("custom.test-last").unwrap();
        assert!(!last.ok && last.message == "原因");
    }
}
