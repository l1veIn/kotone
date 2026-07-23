//! 全局热键：注册/注销，hold / toggle 两种触发模式（docs/development.md §3.6、§5.1）
//! 依赖 tauri-plugin-global-shortcut。
//!
//! - hold：插件回调按 Pressed/Released 区分，按下开始、松开结束；
//! - toggle：只响应 Pressed，按一下开始、再按结束（转写/发送中再按 = 取消）；
//! - 取消路径：会话激活期间（Listening）临时全局注册 Escape，会话结束即注销，
//!   避免长期劫持游戏的 Esc 键；
//! - 运行时改键/改模式：unregister + register 重注册。

use std::sync::Mutex;

use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use crate::SharedState;

/// 热键触发模式（用户在设置中选择，默认 toggle 引导时确认）
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HotkeyMode {
    /// 按住说话，松手结束
    Hold,
    /// 按一下开始，再按一下结束
    Toggle,
}

/// 热键管理器：记录当前注册内容，支持运行时改键/改模式
pub struct HotkeyManager {
    current: Mutex<Option<Shortcut>>,
    /// 会话激活期间临时注册的 Esc 取消键
    cancel: Mutex<Option<Shortcut>>,
}

impl Default for HotkeyManager {
    fn default() -> Self {
        Self::new()
    }
}

impl HotkeyManager {
    pub fn new() -> Self {
        Self {
            current: Mutex::new(None),
            cancel: Mutex::new(None),
        }
    }

    /// 注册全局热键（已注册则先注销，实现运行时改键/改模式）
    pub fn register(&self, app: &AppHandle, key: &str, mode: HotkeyMode) -> Result<(), String> {
        let _ = self.unregister(app);
        let shortcut: Shortcut = key
            .parse()
            .map_err(|e| format!("无法解析热键「{key}」: {e}"))?;

        app.global_shortcut()
            .on_shortcut(shortcut.clone(), move |app, _sc, event| {
                let orch = app.state::<SharedState>().orchestrator.clone();
                match mode {
                    HotkeyMode::Hold => {
                        let pressed = event.state() == ShortcutState::Pressed;
                        tauri::async_runtime::spawn(async move {
                            orch.on_hotkey_hold(pressed).await;
                        });
                    }
                    HotkeyMode::Toggle => {
                        if event.state() == ShortcutState::Pressed {
                            tauri::async_runtime::spawn(async move {
                                orch.on_hotkey_toggle().await;
                            });
                        }
                    }
                }
            })
            .map_err(|e| format!("注册热键「{key}」失败: {e}（键位可能被其他程序占用）"))?;

        *self.current.lock().unwrap() = Some(shortcut);
        Ok(())
    }

    /// 注销当前热键
    pub fn unregister(&self, app: &AppHandle) -> Result<(), String> {
        if let Some(sc) = self.current.lock().unwrap().take() {
            app.global_shortcut()
                .unregister(sc)
                .map_err(|e| format!("注销热键失败: {e}"))?;
        }
        Ok(())
    }

    /// 会话激活期间临时注册 Esc 全局取消键；会话结束注销。
    /// 注册失败不致命（仍可通过悬浮窗/热键取消），仅记录日志。
    pub fn set_cancel_enabled(&self, app: &AppHandle, enabled: bool) {
        let mut guard = self.cancel.lock().unwrap();
        match (enabled, guard.is_some()) {
            (true, false) => {
                let shortcut: Shortcut = match "Escape".parse() {
                    Ok(s) => s,
                    Err(_) => return,
                };
                let registered = app.global_shortcut().on_shortcut(
                    shortcut.clone(),
                    |app, _sc, event| {
                        if event.state() == ShortcutState::Pressed {
                            let orch = app.state::<SharedState>().orchestrator.clone();
                            tauri::async_runtime::spawn(async move {
                                orch.cancel().await;
                            });
                        }
                    },
                );
                match registered {
                    Ok(()) => *guard = Some(shortcut),
                    Err(e) => eprintln!("[kotone hotkey] 注册 Esc 取消键失败（不致命）: {e}"),
                }
            }
            (false, true) => {
                if let Some(sc) = guard.take() {
                    if let Err(e) = app.global_shortcut().unregister(sc) {
                        eprintln!("[kotone hotkey] 注销 Esc 取消键失败: {e}");
                    }
                }
            }
            _ => {}
        }
    }

    /// 当前注册的热键（调试用）
    #[allow(dead_code)]
    pub fn current_key(&self) -> Option<String> {
        self.current.lock().unwrap().as_ref().map(|s| s.to_string())
    }
}

/// 检测与常见游戏键位的冲突（首次启动引导用）。
/// TODO(后续)：与常见游戏默认键位表对比 + 尝试注册探测占用情况。当前仅做静态提示。
#[allow(dead_code)] // 供首次启动引导 UI 调用（前端子代理接入）
pub fn detect_conflicts(key: &str) -> Vec<String> {
    let common_game_keys = [
        "F1", "F2", "F3", "F4", "F5", "F6", "F7", "F9", "F10", "F11", "F12", "Tab", "Space",
        "Enter", "Shift", "Control", "Alt",
    ];
    if common_game_keys.iter().any(|k| k.eq_ignore_ascii_case(key)) {
        vec![format!("「{key}」是常见游戏/系统键位，可能与游戏内操作冲突")]
    } else {
        Vec::new()
    }
}
