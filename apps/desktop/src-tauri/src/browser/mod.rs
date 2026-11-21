//! 内置浏览器（ADR 031）：聊天页右侧栏里的子网页视图，后台任务经调试协议操作它。
//!
//! - 视图是主窗口（`main`，关闭时只是隐藏）里的一个 Tauri 子网页视图（`add_child`，label `browser`），
//!   第一次打开网页时创建；数据单独放在 `<KOTONE_HOME>/browser/`（独立的 WebView2 环境，和 Kotone
//!   自己的界面分开）。右侧栏收起时只隐藏、不销毁：页面、登录、正在放的歌都保留（照 DSH 桌面版
//!   `keepMounted`，`.agents/notes/implemented/feature/2026-09-20-desktop-browser-webview.md`，MIT）。
//! - 访客策略（照同一篇 DSH 记录）：只许不带账号密码的 http(s)，拦本机、内网、Kotone 自己的地址
//!   （网址检查照 xylt369 url-guard）；拒绝权限请求、下载、HTTP 认证弹窗；新窗口在本视图里打开；
//!   网页不能抢焦点、弹对话框。网页调用不了 Kotone 的命令：Tauri 2.11 对非本地来源一律拒绝自定义命令
//!   （`webview/mod.rs` 的 remote-origin 守卫），我们的 capabilities 也没有 `remote`（见测试）。
//! - 被全屏游戏挡住时收不到输入（wqty123/dsh-browser `docs/limitations.md`）：浏览器环境加
//!   `--disable-features=CalculateNativeWinOcclusion` 等启动参数；点击、输入走页面内的 `click()` /
//!   原生 setter，不走输入管线。

mod cdp;
mod page;

pub use page::*;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use kotone_core::log;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager, Webview};

pub const LABEL: &str = "browser";
pub const EVENT: &str = "kotone://browser";

/// 只加在浏览器自己的 WebView2 环境上（独立数据目录），不影响 Kotone 主界面。
/// wry 默认会传 `--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection`，自己传参数时要带上。
const BROWSER_ARGS: &str = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection,CalculateNativeWinOcclusion \
--disable-renderer-backgrounding --disable-background-timer-throttling --disable-backgrounding-occluded-windows \
--autoplay-policy=no-user-gesture-required";

/// 交给界面的浏览器状态（`kotone://browser`，接口约定见右侧栏）。
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserState {
    pub open: bool,
    pub url: Option<String>,
    pub title: Option<String>,
    pub site: Option<String>,
    pub loading: bool,
    pub handover: Option<Handover>,
    pub media: Option<Media>,
    pub busy: bool,
    /// 浏览器没能打开时给玩家看的原因（右侧栏网页卡片上显示）。
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Handover {
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Media {
    pub playing: bool,
    pub title: Option<String>,
    pub artist: Option<String>,
}

/// 右侧栏里浏览器区域的位置（逻辑像素，相对窗口内容区左上角）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize)]
pub struct Bounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub visible: bool,
}

/// 最近一次读页面（编号只认这一次的；导航后清空，旧编号就会被拒绝）。
#[derive(Default)]
struct Memo {
    url: String,
    text: String,
    refs: Vec<kotone_agent::browser::SnapshotRef>,
    next: Option<usize>,
}

#[derive(Default)]
struct Inner {
    webview: Option<Webview>,
    state: BrowserState,
    bounds: Bounds,
    memo: Option<Memo>,
    busy: usize,
    handover: Option<tokio::sync::oneshot::Sender<()>>,
}

#[derive(Default)]
pub struct BrowserRuntime {
    inner: Mutex<Inner>,
    /// 创建视图串行（第一次 open 同时来两个也只建一个）。
    create: tokio::sync::Mutex<()>,
    /// 页面操作串行（同一时间只有一件后台的事在动这个页面）。
    ops: tokio::sync::Mutex<()>,
    /// 去抖：只应用最新一次的位置。
    bounds_gen: AtomicU64,
    /// 轮询歌和标题的任务代数（关掉视图时让旧任务退出）。
    poll_gen: AtomicU64,
}

fn runtime(app: &AppHandle) -> Option<tauri::State<'_, BrowserRuntime>> {
    app.try_state::<BrowserRuntime>()
}

pub fn data_dir() -> std::path::PathBuf {
    kotone_core::settings::kotone_dir().join("browser")
}

pub fn sites_path() -> std::path::PathBuf {
    kotone_core::settings::kotone_dir().join("browser-sites.json")
}

