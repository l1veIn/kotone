//! 屏幕角落分配（共享资源评估第四节“屏幕”：屏幕角落按位置分配，右下角排成一列）。盖在游戏上、摆在角落的小窗有两个：
//! 语音输入的输入条（玩家选了角上的预设、或拖到了角上）、悬浮窗（ADR 043 D6：她的气泡、推送的一行和小窗，原来的说话气泡
//! 和消息提示并进了它）。谁贴着角、谁往外让，只在这里写：
//!
//! - **从角落往外的顺序**（[`ORDER`]）：输入条 → 悬浮窗。输入条的位置是玩家自己选的、能点能拖，从不挪；悬浮窗让着它。
//!   照 Steam 覆盖层：游戏自己的界面占着那个角时，通知离开角落让出来（`ISteamUtils::SetOverlayNotificationInset`，
//!   <https://partner.steamgames.com/doc/api/ISteamUtils>）——来来去去的东西让着常驻在那儿的东西；同一个角的往上叠
//!   （Windows 的通知也在右下角、任务栏上面一个个往上叠，
//!   <https://learn.microsoft.com/windows/apps/design/shell/tiles-and-notifications/toast-ux-guidance>）。
//! - **怎么让**（[`stack`]、[`clear`]）：先贴着角放；和比它靠角落的、正显示着的哪个窗口叠上了（隔不到 [`GAP`] 也算），
//!   就跳到那个窗口外面（底下的角往上跳，上面的角往下跳），再看，直到谁都不叠。不在同一列的（另一个角的、拖到屏幕
//!   中间的输入条）不碍事；屏幕太小放不下时停在工作区另一头（还叠着，但不出屏幕）。
//! - **什么时候挪**：每个窗口显示时自己按上面摆（悬浮窗每次变大变小都重摆）；比它靠角落的窗口后出来时（[`shown`]，
//!   `show_window_no_focus` 每次都调），外面的要是被叠上了就挪出去，没叠上的不动。收起时不往回挪。
//! - **拖过的不排**（[`set_free`]）：玩家把悬浮窗拖到了别处，它就在那儿，不再贴角、不让输入条；输入条照旧在最里面、从不被挪。
//!
//! 不归这里的：弹幕窗口是整块屏幕大的透明窗口，字在上面那条道上飘（离上边 `danmakuTopOffsetPx`），不占角落。

use tauri::Manager;

/// 从角落往外的顺序（窗口的 label）：前面的贴着角，后面的让着前面的。
pub const ORDER: [&str; 2] = ["overlay", "floating"];
/// 同一列里两个窗口之间至少隔多少（逻辑像素）。
pub const GAP: f64 = 12.0;

/// 屏幕上的一块（物理像素；右、下不含）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub fn at(left: i32, top: i32, (width, height): (i32, i32)) -> Self {
        Self {
            left,
            top,
            right: left + width,
            bottom: top + height,
        }
    }

    fn height(&self) -> i32 {
        self.bottom - self.top
    }

    fn grown(&self, by: i32) -> Self {
        Self {
            left: self.left - by,
            top: self.top - by,
            right: self.right + by,
            bottom: self.bottom + by,
        }
    }

    fn overlaps(&self, other: &Rect) -> bool {
        self.left < other.right
            && other.left < self.right
            && self.top < other.bottom
            && other.top < self.bottom
    }

    fn moved_to(&self, top: i32) -> Self {
        Self {
            top,
            bottom: top + self.height(),
            ..*self
        }
    }
}

/// 窗口贴着哪个角。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Corner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl Corner {
    fn right(self) -> bool {
        matches!(self, Corner::TopRight | Corner::BottomRight)
    }

    fn bottom(self) -> bool {
        matches!(self, Corner::BottomLeft | Corner::BottomRight)
    }
}

