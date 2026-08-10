//! Kotone Rust 核心：模块组装、共享状态与 IPC 命令。
//! 职责划分见 docs/development.md §5.1；IPC 契约见 §5.3（类型对齐 src/lib/ipc.ts）。

mod diagnostics;
mod hotkey;
mod runtime;
mod tray;

use std::sync::{Arc, Mutex, RwLock};

use tauri::{AppHandle, Emitter as TauriEventEmitter, Manager};

use hotkey::{HotkeyManager, HotkeyStatus, InputEnvironmentCheck};
use kotone_core::audio::AudioDevice;
use kotone_core::inject::{CancelToken, FocusBackend, InjectError, Injector};
use kotone_core::interaction::{effective_hotkey_mode, InteractionPolicy};
use kotone_core::orchestrator::{Emitter, Orchestrator};
use kotone_core::profile::{
    self, format_hotwords_export, merge_hotwords, parse_hotwords_import, GameProfile,
    HotwordMergeReport, ProfileDeleteOutcome,
};
use kotone_core::runtime::RuntimePhase;
use kotone_core::settings::{
    self, OverlayConfig, OverlayPosition, OverlayStyle, OverlayVisibility, Settings,
};
use kotone_core::stt::{EngineInfo, EngineRegistry};
use kotone_core::{log, process_log};
use kotone_platform_windows::inject::{WinFocusBackend, WindowsInjector};
use kotone_platform_windows::{audio as platform_audio, elevation, inject as platform_inject};
use kotone_stt::model;
use runtime::{RuntimeManager, RuntimeStatus};

/// 全局共享状态：settings 双端共享，orchestrator 是唯一业务状态所有者
pub struct SharedState {
    pub settings: Arc<RwLock<Settings>>,
    pub orchestrator: Arc<Orchestrator>,
    pub engines: Arc<EngineRegistry>,
    pub injector: Arc<dyn Injector>,
}

/// 启动时如何处理新手向导。
///
/// - auto：仅配置中尚未完成时显示（正式用户默认）
/// - always：本次强制显示，但不重置持久化完成标记（回归测试）
/// - never：本次强制跳过（自动化/故障排查）
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
enum OnboardingLaunchMode {
    Auto,
    Always,
    Never,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct StartupOptions {
    onboarding: OnboardingLaunchMode,
}

fn parse_onboarding_launch_mode<I, S>(args: I) -> OnboardingLaunchMode
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut mode = OnboardingLaunchMode::Auto;
    let mut expect_value = false;
    for arg in args {
        let arg = arg.as_ref();
        let value = if expect_value {
            expect_value = false;
            Some(arg)
        } else if arg == "--onboarding" {
            expect_value = true;
            None
        } else {
            arg.strip_prefix("--onboarding=")
        };
        mode = match value {
            Some("always") => OnboardingLaunchMode::Always,
            Some("never") => OnboardingLaunchMode::Never,
            Some("auto") => OnboardingLaunchMode::Auto,
            _ => mode,
        };
    }
    mode
}

#[tauri::command]
fn get_startup_options(options: tauri::State<StartupOptions>) -> StartupOptions {
    options.inner().clone()
}

#[cfg(test)]
mod startup_options_tests {
    use super::{parse_onboarding_launch_mode, OnboardingLaunchMode};

    #[test]
    fn onboarding_mode_defaults_to_auto() {
        assert_eq!(
            parse_onboarding_launch_mode(["kotone.exe"]),
            OnboardingLaunchMode::Auto
        );
    }

    #[test]
    fn onboarding_mode_accepts_equals_and_split_forms() {
        assert_eq!(
            parse_onboarding_launch_mode(["kotone.exe", "--onboarding=always"]),
            OnboardingLaunchMode::Always
        );
        assert_eq!(
            parse_onboarding_launch_mode(["kotone.exe", "--onboarding", "never"]),
            OnboardingLaunchMode::Never
        );
    }

