//! 一个标签页的网页视图：建视图（访客策略、建完确认）、挂 WebView2 事件（标题、网址、前进后退、页面全屏），
//! 以及前进 / 后退 / 停止。
//!
//! **回调里不 emit、不阻塞**：在 `with_webview` 回调里发事件会让 tauri-runtime-wry 自锁（2026-11-17 的教训）。
//! WebView2 的事件处理器只读属性，然后交给 tokio（`spawn`）去改状态、发事件。

use std::time::Duration;

use kotone_core::log;
use tauri::{AppHandle, Manager, Webview};

use super::{cdp, update, Bounds, LABEL_PREFIX};

/// 只加在浏览器自己的 WebView2 环境上（独立数据目录），不影响 Kotone 主界面。所有标签页用同一份数据目录，
/// 启动参数和插件开关必须完全一样（WebView2：同一数据目录的环境选项不一致会报 0x8007139F）。
/// wry 默认会传 `--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection`，自己传参数时要带上。
const BROWSER_ARGS: &str = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection,CalculateNativeWinOcclusion \
--disable-renderer-backgrounding --disable-background-timer-throttling --disable-backgrounding-occluded-windows \
--autoplay-policy=no-user-gesture-required";

/// 网页能不能导航到这个地址（顶层导航；空白页放行）。
pub(super) fn allowed_navigation(url: &url::Url) -> bool {
    url.as_str() == "about:blank" || kotone_agent::browser::check_url(url.as_str()).is_ok()
}

