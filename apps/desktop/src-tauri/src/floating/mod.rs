//! 悬浮窗（ADR 043 D6，2026-11-26 晚修订）：游戏所在屏幕上的一个小窗。出现时角上一定是她的头像（外面一圈进度条），
//! 旁边是**气泡**（一条简单的：她在听、在想、说了一句短话，推送的一行）或**聊天小窗**（复杂的：卡片、工具调用、长回答、
//! 同时来了几条；就是聊天页那段对话最近的几条，用聊天页的组件）。原来的说话气泡（ADR 018，窗口 `companion`）和右下角的
//! 消息提示（ADR 039 D4，窗口 `message-popup`）都并进了它。
//!
//! - **显示不显示、进度条**：纯状态在 [`model`]（气泡 6 秒、聊天小窗 10 秒，从最后一次变化算；鼠标在上面、她在听 / 在想 /
//!   在说时不走，移开接着走；走完整个收起；× 收到下一条推送）。旁边放气泡还是聊天小窗由界面按内容定，量好大小时一起报上来
//!   （[`floating_layout`] 的 `mini`）。她这一行的内容照旧是 `bubble.rs` 的状态机，它每次改状态都调 [`her`]（带上她在不在忙）；
//!   推送由 `agent/push.rs` 调 [`pushed`] / [`done`]。
//! - **在哪**：游戏（前台窗口）在哪块屏，就在哪块屏。没拖过贴着设置里的角，让开同一个角上的输入条（屏幕角落分配，
//!   `screen_corner.rs`）；拖过就照记下的位置（相对工作区，`assistant.bubble.position`，[`place`]）。窗口变大变小时贴着
//!   的那个角不动。窗口跟着内容的大小走：界面量好内容报上来（[`floating_layout`]），这里按那块屏的缩放摆；从收着到显示，
//!   等界面报了这一版的大小才显示，免得先用旧的大小闪一下（界面 1.5 秒还没报就照旧显示）。小窗最高是工作区高度的 40%。
//! - **硬规矩：绝不抢焦点**：窗口不可聚焦（`focusable: false`，Windows 上是 `WS_EX_NOACTIVATE`）、建的时候不要焦点、
//!   `SW_SHOWNA` 显示；点 ×、点卡片里的按钮都不激活它（消息提示真机验证过：不可聚焦照样点得到）。拖动是 Kotone 自己
//!   跟着光标挪（[`floating_drag`]，`SetWindowPos` 不激活），照语音输入的 `drag_overlay`。点卡片上面那一行、“打开 Kotone”
//!   才把主窗口拿到前面（`agent::auto_card_open`，玩家自己的动作）。
//! - **不显示的时候**：游戏独占全屏、演示模式（`attention::floating_hidden`）；推送还要看 `attention::popup_blocked`
//!   （Kotone 在前台、别的程序全屏时不弹，推送照样在聊天里）。设置里关了“显示悬浮窗”（`assistant.bubble.enabled`），
//!   推送不弹；她那一行照旧由 `bubble.rs` 按同一个开关决定。
//! - **不进截图**（`WDA_EXCLUDEFROMCAPTURE`）：她看屏幕、截图识别时不会把它认进去。真机脚本设 `KOTONE_FLOATING_CAPTURABLE=1`
//!   才截得到它。
//!
//! 线程约定（照 `bubble.rs`）：状态只在短临界区里改；窗口的建、显示、隐藏、摆位一律经 `run_on_main_thread` 在主线程按顺序
//! 执行，执行时读最新状态；不在持锁时等主线程。

mod model;
mod place;

use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use kotone_core::assistant::{BubbleConfig, BubbleCorner};
use kotone_core::log;
use kotone_plugin_api::PushLine;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::bubble::Phase;
use crate::SharedState;
use model::{Line, Model, View};

pub const LABEL: &str = "floating";
/// 给界面的样子（[`Shown`]）。
pub const EVENT: &str = "kotone://floating";
/// 窗口离工作区边缘多远（逻辑像素）。页面四周还留 8 给阴影，内容离边 24，和原来的气泡一样。
const MARGIN: f64 = 16.0;
/// 小窗最高占工作区高度的多少（讨论记录第三节：“屏幕高度的 40% 左右”）。
const MAX_HEIGHT_SHARE: f64 = 0.4;
/// 界面一直没报大小时，过多久照旧显示。
const LAYOUT_FALLBACK: Duration = Duration::from_millis(1_500);
/// 界面一直没报大小时用的大小（逻辑像素）。
const FALLBACK_SIZE: (f64, f64) = (376.0, 112.0);

