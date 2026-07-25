//! 全局热键（Tauri 壳）：后端选择/回退/状态暴露（docs/development.md §3.6、§5.1）
//!
//! 端口在 core（`HotkeySource`）；两种实现：
//! - **LL 钩子**（kotone-platform-windows，Windows 默认）：WH_KEYBOARD_LL，
//!   解决 RegisterHotKey 在 LOL 等游戏前台不投递事件的问题；
//! - **RegisterHotKey**（tauri-plugin-global-shortcut，本文件 `PluginHotkeySource`）：
//!   依赖 AppHandle 故留在壳内；LL 钩子安装失败时回退，也是非 Windows 唯一实现。
//!
//! 对上层（TauriEmitter / IPC）签名不变：register/unregister/set_cancel_enabled/status。

use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use kotone_core::hotkey::{HookEvent, HotkeySource};
use kotone_core::orchestrator::Orchestrator;
use kotone_core::settings::HotkeyBackend;

use crate::SharedState;

pub use kotone_core::hotkey::HotkeyMode;

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

/// RegisterHotKey 热键源（tauri-plugin-global-shortcut）：依赖 AppHandle，留在壳内。
/// Esc 取消走「会话激活期间临时注册 Escape」策略（LL 钩子后端则内建处理）。
pub struct PluginHotkeySource {
    app: AppHandle,
    orch: Arc<Orchestrator>,
    current: Mutex<Option<Shortcut>>,
    /// 会话激活期间临时注册的 Esc 取消键
    cancel: Mutex<Option<Shortcut>>,
}

impl PluginHotkeySource {
    pub fn new(app: &AppHandle, orch: Arc<Orchestrator>) -> Self {
        Self {
            app: app.clone(),
            orch,
            current: Mutex::new(None),
            cancel: Mutex::new(None),
        }
    }
}

