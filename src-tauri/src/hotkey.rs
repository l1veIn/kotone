//! 全局热键：注册/注销，hold / toggle 两种触发模式（docs/development.md §3.6、§5.1）
//!
//! 双后端架构（对上层透明，register/unregister/status 签名不变）：
//! - **LL 钩子后端**（hotkey_ll.rs，Windows 默认）：WH_KEYBOARD_LL 低级键盘钩子，
//!   解决 RegisterHotKey 在 LOL 等游戏前台不投递事件的问题（实测日志实证）；
//! - **RegisterHotKey 后端**（tauri-plugin-global-shortcut）：LL 钩子安装失败时回退，
//!   以及非 Windows 平台的唯一实现。
//!
//! - hold：按下开始、松开结束；toggle：按一下开始、再按结束（转写/发送中再按 = 取消）；
//! - Esc 取消：LL 钩子后端由钩子内建处理（会话激活时吞 Esc 并 cancel）；
//!   RegisterHotKey 后端在会话激活期间临时注册 Escape，会话结束即注销；
//! - 运行时改键/改模式/改后端：unregister + register 重注册。

use std::sync::Mutex;

use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use crate::settings::HotkeyBackend;
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

/// 当前生效的热键后端
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveBackend {
    None,
    /// WH_KEYBOARD_LL 低级键盘钩子
    LlHook,
    /// RegisterHotKey（tauri-plugin-global-shortcut）
    Plugin,
}

impl ActiveBackend {
    pub fn label(&self) -> &'static str {
        match self {
            ActiveBackend::None => "none",
            ActiveBackend::LlHook => "llhook",
            ActiveBackend::Plugin => "register",
        }
    }
}

/// 热键注册状态（设置页展示用）：注册失败时暴露错误原因与当前后端
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HotkeyStatus {
    /// 当前是否处于已注册状态
    pub registered: bool,
    /// 当前注册的热键（未注册为 null）
    pub key: Option<String>,
    /// 最近一次注册失败信息（成功后清空）
    pub error: Option<String>,
    /// 当前生效后端：llhook / register / none
    pub backend: String,
}

/// 热键管理器：记录当前注册内容与生效后端，支持运行时改键/改模式/改后端
pub struct HotkeyManager {
    /// RegisterHotKey 后端：当前注册的主热键
    current: Mutex<Option<Shortcut>>,
    /// RegisterHotKey 后端：会话激活期间临时注册的 Esc 取消键
    cancel: Mutex<Option<Shortcut>>,
    /// 最近一次注册失败信息（设置页提示「可能被其他程序/实例占用」）
    last_error: Mutex<Option<String>>,
    /// 当前生效后端
    backend: Mutex<ActiveBackend>,
    /// 当前注册的热键名（两个后端通用；plugin 的 Shortcut 句柄仅 RegisterHotKey 路径有）
    registered_key: Mutex<Option<String>>,
    /// LL 钩子后端（Windows）
    #[cfg(windows)]
    llhook: crate::hotkey_ll::LlHook,
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
            last_error: Mutex::new(None),
            backend: Mutex::new(ActiveBackend::None),
            registered_key: Mutex::new(None),
            #[cfg(windows)]
            llhook: crate::hotkey_ll::LlHook::new(),
        }
    }

    /// 注册全局热键（已注册则先注销，实现运行时改键/改模式/改后端）。
    /// Windows 上按配置优先 LL 钩子，安装失败回退 RegisterHotKey 并记录日志。
    pub fn register(&self, app: &AppHandle, key: &str, mode: HotkeyMode) -> Result<(), String> {
        let _ = self.unregister(app);
        let pref = backend_preference(app);

        #[cfg(windows)]
        if pref != HotkeyBackend::Register {
            match self.llhook.register(app, key, mode) {
                Ok(()) => {
                    crate::log::log(&format!("hotkey backend=llhook registered ok: {key} ({mode:?})"));
                    *self.backend.lock().unwrap() = ActiveBackend::LlHook;
                    *self.registered_key.lock().unwrap() = Some(key.to_string());
                    *self.last_error.lock().unwrap() = None;
                    return Ok(());
                }
                Err(e) => {
                    crate::log::log(&format!(
                        "llhook 后端不可用，回退 RegisterHotKey: {e}"
                    ));
                }
            }
        }

        self.register_plugin(app, key, mode)
    }

    /// RegisterHotKey 后端注册路径（tauri-plugin-global-shortcut）
    fn register_plugin(&self, app: &AppHandle, key: &str, mode: HotkeyMode) -> Result<(), String> {
        let shortcut: Shortcut = key
            .parse()
            .map_err(|e| format!("无法解析热键「{key}」: {e}"))?;

        let registered = app.global_shortcut().on_shortcut(shortcut.clone(), move |app, _sc, event| {
            crate::log::log(&format!("hotkey fired: mode={mode:?} state={:?}", event.state()));
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
        });

        match registered {
            Ok(()) => {
                crate::log::log(&format!("hotkey backend=register registered ok: {key} ({mode:?})"));
                *self.current.lock().unwrap() = Some(shortcut);
                *self.backend.lock().unwrap() = ActiveBackend::Plugin;
                *self.registered_key.lock().unwrap() = Some(key.to_string());
                *self.last_error.lock().unwrap() = None;
                Ok(())
            }
            Err(e) => {
                let msg = format!("注册热键「{key}」失败: {e}（键位可能被其他程序或其他 Kotone 实例占用）");
                crate::log::log(&format!("hotkey register FAILED: {msg}"));
                *self.last_error.lock().unwrap() = Some(msg.clone());
                Err(msg)
            }
        }
    }

    /// 注销当前热键（两个后端都停）
    pub fn unregister(&self, app: &AppHandle) -> Result<(), String> {
        #[cfg(windows)]
        self.llhook.unregister();
        if let Some(sc) = self.current.lock().unwrap().take() {
            app.global_shortcut()
                .unregister(sc)
                .map_err(|e| format!("注销热键失败: {e}"))?;
        }
        *self.backend.lock().unwrap() = ActiveBackend::None;
        *self.registered_key.lock().unwrap() = None;
        Ok(())
    }

    /// 会话激活期间的 Esc 取消：
    /// - LL 钩子后端：钩子内建处理（会话激活时吞 Esc → cancel），无需临时注册；
    /// - RegisterHotKey 后端：临时全局注册 Escape，会话结束注销。
    /// 注册失败不致命（仍可通过悬浮窗/热键取消），仅记录日志。
    pub fn set_cancel_enabled(&self, app: &AppHandle, enabled: bool) {
        #[cfg(windows)]
        if *self.backend.lock().unwrap() == ActiveBackend::LlHook {
            self.llhook.set_session_active(enabled);
            return;
        }

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
        self.registered_key.lock().unwrap().clone()
    }

    /// 注册状态快照（设置页热键分区展示注册失败原因与当前后端）
    pub fn status(&self) -> HotkeyStatus {
        let backend = *self.backend.lock().unwrap();
        let key = self.registered_key.lock().unwrap().clone();
        let error = self.last_error.lock().unwrap().clone();
        HotkeyStatus {
            registered: backend != ActiveBackend::None,
            key,
            error,
            backend: backend.label().to_string(),
        }
    }
}

/// 从共享设置读取后端偏好；设置未就绪（启动早期）按 auto 处理
fn backend_preference(app: &AppHandle) -> HotkeyBackend {
    app.try_state::<SharedState>()
        .map(|s| s.settings.read().unwrap().hotkey_backend)
        .unwrap_or_default()
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
