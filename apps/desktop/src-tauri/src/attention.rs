//! 能不能现在打扰玩家（共享资源评估第四节“屏幕”：一个判断，全宿主共用）。前台、全屏、游戏在不在前面，只在这里查。
//!
//! - [`may_take_focus`]：Kotone 的窗口能不能拿前台。只有玩家正看着 Kotone 自己的窗口时才行——那是他在 Kotone 里
//!   点了“登录”“打开”。她、外部 Agent（工具服务 / `kotone.exe mcp`）、自动触发的事都不行：玩家可能在游戏里，
//!   前台被拿走，游戏就收不到他的键了（真机：`scripts/manual-e2e/focus_steal_real.py`）。
//! - [`present`]：想让玩家看到一个窗口时用它：能拿前台就拿到前面；不能就放在最后面、不激活，任务栏按钮闪。
//!   照 Windows 自己的前台锁：后台程序要前台时，系统也只是闪它的任务栏按钮，玩家切出来就看得到。
//! - [`popup_blocked`]：消息提示（ADR 039 D4）现在能不能弹：Kotone 在前台、独占全屏、演示模式、别的程序全屏时不弹。
//! - [`remember_player_window`] / [`give_back_foreground`]：玩家点了盖在游戏上面、又必须能点的 Kotone 小窗
//!   （语音输入悬浮窗开着“允许交互并拖动”时）：点完把前台还给他原来在用的窗口，游戏马上又收得到键。
//!   这种小窗不能做成不可聚焦：`WS_EX_NOACTIVATE` 的窗口里，WebView2 收不到鼠标的按下和点击（2026-11-25 真机，
//!   `focus_steal_real.py` B2 / B3：页面一个事件都没收到，按钮点不了、也拖不动）。
//!
//! 气泡、弹幕、语音输入悬浮窗、插件覆盖层现在各自查独占全屏（`fullscreen::is_exclusive_fullscreen_active`），
//! 以后它们要“现在能不能出来”也从这里拿。

/// 前台窗口，和它是不是 Kotone 自己（这个进程）的窗口；没有前台窗口时 None。
/// 只看进程号：不用挨个向 Kotone 的窗口要句柄（那要经过界面线程，界面线程正忙或窗口正在关的时候就得等）。
#[cfg(windows)]
fn foreground() -> Option<(windows::Win32::Foundation::HWND, bool)> {
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
    // SAFETY: 只读当前前台窗口的句柄和它的进程号
    let front = unsafe { GetForegroundWindow() };
    if front.0.is_null() {
        return None;
    }
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(front, Some(&mut pid)) };
    Some((front, pid == std::process::id()))
}

/// 玩家正看着 Kotone 自己的窗口（主窗口、网页登录窗口……）吗：前台窗口是它、而且看得见、没最小化。
/// 看不见的不算：`--background` 启动时藏着的主窗口会占着“前台”（2026-11-25 真机看到的），那时玩家看的是别的。
pub fn kotone_in_front() -> bool {
    #[cfg(windows)]
    {
        use windows::Win32::UI::WindowsAndMessaging::{IsIconic, IsWindowVisible};
        foreground().is_some_and(|(front, ours)| {
            // SAFETY: 只读前台窗口的状态
            ours && unsafe { IsWindowVisible(front).as_bool() && !IsIconic(front).as_bool() }
        })
    }
    #[cfg(not(windows))]
    false
}

/// Kotone 的窗口现在能不能拿前台（见模块说明）。
pub fn may_take_focus() -> bool {
    kotone_in_front()
}

/// 让玩家看到这个窗口：能拿前台（[`may_take_focus`]）就显示、拿到前面、键盘焦点给页面；
/// 不能就放在最后面、不激活、闪任务栏（[`crate::show_behind`]）。返回拿没拿前台。
pub fn present(window: &tauri::WebviewWindow) -> bool {
    if may_take_focus() {
        crate::show_and_focus(window);
        true
    } else {
        crate::show_behind(window);
        false
    }
}

/// 鼠标移到一个会被点的 Kotone 小窗上时，玩家正在用的窗口（不是 Kotone 的；是 Kotone 的就记 0：点完不用还）。
#[cfg(windows)]
static PLAYER_WINDOW: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

