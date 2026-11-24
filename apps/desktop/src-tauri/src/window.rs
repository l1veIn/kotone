//! 窗口小工具：不抢焦点地显示、隐藏，关掉 WebView 的浏览器行为。

use super::*;

/// 显示窗口但不抢焦点（焦点必须留在游戏/目标窗口，否则注入会打错窗口）。
/// Windows 上用 SW_SHOWNA；其他平台回退普通 show。
///
/// 注意：本函数绕开 tao 的 window_flags 缓存直接操作 Win32 状态，因此隐藏必须
/// 走对称的 `hide_window`（见下），不能混用 Tauri 的 hide()/show()。
#[cfg(windows)]
pub(crate) fn show_window_no_focus<R: tauri::Runtime>(win: &tauri::WebviewWindow<R>) {
    overlay::set_visible(win, true);
}

/// 隐藏窗口（与 show_window_no_focus 对称的原始 Win32 路径）。
///
/// 为什么不能用 Tauri 的 hide()：窗口经 SW_SHOWNA 显示后，tao 的 window_flags
/// 缓存仍停留在「不可见」（创建时 visible:false）；tao 的 set_visible(false)
/// 走 apply_diff，对缓存新旧值做 diff——diff 为空则提前返回，根本到不了
/// ShowWindow(SW_HIDE)（tao 0.35 window_state.rs）。这正是「停止后悬浮窗
/// 不隐藏」的根因：显隐机制不对称，与 runtime 编排无关。
#[cfg(windows)]
pub(crate) fn hide_window<R: tauri::Runtime>(win: &tauri::WebviewWindow<R>) {
    overlay::set_visible(win, false);
}

/// 玩家自己要打开窗口时（正常启动、托盘、第二个实例、点消息提示）：显示、取消最小化、拿到前面，
/// 再把键盘焦点交给窗口里的页面。
///
/// 设置窗口建的时候不要焦点（`focus: false`，见 [`create_startup_windows`]）。桌面端开了 Tauri 的
/// `unstable`（多 WebView），页面是窗口的子 WebView：激活窗口不会自己把键盘焦点交给它，
/// 要显式 `Webview::set_focus`，不然打开后页面收不到键，得先点一下。
pub(crate) fn show_and_focus<R: tauri::Runtime>(win: &tauri::WebviewWindow<R>) {
    let _ = win.show();
    let _ = win.unminimize();
    let _ = win.set_focus();
    let _ = AsRef::<tauri::Webview<R>>::as_ref(win).set_focus();
}

#[cfg(not(windows))]
fn show_window_no_focus<R: tauri::Runtime>(win: &tauri::WebviewWindow<R>) {
    let _ = win.show();
}

#[cfg(not(windows))]
fn hide_window<R: tauri::Runtime>(win: &tauri::WebviewWindow<R>) {
    let _ = win.hide();
}

/// 把 WebView2 收敛为桌面应用壳：关闭右键浏览器菜单、开发者工具和浏览器快捷键。
///
/// 前端还有 capture 阶段的快捷键拦截作为跨平台兜底；Windows 侧必须从
/// WebView2 Settings 关闭 accelerator keys，才能在 DOM 收到事件前拦住
/// Ctrl+F、F5、Ctrl+Shift+R 等浏览器行为。
#[cfg(windows)]
pub(crate) fn harden_webview<R: tauri::Runtime>(win: &tauri::WebviewWindow<R>) {
    let label = win.label().to_string();
    let _ = win.with_webview(move |webview| unsafe {
        use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Settings3;
        use windows::core::Interface;

        let result = (|| -> windows::core::Result<()> {
            let core = webview.controller().CoreWebView2()?;
            let settings = core.Settings()?;
            if cfg!(debug_assertions) {
                // 开发构建保留右键菜单与开发者工具，便于查看报错
                settings.SetAreDefaultContextMenusEnabled(true)?;
                settings.SetAreDevToolsEnabled(true)?;
            } else {
                settings.SetAreDefaultContextMenusEnabled(false)?;
                settings.SetAreDevToolsEnabled(false)?;
            }
            let settings3: ICoreWebView2Settings3 = settings.cast()?;
            settings3.SetAreBrowserAcceleratorKeysEnabled(false)?;
            Ok(())
        })();
        if let Err(error) = result {
            log::log(&format!("webview hardening failed ({label}): {error}"));
        }
    });
}

#[cfg(not(windows))]
fn harden_webview<R: tauri::Runtime>(_win: &tauri::WebviewWindow<R>) {}
