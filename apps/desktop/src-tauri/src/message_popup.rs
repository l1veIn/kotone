//! 消息提示（ADR 039 D4）：发自动卡片时，屏幕右下角弹的小窗——她的头像 + 一句话，点它打开聊天、定位到那张卡片。
//! 照桌面聊天软件（QQ、微信电脑版）的新消息提示；外观和时长照 `docs/design/lol-app.md` 第四节 3。
//!
//! - **绝不抢焦点**：和气泡同一套做法（ADR 018）——窗口不可聚焦（`focusable: false`，Windows 上是
//!   `WS_EX_NOACTIVATE`）、不进任务栏、置顶，用 `SW_SHOWNA` 显示（不激活窗口，不 `SetForegroundWindow`）。
//!   和气泡不同的是它能点（不整窗鼠标穿透）；玩家点了它才把 Kotone 主窗口拿到前面（`agent::auto_card_open`）。
//!   不可聚焦照样点得到：2026-11-25 真机（`scripts/manual-e2e/message_popup_real.py` M4、M6）页面收到按下和点击，
//!   点卡片打开聊天、滚到那张卡片，点 × 收起时前台一直是游戏（测试窗口）。别照语音输入悬浮窗改成可聚焦（那边不可
//!   聚焦时收不到点击，`attention.rs`）：改了以后点 × 会把游戏的前台拿走。
//! - **不弹的时候**：游戏独占全屏、演示模式、别的程序全屏（`fullscreen::popup_blocked`：无边框全屏的游戏照样弹）；
//!   Kotone 自己的窗口在前台（玩家正看着 Kotone，卡片就在聊天里）。卡片照样进聊天。判断在 [`crate::attention`]
//!   （“能不能现在打扰”全宿主共用一份）。
//! - 同一时间只有一个：新的替换旧的。界面（`routes/messagePopup`）管动效和 6 秒倒计时（鼠标停在上面时不走），
//!   到点调 [`message_popup_close`]；万一界面没调，这里 60 秒后也收起。
//! - 位置：前台窗口所在显示器的工作区右下角（避开任务栏），DPI 按那块屏幕换算；同一个角上的语音输入悬浮窗、气泡
//!   正显示着时让开它们，排在最外面（屏幕角落分配，`screen_corner.rs`：右下角排成一列，后出来的悬浮窗、气泡叠上它时
//!   它往外挪）。
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

/// 弹一条（替换正在显示的）。不弹的时候只记一笔。
pub fn show(app: &AppHandle, mut popup: Popup) {
    if let Some(why) = crate::attention::popup_blocked() {
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
        if let Err(e) = place(&window) {
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

/// 摆到前台窗口所在显示器工作区的右下角，让开同一个角上正显示着的悬浮窗、气泡。
fn place(window: &tauri::WebviewWindow) -> Result<(), String> {
    crate::screen_corner::place(
        window,
        crate::screen_corner::Corner::BottomRight,
        WINDOW_SIZE,
        (MARGIN, MARGIN),
    )
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
