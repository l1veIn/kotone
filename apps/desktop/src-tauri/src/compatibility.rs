//! Windows 游戏兼容性监测：UIPI 提权状态与独占全屏浮窗保护。

use tauri::{AppHandle, Emitter, Manager};

use kotone_core::log;
use kotone_core::profile::{self, GameProfile};
use kotone_core::runtime::RuntimePhase;
use kotone_core::settings::OverlayVisibility;
use kotone_platform_windows::{elevation, fullscreen, inject as platform_inject};

use crate::runtime::RuntimeManager;
use crate::{hide_window, show_window_no_focus, SharedState};

/// 游戏兼容状态：提权链路 + 当前激活 profile 是否处于独占全屏。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ElevationStatus {
    pub elevated: bool,
    pub active_game_elevated: Option<bool>,
    /// 当前游戏是否处于独占全屏；无活动游戏或系统调用失败时为 null。
    pub active_game_fullscreen: Option<bool>,
    /// 当前平台是否支持提权语义（仅 Windows）。
    pub supported: bool,
}

/// 从当前激活 profile 解析正在运行的游戏 PID。提权检查与
/// 独占全屏监控共用这一真源，避免一边回退内置 profile、另一边漏掉。
fn resolve_active_game_pid(state: &SharedState) -> Option<u32> {
    let guard = state.settings.read().unwrap();
    let mut available = profile::list();
    for builtin in [GameProfile::builtin_lol(), GameProfile::builtin_generic()] {
        if !available.iter().any(|candidate| candidate.id == builtin.id) {
            available.push(builtin);
        }
    }
    elevation::resolve_active_game_pid(
        guard.active_profile_id.as_deref(),
        &available,
        &mut |name| platform_inject::find_pid_by_name(name),
    )
}

fn active_game_fullscreen(state: &SharedState) -> Option<bool> {
    resolve_active_game_pid(state).and_then(|pid| {
        if platform_inject::foreground_pid() == Some(pid) {
            fullscreen::is_exclusive_fullscreen_active()
        } else {
            Some(false)
        }
    })
}

pub(crate) fn elevation_status(state: &SharedState) -> ElevationStatus {
    let active_game_pid = resolve_active_game_pid(state);
    ElevationStatus {
        elevated: elevation::is_elevated(),
        active_game_elevated: active_game_pid.and_then(elevation::is_process_elevated),
        active_game_fullscreen: active_game_fullscreen(state),
        supported: cfg!(windows),
    }
}

/// 设置窗口隐藏/停留在其它分页时仍监控活动游戏。检测到独占
/// 全屏后立即隐藏浮窗并发事件；离开全屏后，只恢复用户明确选择的
/// Always 模式。任务随 AppHandle 生命周期结束，不持有任何设置锁跨 await。
pub(crate) fn start_fullscreen_monitor(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut warned = false;
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            let Some(state) = app.try_state::<SharedState>() else {
                break;
            };
            let is_fullscreen = active_game_fullscreen(&state) == Some(true);
            if is_fullscreen && !warned {
                warned = true;
                if let Some(win) = app.get_webview_window("overlay") {
                    hide_window(&win);
                }
                let _ = app.emit(
                    "kotone://fullscreen-warning",
                    serde_json::json!({ "exclusiveFullscreen": true }),
                );
                log::log("exclusive fullscreen game detected: overlay hidden");
            } else if !is_fullscreen && warned {
                warned = false;
                let should_restore = state.settings.read().unwrap().overlay.visibility
                    == OverlayVisibility::Always
                    && app
                        .try_state::<RuntimeManager>()
                        .is_some_and(|runtime| runtime.phase() == RuntimePhase::Running);
                if should_restore {
                    if let Some(win) = app.get_webview_window("overlay") {
                        show_window_no_focus(&win);
                    }
                }
            }
        }
    });
}