pub fn state(app: &AppHandle) -> BrowserState {
    runtime(app)
        .map(|rt| rt.inner.lock().unwrap().state.clone())
        .unwrap_or_default()
}

/// 改状态，有变化才发事件（不在 with_webview 回调里调用）。
fn update(app: &AppHandle, f: impl FnOnce(&mut BrowserState)) {
    let Some(rt) = runtime(app) else { return };
    let changed = {
        let mut inner = rt.inner.lock().unwrap();
        let before = inner.state.clone();
        f(&mut inner.state);
        (inner.state != before).then(|| inner.state.clone())
    };
    if let Some(state) = changed {
        let _ = app.emit(EVENT, state);
    }
}

pub(crate) fn current(app: &AppHandle) -> Option<Webview> {
    runtime(app)?.inner.lock().unwrap().webview.clone()
}

/// 后台正在操作页面（界面显示“她在操作”）。
pub(crate) struct Busy(AppHandle);

impl Busy {
    pub fn new(app: &AppHandle) -> Self {
        if let Some(rt) = runtime(app) {
            rt.inner.lock().unwrap().busy += 1;
        }
        update(app, |s| s.busy = true);
        Self(app.clone())
    }
}

impl Drop for Busy {
    fn drop(&mut self) {
        let idle = runtime(&self.0).is_some_and(|rt| {
            let mut inner = rt.inner.lock().unwrap();
            inner.busy = inner.busy.saturating_sub(1);
            inner.busy == 0
        });
        if idle {
            update(&self.0, |s| s.busy = false);
        }
    }
}

/// 网页能不能导航到这个地址（顶层导航；空白页放行）。
fn allowed_navigation(url: &url::Url) -> bool {
    url.as_str() == "about:blank" || kotone_agent::browser::check_url(url.as_str()).is_ok()
}