    #[test]
    fn invalid_onboarding_mode_keeps_last_valid_value() {
        assert_eq!(
            parse_onboarding_launch_mode([
                "kotone.exe",
                "--onboarding=always",
                "--onboarding=invalid",
            ]),
            OnboardingLaunchMode::Always
        );
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OnDemandOverlayAction {
    Show,
    Hide,
    HideSuccessAfterDwell,
    Keep,
}

fn on_demand_overlay_action(state: &str, continuous: bool) -> OnDemandOverlayAction {
    match state {
        "listening" | "transcribing" | "preview" | "sending" | "error" => {
            OnDemandOverlayAction::Show
        }
        "success" if !continuous => OnDemandOverlayAction::HideSuccessAfterDwell,
        "idle" => OnDemandOverlayAction::Hide,
        _ => OnDemandOverlayAction::Keep,
    }
}

#[cfg(test)]
mod overlay_visibility_tests {
    use super::{on_demand_overlay_action, OnDemandOverlayAction};

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
}

/// 生产事件出口：转发为 Tauri 事件；联动 Esc 取消键注册与 overlay 窗口显隐。
/// overlay 显隐规则（后端驱动，幂等，与前端逻辑不冲突）按 `overlay.visibility` 分档：
/// - always（常驻，默认）：会话态（Listening/…/Error）→ show；Running 期间 idle 不隐藏
///   （悬浮窗兼作运行指示）；Stopped 由 stop_runtime 显式隐藏。
/// - on_demand（用时浮现）：平时隐藏；Listening/Transcribing/Preview/Sending → show；
///   Success 延迟 ~600ms 自动隐藏（vis_gen 代际防新会话误藏）；
///   Error 无条件显示并保持到用户关闭/重试，避免核心链路失败只进日志；
///   solo 连续模式保持显示直到会话停止（idle → hide）。
struct TauriEmitter {
    app: AppHandle,
    /// overlay 显隐代际：每个状态事件 +1；延迟隐藏任务只认调度时的代际
    vis_gen: Arc<std::sync::atomic::AtomicU64>,
}

impl Emitter for TauriEmitter {
    fn emit(&self, event: &str, payload: serde_json::Value) {
        use tauri::Emitter as _;
        let _ = self.app.emit(event, payload.clone());
        if event == "kotone://state" {
            let state = payload.get("state").and_then(|s| s.as_str()).unwrap_or("");
            let text_chars = payload
                .pointer("/payload/text")
                .and_then(|v| v.as_str())
                .map(|text| text.chars().count())
                .unwrap_or(0);
            let has_error = payload.pointer("/payload/message").is_some();
            log::log(&format!(
                "state -> {state} text_chars={text_chars} has_error={has_error}"
            ));
            // 会话激活期间（含 Preview）临时注册 Esc 全局取消键，回 Idle 即注销。
            // Preview 态同样需要 Esc：overlay 不抢焦点，Esc 是预览确认的主要键盘出口。
            if let Some(mgr) = self.app.try_state::<HotkeyManager>() {
                mgr.set_cancel_enabled(&self.app, state != "idle" && !state.is_empty());
            }
            // 后端驱动 overlay 显隐（显隐一律走原始 Win32 SW_SHOWNA/SW_HIDE 路径，
            // 不碰 tao set_visible——tao 缓存 diff 短路坑，见 hide_window 注释）
            if let Some(win) = self.app.get_webview_window("overlay") {
                let (visibility, continuous) = self
                    .app
                    .try_state::<SharedState>()
                    .map(|s| {
                        let g = s.settings.read().unwrap();
                        (
                            g.overlay.visibility,
                            InteractionPolicy::from_settings(&g).continuous,
                        )
                    })
                    .unwrap_or((OverlayVisibility::Always, false));
                let gen = self
                    .vis_gen
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                    + 1;
                match visibility {
                    OverlayVisibility::Always => {
                        let running = self
                            .app
                            .try_state::<RuntimeManager>()
                            .map(|rt| rt.phase() == RuntimePhase::Running)
                            .unwrap_or(false);
                        if state == "idle" && !running {
                            hide_window(&win);
                        } else if state != "idle" {
                            show_window_no_focus(&win);
                        }
                    }
                    OverlayVisibility::OnDemand => {
                        match on_demand_overlay_action(state, continuous) {
                            OnDemandOverlayAction::Show => {
                                // Error 可能从 Idle 直接到达（例如音频设备打开失败），不能
                                // 假设 Listening 已经显示过窗口；它会保持到用户明确确认。
                                show_window_no_focus(&win);
                            }
                            OnDemandOverlayAction::HideSuccessAfterDwell => {
                                let app = self.app.clone();
                                let vis_gen = self.vis_gen.clone();
                                tauri::async_runtime::spawn(async move {
                                    tokio::time::sleep(std::time::Duration::from_millis(600)).await;
                                    if vis_gen.load(std::sync::atomic::Ordering::SeqCst) != gen {
                                        return; // 600ms 内已有新状态事件（新会话/停止），不藏
                                    }
                                    if let Some(win) = app.get_webview_window("overlay") {
                                        hide_window(&win);
                                    }
                                });
                            }
                            OnDemandOverlayAction::Hide => hide_window(&win),
                            // continuous（solo）的 success：会话未停，保持显示
                            OnDemandOverlayAction::Keep => {}
                        }
                    }
                }
            }
        } else if event == "kotone://channel" {
            // 频道切换（ADR-008）：on_demand 模式下悬浮窗平时隐藏，
            // 切换瞬间需要「露个脸」让用户看到频道徽标，~1.2s 后自动收回；
            // always 模式运行期间本就常显，无需处理。vis_gen 代际防止
            // 紧随其后的新会话被这次延迟隐藏误伤。
            let on_demand = self
                .app
                .try_state::<SharedState>()
                .map(|s| {
                    s.settings.read().unwrap().overlay.visibility == OverlayVisibility::OnDemand
                })
                .unwrap_or(false);
            if on_demand {
                if let Some(win) = self.app.get_webview_window("overlay") {
                    show_window_no_focus(&win);
                    let app = self.app.clone();
                    let vis_gen = self.vis_gen.clone();
                    let gen = self
                        .vis_gen
                        .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                        + 1;
                    tauri::async_runtime::spawn(async move {
                        tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
                        if vis_gen.load(std::sync::atomic::Ordering::SeqCst) != gen {
                            return; // 期间已有新状态事件（新会话/再切换），不藏
                        }
                        if let Some(win) = app.get_webview_window("overlay") {
                            hide_window(&win);
                        }
                    });
                }
            }
        } else if event == "kotone://process" {
            record_process_event(&self.app, &payload);
        }
    }
}

fn record_process_event(app: &AppHandle, payload: &serde_json::Value) {
    let Some(case_id) = payload.get("caseId").and_then(|v| v.as_str()) else {
        return;
    };
    let Some(activity) = payload.get("activity").and_then(|v| v.as_str()) else {
        return;
    };
    let mut event = process_log::ProcessEvent::new(case_id, activity);
    if let Some(state) = app.try_state::<SharedState>() {
        let settings = state.settings.read().unwrap();
        event.context.engine_id = Some(settings.stt_engine.clone());
        event.context.model_id = Some(model::active_model_from(&settings, &settings.stt_engine));
        event.context.profile_id = settings.active_profile_id.clone();
        event.context.interaction_mode = settings.interaction_mode.as_ref().map(|mode| {
            serde_json::to_string(mode)
                .unwrap_or_default()
                .trim_matches('"')
                .to_string()
        });
    }
    event.context.elevated = Some(elevation::is_elevated());
    if let Some(data) = payload.get("data") {
        event.data.outcome = data
            .get("outcome")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        event.data.error_code = data
            .get("errorCode")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        event.data.duration_ms = data.get("durationMs").and_then(|v| v.as_u64());
        event.data.audio_ms = data.get("audioMs").and_then(|v| v.as_u64());
        event.data.text_chars = data.get("textChars").and_then(|v| v.as_u64());
    }
    if let Err(error) = process_log::record(event) {
        log::log(&format!("process event write failed: {error}"));
    }
}

/// 显示窗口但不抢焦点（焦点必须留在游戏/目标窗口，否则注入会打错窗口）。
/// Windows 上用 SW_SHOWNA；其他平台回退普通 show。
///
/// 注意：本函数绕开 tao 的 window_flags 缓存直接操作 Win32 状态，因此隐藏必须
/// 走对称的 `hide_window`（见下），不能混用 Tauri 的 hide()/show()。
#[cfg(windows)]
fn show_window_no_focus<R: tauri::Runtime>(win: &tauri::WebviewWindow<R>) {
    use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_SHOWNA};
    match win.hwnd() {
        Ok(hwnd) => unsafe {
            let _ = ShowWindow(hwnd, SW_SHOWNA);
        },
        Err(_) => {
            let _ = win.show();
        }
    }
}

/// 隐藏窗口（与 show_window_no_focus 对称的原始 Win32 路径）。
///
/// 为什么不能用 Tauri 的 hide()：窗口经 SW_SHOWNA 显示后，tao 的 window_flags
/// 缓存仍停留在「不可见」（创建时 visible:false）；tao 的 set_visible(false)
/// 走 apply_diff，对缓存新旧值做 diff——diff 为空则提前返回，根本到不了
/// ShowWindow(SW_HIDE)（tao 0.35 window_state.rs）。这正是「停止后悬浮窗
/// 不隐藏」的根因：显隐机制不对称，与 runtime 编排无关。
#[cfg(windows)]
fn hide_window<R: tauri::Runtime>(win: &tauri::WebviewWindow<R>) {
    use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE};
    match win.hwnd() {
        Ok(hwnd) => unsafe {
            let _ = ShowWindow(hwnd, SW_HIDE);
        },
        Err(_) => {
            let _ = win.hide();
        }
    }
}

