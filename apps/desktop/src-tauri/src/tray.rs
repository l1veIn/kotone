//! 系统托盘：菜单「显示悬浮条 / 设置 / 退出」（docs/development.md §3.6）

use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    AppHandle, Manager, Runtime,
};

/// 构建托盘图标与菜单，点击行为：唤起对应窗口或退出
pub fn setup_tray<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let show_overlay = MenuItem::with_id(app, "show-overlay", "显示悬浮条", true, None::<&str>)?;
    let show_settings = MenuItem::with_id(app, "show-settings", "设置", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show_overlay, &show_settings, &quit])?;

    let mut builder = TrayIconBuilder::with_id("kotone-tray")
        .tooltip("Kotone 琴音")
        .menu(&menu)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show-overlay" => show_window(app, "overlay"),
            "show-settings" => show_window(app, "main"),
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