#[derive(Default)]
struct State {
    model: Model,
    /// 界面报的内容大小（逻辑像素）。
    size: Option<(f64, f64)>,
    /// 界面报的挡鼠标的几块（头像、气泡 / 聊天小窗；逻辑像素，相对内容左上角，[`place::hit_rects`]）。
    boxes: Option<Vec<[f64; 4]>>,
    /// 上一次给窗口设的那几块（窗口句柄 + 物理像素；一样就不再设）。
    clipped: Option<(isize, Option<Vec<crate::screen_corner::Rect>>)>,
    /// 界面报大小时说的版本（最大的那个）。
    laid_out: u64,
    /// 从收着变成显示的那一版：界面报了这一版（或更新的）大小以后才显示。
    visible_since: u64,
    /// 上一次发给界面的样子是显示着的。
    visible: bool,
    /// 正在拖（拖完再摆）。
    dragging: bool,
    /// 倒计时的代数（新的倒计时作废旧的）。
    timer: u64,
    /// 窗口收拾过（关掉浏览器行为、不进截图）。
    prepared: bool,
}

fn state() -> &'static Mutex<State> {
    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    STATE.get_or_init(Default::default)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default()
}

/// 给界面的样子：显示什么、进度条（[`View`]），加上窗口贴着哪个角（头像在哪个角、内容往哪边长）、聊天小窗最高多少。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Shown {
    #[serde(flatten)]
    pub view: View,
    pub corner: BubbleCorner,
    /// 聊天小窗最高多少（逻辑像素）：游戏所在那块屏工作区高度的 40%。
    pub max_height: u32,
}

fn config(app: &AppHandle) -> BubbleConfig {
    app.try_state::<SharedState>()
        .map(|state| state.settings.read().unwrap().assistant.bubble.clone())
        .unwrap_or_default()
}

fn compose(app: &AppHandle, view: View) -> Shown {
    let config = config(app);
    let max_height = crate::screen_corner::foreground_monitor()
        .map(|(area, scale)| ((area.bottom - area.top) as f64 / scale * MAX_HEIGHT_SHARE) as u32)
        .unwrap_or(400);
    Shown {
        view,
        corner: config.position.map_or(config.corner, |p| p.corner),
        max_height,
    }
}

/// 现在的样子（不改状态）。
fn current(app: &AppHandle) -> Shown {
    let view = state().lock().unwrap().model.view(now_ms());
    compose(app, view)
}

/// 只是进度条变了（鼠标进出、拖动）：告诉界面，排下一次到点。窗口不用动。
fn announce(app: &AppHandle) {
    let view = state().lock().unwrap().model.view(now_ms());
    let _ = app.emit(EVENT, &compose(app, view));
    schedule(app);
}

/// 状态变了：告诉界面，按新的样子显示 / 收起，排下一次到点。
fn publish(app: &AppHandle) {
    let (view, waiting) = {
        let mut s = state().lock().unwrap();
        let view = s.model.view(now_ms());
        let visible = view.shown;
        let appearing = visible && !s.visible;
        if appearing {
            s.visible_since = view.revision;
        }
        s.visible = visible;
        (view, appearing.then_some(s.visible_since))
    };
    let _ = app.emit(EVENT, &compose(app, view));
    apply(app);
    schedule(app);
    if let Some(since) = waiting {
        // 界面一直没报大小（页面还没加载好、出错了）：照旧显示，不至于一直不出来
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(LAYOUT_FALLBACK).await;
            let late = {
                let mut s = state().lock().unwrap();
                let late = s.visible && s.visible_since == since && s.laid_out < since;
                if late {
                    s.laid_out = since;
                }
                late
            };
            if late {
                log::log("悬浮窗：界面没报大小，照旧显示");
                apply(&app);
            }
        });
    }
}

/// 排下一次到点（进度条走完、等插件说结束的那一行到时候；新的作废旧的）。
fn schedule(app: &AppHandle) {
    let (deadline, generation) = {
        let mut s = state().lock().unwrap();
        s.timer += 1;
        (s.model.next_deadline(), s.timer)
    };
    let Some(deadline) = deadline else {
        return;
    };
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(deadline.saturating_sub(now_ms()))).await;
        let changed = {
            let mut s = state().lock().unwrap();
            if s.timer != generation {
                return;
            }
            s.model.tick(now_ms())
        };
        if changed {
            publish(&app);
        } else {
            schedule(&app);
        }
    });
}