/// 没有视图就建一个（第一次打开网页时）。必须在 tokio 线程上调用：`add_child` 会等主线程建好。
pub(crate) async fn ensure(app: &AppHandle) -> Result<Webview, String> {
    if let Some(webview) = current(app) {
        return Ok(webview);
    }
    let rt = runtime(app).ok_or("浏览器还没准备好")?;
    let _guard = rt.create.lock().await;
    if let Some(webview) = current(app) {
        return Ok(webview);
    }
    let window = app.get_window("main").ok_or("主窗口还没准备好")?;
    // 开发 / 真机脚本用的 WebView2 环境变量会盖过这里指定的数据目录和启动参数，浏览器就会和 Kotone
    // 主界面抢同一个数据目录、启动参数又不同，WebView2 报 0x8007139F（状态不对）建不出来
    // （2026-11-21 真机）。这两个变量只给 Kotone 自己的界面用（它们已经建好了），建浏览器前去掉。
    // 玩家不会设它们；副作用是之后才建的悬浮窗不再用测试目录。
    for key in [
        "WEBVIEW2_USER_DATA_FOLDER",
        "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS",
    ] {
        if std::env::var_os(key).is_some() {
            std::env::remove_var(key);
            log::log(&format!(
                "browser: 去掉了开发用的环境变量 {key}，浏览器用自己的数据目录"
            ));
        }
    }
    let dir = data_dir();
    let _ = std::fs::create_dir_all(&dir);
    let nav_app = app.clone();
    let load_app = app.clone();
    let builder = tauri::webview::WebviewBuilder::new(
        LABEL,
        tauri::WebviewUrl::External("about:blank".parse().map_err(|_| "about:blank")?),
    )
    .data_directory(dir)
    .additional_browser_args(BROWSER_ARGS)
    // 不抢焦点：wry 建好后对要焦点的视图调 MoveFocus，失败就把整个创建判为失败；主窗口最小化时
    // MoveFocus 报 0x80070057，浏览器就建不出来（rc1 玩家试玩时复现，见 ADR 031）。浏览器本来也
    // 不该抢焦点（玩家可能在游戏里）。
    .focused(false)
    .initialization_script_for_all_frames(kotone_agent::browser::GUEST_INIT_SCRIPT)
    .on_navigation(allowed_navigation)
    // 下载一律拒绝（DSH 访客策略）。
    .on_download(|_, _| false)
    // 新窗口请求在本视图里打开（照 DSH：弹出的 http(s) 页面进同一个浏览器）。
    .on_new_window(move |url, _features| {
        if allowed_navigation(&url) {
            let app = nav_app.clone();
            tauri::async_runtime::spawn(async move {
                if let Some(webview) = current(&app) {
                    let _ = webview.navigate(url);
                }
            });
        }
        tauri::webview::NewWindowResponse::Deny
    })
    .on_page_load(move |webview, payload| {
        let started = matches!(payload.event(), tauri::webview::PageLoadEvent::Started);
        let url = payload.url().to_string();
        let app = load_app.clone();
        let _ = webview;
        // 不在回调里发事件：交给 tokio。
        tauri::async_runtime::spawn(async move {
            if started {
                if let Some(rt) = runtime(&app) {
                    // 导航了：上一次读页面的编号作废。
                    rt.inner.lock().unwrap().memo = None;
                }
            }
            let site = url::Url::parse(&url)
                .ok()
                .filter(|u| u.has_host())
                .map(|u| kotone_agent::browser::site_of(&u));
            update(&app, |s| {
                s.loading = started;
                if url != "about:blank" {
                    s.url = Some(url);
                    s.site = site;
                }
            });
        });
    });
    let bounds = rt.inner.lock().unwrap().bounds;
    let (position, size) = placement(bounds);
    let webview =
        tauri::async_runtime::spawn_blocking(move || window.add_child(builder, position, size))
            .await
            .map_err(|e| format!("打开浏览器失败：{e}"))?
            .map_err(|e| format!("打开浏览器失败：{e}"))?;
    if !bounds.visible {
        let _ = webview.hide();
    }
    cdp::TRACE.store(
        std::env::var_os("KOTONE_BROWSER_TRACE").is_some(),
        Ordering::Relaxed,
    );
    guard_guest(&webview);
    {
        let mut inner = rt.inner.lock().unwrap();
        inner.webview = Some(webview.clone());
    }
    // add_child 只是把“创建”交给主线程就返回，真正建失败只写进日志（tauri-runtime-wry）。
    // 用一次调试协议调用确认它真的建好了；建不出来就清掉、如实告诉玩家和后台任务。
    let ready = tokio::time::timeout(
        std::time::Duration::from_secs(8),
        cdp::call(&webview, "DOM.enable", serde_json::json!({})),
    )
    .await;
    if !matches!(ready, Ok(Ok(_))) {
        let reason = match ready {
            Ok(Err(e)) => e,
            _ => "8 秒没有回应".to_string(),
        };
        log::log(&format!("browser: 浏览器视图没有建起来：{reason}"));
        rt.inner.lock().unwrap().webview = None;
        let _ = webview.close();
        let message = format!(
            "浏览器没能打开（{reason}）。这是 Kotone 的问题，不是网站的问题：不要换网站重试，直接告诉玩家，\
             建议重启 Kotone 后再试，或者把日志发给开发者。"
        );
        update(app, |s| {
            s.open = false;
            s.error = Some("浏览器没能打开，重启 Kotone 后再试".into());
        });
        return Err(message);
    }
    update(app, |s| {
        s.open = true;
        s.error = None;
    });
    log::log("browser: 浏览器视图已创建");
    start_polling(app);
    Ok(webview)
}

/// 位置：界面报的区域；没报过或收起时放在窗口外面（隐藏的视图不显示）。
fn placement(bounds: Bounds) -> (tauri::LogicalPosition<f64>, tauri::LogicalSize<f64>) {
    if bounds.width >= 1.0 && bounds.height >= 1.0 {
        (
            tauri::LogicalPosition::new(bounds.x, bounds.y),
            tauri::LogicalSize::new(bounds.width, bounds.height),
        )
    } else {
        (
            tauri::LogicalPosition::new(-10_000.0, -10_000.0),
            tauri::LogicalSize::new(960.0, 720.0),
        )
    }
}