/// 位置：界面报的区域；没报过或收起时放在窗口外面（隐藏的视图不显示）。
pub(super) fn placement(bounds: Bounds) -> (tauri::LogicalPosition<f64>, tauri::LogicalSize<f64>) {
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

/// 建一个标签页的视图（空白页，放在窗口外、隐藏），确认真的建好了、访客策略装上了再交回。
/// 先开空白页：访客策略（拒绝权限、对话框、拉起本机程序）装好之前不加载任何网页；要打开的网址由调用方
/// 用 `location.replace` 换掉空白页（历史里不留空白页）。必须在 tokio 线程上调用：`add_child` 会等主线程。
pub(super) async fn create(app: &AppHandle, id: &str) -> Result<Webview, String> {
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
    let dir = super::data_dir();
    let _ = std::fs::create_dir_all(&dir);
    let label = format!("{LABEL_PREFIX}{id}");
    let new_window_app = app.clone();
    let download_app = app.clone();
    let load_app = app.clone();
    let opener = id.to_string();
    let loading_tab = id.to_string();
    let blank = "about:blank".parse().map_err(|_| "about:blank")?;
    let builder = tauri::webview::WebviewBuilder::new(label, tauri::WebviewUrl::External(blank))
        .data_directory(dir)
        .additional_browser_args(BROWSER_ARGS)
        // 插件（ADR 031）：环境上打开插件支持；装哪些、启用哪些由 `extensions` 经 WebView2 Profile 管
        // （不用 wry 的 extensions_path：它把目录里的子文件夹逐个装、任何一个出错整个视图就建不出来）。
        .browser_extensions_enabled(true)
        // 不抢焦点：wry 建好后对要焦点的视图调 MoveFocus，失败就把整个创建判为失败；主窗口最小化时
        // MoveFocus 报 0x80070057，浏览器就建不出来（rc1 玩家试玩时复现，见 ADR 031）。浏览器本来也
        // 不该抢焦点（玩家可能在游戏里）。
        .focused(false)
        .initialization_script_for_all_frames(kotone_agent::browser::GUEST_INIT_SCRIPT)
        .on_navigation(allowed_navigation)
        // 下载一律拒绝（DSH 访客策略），告诉玩家一声。
        .on_download(move |_, event| {
            if let tauri::webview::DownloadEvent::Requested { url, .. } = event {
                let file = url
                    .path_segments()
                    .and_then(|mut s| s.next_back())
                    .filter(|s| !s.is_empty())
                    .unwrap_or("文件")
                    .to_string();
                let app = download_app.clone();
                tauri::async_runtime::spawn(async move {
                    super::notice(
                        &app,
                        format!("网页想下载 {file}，Kotone 的浏览器现在不下载文件"),
                    );
                });
            }
            false
        })
        // 新窗口请求（window.open、target=_blank）在新标签页打开（照 DSH：http(s) 才开）；从她的标签页开出来的，
        // 新标签页也是她的。
        .on_new_window(move |url, _features| {
            if allowed_navigation(&url) && url.as_str() != "about:blank" {
                super::open_from_page_later(new_window_app.clone(), opener.clone(), url);
            }
            tauri::webview::NewWindowResponse::Deny
        })
        .on_page_load(move |_, payload| {
            let started = matches!(payload.event(), tauri::webview::PageLoadEvent::Started);
            let url = payload.url().to_string();
            let app = load_app.clone();
            let tab = loading_tab.clone();
            // 不在回调里发事件：交给 tokio。
            tauri::async_runtime::spawn(async move {
                super::tab_loaded(&app, &tab, started, &url);
            });
        });
    let (position, size) = placement(Bounds::default());
    let webview =
        tauri::async_runtime::spawn_blocking(move || window.add_child(builder, position, size))
            .await
            .map_err(|e| format!("打开浏览器失败：{e}"))?
            .map_err(|e| format!("打开浏览器失败：{e}"))?;
    let _ = webview.hide();
    // add_child 只是把“创建”交给主线程就返回，真正建失败只写进日志（tauri-runtime-wry）。
    // 用一次调试协议调用确认它真的建好了；建不出来就关掉、如实告诉玩家和后台任务。
    let ready = tokio::time::timeout(
        Duration::from_secs(8),
        cdp::call(&webview, "DOM.enable", serde_json::json!({})),
    )
    .await;
    if !matches!(ready, Ok(Ok(_))) {
        let reason = match ready {
            Ok(Err(e)) => e,
            _ => "8 秒没有回应".to_string(),
        };
        log::log(&format!("browser: 浏览器视图没有建起来：{reason}"));
        let _ = webview.close();
        update(app, |inner| {
            inner.error = Some("浏览器没能打开，重启 Kotone 后再试".into());
        });
        return Err(format!(
            "浏览器没能打开（{reason}）。这是 Kotone 的问题，不是网站的问题：不要换网站重试，直接告诉玩家，\
             建议重启 Kotone 后再试，或者把日志发给开发者。"
        ));
    }
    install_hooks(app, id, &webview);
    Ok(webview)
}

/// 访客策略（拒绝权限请求、HTTP 认证、原生对话框、拉起本机程序，照 DSH 访客策略）和页面事件
/// （标题、网址、前进后退、页面全屏）。
fn install_hooks(app: &AppHandle, id: &str, webview: &Webview) {
    let app = app.clone();
    let tab = id.to_string();
    let _ = webview.with_webview(move |platform| {
        #[cfg(windows)]
        unsafe {
            use webview2_com::{
                BasicAuthenticationRequestedEventHandler,
                ContainsFullScreenElementChangedEventHandler, DocumentTitleChangedEventHandler,
                HistoryChangedEventHandler, Microsoft::Web::WebView2::Win32::*,
                PermissionRequestedEventHandler, ScriptDialogOpeningEventHandler,
                SourceChangedEventHandler,
            };
            use windows::core::Interface;
            let registered = (|| -> windows::core::Result<()> {
                let core = platform.controller().CoreWebView2()?;
                let deny = PermissionRequestedEventHandler::create(Box::new(|_, args| {
                    if let Some(args) = args {
                        let mut kind = COREWEBVIEW2_PERMISSION_KIND::default();
                        let _ = args.PermissionKind(&mut kind);
                        args.SetState(COREWEBVIEW2_PERMISSION_STATE_DENY)?;
                        kotone_core::log::log(&format!(
                            "browser: 网页请求权限（WebView2 权限种类 {}），已拒绝",
                            kind.0
                        ));
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

                // ---- 页面事件：只读属性，交给 tokio ----
                let (title_app, title_tab) = (app.clone(), tab.clone());
                let title = DocumentTitleChangedEventHandler::create(Box::new(move |sender, _| {
                    if let Some(core) = sender {
                        let mut value = windows::core::PWSTR::null();
                        if core.DocumentTitle(&mut value).is_ok() {
                            let title = webview2_com::take_pwstr(value);
                            let (app, tab) = (title_app.clone(), title_tab.clone());
                            tauri::async_runtime::spawn(async move {
                                super::tab_title(&app, &tab, title);
                            });
                        }
                    }
                    Ok(())
                }));
                core.add_DocumentTitleChanged(&title, &mut token)?;
                let (source_app, source_tab) = (app.clone(), tab.clone());
                let source = SourceChangedEventHandler::create(Box::new(move |sender, _| {
                    if let Some(core) = sender {
                        let mut value = windows::core::PWSTR::null();
                        if core.Source(&mut value).is_ok() {
                            let url = webview2_com::take_pwstr(value);
                            let (app, tab) = (source_app.clone(), source_tab.clone());
                            tauri::async_runtime::spawn(async move {
                                super::tab_source(&app, &tab, &url);
                            });
                        }
                    }
                    Ok(())
                }));
                core.add_SourceChanged(&source, &mut token)?;
                let (history_app, history_tab) = (app.clone(), tab.clone());
                let history = HistoryChangedEventHandler::create(Box::new(move |sender, _| {
                    if let Some(core) = sender {
                        let (mut back, mut forward) = (
                            windows::core::BOOL::default(),
                            windows::core::BOOL::default(),
                        );
                        let _ = core.CanGoBack(&mut back);
                        let _ = core.CanGoForward(&mut forward);
                        let (back, forward) = (back.as_bool(), forward.as_bool());
                        let (app, tab) = (history_app.clone(), history_tab.clone());
                        tauri::async_runtime::spawn(async move {
                            super::tab_history(&app, &tab, back, forward);
                        });
                    }
                    Ok(())
                }));
                core.add_HistoryChanged(&history, &mut token)?;
                let (full_app, full_tab) = (app.clone(), tab.clone());
                let fullscreen = ContainsFullScreenElementChangedEventHandler::create(Box::new(
                    move |sender, _| {
                        if let Some(core) = sender {
                            let mut value = windows::core::BOOL::default();
                            if core.ContainsFullScreenElement(&mut value).is_ok() {
                                let on = value.as_bool();
                                let (app, tab) = (full_app.clone(), full_tab.clone());
                                tauri::async_runtime::spawn(async move {
                                    super::tab_fullscreen(&app, &tab, on);
                                });
                            }
                        }
                        Ok(())
                    },
                ));
                core.add_ContainsFullScreenElementChanged(&fullscreen, &mut token)?;

                // 事件装上之前的标题、网址、前进后退补读一次（以防万一）。
                let mut value = windows::core::PWSTR::null();
                let title = if core.DocumentTitle(&mut value).is_ok() {
                    Some(webview2_com::take_pwstr(value))
                } else {
                    None
                };
                let mut value = windows::core::PWSTR::null();
                let source = if core.Source(&mut value).is_ok() {
                    Some(webview2_com::take_pwstr(value))
                } else {
                    None
                };
                let (mut back, mut forward) = (
                    windows::core::BOOL::default(),
                    windows::core::BOOL::default(),
                );
                let _ = core.CanGoBack(&mut back);
                let _ = core.CanGoForward(&mut forward);
                let (back, forward) = (back.as_bool(), forward.as_bool());
                let (app, tab) = (app.clone(), tab.clone());
                tauri::async_runtime::spawn(async move {
                    if let Some(url) = source {
                        super::tab_source(&app, &tab, &url);
                    }
                    if let Some(title) = title {
                        super::tab_title(&app, &tab, title);
                    }
                    super::tab_history(&app, &tab, back, forward);
                });
                Ok(())
            })();
            if let Err(e) = registered {
                kotone_core::log::log(&format!("browser: 访客策略 / 页面事件没装上：{e}"));
            }
        }
        #[cfg(not(windows))]
        let _ = (platform, app, tab);
    });
}

/// 前进 / 后退 / 停止（WebView2 自带的，跨域、单页应用都准）。
pub(super) fn history(webview: &Webview, action: &str) -> Result<(), String> {
    let action = action.to_string();
    webview
        .with_webview(move |platform| {
            #[cfg(windows)]
            unsafe {
                let done = (|| -> windows::core::Result<()> {
                    let core = platform.controller().CoreWebView2()?;
                    match action.as_str() {
                        "back" => core.GoBack(),
                        "forward" => core.GoForward(),
                        _ => core.Stop(),
                    }
                })();
                if let Err(e) = done {
                    kotone_core::log::log(&format!("browser: {action} 没做成：{e}"));
                }
            }
            #[cfg(not(windows))]
            let _ = (platform, action);
        })
        .map_err(|e| format!("浏览器不可用：{e}"))
}