#[cfg(not(windows))]
fn show_window_no_focus<R: tauri::Runtime>(win: &tauri::WebviewWindow<R>) {
    let _ = win.show();
}

#[cfg(not(windows))]
fn hide_window<R: tauri::Runtime>(win: &tauri::WebviewWindow<R>) {
    let _ = win.hide();
}

/// 把 WebView2 收敛为桌面应用壳：关闭右键浏览器菜单、开发者工具和浏览器快捷键。
///
/// 前端还有 capture 阶段的快捷键拦截作为跨平台兜底；Windows 侧必须从
/// WebView2 Settings 关闭 accelerator keys，才能在 DOM 收到事件前拦住
/// Ctrl+F、F5、Ctrl+Shift+R 等浏览器行为。
#[cfg(windows)]
fn harden_webview<R: tauri::Runtime>(win: &tauri::WebviewWindow<R>) {
    let label = win.label().to_string();
    let _ = win.with_webview(move |webview| unsafe {
        use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Settings3;
        use windows::core::Interface;

        let result = (|| -> windows::core::Result<()> {
            let core = webview.controller().CoreWebView2()?;
            let settings = core.Settings()?;
            settings.SetAreDefaultContextMenusEnabled(false)?;
            settings.SetAreDevToolsEnabled(false)?;
            let settings3: ICoreWebView2Settings3 = settings.cast()?;
            settings3.SetAreBrowserAcceleratorKeysEnabled(false)?;
            Ok(())
        })();
        if let Err(error) = result {
            log::log(&format!("webview hardening failed ({label}): {error}"));
        }
    });
}

#[cfg(not(windows))]
fn harden_webview<R: tauri::Runtime>(_win: &tauri::WebviewWindow<R>) {}

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
        let mut mi = MONITORINFO::default();
        mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
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

fn apply_overlay_window_config<R: tauri::Runtime>(
    win: &tauri::WebviewWindow<R>,
    overlay: &OverlayConfig,
) {
    layout_overlay_window(win, overlay);
    let _ = win.set_ignore_cursor_events(overlay.click_through);
}

/// 冒烟测试命令：前端可 invoke("ping") 验证 IPC 通路
#[tauri::command]
fn ping() -> &'static str {
    "pong"
}

/// 导出不含识别文本、音频和热词的诊断 ZIP。
#[tauri::command]
fn export_diagnostics(
    app: AppHandle,
    path: String,
) -> Result<diagnostics::DiagnosticExportResult, String> {
    diagnostics::export(&app, std::path::Path::new(&path))
}

/// 前端全局异常与更新器错误落入持久日志。消息先做主目录脱敏并限制长度。
#[tauri::command]
fn log_frontend_error(context: String, message: String) {
    let context: String = context
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
        .take(64)
        .collect();
    let message = diagnostics::redact_home(&message.replace('\r', " ").replace('\n', " "));
    log::log(&format!("frontend error [{context}]: {message}"));
}

// ---------- 设置与配置（§5.3） ----------

#[tauri::command]
fn get_settings(state: tauri::State<SharedState>) -> Settings {
    state.settings.read().unwrap().clone()
}

/// 局部更新配置；热键变化时触发重注册
#[tauri::command]
fn update_settings(
    app: AppHandle,
    state: tauri::State<SharedState>,
    patch: serde_json::Value,
) -> Result<Settings, String> {
    let (old_hotkey, old_overlay, updated) = {
        let mut guard = state.settings.write().unwrap();
        let old_hotkey = (
            guard.hotkey.key.clone(),
            effective_hotkey_mode(&guard),
            guard.hotkey_backend,
            guard.channel_cycle_hotkey.clone(),
            guard.resend_last_hotkey.clone(),
        );
        let old_overlay = guard.overlay.clone();
        let mut merged =
            serde_json::to_value(&*guard).map_err(|e| format!("序列化配置失败: {e}"))?;
        settings::merge_json(&mut merged, &patch);
        let mut next: Settings =
            serde_json::from_value(merged).map_err(|e| format!("配置项不合法: {e}"))?;
        next.overlay.normalize_interaction();
        *guard = next.clone();
        (old_hotkey, old_overlay, next)
    };
    settings::save(&updated)?;

    // overlay 配置变化 → 立即重排几何/点击穿透 + 通知前端（无需重启）
    if old_overlay != updated.overlay {
        if let Some(win) = app.get_webview_window("overlay") {
            apply_overlay_window_config(&win, &updated.overlay);
        }
        use tauri::Emitter as _;
        let _ = app.emit("kotone://overlay-config", &updated.overlay);
    }

    // 热键键位/生效模式/后端变化 → 重注册。生效模式由 interactionMode 预设推导
    // （effective_hotkey_mode），所以切预设（如 push-to-talk）也会走到这里。
    // 仅 Running 时注册热键：Stopped 语义就是「按热键无反应」，
    // 配置变更会在下次 start_runtime 时生效。
    let next_mode = effective_hotkey_mode(&updated);
    let hotkey_changed = old_hotkey.0 != updated.hotkey.key
        || old_hotkey.1 != next_mode
        || old_hotkey.2 != updated.hotkey_backend
        // 频道切换热键（ADR-008）变化也要重注册（HotkeyManager 注册时一并应用）
        || old_hotkey.3 != updated.channel_cycle_hotkey
        // 重发最近一条热键变化也要重注册
        || old_hotkey.4 != updated.resend_last_hotkey;
    let running = app
        .try_state::<RuntimeManager>()
        .map(|rt| rt.phase() == RuntimePhase::Running)
        .unwrap_or(false);
    if hotkey_changed && running {
        if let Some(mgr) = app.try_state::<HotkeyManager>() {
            mgr.register(&app, &updated.hotkey.key, next_mode)?;
        }
    }
    // 引擎/模型/模式可能经 patch 变更 → restartNeeded 推导依赖最新配置，推送全量状态
    runtime::snapshot_and_emit(&app, None);
    Ok(updated)
}

