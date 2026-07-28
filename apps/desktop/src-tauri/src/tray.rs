//! 系统托盘：左键唤起主窗口；右键菜单「打开主页面 / 启动引擎 / 退出」
//! （docs/development.md §3.6）
//!
//! 「启动引擎」为动态文案：引擎运行中显示「停止引擎」，由 runtime.rs 在
//! 状态推送时（snapshot_and_emit）同步更新；启动/停止完成都会经过该推送点。

use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, Runtime,
};

/// 托盘状态：toggle 引擎菜单项句柄，供 runtime 状态推送时动态改文案
pub struct TrayState {
    pub toggle_item: MenuItem<tauri::Wry>,
}

/// 按运行时相位更新「启动引擎 / 停止引擎」文案。
/// 任何相位变化（启动/停止完成、失败回滚）都应调用，保证文案始终正确。
pub fn sync_toggle_label(app: &AppHandle, running: bool) {
    if let Some(tray) = app.try_state::<TrayState>() {
        let text = if running { "停止引擎" } else { "启动引擎" };
        let _ = tray.toggle_item.set_text(text);
    }
}

/// 构建托盘图标与菜单：左键释放唤起主窗口，右键菜单切换引擎或退出
pub fn setup_tray(app: &AppHandle<tauri::Wry>) -> tauri::Result<()> {
    let open_main = MenuItem::with_id(app, "open-main", "打开主页面", true, None::<&str>)?;
    let toggle_engine = MenuItem::with_id(app, "toggle-engine", "启动引擎", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open_main, &toggle_engine, &quit])?;

    // 存句柄供 runtime 状态推送时更新文案
    app.manage(TrayState {
        toggle_item: toggle_engine.clone(),
    });

    let mut builder = TrayIconBuilder::with_id("kotone-tray")
        .tooltip("Kotone 琴音")
        .menu(&menu)
        // 左键不再弹菜单：释放时显示并聚焦主窗口
        // （回调首参是 &TrayIcon 而非 AppHandle，经 app_handle() 取回）
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_window(tray.app_handle(), "main");
            }
        })
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open-main" => show_window(app, "main"),
            "toggle-engine" => {
                // 按当前相位异步启动/停止（避免阻塞菜单事件循环）；
                // 文案更新统一走 runtime 状态推送（snapshot_and_emit）
                let running = app
                    .try_state::<crate::runtime::RuntimeManager>()
                    .map(|rt| rt.phase() == kotone_core::runtime::RuntimePhase::Running)
                    .unwrap_or(false);
                let handle = app.clone();
                tauri::async_runtime::spawn(async move {
                    let result = if running {
                        crate::runtime::stop(&handle).await
                    } else {
                        crate::runtime::start(&handle).await
                    };
                    if let Err(e) = result {
                        kotone_core::log::log(&format!("tray toggle engine failed: {e}"));
                    }
                });
            }
            "quit" => app.exit(0),
            _ => {}
        });

    // 图标缺失时仍保证应用可用（开发期 icons 可能尚未生成）
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }

    let _tray = builder.build(app)?;
    Ok(())
}

/// 显示并聚焦指定窗口（窗口初始 invisible，由托盘/热键唤起）。
/// overlay 走 show_window_no_focus（原始 SW_SHOWNA，与运行时的显隐机制一致，
/// 不混用 tao 可见性缓存）；main 走 Tauri show + 聚焦。
fn show_window<R: Runtime>(app: &AppHandle<R>, label: &str) {
    if let Some(win) = app.get_webview_window(label) {
        if label == "overlay" {
            crate::show_window_no_focus(&win);
            return;
        }
        let _ = win.show();
        let _ = win.unminimize();
        let _ = win.set_focus();
    }
}
