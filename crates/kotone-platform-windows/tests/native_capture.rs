//! 原生窗口观察 / GDI 前台客户区捕获测试，只使用本测试创建的受控窗口。
//!
//! - 默认测试不需要焦点：窗口以 SW_SHOWNOACTIVATE 显示，验证枚举、物理客户区、
//!   非前台/最小化/关闭/身份不符时的明确错误，以及捕获调用不改变前台窗口。
//! - `#[ignore]` 测试需要交互式桌面并会短暂把测试窗口切到前台（结束后尝试还原），
//!   验证逐像素内容、完整的 HTTP 工具链与 GDI 对象不泄漏：
//!   `cargo test -p kotone-platform-windows --test native_capture -- --ignored --test-threads=1`

#![cfg(windows)]

use std::sync::atomic::{AtomicIsize, AtomicU32, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use kotone_core::tools::{
    WindowCapturer, WindowIdentity, WindowInfo, WindowObserver, WindowToolError,
};
use kotone_platform_windows::window_tools::{
    current_process_gdi_objects, GdiForegroundCapturer, WindowsWindowObserver, GDI_CAPTURE_METHOD,
};

use windows::core::PCWSTR;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateSolidBrush, EndPaint, FillRect, HBRUSH, PAINTSTRUCT,
};
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::HiDpi::{
    SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
    GetClientRect, GetForegroundWindow, GetMessageW, GetWindowThreadProcessId, PostMessageW,
    PostQuitMessage, RegisterClassW, SetForegroundWindow, SetLayeredWindowAttributes, SetWindowPos,
    ShowWindow, ShowWindowAsync, TranslateMessage, UnregisterClassW, HWND_TOPMOST, LWA_ALPHA, MSG,
    SWP_NOACTIVATE, SWP_SHOWWINDOW, SW_RESTORE, SW_SHOW, SW_SHOWMINNOACTIVE, SW_SHOWNOACTIVATE,
    WM_CLOSE, WM_DESTROY, WM_PAINT, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

const WIDTH: i32 = 160;
const HEIGHT: i32 = 100;
const RED: [u8; 4] = [255, 0, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];

static LEFT_BRUSH: AtomicIsize = AtomicIsize::new(0);
static RIGHT_BRUSH: AtomicIsize = AtomicIsize::new(0);
static CLASS_COUNTER: AtomicU32 = AtomicU32::new(0);
static GREEN_BRUSH: AtomicIsize = AtomicIsize::new(0);

unsafe extern "system" fn overlay_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);
            let mut rc = RECT::default();
            let _ = GetClientRect(hwnd, &mut rc);
            FillRect(
                hdc,
                &rc,
                HBRUSH(GREEN_BRUSH.load(Ordering::SeqCst) as *mut _),
            );
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_CLOSE => {
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// 不激活、鼠标穿透的 WS_EX_LAYERED 置顶绿色覆盖窗口（模拟弹幕类分层叠加层）。
struct LayeredOverlay {
    hwnd: HWND,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl LayeredOverlay {
    fn open(x: i32, y: i32, w: i32, h: i32) -> Self {
        if GREEN_BRUSH.load(Ordering::SeqCst) == 0 {
            unsafe {
                GREEN_BRUSH.store(
                    CreateSolidBrush(COLORREF(0x0000_FF00)).0 as isize,
                    Ordering::SeqCst,
                );
            }
        }
        let n = CLASS_COUNTER.fetch_add(1, Ordering::SeqCst);
        let class = format!("KotoneCaptureOverlay{}_{n}", std::process::id());
        let (tx, rx) = mpsc::channel::<isize>();
        let thread = std::thread::spawn(move || unsafe {
            SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            let class_w = wide(&class);
            let wc = WNDCLASSW {
                lpfnWndProc: Some(overlay_wndproc),
                lpszClassName: PCWSTR(class_w.as_ptr()),
                ..Default::default()
            };
            assert_ne!(RegisterClassW(&wc), 0, "RegisterClassW overlay");
            let hwnd = CreateWindowExW(
                WS_EX_LAYERED
                    | WS_EX_TOPMOST
                    | WS_EX_TOOLWINDOW
                    | WS_EX_NOACTIVATE
                    | WS_EX_TRANSPARENT,
                PCWSTR(class_w.as_ptr()),
                PCWSTR(class_w.as_ptr()),
                WS_POPUP,
                x,
                y,
                w,
                h,
                None,
                None,
                None,
                None,
            )
            .expect("CreateWindowExW overlay");
            SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA)
                .expect("SetLayeredWindowAttributes");
            let _ = SetWindowPos(
                hwnd,
                Some(HWND_TOPMOST),
                x,
                y,
                w,
                h,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
            tx.send(hwnd.0 as isize).unwrap();
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            let _ = UnregisterClassW(PCWSTR(class_w.as_ptr()), None);
        });
        let hwnd = HWND(rx.recv().unwrap() as *mut _);
        std::thread::sleep(Duration::from_millis(300));
        Self {
            hwnd,
            thread: Some(thread),
        }
    }

    fn close(&mut self) {
        unsafe {
            let _ = PostMessageW(Some(self.hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
    }
}

impl Drop for LayeredOverlay {
    fn drop(&mut self) {
        self.close();
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);
            let mut rc = RECT::default();
            let _ = GetClientRect(hwnd, &mut rc);
            let mid = (rc.right - rc.left) / 2;
            let left = RECT { right: mid, ..rc };
            let right = RECT { left: mid, ..rc };
            FillRect(
                hdc,
                &left,
                HBRUSH(LEFT_BRUSH.load(Ordering::SeqCst) as *mut _),
            );
            FillRect(
                hdc,
                &right,
                HBRUSH(RIGHT_BRUSH.load(Ordering::SeqCst) as *mut _),
            );
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_CLOSE => {
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// 受控测试窗口：独立线程持有消息循环，左半红、右半蓝，置顶、不进任务栏。
struct TestWindow {
    hwnd: HWND,
    title: String,
    thread: Option<std::thread::JoinHandle<()>>,
}

unsafe impl Send for TestWindow {}

impl TestWindow {
    fn open(activate: bool) -> Self {
        if LEFT_BRUSH.load(Ordering::SeqCst) == 0 {
            // 画刷只创建一次，避免重绘时 GDI 计数抖动影响泄漏检测
            unsafe {
                LEFT_BRUSH.store(
                    CreateSolidBrush(COLORREF(0x0000_00FF)).0 as isize,
                    Ordering::SeqCst,
                );
                RIGHT_BRUSH.store(
                    CreateSolidBrush(COLORREF(0x00FF_0000)).0 as isize,
                    Ordering::SeqCst,
                );
            }
        }
        let n = CLASS_COUNTER.fetch_add(1, Ordering::SeqCst);
        let title = format!("kotone-native-capture-test-{}-{n}", std::process::id());
        let class = format!("KotoneCaptureTest{}_{n}", std::process::id());
        let (tx, rx) = mpsc::channel::<isize>();
        let thread_title = title.clone();
        let thread = std::thread::spawn(move || unsafe {
            SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            let class_w = wide(&class);
            let title_w = wide(&thread_title);
            let wc = WNDCLASSW {
                lpfnWndProc: Some(wndproc),
                lpszClassName: PCWSTR(class_w.as_ptr()),
                ..Default::default()
            };
            assert_ne!(RegisterClassW(&wc), 0, "RegisterClassW");
            let ex = if activate {
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW
            } else {
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE
            };
            let hwnd = CreateWindowExW(
                ex,
                PCWSTR(class_w.as_ptr()),
                PCWSTR(title_w.as_ptr()),
                WS_POPUP,
                64,
                64,
                WIDTH,
                HEIGHT,
                None,
                None,
                None,
                None,
            )
            .expect("CreateWindowExW");
            let _ = ShowWindow(hwnd, if activate { SW_SHOW } else { SW_SHOWNOACTIVATE });
            tx.send(hwnd.0 as isize).unwrap();
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            let _ = UnregisterClassW(PCWSTR(class_w.as_ptr()), None);
        });
        let hwnd = HWND(rx.recv().unwrap() as *mut _);
        std::thread::sleep(Duration::from_millis(150));
        Self {
            hwnd,
            title,
            thread: Some(thread),
        }
    }

    fn close(&mut self) {
        unsafe {
            let _ = PostMessageW(Some(self.hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
    }

    fn find(&self) -> WindowInfo {
        WindowsWindowObserver
            .list_windows()
            .unwrap()
            .into_iter()
            .find(|w| w.title == self.title)
            .expect("window.list 应包含受控测试窗口")
    }
}

impl Drop for TestWindow {
    fn drop(&mut self) {
        self.close();
    }
}

fn foreground() -> isize {
    unsafe { GetForegroundWindow().0 as isize }
}

fn wait_until(mut f: impl FnMut() -> bool, timeout: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    f()
}

#[test]
fn observer_reports_physical_client_area_and_capture_fails_closed() {
    let mut window = TestWindow::open(false);
    assert_ne!(
        foreground(),
        window.hwnd.0 as isize,
        "受控窗口以不激活方式显示"
    );
    let info = window.find();
    assert_eq!(info.identity.pid, std::process::id());
    assert!(info.identity.process_started_at.is_some());
    assert!(info.visible && !info.minimized && !info.cloaked);
    let rect = info.client_rect.expect("客户区");
    eprintln!(
        "native window: dpi={:?} client={:?} monitor={:?}",
        info.dpi, rect, info.monitor_rect
    );
    assert_eq!((rect.width, rect.height), (WIDTH as u32, HEIGHT as u32));
    assert!(info.monitor_rect.is_some() && info.dpi.is_some());
    assert!(info.class_name.starts_with("KotoneCaptureTest"));
    assert_eq!(
        GdiForegroundCapturer.backend_info().method,
        GDI_CAPTURE_METHOD
    );

    let gdi_before = current_process_gdi_objects();
    if !info.foreground {
        // 非前台：拒绝捕获，且不替调用方切换焦点
        for _ in 0..10 {
            assert_eq!(
                GdiForegroundCapturer
                    .capture_foreground_client(&info.identity, 1 << 24)
                    .unwrap_err(),
                WindowToolError::NotForeground
            );
        }
        assert_ne!(
            foreground(),
            window.hwnd.0 as isize,
            "捕获调用不得把目标切到前台"
        );
    }
    // 身份绑定：PID 或进程创建时间不符 → IdentityChanged
    let wrong_pid = WindowIdentity {
        pid: info.identity.pid.wrapping_add(1),
        ..info.identity
    };
    assert_eq!(
        WindowsWindowObserver.inspect(&wrong_pid).unwrap_err(),
        WindowToolError::IdentityChanged
    );
    let wrong_start = WindowIdentity {
        process_started_at: Some(1),
        ..info.identity
    };
    assert_eq!(
        GdiForegroundCapturer
            .capture_foreground_client(&wrong_start, 1 << 24)
            .unwrap_err(),
        WindowToolError::IdentityChanged
    );
    // 最小化 → Minimized，且客户区为 null
    unsafe {
        let _ = ShowWindowAsync(window.hwnd, SW_SHOWMINNOACTIVE);
    }
    assert!(wait_until(
        || WindowsWindowObserver
            .inspect(&info.identity)
            .unwrap()
            .minimized,
        Duration::from_secs(2)
    ));
    let minimized = WindowsWindowObserver.inspect(&info.identity).unwrap();
    assert!(minimized.client_rect.is_none());
    assert_eq!(
        GdiForegroundCapturer
            .capture_foreground_client(&info.identity, 1 << 24)
            .unwrap_err(),
        WindowToolError::Minimized
    );
    assert_eq!(
        current_process_gdi_objects(),
        gdi_before,
        "失败路径不泄漏 GDI 对象"
    );
    window.close();
    assert_eq!(
        WindowsWindowObserver.inspect(&info.identity).unwrap_err(),
        WindowToolError::Gone
    );
    assert_eq!(
        GdiForegroundCapturer
            .capture_foreground_client(&info.identity, 1 << 24)
            .unwrap_err(),
        WindowToolError::Gone
    );
}

/// 尝试让测试窗口成为前台（附加到当前前台线程的输入队列）；返回之前的前台窗口。
fn bring_to_foreground(hwnd: HWND) -> HWND {
    unsafe {
        let previous = GetForegroundWindow();
        let fg_thread = GetWindowThreadProcessId(previous, None);
        let me = GetCurrentThreadId();
        let attached =
            fg_thread != 0 && fg_thread != me && AttachThreadInput(me, fg_thread, true).as_bool();
        let _ = ShowWindow(hwnd, SW_RESTORE);
        let _ = BringWindowToTop(hwnd);
        let _ = SetForegroundWindow(hwnd);
        if attached {
            let _ = AttachThreadInput(me, fg_thread, false);
        }
        previous
    }
}

fn restore_foreground(previous: HWND) {
    if !previous.0.is_null() {
        unsafe {
            let _ = SetForegroundWindow(previous);
        }
    }
}

fn pixel(rgba: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * width + x) * 4) as usize;
    [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]]
}

#[test]
#[ignore = "需要交互式桌面，并会短暂把受控测试窗口切到前台"]
fn foreground_capture_exact_pixels_full_chain_and_no_gdi_leak() {
    let mut window = TestWindow::open(true);
    let previous = bring_to_foreground(window.hwnd);
    let got_foreground = wait_until(
        || WindowsWindowObserver.find_foreground_title() == Some(window.title.clone()),
        Duration::from_secs(3),
    );
    if !got_foreground {
        window.close();
        restore_foreground(previous);
        panic!("无法让受控测试窗口成为前台（前台锁定）；像素验证未执行");
    }
    std::thread::sleep(Duration::from_millis(300));
    let info = window.find();
    assert!(info.foreground);

    // 1) 直接调用 GDI 后端：逐像素核对与 GDI 对象计数
    let gdi_before = current_process_gdi_objects();
    let mut last = None;
    for _ in 0..25 {
        let image = GdiForegroundCapturer
            .capture_foreground_client(&info.identity, 1 << 24)
            .expect("前台受控窗口应可捕获");
        assert_eq!((image.width, image.height), (WIDTH as u32, HEIGHT as u32));
        assert_eq!(image.client_rect, info.client_rect.unwrap());
        assert_eq!(pixel(&image.rgba, image.width, 5, 50), RED);
        assert_eq!(pixel(&image.rgba, image.width, WIDTH as u32 - 5, 50), BLUE);
        assert_eq!(
            pixel(&image.rgba, image.width, WIDTH as u32 / 2 - 1, 0),
            RED
        );
        assert_eq!(
            pixel(
                &image.rgba,
                image.width,
                WIDTH as u32 / 2,
                HEIGHT as u32 - 1
            ),
            BLUE
        );
        assert!(image.warnings.iter().all(|w| w.code != "UNIFORM_PIXELS"));
        last = Some(image);
    }
    assert_eq!(
        current_process_gdi_objects(),
        gdi_before,
        "成功路径不泄漏 GDI 对象"
    );
    drop(last);

    // 1b) CAPTUREBLT：覆盖在目标客户区上的分层（WS_EX_LAYERED）窗口出现在图中，
    //     且叠加层不激活、目标仍为前台。
    let client = info.client_rect.unwrap();
    let mut overlay = LayeredOverlay::open(client.x + 20, client.y + 20, 40, 30);
    assert_eq!(
        foreground(),
        window.hwnd.0 as isize,
        "不激活的叠加层不改变前台"
    );
    let image = GdiForegroundCapturer
        .capture_foreground_client(&info.identity, 1 << 24)
        .expect("叠加层存在时仍可捕获前台目标");
    assert_eq!(
        pixel(&image.rgba, image.width, 30, 30),
        GREEN,
        "分层叠加层被合入"
    );
    assert_eq!(pixel(&image.rgba, image.width, 59, 49), GREEN);
    assert_eq!(
        pixel(&image.rgba, image.width, 19, 30),
        RED,
        "叠加层外仍是目标像素"
    );
    assert_eq!(pixel(&image.rgba, image.width, 60, 30), RED);
    assert_eq!(pixel(&image.rgba, image.width, WIDTH as u32 - 5, 50), BLUE);
    overlay.close();
    std::thread::sleep(Duration::from_millis(300));
    let image = GdiForegroundCapturer
        .capture_foreground_client(&info.identity, 1 << 24)
        .expect("移除叠加层后捕获");
    assert_eq!(
        pixel(&image.rgba, image.width, 30, 30),
        RED,
        "叠加层移除后恢复目标像素"
    );

    // 2) 完整链路：真实后端 → ToolService → loopback HTTP → 客户端 → crop
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        use kotone_tools::testing::{decode_png, FakeHost};
        use serde_json::json;
        use std::sync::Arc;
        let dir = tempfile::tempdir().unwrap();
        let host = FakeHost::new(Some(1));
        let deps = kotone_tools::ServiceDeps {
            instance_id: "inst_native".into(),
            app_version: "test".into(),
            observer: Arc::new(WindowsWindowObserver),
            capturer: Arc::new(GdiForegroundCapturer),
            asr_phase: Arc::new(|| "stopped".into()),
            limits: kotone_tools::ServiceLimits::default(),
            clock: Arc::new(kotone_tools::artifacts::SystemClock),
            ocr_backends: Vec::new(),
            host: std::sync::Arc::new(kotone_tools::host::NoAppHost),
        };
        let running = kotone_tools::RunningToolService::start(deps, host.clone(), dir.path())
            .await
            .unwrap();
        let client = kotone_tools::ToolClient::discover(dir.path(), host.as_ref()).unwrap();
        let call = |tool: &str, args: serde_json::Value| {
            let request = kotone_tools::protocol::ToolCallRequest {
                request_id: None,
                tool: tool.into(),
                arguments: args,
                timeout_ms: Some(5_000),
            };
            let client = &client;
            async move { client.call(&request).await.unwrap() }
        };
        let list = call("window.list", json!({"titleContains": window.title})).await;
        let id = list.result.unwrap()["windows"][0]["windowId"].clone();
        let capture = call("screen.capture", json!({"windowId": id})).await;
        assert!(capture.ok, "{:?}", capture.error);
        let capture = capture.result.unwrap();
        assert_eq!(capture["source"]["captureMethod"], GDI_CAPTURE_METHOD);
        let half = WIDTH / 2;
        let crop = call(
            "image.crop",
            json!({"artifactId": capture["artifactId"], "rect": {"x": half, "y": 0, "width": half, "height": HEIGHT}}),
        )
        .await
        .result
        .unwrap();
        let got = call("artifact.get", json!({"artifactId": crop["artifactId"]})).await;
        let data = got.result.unwrap()["content"]["data"].as_str().unwrap().to_string();
        use base64::Engine as _;
        let png = base64::engine::general_purpose::STANDARD.decode(data).unwrap();
        let (w, h, px) = decode_png(&png);
        assert_eq!((w, h), (half as u32, HEIGHT as u32));
        assert!(px.chunks_exact(4).all(|p| p == BLUE), "右半裁剪应全部为蓝色");
        // 最小化 → WINDOW_MINIMIZED；关闭 → WINDOW_GONE
        unsafe {
            let _ = ShowWindowAsync(window.hwnd, SW_SHOWMINNOACTIVE);
        }
        tokio::time::sleep(Duration::from_millis(400)).await;
        let minimized = call("screen.capture", json!({"windowId": id})).await;
        assert_eq!(minimized.error.unwrap().code, "WINDOW_MINIMIZED");
        window.close();
        let gone = call("screen.capture", json!({"windowId": id})).await;
        assert_eq!(gone.error.unwrap().code, "WINDOW_GONE");
        drop(running);
    });
    restore_foreground(previous);
}

trait ForegroundTitle {
    fn find_foreground_title(&self) -> Option<String>;
}

impl ForegroundTitle for WindowsWindowObserver {
    fn find_foreground_title(&self) -> Option<String> {
        self.foreground_window().ok().flatten().map(|w| w.title)
    }
}

// ---------- 游戏场景的窗口操作（game_window，ADR 014 第 5 步） ----------

#[test]
fn game_window_close_posts_a_normal_close_request() {
    use kotone_platform_windows::game_window;
    let window = TestWindow::open(false);
    let handle = window.find().identity.handle;
    game_window::close_window(handle).expect("WM_CLOSE");
    assert!(
        wait_until(
            || !WindowsWindowObserver
                .list_windows()
                .unwrap()
                .iter()
                .any(|w| w.title == window.title),
            Duration::from_secs(3)
        ),
        "窗口应在收到关闭请求后消失"
    );
    // 已关闭的窗口：明确报错，不假装成功。
    assert!(game_window::close_window(handle).is_err());
    assert!(game_window::focus_window(handle).is_err());
}

#[test]
#[ignore = "需要交互式桌面，并会短暂把受控测试窗口切到前台"]
fn game_window_focus_restores_a_minimized_window_to_the_front() {
    use kotone_platform_windows::game_window;
    let window = TestWindow::open(true);
    let previous = unsafe { GetForegroundWindow() };
    unsafe {
        let _ = ShowWindow(window.hwnd, SW_SHOWMINNOACTIVE);
    }
    assert!(wait_until(
        || window.find().minimized,
        Duration::from_secs(2)
    ));
    let result = game_window::focus_window(window.hwnd.0 as u64);
    let focused = foreground() == window.hwnd.0 as isize;
    let minimized = window.find().minimized;
    restore_foreground(previous);
    result.expect("focus_window");
    assert!(focused, "目标窗口应在前台");
    assert!(!minimized, "最小化的窗口应被还原");
}