/// 按最新的状态显示 / 收起 / 摆位。在主线程上按顺序执行。
fn apply(app: &AppHandle) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        let (visible, ready, size, boxes, dragging) = {
            let s = state().lock().unwrap();
            let boxes = s.boxes.clone();
            (
                s.visible,
                s.laid_out >= s.visible_since,
                s.size,
                boxes,
                s.dragging,
            )
        };
        let window = match handle.get_webview_window(LABEL) {
            Some(window) => window,
            None if visible => match create(&handle) {
                Some(window) => window,
                None => return,
            },
            None => return,
        };
        prepare(&window);
        if !visible || crate::attention::floating_hidden().is_some() {
            crate::hide_window(&window);
            return;
        }
        if !ready {
            return;
        }
        if !dragging {
            match place_window(&handle, &window, size.unwrap_or(FALLBACK_SIZE)) {
                Ok((scale, physical)) => clip(&window, boxes.as_deref(), scale, physical),
                Err(e) => log::log(&format!("悬浮窗：摆不好位置：{e}")),
            }
        }
        crate::show_window_no_focus(&window);
    });
}

/// 窗口通常在启动时就建好了（`create_startup_windows`，她一开口就要出来）；没有时（还没启用她那会儿）第一次要显示再建。
fn create(app: &AppHandle) -> Option<tauri::WebviewWindow> {
    let config = app
        .config()
        .app
        .windows
        .iter()
        .find(|c| c.label == LABEL)
        .cloned()?;
    match tauri::WebviewWindowBuilder::from_config(app, &config).and_then(|b| b.build()) {
        Ok(window) => Some(window),
        Err(e) => {
            log::log(&format!("悬浮窗：窗口建不起来：{e}"));
            None
        }
    }
}

/// 第一次用之前收拾一下窗口：关掉浏览器行为（右键菜单、快捷键），不进截图。
fn prepare(window: &tauri::WebviewWindow) {
    {
        let mut s = state().lock().unwrap();
        if s.prepared {
            return;
        }
        s.prepared = true;
    }
    crate::harden_webview(window);
    if std::env::var_os("KOTONE_FLOATING_CAPTURABLE").is_none() {
        crate::exclude_from_capture(window);
    }
}

/// 摆到游戏所在那块屏上：拖过就照记下的位置，没拖过贴着设置里的角、让开输入条。`size` 是内容的逻辑大小。
/// 回那块屏的缩放和窗口的物理大小。
fn place_window(
    app: &AppHandle,
    window: &tauri::WebviewWindow,
    size: (f64, f64),
) -> Result<(f64, (i32, i32)), String> {
    use crate::screen_corner;
    let config = config(app);
    let (area, scale) = screen_corner::foreground_monitor().ok_or("无法确定显示器")?;
    let px = |logical: f64| (logical * scale).round() as i32;
    let size = (
        px(size.0).clamp(1, (area.right - area.left).max(1)),
        px(size.1).clamp(1, (area.bottom - area.top).max(1)),
    );
    let rect = match config.position {
        Some(position) => {
            screen_corner::set_free(LABEL, true);
            place::at(position, area, size)
        }
        None => {
            screen_corner::set_free(LABEL, false);
            let inner = screen_corner::inner_rects(app, LABEL);
            screen_corner::stack(
                area,
                place::corner_of(config.corner),
                size,
                (px(MARGIN), px(MARGIN + config.offset_y as f64)),
                px(screen_corner::GAP),
                &inner,
            )
        }
    };
    let position = tauri::PhysicalPosition::new(rect.left, rect.top);
    // 先挪到那块屏再定大小、再挪一次：换到缩放不同的屏时，系统按新缩放改过一次大小（WM_DPICHANGED）
    window.set_position(position).map_err(|e| e.to_string())?;
    window
        .set_size(tauri::PhysicalSize::new(size.0 as u32, size.1 as u32))
        .map_err(|e| e.to_string())?;
    window.set_position(position).map_err(|e| e.to_string())?;
    Ok((scale, size))
}

