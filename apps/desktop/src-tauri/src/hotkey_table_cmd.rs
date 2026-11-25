//! 热键表的命令（ADR 040 第 1 期）：「设置 → 热键」那一页、各页面改键前的冲突提示、
//! `hotkeys.status` / `hotkeys.conflicts` 工具都经这里读 [`kotone_core::hotkey_table`]——冲突只在那里查一次，
//! 前端只显示结果。

use kotone_core::hotkey_table::{self, HotkeyCheck, HotkeyTable, PluginInfo, Sources};
use kotone_core::profile;
use tauri::{AppHandle, Manager};

use crate::SharedState;

/// 照当前设置和全部游戏配置查。锁里只复制设置，读游戏配置（磁盘）在锁外。
fn with_sources<T>(app: &AppHandle, f: impl FnOnce(&Sources<'_>) -> T) -> T {
    let settings = app.state::<SharedState>().settings.read().unwrap().clone();
    let profiles = profile::list();
    // 语音输入的 5 个键还写死在本体的设置里（ADR 040 第 2 期改成插件登记的命令）
    let manifest = kotone_plugin_voice_input::plugin::manifest();
    let voice_input = PluginInfo {
        id: &manifest.id,
        name: &manifest.name,
        enabled: crate::plugins::voice_input_on(&settings),
    };
    f(&Sources {
        settings: &settings,
        profiles: &profiles,
        voice_input,
    })
}

pub(crate) fn table(app: &AppHandle) -> HotkeyTable {
    with_sources(app, hotkey_table::table)
}

pub(crate) fn check(app: &AppHandle, key: &str, except: Option<&str>) -> HotkeyCheck {
    with_sources(app, |src| hotkey_table::check(src, key, except))
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
