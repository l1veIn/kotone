//! Windows 独占全屏探测。
//!
//! 悬浮窗在独占全屏 Direct3D 游戏上方出现时，Windows 可能把游戏最小化。
//! `SHQueryUserNotificationState` 能区分该状态与普通的无边框全屏，因此这里只在
//! `QUNS_RUNNING_D3D_FULL_SCREEN` 时提醒，不用窗口尺寸启发式误伤无边框模式。

const QUNS_RUNNING_D3D_FULL_SCREEN_CODE: i32 = 3;
/// 有程序全屏（不是独占的 Direct3D，例如全屏看视频、无边框全屏的游戏）或开了“演示设置”。
const QUNS_BUSY_CODE: i32 = 2;
/// 演示模式（投屏讲 PPT）。
const QUNS_PRESENTATION_MODE_CODE: i32 = 4;

fn is_exclusive_fullscreen_state(code: i32) -> bool {
    code == QUNS_RUNNING_D3D_FULL_SCREEN_CODE
}

/// 现在弹消息提示（ADR 039 D4，右下角的小窗）合不合适，照 Windows 自己的通知（专注助手）用同一个查询：
/// 独占全屏的游戏、演示模式不弹；别的程序全屏（`QUNS_BUSY`：全屏看视频……）也不弹，**但前台是 Kotone 认得的
/// 游戏时照样弹**——无边框窗口化的游戏也会报 `QUNS_BUSY`，盖在它上面的置顶小窗显示得出来（和气泡、覆盖层一样）。
/// `code` 是 `SHQueryUserNotificationState` 的值。
pub fn popup_blocked(code: i32, foreground_is_game: bool) -> bool {
    match code {
        QUNS_RUNNING_D3D_FULL_SCREEN_CODE | QUNS_PRESENTATION_MODE_CODE => true,
        QUNS_BUSY_CODE => !foreground_is_game,
        _ => false,
    }
}

/// `SHQueryUserNotificationState` 的原始值（见 [`popup_blocked`]）；查不到时 None。
#[cfg(windows)]
pub fn notification_state() -> Option<i32> {
    use windows::Win32::UI::Shell::SHQueryUserNotificationState;

    unsafe { SHQueryUserNotificationState() }
        .ok()
        .map(|state| state.0)
}

#[cfg(not(windows))]
pub fn notification_state() -> Option<i32> {
    None
}

/// 当前桌面是否有独占全屏 Direct3D 应用。
///
/// `Some(true/false)` = Windows 已成功查询；`None` = 非 Windows 或系统调用失败。
#[cfg(windows)]
pub fn is_exclusive_fullscreen_active() -> Option<bool> {
    use windows::Win32::UI::Shell::SHQueryUserNotificationState;

    unsafe { SHQueryUserNotificationState() }
        .ok()
        .map(|state| is_exclusive_fullscreen_state(state.0))
}

#[cfg(not(windows))]
pub fn is_exclusive_fullscreen_active() -> Option<bool> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_direct3d_exclusive_state_is_treated_as_fullscreen() {
        assert!(is_exclusive_fullscreen_state(3));
        assert!(!is_exclusive_fullscreen_state(2));
        assert!(!is_exclusive_fullscreen_state(5));
    }

    #[test]
    fn popups_stay_away_from_exclusive_fullscreen_and_presentations_but_not_borderless_games() {
        // 独占全屏、演示模式：不弹（游戏照样是不是前台都一样）
        assert!(popup_blocked(3, true));
        assert!(popup_blocked(4, false));
        // 别的程序全屏（看视频）不弹；无边框全屏的游戏在前台照样弹
        assert!(popup_blocked(2, false));
        assert!(!popup_blocked(2, true));
        // 平常（5 = 接受通知）、没登录（1）、勿扰时段（6）、商店应用全屏（7）：弹（勿扰只管系统通知）
        for code in [1, 5, 6, 7] {
            assert!(!popup_blocked(code, false), "{code}");
        }
    }
}