/// 窗口只留挡鼠标的那几块（头像、气泡 / 聊天小窗，[`place::hit_rects`]；`SetWindowRgn`）：透明的地方——聊天小窗旁边
/// 头像上面那一条——不再挡住下面游戏的点击（Windows 按窗口区域判断点的是谁，区域外的点直接落到下面的窗口）。和窗口大小
/// 一起改（界面每次报大小都带着这几块）；拖动只挪窗口，区域跟着窗口走。没报的时候整个窗口都挡。
#[cfg(windows)]
fn clip(window: &tauri::WebviewWindow, boxes: Option<&[[f64; 4]]>, scale: f64, size: (i32, i32)) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Gdi::{
        CombineRgn, CreateRectRgn, DeleteObject, SetWindowRgn, RGN_OR,
    };
    let Ok(hwnd) = window.hwnd() else {
        return;
    };
    let raw = hwnd.0 as isize;
    let rects = boxes.and_then(|b| place::hit_rects(b, scale, size));
    {
        let mut s = state().lock().unwrap();
        if s.clipped.as_ref() == Some(&(raw, rects.clone())) {
            return;
        }
        s.clipped = Some((raw, rects.clone()));
    }
    let hwnd = HWND(raw as _);
    // SAFETY: 只改这个窗口的区域；SetWindowRgn 成功后区域归系统（不再用、不删），失败了自己删；拼区域用的那几块用完就删
    unsafe {
        let Some(rects) = rects else {
            SetWindowRgn(hwnd, None, true);
            return;
        };
        let region = CreateRectRgn(0, 0, 0, 0);
        for r in &rects {
            let part = CreateRectRgn(r.left, r.top, r.right, r.bottom);
            CombineRgn(Some(region), Some(region), Some(part), RGN_OR);
            let _ = DeleteObject(part.into());
        }
        if SetWindowRgn(hwnd, Some(region), true) == 0 {
            let _ = DeleteObject(region.into());
            log::log("悬浮窗：设不了窗口区域，整个窗口照旧挡鼠标");
            state().lock().unwrap().clipped = None;
        }
    }
}

#[cfg(not(windows))]
fn clip(
    _window: &tauri::WebviewWindow,
    _boxes: Option<&[[f64; 4]]>,
    _scale: f64,
    _size: (i32, i32),
) {
}

// ---------- 别处调的 ----------

/// 她这一行变了（`bubble.rs` 每次改状态、念完一句都调；`None` = 那边收起了）。`active`：她在听 / 在想 / 在说 / 等你回答
/// （进度条停着）。内容在 `kotone://bubble` 里，界面自己听。
pub fn her(app: &AppHandle, phase: Option<Phase>, active: bool) {
    let changed = state().lock().unwrap().model.her(phase, active, now_ms());
    if changed {
        publish(app);
    }
}

/// 插件推送了一条（`agent/push.rs`：记进对话、或排着等她说完以后）。
pub struct Pushed<'a> {
    pub plugin: &'a str,
    pub plugin_name: &'a str,
    pub conversation_id: &'a str,
    pub key: &'a str,
    /// 一行（截过的）。
    pub line: Option<&'a PushLine>,
    /// 清单里这条写了 `until_done`（一行显示到插件说结束）。
    pub until_done: bool,
    /// 有卡片。
    pub card: bool,
    /// 这个 key 第一次推（不是更新）。
    pub posted: bool,
    /// 带“要提醒”的更新：小窗再弹一次。
    pub alert: bool,
}

/// 推送来了：一行显示在气泡里；新的卡片（或带 alert 的更新）开着“推送时弹出小窗”时弹出聊天小窗、定位到它；
/// 别的原地更新（聊天小窗开着时）让进度条从头走。关着“显示悬浮窗”、现在不能弹（`attention::popup_blocked`）时什么都不做，
/// 推送照样在聊天里。
pub fn pushed(app: &AppHandle, push: Pushed) {
    let Some(state_handle) = app.try_state::<SharedState>() else {
        return;
    };
    let (enabled, popup) = {
        let settings = state_handle.settings.read().unwrap();
        (
            settings.assistant.bubble.enabled,
            settings.agent.auto_cards.popup,
        )
    };
    let pop = push.card && popup && (push.posted || push.alert);
    if !enabled {
        return;
    }
    if push.line.is_none() && !pop {
        // 没弹出的原地更新：聊天小窗开着时小窗里那张变了
        let changed = !push.posted && state().lock().unwrap().model.updated(now_ms());
        if changed {
            publish(app);
        }
        return;
    }
    if let Some(why) = crate::attention::popup_blocked() {
        log::log(&format!(
            "悬浮窗：没弹（{why}），推送照样在聊天里：{} {}",
            push.plugin_name, push.key
        ));
        return;
    }
    let now = now_ms();
    let changed = {
        let mut s = state().lock().unwrap();
        let mut changed = false;
        if let Some(line) = push.line {
            changed |= s.model.line(
                Line::new(
                    push.plugin,
                    push.plugin_name,
                    push.key,
                    push.conversation_id,
                    line.cells.clone(),
                    push.until_done,
                ),
                now,
            );
        }
        if pop {
            changed |= s
                .model
                .card(push.conversation_id, push.plugin, push.key, now);
        }
        changed
    };
    if changed {
        publish(app);
    }
}