/// 拒绝权限请求（麦克风、摄像头、定位、通知……）和 HTTP 认证弹窗（照 DSH 访客策略）。
fn guard_guest(webview: &Webview) {
    let _ = webview.with_webview(|platform| {
        #[cfg(windows)]
        unsafe {
            use webview2_com::{
                BasicAuthenticationRequestedEventHandler, Microsoft::Web::WebView2::Win32::*,
                PermissionRequestedEventHandler, ScriptDialogOpeningEventHandler,
            };
            use windows::core::Interface;
            let registered = (|| -> windows::core::Result<()> {
                let core = platform.controller().CoreWebView2()?;
                let deny = PermissionRequestedEventHandler::create(Box::new(|_, args| {
                    if let Some(args) = args {
                        args.SetState(COREWEBVIEW2_PERMISSION_STATE_DENY)?;
                    }
                    Ok(())
                }));
                let mut token = 0;
                core.add_PermissionRequested(&deny, &mut token)?;
                if let Ok(core10) = core.cast::<ICoreWebView2_10>() {
                    let cancel =
                        BasicAuthenticationRequestedEventHandler::create(Box::new(|_, args| {
                            if let Some(args) = args {
                                args.SetCancel(true)?;
                            }
                            Ok(())
                        }));
                    core10.add_BasicAuthenticationRequested(&cancel, &mut token)?;
                }
                if let Ok(settings) = core.Settings() {
                    let _ = settings.SetAreDevToolsEnabled(false);
                    let _ = settings.SetIsStatusBarEnabled(false);
                    // 网页弹原生对话框会把隐藏着的页面卡住（没人点得掉）：不用默认对话框，
                    // alert 直接确定，confirm / prompt / 离开页面提示一律当“取消”（DSH 访客策略：拒绝对话框）。
                    let _ = settings.SetAreDefaultScriptDialogsEnabled(false);
                }
                let dialogs = ScriptDialogOpeningEventHandler::create(Box::new(|_, args| {
                    if let Some(args) = args {
                        let mut kind = COREWEBVIEW2_SCRIPT_DIALOG_KIND::default();
                        args.Kind(&mut kind)?;
                        if kind == COREWEBVIEW2_SCRIPT_DIALOG_KIND_ALERT {
                            args.Accept()?;
                        }
                        kotone_core::log::log(&format!(
                            "browser: 网页想弹对话框（{}），已替玩家关掉",
                            kind.0
                        ));
                    }
                    Ok(())
                }));
                core.add_ScriptDialogOpening(&dialogs, &mut token)?;
                // 网页想拉起本机程序（网易云的“去客户端播放”用 orpheus://、QQ 的 tencent:// 等）：
                // WebView2 默认弹“要打开这个应用吗”，窗口隐藏时没人点得掉，页面会一直卡住
                // （真机：网易云点“播放”后 60 秒没反应）。一律取消、记日志（DSH 访客策略：拒绝特殊协议）。
                if let Ok(core18) = core.cast::<ICoreWebView2_18>() {
                    let block = webview2_com::LaunchingExternalUriSchemeEventHandler::create(
                        Box::new(|_, args| {
                            if let Some(args) = args {
                                let mut uri = windows::core::PWSTR::null();
                                let scheme = if args.Uri(&mut uri).is_ok() {
                                    let uri = webview2_com::take_pwstr(uri);
                                    uri.split(':').next().unwrap_or_default().to_string()
                                } else {
                                    String::new()
                                };
                                args.SetCancel(true)?;
                                kotone_core::log::log(&format!(
                                    "browser: 网页想打开本机程序（{scheme}:），已拦下"
                                ));
                            }
                            Ok(())
                        }),
                    );
                    core18.add_LaunchingExternalUriScheme(&block, &mut token)?;
                }
                // 真机脚本半夜测放歌：整个浏览器静音（页面照常播放，“在放”照样读得到）。
                if std::env::var_os("KOTONE_BROWSER_MUTE").is_some() {
                    if let Ok(core8) = core.cast::<ICoreWebView2_8>() {
                        let _ = core8.SetIsMuted(true);
                    }
                }
                Ok(())
            })();
            if let Err(e) = registered {
                kotone_core::log::log(&format!("browser: 访客策略没装上：{e}"));
            }
        }
        #[cfg(not(windows))]
        let _ = platform;
    });
}

/// Tauri / wry 用 `log` 库报自己的警告和错误（例如子网页视图在主线程上没建出来：调用方照样拿到句柄，
/// 之后发给它的消息都被悄悄丢掉），Kotone 原来没有接，这里转进 Kotone 的日志。
pub fn forward_runtime_log() {
    struct Forward;
    impl ::log::Log for Forward {
        fn enabled(&self, metadata: &::log::Metadata) -> bool {
            metadata.level() <= ::log::Level::Warn
        }
        fn log(&self, record: &::log::Record) {
            if self.enabled(record.metadata()) {
                kotone_core::log::log(&format!(
                    "{} {}: {}",
                    record.level(),
                    record.target(),
                    record.args()
                ));
            }
        }
        fn flush(&self) {}
    }
    static FORWARD: Forward = Forward;
    if ::log::set_logger(&FORWARD).is_ok() {
        ::log::set_max_level(::log::LevelFilter::Warn);
    }
}

