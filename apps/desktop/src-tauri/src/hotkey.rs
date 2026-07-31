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
    /// 已生效的频道切换热键（ADR-008；未配置/未生效为 null）
    pub cycle_key: Option<String>,
    /// 频道切换热键最近一次失败信息（如与录制热键冲突；成功/未配置为 null）
    pub cycle_error: Option<String>,
}

/// 录入/启动前的输入环境自检结果。
///
/// `available=true` 表示 Win32 已接受探测输入，可以继续进入热键录入；
/// `hook_verified=false` 只表示合成事件未能闭环确认，随后仍由真实物理按键兜底验证，
/// 不能据此把正常环境误报为安全软件拦截。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InputEnvironmentCheck {
    pub available: bool,
    pub hook_verified: bool,
    pub observed: usize,
    pub expected: usize,
    pub detail: Option<String>,
}

/// RegisterHotKey 热键源（tauri-plugin-global-shortcut）：依赖 AppHandle，留在壳内。
/// Esc 取消走「会话激活期间临时注册 Escape」策略（LL 钩子后端则内建处理）。
pub struct PluginHotkeySource {
    app: AppHandle,
    orch: Arc<Orchestrator>,
    current: Mutex<Option<Shortcut>>,
    /// 会话激活期间临时注册的 Esc 取消键
    cancel: Mutex<Option<Shortcut>>,
    /// 频道切换键（ADR-008）
    cycle: Mutex<Option<Shortcut>>,
}

impl PluginHotkeySource {
    pub fn new(app: &AppHandle, orch: Arc<Orchestrator>) -> Self {
        Self {
            app: app.clone(),
            orch,
            current: Mutex::new(None),
            cancel: Mutex::new(None),
            cycle: Mutex::new(None),
        }
    }

