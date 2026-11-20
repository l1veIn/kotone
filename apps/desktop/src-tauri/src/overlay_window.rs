//! 语音输入悬浮窗：按需显隐、摆位与尺寸、界面查询与拖动记位置。

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OnDemandOverlayAction {
    Show,
    Hide,
    HideSuccessAfterDwell,
    Keep,
}

fn on_demand_overlay_action(state: &str, continuous: bool) -> OnDemandOverlayAction {
    match state {
        "listening" | "transcribing" | "processing" | "preview" | "sending" | "error" => {
            OnDemandOverlayAction::Show
        }
        "success" if !continuous => OnDemandOverlayAction::HideSuccessAfterDwell,
        "idle" => OnDemandOverlayAction::Hide,
        _ => OnDemandOverlayAction::Keep,
    }
}

pub(crate) fn overlay_window_action(
    visibility: OverlayVisibility,
    state: &str,
    continuous: bool,
    runtime_running: bool,
) -> OnDemandOverlayAction {
    match visibility {
        OverlayVisibility::Never => OnDemandOverlayAction::Hide,
        OverlayVisibility::Always => {
            if state == "idle" && !runtime_running {
                OnDemandOverlayAction::Hide
            } else if state != "idle" {
                OnDemandOverlayAction::Show
            } else {
                OnDemandOverlayAction::Keep
            }
        }
        OverlayVisibility::OnDemand => on_demand_overlay_action(state, continuous),
    }
}

fn orchestrator_state_label(state: OrchestratorState) -> &'static str {
    match state {
        OrchestratorState::Idle => "idle",
        OrchestratorState::Listening => "listening",
        OrchestratorState::Transcribing => "transcribing",
        OrchestratorState::Processing => "processing",
        OrchestratorState::Preview => "preview",
        OrchestratorState::Sending => "sending",
        OrchestratorState::Success => "success",
        OrchestratorState::Error => "error",
    }
}

/// OnDemand 瞬时提示只在窗口本就会藏时 peek；Show/Keep 时 peek 会 vis_gen 误藏会话 UI。
fn should_peek_on_demand_notice(action: OnDemandOverlayAction) -> bool {
    matches!(
        action,
        OnDemandOverlayAction::Hide | OnDemandOverlayAction::HideSuccessAfterDwell
    )
}

pub(crate) fn current_overlay_action(
    app: &AppHandle,
) -> Option<(OverlayVisibility, OnDemandOverlayAction)> {
    let shared = app.try_state::<SharedState>()?;
    let visibility = shared.settings.read().unwrap().overlay.visibility;
    let continuous = shared.orchestrator.continuous_session();
    let state = orchestrator_state_label(shared.orchestrator.state());
    let running = app
        .try_state::<RuntimeManager>()
        .map(|rt| rt.phase() == RuntimePhase::Running)
        .unwrap_or(false);
    Some((
        visibility,
        overlay_window_action(visibility, state, continuous, running),
    ))
}

pub(crate) fn record_overlay_event(activity: &str, reason: &str, fullscreen: Option<bool>) {
    let fs = match fullscreen {
        Some(true) => "true",
        Some(false) => "false",
        None => "unknown",
    };
    process_log::record_activity(
        activity,
        process_log::EventData {
            detail: Some(format!("reason={reason} fullscreen={fs}")),
            ..Default::default()
        },
    );
}