/// 用户拖动悬浮窗后保存物理屏幕坐标；下次启动仍在该位置。
#[tauri::command]
fn save_overlay_position(
    app: AppHandle,
    state: tauri::State<SharedState>,
) -> Result<Settings, String> {
    let win = app
        .get_webview_window("overlay")
        .ok_or_else(|| "悬浮窗不存在".to_string())?;
    let position = win
        .outer_position()
        .map_err(|e| format!("读取悬浮窗位置失败: {e}"))?;
    let updated = {
        let mut guard = state.settings.write().unwrap();
        guard.overlay.position = OverlayPosition::Custom;
        guard.overlay.custom_x = Some(position.x);
        guard.overlay.custom_y = Some(position.y);
        let updated = guard.clone();
        settings::save(&updated)?;
        updated
    };
    use tauri::Emitter as _;
    let _ = app.emit("kotone://overlay-config", &updated.overlay);
    Ok(updated)
}

#[tauri::command]
fn list_audio_devices() -> Vec<AudioDevice> {
    platform_audio::list_devices()
}

#[tauri::command]
fn set_audio_device(state: tauri::State<SharedState>, id: String) -> Result<(), String> {
    let updated = {
        let mut guard = state.settings.write().unwrap();
        guard.audio_device_id = id;
        guard.clone()
    };
    settings::save(&updated)
}

// ---------- STT 引擎（§5.3） ----------

#[tauri::command]
fn list_stt_engines(state: tauri::State<SharedState>) -> Vec<EngineInfo> {
    state.engines.list_info()
}

#[tauri::command]
fn set_stt_engine(
    app: AppHandle,
    state: tauri::State<SharedState>,
    id: String,
) -> Result<(), String> {
    if state.engines.get(&id).is_none() {
        return Err(format!("未注册的 STT 引擎: {id}"));
    }
    let updated = {
        let mut guard = state.settings.write().unwrap();
        guard.stt_engine = id;
        guard.clone()
    };
    settings::save(&updated)?;
    // Running 期间换引擎 → restartNeeded 推导变化，推送全量状态
    runtime::snapshot_and_emit(&app, None);
    Ok(())
}

#[tauri::command]
fn get_engine_options(state: tauri::State<SharedState>, id: String) -> serde_json::Value {
    let guard = state.settings.read().unwrap();
    guard
        .engine_options
        .get(&id)
        .cloned()
        .unwrap_or(serde_json::Value::Null)
}

// ---------- 游戏 profile（§5.3） ----------

#[tauri::command]
fn list_profiles() -> Vec<GameProfile> {
    profile::list()
}

/// 保存 profile。内置 profile（lol / generic）会同步推导 removedBuiltinHotwords：
/// 内置热词全量列表 − 传入热词列表 = 用户主动删除的内置热词（供后续合并逻辑排除）。
/// 非内置 id 不触碰该字段。
#[tauri::command]
fn save_profile(mut profile: GameProfile) -> Result<(), String> {
    let builtin = match profile.id.as_str() {
        "lol" => Some(GameProfile::builtin_lol()),
        "generic" => Some(GameProfile::builtin_generic()),
        _ => None,
    };
    if let Some(builtin) = builtin {
        profile.removed_builtin_hotwords = builtin
            .hotwords
            .iter()
            .filter(|w| !profile.hotwords.contains(w))
            .cloned()
            .collect();
    }
    profile::save(&profile)
}

/// 导出 profile 热词到 UTF-8 文本（每行一词条，无权重），返回条数。
#[tauri::command]
fn export_hotwords(profile_id: String, path: String) -> Result<u32, String> {
    let p = profile::get(&profile_id).ok_or_else(|| format!("profile 不存在：{profile_id}"))?;
    let text = format_hotwords_export(&p.hotwords);
    std::fs::write(&path, text).map_err(|e| format!("写入 {path} 失败：{e}"))?;
    Ok(p.hotwords.len() as u32)
}

/// 从 UTF-8 文本导入热词（合并去重，追加到现有列表末尾），返回合并报告。
#[tauri::command]
fn import_hotwords(profile_id: String, path: String) -> Result<HotwordMergeReport, String> {
    let text = std::fs::read_to_string(&path).map_err(|e| format!("读取 {path} 失败：{e}"))?;
    let mut p = profile::get(&profile_id).ok_or_else(|| format!("profile 不存在：{profile_id}"))?;
    let incoming = parse_hotwords_import(&text);
    let (merged, report) = merge_hotwords(&p.hotwords, &incoming);
    p.hotwords = merged;
    profile::save(&p)?;
    Ok(report)
}

/// 导出整包 profile 为 .kprofile ZIP（profile.json + icon.*）。
#[tauri::command]
fn export_profile(profile_id: String, path: String) -> Result<(), String> {
    profile::export_profile(&profile_id, std::path::Path::new(&path))
}

/// 导入 .kprofile ZIP 包：生成新随机 id 落盘，返回导入后的 profile。
#[tauri::command]
fn import_profile(path: String) -> Result<GameProfile, String> {
    profile::import_profile(std::path::Path::new(&path))
}

/// 删除 profile：内置 = 恢复出厂；导入的 = 永久删除（含 icon）。
#[tauri::command]
fn delete_profile(profile_id: String) -> Result<ProfileDeleteOutcome, String> {
    profile::delete_profile(&profile_id)
}

/// 读取 profile 图标字节（无图标 → None）。前端按扩展名推断 mime 建 Blob URL。
#[tauri::command]
fn get_profile_icon(profile_id: String) -> Result<Option<Vec<u8>>, String> {
    Ok(profile::profile_icon_bytes(&profile_id))
}

// ---------- 提权（UIPI 方案，§10 R-1） ----------

/// 提权状态：自身是否提权 + 当前激活 profile 的游戏进程是否提权（null = 无法判断）
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ElevationStatus {
    pub elevated: bool,
    pub active_game_elevated: Option<bool>,
    /// 当前平台是否支持提权语义（仅 Windows；其他平台前端据此不弹提权提示）
    pub supported: bool,
}

#[tauri::command]
fn get_elevation_status(state: tauri::State<SharedState>) -> ElevationStatus {
    let elevated = elevation::is_elevated();
    // 激活 profile 的进程名 → 运行中的 pid → TokenElevation
    // 链路修复（resolve_active_game_pid）：profile 文件缺失时回退内置 profile，
    // 可用列表 = 磁盘 profile + 未覆盖的内置 profile
    let active_game_elevated = {
        let guard = state.settings.read().unwrap();
        let mut available = profile::list();
        for b in [GameProfile::builtin_lol(), GameProfile::builtin_generic()] {
            if !available.iter().any(|p| p.id == b.id) {
                available.push(b);
            }
        }
        elevation::resolve_active_game_pid(
            guard.active_profile_id.as_deref(),
            &available,
            &mut |name| platform_inject::find_pid_by_name(name),
        )
        .and_then(elevation::is_process_elevated)
    };
    ElevationStatus {
        elevated,
        active_game_elevated,
        supported: cfg!(windows),
    }
}

