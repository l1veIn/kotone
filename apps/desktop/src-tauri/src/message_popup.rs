//! 消息提示（ADR 039 D4）：发自动卡片时，屏幕右下角弹的小窗——她的头像 + 一句话，点它打开聊天、定位到那张卡片。
//! 照桌面聊天软件（QQ、微信电脑版）的新消息提示；外观和时长照 `docs/design/lol-app.md` 第四节 3。
//!
//! - **绝不抢焦点**：和气泡同一套做法（ADR 018）——窗口不可聚焦（`focusable: false`，Windows 上是
//!   `WS_EX_NOACTIVATE`）、不进任务栏、置顶，用 `SW_SHOWNA` 显示（不激活窗口，不 `SetForegroundWindow`）。
//!   和气泡不同的是它能点（不整窗鼠标穿透）；玩家点了它才把 Kotone 主窗口拿到前面（`agent::auto_card_open`）。
//! - **不弹的时候**：游戏独占全屏、演示模式、别的程序全屏（`fullscreen::popup_blocked`：无边框全屏的游戏照样弹）；
//!   Kotone 自己的窗口在前台（玩家正看着 Kotone，卡片就在聊天里）。卡片照样进聊天。
//! - 同一时间只有一个：新的替换旧的。界面（`routes/messagePopup`）管动效和 6 秒倒计时（鼠标停在上面时不走），
//!   到点调 [`message_popup_close`]；万一界面没调，这里 60 秒后也收起。
//! - 位置：前台窗口所在显示器的工作区右下角（避开任务栏），DPI 按那块屏幕换算；气泡也在右下角而且正显示着时，
//!   放在气泡窗口上面。
//! - 不进截图（`WDA_EXCLUDEFROMCAPTURE`，照覆盖层）：她看屏幕、截图识别时不会把它认进去。

use std::sync::Mutex;
use std::time::Duration;

use kotone_core::log;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

pub const LABEL: &str = "message-popup";
pub const EVENT: &str = "kotone://message-popup";
/// 窗口的逻辑尺寸：320 × 72 的小窗，四周留 12 给阴影（设计文档：阴影 0 10px 28px）。
const WINDOW_SIZE: (f64, f64) = (344.0, 104.0);
/// 小窗离工作区边缘 24（和气泡一样）；窗口四周有 12 的阴影边，所以窗口离边缘 12。
const MARGIN: f64 = 12.0;
/// 气泡在右下角显示着时，和气泡窗口之间隔多少。
const GAP: f64 = 12.0;
/// 界面没来收起时，最多显示多久。
const MAX_SHOWN: Duration = Duration::from_secs(60);

/// 一条消息提示（发给小窗的界面）。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Popup {
    pub plugin: String,
    pub plugin_name: String,
    /// 她现在叫什么（“{name} · {插件名}”那一行）。
    pub name: String,
    pub text: String,
    pub conversation_id: String,
    /// 卡片的去重键（点它时定位到这张卡片）。
    pub key: String,
    /// 第几条（新的替换旧的；界面收起时带上，旧的收起请求不收新的）。
    pub revision: u64,
}

#[derive(Default)]
struct State {
    current: Option<Popup>,
    revision: u64,
}

fn state() -> &'static Mutex<State> {
    static STATE: std::sync::OnceLock<Mutex<State>> = std::sync::OnceLock::new();
    STATE.get_or_init(Default::default)
}

/// 玩家正看着 Kotone 自己的窗口（主窗口、网页登录窗口……）吗：前台窗口是它、而且看得见、没最小化。
/// 看不见的不算：`--background` 启动时藏着的主窗口会占着“前台”（2026-11-25 真机看到的），那时玩家看的是别的。
fn kotone_in_front(app: &AppHandle) -> bool {
    #[cfg(windows)]
    {
        use windows::Win32::UI::WindowsAndMessaging::{
            GetForegroundWindow, IsIconic, IsWindowVisible,
        };
        // SAFETY: 只读当前前台窗口的句柄和状态
        let front = unsafe { GetForegroundWindow() };
        let shown = !front.0.is_null()
            && unsafe { IsWindowVisible(front).as_bool() && !IsIconic(front).as_bool() };
        shown
            && app
                .webview_windows()
                .values()
                .any(|w| w.hwnd().is_ok_and(|h| h.0 == front.0))
    }
    #[cfg(not(windows))]
    {
        let _ = app;
        false
    }
}

/// 现在不弹的原因（None = 可以弹）。
fn blocked(app: &AppHandle) -> Option<&'static str> {
    if kotone_in_front(app) {
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

/// 弹一条（替换正在显示的）。不弹的时候只记一笔。
pub fn show(app: &AppHandle, mut popup: Popup) {
    if let Some(why) = blocked(app) {
        log::log(&format!("消息提示：没弹（{why}），卡片照样在聊天里"));
        return;
    }
    let revision = {
        let mut s = state().lock().unwrap();
        s.revision += 1;
        popup.revision = s.revision;
        s.current = Some(popup.clone());
        s.revision
    };
    log::log(&format!(
        "消息提示：{} · {}（第 {revision} 条）",
        popup.plugin_name, popup.text
    ));
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        let Some(window) = handle.get_webview_window(LABEL).or_else(|| create(&handle)) else {
            return;
        };
        if let Err(e) = place(&handle, &window) {
            log::log(&format!("消息提示：摆不好位置：{e}"));
        }
        if std::env::var_os("KOTONE_POPUP_CAPTURABLE").is_none() {
            crate::overlay_layer::exclude_from_capture(&window);
        }
        let _ = handle.emit_to(LABEL, EVENT, &popup);
        crate::show_window_no_focus(&window);
    });
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(MAX_SHOWN).await;
        hide_if(&app, revision);
    });
}

