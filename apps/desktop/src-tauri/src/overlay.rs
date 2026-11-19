//! Optional presentation must never be a prerequisite for recording or sending.
//!
//! Window creation and failure recovery run outside the speech emitter and COM
//! callbacks. In particular, do not build a WebView from a synchronous command
//! or destroy it from its own ProcessFailed callback on Windows.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;

use kotone_core::settings::OverlayVisibility;
use kotone_core::{log, process_log};
use tauri::{AppHandle, Emitter, Manager};

use crate::SharedState;

#[derive(Default)]
pub struct OverlayManager {
    degraded: AtomicBool,
    reason: Mutex<Option<String>>,
    state: Mutex<Option<serde_json::Value>>,
    revision: AtomicU64,
    generation: AtomicU64,
    destroying: AtomicBool,
    lifecycle: Mutex<()>,
    /// 没准备好提示的代际：新提示到来时旧的定时收起作废
    hint_generation: AtomicU64,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayHealth {
    pub(crate) degraded: bool,
    pub(crate) reason: Option<String>,
}

pub fn health<R: tauri::Runtime>(app: &AppHandle<R>) -> OverlayHealth {
    let manager = app.state::<OverlayManager>();
    let reason = manager
        .reason
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    OverlayHealth {
        degraded: manager.degraded.load(Ordering::SeqCst),
        reason,
    }
}

pub fn remember_state(app: &AppHandle, payload: &serde_json::Value) {
    if let Some(manager) = app.try_state::<OverlayManager>() {
        *manager.state.lock().unwrap_or_else(|e| e.into_inner()) = Some(payload.clone());
    }
}

pub fn state_snapshot(app: &AppHandle) -> Option<serde_json::Value> {
    app.state::<OverlayManager>()
        .state
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

pub fn can_show<R: tauri::Runtime>(app: &AppHandle<R>) -> bool {
    !app.state::<OverlayManager>()
        .degraded
        .load(Ordering::SeqCst)
        && app
            .state::<SharedState>()
            .settings
            .read()
            .unwrap()
            .overlay
            .visibility
            != OverlayVisibility::Never
}

/// Called inside the settings transaction, before publishing an explicit
/// enable, so an older recovery worker cannot overwrite that newer choice.
pub fn prepare_enable(app: &AppHandle) {
    let manager = app.state::<OverlayManager>();
    manager.degraded.store(false, Ordering::SeqCst);
    manager.revision.fetch_add(1, Ordering::SeqCst);
    *manager.reason.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// Reconcile a configuration change on a worker. Disabled overlays have no
/// native window or WebView, including on the next launch.
pub fn reconcile(app: &AppHandle, explicitly_enabled: bool) {
    let manager = app.state::<OverlayManager>();
    manager.revision.fetch_add(1, Ordering::SeqCst);
    if explicitly_enabled {
        // prepare_enable already cleared the old failure inside the settings
        // transaction. Never clear a newer failure arriving after that commit.
        let _ = app.emit("kotone://overlay-degraded", health(app));
    }
    let app = app.clone();
    std::thread::spawn(move || {
        let manager = app.state::<OverlayManager>();
        let _lifecycle = manager.lifecycle.lock().unwrap_or_else(|e| e.into_inner());
        let revision = manager.revision.load(Ordering::SeqCst);
        if !can_show(&app) {
            destroy_window(&app);
            return;
        }
        let result = (|| -> Result<(), String> {
            if manager.destroying.load(Ordering::SeqCst) && !wait_for_removal(&app) {
                return Err("previous overlay window has not finished closing".into());
            }
            let win = if let Some(win) = app.get_webview_window("overlay") {
                win
            } else {
                let config = app
                    .config()
                    .app
                    .windows
                    .iter()
                    .find(|config| config.label == "overlay")
                    .ok_or_else(|| "missing overlay window configuration".to_string())?;
                let win = tauri::WebviewWindowBuilder::from_config(&app, config)
                    .map_err(|e| e.to_string())?
                    .focused(false)
                    .build()
                    .map_err(|e| e.to_string())?;
                crate::harden_webview(&win);
                let generation = manager.generation.fetch_add(1, Ordering::SeqCst) + 1;
                watch_failures(&win, generation);
                // Native visibility starts hidden; synchronize WebView2 too.
                crate::hide_window(&win);
                win
            };
            // A user may turn the overlay off while WebView2 is being created.
            if revision != manager.revision.load(Ordering::SeqCst) || !can_show(&app) {
                if !can_show(&app) {
                    destroy_window(&app);
                }
                return Ok(());
            }
            let handle = app.clone();
            let _ = app.run_on_main_thread(move || {
                if !can_show(&handle) {
                    return;
                }
                let config = handle
                    .state::<SharedState>()
                    .settings
                    .read()
                    .unwrap()
                    .overlay
                    .clone();
                crate::apply_overlay_window_config(&win, &config);
                if let Some((visibility, action)) = crate::current_overlay_action(&handle) {
                    let show = action == crate::OnDemandOverlayAction::Show
                        || (visibility == OverlayVisibility::Always
                            && handle.state::<crate::RuntimeManager>().phase()
                                == kotone_core::runtime::RuntimePhase::Running);
                    if show
                        && kotone_platform_windows::fullscreen::is_exclusive_fullscreen_active()
                            != Some(true)
                    {
                        crate::show_window_no_focus(&win);
                    } else {
                        crate::hide_window(&win);
                    }
                }
            });
            Ok(())
        })();
        if let Err(error) = result {
            log::log(&format!("overlay window unavailable: {error}"));
            degrade(&app, "window_creation_failed");
        }
    });
}

fn destroy_window<R: tauri::Runtime>(app: &AppHandle<R>) {
    let handle = app.clone();
    let (done, receiver) = std::sync::mpsc::sync_channel(1);
    if app
        .run_on_main_thread(move || {
            if can_show(&handle) {
                let _ = done.send(false);
                return;
            }
            if let Some(win) = handle.get_webview_window("overlay") {
                handle
                    .state::<OverlayManager>()
                    .generation
                    .fetch_add(1, Ordering::SeqCst);
                // destroy bypasses the application's close-to-tray handler.
                if win.destroy().is_ok() {
                    handle
                        .state::<OverlayManager>()
                        .destroying
                        .store(true, Ordering::SeqCst);
                    let _ = done.send(true);
                    return;
                }
            }
            let _ = done.send(false);
        })
        .is_ok()
    {
        // Only a presentation worker waits; the speech pipeline never does.
        if receiver.recv_timeout(std::time::Duration::from_secs(2)) == Ok(true) {
            // destroy() posts a separate runtime event even on the UI thread.
            // Do not let an enable reuse the window awaiting that event.
            let _ = wait_for_removal(app);
        }
    }
}

fn wait_for_removal<R: tauri::Runtime>(app: &AppHandle<R>) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        if app.get_webview_window("overlay").is_none() {
            app.state::<OverlayManager>()
                .destroying
                .store(false, Ordering::SeqCst);
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

pub fn degrade<R: tauri::Runtime>(app: &AppHandle<R>, code: &'static str) {
    let manager = app.state::<OverlayManager>();
    if manager.degraded.swap(true, Ordering::SeqCst) {
        return;
    }
    manager.revision.fetch_add(1, Ordering::SeqCst);
    *manager.reason.lock().unwrap_or_else(|e| e.into_inner()) =
        Some("悬浮窗显示异常，已自动关闭。语音输入仍可使用。".into());
    // Always leave the COM callback before logging, saving or destroying.
    let app = app.clone();
    std::thread::spawn(move || {
        let manager = app.state::<OverlayManager>();
        if !manager.degraded.load(Ordering::SeqCst) {
            return;
        }
        log::log(&format!(
            "overlay degraded: {code}; voice input remains available"
        ));
        process_log::record_activity(
            "overlay_degraded",
            process_log::EventData {
                error_code: Some(code.into()),
                ..Default::default()
            },
        );
        if let Err(error) = app
            .state::<SharedState>()
            .settings_repository
            .update(|settings| {
                apply_fallback(settings, manager.degraded.load(Ordering::SeqCst));
                Ok(())
            })
        {
            log::log(&format!("persist overlay fallback failed: {error}"));
        }
        let manager = app.state::<OverlayManager>();
        let _lifecycle = manager.lifecycle.lock().unwrap_or_else(|e| e.into_inner());
        if !manager.degraded.load(Ordering::SeqCst) {
            return;
        }
        destroy_window(&app);
        let _ = app.emit("kotone://overlay-degraded", health(&app));
    });
}

fn apply_fallback(settings: &mut kotone_core::settings::Settings, degraded: bool) {
    if degraded {
        settings.overlay.visibility = OverlayVisibility::Never;
    }
}

#[cfg(windows)]
fn watch_failures(win: &tauri::WebviewWindow, generation: u64) {
    let app = win.app_handle().clone();
    let fallback = app.clone();
    if win
        .with_webview(move |webview| unsafe {
            use webview2_com::{Microsoft::Web::WebView2::Win32::*, ProcessFailedEventHandler};
            let register = (|| -> windows::core::Result<()> {
                let core = webview.controller().CoreWebView2()?;
                let handle = app.clone();
                let callback = ProcessFailedEventHandler::create(Box::new(move |_, args| {
                    if let Some(args) = args {
                        let mut kind = COREWEBVIEW2_PROCESS_FAILED_KIND::default();
                        args.ProcessFailedKind(&mut kind)?;
                        if failure_requires_fallback(kind.0)
                            && handle
                                .state::<OverlayManager>()
                                .generation
                                .load(Ordering::SeqCst)
                                == generation
                        {
                            let code = match kind.0 {
                                0 => "webview_browser_process_exited",
                                1 => "webview_renderer_process_exited",
                                2 => "webview_renderer_unresponsive",
                                _ => "webview_gpu_process_exited",
                            };
                            degrade(&handle, code);
                        }
                    }
                    Ok(())
                }));
                let mut token = 0;
                core.add_ProcessFailed(&callback, &mut token)?;
                // WebView owns its handler and releases it when destroyed.
                Ok(())
            })();
            if register.is_err()
                && can_show(&app)
                && app
                    .state::<OverlayManager>()
                    .generation
                    .load(Ordering::SeqCst)
                    == generation
            {
                degrade(&app, "webview_monitor_failed");
            }
        })
        .is_err()
        && can_show(&fallback)
        && fallback
            .state::<OverlayManager>()
            .generation
            .load(Ordering::SeqCst)
            == generation
    {
        degrade(&fallback, "webview_monitor_failed");
    }
}

#[cfg(not(windows))]
fn watch_failures(_: &tauri::WebviewWindow, _: u64) {}

/// Keep the native host and WebView2 controller in the same visibility state.
/// Actual-state checks make repeated state events idempotent. with_webview
/// queues this work onto the UI thread instead of blocking the speech emitter.
#[cfg(windows)]
pub fn set_visible<R: tauri::Runtime>(win: &tauri::WebviewWindow<R>, visible: bool) {
    let window = win.clone();
    let fallback = win.app_handle().clone();
    let generation = win
        .app_handle()
        .try_state::<OverlayManager>()
        .map(|manager| manager.generation.load(Ordering::SeqCst));
    if win
        .with_webview(move |webview| unsafe {
            use windows::core::BOOL;
            use windows::Win32::UI::WindowsAndMessaging::{
                IsWindowVisible, ShowWindow, SW_HIDE, SW_SHOWNA,
            };
            if visible && window.label() == "overlay" && !can_show(window.app_handle()) {
                return;
            }
            if visible
                && window.label() == "overlay"
                && kotone_platform_windows::fullscreen::is_exclusive_fullscreen_active()
                    == Some(true)
            {
                return;
            }
            let result = (|| -> windows::core::Result<bool> {
                let hwnd = window
                    .hwnd()
                    .map_err(|_| windows::core::Error::from_win32())?;
                let mut controller_visible = BOOL::default();
                webview.controller().IsVisible(&mut controller_visible)?;
                if IsWindowVisible(hwnd).as_bool() == visible
                    && controller_visible.as_bool() == visible
                {
                    return Ok(false);
                }
                if visible {
                    webview.controller().SetIsVisible(true)?;
                    let _ = ShowWindow(hwnd, SW_SHOWNA);
                } else {
                    let _ = ShowWindow(hwnd, SW_HIDE);
                    webview.controller().SetIsVisible(false)?;
                }
                Ok(true)
            })();
            match result {
                Ok(true) if window.label() == "overlay" => {
                    // 不能在 with_webview 回调里直接发事件：在主线程上调用时（例如成功提示停留后
                    // 隐藏悬浮窗），回调执行期间 Tauri 一直占着这个窗口的内部锁，emit 要再拿同一把锁，
                    // 主线程就把自己锁死了，整个应用随之卡住。换个线程发，等回调结束、锁放开后送达。
                    let notify = window.clone();
                    std::thread::spawn(move || {
                        let _ = notify.emit_to("overlay", "kotone://overlay-visibility", visible);
                    });
                }
                Err(_)
                    if window.label() == "overlay"
                        && can_show(window.app_handle())
                        && generation
                            == window
                                .app_handle()
                                .try_state::<OverlayManager>()
                                .map(|manager| manager.generation.load(Ordering::SeqCst)) =>
                {
                    degrade(window.app_handle(), "webview_visibility_failed")
                }
                _ => {}
            }
        })
        .is_err()
        && win.label() == "overlay"
        && can_show(&fallback)
        && generation
            == fallback
                .try_state::<OverlayManager>()
                .map(|manager| manager.generation.load(Ordering::SeqCst))
    {
        degrade(&fallback, "webview_visibility_failed");
    }
}

// WebView2 browser, main renderer, unresponsive renderer or GPU process.
fn failure_requires_fallback(kind: i32) -> bool {
    matches!(kind, 0 | 1 | 2 | 6)
}

#[cfg(test)]
mod tests {
    use super::{apply_fallback, failure_requires_fallback};
    use kotone_core::settings::{OverlayVisibility, Settings};

    #[test]
    fn core_render_failures_disable_optional_overlay() {
        for kind in [0, 1, 2, 6] {
            assert!(failure_requires_fallback(kind));
        }
        // An unrelated subframe or utility exit is recoverable and must not
        // disable the user's overlay.
        for kind in [3, 4, 5, 7, 8, 9] {
            assert!(!failure_requires_fallback(kind));
        }
    }

    #[test]
    fn presentation_failure_preserves_every_voice_input_setting() {
        let mut settings = Settings::default();
        settings.hotkey.key = "Insert".into();
        settings.overlay.visibility = OverlayVisibility::Always;
        let mut expected = settings.clone();
        expected.overlay.visibility = OverlayVisibility::Never;
        apply_fallback(&mut settings, true);
        assert_eq!(
            serde_json::to_value(settings).unwrap(),
            serde_json::to_value(expected).unwrap()
        );
    }

    #[test]
    fn cancelled_recovery_cannot_override_explicit_enable() {
        let mut settings = Settings::default();
        settings.overlay.visibility = OverlayVisibility::Always;
        apply_fallback(&mut settings, false);
        assert_eq!(settings.overlay.visibility, OverlayVisibility::Always);
    }
}

/// 悬浮窗上的一条提示（`kotone://overlay-hint`）；`null` = 收起。
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayHint {
    pub title: String,
    pub detail: String,
}

const HINT_DWELL: std::time::Duration = std::time::Duration::from_millis(3_500);

/// 语音识别没准备好时按了语音输入热键：在悬浮窗提示怎么启动（ADR 028，D8）。
/// 悬浮窗只属于语音输入；关了悬浮窗（never）、悬浮窗降级或游戏独占全屏时不显示。
/// 约 3.5 秒后收起；期间语音识别已就绪（Running）就只撤掉提示，窗口交给平常的显隐规则。
pub fn show_hint(app: &AppHandle, title: &str, detail: &str) {
    if !can_show(app) {
        return;
    }
    if kotone_platform_windows::fullscreen::is_exclusive_fullscreen_active() == Some(true) {
        log::log("overlay hint skipped: exclusive fullscreen game is active");
        return;
    }
    let Some(win) = app.get_webview_window("overlay") else {
        return;
    };
    let generation = app
        .state::<OverlayManager>()
        .hint_generation
        .fetch_add(1, Ordering::SeqCst)
        + 1;
    let _ = app.emit_to(
        "overlay",
        "kotone://overlay-hint",
        Some(OverlayHint {
            title: title.to_string(),
            detail: detail.to_string(),
        }),
    );
    crate::show_window_no_focus(&win);
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(HINT_DWELL).await;
        if app
            .state::<OverlayManager>()
            .hint_generation
            .load(Ordering::SeqCst)
            != generation
        {
            return;
        }
        let _ = app.emit_to("overlay", "kotone://overlay-hint", None::<OverlayHint>);
        let running = app.state::<crate::RuntimeManager>().phase()
            == kotone_core::runtime::RuntimePhase::Running;
        if running {
            return;
        }
        let handle = app.clone();
        let _ = app.run_on_main_thread(move || {
            let still_stopped = handle.state::<crate::RuntimeManager>().phase()
                != kotone_core::runtime::RuntimePhase::Running;
            if still_stopped
                && handle
                    .state::<OverlayManager>()
                    .hint_generation
                    .load(Ordering::SeqCst)
                    == generation
            {
                if let Some(win) = handle.get_webview_window("overlay") {
                    crate::hide_window(&win);
                }
            }
        });
    });
}
