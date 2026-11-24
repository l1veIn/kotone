//! 游戏上方的覆盖层（ADR 038 D5 的第三个显示位置）：**只有一个，在本体里**。插件只说放什么、放在哪、什么时候显示
//! （[`kotone_plugin_api::OverlayItem`]），难的部分在这里写一次：
//!
//! - 一个透明、无边框、置顶、点击穿透、不抢焦点的窗口（`plugin-overlay`，和弹幕窗口同一套做法），**只盖住条目所在的
//!   那一块**（所有条目的并集加一点边），不盖满整个游戏：照 aramhex 的显示性能排查，盖在无边框游戏上的透明置顶窗口
//!   可能让游戏退出 Independent Flip、帧率被压到刷新率，窗口越小、显示的时间越短越好；
//! - 只在目标窗口在前台时显示（游戏切走、最小化就藏起来），每秒跟一次位置和大小（多屏、缩放按物理像素）；
//! - 窗口设成不进截图（`SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)`，Windows 10 2004 起）：截图识别
//!   （读聊天、三选一识别海克斯）不会把自己画的东西认进去，照 aramhex“识别时遮掉自己画的东西”；
//! - 独占全屏显示不了（和弹幕、aramhex 一样，要玩家改成无边框）。
//!
//! 插件之间互不影响：每个插件一份条目，换的时候只换自己的。

use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use kotone_core::tools::{PhysicalRect, WindowObserver};
use kotone_platform_windows::window_tools::WindowsWindowObserver;
use kotone_plugin_api::{OverlayItem, OverlayTarget};
use serde_json::json;
use tauri::{AppHandle, Emitter, Manager};

pub const WINDOW: &str = "plugin-overlay";
pub const EVENT: &str = "kotone://plugin-overlay";
/// 显示时多久跟一次目标窗口。
const FOLLOW: Duration = Duration::from_secs(1);
/// 并集外面留的边（客户区的比例）。
const PAD: f64 = 0.005;

/// 条目的并集（客户区比例），加一点边、夹在客户区里。
pub fn union(rects: &[[f64; 4]]) -> Option<[f64; 4]> {
    let first = rects.first()?;
    let (mut x0, mut y0) = (first[0], first[1]);
    let (mut x1, mut y1) = (first[0] + first[2], first[1] + first[3]);
    for r in rects {
        x0 = x0.min(r[0]);
        y0 = y0.min(r[1]);
        x1 = x1.max(r[0] + r[2]);
        y1 = y1.max(r[1] + r[3]);
    }
    let (x0, y0) = ((x0 - PAD).max(0.0), (y0 - PAD).max(0.0));
    let (x1, y1) = ((x1 + PAD).min(1.0), (y1 + PAD).min(1.0));
    (x1 > x0 && y1 > y0).then_some([x0, y0, x1 - x0, y1 - y0])
}

/// 客户区比例的一块 → 相对窗口（并集）的比例。
pub fn relative(rect: [f64; 4], to: [f64; 4]) -> [f64; 4] {
    [
        (rect[0] - to[0]) / to[2],
        (rect[1] - to[1]) / to[3],
        rect[2] / to[2],
        rect[3] / to[3],
    ]
}

#[derive(Default)]
struct Layer {
    /// 插件 id → 它现在要显示的条目。
    items: BTreeMap<String, Vec<OverlayItem>>,
    /// 现在盖在哪（物理像素）；None = 藏着。
    shown_at: Option<PhysicalRect>,
}

fn layer() -> &'static Mutex<Layer> {
    static LAYER: OnceLock<Mutex<Layer>> = OnceLock::new();
    LAYER.get_or_init(Default::default)
}

/// 换掉某个插件的覆盖层条目（空 = 这个插件什么都不显示）。
pub fn set(app: &AppHandle, plugin: &str, items: Vec<OverlayItem>) {
    {
        let mut l = layer().lock().unwrap();
        if items.is_empty() {
            l.items.remove(plugin);
        } else {
            l.items.insert(plugin.to_string(), items);
        }
    }
    refresh(app);
}

/// 条目的目标窗口在不在前台：在的话返回它的客户区。
fn target_rect(target: &OverlayTarget) -> Option<PhysicalRect> {
    let observer = WindowsWindowObserver;
    let foreground = observer.foreground_window().ok().flatten()?;
    if foreground.minimized || foreground.cloaked {
        return None;
    }
    let process = foreground.process_name.as_deref().unwrap_or("");
    let hit = match target {
        OverlayTarget::Process(exe) => process.eq_ignore_ascii_case(exe),
        OverlayTarget::AnyOf(list) => list.iter().any(|exe| process.eq_ignore_ascii_case(exe)),
    };
    hit.then_some(foreground.client_rect).flatten()
}

