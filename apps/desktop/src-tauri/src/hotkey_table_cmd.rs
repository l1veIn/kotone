//! 热键表的命令（ADR 040）：「设置 → 热键」那一页、各页面改键前的冲突提示、`settings.update` 拒绝新冲突、
//! 宿主登记热键（`hotkey.rs`）、`hotkeys.status` / `hotkeys.conflicts` 工具都经这里读
//! [`kotone_core::hotkey_table`]——表、冲突、谁先登记都只在那里算一次，前端只显示结果。自己加的调工具、打开页面现在
//! 能不能按（工具目录、插件开关、权限）由 [`crate::hotkey_bind::state`] 交给它（ADR 040 第 3 期 D9）。

use kotone_core::hotkey_bindings::legacy_by_id;
use kotone_core::hotkey_commands::split_command;
use kotone_core::hotkey_table::{self, HotkeyCheck, HotkeyTable, Planned, PluginInfo, Sources};
use kotone_core::profile::{self, GameProfile};
use kotone_core::settings::Settings;
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

use crate::SharedState;

/// 内置插件（插件表的顺序）：名字、开没开、清单里登记的命令（ADR 040 D1）。
fn plugin_infos(settings: &Settings) -> Vec<PluginInfo<'static>> {
    crate::plugins::registry()
        .iter()
        .map(|plugin| {
            let manifest = plugin.manifest();
            PluginInfo {
                id: &manifest.id,
                name: &manifest.name,
                enabled: crate::plugins::enabled(settings, manifest),
                commands: &manifest.commands,
            }
        })
        .collect()
}

