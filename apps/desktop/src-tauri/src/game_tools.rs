//! 游戏场景工具（ADR 014 第 5 步）：`game.list` / `game.focus` / `game.launch` / `game.close`，
//! 以及按游戏配置的“允许自动化输入”开关检查自动化程序。
//!
//! - 游戏 = 一个有 `processNames` 的游戏配置；通用配置匹配任何进程，不算“在运行”；
//! - 从不操作 Kotone 自己的窗口；关闭只发普通的关闭请求，从不强制结束进程；
//! - 打开只用用户在游戏配置里填写的启动路径。

use std::time::Duration;

use kotone_core::profile::{self, GameProfile};
use kotone_core::tools::{WindowInfo, WindowObserver};
use kotone_platform_windows::game_window;
use kotone_platform_windows::window_tools::WindowsWindowObserver;
use serde_json::{json, Value};

/// 切到前台后等游戏重绘一下，紧接着的截图 / 识别才拿得到画面。
const AFTER_FOCUS: Duration = Duration::from_millis(250);

fn all_windows() -> Result<Vec<WindowInfo>, String> {
    WindowsWindowObserver
        .list_windows()
        .map_err(|e| format!("读取窗口列表失败：{e}"))
}

/// 不是 Kotone 自己、看得见、有标题的顶层窗口。
fn candidate(window: &WindowInfo) -> bool {
    window.identity.pid != std::process::id() && !window.cloaked && !window.title.trim().is_empty()
}

fn game_windows<'a>(game: &GameProfile, all: &'a [WindowInfo]) -> Vec<&'a WindowInfo> {
    if game.process_names.is_empty() {
        return Vec::new();
    }
    all.iter()
        .filter(|w| candidate(w))
        .filter(|w| {
            w.process_name
                .as_deref()
                .is_some_and(|p| profile::matches_process(game, p))
        })
        .collect()
}

fn window_json(window: &WindowInfo) -> Value {
    json!({
        "title": window.title,
        "process": window.process_name,
        "minimized": window.minimized,
        "foreground": window.foreground,
    })
}

fn find_game(id: &str) -> Result<GameProfile, String> {
    profile::get(id).ok_or_else(|| format!("找不到游戏配置 {id}"))
}

pub fn list() -> Result<Value, String> {
    let all = all_windows()?;
    let games: Vec<Value> = profile::list()
        .iter()
        .map(|game| {
            let windows = game_windows(game, &all);
            json!({
                "id": game.id,
                "name": game.display_name,
                "processNames": game.process_names,
                "running": !windows.is_empty(),
                "foreground": windows.iter().any(|w| w.foreground),
                "windows": windows.iter().take(5).map(|w| window_json(w)).collect::<Vec<_>>(),
                "allowAutomationInput": game.allow_automation_input,
                "canLaunch": !game.launch_path.trim().is_empty(),
            })
        })
        .collect();
    Ok(json!({"games": games}))
}

/// 按条件挑窗口（纯函数，便于测试）：优先没最小化的。
fn pick<'a>(
    all: &'a [WindowInfo],
    game: Option<&GameProfile>,
    process_name: Option<&str>,
    title_contains: Option<&str>,
) -> Option<&'a WindowInfo> {
    let title = title_contains.map(str::to_lowercase);
    let mut matches: Vec<&WindowInfo> = all
        .iter()
        .filter(|w| candidate(w))
        .filter(|w| {
            game.map_or(true, |g| {
                !g.process_names.is_empty()
                    && w.process_name
                        .as_deref()
                        .is_some_and(|p| profile::matches_process(g, p))
            })
        })
        .filter(|w| {
            process_name.map_or(true, |name| {
                w.process_name
                    .as_deref()
                    .is_some_and(|p| p.eq_ignore_ascii_case(name))
            })
        })
        .filter(|w| {
            title
                .as_deref()
                .map_or(true, |t| w.title.to_lowercase().contains(t))
        })
        .collect();
    matches.sort_by_key(|w| w.minimized);
    matches.into_iter().next()
}

pub fn focus(
    profile_id: Option<&str>,
    process_name: Option<&str>,
    title_contains: Option<&str>,
) -> Result<Value, String> {
    if profile_id.is_none() && process_name.is_none() && title_contains.is_none() {
        return Err("要给 profileId、processName 或 titleContains 之一".into());
    }
    let game = profile_id.map(find_game).transpose()?;
    if game.as_ref().is_some_and(|g| g.process_names.is_empty()) {
        return Err("这是通用配置，不对应具体游戏：请用 processName 或 titleContains".into());
    }
    let all = all_windows()?;
    let window = pick(&all, game.as_ref(), process_name, title_contains)
        .ok_or("没有找到对应的游戏窗口：游戏可能没开（可以用 game.list 查看）")?;
    game_window::focus_window(window.identity.handle)?;
    std::thread::sleep(AFTER_FOCUS);
    Ok(json!({"focused": window_json(window)}))
}