/// 按现在的条目和前台窗口摆好、显示或藏起覆盖层，并把条目发给它。
fn refresh(app: &AppHandle) {
    let (visible, rect) = {
        let l = layer().lock().unwrap();
        let mut rect = None;
        let visible: Vec<_> = l
            .items
            .iter()
            .flat_map(|(plugin, items)| items.iter().map(move |i| (plugin.clone(), i.clone())))
            .filter(|(_, item)| match target_rect(&item.target) {
                Some(r) => {
                    rect.get_or_insert(r);
                    true
                }
                None => false,
            })
            .collect();
        (visible, rect)
    };
    if visible.is_empty() && layer().lock().unwrap().shown_at.is_none() {
        return;
    }
    let Some(window) = app.get_webview_window(WINDOW).or_else(|| create(app)) else {
        return;
    };
    let rects: Vec<[f64; 4]> = visible.iter().map(|(_, item)| item.anchor.rect()).collect();
    let area = union(&rects);
    // 客户区 → 窗口实际占的那一块（物理像素）
    let rect = match (rect, area) {
        (Some(client), Some([x, y, w, h])) => Some(PhysicalRect {
            x: client.x + (x * f64::from(client.width)).round() as i32,
            y: client.y + (y * f64::from(client.height)).round() as i32,
            width: ((w * f64::from(client.width)).round() as u32).max(1),
            height: ((h * f64::from(client.height)).round() as u32).max(1),
        }),
        _ => None,
    };
    match rect {
        Some(rect) if !visible.is_empty() => {
            let moved = layer().lock().unwrap().shown_at != Some(rect);
            if moved {
                let _ = window.set_position(tauri::PhysicalPosition::new(rect.x, rect.y));
                let _ = window.set_size(tauri::PhysicalSize::new(rect.width, rect.height));
            }
            let _ = window.set_ignore_cursor_events(true);
            exclude_from_capture(&window);
            let area = area.unwrap_or([0.0, 0.0, 1.0, 1.0]);
            let items: Vec<_> = visible
                .iter()
                .map(|(plugin, item)| {
                    json!({"plugin": plugin, "item": item, "rect": relative(item.anchor.rect(), area)})
                })
                .collect();
            let _ = app.emit_to(
                WINDOW,
                EVENT,
                json!({"items": items, "width": rect.width, "height": rect.height}),
            );
            if layer().lock().unwrap().shown_at.is_none() {
                crate::show_window_no_focus(&window);
            }
            layer().lock().unwrap().shown_at = Some(rect);
        }
        _ => {
            if layer().lock().unwrap().shown_at.take().is_some() {
                crate::hide_window(&window);
                let _ = app.emit_to(WINDOW, EVENT, json!({"items": []}));
            }
        }
    }
}

/// 第一次要显示时才建窗口（`tauri.conf.json` 里 `create: false`）：没有插件用覆盖层时不多占一个网页进程。
fn create(app: &AppHandle) -> Option<tauri::WebviewWindow> {
    let config = app
        .config()
        .app
        .windows
        .iter()
        .find(|c| c.label == WINDOW)
        .cloned()?;
    match tauri::WebviewWindowBuilder::from_config(app, &config).and_then(|b| b.build()) {
        Ok(window) => {
            let _ = window.set_ignore_cursor_events(true);
            Some(window)
        }
        Err(e) => {
            kotone_core::log::log(&format!("覆盖层窗口建不起来：{e}"));
            None
        }
    }
}

/// 不进截图（自己的截图识别、录屏软件的整屏捕获都看不到它；游戏捕获照常只录游戏）。
fn exclude_from_capture(window: &tauri::WebviewWindow) {
    #[cfg(windows)]
    if let Ok(hwnd) = window.hwnd() {
        use windows::Win32::UI::WindowsAndMessaging::{
            SetWindowDisplayAffinity, WDA_EXCLUDEFROMCAPTURE,
        };
        // SAFETY: hwnd 来自活着的 Tauri 窗口；失败（老系统）只是截图里会有它，不影响显示。
        unsafe {
            let _ = SetWindowDisplayAffinity(
                windows::Win32::Foundation::HWND(hwnd.0),
                WDA_EXCLUDEFROMCAPTURE,
            );
        }
    }
    #[cfg(not(windows))]
    let _ = window;
}

/// 启动时：有条目时每秒跟一次前台窗口（切走就藏、回来再显示、窗口挪了跟着挪）。
pub fn start(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(FOLLOW).await;
            let any = !layer().lock().unwrap().items.is_empty();
            if any || layer().lock().unwrap().shown_at.is_some() {
                refresh(&app);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_covers_only_the_items() {
        assert_eq!(union(&[]), None);
        let cards = [
            [0.2, 0.3, 0.14, 0.05],
            [0.43, 0.3, 0.14, 0.05],
            [0.66, 0.31, 0.14, 0.05],
        ];
        let u = union(&cards).unwrap();
        assert!((u[0] - 0.195).abs() < 1e-9 && (u[1] - 0.295).abs() < 1e-9);
        assert!((u[0] + u[2] - 0.805).abs() < 1e-9 && (u[1] + u[3] - 0.365).abs() < 1e-9);
        assert!(u[2] < 0.7 && u[3] < 0.08, "不盖满整个游戏：{u:?}");
        let r = relative(cards[0], u);
        assert!(r[0] > 0.0 && r[0] < 0.02 && r[1] > 0.0 && r[2] < 0.25);
        // 贴边的不越界
        let edge = union(&[[0.0, 0.0, 0.2, 0.3]]).unwrap();
        assert_eq!(edge[0], 0.0);
        assert_eq!(edge[1], 0.0);
    }
}