pub(crate) fn peek_on_demand_overlay(
    app: &AppHandle,
    vis_gen: &Arc<std::sync::atomic::AtomicU64>,
    reason: &'static str,
    hide_after_ms: u64,
) {
    let Some((visibility, action)) = current_overlay_action(app) else {
        return;
    };
    if visibility != OverlayVisibility::OnDemand || !should_peek_on_demand_notice(action) {
        return;
    }
    let fullscreen = fullscreen::is_exclusive_fullscreen_active();
    if fullscreen == Some(true) {
        log::log(&format!(
            "{reason} overlay peek skipped: exclusive fullscreen game is active"
        ));
        record_overlay_event("fullscreen_state", reason, Some(true));
        return;
    }
    if let Some(win) = app.get_webview_window("overlay") {
        show_window_no_focus(&win);
        record_overlay_event("overlay_show", reason, fullscreen);
        let app = app.clone();
        let vis_gen = vis_gen.clone();
        let gen = vis_gen.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(hide_after_ms)).await;
            if vis_gen.load(std::sync::atomic::Ordering::SeqCst) != gen {
                return;
            }
            let handle = app.clone();
            let _ = app.run_on_main_thread(move || {
                if vis_gen.load(std::sync::atomic::Ordering::SeqCst) != gen {
                    return;
                }
                let still_peek = current_overlay_action(&handle)
                    .map(|(visibility, action)| {
                        visibility == OverlayVisibility::OnDemand
                            && should_peek_on_demand_notice(action)
                    })
                    .unwrap_or(false);
                if !still_peek {
                    return;
                }
                if let Some(win) = handle.get_webview_window("overlay") {
                    hide_window(&win);
                    record_overlay_event(
                        "overlay_hide",
                        reason,
                        fullscreen::is_exclusive_fullscreen_active(),
                    );
                }
            });
        });
    }
}

#[cfg(test)]
mod overlay_visibility_tests {
    use super::{
        on_demand_overlay_action, orchestrator_state_label, overlay_window_action,
        should_peek_on_demand_notice, OnDemandOverlayAction, OrchestratorState, OverlayVisibility,
    };

    #[test]
    fn on_demand_error_is_always_shown() {
        assert_eq!(
            on_demand_overlay_action("error", false),
            OnDemandOverlayAction::Show
        );
        assert_eq!(
            on_demand_overlay_action("error", true),
            OnDemandOverlayAction::Show
        );
    }

    #[test]
    fn never_mode_hides_for_every_session_state() {
        for state in [
            "idle",
            "listening",
            "transcribing",
            "processing",
            "preview",
            "sending",
            "success",
            "error",
        ] {
            assert_eq!(
                overlay_window_action(OverlayVisibility::Never, state, false, true),
                OnDemandOverlayAction::Hide,
                "state={state}"
            );
        }
    }

    #[test]
    fn pp_status_peeks_only_when_overlay_would_hide() {
        assert!(should_peek_on_demand_notice(OnDemandOverlayAction::Hide));
        assert!(should_peek_on_demand_notice(
            OnDemandOverlayAction::HideSuccessAfterDwell
        ));
        assert!(!should_peek_on_demand_notice(OnDemandOverlayAction::Show));
        assert!(!should_peek_on_demand_notice(OnDemandOverlayAction::Keep));
    }

    #[test]
    fn orchestrator_state_label_matches_overlay_keys() {
        assert_eq!(orchestrator_state_label(OrchestratorState::Idle), "idle");
        assert_eq!(
            orchestrator_state_label(OrchestratorState::Listening),
            "listening"
        );
        assert_eq!(orchestrator_state_label(OrchestratorState::Error), "error");
        assert_eq!(
            overlay_window_action(
                OverlayVisibility::OnDemand,
                orchestrator_state_label(OrchestratorState::Listening),
                false,
                true
            ),
            OnDemandOverlayAction::Show
        );
    }
}

/// 胶囊样式窗口几何（逻辑像素，SetWindowPos 前按窗口 DPI 换算物理像素）
#[cfg(windows)]
const CAPSULE_LOGICAL_W: i32 = 520;
#[cfg(windows)]
const CAPSULE_LOGICAL_H: i32 = 64;
#[cfg(windows)]
const CARD_LOGICAL_W: i32 = 480;
#[cfg(windows)]
const CARD_LOGICAL_H: i32 = 120;
/// 胶囊底边距屏幕工作区底部的间距（逻辑像素）
#[cfg(windows)]
const CAPSULE_BOTTOM_GAP: i32 = 48;
#[cfg(windows)]
const OVERLAY_EDGE_GAP: i32 = 24;

