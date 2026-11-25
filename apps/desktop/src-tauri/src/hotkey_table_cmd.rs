//! 热键表的命令（ADR 040）：「设置 → 热键」那一页、各页面改键前的冲突提示、`settings.update` 拒绝新冲突、
//! 宿主登记热键（`hotkey.rs`）、`hotkeys.status` / `hotkeys.conflicts` 工具都经这里读
//! [`kotone_core::hotkey_table`]——表、冲突、谁先登记都只在那里算一次，前端只显示结果。

use kotone_core::hotkey_table::{self, HotkeyCheck, HotkeyTable, Planned, PluginInfo, Sources};
use kotone_core::profile::{self, GameProfile};
use kotone_core::settings::Settings;
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
    f(&Sources {
        settings,
        profiles,
        plugins: &plugins,
    })
}

/// 照当前设置和全部游戏配置查。锁里只复制设置，读游戏配置（磁盘）在锁外。
fn with_current<T>(app: &AppHandle, f: impl FnOnce(&Sources<'_>) -> T) -> T {
    let settings = app.state::<SharedState>().settings.read().unwrap().clone();
    let profiles = profile::list();
    with_sources(&settings, &profiles, f)
}

pub(crate) fn table(app: &AppHandle) -> HotkeyTable {
    with_current(app, hotkey_table::table)
}

pub(crate) fn check(app: &AppHandle, key: &str, except: Option<&str>) -> HotkeyCheck {
    with_current(app, |src| hotkey_table::check(src, key, except))
}

/// 登记计划（谁先拿到键、哪些识别没跑也登记、哪些只看不吞）。登记不看游戏配置，不读磁盘，可以在设置事务里调。
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
        },
        &Sources {
            settings: after,
            profiles,
            plugins: &after_plugins,
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