pub fn launch(profile_id: &str) -> Result<Value, String> {
    let game = find_game(profile_id)?;
    if game.launch_path.trim().is_empty() {
        return Err(format!(
            "「{}」没有设置启动路径：请在「游戏适配」里填写",
            game.display_name
        ));
    }
    let all = all_windows()?;
    if !game_windows(&game, &all).is_empty() {
        return Ok(json!({"launched": false, "alreadyRunning": true}));
    }
    game_window::launch(&game.launch_path)?;
    Ok(json!({"launched": true, "alreadyRunning": false}))
}

pub fn close(profile_id: &str) -> Result<Value, String> {
    let game = find_game(profile_id)?;
    if game.process_names.is_empty() {
        return Err("这是通用配置，不对应具体游戏".into());
    }
    let all = all_windows()?;
    let windows = game_windows(&game, &all);
    if windows.is_empty() {
        return Err(format!("「{}」没有在运行", game.display_name));
    }
    let mut requested = 0;
    for window in windows {
        if game_window::close_window(window.identity.handle).is_ok() {
            requested += 1;
        }
    }
    Ok(json!({"requested": requested}))
}

/// 自动化程序能不能对这个进程按键：看它对应的游戏配置的“允许自动化输入”。
pub fn input_allowed(process_name: Option<&str>) -> Result<(), String> {
    let Some(name) = process_name else {
        return Ok(());
    };
    check_input(&profile::list(), name)
}

fn check_input(profiles: &[GameProfile], process_name: &str) -> Result<(), String> {
    match profile::find_by_process(profiles, process_name) {
        Some(game) if !game.allow_automation_input => Err(format!(
            "「{}」设置为不允许自动化输入，自动化没有启动（可以在「游戏适配」里打开）",
            game.display_name
        )),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kotone_core::tools::WindowIdentity;

    fn window(handle: u64, pid: u32, process: &str, title: &str, minimized: bool) -> WindowInfo {
        WindowInfo {
            identity: WindowIdentity {
                handle,
                pid,
                process_started_at: None,
            },
            title: title.into(),
            class_name: String::new(),
            process_name: Some(process.into()),
            visible: true,
            minimized,
            cloaked: false,
            foreground: false,
            client_rect: None,
            monitor_rect: None,
            dpi: None,
        }
    }

    #[test]
    fn picks_a_matching_window_and_never_kotone_itself() {
        let own = std::process::id();
        let all = vec![
            window(1, own, "kotone.exe", "Kotone 设置", false),
            window(
                2,
                10,
                "League of Legends.exe",
                "League of Legends (TM) Client",
                true,
            ),
            window(3, 11, "League of Legends.exe", "League of Legends", false),
            window(4, 12, "chrome.exe", "攻略 - Chrome", false),
        ];
        let lol = GameProfile::builtin_lol();
        // 同一游戏两个窗口：优先没最小化的。
        assert_eq!(
            pick(&all, Some(&lol), None, None).unwrap().identity.handle,
            3
        );
        assert_eq!(
            pick(&all, None, Some("chrome.EXE"), None)
                .unwrap()
                .identity
                .handle,
            4
        );
        assert_eq!(
            pick(&all, None, None, Some("client"))
                .unwrap()
                .identity
                .handle,
            2
        );
        assert!(pick(&all, None, Some("kotone.exe"), None).is_none());
        assert!(pick(&all, None, Some("dota2.exe"), None).is_none());
        // 通用配置不对应具体游戏。
        assert!(pick(&all, Some(&GameProfile::builtin_generic()), None, None).is_none());
    }

    #[test]
    fn automation_input_follows_the_matching_profile() {
        let mut lol = GameProfile::builtin_lol();
        let generic = GameProfile::builtin_generic();
        assert!(check_input(&[lol.clone(), generic.clone()], "League of Legends.exe").is_ok());
        lol.allow_automation_input = false;
        let err =
            check_input(&[lol.clone(), generic.clone()], "League of Legends.exe").unwrap_err();
        assert!(err.contains("不允许自动化输入"));
        // 别的游戏走通用配置，不受影响。
        assert!(check_input(&[lol, generic], "dota2.exe").is_ok());
    }
}
