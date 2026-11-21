//! 工具服务 M1 的 Windows 窗口观察与 GDI 前台可见客户区捕获。
//!
//! - [`WindowsWindowObserver`]：EnumWindows 枚举顶层窗口，读取进程身份、客户区物理坐标与 DPI。
//! - [`GdiForegroundCapturer`]：仅当目标是前台、未最小化、未被 DWM 隐藏且客户区完整
//!   位于单个显示器内时，用 GDI 从屏幕 DC 复制该客户区矩形的已合成像素。
//!
//! 边界（同时写入 capabilities）：这是“屏幕上看到的前台客户区像素”，不是窗口内容隔离
//! 捕获；覆盖在该矩形内的置顶窗口/弹幕会一并出现，最小化/遮挡/后台窗口不可捕获，
//! 独占全屏或受保护内容可能得到黑图。不移动、显示或聚焦任何窗口。
//!
//! 所有坐标在线程级 Per-Monitor-V2 DPI 上下文中读取，得到物理像素；上下文在调用结束时恢复。
//!
//! Kotone 自己用的 [`GdiForegroundCapturer::capture_foreground_view`]（开口时的截图、读聊天、
//! `game.look`、自动化）放宽两条：跨显示器时只截客户区在主要那块屏上的部分，超过像素上限时
//! 等比缩小；两种情况都在 `warnings` 里说明（`PARTIAL_VIEW` / `DOWNSCALED`）。外部工具服务仍用
//! 上面严格的 `capture_foreground_client`。

use kotone_core::tools::{
    capture_view, downscale_area, fit_size, CaptureBackendInfo, CaptureWarning, CapturedImage,
    PhysicalRect, ViewCapture, WindowCapturer, WindowIdentity, WindowInfo, WindowObserver,
    WindowToolError,
};

/// GDI 后端的方法标识（进入 capabilities 与每个捕获产物的来源记录）。
pub const GDI_CAPTURE_METHOD: &str = "gdi-screen-foreground-client";

