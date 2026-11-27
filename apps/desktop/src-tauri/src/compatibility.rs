//! Windows 游戏兼容性监测：UIPI 提权状态与独占全屏浮窗保护。

use std::sync::OnceLock;

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
    /// 管理员身份的说明现在是第几版（设置里的 `adminPromptRevision` 比它小 = 窗口里要先说明一次，ADR 044 D1）。
    /// 隔离实例（设了 KOTONE_HOME）是 0：不参与默认管理员，不说明。
    pub admin_prompt_revision: u32,
    /// 这次启动里已经发现的“用不了”（[`ELEVATION_NEEDED_EVENT`]）：`foreground`（管理员身份的游戏在前面）/
    /// `blocked`（打字或按键真的被拦了）；没有为 null。窗口晚打开时据此补上提示。
    pub notice: Option<&'static str>,
}

/// 所有游戏配置（磁盘上的 + 没被删的内置的）。提权检查、独占全屏监控、“前面是不是游戏”共用这一份，
/// 避免一边回退内置 profile、另一边漏掉。
fn game_profiles() -> Vec<GameProfile> {
    let mut available = profile::list();
    for builtin in [
        GameProfile::builtin_lol(),
        GameProfile::builtin_dota2(),
        GameProfile::builtin_cs2(),
        GameProfile::builtin_genshin(),
        GameProfile::builtin_generic(),
    ] {
        if (builtin.id == "dota2" && profile::builtin_dota2_is_deleted())
            || (builtin.id == "cs2" && profile::builtin_cs2_is_deleted())
            || (builtin.id == "genshin" && profile::builtin_genshin_is_deleted())
        {
            continue;
        }
        if !available.iter().any(|candidate| candidate.id == builtin.id) {
            available.push(builtin);
        }
    }
    available
}

/// 从当前激活 profile 解析正在运行的游戏 PID。提权检查与
/// 独占全屏监控共用这一真源，避免一边回退内置 profile、另一边漏掉。
pub(crate) fn active_game_pid(state: &SharedState) -> Option<u32> {
    resolve_active_game_pid(state)
}

fn resolve_active_game_pid(state: &SharedState) -> Option<u32> {
    let active = state.settings.read().unwrap().active_profile_id.clone();
    elevation::resolve_active_game_pid(active.as_deref(), &game_profiles(), &mut |name| {
        platform_inject::find_pid_by_name(name)
    })
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
        admin_prompt_revision: if isolated() {
            0
        } else {
            elevation::ADMIN_PROMPT_REVISION
        },
        notice: NOTICE.get().copied(),
    }
}

/// Kotone 不是管理员身份（UAC 点了“否”、或者关着“启动时以管理员身份运行”）而管理员身份的游戏在前面时
/// （ADR 044 D1），通知主窗口说清楚哪些用不了、给“以管理员身份重开”。载荷 `{reason}`：`foreground`
/// （发现前面是管理员身份的游戏）/ `blocked`（打字或按键真的被拦下了，[`platform_inject::elevation_refusal`]，
/// 她自己也会从工具结果里看到原因，用语音告诉玩家）。每次启动最多说一次。
pub(crate) const ELEVATION_NEEDED_EVENT: &str = "kotone://elevation-needed";

/// 隔离实例（设了 KOTONE_HOME：开发、测试、外部 Agent 的独立实例）：不参与默认管理员（见 `startup.rs`）。
fn isolated() -> bool {
    std::env::var_os("KOTONE_HOME").is_some()
}

/// 这次启动已经说过的那一次（[`ElevationStatus::notice`]）。
static NOTICE: OnceLock<&'static str> = OnceLock::new();

fn notify_elevation_needed(app: &AppHandle, reason: &'static str) {
    if NOTICE.set(reason).is_ok() {
        log::log(&format!("elevation: notice {reason}"));
        let _ = app.emit(
            ELEVATION_NEEDED_EVENT,
            serde_json::json!({ "reason": reason }),
        );
    }
}

/// 这个程序的窗口是不是游戏的：游戏配置里的进程，或者插件登记了要往里打字的窗口（英雄联盟客户端）。
fn is_game_window(process: &str) -> bool {
    game_profiles()
        .iter()
        .any(|p| !p.process_names.is_empty() && profile::matches_process(p, process))
        || crate::plugins::registry().iter().any(|plugin| {
            plugin
                .manifest()
                .send_to_game
                .iter()
                .any(|spec| spec.covers(process))
        })
}

/// 打字 / 按键被拦下时说一次；Kotone 不是管理员身份时，每 2 秒看一次前面是不是管理员身份的游戏，看到了说一次就停。
pub(crate) fn watch_input_refusals(app: AppHandle) {
    let refused = app.clone();
    platform_inject::set_elevation_listener(move || {
        notify_elevation_needed(&refused, "blocked");
    });
    if !cfg!(windows) || elevation::is_elevated() || isolated() {
        return;
    }
    let _ = std::thread::Builder::new()
        .name("kotone-elevation-watch".into())
        .spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_secs(2));
            if NOTICE.get().is_some() {
                break;
            }
            let Some(pid) = platform_inject::foreground_pid() else {
                continue;
            };
            if pid == std::process::id() || elevation::is_process_elevated(pid) != Some(true) {
                continue;
            }
            let Some(name) = platform_inject::process_name_from_pid(pid) else {
                continue;
            };
            if is_game_window(&name) {
                notify_elevation_needed(&app, "foreground");
                break;
            }
        });
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