/// 摆一个 `size` 大的窗口：贴着工作区 `area` 的 `corner`（离左右边 `margin.0`、离上下边 `margin.1`），
/// 再让开 `inner`（比它靠角落的、正显示着的窗口，见 [`clear`]）。
pub fn stack(
    area: Rect,
    corner: Corner,
    size: (i32, i32),
    margin: (i32, i32),
    gap: i32,
    inner: &[Rect],
) -> Rect {
    let left = if corner.right() {
        area.right - size.0 - margin.0
    } else {
        area.left + margin.0
    };
    let top = if corner.bottom() {
        area.bottom - size.1 - margin.1
    } else {
        area.top + margin.1
    };
    clear(Rect::at(left, top, size), corner.bottom(), area, gap, inner)
}

/// 让开：`rect` 和 `inner` 里的哪个叠上了（隔不到 `gap` 也算），就跳到它外面（`upward`：跳到它上面，
/// 否则跳到它下面），直到谁都不叠；跳到工作区另一头还放不下就停在那头。谁都不叠时原样返回。
pub fn clear(rect: Rect, upward: bool, area: Rect, gap: i32, inner: &[Rect]) -> Rect {
    let taken: Vec<Rect> = inner.iter().map(|r| r.grown(gap)).collect();
    let mut rect = rect;
    // 每跳一次都整个越过一个窗口、不往回跳，所以最多跳 inner.len() 次
    for _ in 0..taken.len() {
        let Some(hit) = taken.iter().find(|t| t.overlaps(&rect)) else {
            break;
        };
        rect = rect.moved_to(if upward {
            hit.top - rect.height()
        } else {
            hit.bottom
        });
    }
    let lowest = (area.bottom - rect.height()).max(area.top);
    rect.moved_to(rect.top.clamp(area.top, lowest))
}

/// 拖到了别处、不排队的角落窗口（[`set_free`]）。
fn free() -> &'static std::sync::Mutex<Vec<&'static str>> {
    static FREE: std::sync::OnceLock<std::sync::Mutex<Vec<&'static str>>> =
        std::sync::OnceLock::new();
    FREE.get_or_init(Default::default)
}

/// 这个角落窗口是不是玩家拖到了别处（`free`）：是的话它不再贴角、不让里面的窗口（[`settle`] 不挪它）。
pub fn set_free(label: &'static str, is_free: bool) {
    let mut free = free().lock().unwrap();
    free.retain(|l| *l != label);
    if is_free {
        free.push(label);
    }
}

fn is_free(label: &str) -> bool {
    free().lock().unwrap().contains(&label)
}

/// 一个窗口刚显示出来（`show_window_no_focus` 调）：它是角落窗口的话，在主线程上把被它叠上的外面的窗口挪出去
/// （[`settle`]）。不在主线程时排在显示之后（同一个队列，先进先出）。
pub fn shown<R: tauri::Runtime>(window: &tauri::WebviewWindow<R>) {
    if !ORDER.contains(&window.label()) {
        return;
    }
    let app = window.app_handle().clone();
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || settle(&handle));
}

/// 从里往外看一遍：哪个窗口和比它靠角落的、正显示着的窗口叠上了，就挪出去（[`clear`]）；没叠上的不动。
fn settle<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    let mut shown: Vec<Option<Rect>> = ORDER
        .iter()
        .map(|label| app.get_webview_window(label).and_then(|w| shown_rect(&w)))
        .collect();
    for (i, label) in ORDER.iter().enumerate().skip(1) {
        let Some(rect) = shown[i] else { continue };
        if is_free(label) {
            continue;
        }
        let Some(window) = app.get_webview_window(label) else {
            continue;
        };
        let Some((area, scale)) = window_area(&window) else {
            continue;
        };
        let inner: Vec<Rect> = shown[..i].iter().flatten().copied().collect();
        // 在工作区下半截的是底下的角，往上让
        let upward = rect.top + rect.bottom > area.top + area.bottom;
        let moved = clear(rect, upward, area, (GAP * scale).round() as i32, &inner);
        if moved != rect {
            let _ = window.set_position(tauri::PhysicalPosition::new(moved.left, moved.top));
            shown[i] = Some(moved);
        }
    }
}