/// 必须告知调用方的 GDI 捕获边界。
pub fn gdi_capture_limitations() -> Vec<String> {
    [
        "Copies the pixels composed on screen inside the target window's client rectangle while that window is the foreground window (BitBlt SRCCOPY|CAPTUREBLT from the screen DC). Anything drawn above that rectangle is part of the image: topmost windows, notifications and layered overlays such as Kotone danmaku (verified natively with a test WS_EX_LAYERED window).",
        "CAPTUREBLT may make the mouse cursor flicker briefly on some systems; the cursor itself is not drawn into the image.",
        "Hidden, occluded, minimized, cloaked or background windows are not captured and occluded content is not restored; a non-foreground target is rejected rather than focused.",
        "Exclusive-fullscreen, hardware-overlay or protected GPU content may come back black or stale; check the UNIFORM_PIXELS and EXCLUSIVE_FULLSCREEN_SUSPECTED warnings.",
        "The client area must lie entirely within one monitor; windows that are partially off-screen or span monitors are rejected instead of falling back to a desktop capture.",
        "Coordinates are physical pixels read under a verified per-monitor DPI-aware thread context; if that context cannot be entered the call fails instead of reporting unverified coordinates. The service never moves, shows or focuses windows.",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// 前台客户区像素全部相同（全黑/全白等）时给出的警告码。
pub const WARNING_UNIFORM_PIXELS: &str = "UNIFORM_PIXELS";
/// 系统报告 Direct3D 独占全屏时给出的警告码。
pub const WARNING_EXCLUSIVE_FULLSCREEN: &str = "EXCLUSIVE_FULLSCREEN_SUSPECTED";
/// 窗口横跨多块屏幕，只截了它在主要那块屏上的部分（[`GdiForegroundCapturer::capture_foreground_view`]）。
pub const WARNING_PARTIAL_VIEW: &str = "PARTIAL_VIEW";
/// 截到的区域超过像素上限，等比缩小了（[`GdiForegroundCapturer::capture_foreground_view`]）。
pub const WARNING_DOWNSCALED: &str = "DOWNSCALED";

/// Kotone 自己截前台窗口前的检查与取景（纯逻辑，便于测试）：窗口要在前台、没最小化、可见；
/// 取客户区和它所在显示器的交集（[`capture_view`]）。和工具服务的严格截图不同，跨屏不拒绝、
/// 超过像素上限也不拒绝（截完再等比缩小，见 [`finish_view`]）。
pub fn view_rect(info: &WindowInfo) -> Result<PhysicalRect, WindowToolError> {
    if info.minimized {
        return Err(WindowToolError::Minimized);
    }
    if !info.visible || info.cloaked {
        return Err(WindowToolError::NotVisible);
    }
    if !info.foreground {
        return Err(WindowToolError::NotForeground);
    }
    let client = info
        .client_rect
        .ok_or_else(|| WindowToolError::InvalidGeometry("无法读取客户区位置".into()))?;
    if client.width == 0 || client.height == 0 {
        return Err(WindowToolError::InvalidGeometry("客户区为空".into()));
    }
    let monitor = info
        .monitor_rect
        .ok_or_else(|| WindowToolError::InvalidGeometry("窗口不在任何显示器上".into()))?;
    capture_view(&client, &monitor).ok_or_else(|| {
        WindowToolError::InvalidGeometry(
            "客户区不在它所在的显示器上（可能整个移到了屏幕外）".into(),
        )
    })
}

/// 截完之后：取景区域比客户区小就加“只截了一部分”的提示；超过像素上限就等比缩小并加提示。
/// 返回（像素、宽、高）；提示追加到 `warnings`。
pub fn finish_view(
    rgba: Vec<u8>,
    view: PhysicalRect,
    client: PhysicalRect,
    max_pixels: u64,
    warnings: &mut Vec<CaptureWarning>,
) -> (Vec<u8>, u32, u32) {
    if view != client {
        warnings.push(CaptureWarning::new(
            WARNING_PARTIAL_VIEW,
            format!(
                "窗口横跨多块屏幕：只截了它在主要那块屏上的部分（屏幕物理像素 {},{} 起 {}×{}；整个客户区 {}×{}），别的屏上的内容不在图里。",
                view.x, view.y, view.width, view.height, client.width, client.height
            ),
        ));
    }
    let (w, h) = fit_size(view.width, view.height, max_pixels);
    if (w, h) == (view.width, view.height) {
        return (rgba, w, h);
    }
    let out = downscale_area(&rgba, view.width, view.height, w, h);
    warnings.push(CaptureWarning::new(
        WARNING_DOWNSCALED,
        format!(
            "截到的区域 {}×{} 超过 {} 像素，等比缩小到 {}×{}（{:.0}%）；很小的字可能更难认，需要时放大局部再看。",
            view.width,
            view.height,
            max_pixels,
            w,
            h,
            f64::from(w) / f64::from(view.width) * 100.0
        ),
    ));
    (out, w, h)
}

/// RGBA 像素是否全部相同。
pub fn is_uniform_rgba(rgba: &[u8]) -> bool {
    let mut chunks = rgba.chunks_exact(4);
    let Some(first) = chunks.next() else {
        return true;
    };
    chunks.all(|px| px == first)
}

/// DPI 作用域进入结果的判定（纯逻辑，便于测试）：只有确认当前线程处于 per-monitor
/// DPI 感知时才允许以物理像素报告坐标；否则明确失败，绝不在未知上下文里贴“物理像素”标签。
///
/// - `switched`：SetThreadDpiAwarenessContext 是否成功（返回了非空的旧上下文）。
/// - `effective_per_monitor`：切换后（或切换失败时）线程实际上下文是否为 per-monitor 感知。
///
/// 返回 Ok(是否需要在离开时恢复旧上下文)。
pub fn dpi_scope_decision(
    switched: bool,
    effective_per_monitor: bool,
) -> Result<bool, WindowToolError> {
    if effective_per_monitor {
        Ok(switched)
    } else {
        Err(WindowToolError::Unsupported(
            "无法进入 per-monitor DPI 感知线程上下文，拒绝以未经验证的坐标报告物理像素".into(),
        ))
    }
}

/// Windows 窗口观察实现。
#[derive(Debug, Default, Clone, Copy)]
pub struct WindowsWindowObserver;

/// GDI 前台可见客户区捕获实现。
#[derive(Debug, Default, Clone, Copy)]
pub struct GdiForegroundCapturer;

#[cfg(windows)]
pub use imp::{
    current_process_gdi_objects, current_thread_per_monitor_aware, process_started_at,
    with_verified_dpi_scope,
};

#[cfg(windows)]
mod imp {
    use super::*;
    use std::ffi::c_void;

    use windows::core::BOOL;
    use windows::Win32::Foundation::{CloseHandle, FILETIME, HWND, LPARAM, POINT, RECT};
    use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
    use windows::Win32::Graphics::Gdi::{
        BitBlt, ClientToScreen, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject,
        GdiFlush, GetDC, GetMonitorInfoW, MonitorFromWindow, ReleaseDC, SelectObject, BITMAPINFO,
        BITMAPINFOHEADER, BI_RGB, CAPTUREBLT, DIB_RGB_COLORS, HBITMAP, HDC, HGDIOBJ, MONITORINFO,
        MONITOR_DEFAULTTONULL, ROP_CODE, SRCCOPY,
    };
    use windows::Win32::System::Threading::{
        GetCurrentProcess, GetExitCodeProcess, GetGuiResources, GetProcessTimes, OpenProcess,
        QueryFullProcessImageNameW, GR_GDIOBJECTS, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows::Win32::UI::HiDpi::{
        GetAwarenessFromDpiAwarenessContext, GetDpiForWindow, GetThreadDpiAwarenessContext,
        SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT,
        DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, DPI_AWARENESS_PER_MONITOR_AWARE,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetAncestor, GetClassNameW, GetClientRect, GetForegroundWindow,
        GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindow,
        IsWindowVisible, GA_ROOT,
    };

    const STILL_ACTIVE: u32 = 259;

    /// 线程级 DPI 上下文作用域：进入时切到 Per-Monitor-V2 并核实实际上下文为
    /// per-monitor 感知，离开时恢复原值。切换失败且当前上下文也不是 per-monitor 感知时
    /// 返回错误，调用方不得继续读取坐标。
    pub(crate) struct DpiScope(Option<DPI_AWARENESS_CONTEXT>);

    fn thread_is_per_monitor_aware() -> bool {
        unsafe {
            GetAwarenessFromDpiAwarenessContext(GetThreadDpiAwarenessContext())
                == DPI_AWARENESS_PER_MONITOR_AWARE
        }
    }

    impl DpiScope {
        pub(crate) fn enter() -> Result<Self, WindowToolError> {
            let previous =
                unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
            let switched = !previous.0.is_null();
            // 判定失败时 scope 被丢弃，已切换的旧上下文随之恢复
            let scope = Self(switched.then_some(previous));
            dpi_scope_decision(switched, thread_is_per_monitor_aware())?;
            Ok(scope)
        }
    }

    impl Drop for DpiScope {
        fn drop(&mut self) {
            if let Some(previous) = self.0.take() {
                unsafe {
                    SetThreadDpiAwarenessContext(previous);
                }
            }
        }
    }

    /// 测试用：在已验证的 DPI 作用域内执行闭包。
    pub fn with_verified_dpi_scope<T>(f: impl FnOnce() -> T) -> Result<T, WindowToolError> {
        let _scope = DpiScope::enter()?;
        Ok(f())
    }

    /// 测试用：当前线程是否处于 per-monitor DPI 感知。
    pub fn current_thread_per_monitor_aware() -> bool {
        thread_is_per_monitor_aware()
    }

    /// 当前进程持有的 GDI 对象数量（测试用：核对捕获不泄漏 DC/位图）。
    pub fn current_process_gdi_objects() -> u32 {
        unsafe { GetGuiResources(GetCurrentProcess(), GR_GDIOBJECTS) }
    }

    /// 仍在运行的进程的创建时间（FILETIME 100ns）；进程已退出或无权查询时为 None。
    pub fn process_started_at(pid: u32) -> Option<u64> {
        if pid == 0 {
            return None;
        }
        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
            let result = (|| {
                let mut exit_code = 0u32;
                GetExitCodeProcess(handle, &mut exit_code).ok()?;
                if exit_code != STILL_ACTIVE {
                    return None;
                }
                let mut created = FILETIME::default();
                let mut exited = FILETIME::default();
                let mut kernel = FILETIME::default();
                let mut user = FILETIME::default();
                GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user).ok()?;
                Some((u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime))
            })();
            let _ = CloseHandle(handle);
            result
        }
    }

    fn process_image_name(pid: u32) -> Option<String> {
        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
            let mut buf = vec![0u16; 1024];
            let mut len = buf.len() as u32;
            let ok = QueryFullProcessImageNameW(
                handle,
                PROCESS_NAME_WIN32,
                windows::core::PWSTR(buf.as_mut_ptr()),
                &mut len,
            )
            .is_ok();
            let _ = CloseHandle(handle);
            if !ok {
                return None;
            }
            let full = String::from_utf16_lossy(&buf[..len as usize]);
            // 只返回文件名，不向调用方暴露用户目录结构
            std::path::Path::new(&full)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
        }
    }

    fn hwnd_of(identity: &WindowIdentity) -> HWND {
        HWND(identity.handle as usize as *mut c_void)
    }

    fn foreground_root() -> Option<HWND> {
        unsafe {
            let fg = GetForegroundWindow();
            if fg.0.is_null() {
                return None;
            }
            let root = GetAncestor(fg, GA_ROOT);
            Some(if root.0.is_null() { fg } else { root })
        }
    }

    fn window_text(hwnd: HWND) -> String {
        unsafe {
            let len = GetWindowTextLengthW(hwnd);
            if len <= 0 {
                return String::new();
            }
            let mut buf = vec![0u16; len as usize + 1];
            let copied = GetWindowTextW(hwnd, &mut buf);
            String::from_utf16_lossy(&buf[..copied.max(0) as usize])
        }
    }

    fn class_name(hwnd: HWND) -> String {
        unsafe {
            let mut buf = [0u16; 256];
            let copied = GetClassNameW(hwnd, &mut buf);
            String::from_utf16_lossy(&buf[..copied.max(0) as usize])
        }
    }

    fn is_cloaked(hwnd: HWND) -> bool {
        let mut value: u32 = 0;
        unsafe {
            DwmGetWindowAttribute(
                hwnd,
                DWMWA_CLOAKED,
                &mut value as *mut u32 as *mut c_void,
                std::mem::size_of::<u32>() as u32,
            )
            .is_ok()
                && value != 0
        }
    }

    fn rect_from(rc: RECT) -> Option<PhysicalRect> {
        let width = i64::from(rc.right) - i64::from(rc.left);
        let height = i64::from(rc.bottom) - i64::from(rc.top);
        if width < 0 || height < 0 {
            return None;
        }
        Some(PhysicalRect {
            x: rc.left,
            y: rc.top,
            width: u32::try_from(width).ok()?,
            height: u32::try_from(height).ok()?,
        })
    }

    fn client_rect_on_screen(hwnd: HWND) -> Option<PhysicalRect> {
        unsafe {
            let mut rc = RECT::default();
            GetClientRect(hwnd, &mut rc).ok()?;
            let mut origin = POINT { x: 0, y: 0 };
            if !ClientToScreen(hwnd, &mut origin).as_bool() {
                return None;
            }
            let local = rect_from(rc)?;
            Some(PhysicalRect {
                x: origin.x,
                y: origin.y,
                ..local
            })
        }
    }

    fn monitor_rect(hwnd: HWND) -> Option<PhysicalRect> {
        unsafe {
            let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONULL);
            if monitor.0.is_null() {
                return None;
            }
            let mut info = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            if !GetMonitorInfoW(monitor, &mut info).as_bool() {
                return None;
            }
            rect_from(info.rcMonitor)
        }
    }

    /// 单次调用内按 PID 缓存进程身份与名称（枚举时同一进程常有多个顶层窗口）。
    type ProcessCache = std::collections::HashMap<u32, (Option<u64>, Option<String>)>;

    fn process_facts(pid: u32, cache: &mut ProcessCache) -> (Option<u64>, Option<String>) {
        cache
            .entry(pid)
            .or_insert_with(|| {
                (
                    process_started_at(pid),
                    process_image_name(pid).or_else(|| crate::inject::process_name_from_pid(pid)),
                )
            })
            .clone()
    }

    /// 在已进入 DPI 作用域的线程上读取窗口元数据（不校验身份）。
    fn describe(
        hwnd: HWND,
        fg_root: Option<HWND>,
        cache: &mut ProcessCache,
    ) -> Result<WindowInfo, WindowToolError> {
        unsafe {
            if !IsWindow(Some(hwnd)).as_bool() {
                return Err(WindowToolError::Gone);
            }
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            if pid == 0 {
                return Err(WindowToolError::Gone);
            }
            let minimized = IsIconic(hwnd).as_bool();
            let (process_started_at, process_name) = process_facts(pid, cache);
            let dpi = match GetDpiForWindow(hwnd) {
                0 => None,
                value => Some(value),
            };
            Ok(WindowInfo {
                identity: WindowIdentity {
                    handle: hwnd.0 as usize as u64,
                    pid,
                    process_started_at,
                },
                title: window_text(hwnd),
                class_name: class_name(hwnd),
                process_name,
                visible: IsWindowVisible(hwnd).as_bool(),
                minimized,
                cloaked: is_cloaked(hwnd),
                foreground: fg_root == Some(hwnd),
                client_rect: if minimized {
                    None
                } else {
                    client_rect_on_screen(hwnd)
                },
                monitor_rect: monitor_rect(hwnd),
                dpi,
            })
        }
    }

    fn inspect_in_scope(identity: &WindowIdentity) -> Result<WindowInfo, WindowToolError> {
        let info = describe(
            hwnd_of(identity),
            foreground_root(),
            &mut ProcessCache::new(),
        )?;
        if info.identity.pid != identity.pid {
            return Err(WindowToolError::IdentityChanged);
        }
        if identity.process_started_at.is_some()
            && info.identity.process_started_at != identity.process_started_at
        {
            return Err(WindowToolError::IdentityChanged);
        }
        Ok(info)
    }

    unsafe extern "system" fn collect_hwnd(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let list = &mut *(lparam.0 as *mut Vec<HWND>);
        list.push(hwnd);
        BOOL(1)
    }

    impl WindowObserver for WindowsWindowObserver {
        fn list_windows(&self) -> Result<Vec<WindowInfo>, WindowToolError> {
            let _dpi = DpiScope::enter()?;
            let mut handles: Vec<HWND> = Vec::new();
            unsafe {
                EnumWindows(
                    Some(collect_hwnd),
                    LPARAM(&mut handles as *mut Vec<HWND> as isize),
                )
                .map_err(|e| WindowToolError::Platform(format!("EnumWindows: {e}")))?;
            }
            let fg = foreground_root();
            let mut cache = ProcessCache::new();
            Ok(handles
                .into_iter()
                .filter_map(|hwnd| describe(hwnd, fg, &mut cache).ok())
                .filter(|info| info.visible && !info.cloaked)
                .collect())
        }

        fn foreground_window(&self) -> Result<Option<WindowInfo>, WindowToolError> {
            let _dpi = DpiScope::enter()?;
            let Some(fg) = foreground_root() else {
                return Ok(None);
            };
            match describe(fg, Some(fg), &mut ProcessCache::new()) {
                Ok(info) => Ok(Some(info)),
                Err(WindowToolError::Gone) => Ok(None),
                Err(other) => Err(other),
            }
        }

        fn inspect(&self, identity: &WindowIdentity) -> Result<WindowInfo, WindowToolError> {
            let _dpi = DpiScope::enter()?;
            inspect_in_scope(identity)
        }
    }

    /// 校验捕获前置条件，返回待复制的屏幕物理矩形。
    fn capturable_rect(
        info: &WindowInfo,
        max_pixels: u64,
    ) -> Result<PhysicalRect, WindowToolError> {
        if info.minimized {
            return Err(WindowToolError::Minimized);
        }
        if !info.visible || info.cloaked {
            return Err(WindowToolError::NotVisible);
        }
        if !info.foreground {
            return Err(WindowToolError::NotForeground);
        }
        let rect = info
            .client_rect
            .ok_or_else(|| WindowToolError::InvalidGeometry("无法读取客户区位置".into()))?;
        if rect.width == 0 || rect.height == 0 {
            return Err(WindowToolError::InvalidGeometry("客户区为空".into()));
        }
        if rect.pixel_count() > max_pixels {
            return Err(WindowToolError::TooLarge {
                pixels: rect.pixel_count(),
                max_pixels,
            });
        }
        let monitor = info
            .monitor_rect
            .ok_or_else(|| WindowToolError::InvalidGeometry("窗口不在任何显示器上".into()))?;
        if !rect.is_within(&monitor) {
            return Err(WindowToolError::InvalidGeometry(
                "客户区超出所在显示器或跨越多个显示器，拒绝退化为桌面截图".into(),
            ));
        }
        Ok(rect)
    }

    struct ScreenDc(HDC);
    impl Drop for ScreenDc {
        fn drop(&mut self) {
            unsafe {
                ReleaseDC(None, self.0);
            }
        }
    }

    struct MemoryDc(HDC);
    impl Drop for MemoryDc {
        fn drop(&mut self) {
            unsafe {
                let _ = DeleteDC(self.0);
            }
        }
    }

    struct DibSection(HBITMAP);
    impl Drop for DibSection {
        fn drop(&mut self) {
            unsafe {
                let _ = DeleteObject(self.0.into());
            }
        }
    }

    struct Selection {
        dc: HDC,
        previous: HGDIOBJ,
    }
    impl Drop for Selection {
        fn drop(&mut self) {
            unsafe {
                SelectObject(self.dc, self.previous);
            }
        }
    }

    /// 从屏幕 DC 复制物理矩形为 RGBA8。每个 GDI 对象都由 RAII 守卫释放（成功与失败路径一致）。
    fn copy_screen_rect(rect: PhysicalRect) -> Result<Vec<u8>, WindowToolError> {
        let width = i32::try_from(rect.width)
            .map_err(|_| WindowToolError::InvalidGeometry("宽度溢出".into()))?;
        let height = i32::try_from(rect.height)
            .map_err(|_| WindowToolError::InvalidGeometry("高度溢出".into()))?;
        let byte_len = usize::try_from(rect.pixel_count())
            .ok()
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or_else(|| WindowToolError::InvalidGeometry("像素缓冲区溢出".into()))?;
        unsafe {
            let screen = GetDC(None);
            if screen.0.is_null() {
                return Err(WindowToolError::Platform("GetDC(屏幕) 失败".into()));
            }
            let screen = ScreenDc(screen);
            let memory = CreateCompatibleDC(Some(screen.0));
            if memory.0.is_null() {
                return Err(WindowToolError::Platform("CreateCompatibleDC 失败".into()));
            }
            let memory = MemoryDc(memory);
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: width,
                    // 负高度 = 自上而下的 DIB，行序与产物一致
                    biHeight: -height,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits: *mut c_void = std::ptr::null_mut();
            let bitmap =
                CreateDIBSection(Some(memory.0), &info, DIB_RGB_COLORS, &mut bits, None, 0)
                    .map_err(|e| WindowToolError::Platform(format!("CreateDIBSection: {e}")))?;
            let bitmap = DibSection(bitmap);
            if bits.is_null() {
                return Err(WindowToolError::Platform("DIB 像素缓冲区为空".into()));
            }
            let previous = SelectObject(memory.0, bitmap.0.into());
            if previous.0.is_null() {
                return Err(WindowToolError::Platform("SelectObject 失败".into()));
            }
            let _selection = Selection {
                dc: memory.0,
                previous,
            };
            BitBlt(
                memory.0,
                0,
                0,
                width,
                height,
                Some(screen.0),
                rect.x,
                rect.y,
                // CAPTUREBLT：把分层（WS_EX_LAYERED）窗口一并合入，使结果与能力声明一致
                ROP_CODE(SRCCOPY.0 | CAPTUREBLT.0),
            )
            .map_err(|e| WindowToolError::Platform(format!("BitBlt: {e}")))?;
            let _ = GdiFlush();
            let bgra = std::slice::from_raw_parts(bits as *const u8, byte_len);
            let mut rgba = Vec::with_capacity(byte_len);
            for px in bgra.chunks_exact(4) {
                rgba.extend_from_slice(&[px[2], px[1], px[0], 255]);
            }
            Ok(rgba)
        }
    }

    impl WindowCapturer for GdiForegroundCapturer {
        fn backend_info(&self) -> CaptureBackendInfo {
            CaptureBackendInfo {
                method: GDI_CAPTURE_METHOD.into(),
                available: true,
                unavailable_reason: None,
                limitations: gdi_capture_limitations(),
            }
        }

        fn capture_foreground_client(
            &self,
            identity: &WindowIdentity,
            max_pixels: u64,
        ) -> Result<CapturedImage, WindowToolError> {
            let _dpi = DpiScope::enter()?;
            let before = inspect_in_scope(identity)?;
            let rect = capturable_rect(&before, max_pixels)?;
            let rgba = copy_screen_rect(rect)?;
            let after = recheck_after_copy(identity, rect)?;
            let warnings = content_warnings(&rgba);
            Ok(CapturedImage {
                width: rect.width,
                height: rect.height,
                rgba,
                client_rect: rect,
                dpi: after.dpi,
                warnings,
            })
        }
    }

    /// 复制后再次核对：期间失去前台、移动 / 缩放或关闭的帧不交付。
    fn recheck_after_copy(
        identity: &WindowIdentity,
        client: PhysicalRect,
    ) -> Result<WindowInfo, WindowToolError> {
        let after = inspect_in_scope(identity).map_err(|error| match error {
            WindowToolError::Gone | WindowToolError::IdentityChanged => {
                WindowToolError::ChangedDuringCapture(error.to_string())
            }
            other => other,
        })?;
        if after.minimized {
            return Err(WindowToolError::ChangedDuringCapture("目标已最小化".into()));
        }
        if !after.foreground {
            return Err(WindowToolError::ChangedDuringCapture("目标失去前台".into()));
        }
        if after.client_rect != Some(client) {
            return Err(WindowToolError::ChangedDuringCapture(
                "客户区已移动或改变大小".into(),
            ));
        }
        Ok(after)
    }

    /// 画面本身的提示：纯色（可能是独占全屏或受保护内容）、系统报告独占全屏。
    fn content_warnings(rgba: &[u8]) -> Vec<CaptureWarning> {
        let mut warnings = Vec::new();
        if is_uniform_rgba(rgba) {
            warnings.push(CaptureWarning::new(
                WARNING_UNIFORM_PIXELS,
                "All captured pixels are identical; the window may be blank, exclusive-fullscreen or protected. A uniform image is not evidence of readable content.",
            ));
        }
        if crate::fullscreen::is_exclusive_fullscreen_active() == Some(true) {
            warnings.push(CaptureWarning::new(
                WARNING_EXCLUSIVE_FULLSCREEN,
                "Windows reports a Direct3D exclusive-fullscreen application; GDI screen pixels may be black or stale.",
            ));
        }
        warnings
    }

    impl GdiForegroundCapturer {
        /// Kotone 自己用的前台截图（开口时的截图、读聊天、`game.look`、自动化）：截客户区和它所在
        /// 显示器的交集（跨屏不拒绝，只截主要那块屏上的部分），超过 `max_pixels` 就等比缩小
        /// （不拒绝）。仍然只截这个窗口、要求它在前台，复制后同样再核对一次。
        /// 外部工具服务继续用严格的 [`WindowCapturer::capture_foreground_client`]。
        pub fn capture_foreground_view(
            &self,
            identity: &WindowIdentity,
            max_pixels: u64,
        ) -> Result<ViewCapture, WindowToolError> {
            let _dpi = DpiScope::enter()?;
            let before = inspect_in_scope(identity)?;
            let view = view_rect(&before)?;
            let client = before
                .client_rect
                .ok_or_else(|| WindowToolError::InvalidGeometry("无法读取客户区位置".into()))?;
            let rgba = copy_screen_rect(view)?;
            let after = recheck_after_copy(identity, client)?;
            let mut warnings = content_warnings(&rgba);
            let (rgba, width, height) = finish_view(rgba, view, client, max_pixels, &mut warnings);
            Ok(ViewCapture {
                image: CapturedImage {
                    width,
                    height,
                    rgba,
                    client_rect: client,
                    dpi: after.dpi,
                    warnings,
                },
                view,
            })
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::*;

    fn unsupported() -> WindowToolError {
        WindowToolError::Unsupported("窗口观察与捕获仅在 Windows 上实现".into())
    }

    impl WindowObserver for WindowsWindowObserver {
        fn list_windows(&self) -> Result<Vec<WindowInfo>, WindowToolError> {
            Err(unsupported())
        }
        fn foreground_window(&self) -> Result<Option<WindowInfo>, WindowToolError> {
            Err(unsupported())
        }
        fn inspect(&self, _identity: &WindowIdentity) -> Result<WindowInfo, WindowToolError> {
            Err(unsupported())
        }
    }

    impl WindowCapturer for GdiForegroundCapturer {
        fn backend_info(&self) -> CaptureBackendInfo {
            CaptureBackendInfo {
                method: GDI_CAPTURE_METHOD.into(),
                available: false,
                unavailable_reason: Some("GDI capture requires Windows".into()),
                limitations: gdi_capture_limitations(),
            }
        }
        fn capture_foreground_client(
            &self,
            _identity: &WindowIdentity,
            _max_pixels: u64,
        ) -> Result<CapturedImage, WindowToolError> {
            Err(unsupported())
        }
    }

    impl GdiForegroundCapturer {
        pub fn capture_foreground_view(
            &self,
            _identity: &WindowIdentity,
            _max_pixels: u64,
        ) -> Result<ViewCapture, WindowToolError> {
            Err(unsupported())
        }
    }

    pub fn process_started_at(_pid: u32) -> Option<u64> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dpi_scope_requires_verified_per_monitor_context() {
        assert_eq!(dpi_scope_decision(true, true), Ok(true));
        // 切换失败但线程本来就是 per-monitor 感知：可以继续，且无需恢复
        assert_eq!(dpi_scope_decision(false, true), Ok(false));
        // 切换失败且上下文非 per-monitor：必须失败，而不是贴上“物理像素”标签
        assert!(matches!(
            dpi_scope_decision(false, false),
            Err(WindowToolError::Unsupported(_))
        ));
        assert!(matches!(
            dpi_scope_decision(true, false),
            Err(WindowToolError::Unsupported(_))
        ));
    }

    #[cfg(windows)]
    #[test]
    fn verified_scope_is_per_monitor_and_restores_previous_context() {
        let before = current_thread_per_monitor_aware();
        let inside = with_verified_dpi_scope(current_thread_per_monitor_aware).unwrap();
        assert!(inside, "作用域内必须是 per-monitor 感知");
        assert_eq!(
            current_thread_per_monitor_aware(),
            before,
            "离开作用域恢复原上下文"
        );
    }

    #[test]
    fn uniform_detection() {
        assert!(is_uniform_rgba(&[]));
        assert!(is_uniform_rgba(&[0, 0, 0, 255, 0, 0, 0, 255]));
        assert!(!is_uniform_rgba(&[0, 0, 0, 255, 0, 0, 1, 255]));
    }

    fn rect(x: i32, y: i32, width: u32, height: u32) -> PhysicalRect {
        PhysicalRect {
            x,
            y,
            width,
            height,
        }
    }

    fn window(client: PhysicalRect, monitor: PhysicalRect) -> WindowInfo {
        WindowInfo {
            identity: WindowIdentity {
                handle: 1,
                pid: 2,
                process_started_at: None,
            },
            title: "游戏".into(),
            class_name: "Game".into(),
            process_name: Some("game.exe".into()),
            visible: true,
            minimized: false,
            cloaked: false,
            foreground: true,
            client_rect: Some(client),
            monitor_rect: Some(monitor),
            dpi: Some(144),
        }
    }

    const FOUR_K: u64 = 3840 * 2160;

    #[test]
    fn the_view_is_the_window_on_its_main_monitor() {
        let left = rect(0, 0, 3840, 2160);
        // 玩家日志：客户区约 6400×2160 横跨两块 4K 屏 → 只截左屏那块，不再“超过上限”。
        let spanning = window(rect(0, 0, 6400, 2160), left);
        assert_eq!(view_rect(&spanning).unwrap(), left);
        // 没跨屏：整个客户区，和以前一样。
        let normal = window(rect(200, 100, 1920, 1080), left);
        assert_eq!(view_rect(&normal).unwrap(), rect(200, 100, 1920, 1080));
        // 前置条件照旧：最小化、不在前台、看不见的不截。
        let mut minimized = normal.clone();
        minimized.minimized = true;
        assert!(matches!(
            view_rect(&minimized),
            Err(WindowToolError::Minimized)
        ));
        let mut background = normal.clone();
        background.foreground = false;
        assert!(matches!(
            view_rect(&background),
            Err(WindowToolError::NotForeground)
        ));
        let mut cloaked = normal.clone();
        cloaked.cloaked = true;
        assert!(matches!(
            view_rect(&cloaked),
            Err(WindowToolError::NotVisible)
        ));
        // 整个移到屏幕外：没有可截的，不退化成截桌面。
        let off = window(rect(5000, 0, 100, 100), left);
        assert!(matches!(
            view_rect(&off),
            Err(WindowToolError::InvalidGeometry(_))
        ));
    }

    #[test]
    fn finishing_a_view_shrinks_oversized_images_and_explains_it() {
        // 带鱼屏 5120×2160 的全屏游戏：没跨屏，但超过 4K 像素 → 等比缩小，提示 DOWNSCALED。
        // （按 1/10 的同比例尺寸测：512×216 对上限 384×216，单测跑得快。）
        let view = rect(0, 0, 512, 216);
        let max = 384 * 216;
        let rgba = [10u8, 20, 30, 255].repeat(512 * 216);
        let mut warnings = Vec::new();
        let (out, w, h) = finish_view(rgba, view, view, max, &mut warnings);
        assert!(u64::from(w) * u64::from(h) <= max);
        assert_eq!(out.len(), (w * h * 4) as usize);
        assert!(out.chunks(4).all(|p| p == [10, 20, 30, 255]));
        assert!((f64::from(w) / f64::from(h) - 512.0 / 216.0).abs() < 0.02);
        assert_eq!(
            warnings.iter().map(|w| w.code.as_str()).collect::<Vec<_>>(),
            [WARNING_DOWNSCALED]
        );
        assert!(warnings[0].message.contains("512×216"));
        // 跨屏：只截了一部分 → PARTIAL_VIEW；4K 一块屏不超限，不缩小。
        let mut warnings = Vec::new();
        let left = rect(0, 0, 3840, 2160);
        let (_, w, h) = finish_view(
            vec![0; 3840 * 2160 * 4],
            left,
            rect(0, 0, 6400, 2160),
            FOUR_K,
            &mut warnings,
        );
        assert_eq!((w, h), (3840, 2160));
        assert_eq!(
            warnings.iter().map(|w| w.code.as_str()).collect::<Vec<_>>(),
            [WARNING_PARTIAL_VIEW]
        );
        assert!(warnings[0].message.contains("6400×2160"));
        // 普通窗口：什么都不加，像素原样。
        let mut warnings = Vec::new();
        let small = rect(10, 10, 4, 2);
        let pixels = (0..32).collect::<Vec<u8>>();
        let (out, w, h) = finish_view(pixels.clone(), small, small, FOUR_K, &mut warnings);
        assert_eq!((out, w, h), (pixels, 4, 2));
        assert!(warnings.is_empty());
    }

    #[test]
    fn backend_reports_method_and_limitations() {
        let info = GdiForegroundCapturer.backend_info();
        assert_eq!(info.method, GDI_CAPTURE_METHOD);
        assert!(info.limitations.iter().any(|l| l.contains("foreground")));
        assert!(info
            .limitations
            .iter()
            .any(|l| l.contains("Exclusive-fullscreen")));
    }
}
