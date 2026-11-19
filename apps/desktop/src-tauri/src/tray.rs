//! 系统托盘：左键唤起主窗口；右键菜单（ADR 028）：
//! 打开主页面 ·〔启动 / 停止本地语音识别〕· 停下 · 退出。
//!
//! 「启动 / 停止本地语音识别」只在用本地识别时出现（在线识别打开就能用，没有“启动”），
//! 文案随相位变。runtime.rs 每次推送状态（snapshot_and_emit）都调用 [`sync`]：
//! 本地 / 在线切换时重建菜单，相位变化时只改文案。

use std::sync::Mutex;

use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, Runtime,
};

const TRAY_ID: &str = "kotone-tray";

/// 托盘状态：当前菜单是否带「本地语音识别」一项，以及这一项的句柄（改文案用）。
pub struct TrayState {
    local: Mutex<Option<bool>>,
    toggle_item: Mutex<Option<MenuItem<tauri::Wry>>>,
}

/// 「启动 / 停止本地语音识别」的文案与是否可点。
fn toggle_label(phase: &str) -> (&'static str, bool) {
    match phase {
        "running" => ("停止本地语音识别", true),
        "starting" => ("本地语音识别准备中…", false),
        "stopping" => ("正在停止本地语音识别…", false),
        _ => ("启动本地语音识别", true),
    }
}

fn build_menu(
    app: &AppHandle,
    local_phase: Option<&str>,
) -> tauri::Result<(Menu<tauri::Wry>, Option<MenuItem<tauri::Wry>>)> {
    let open_main = MenuItem::with_id(app, "open-main", "打开主页面", true, None::<&str>)?;
    let toggle = match local_phase {
        Some(phase) => {
            let (text, enabled) = toggle_label(phase);
            Some(MenuItem::with_id(
                app,
                "toggle-local-asr",
                text,
                enabled,
                None::<&str>,
            )?)
        }
        None => None,
    };
    let stop_all = MenuItem::with_id(app, "stop-all", "停下", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = match &toggle {
        Some(toggle) => Menu::with_items(app, &[&open_main, toggle, &stop_all, &quit])?,
        None => Menu::with_items(app, &[&open_main, &stop_all, &quit])?,
    };
    Ok((menu, toggle))
}

/// 跟着运行时状态更新托盘菜单：本地 / 在线切换时重建，相位变化时改文案。
/// 菜单操作会切到主线程同步执行：调用时不能持有任何锁（主线程也可能正在推送状态）。
pub fn sync(app: &AppHandle, status: &crate::runtime::RuntimeStatus) {
    let Some(tray_state) = app.try_state::<TrayState>() else {
        return;
    };
    let rebuild = {
        let mut local = tray_state.local.lock().unwrap();
        let changed = *local != Some(status.asr_local);
        if changed {
            *local = Some(status.asr_local);
        }
        changed
    };
    if rebuild {
        let phase = status.asr_local.then_some(status.phase.as_str());
        let applied = build_menu(app, phase).and_then(|(menu, toggle)| {
            if let Some(tray) = app.tray_by_id(TRAY_ID) {
                tray.set_menu(Some(menu))?;
            }
            Ok(toggle)
        });
        match applied {
            Ok(toggle) => *tray_state.toggle_item.lock().unwrap() = toggle,
            Err(e) => {
                kotone_core::log::log(&format!("托盘菜单更新失败: {e}"));
                // 下次推送再试
                *tray_state.local.lock().unwrap() = None;
            }
        }
        return;
    }
    let item = tray_state.toggle_item.lock().unwrap().clone();
    if let Some(item) = item {
        let (text, enabled) = toggle_label(&status.phase);
        let _ = item.set_text(text);
        let _ = item.set_enabled(enabled);
    }
}

/// 构建托盘图标与菜单：左键释放唤起主窗口，右键菜单见模块说明。
pub fn setup_tray(app: &AppHandle<tauri::Wry>) -> tauri::Result<()> {
    // 先建不带本地识别项的菜单；setup 末尾的 refresh 按实际识别方式补上。
    let (menu, _) = build_menu(app, None)?;
    app.manage(TrayState {
        local: Mutex::new(Some(false)),
        toggle_item: Mutex::new(None),
    });

    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
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
            "stop-all" => {
                crate::stop::stop_everything(app, "托盘「停下」");
            }
            "toggle-local-asr" => {
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
                        kotone_core::log::log(&format!("tray toggle local asr failed: {e}"));
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

/// 按当前识别方式补上 / 去掉「本地语音识别」一项（setup 末尾调用；之后跟着状态推送走）。
pub fn refresh(app: &AppHandle) {
    let (Some(state), Some(rt)) = (
        app.try_state::<crate::SharedState>(),
        app.try_state::<crate::runtime::RuntimeManager>(),
    ) else {
        return;
    };
    let status = {
        let settings = state.settings.read().unwrap();
        rt.status(&settings, &state.engines, None)
    };
    sync(app, &status);
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

#[cfg(test)]
mod tests {
    use super::toggle_label;

    #[test]
    fn local_recognition_item_follows_phase() {
        assert_eq!(toggle_label("stopped"), ("启动本地语音识别", true));
        assert_eq!(toggle_label("running"), ("停止本地语音识别", true));
        assert!(!toggle_label("starting").1);
        assert!(!toggle_label("stopping").1);
    }
}