/// 热键注册状态：设置页热键分区展示「注册失败，可能被其他程序/其他 Kotone 实例占用」
#[tauri::command]
fn get_hotkey_status(app: AppHandle) -> HotkeyStatus {
    app.state::<HotkeyManager>().status()
}

/// 主动检测低级键盘钩子与 SendInput 环境；不要求先开始录入或启动运行时。
#[tauri::command]
async fn check_input_environment(app: AppHandle) -> Result<InputEnvironmentCheck, String> {
    let worker_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        worker_app
            .state::<HotkeyManager>()
            .check_input_environment()
    })
    .await
    .map_err(|e| format!("输入环境自检任务失败：{e}"))
}

/// 静态键位冲突提示（P2-⑩：向导热键步骤录入后展示；常见游戏键位表）
#[tauri::command]
fn detect_hotkey_conflicts(key: String) -> Vec<String> {
    crate::hotkey::detect_conflicts(&key)
}

/// 开始热键捕获（设置页「点击录入」）：结果经 `kotone://hotkey-capture` 事件推送
#[tauri::command]
async fn start_hotkey_capture(app: AppHandle) -> Result<(), String> {
    let worker_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        worker_app
            .state::<HotkeyManager>()
            .start_capture(worker_app.clone())
    })
    .await
    .map_err(|e| format!("启动热键自检任务失败：{e}"))?
}

/// 取消进行中的热键捕获
#[tauri::command]
fn cancel_hotkey_capture(app: AppHandle) {
    app.state::<HotkeyManager>().cancel_capture();
}

/// 一键管理员重启：ShellExecuteExW "runas" 拉起新进程后退出当前进程。
/// 用户在 UAC 弹窗点「否」会返回错误，当前进程继续运行。
#[tauri::command]
fn restart_as_admin(app: AppHandle) -> Result<(), String> {
    elevation::restart_as_admin()?;
    app.exit(0);
    Ok(())
}

// ---------- 模型 / 评测 ----------

#[tauri::command]
fn list_models() -> Result<Vec<model::ModelInfo>, String> {
    model::list()
}

/// 下载模型（id = 清单内任意模型 / silero-vad；镜像策略见 settings.download）。
/// 进度经 "kotone://download" 事件外发：{ id, downloaded, total }。
/// async 命令 + spawn_blocking：大模型下载不阻塞 UI 线程；IPC 签名不变。
/// 下载前做磁盘空间预检（P2-⑦）：目标目录所在卷剩余空间不足时直接拒绝，
/// 避免下载到一半才发现 C 盘/目标盘已满。
#[tauri::command]
async fn download_model(
    app: AppHandle,
    state: tauri::State<'_, SharedState>,
    id: String,
) -> Result<(), String> {
    // 磁盘空间预检：模型体积 vs 模型目录所在卷可用空间
    if let (Some(need), Some(dir)) = (model::model_size_bytes(&id), Some(model_dir_now(&state))) {
        if let Some(avail) = disk_available_space(&dir) {
            if avail < need {
                let mb = |b: u64| b / 1_000_000;
                return Err(format!(
                    "磁盘空间不足：模型需要约 {} MB，但「{}」所在磁盘仅剩 {} MB。\
                     请清理磁盘或在设置中更换模型存储位置",
                    mb(need),
                    dir.display(),
                    mb(avail)
                ));
            }
        }
    }
    let case_id = format!(
        "model-download-{}-{}",
        id,
        kotone_core::eval::new_session_id()
    );
    record_process_event(
        &app,
        &serde_json::json!({
            "caseId": case_id.clone(),
            "activity": "model_download_started",
            "data": {}
        }),
    );
    let started = std::time::Instant::now();
    let app_for_download = app.clone();
    let case_for_result = case_id.clone();
    let result = match tauri::async_runtime::spawn_blocking(move || {
        let app2 = app_for_download.clone();
        let id2 = id.clone();
        model::download(&id, &move |downloaded, total| {
            use tauri::Emitter as _;
            let _ = app2.emit(
                "kotone://download",
                serde_json::json!({ "id": id2, "downloaded": downloaded, "total": total }),
            );
        })
    })
    .await
    {
        Ok(result) => result,
        Err(error) => Err(format!("下载任务异常：{error}")),
    };
    let (activity, outcome, error_code) = if result.is_ok() {
        ("model_download_succeeded", "success", None)
    } else {
        (
            "model_download_failed",
            "error",
            Some("MODEL_DOWNLOAD_FAILED"),
        )
    };
    record_process_event(
        &app,
        &serde_json::json!({
            "caseId": case_for_result,
            "activity": activity,
            "data": {
                "outcome": outcome,
                "errorCode": error_code,
                "durationMs": started.elapsed().as_millis() as u64
            }
        }),
    );
    result
}

/// 当前生效模型目录（磁盘预检用）
fn model_dir_now(state: &tauri::State<'_, SharedState>) -> std::path::PathBuf {
    let settings = state.settings.read().unwrap();
    model::models_dir_from(&settings)
}

/// 路径所在卷的可用空间（sysinfo Disks；取最长挂载前缀匹配；失败返回 None 不阻断下载）
fn disk_available_space(path: &std::path::Path) -> Option<u64> {
    use sysinfo::Disks;
    let disks = Disks::new_with_refreshed_list();
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    disks
        .iter()
        .filter(|d| canonical.starts_with(d.mount_point()))
        .max_by_key(|d| d.mount_point().as_os_str().len())
        .map(|d| d.available_space())
}

/// 请求取消进行中的模型下载（幂等；.part 保留可续传）
#[tauri::command]
fn cancel_download() {
    model::cancel_download();
    log::log("download cancel requested");
}

#[tauri::command]
fn set_active_model(
    app: AppHandle,
    state: tauri::State<SharedState>,
    engine_id: String,
    model_id: String,
) -> Result<(), String> {
    model::set_active(&engine_id, &model_id)?;
    // model::set_active 直接读写 config.json；同步 SharedState 内存副本，
    // restartNeeded 推导（快照 vs 当前配置）以内存为准
    {
        let mut guard = state.settings.write().unwrap();
        if let Some(opts) = guard.engine_options.as_object_mut() {
            let entry = opts
                .entry(engine_id.clone())
                .or_insert_with(|| serde_json::json!({}));
            entry["model"] = serde_json::Value::String(model_id);
        }
    }
    // Running 期间换活动模型 → restartNeeded = true（不自动重启，用户显式重启生效）
    runtime::snapshot_and_emit(&app, None);
    Ok(())
}

// ---------- 运行时「启动」开关 ----------

