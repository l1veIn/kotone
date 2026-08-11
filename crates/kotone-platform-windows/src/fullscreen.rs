//! Windows 独占全屏探测。
//!
//! 悬浮窗在独占全屏 Direct3D 游戏上方出现时，Windows 可能把游戏最小化。
//! `SHQueryUserNotificationState` 能区分该状态与普通的无边框全屏，因此这里只在
//! `QUNS_RUNNING_D3D_FULL_SCREEN` 时提醒，不用窗口尺寸启发式误伤无边框模式。

const QUNS_RUNNING_D3D_FULL_SCREEN_CODE: i32 = 3;

fn is_exclusive_fullscreen_state(code: i32) -> bool {
    code == QUNS_RUNNING_D3D_FULL_SCREEN_CODE
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
}