fn with_sources<T>(
    settings: &Settings,
    profiles: &[GameProfile],
    f: impl FnOnce(&Sources<'_>) -> T,
) -> T {
    let plugins = plugin_infos(settings);
    let catalog = crate::hotkey_bind::catalog(settings);
    let custom = |binding: &kotone_core::hotkey_bindings::Binding| {
        crate::hotkey_bind::state(settings, &catalog, binding)
    };
    f(&Sources {
        settings,
        profiles,
        plugins: &plugins,
        custom: Some(&custom),
    })
}

/// 照当前设置和全部游戏配置查。锁里只复制设置，读游戏配置（磁盘）在锁外。
fn with_current<T>(app: &AppHandle, f: impl FnOnce(&Sources<'_>) -> T) -> T {
    let settings = app.state::<SharedState>().settings.read().unwrap().clone();
    let profiles = profile::list();
    with_sources(&settings, &profiles, f)
}

/// 整张表；自己加的那几行带最近一次按下的结果（`lastRun`，[`crate::hotkey_run`]）。
pub(crate) fn table(app: &AppHandle) -> HotkeyTable {
    let mut table = with_current(app, hotkey_table::table);
    for row in &mut table.bindings {
        row.last_run = crate::hotkey_run::last_run(&row.id);
    }
    table
}

pub(crate) fn check(app: &AppHandle, key: &str, except: Option<&str>) -> HotkeyCheck {
    with_current(app, |src| hotkey_table::check(src, key, except))
}

/// 登记计划（谁先拿到键、哪些识别没跑也登记、哪些只看不吞）。登记不看游戏配置，不读磁盘，可以在设置事务里调
/// （自己加的能不能按只读内存里的工具目录和设置）。
pub(crate) fn registration_plan(settings: &Settings) -> Vec<Planned> {
    with_sources(settings, &[], hotkey_table::registration_plan)
}

/// 一次修改新造成的冲突（不让存；已经冲突着、这次没动的不拦）。`profiles` 在设置事务外读好。
pub(crate) fn new_conflict(
    before: &Settings,
    after: &Settings,
    profiles: &[GameProfile],
) -> Option<String> {
    let before_plugins = plugin_infos(before);
    let after_plugins = plugin_infos(after);
    hotkey_table::new_conflict_error(
        &Sources {
            settings: before,
            profiles,
            plugins: &before_plugins,
            custom: None,
        },
        &Sources {
            settings: after,
            profiles,
            plugins: &after_plugins,
            custom: None,
        },
    )
}

/// 「设置 → 热键」：全部热键（本体的、插件的、自己加的、Esc），每一项带冲突和在哪改。
#[tauri::command]
pub(crate) async fn get_hotkey_table(app: AppHandle) -> Result<HotkeyTable, String> {
    tauri::async_runtime::spawn_blocking(move || table(&app))
        .await
        .map_err(|e| format!("读热键表失败：{e}"))
}

/// 把 `except` 这一项改成 `key` 之前查一次：和谁冲突、这个键游戏里常不常用。
#[tauri::command]
pub(crate) async fn check_hotkey(
    app: AppHandle,
    key: String,
    except: Option<String>,
) -> Result<HotkeyCheck, String> {
    tauri::async_runtime::spawn_blocking(move || check(&app, &key, except.as_deref()))
        .await
        .map_err(|e| format!("查热键冲突失败：{e}"))
}

/// 「设置 → 热键」里改一个键（ADR 040 D3：热键只在这一页改；新手引导第一次设呼叫键也走这里）。
/// `id` 是表里那一行的 id（命令名、自己加的 `custom.<id>`；第 1 期的 id、`quick-message-n` 也认），`key` 空 = 清掉。
/// 走和设置页保存同一条路：新造成的冲突不让存，说清楚和谁撞了；存了马上重新登记。
#[tauri::command]
pub(crate) fn set_hotkey(app: AppHandle, id: String, key: String) -> Result<Settings, String> {
    let current = app.state::<SharedState>().settings.read().unwrap().clone();
    let patch = hotkey_patch(&current, &id, key.trim())?;
    crate::settings_cmd::update_settings_from_backend(&app, patch)
}

/// 改一个键的设置补丁：写热键表（整组替换，别的条目原样）。命令改它的第一条；自己加的（快捷指令、运行自动化，
/// 第 3 期搬进了表）改那一条的键，条目留着。
fn hotkey_patch(current: &Settings, id: &str, key: &str) -> Result<Value, String> {
    if !key.is_empty() && kotone_core::hotkey::parse_hotkey(key).is_none() {
        return Err(format!(
            "认不出「{key}」这个键：写成 F8、Ctrl+Alt+K、Mouse4 这样，或者点“录入”直接按"
        ));
    }
    if kotone_core::hotkey_custom::resolve_row(current, id).is_some() {
        let mut next = current.clone();
        next.set_custom_key(id, key)?;
        return Ok(json!({"hotkeys": {"bindings": next.hotkeys.bindings}}));
    }
    let command = legacy_by_id(id).map_or(id, |legacy| legacy.command);
    if split_command(command).is_none() {
        return Err(match id {
            hotkey_table::KEY_ESCAPE => "Esc 是固定的，不能改".to_string(),
            hotkey_table::HK_LEGACY_REMINDER => "提醒助手热键在「提醒助手」页改".to_string(),
            _ => format!("「{id}」在这里改不了"),
        });
    }
    let mut next = current.clone();
    next.set_binding(command, key);
    Ok(json!({"hotkeys": {"bindings": next.hotkeys.bindings}}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use kotone_core::hotkey_bindings::{CMD_CALL, CMD_CHANNEL_CYCLE, CMD_TALK};

    #[test]
    fn a_hotkey_change_is_a_settings_patch() {
        let mut settings = Settings::default();
        settings
            .hotkeys
            .bindings
            .push(kotone_core::hotkey_custom::binding_of(
                &kotone_core::assistant::QuickMessage {
                    id: "q1".into(),
                    text: "集合".into(),
                    ..Default::default()
                },
            ));
        settings.sync_hotkey_mirrors();
        // 命令：整组替换热键表，别的条目原样
        let patch = hotkey_patch(&settings, CMD_CHANNEL_CYCLE, "Alt+C").unwrap();
        let bindings = &patch["hotkeys"]["bindings"];
        assert_eq!(bindings.as_array().unwrap().len(), 5);
        assert_eq!(
            bindings[4],
            json!({"key": "Alt+C", "command": CMD_CHANNEL_CYCLE})
        );
        // 第 1 期的 id 也认；空 = 清掉
        let patch = hotkey_patch(&settings, "record", "").unwrap();
        assert!(patch["hotkeys"]["bindings"]
            .as_array()
            .unwrap()
            .iter()
            .all(|b| b["command"] != CMD_TALK));
        assert_eq!(
            hotkey_patch(&settings, CMD_CALL, "F9").unwrap()["hotkeys"]["bindings"][0]["key"],
            "F9"
        );
        // 自己加的（快捷指令搬进了表）：改那一条的键，话不动；第 1、2 期的 `quick-message-n` 也认
        for id in ["custom.q1", "quick-message-1"] {
            let patch = hotkey_patch(&settings, id, "F8").unwrap();
            assert_eq!(
                patch["hotkeys"]["bindings"][3],
                json!({"key": "F8", "say": "集合", "id": "q1"}),
                "{id}"
            );
        }
        assert!(hotkey_patch(&settings, "quick-message-2", "F8").is_err());
        assert!(hotkey_patch(&settings, "custom.gone", "F8").is_err());
        // 改不了的、认不出的键
        assert!(hotkey_patch(&settings, "escape", "F8").is_err());
        assert!(hotkey_patch(&settings, "legacy-reminder", "F8").is_err());
        assert!(hotkey_patch(&settings, CMD_TALK, "NotAKey").is_err());
    }
}
