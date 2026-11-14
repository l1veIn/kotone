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

use kotone_core::tools::{
    CaptureBackendInfo, CaptureWarning, CapturedImage, PhysicalRect, WindowCapturer,
    WindowIdentity, WindowInfo, WindowObserver, WindowToolError,
};

/// GDI 后端的方法标识（进入 capabilities 与每个捕获产物的来源记录）。
pub const GDI_CAPTURE_METHOD: &str = "gdi-screen-foreground-client";

/// 必须告知调用方的 GDI 捕获边界。
pub fn gdi_capture_limitations() -> Vec<String> {
    [
        "Copies the pixels composed on screen inside the target window's client rectangle while that window is the foreground window; topmost windows or overlays covering that rectangle are included.",
        "Hidden, occluded, minimized, cloaked or background windows are not captured and occluded content is not restored; a non-foreground target is rejected rather than focused.",
        "Exclusive-fullscreen, hardware-overlay or protected GPU content may come back black or stale; check the UNIFORM_PIXELS and EXCLUSIVE_FULLSCREEN_SUSPECTED warnings.",
        "The client area must lie entirely within one monitor; windows that are partially off-screen or span monitors are rejected instead of falling back to a desktop capture.",
        "Coordinates are physical pixels read under a per-monitor-v2 DPI context; the service never moves, shows or focuses windows.",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// 前台客户区像素全部相同（全黑/全白等）时给出的警告码。
pub const WARNING_UNIFORM_PIXELS: &str = "UNIFORM_PIXELS";
/// 系统报告 Direct3D 独占全屏时给出的警告码。
pub const WARNING_EXCLUSIVE_FULLSCREEN: &str = "EXCLUSIVE_FULLSCREEN_SUSPECTED";

/// RGBA 像素是否全部相同。
pub fn is_uniform_rgba(rgba: &[u8]) -> bool {
    let mut chunks = rgba.chunks_exact(4);
    let Some(first) = chunks.next() else {
        return true;
    };
    chunks.all(|px| px == first)
}

/// Windows 窗口观察实现。
#[derive(Debug, Default, Clone, Copy)]
pub struct WindowsWindowObserver;

/// GDI 前台可见客户区捕获实现。
#[derive(Debug, Default, Clone, Copy)]
pub struct GdiForegroundCapturer;

#[cfg(windows)]
pub use imp::{current_process_gdi_objects, process_started_at};

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
        BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HBITMAP, HDC, HGDIOBJ, MONITORINFO,
        MONITOR_DEFAULTTONULL, SRCCOPY,
    };
    use windows::Win32::System::Threading::{
        GetCurrentProcess, GetExitCodeProcess, GetGuiResources, GetProcessTimes, OpenProcess,
        QueryFullProcessImageNameW, GR_GDIOBJECTS, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows::Win32::UI::HiDpi::{
        GetDpiForWindow, SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT,
        DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetAncestor, GetClassNameW, GetClientRect, GetForegroundWindow,
        GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindow,
        IsWindowVisible, GA_ROOT,
    };

    const STILL_ACTIVE: u32 = 259;

    /// 线程级 DPI 上下文作用域：进入时切到 Per-Monitor-V2，离开时恢复原值。
    struct DpiScope(DPI_AWARENESS_CONTEXT);

    impl DpiScope {
        fn per_monitor_v2() -> Self {
            Self(unsafe {
                SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)
            })
        }
    }

    impl Drop for DpiScope {
        fn drop(&mut self) {
            if !self.0 .0.is_null() {
                unsafe {
                    SetThreadDpiAwarenessContext(self.0);
                }
            }
        }
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
            let _dpi = DpiScope::per_monitor_v2();
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
            let _dpi = DpiScope::per_monitor_v2();
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
            let _dpi = DpiScope::per_monitor_v2();
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
                SRCCOPY,
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
            let _dpi = DpiScope::per_monitor_v2();
            let before = inspect_in_scope(identity)?;
            let rect = capturable_rect(&before, max_pixels)?;
            let rgba = copy_screen_rect(rect)?;
            // 复制后再次核对：期间失去前台、移动/缩放或关闭的帧不交付
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
            if after.client_rect != Some(rect) {
                return Err(WindowToolError::ChangedDuringCapture(
                    "客户区已移动或改变大小".into(),
                ));
            }
            let mut warnings = Vec::new();
            if is_uniform_rgba(&rgba) {
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

    pub fn process_started_at(_pid: u32) -> Option<u64> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uniform_detection() {
        assert!(is_uniform_rgba(&[]));
        assert!(is_uniform_rgba(&[0, 0, 0, 255, 0, 0, 0, 255]));
        assert!(!is_uniform_rgba(&[0, 0, 0, 255, 0, 0, 1, 255]));
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