/// 按 overlay 配置摆放悬浮窗（原始 Win32 SetWindowPos；DPI 感知）。
/// auto 保留样式语义：卡片居中、胶囊靠下；固定预设和拖动后的 custom 覆盖它。
/// 与显隐同原则：不走 tao set_position/set_size，避免与 SW_SHOWNA 路径的状态缓存分叉。
#[cfg(windows)]
fn layout_overlay_window<R: tauri::Runtime>(
    win: &tauri::WebviewWindow<R>,
    overlay: &OverlayConfig,
) {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromPoint, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    use windows::Win32::UI::HiDpi::GetDpiForWindow;
    use windows::Win32::UI::WindowsAndMessaging::{
        SetWindowPos, SET_WINDOW_POS_FLAGS, SWP_NOACTIVATE, SWP_NOZORDER,
    };
    let Ok(hwnd) = win.hwnd() else { return };
    unsafe {
        let dpi = GetDpiForWindow(hwnd);
        let dpi = if dpi == 0 { 96 } else { dpi };
        let scale = |logical: i32| (logical as u32 * dpi).div_ceil(96) as i32;
        let (w, h) = match overlay.style {
            OverlayStyle::Card => (scale(CARD_LOGICAL_W), scale(CARD_LOGICAL_H)),
            OverlayStyle::Capsule => (scale(CAPSULE_LOGICAL_W), scale(CAPSULE_LOGICAL_H)),
        };
        let custom = match (overlay.position, overlay.custom_x, overlay.custom_y) {
            (OverlayPosition::Custom, Some(x), Some(y)) => Some((x, y)),
            _ => None,
        };
        let monitor = if let Some((x, y)) = custom {
            MonitorFromPoint(
                POINT {
                    x: x + w / 2,
                    y: y + h / 2,
                },
                MONITOR_DEFAULTTONEAREST,
            )
        } else {
            MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST)
        };
        let mut mi = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !GetMonitorInfoW(monitor, &mut mi).as_bool() {
            return;
        }
        let wa = mi.rcWork;
        let gap = scale(OVERLAY_EDGE_GAP);
        let left = wa.left + gap;
        let center_x = wa.left + ((wa.right - wa.left) - w) / 2;
        let right = wa.right - w - gap;
        let top = wa.top + gap;
        let center_y = wa.top + ((wa.bottom - wa.top) - h) / 2;
        let bottom = wa.bottom - h - gap;
        let (x, y) = match overlay.position {
            OverlayPosition::Auto => match overlay.style {
                OverlayStyle::Card => (center_x, center_y),
                OverlayStyle::Capsule => (center_x, wa.bottom - h - scale(CAPSULE_BOTTOM_GAP)),
            },
            OverlayPosition::TopLeft => (left, top),
            OverlayPosition::TopCenter => (center_x, top),
            OverlayPosition::TopRight => (right, top),
            OverlayPosition::Center => (center_x, center_y),
            OverlayPosition::BottomLeft => (left, bottom),
            OverlayPosition::BottomCenter => (center_x, bottom),
            OverlayPosition::BottomRight => (right, bottom),
            OverlayPosition::Custom => {
                let (x, y) = custom.unwrap_or((center_x, center_y));
                (
                    x.clamp(wa.left, wa.right - w),
                    y.clamp(wa.top, wa.bottom - h),
                )
            }
        };
        let flags: SET_WINDOW_POS_FLAGS = SWP_NOZORDER | SWP_NOACTIVATE;
        let _ = SetWindowPos(hwnd, None, x, y, w, h, flags);
    }
}