/// 插件说这条推送结束了（`Host::push_done`）：收起它的那一行。
pub fn done(app: &AppHandle, plugin: &str, key: &str) {
    let changed = state()
        .lock()
        .unwrap()
        .model
        .done(&model::id(plugin, key), now_ms());
    if changed {
        publish(app);
    }
}

/// 玩家打开了 Kotone（点了卡片上面那一行、“打开 Kotone”）：都在聊天里了，和 × 一样整个收起。
pub fn opened(app: &AppHandle) {
    let changed = state().lock().unwrap().model.close(now_ms());
    if changed {
        publish(app);
    }
}

// ---------- 界面调的 ----------

/// 界面刚加载好：拿现在的样子（窗口第一次建的时候，事件可能比页面先到）。
#[tauri::command]
pub fn floating_current(app: AppHandle) -> Shown {
    current(&app)
}

/// 界面量好了内容的大小（逻辑像素；每画一版都报，内容变大变小时也报）：按它摆、该显示就显示。`mini`：头像旁边放的是
/// 聊天小窗（false = 气泡；旧界面不报）——换了样子进度条从头走，小窗的一圈长一些。`boxes`：挡鼠标的那几块
/// （`[x, y, 宽, 高]`，相对内容左上角，带着阴影那一圈；旧界面不报 = 整个窗口都挡），别的地方点得到下面的游戏。
#[tauri::command]
pub fn floating_layout(
    app: AppHandle,
    revision: u64,
    width: f64,
    height: f64,
    mini: Option<bool>,
    boxes: Option<Vec<[f64; 4]>>,
) {
    let reshaped = {
        let mut s = state().lock().unwrap();
        if width.is_finite() && height.is_finite() {
            s.size = Some((width.max(1.0), height.max(1.0)));
            s.boxes = boxes;
        }
        s.laid_out = s.laid_out.max(revision);
        mini.is_some_and(|mini| s.model.shape(mini, revision, now_ms()))
    };
    if reshaped {
        announce(&app);
    }
    apply(&app);
}

/// 鼠标进出悬浮窗：在上面时进度条停住，移开接着走。
#[tauri::command]
pub fn floating_hover(app: AppHandle, inside: bool) {
    let changed = state().lock().unwrap().model.hover(inside, now_ms());
    if changed {
        announce(&app);
    }
}

/// 玩家点了 ×：现在显示着的都收起，直到下一条推送。
#[tauri::command]
pub fn floating_close(app: AppHandle) {
    let changed = state().lock().unwrap().model.close(now_ms());
    if changed {
        publish(&app);
    }
}

/// 拖动悬浮窗（页面在头像、气泡、聊天小窗上面那一行按下左键时调）：照语音输入的 `drag_overlay`，按着左键时每 8 毫秒看一次光标，
/// 挪过 4 个像素（逻辑像素）才算拖（只是点一下不动它，照 dsh-orb-cordis 的拖动门槛），用原始 `SetWindowPos` 挪
/// （不激活、不改叠放次序）；跟着光标走的是按下那一刻光标到窗口的距离（dsh-orb-cordis 的 `grab`：每次都从光标算，
/// 窗口挪了不会反过来影响下一步）。松开就停（最多跟 60 秒），记下拖到的位置（相对那块屏的工作区，[`place::remember`]），
/// 再整个拉回工作区里。拖着时进度条停住。窗口不可聚焦，拖完不用还前台。
#[tauri::command]
pub fn floating_drag(app: AppHandle) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::sync::atomic::{AtomicBool, Ordering};
        static DRAGGING: AtomicBool = AtomicBool::new(false);
        let window = app
            .get_webview_window(LABEL)
            .ok_or_else(|| "悬浮窗不存在".to_string())?;
        let hwnd = window.hwnd().map_err(|e| e.to_string())?.0 as isize;
        if DRAGGING.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        std::thread::spawn(move || {
            let moved = follow_cursor(&app, windows::Win32::Foundation::HWND(hwnd as _));
            if moved {
                remember_position(&app, &window);
            }
            {
                let mut s = state().lock().unwrap();
                s.dragging = false;
                s.model.hold(false, now_ms());
            }
            DRAGGING.store(false, Ordering::SeqCst);
            apply(&app);
            announce(&app);
        });
    }
    #[cfg(not(windows))]
    let _ = app;
    Ok(())
}