/// 比 `label` 靠角落的、正显示着的窗口占的地方。
pub(crate) fn inner_rects<R: tauri::Runtime>(app: &tauri::AppHandle<R>, label: &str) -> Vec<Rect> {
    ORDER
        .iter()
        .take_while(|l| **l != label)
        .filter_map(|l| app.get_webview_window(l).and_then(|w| shown_rect(&w)))
        .collect()
}

/// 窗口显示着（没最小化）时它占的地方。直接问系统：显示 / 隐藏走的是原始的 `SW_SHOWNA` / `SW_HIDE`，
/// tao 记的可见状态不准（见 `window.rs` 的 `hide_window`）。
#[cfg(windows)]
pub(crate) fn shown_rect<R: tauri::Runtime>(window: &tauri::WebviewWindow<R>) -> Option<Rect> {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::UI::WindowsAndMessaging::{GetWindowRect, IsIconic, IsWindowVisible};
    let hwnd = window.hwnd().ok()?;
    let mut r = RECT::default();
    // SAFETY: 只读这个窗口的可见状态和位置
    unsafe {
        if !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
            return None;
        }
        GetWindowRect(hwnd, &mut r).ok()?;
    }
    Some(Rect {
        left: r.left,
        top: r.top,
        right: r.right,
        bottom: r.bottom,
    })
}

#[cfg(not(windows))]
pub(crate) fn shown_rect<R: tauri::Runtime>(window: &tauri::WebviewWindow<R>) -> Option<Rect> {
    if !window.is_visible().ok()? {
        return None;
    }
    let p = window.outer_position().ok()?;
    let s = window.outer_size().ok()?;
    Some(Rect::at(p.x, p.y, (s.width as i32, s.height as i32)))
}

/// 前台窗口（游戏）所在显示器的工作区（物理像素，避开任务栏）和那块屏的缩放：角落的小窗出现在玩家正在用的那块屏上。
/// 只问系统（`MonitorFromWindow`、`GetDpiForMonitor`），哪个线程都能调。
#[cfg(windows)]
pub(crate) fn foreground_monitor() -> Option<(Rect, f64)> {
    use windows::Win32::Graphics::Gdi::{MonitorFromWindow, MONITOR_DEFAULTTOPRIMARY};
    use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
    use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
    // SAFETY: 只读当前前台窗口所在的显示器和它的 DPI（窗口没了时 MONITOR_DEFAULTTOPRIMARY 给主屏）
    let monitor = unsafe { MonitorFromWindow(GetForegroundWindow(), MONITOR_DEFAULTTOPRIMARY) };
    let area = monitor_info_work_area(monitor)?;
    let (mut x, mut y) = (0u32, 0u32);
    let scale = match unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut x, &mut y) } {
        Ok(()) if x > 0 => x as f64 / 96.0,
        _ => 1.0,
    };
    Some((area, scale))
}

#[cfg(not(windows))]
pub(crate) fn foreground_monitor() -> Option<(Rect, f64)> {
    None
}

/// 窗口自己所在显示器的工作区和缩放（挪已经显示着的窗口、拖完记位置时用）。
#[cfg(windows)]
pub(crate) fn window_area<R: tauri::Runtime>(
    window: &tauri::WebviewWindow<R>,
) -> Option<(Rect, f64)> {
    use windows::Win32::UI::HiDpi::GetDpiForWindow;
    let hwnd = window.hwnd().ok()?;
    let area = monitor_work_area(hwnd)?;
    // SAFETY: 只读这个窗口的 DPI
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    Some((area, if dpi == 0 { 1.0 } else { dpi as f64 / 96.0 }))
}