/// 全量运行时状态（相位 + restartNeeded + 当前引擎/模型/模式）
#[tauri::command]
fn get_runtime_status(app: AppHandle, state: tauri::State<SharedState>) -> RuntimeStatus {
    let rt = app.state::<RuntimeManager>();
    let settings = state.settings.read().unwrap();
    rt.status(&settings, &state.engines, None)
}

/// 启动：warmup 引擎 → 注册热键 → 显示悬浮窗（进度经 kotone://runtime 推送）。
/// Running + restartNeeded 时等价于 stop + start（重启语义）。
#[tauri::command]
async fn start_runtime(app: AppHandle) -> Result<RuntimeStatus, String> {
    runtime::start(&app).await
}

/// 停止：取消会话 → 注销热键 → 隐藏悬浮窗 → 卸载引擎。Stopped 时幂等。
#[tauri::command]
async fn stop_runtime(app: AppHandle) -> Result<RuntimeStatus, String> {
    runtime::stop(&app).await
}

// ---------- 模型目录管理 ----------

/// 当前模型目录（生效路径 + 是否默认）
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelsDirInfo {
    pub dir: String,
    pub is_default: bool,
}

#[tauri::command]
fn get_models_dir(state: tauri::State<SharedState>) -> ModelsDirInfo {
    let settings = state.settings.read().unwrap();
    ModelsDirInfo {
        dir: model::models_dir_from(&settings)
            .to_string_lossy()
            .into_owned(),
        is_default: settings.models.dir.trim().is_empty(),
    }
}

/// set_models_dir 返回：新目录 + 迁移报告（moved / failed 条目名）
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelsDirMigration {
    pub dir: String,
    pub moved: Vec<String>,
    pub failed: Vec<String>,
}

/// 切换模型存储目录：先把旧目录内容移动到新目录（跨卷回退复制），
/// 迁移完成后才写配置——迁移失败过半也不丢配置一致性（failed 条目需重新下载）。
/// 传空字符串 = 恢复默认 ~/.kotone/models。
/// 运行时 Running 期间拒绝迁移：模型文件被引擎占用（Windows 文件锁），
/// 迁移必然失败且产生困惑的 failed 列表（P2-⑧）。
#[tauri::command]
fn set_models_dir(
    app: AppHandle,
    state: tauri::State<SharedState>,
    dir: String,
) -> Result<ModelsDirMigration, String> {
    let running = app
        .try_state::<RuntimeManager>()
        .map(|rt| rt.phase() == RuntimePhase::Running)
        .unwrap_or(false);
    if running {
        return Err("引擎正在运行，请先停止引擎再迁移模型目录（设置页或托盘「停止引擎」）".into());
    }
    let (old_dir, new_dir) = {
        let settings = state.settings.read().unwrap();
        let old = model::models_dir_from(&settings);
        let mut next = settings.clone();
        next.models.dir = dir.trim().to_string();
        (old, model::models_dir_from(&next))
    };
    if old_dir == new_dir {
        return Err("新目录与当前目录相同".into());
    }
    let report = model::migrate_dir_contents(&old_dir, &new_dir)?;

    let updated = {
        let mut guard = state.settings.write().unwrap();
        guard.models.dir = dir.trim().to_string();
        guard.clone()
    };
    settings::save(&updated)?;
    // 模型路径变化影响就绪判断与 restartNeeded 推导，推送状态
    runtime::snapshot_and_emit(&app, None);
    Ok(ModelsDirMigration {
        dir: new_dir.to_string_lossy().into_owned(),
        moved: report.moved,
        failed: report.failed,
    })
}

/// 删除已下载模型；active 模型被删时回退默认并同步 SharedState
#[tauri::command]
fn delete_model(
    app: AppHandle,
    state: tauri::State<SharedState>,
    id: String,
) -> Result<model::DeleteOutcome, String> {
    let outcome = model::delete(&id)?;
    if outcome.was_active {
        // model::delete 清了磁盘配置的 engineOptions[engine].model；同步内存副本
        let mut guard = state.settings.write().unwrap();
        if let Some(opts) = guard.engine_options.as_object_mut() {
            for entry in opts.values_mut().filter_map(|e| e.as_object_mut()) {
                if entry.get("model").and_then(|m| m.as_str()) == Some(id.as_str()) {
                    entry.remove("model");
                }
            }
        }
    }
    runtime::snapshot_and_emit(&app, None);
    Ok(outcome)
}

/// 在系统文件管理器中打开历史记录目录（P2-⑨：历史目录可自定义后仍需可视入口）
#[tauri::command]
fn open_history_dir() -> Result<(), String> {
    let dir = kotone_core::history::history_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("无法创建目录 {}：{e}", dir.display()))?;
    open_in_file_manager(&dir)
}

/// 在系统文件管理器中打开模型目录
#[tauri::command]
fn open_models_dir(state: tauri::State<SharedState>) -> Result<(), String> {
    let dir = {
        let settings = state.settings.read().unwrap();
        model::models_dir_from(&settings)
    };
    std::fs::create_dir_all(&dir).map_err(|e| format!("无法创建目录 {}：{e}", dir.display()))?;
    open_in_file_manager(&dir)
}

/// 在系统默认浏览器中打开外部链接（关于页 GitHub 等）。
/// webview 内 target=_blank 不会调起系统浏览器；rundll32 方案零新增依赖，
/// 白名单只允许 http/https，避免被当成任意协议执行入口。
#[tauri::command]
fn open_external(url: String) -> Result<(), String> {
    if !url.starts_with("https://") && !url.starts_with("http://") {
        return Err("仅支持打开 http/https 链接".into());
    }
    std::process::Command::new("rundll32")
        .args(["url.dll,FileProtocolHandler", &url])
        .spawn()
        .map_err(|e| format!("打开浏览器失败：{e}"))?;
    Ok(())
}

#[cfg(windows)]
fn open_in_file_manager(dir: &std::path::Path) -> Result<(), String> {
    std::process::Command::new("explorer.exe")
        .arg(dir)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("打开目录失败：{e}"))
}

#[cfg(not(windows))]
fn open_in_file_manager(dir: &std::path::Path) -> Result<(), String> {
    std::process::Command::new("xdg-open")
        .arg(dir)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("打开目录失败：{e}"))
}

// ---------- 识别历史（core history 薄转发；参考 CLI log list/clear，§8.1 历史面板） ----------

/// 识别历史列表（新→旧；模式/容量配置走通用 update_settings patch）
#[tauri::command]
fn get_history() -> Result<Vec<kotone_core::history::HistoryRecord>, String> {
    kotone_core::history::list()
}

/// 清空全部识别历史（含音频文件；前端做二次确认，后端不再拦截）
#[tauri::command]
fn clear_history() -> Result<(), String> {
    kotone_core::history::clear()
}