    fn unregister_cycle(&self) {
        if let Some(sc) = self.cycle.lock().unwrap().take() {
            if let Err(e) = self.app.global_shortcut().unregister(sc) {
                kotone_core::log::log(&format!("注销频道切换热键失败: {e}"));
            }
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
            .map_err(|e| {
                format!("注册热键「{key}」失败: {e}（键位可能被其他程序或其他 Kotone 实例占用）")
            })?;

        *self.current.lock().unwrap() = Some(shortcut);
        Ok(())
    }

    fn unregister(&self) {
        if let Some(sc) = self.current.lock().unwrap().take() {
            if let Err(e) = self.app.global_shortcut().unregister(sc) {
                kotone_core::log::log(&format!("注销热键失败: {e}"));
            }
        }
        self.unregister_cycle();
    }

    /// 频道切换键（ADR-008）：注册第二个全局快捷键，按下即循环切换频道
    fn set_cycle_key(&self, key: Option<&str>) -> Result<(), String> {
        self.unregister_cycle();
        let Some(key) = key.filter(|k| !k.trim().is_empty()) else {
            return Ok(());
        };
        let shortcut: Shortcut = key
            .parse()
            .map_err(|e| format!("无法解析频道切换热键「{key}」: {e}"))?;
        let orch = self.orch.clone();
        self.app
            .global_shortcut()
            .on_shortcut(shortcut.clone(), move |_app, _sc, event| {
                if event.state() == ShortcutState::Pressed {
                    let orch = orch.clone();
                    tauri::async_runtime::spawn(async move {
                        orch.on_cycle_channel().await;
                    });
                }
            })
            .map_err(|e| format!("注册频道切换热键「{key}」失败: {e}（键位可能被其他程序占用）"))?;
        *self.cycle.lock().unwrap() = Some(shortcut);
        Ok(())
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
fn make_llhook_sink(orch: Arc<Orchestrator>) -> kotone_platform_windows::hotkey_ll::HookSink {
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
            HookEvent::CycleChannel => {
                tauri::async_runtime::spawn(async move {
                    orch.on_cycle_channel().await;
                });
            }
            // 诊断事件（修饰键失配）：consumer 已记日志，业务不处理
            HookEvent::MainKeyMissed { .. } => {}
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
    /// 已生效的频道切换热键（ADR-008）
    cycle_key: Mutex<Option<String>>,
    /// 频道切换热键最近一次失败信息（冲突/注册失败）
    cycle_error: Mutex<Option<String>>,
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
            cycle_key: Mutex::new(None),
            cycle_error: Mutex::new(None),
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
                    self.apply_cycle_key(app, key);
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
                self.apply_cycle_key(app, key);
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
        {
            self.llhook.unregister();
            let _ = self.llhook.set_cycle_key(None);
        }
        self.plugin.unregister();
        *self.backend.lock().unwrap() = ActiveBackend::None;
        *self.registered_key.lock().unwrap() = None;
        *self.cycle_key.lock().unwrap() = None;
        *self.cycle_error.lock().unwrap() = None;
        Ok(())
    }

    /// 注册频道切换热键（ADR-008）：主热键注册成功后按当前生效后端应用。
    /// 与录制热键同组合时拒绝注册并记入 cycle_error（设置页展示）。
    fn apply_cycle_key(&self, app: &AppHandle, main_key: &str) {
        let cycle = app
            .try_state::<SharedState>()
            .map(|s| s.settings.read().unwrap().channel_cycle_hotkey.clone())
            .unwrap_or_default();
        let backend = *self.backend.lock().unwrap();
        let mut applied: Option<String> = None;
        let mut error: Option<String> = None;
        if !cycle.trim().is_empty() {
            if kotone_core::hotkey::combos_conflict(&cycle, main_key) {
                let msg = format!("频道切换热键「{cycle}」与录制热键冲突，未注册");
                kotone_core::log::log(&msg);
                error = Some(msg);
            } else {
                let res = {
                    #[cfg(windows)]
                    {
                        if backend == ActiveBackend::LlHook {
                            self.llhook.set_cycle_key(Some(&cycle))
                        } else {
                            self.plugin.set_cycle_key(Some(&cycle))
                        }
                    }
                    #[cfg(not(windows))]
                    {
                        let _ = backend;
                        self.plugin.set_cycle_key(Some(&cycle))
                    }
                };
                match res {
                    Ok(()) => {
                        kotone_core::log::log(&format!("cycle hotkey registered: {cycle}"));
                        applied = Some(cycle.clone());
                    }
                    Err(e) => {
                        kotone_core::log::log(&format!("cycle hotkey register FAILED: {e}"));
                        error = Some(e);
                    }
                }
            }
        }
        *self.cycle_key.lock().unwrap() = applied;
        *self.cycle_error.lock().unwrap() = error;
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
            cycle_key: self.cycle_key.lock().unwrap().clone(),
            cycle_error: self.cycle_error.lock().unwrap().clone(),
        }
    }

    /// 独立输入环境自检：可由首启向导等流程主动调用，不要求用户先点击热键录入。
    ///
    /// 明确的 hook 安装失败或 SendInput 少发事件返回 `available=false`，供前端提前
    /// 提示 360/火绒信任区；合成事件未回环属于不确定结果，不能硬拦，真实物理键
    /// 会在 capture 阶段继续验证。
    pub fn check_input_environment(&self) -> InputEnvironmentCheck {
        #[cfg(windows)]
        {
            use kotone_platform_windows::hotkey_ll::ProbeOutcome;

            let result = match self
                .llhook
                .probe_available(std::time::Duration::from_millis(750))
            {
                Ok(ProbeOutcome::Verified) => InputEnvironmentCheck {
                    available: true,
                    hook_verified: true,
                    observed: 2,
                    expected: 2,
                    detail: None,
                },
                Ok(ProbeOutcome::Inconclusive { observed, expected }) => InputEnvironmentCheck {
                    available: true,
                    hook_verified: false,
                    observed,
                    expected,
                    detail: Some(format!(
                        "SendInput 已发出，但低级键盘钩子只收到 {observed}/{expected} 个探测事件；\
                             将在实际录入按键时继续验证"
                    )),
                },
                Err(detail) => InputEnvironmentCheck {
                    available: false,
                    hook_verified: false,
                    observed: 0,
                    expected: 2,
                    detail: Some(detail),
                },
            };
            kotone_core::log::log(&format!(
                "input environment preflight: available={} hook_verified={} observed={}/{} detail={}",
                result.available,
                result.hook_verified,
                result.observed,
                result.expected,
                result.detail.as_deref().unwrap_or("none")
            ));
            return result;
        }
        #[cfg(not(windows))]
        InputEnvironmentCheck {
            available: false,
            hook_verified: false,
            observed: 0,
            expected: 0,
            detail: Some("当前平台暂不支持低级键盘钩子与 SendInput 自检".into()),
        }
    }

    /// 开始热键捕获（设置页「点击录入」）：LL 钩子捕获下一个按键组合，
    /// 结果经 `kotone://hotkey-capture` 事件推送（{combo} / {cancelled} / {timeout}）。
    /// 捕获期间正常热键匹配暂停（matcher 吞掉录入主键、不产生会话事件）。
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
                .capture_next(cb, std::time::Duration::from_secs(10))
                .map_err(|detail| {
                    format!(
                        "低级键盘钩子自检未通过，暂时无法录入快捷键。\
                         可能是 360、火绒等安全软件拦截了键盘钩子或模拟输入；\
                         请将 Kotone 加入信任区后重试。检测详情：{detail}"
                    )
                });
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
        vec![format!(
            "「{key}」是常见游戏/系统键位，可能与游戏内操作冲突"
        )]
    } else {
        Vec::new()
    }
}