#[cfg(not(windows))]
pub(crate) fn window_area<R: tauri::Runtime>(
    window: &tauri::WebviewWindow<R>,
) -> Option<(Rect, f64)> {
    let monitor = window.current_monitor().ok()??;
    let wa = monitor.work_area();
    Some((
        Rect::at(
            wa.position.x,
            wa.position.y,
            (wa.size.width as i32, wa.size.height as i32),
        ),
        monitor.scale_factor(),
    ))
}

#[cfg(windows)]
fn monitor_work_area(hwnd: windows::Win32::Foundation::HWND) -> Option<Rect> {
    use windows::Win32::Graphics::Gdi::{MonitorFromWindow, MONITOR_DEFAULTTOPRIMARY};
    // SAFETY: 只读这个窗口所在的显示器（窗口没了时 MONITOR_DEFAULTTOPRIMARY 给主屏）
    monitor_info_work_area(unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTOPRIMARY) })
}

#[cfg(windows)]
fn monitor_info_work_area(monitor: windows::Win32::Graphics::Gdi::HMONITOR) -> Option<Rect> {
    use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITORINFO};
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: 只读这块显示器的信息
    let ok = unsafe { GetMonitorInfoW(monitor, &mut info) };
    ok.as_bool().then(|| {
        let r = info.rcWork;
        Rect {
            left: r.left,
            top: r.top,
            right: r.right,
            bottom: r.bottom,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-11-25 真机那块屏：3840 × 2160、缩放 150%、任务栏 72 高（`message_popup_real.py` 打出来的窗口位置）。
    const AREA: Rect = Rect {
        left: 0,
        top: 0,
        right: 3840,
        bottom: 2088,
    };
    const S: f64 = 1.5;

    fn px(logical: f64) -> i32 {
        (logical * S).round() as i32
    }

    /// 输入条选“右下角”预设时的位置（`overlay_window.rs` 的 `layout_overlay_window`：离边 24）。
    fn input_bar_bottom_right(size: (f64, f64)) -> Rect {
        let (w, h) = (px(size.0), px(size.1));
        Rect::at(
            AREA.right - w - px(24.0),
            AREA.bottom - h - px(24.0),
            (w, h),
        )
    }
    const CAPSULE: (f64, f64) = (520.0, 64.0);
    const CARD: (f64, f64) = (480.0, 120.0);

    /// 悬浮窗（`floating/mod.rs`：离边 16，再加玩家设的上下偏移）；气泡一行 360 × 96、小窗 400 × 520。
    fn floating(corner: Corner, size: (f64, f64), offset: f64, inner: &[Rect]) -> Rect {
        stack(
            AREA,
            corner,
            (px(size.0), px(size.1)),
            (px(16.0), px(16.0 + offset)),
            px(GAP),
            inner,
        )
    }
    const BUBBLE: (f64, f64) = (360.0, 96.0);
    const MINI: (f64, f64) = (400.0, 520.0);

    fn apart(a: &Rect, b: &Rect) {
        assert!(!a.overlaps(b), "{a:?} 和 {b:?} 叠上了");
    }

    /// 共享资源评估第二节第 10 条：右下角的小窗和输入条用同样的边距会叠在一起（2026-11-25 真机复现：
    /// `message_popup_real.py` M3）。现在右下角排成一列：输入条贴着角，悬浮窗（气泡或小窗）在它上面；输入条没显示就贴着角。
    #[test]
    fn the_bottom_right_corner_is_one_column() {
        for bar in [CAPSULE, CARD] {
            let bar = input_bar_bottom_right(bar);
            for size in [BUBBLE, MINI] {
                let f = floating(Corner::BottomRight, size, 0.0, &[bar]);
                apart(&f, &bar);
                assert_eq!(
                    f.bottom,
                    bar.top - px(GAP),
                    "悬浮窗在输入条上面：{f:?} {bar:?}"
                );
                assert_eq!(f.right, AREA.right - px(16.0), "左右还贴着角");
            }
        }
        let alone = floating(Corner::BottomRight, BUBBLE, 0.0, &[]);
        assert_eq!(
            (alone.right, alone.bottom),
            (AREA.right - px(16.0), AREA.bottom - px(16.0))
        );
    }

    /// 不在同一列的不碍事：输入条用默认的“跟随样式”（胶囊在底部正中）、拖到屏幕中间；悬浮窗在左下角。
    #[test]
    fn windows_elsewhere_do_not_push() {
        let alone = floating(Corner::BottomRight, BUBBLE, 0.0, &[]);
        let (w, h) = (px(CAPSULE.0), px(CAPSULE.1));
        let capsule_auto = Rect::at((AREA.right - w) / 2, AREA.bottom - h - px(48.0), (w, h));
        let dragged = Rect::at(1600, 900, (w, h));
        assert_eq!(
            floating(Corner::BottomRight, BUBBLE, 0.0, &[capsule_auto, dragged]),
            alone
        );
        let bar = input_bar_bottom_right(CAPSULE);
        let left = floating(Corner::BottomLeft, BUBBLE, 0.0, &[]);
        assert_eq!(floating(Corner::BottomLeft, BUBBLE, 0.0, &[bar]), left);
        // 拖到角上的输入条（custom）照样让开它，它自己不动（它在最里面，没有东西让它挪）
        let dragged_corner = Rect::at(AREA.right - w - 10, AREA.bottom - h - 10, (w, h));
        apart(
            &floating(Corner::BottomRight, BUBBLE, 0.0, &[dragged_corner]),
            &dragged_corner,
        );
    }

    /// 上面的角往下排：输入条选“右上角”预设时，悬浮窗在它下面。玩家设的偏移照旧算。
    #[test]
    fn top_corners_stack_downward_and_offsets_still_count() {
        let (w, h) = (px(CARD.0), px(CARD.1));
        let bar_top_right = Rect::at(AREA.right - w - px(24.0), AREA.top + px(24.0), (w, h));
        let f = floating(Corner::TopRight, BUBBLE, 0.0, &[bar_top_right]);
        apart(&f, &bar_top_right);
        assert_eq!(f.top, bar_top_right.bottom + px(GAP));
        // 偏移够大、本来就不叠：不挪
        let far = floating(Corner::TopRight, BUBBLE, 200.0, &[bar_top_right]);
        assert_eq!(far.top, AREA.top + px(216.0));
    }

    /// 后出来的输入条叠上了贴着角的悬浮窗：悬浮窗挪出去（`settle` 用的就是 `clear`）；不叠的不动。
    /// 拖到别处的（`set_free`）`settle` 不挪。
    #[test]
    fn a_window_shown_later_pushes_the_outer_ones_out() {
        let at_corner = floating(Corner::BottomRight, BUBBLE, 0.0, &[]);
        let bar = input_bar_bottom_right(CAPSULE);
        let moved = clear(at_corner, true, AREA, px(GAP), &[bar]);
        apart(&moved, &bar);
        assert_eq!(moved.bottom, bar.top - px(GAP));
        assert_eq!((moved.left, moved.right), (at_corner.left, at_corner.right));
        // 已经在外面的不动
        assert_eq!(clear(moved, true, AREA, px(GAP), &[bar]), moved);
        assert_eq!(clear(moved, true, AREA, px(GAP), &[]), moved);
        // 拖到别处的不排
        assert!(!is_free("floating"));
        set_free("floating", true);
        assert!(is_free("floating"));
        set_free("floating", false);
        assert!(!is_free("floating"));
    }

    /// 屏幕放不下时停在工作区里（叠着也不出屏幕）。
    #[test]
    fn never_leaves_the_work_area() {
        let small = Rect {
            left: 0,
            top: 0,
            right: 800,
            bottom: 300,
        };
        let tall = Rect::at(400, 40, (380, 250));
        let r = stack(
            small,
            Corner::BottomRight,
            (300, 120),
            (12, 12),
            12,
            &[tall],
        );
        assert_eq!(r.top, small.top);
        assert!(r.bottom <= small.bottom);
    }
}