/// 删除单条识别历史（按 sessionId + ts 精确匹配；带录音且不再被其他记录
/// 引用时一并删除对应 wav；记录不存在 = 静默成功，幂等）
#[tauri::command]
fn delete_history_record(session_id: String, ts: String) -> Result<(), String> {
    kotone_core::history::delete(&session_id, &ts)
}

/// 读取历史记录的音频文件字节（~/.kotone/history/audio/<file_name>）。
/// file_name 只允许纯文件名：含 /、\\、.. 或为空直接拒绝（防路径穿越）。
#[tauri::command]
fn read_history_audio(file_name: String) -> Result<Vec<u8>, String> {
    if file_name.is_empty()
        || file_name.contains('/')
        || file_name.contains('\\')
        || file_name.contains("..")
    {
        return Err(format!("非法的音频文件名：{file_name}"));
    }
    let path = kotone_core::history::history_dir()
        .join("audio")
        .join(&file_name);
    std::fs::read(&path).map_err(|e| format!("读取音频 {} 失败：{e}", path.display()))
}

// ---------- 进程资源占用 ----------

/// sysinfo System 常驻进程内存：CPU 百分比依赖两次 refresh 的间隔采样，
/// 复用同一实例才能保证前端每次轮询拿到增量而非 0。
struct ResourceMonitor(Mutex<sysinfo::System>);

/// 当前进程资源占用（前端每 2s 轮询一次）
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceUsage {
    /// CPU 占用百分比（保留 1 位小数）。sysinfo 语义：自上次 refresh 以来
    /// 的增量采样，首次调用可能为 0；前端 2s 轮询正好提供采样间隔。
    pub cpu_percent: f32,
    /// 常驻内存（字节）
    pub memory_bytes: u64,
}

#[tauri::command]
fn get_resource_usage(monitor: tauri::State<ResourceMonitor>) -> ResourceUsage {
    let pid = sysinfo::Pid::from_u32(std::process::id());
    let mut sys = monitor.0.lock().unwrap();
    // sysinfo 0.30 无按 pid 过滤的 refresh（ProcessesToUpdate 是 0.32+ 的 API），
    // 全量刷新后只取本进程；2s 轮询频率下开销可接受。
    sys.refresh_processes();
    let (cpu_percent, memory_bytes) = sys
        .process(pid)
        .map(|p| ((p.cpu_usage() * 10.0).round() / 10.0, p.memory()))
        .unwrap_or((0.0, 0));
    ResourceUsage {
        cpu_percent,
        memory_bytes,
    }
}

// ---------- 会话控制 / 调试 ----------

/// Preview 状态下确认发送（ADR-006：预览只读，始终发送预览文本）
#[tauri::command]
async fn confirm_send(state: tauri::State<'_, SharedState>) -> Result<(), String> {
    state.orchestrator.confirm_send().await
}

/// 取消当前会话（悬浮窗取消按钮 / 调试）
#[tauri::command]
async fn cancel_session(state: tauri::State<'_, SharedState>) -> Result<(), String> {
    state.orchestrator.cancel().await;
    Ok(())
}

/// 手动触发发送（调试/记事本测试用）：走真实 WindowsInjector 时序
/// （openChatKey → Unicode/剪贴板 → sendKey，直发当前前台窗口）。
///
/// P0（用户反馈「SendInput 被 360 拦截后卡死」）：此前是同步命令，在主线程
/// 直接调 SendInput——被安全软件钩住挂起时整个主进程（UI/托盘/IPC）冻结。
/// 改为 async + spawn_blocking + 10s 超时：挂起只阻塞工作线程，超时后返回
/// 清晰错误，UI 保持响应。
#[tauri::command]
async fn simulate_send(
    state: tauri::State<'_, SharedState>,
    text: String,
    profile_id: Option<String>,
) -> Result<(), InjectError> {
    let profile = profile_id
        .as_deref()
        .and_then(profile::get)
        .unwrap_or_else(GameProfile::builtin_generic);
    let injector = state.injector.clone();
    let task = tauri::async_runtime::spawn_blocking(move || {
        injector.send(&text, &profile, CancelToken::default())
    });
    const SEND_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
    match tokio::time::timeout(SEND_TIMEOUT, task).await {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => Err(InjectError::new("发送线程异常".to_string())),
        Err(_) => Err(InjectError::new(
            "发送超时（10s）：模拟输入未在限时内完成，通常是被安全软件或游戏反作弊拦截".to_string(),
        )),
    }
}