/// 非 Windows：回退 tao 逻辑像素定位（MVP Windows-first，保证可编译即可）
#[cfg(not(windows))]
fn layout_overlay_window<R: tauri::Runtime>(
    win: &tauri::WebviewWindow<R>,
    overlay: &OverlayConfig,
) {
    use tauri::{LogicalSize, PhysicalPosition};
    let (w, h) = match overlay.style {
        OverlayStyle::Card => (CARD_FALLBACK.0, CARD_FALLBACK.1),
        OverlayStyle::Capsule => (520.0, 64.0),
    };
    let _ = win.set_size(LogicalSize::new(w, h));
    if let Ok(Some(monitor)) = win.current_monitor() {
        let wa = monitor.work_area();
        let center_x = wa.position.x + (wa.size.width as i32 - w as i32) / 2;
        let center_y = wa.position.y + (wa.size.height as i32 - h as i32) / 2;
        let (x, y) = match overlay.position {
            OverlayPosition::Custom => (
                overlay.custom_x.unwrap_or(center_x),
                overlay.custom_y.unwrap_or(center_y),
            ),
            OverlayPosition::TopLeft => (wa.position.x + 24, wa.position.y + 24),
            OverlayPosition::TopCenter => (center_x, wa.position.y + 24),
            OverlayPosition::TopRight => (
                wa.position.x + wa.size.width as i32 - w as i32 - 24,
                wa.position.y + 24,
            ),
            OverlayPosition::Center => (center_x, center_y),
            OverlayPosition::BottomLeft => (
                wa.position.x + 24,
                wa.position.y + wa.size.height as i32 - h as i32 - 24,
            ),
            OverlayPosition::BottomCenter => (
                center_x,
                wa.position.y + wa.size.height as i32 - h as i32 - 24,
            ),
            OverlayPosition::BottomRight => (
                wa.position.x + wa.size.width as i32 - w as i32 - 24,
                wa.position.y + wa.size.height as i32 - h as i32 - 24,
            ),
            OverlayPosition::Auto => match overlay.style {
                OverlayStyle::Card => (center_x, center_y),
                OverlayStyle::Capsule => (
                    center_x,
                    wa.position.y + wa.size.height as i32 - h as i32 - 48,
                ),
            },
        };
        let _ = win.set_position(PhysicalPosition::new(x, y));
    }
}

#[cfg(not(windows))]
const CARD_FALLBACK: (f64, f64) = (480.0, 120.0);

pub(crate) fn apply_overlay_window_config<R: tauri::Runtime>(
    win: &tauri::WebviewWindow<R>,
    overlay: &OverlayConfig,
) {
    layout_overlay_window(win, overlay);
    #[cfg(windows)]
    if let Ok(hwnd) = win.hwnd() {
        use windows::Win32::UI::WindowsAndMessaging::{
            GetWindowLongPtrW, GWL_EXSTYLE, WS_EX_TRANSPARENT,
        };
        let click_through =
            unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } & WS_EX_TRANSPARENT.0 as isize != 0;
        if click_through == overlay.click_through {
            return;
        }
    }
    let _ = win.set_ignore_cursor_events(overlay.click_through);
}

#[tauri::command]
pub(crate) fn get_overlay_health(app: AppHandle) -> overlay::OverlayHealth {
    overlay::health(&app)
}

#[tauri::command]
pub(crate) fn get_overlay_state(app: AppHandle) -> Option<serde_json::Value> {
    overlay::state_snapshot(&app)
}

/// 用户拖动悬浮窗后保存物理屏幕坐标；下次启动仍在该位置。
#[tauri::command]
pub(crate) fn save_overlay_position(
    app: AppHandle,
    state: tauri::State<SharedState>,
) -> Result<Settings, String> {
    let win = app
        .get_webview_window("overlay")
        .ok_or_else(|| "悬浮窗不存在".to_string())?;
    let position = win
        .outer_position()
        .map_err(|e| format!("读取悬浮窗位置失败: {e}"))?;
    let updated = state.settings_repository.update(|next| {
        next.overlay.position = OverlayPosition::Custom;
        next.overlay.custom_x = Some(position.x);
        next.overlay.custom_y = Some(position.y);
        Ok(())
    })?;
    use tauri::Emitter as _;
    let _ = app.emit("kotone://overlay-config", &updated.overlay);
    Ok(updated)
}