/// 记下玩家现在在用的窗口（鼠标移到能点的 Kotone 小窗上时调；小窗还没被点，前台还是他的）。
pub fn remember_player_window() {
    #[cfg(windows)]
    {
        let value = match foreground() {
            Some((front, false)) => front.0 as isize,
            _ => 0,
        };
        PLAYER_WINDOW.store(value, std::sync::atomic::Ordering::SeqCst);
    }
}

/// `window` 拿着前台时（玩家刚点了它），把前台还给 [`remember_player_window`] 记下的窗口。
/// 那时前台是 Kotone 自己，系统允许它把前台交出去，不用按 Alt。返回还了没有。
pub fn give_back_foreground(window: &tauri::WebviewWindow) -> bool {
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::HWND;
        use windows::Win32::UI::WindowsAndMessaging::{
            GetForegroundWindow, IsWindow, SetForegroundWindow,
        };
        let Ok(ours) = window.hwnd() else {
            return false;
        };
        let player = PLAYER_WINDOW.load(std::sync::atomic::Ordering::SeqCst);
        // SAFETY: 只读前台窗口、核对记下的窗口还在，再把前台交给它
        unsafe {
            let player = HWND(player as _);
            if player.0.is_null()
                || GetForegroundWindow().0 != ours.0
                || !IsWindow(Some(player)).as_bool()
            {
                return false;
            }
            SetForegroundWindow(player).as_bool()
        }
    }
    #[cfg(not(windows))]
    {
        let _ = window;
        false
    }
}

/// 消息提示现在不弹的原因（None = 可以弹）。
pub fn popup_blocked() -> Option<&'static str> {
    if kotone_in_front() {
        return Some("Kotone 的窗口在前台");
    }
    let game_in_front = crate::games::states().iter().any(|s| s.focused);
    match kotone_platform_windows::fullscreen::notification_state() {
        Some(code) if kotone_platform_windows::fullscreen::popup_blocked(code, game_in_front) => {
            Some(match code {
                3 => "游戏独占全屏",
                4 => "演示模式",
                _ => "别的程序全屏",
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    /// 盖在游戏上面的窗口（悬浮窗、气泡、弹幕、消息提示、插件覆盖层）建的时候都不要焦点（`focus: false`），
    /// 显示时走 `SW_SHOWNA`：出现的时候前台照样是游戏。只看、不点的（气泡、弹幕、插件覆盖层）还不可聚焦。
    /// 语音输入悬浮窗例外：开着“允许交互并拖动”时要能点，不可聚焦的窗口里 WebView2 收不到点击，
    /// 所以它可聚焦、点完把前台还回去（[`super::give_back_foreground`]）。主窗口是玩家自己打开的，能拿焦点。
    #[test]
    fn windows_over_the_game_never_take_the_focus() {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let windows = config["app"]["windows"].as_array().unwrap();
        let window = |label: &str| {
            windows
                .iter()
                .find(|w| w["label"] == label)
                .unwrap_or_else(|| panic!("tauri.conf.json 里没有 {label}"))
        };
        let over_game: Vec<&serde_json::Value> =
            windows.iter().filter(|w| w["label"] != "main").collect();
        assert!(over_game.len() >= 5, "{windows:?}");
        for w in over_game {
            assert_eq!(
                w["focus"], false,
                "{} 要 focus: false（出现的时候不能把游戏切出去）",
                w["label"]
            );
        }
        for label in ["companion", "danmaku", "plugin-overlay"] {
            assert_eq!(
                window(label)["focusable"],
                false,
                "{label} 只看不点，要 focusable: false"
            );
        }
        assert_ne!(
            window("overlay")["focusable"],
            false,
            "悬浮窗要能点（不可聚焦时 WebView2 收不到点击），点完把前台还回去"
        );
        let main = window("main");
        assert_ne!(main["focusable"], false, "主窗口要能拿焦点（能打字）");
        // 建的时候也不要焦点：--background 启动时不抢前台（background_focus_real.py）
        assert_eq!(main["focus"], false);
    }
}