/// 自启动提权（§10 R-1）：设置开启且当前未提权时，runas 重启自身并立即退出。
///
/// 必须在 Tauri Builder 初始化之前执行：单实例插件在插件初始化阶段创建
/// 命名互斥锁，若在 setup() 里才 runas，未提权父进程已持有该锁——提权子进程
/// 会把自己当成「第二实例」，把参数转发给正在退出的父进程后自杀，父进程随后
/// 也退出，表现为「双击后没有任何窗口」（0.1.4 用户反馈）。在这里提前判定并
/// std::process::exit，父进程从不触碰单实例锁 / WebView2，子进程是唯一实例。
///
/// 防循环：子进程带 ELEVATED_RETRY_ARG 标记；用户取消 UAC 时 ShellExecuteExW
/// 直接返回错误，本进程按普通权限继续启动（不退出）。
#[cfg(windows)]
fn auto_elevate_before_tauri_init() {
    let settings = settings::load();
    if !elevation::should_auto_elevate(
        settings.run_as_admin_on_start,
        elevation::is_elevated(),
        elevation::retry_marker_present(),
    ) {
        return;
    }
    match elevation::restart_for_auto_elevate() {
        Ok(()) => {
            // 提权副本已拉起：立刻退出，不做任何 Tauri/WebView2 初始化。
            std::process::exit(0);
        }
        Err(e) => eprintln!("[kotone] 自动提权重启失败: {e}"),
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // 见函数注释：自启动提权必须先于单实例插件的互斥锁创建。
    #[cfg(windows)]
    auto_elevate_before_tauri_init();

    let startup_options = StartupOptions {
        onboarding: parse_onboarding_launch_mode(std::env::args()),
    };
    tauri::Builder::default()
        // 单实例：第二实例启动时唤起已有实例的设置窗口并退出自身，
        // 避免旧进程未退出导致热键「already registered」与 WebView2 类注册冲突
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.show();
                let _ = win.unminimize();
                let _ = win.set_focus();
            }
            // 测试时常从已在托盘运行的实例再次执行
            // `kotone.exe --onboarding=always`。第二实例不会进入 setup，
            // 因此必须把强制打开请求转发给现有设置窗口。
            if parse_onboarding_launch_mode(args.iter().map(String::as_str))
                == OnboardingLaunchMode::Always
            {
                let _ = app.emit_to("main", "kotone://open-onboarding", ());
            }
        }))
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        // 关闭按钮不退出应用：main / overlay 窗口 CloseRequested 一律转 hide（托盘常驻）；
        // 仅托盘菜单「退出」（app.exit）真正结束进程。
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let _ = window.hide();
                api.prevent_close();
            }
        })
        .setup(move |app| {
            log::init();
            let previous_panic_hook = std::panic::take_hook();
            std::panic::set_hook(Box::new(move |info| {
                log::log(&format!("panic: {info}"));
                previous_panic_hook(info);
            }));
            log::log(&format!(
                "startup: version={} elevated={} arch={}",
                env!("CARGO_PKG_VERSION"),
                elevation::is_elevated(),
                std::env::consts::ARCH
            ));
            tray::setup_tray(app.handle())?;
            app.manage(startup_options.clone());

            // 开发调试：`pnpm tauri dev -- --console` 启动后自动打开 webview 控制台。
            // `open_devtools` 只在 Tauri 的开发构建中可用，必须从 release 编译中移除。
            #[cfg(debug_assertions)]
            {
                if std::env::args().any(|a| a == "--console") {
                    for label in ["main", "overlay"] {
                        if let Some(win) = app.get_webview_window(label) {
                            win.open_devtools();
                        }
                    }
                }
            }

            // 首次运行：默认配置 + 内置 profile 落盘（~/.kotone/）
            let settings = settings::load();
            let _ = settings::save(&settings);
            let mut app_started =
                process_log::ProcessEvent::new(process_log::app_session_id(), "app_started");
            app_started.context.engine_id = Some(settings.stt_engine.clone());
            app_started.context.model_id =
                Some(model::active_model_from(&settings, &settings.stt_engine));
            app_started.context.profile_id = settings.active_profile_id.clone();
            app_started.context.elevated = Some(elevation::is_elevated());
            let _ = process_log::record(app_started);

            // 自启动提权已前移到 run() 顶部（auto_elevate_before_tauri_init）：
            // 单实例插件初始化早于 setup，父进程若在此才 runas 退出，提权子进程
            // 会被单实例锁误判为第二实例而自杀，导致「双击无反应」。

            if let Err(e) = profile::ensure_builtin() {
                eprintln!("[kotone] 内置 profile 落盘失败: {e}");
            }

            // silero VAD 模型随二进制打包，首次启动解压到模型目录；
            // 失败仅记日志不阻断启动（one-shot 判停缺失时 begin 会报清晰错误）
            match model::ensure_vad_model() {
                Ok(written) => {
                    if written {
                        log::log("silero VAD 模型已写入模型目录");
                    }
                }
                Err(e) => log::log(&format!("silero VAD 模型写入失败: {e}")),
            }

            let settings = Arc::new(RwLock::new(settings));
            let mut registry = EngineRegistry::new();
            kotone_stt::register_builtin(&mut registry);
            let engines = Arc::new(registry);
            let emitter: Arc<dyn Emitter> = Arc::new(TauriEmitter {
                app: app.handle().clone(),
                vis_gen: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            });
            let injector: Arc<dyn Injector> = Arc::new(WindowsInjector);
            let focus: Arc<dyn FocusBackend> = Arc::new(WinFocusBackend);
            let audio_backend: Arc<dyn kotone_core::audio::AudioBackend> =
                Arc::new(platform_audio::CpalBackend);

            #[allow(unused_mut)] // vad-silero feature 关闭时无可变接线
            let mut orchestrator = Orchestrator::new(
                settings.clone(),
                engines.clone(),
                audio_backend,
                injector.clone(),
                focus,
                emitter,
            );
            // VAD 接线（ADR-007）：vad-silero feature 开启时注入 silero 工厂；
            // 默认构建不接入——one-shot 模式 begin 会报清晰错误
            #[cfg(feature = "vad-silero")]
            {
                orchestrator.vad_factory = Some(kotone_stt::vad::silero_factory());
            }
            let orchestrator = orchestrator.into_arc();

            app.manage(SharedState {
                settings: settings.clone(),
                orchestrator: orchestrator.clone(),
                engines,
                injector,
            });
            app.manage(HotkeyManager::new(app.handle(), orchestrator.clone()));
            app.manage(RuntimeManager::new());
            app.manage(ResourceMonitor(Mutex::new(sysinfo::System::new())));

            for label in ["main", "overlay"] {
                if let Some(win) = app.get_webview_window(label) {
                    harden_webview(&win);
                }
            }
            if let Some(win) = app.get_webview_window("overlay") {
                let overlay = settings.read().unwrap().overlay.clone();
                apply_overlay_window_config(&win, &overlay);
            }

            // 「启动」开关（core runtime 状态机）：默认 Stopped——不注册热键、
            // 不 warmup 引擎、悬浮窗隐藏。ui.autoStart = true 时自动 start_runtime
            // （warmup → 注册热键 → 显示悬浮窗，进度经 kotone://runtime 推送）。
            if settings.read().unwrap().ui.auto_start {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = runtime::start(&handle).await {
                        log::log(&format!("auto-start failed: {e}"));
                    }
                });
            }

            // 启动即显示设置窗口：全部窗口默认隐藏时，应用看起来像「没起来」。
            // 托盘常驻语义不变——关闭此窗口只是隐藏，进程仍在托盘。
            // 自动提权重启的场合此分支在提权子进程中执行，同样弹窗，符合预期。
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.show();
                let _ = win.set_focus();
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            ping,
            export_diagnostics,
            log_frontend_error,
            get_startup_options,
            get_settings,
            update_settings,
            list_audio_devices,
            set_audio_device,
            list_stt_engines,
            set_stt_engine,
            get_engine_options,
            list_profiles,
            save_profile,
            export_hotwords,
            import_hotwords,
            export_profile,
            import_profile,
            delete_profile,
            get_profile_icon,
            save_overlay_position,
            get_elevation_status,
            get_hotkey_status,
            check_input_environment,
            detect_hotkey_conflicts,
            start_hotkey_capture,
            cancel_hotkey_capture,
            restart_as_admin,
            list_models,
            download_model,
            cancel_download,
            set_active_model,
            get_runtime_status,
            start_runtime,
            stop_runtime,
            get_models_dir,
            set_models_dir,
            delete_model,
            open_models_dir,
            open_history_dir,
            open_external,
            get_history,
            clear_history,
            delete_history_record,
            read_history_audio,
            get_resource_usage,
            confirm_send,
            cancel_session,
            simulate_send,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Kotone application");
}