/// 按着左键时窗口跟着光标走；返回挪过没有（只是点了一下就没挪）。
#[cfg(windows)]
fn follow_cursor(app: &AppHandle, hwnd: windows::Win32::Foundation::HWND) -> bool {
    use windows::Win32::Foundation::{POINT, RECT};
    use windows::Win32::UI::HiDpi::GetDpiForWindow;
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON, VK_RBUTTON};
    use windows::Win32::UI::WindowsAndMessaging::{
        GetCursorPos, GetSystemMetrics, GetWindowRect, SetWindowPos, SM_SWAPBUTTON, SWP_NOACTIVATE,
        SWP_NOSIZE, SWP_NOZORDER,
    };
    // 左右键对调了的话，主键是物理上的右键（GetAsyncKeyState 看的是物理按键）
    // SAFETY: 只读光标、按键状态、这个窗口的位置和 DPI，只挪这个窗口
    let button = if unsafe { GetSystemMetrics(SM_SWAPBUTTON) } != 0 {
        VK_RBUTTON
    } else {
        VK_LBUTTON
    };
    let pressed = || unsafe { GetAsyncKeyState(button.0 as i32) } < 0;
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    let threshold = (4 * dpi.max(96) / 96) as i32;
    let mut start = POINT::default();
    let mut rect = RECT::default();
    if unsafe { GetCursorPos(&mut start).is_err() || GetWindowRect(hwnd, &mut rect).is_err() } {
        return false;
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    let mut last = start;
    let mut moved = false;
    while pressed() && std::time::Instant::now() < deadline {
        let mut now = POINT::default();
        if unsafe { GetCursorPos(&mut now).is_ok() } && (now.x != last.x || now.y != last.y) {
            let (dx, dy) = (now.x - start.x, now.y - start.y);
            if !moved && dx.abs().max(dy.abs()) > threshold {
                moved = true;
                {
                    let mut s = state().lock().unwrap();
                    s.dragging = true;
                    s.model.hold(true, now_ms());
                }
                announce(app);
            }
            if moved {
                let _ = unsafe {
                    SetWindowPos(
                        hwnd,
                        None,
                        rect.left + dx,
                        rect.top + dy,
                        0,
                        0,
                        SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                    )
                };
                last = now;
            }
        }
        std::thread::sleep(Duration::from_millis(8));
    }
    moved
}

/// 拖完：记下窗口在它那块屏工作区里的位置（`assistant.bubble.position`），以后就从这里出现、不再贴角排队。
#[cfg(windows)]
fn remember_position(app: &AppHandle, window: &tauri::WebviewWindow) {
    let (Some(rect), Some((area, _))) = (
        crate::screen_corner::shown_rect(window),
        crate::screen_corner::window_area(window),
    ) else {
        return;
    };
    let position = place::remember(rect, area);
    let Some(shared) = app.try_state::<SharedState>() else {
        return;
    };
    match shared.settings_repository.update(|next| {
        next.assistant.bubble.position = Some(position);
        Ok(())
    }) {
        Ok(updated) => {
            crate::screen_corner::set_free(LABEL, true);
            log::log(&format!(
                "悬浮窗：拖到了 {:?} ({}, {})（万分之一）",
                position.corner, position.x, position.y
            ));
            let _ = app.emit("kotone://settings-changed", &updated);
        }
        Err(e) => log::log(&format!("悬浮窗：位置没记下：{e}")),
    }
}

/// 设置页点了“位置还原”以后：照现在的设置重摆一次（显示着的话）。
#[tauri::command]
pub fn floating_reposition(app: AppHandle) {
    apply(&app);
}