/// 界面报来浏览器区域的位置（去抖 40 ms，只应用最新一次）。
pub async fn set_bounds(app: &AppHandle, bounds: Bounds) {
    let Some(rt) = runtime(app) else { return };
    rt.inner.lock().unwrap().bounds = bounds;
    let generation = rt.bounds_gen.fetch_add(1, Ordering::SeqCst) + 1;
    tokio::time::sleep(Duration::from_millis(40)).await;
    let Some(rt) = runtime(app) else { return };
    if rt.bounds_gen.load(Ordering::SeqCst) != generation {
        return;
    }
    let Some(webview) = current(app) else { return };
    let (position, size) = placement(bounds);
    let _ = webview.set_position(position);
    let _ = webview.set_size(size);
    let _ = if bounds.visible && bounds.width >= 1.0 && bounds.height >= 1.0 {
        webview.show()
    } else {
        webview.hide()
    };
}

/// 每 2 秒读一次标题、网址和正在放的歌，有变化才发事件。视图关掉后退出。
fn start_polling(app: &AppHandle) {
    let Some(rt) = runtime(app) else { return };
    let generation = rt.poll_gen.fetch_add(1, Ordering::SeqCst) + 1;
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(2)).await;
            let alive =
                runtime(&app).is_some_and(|rt| rt.poll_gen.load(Ordering::SeqCst) == generation);
            if !alive {
                return;
            }
            let Some(webview) = current(&app) else { return };
            let script = format!(
                "(() => {{ const m = {}; return {{ title: document.title, url: location.href, media: m }}; }})()",
                kotone_agent::browser::media_script("now", &[])
            );
            let Ok(value) = cdp::evaluate(&webview, &script).await else {
                continue;
            };
            let media = with_audio(
                media_of(&value["media"]),
                cdp::playing_audio(&webview).await,
            );
            let title = value["title"]
                .as_str()
                .map(str::to_string)
                .filter(|t| !t.is_empty());
            let url = value["url"]
                .as_str()
                .map(str::to_string)
                .filter(|u| u != "about:blank");
            update(&app, |s| {
                s.title = title;
                if let Some(url) = url {
                    s.site = url::Url::parse(&url)
                        .ok()
                        .filter(|u| u.has_host())
                        .map(|u| kotone_agent::browser::site_of(&u));
                    s.url = Some(url);
                }
                s.media = media;
            });
        }
    });
}

/// 媒体脚本的结果 → 界面上的“正在放”。页面里没有音视频时为 None。
/// 页面在出声就算“在放”（播放器用不挂在页面上的 Audio、或者在跨域 iframe 里，脚本找不到时也准）。
fn with_audio(media: Option<Media>, audible: Option<bool>) -> Option<Media> {
    match (media, audible) {
        (Some(mut m), Some(true)) => {
            m.playing = true;
            Some(m)
        }
        (None, Some(true)) => Some(Media {
            playing: true,
            title: None,
            artist: None,
        }),
        (media, _) => media,
    }
}

fn media_of(value: &Value) -> Option<Media> {
    if value["count"].as_u64().unwrap_or(0) == 0 {
        return None;
    }
    let text = |k: &str| {
        value[k]
            .as_str()
            .map(str::to_string)
            .filter(|t| !t.trim().is_empty())
    };
    Some(Media {
        playing: value["playing"].as_bool().unwrap_or(false),
        title: text("title"),
        artist: text("artist"),
    })
}

/// 界面上的按钮：刷新、后退、关掉、播放、暂停、下一首。
pub async fn control(app: &AppHandle, action: &str) -> Result<Value, String> {
    match action {
        "play" | "pause" | "next" => music(app, action).await,
        "reload" => {
            // 上次没建起来（卡片上显示了原因）时，“重试”就是重新建一次。
            let webview = match current(app) {
                Some(webview) => webview,
                None if state(app).error.is_some() => {
                    ensure(app).await?;
                    return Ok(serde_json::json!({"ok": true}));
                }
                None => return Err("浏览器还没打开".into()),
            };
            webview.reload().map_err(|e| e.to_string())?;
            Ok(serde_json::json!({"ok": true}))
        }
        "back" => {
            let webview = current(app).ok_or("浏览器还没打开")?;
            cdp::evaluate(&webview, "history.back(); true").await?;
            Ok(serde_json::json!({"ok": true}))
        }
        "close" => {
            close(app);
            Ok(serde_json::json!({"ok": true}))
        }
        other => Err(format!("不认识的操作：{other}")),
    }
}