/// 收起（不管是第几条）。
pub fn hide(app: &AppHandle) {
    let revision = state().lock().unwrap().revision;
    hide_if(app, revision);
}

/// 还是第 `revision` 条时收起（新的已经换上来就不动）。
fn hide_if(app: &AppHandle, revision: u64) {
    {
        let mut s = state().lock().unwrap();
        if s.revision != revision || s.current.is_none() {
            return;
        }
        s.current = None;
    }
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(window) = handle.get_webview_window(LABEL) {
            crate::hide_window(&window);
        }
    });
}

/// 第一次要弹时才建窗口（`tauri.conf.json` 里 `create: false`）：从没发过自动卡片时不多占一个网页进程。
fn create(app: &AppHandle) -> Option<tauri::WebviewWindow> {
    let config = app
        .config()
        .app
        .windows
        .iter()
        .find(|c| c.label == LABEL)
        .cloned()?;
    match tauri::WebviewWindowBuilder::from_config(app, &config).and_then(|b| b.build()) {
        Ok(window) => {
            crate::harden_webview(&window);
            Some(window)
        }
        Err(e) => {
            log::log(&format!("消息提示：窗口建不起来：{e}"));
            None
        }
    }
}

/// 摆到前台窗口所在显示器工作区的右下角；气泡在右下角显示着时放在气泡上面。
fn place(app: &AppHandle, window: &tauri::WebviewWindow) -> Result<(), String> {
    let (left, top, right, bottom) = foreground_work_area().ok_or("无法确定显示器")?;
    let scale = window
        .available_monitors()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|m| {
            let p = m.position();
            let s = m.size();
            let cx = (left + right) / 2;
            let cy = (top + bottom) / 2;
            cx >= p.x && cx < p.x + s.width as i32 && cy >= p.y && cy < p.y + s.height as i32
        })
        .map(|m| m.scale_factor())
        .unwrap_or(1.0);
    let width = (WINDOW_SIZE.0 * scale).round() as i32;
    let height = (WINDOW_SIZE.1 * scale).round() as i32;
    let margin = (MARGIN * scale).round() as i32;
    let x = right - width - margin;
    let mut y = bottom - height - margin;
    if let Some(bubble_top) = bubble_top_right_corner(app, (left, top, right, bottom)) {
        y = y.min(bubble_top - height - (GAP * scale).round() as i32);
    }
    let y = y.max(top);
    window
        .set_size(tauri::PhysicalSize::new(width as u32, height as u32))
        .map_err(|e| e.to_string())?;
    window
        .set_position(tauri::PhysicalPosition::new(x, y))
        .map_err(|e| e.to_string())
}

/// 气泡正显示在这块屏的右下角时，气泡窗口的上沿（物理像素）。
fn bubble_top_right_corner(app: &AppHandle, area: (i32, i32, i32, i32)) -> Option<i32> {
    let snap = crate::bubble::snapshot(app)?;
    if snap.phase == crate::bubble::Phase::Hidden
        || snap.corner != kotone_core::assistant::BubbleCorner::BottomRight
    {
        return None;
    }
    let window = app.get_webview_window("companion")?;
    let p = window.outer_position().ok()?;
    let (l, t, r, b) = area;
    (p.x >= l && p.x < r && p.y >= t && p.y < b).then_some(p.y)
}

#[cfg(windows)]
fn foreground_work_area() -> Option<(i32, i32, i32, i32)> {
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTOPRIMARY,
    };
    use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: 只读当前前台窗口所在的显示器信息
    let ok = unsafe {
        GetMonitorInfoW(
            MonitorFromWindow(GetForegroundWindow(), MONITOR_DEFAULTTOPRIMARY),
            &mut info,
        )
    };
    ok.as_bool().then(|| {
        let r = info.rcWork;
        (r.left, r.top, r.right, r.bottom)
    })
}

#[cfg(not(windows))]
fn foreground_work_area() -> Option<(i32, i32, i32, i32)> {
    None
}

/// 小窗的界面刚加载好：拿现在要显示的那条（窗口第一次建的时候，事件可能比页面先到）。
#[tauri::command]
pub fn message_popup_current() -> Option<Popup> {
    state().lock().unwrap().current.clone()
}

/// 小窗到点了（或玩家点了 ×）：收起。
#[tauri::command]
pub fn message_popup_close(app: AppHandle, revision: u64) {
    hide_if(&app, revision);
}

/// 玩家点了小窗：打开聊天、定位到那张卡片。
#[tauri::command]
pub fn message_popup_open(app: AppHandle, revision: u64) -> Result<(), String> {
    let popup = {
        let s = state().lock().unwrap();
        s.current
            .clone()
            .filter(|p| p.revision == revision)
            .ok_or("这条消息提示已经收起了")?
    };
    crate::agent::auto_card_open(app, popup.conversation_id, popup.key)
}