impl HotkeySource for PluginHotkeySource {
    fn register(&self, key: &str, mode: HotkeyMode) -> Result<(), String> {
        self.unregister();
        let shortcut: Shortcut = key
            .parse()
            .map_err(|e| format!("无法解析热键「{key}」: {e}"))?;

        let orch = self.orch.clone();
        self.app
            .global_shortcut()
            .on_shortcut(shortcut.clone(), move |_app, _sc, event| {
                kotone_core::log::log(&format!(
                    "hotkey fired: mode={mode:?} state={:?}",
                    event.state()
                ));
                let orch = orch.clone();
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
            .map_err(|e| format!("注册热键「{key}」失败: {e}（键位可能被其他程序或其他 Kotone 实例占用）"))?;

        *self.current.lock().unwrap() = Some(shortcut);
        Ok(())
    }

    fn unregister(&self) {
        if let Some(sc) = self.current.lock().unwrap().take() {
            if let Err(e) = self.app.global_shortcut().unregister(sc) {
                kotone_core::log::log(&format!("注销热键失败: {e}"));
            }
        }
    }

    /// 会话激活期间临时注册 Esc 全局取消键；会话结束注销。
    /// 注册失败不致命（仍可通过悬浮窗/热键取消），仅记录日志。
    fn set_cancel_active(&self, active: bool) {
        let mut guard = self.cancel.lock().unwrap();
        match (active, guard.is_some()) {
            (true, false) => {
                let shortcut: Shortcut = match "Escape".parse() {
                    Ok(s) => s,
                    Err(_) => return,
                };
                let orch = self.orch.clone();
                let registered = self.app.global_shortcut().on_shortcut(
                    shortcut.clone(),
                    move |_app, _sc, event| {
                        if event.state() == ShortcutState::Pressed {
                            let orch = orch.clone();
                            tauri::async_runtime::spawn(async move {
                                orch.cancel().await;
                            });
                        }
                    },
                );
                match registered {
                    Ok(()) => *guard = Some(shortcut),
                    Err(e) => kotone_core::log::log(&format!("注册 Esc 取消键失败（不致命）: {e}")),
                }
            }
            (false, true) => {
                if let Some(sc) = guard.take() {
                    if let Err(e) = self.app.global_shortcut().unregister(sc) {
                        kotone_core::log::log(&format!("注销 Esc 取消键失败: {e}"));
                    }
                }
            }
            _ => {}
        }
    }
}

/// LL 钩子事件 → orchestrator（spawn 进 Tauri 的 tokio runtime）
#[cfg(windows)]
fn make_llhook_sink(
    orch: Arc<Orchestrator>,
) -> kotone_platform_windows::hotkey_ll::HookSink {
    Box::new(move |ev| {
        let orch = orch.clone();
        match ev {
            HookEvent::HoldPressed => {
                tauri::async_runtime::spawn(async move {
                    orch.on_hotkey_hold(true).await;
                });
            }
            HookEvent::HoldReleased => {
                tauri::async_runtime::spawn(async move {
                    orch.on_hotkey_hold(false).await;
                });
            }
            HookEvent::Toggle => {
                tauri::async_runtime::spawn(async move {
                    orch.on_hotkey_toggle().await;
                });
            }
            HookEvent::Cancel => {
                tauri::async_runtime::spawn(async move {
                    orch.cancel().await;
                });
            }
        }
    })
}

/// 热键管理器：后端选择/回退 + 状态暴露；对上层签名不变
pub struct HotkeyManager {
    /// RegisterHotKey 热键源（回退路径 + 非 Windows）
    plugin: PluginHotkeySource,
    /// LL 钩子热键源（Windows 默认）
    #[cfg(windows)]
    llhook: kotone_platform_windows::hotkey_ll::LlHookSource,
    /// 当前生效后端
    backend: Mutex<ActiveBackend>,
    /// 当前注册的热键名（两个后端通用）
    registered_key: Mutex<Option<String>>,
    /// 最近一次注册失败信息（设置页提示「可能被其他程序/实例占用」）
    last_error: Mutex<Option<String>>,
}

impl HotkeyManager {
    pub fn new(app: &AppHandle, orch: Arc<Orchestrator>) -> Self {
        Self {
            plugin: PluginHotkeySource::new(app, orch.clone()),
            #[cfg(windows)]
            llhook: kotone_platform_windows::hotkey_ll::LlHookSource::new(make_llhook_sink(orch)),
            backend: Mutex::new(ActiveBackend::None),
            registered_key: Mutex::new(None),
            last_error: Mutex::new(None),
        }
    }

    /// 注册全局热键（已注册则先注销，实现运行时改键/改模式/改后端）。
    /// Windows 上按配置优先 LL 钩子，安装失败回退 RegisterHotKey 并记录日志。
    pub fn register(&self, app: &AppHandle, key: &str, mode: HotkeyMode) -> Result<(), String> {
        self.unregister(app)?;
        let pref = backend_preference(app);

        #[cfg(windows)]
        if pref != HotkeyBackend::Register {
            match self.llhook.register(key, mode) {
                Ok(()) => {
                    kotone_core::log::log(&format!(
                        "hotkey backend=llhook registered ok: {key} ({mode:?})"
                    ));
                    *self.backend.lock().unwrap() = ActiveBackend::LlHook;
                    *self.registered_key.lock().unwrap() = Some(key.to_string());
                    *self.last_error.lock().unwrap() = None;
                    return Ok(());
                }
                Err(e) => {
                    kotone_core::log::log(&format!("llhook 后端不可用，回退 RegisterHotKey: {e}"));
                }
            }
        }

        match self.plugin.register(key, mode) {
            Ok(()) => {
                kotone_core::log::log(&format!(
                    "hotkey backend=register registered ok: {key} ({mode:?})"
                ));
                *self.backend.lock().unwrap() = ActiveBackend::Plugin;
                *self.registered_key.lock().unwrap() = Some(key.to_string());
                *self.last_error.lock().unwrap() = None;
                Ok(())
            }
            Err(msg) => {
                kotone_core::log::log(&format!("hotkey register FAILED: {msg}"));
                *self.last_error.lock().unwrap() = Some(msg.clone());
                Err(msg)
            }
        }
    }

    /// 注销当前热键（两个后端都停）
    pub fn unregister(&self, _app: &AppHandle) -> Result<(), String> {
        #[cfg(windows)]
        self.llhook.unregister();
        self.plugin.unregister();
        *self.backend.lock().unwrap() = ActiveBackend::None;
        *self.registered_key.lock().unwrap() = None;
        Ok(())
    }

    /// 会话激活期间的 Esc 取消：按当前生效后端路由
    pub fn set_cancel_enabled(&self, _app: &AppHandle, enabled: bool) {
        #[cfg(windows)]
        if *self.backend.lock().unwrap() == ActiveBackend::LlHook {
            self.llhook.set_cancel_active(enabled);
            return;
        }
        self.plugin.set_cancel_active(enabled);
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

    /// 开始热键捕获（设置页「点击录入」）：LL 钩子捕获下一个按键组合，
    /// 结果经 `kotone://hotkey-capture` 事件推送（{combo} / {cancelled} / {timeout}）。
    /// 捕获期间正常热键匹配暂停（matcher 捕获模式不吞键、不产生会话事件）。
    pub fn start_capture(&self, app: AppHandle) -> Result<(), String> {
        #[cfg(windows)]
        {
            use kotone_platform_windows::hotkey_ll::CaptureResult;
            let cb = Box::new(move |result: CaptureResult| {
                let payload = match result {
                    CaptureResult::Captured(spec) => {
                        serde_json::json!({ "combo": spec.combo_name() })
                    }
                    CaptureResult::Cancelled => serde_json::json!({ "cancelled": true }),
                    CaptureResult::Timeout => serde_json::json!({ "timeout": true }),
                };
                if let Err(e) = app.emit("kotone://hotkey-capture", payload) {
                    kotone_core::log::log(&format!("hotkey-capture 事件推送失败: {e}"));
                }
            });
            return self
                .llhook
                .capture_next(cb, std::time::Duration::from_secs(10));
        }
        #[cfg(not(windows))]
        Err("当前平台不支持热键录入捕获".into())
    }

    /// 取消进行中的热键捕获（设置页关闭/重新点击的兜底）
    pub fn cancel_capture(&self) {
        #[cfg(windows)]
        self.llhook.cancel_capture();
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