/// 关掉视图（只有“关掉”和清除登录会销毁；收起只是隐藏）。
pub fn close(app: &AppHandle) {
    let webview = runtime(app).and_then(|rt| {
        rt.poll_gen.fetch_add(1, Ordering::SeqCst);
        let mut inner = rt.inner.lock().unwrap();
        inner.memo = None;
        inner.handover.take();
        inner.webview.take()
    });
    if let Some(webview) = webview {
        let _ = webview.close();
        log::log("browser: 浏览器视图已关闭");
    }
    update(app, |s| *s = BrowserState::default());
}

/// 清除浏览器登录：关掉视图，删掉它的数据目录（浏览器进程退出要一会儿，重试几次）。
pub async fn clear_data(app: &AppHandle) -> Result<(), String> {
    close(app);
    let dir = data_dir();
    let mut last = String::new();
    for _ in 0..20 {
        match std::fs::remove_dir_all(&dir) {
            Ok(()) => {
                log::log("browser: 已清除浏览器登录");
                return Ok(());
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => last = e.to_string(),
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    Err(format!("没能删掉浏览器数据（{last}），重启 Kotone 后再试"))
}

/// 玩家在横幅上点了“好了”。
pub fn handover_done(app: &AppHandle) {
    if let Some(tx) = runtime(app).and_then(|rt| rt.inner.lock().unwrap().handover.take()) {
        let _ = tx.send(());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigation_only_reaches_public_http_pages() {
        let ok = |s: &str| allowed_navigation(&url::Url::parse(s).unwrap());
        assert!(ok("about:blank"));
        assert!(ok("https://music.163.com/#/discover"));
        assert!(!ok("http://tauri.localhost/index.html"));
        assert!(!ok("http://127.0.0.1:1430/"));
        assert!(!ok("file:///C:/Users/"));
        assert!(!ok("kotone://anything"));
    }

    #[test]
    fn hidden_or_unknown_bounds_park_the_view_outside_the_window() {
        let (position, _) = placement(Bounds::default());
        assert!(position.x < 0.0 && position.y < 0.0);
        let (position, size) = placement(Bounds {
            x: 800.0,
            y: 40.0,
            width: 400.0,
            height: 600.0,
            visible: true,
        });
        assert_eq!(
            (position.x, position.y, size.width, size.height),
            (800.0, 40.0, 400.0, 600.0)
        );
    }

    #[test]
    fn audible_pages_count_as_playing() {
        assert_eq!(
            with_audio(None, Some(true)),
            Some(Media {
                playing: true,
                title: None,
                artist: None
            })
        );
        assert_eq!(with_audio(None, Some(false)), None);
        let paused = Media {
            playing: false,
            title: Some("晴天".into()),
            artist: None,
        };
        assert!(
            with_audio(Some(paused.clone()), Some(true))
                .unwrap()
                .playing
        );
        assert_eq!(with_audio(Some(paused.clone()), None), Some(paused));
    }

    #[test]
    fn media_reads_playing_title_and_artist() {
        assert_eq!(media_of(&serde_json::json!({"count": 0})), None);
        assert_eq!(
            media_of(
                &serde_json::json!({"count": 1, "playing": true, "title": "晴天", "artist": "周杰伦"})
            ),
            Some(Media {
                playing: true,
                title: Some("晴天".into()),
                artist: Some("周杰伦".into())
            })
        );
    }

    /// 网页调用不了 Kotone 的命令：Tauri 2.11 对非本地来源拒绝自定义命令，除非某个 capability 写了 `remote`。
    /// 这里确认我们的 capabilities 一个都没写（写了就要重新评估 ADR 031 的隔离）。
    #[test]
    fn no_capability_grants_remote_origins() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("capabilities");
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let text = std::fs::read_to_string(&path).unwrap();
            let json: Value = serde_json::from_str(&text).unwrap();
            assert!(
                json.get("remote").is_none(),
                "{} 不能给远程网页权限",
                path.display()
            );
            let windows = json["windows"].as_array().cloned().unwrap_or_default();
            assert!(
                !windows.iter().any(|w| w == "*" || w == LABEL),
                "{} 不能匹配浏览器视图",
                path.display()
            );
            assert!(json.get("webviews").is_none(), "{}", path.display());
        }
    }
}
