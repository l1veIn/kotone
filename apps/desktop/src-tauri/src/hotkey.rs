//! 全局热键（Tauri 壳）：后端选择/回退/状态暴露（docs/development.md §3.6、§5.1）
//!
//! 端口在 core（`HotkeySource`）；两种实现：
//! - **LL 钩子**（kotone-platform-windows，Windows 默认）：WH_KEYBOARD_LL + WH_MOUSE_LL，
//!   解决 RegisterHotKey 在 LOL 等游戏前台不投递事件的问题；
//! - **RegisterHotKey**（tauri-plugin-global-shortcut，本文件 `PluginHotkeySource`）：
//!   依赖 AppHandle 故留在壳内；LL 钩子安装失败时回退，也是非 Windows 唯一实现。
//!
//! 对上层（TauriEmitter / IPC）签名不变：register/unregister/set_cancel_enabled/status。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use kotone_core::hotkey::{
    parse_hotkey, HookEvent, HotkeySource, HK_ASSISTANT, HK_AUTOMATION_STOP, HK_CHANNEL_CYCLE,
    HK_PP_CYCLE, HK_PP_TOGGLE, HK_RESEND_LAST, HK_RUNTIME_TOGGLE,
};
use kotone_core::hotkey_bindings::{legacy_by_id, CMD_TALK};
use kotone_core::hotkey_commands::{split_command, Edge};
use kotone_core::orchestrator::Orchestrator;
use kotone_core::settings::HotkeyBackend;

pub use kotone_core::hotkey::HotkeyMode;

// 登记什么、谁先拿到键都照「设置 → 热键」那张表算（ADR 040）；提醒助手热键的 id 也共用一份
use kotone_core::hotkey_table::{HintAt, Planned, HK_LEGACY_REMINDER, HK_RECORD};

/// 当前生效的热键后端
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveBackend {
    None,
    /// WH_KEYBOARD_LL + WH_MOUSE_LL 低级键鼠钩子
    LlHook,
    /// RegisterHotKey（tauri-plugin-global-shortcut）
    Plugin,
}

fn uses_mouse_button(key: &str) -> bool {
    parse_hotkey(key).is_some_and(|spec| spec.is_mouse_button())
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

/// 单个具名辅助热键的注册状态（设置页展示；未配置 / 未生效字段为 null）
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NamedHotkeyStatus {
    /// 具名热键稳定 id（见 core HK_* 常量）
    pub id: String,
    /// 已生效组合；未配置 / 未生效为 null
    pub key: Option<String>,
    /// 最近一次失败信息（如与录制/其他具名热键冲突；成功/未配置为 null）
    pub error: Option<String>,
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
    /// 具名辅助热键状态（含未配置项；按声明顺序稳定输出）
    pub named: Vec<NamedHotkeyStatus>,
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
    /// 已注册的具名辅助热键：id → 全局快捷键
    named: Mutex<HashMap<String, Shortcut>>,
}

impl PluginHotkeySource {
    pub fn new(app: &AppHandle, orch: Arc<Orchestrator>) -> Self {
        Self {
            app: app.clone(),
            orch,
            current: Mutex::new(None),
            cancel: Mutex::new(None),
            named: Mutex::new(HashMap::new()),
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
            .on_shortcut(shortcut, move |app, _sc, event| {
                kotone_core::log::log(&format!(
                    "hotkey fired: mode={mode:?} state={:?}",
                    event.state()
                ));
                let orch = orch.clone();
                match mode {
                    HotkeyMode::Hold => {
                        let pressed = event.state() == ShortcutState::Pressed;
                        dispatch_runtime_hotkey(
                            app,
                            &orch,
                            if pressed {
                                HookEvent::HoldPressed
                            } else {
                                HookEvent::HoldReleased
                            },
                        );
                    }
                    HotkeyMode::Toggle => {
                        if event.state() == ShortcutState::Pressed {
                            dispatch_runtime_hotkey(app, &orch, HookEvent::Toggle);
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
        self.clear_named_hotkeys();
    }

    /// 注册/更新/注销一个具名辅助热键（RegisterHotKey 后端）：同 id 幂等
    fn set_named_hotkey(&self, id: &str, combo: Option<&str>) -> Result<(), String> {
        self.set_named_hotkey_mode(id, combo, HotkeyMode::Toggle)
    }

    fn set_named_hotkey_mode(
        &self,
        id: &str,
        combo: Option<&str>,
        mode: HotkeyMode,
    ) -> Result<(), String> {
        if let Some(sc) = self.named.lock().unwrap().remove(id) {
            if let Err(e) = self.app.global_shortcut().unregister(sc) {
                kotone_core::log::log(&format!("注销具名热键失败: {e}"));
            }
        }
        let Some(combo) = combo.filter(|k| !k.trim().is_empty()) else {
            return Ok(());
        };
        let shortcut: Shortcut = combo
            .parse()
            .map_err(|e| format!("无法解析具名热键「{combo}」: {e}"))?;
        let orch = self.orch.clone();
        let id_owned = id.to_string();
        self.app
            .global_shortcut()
            .on_shortcut(shortcut, move |app, _sc, event| {
                if mode == HotkeyMode::Hold {
                    dispatch_runtime_hotkey(
                        app,
                        &orch,
                        HookEvent::NamedHold {
                            id: id_owned.clone(),
                            pressed: event.state() == ShortcutState::Pressed,
                        },
                    );
                } else if event.state() == ShortcutState::Pressed {
                    route_named_hotkey(app, &orch, &id_owned);
                }
            })
            .map_err(|e| format!("注册具名热键「{combo}」失败: {e}（键位可能被其他程序占用）"))?;
        self.named.lock().unwrap().insert(id.to_string(), shortcut);
        Ok(())
    }

    /// 注销全部具名热键（主热键与 Esc 取消不受影响）
    fn clear_named_hotkeys(&self) {
        let mut named = self.named.lock().unwrap();
        for (_, sc) in named.drain() {
            if let Err(e) = self.app.global_shortcut().unregister(sc) {
                kotone_core::log::log(&format!("注销具名热键失败: {e}"));
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
                let registered =
                    self.app
                        .global_shortcut()
                        .on_shortcut(shortcut, move |app, _sc, event| {
                            if event.state() == ShortcutState::Pressed {
                                let orch = orch.clone();
                                dispatch_runtime_hotkey(app, &orch, HookEvent::Cancel);
                            }
                        });
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
    app: AppHandle,
    orch: Arc<Orchestrator>,
) -> kotone_platform_windows::hotkey_ll::HookSink {
    Box::new(move |ev| {
        let orch = orch.clone();
        match ev {
            HookEvent::NamedHotkey { id } => route_named_hotkey(&app, &orch, &id),
            // 诊断事件（修饰键失配）：consumer 已记日志，业务不处理
            HookEvent::MainKeyMissed { .. } | HookEvent::NamedKeyMissed { .. } => {}
            event => dispatch_runtime_hotkey(&app, &orch, event),
        }
    })
}

/// 具名热键 id → 动作（id 由 core HK_* 常量声明；增量 = 加一个 arm）。
/// 两个后端（LL 钩子 sink 与 RegisterHotKey 回调）共用同一路由。
fn route_named_hotkey(app: &AppHandle, orch: &Arc<Orchestrator>, id: &str) {
    // 每个具名热键都留一行日志：热键「配置了但按下没反应」时，
    // 这行决定问题在输入链路（无日志）还是在业务侧（有日志没效果）。
    kotone_core::log::log(&format!("named hotkey fired: {id}"));
    // 应用级热键需要 AppHandle 才能启停运行时，故与其它热键分开走：
    // 它不依赖 runtime 生命周期的 orchestrator 状态。
    if id == HK_AUTOMATION_STOP {
        // 「停下」：自动化、琴音正在做的事、她派出的子会话和正在念的话一起停（ADR 028）。
        crate::stop::stop_everything(app, "停下热键");
        return;
    }
    // 语音识别没准备好时的被动提示（键照常传给游戏，D8）：要“听一句”的热键都这样（ADR 040 实施记录，
    // 主会话定的第 2 条）；插件的命令在悬浮窗提示，呼叫键、快捷指令在气泡。
    if let Some(target) = id.strip_prefix(HINT_PREFIX) {
        let running = app
            .try_state::<crate::runtime::RuntimeManager>()
            .is_some_and(|rt| rt.phase() == kotone_core::runtime::RuntimePhase::Running);
        let at = app.state::<HotkeyManager>().hint_at(target);
        if !running {
            crate::runtime::not_ready_hint(
                app,
                if at == Some(HintAt::Overlay) {
                    crate::runtime::HintKind::VoiceInput
                } else {
                    crate::runtime::HintKind::Call
                },
            );
        }
        return;
    }
    if id == HK_RUNTIME_TOGGLE {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            match crate::runtime::toggle(&app).await {
                Ok(status) => {
                    kotone_core::log::log(&format!("runtime toggle hotkey: -> {}", status.phase))
                }
                Err(e) => kotone_core::log::log(&format!("runtime toggle hotkey: {e}")),
            }
        });
        return;
    }
    // 自己加的、不用听的（运行自动化……，ADR 040 第 3 期）：随应用登记，识别没在跑也照样按
    if crate::hotkey_run::is_custom(id) && !crate::hotkey_run::needs_listening(app, id) {
        crate::hotkey_run::press(app, id);
        return;
    }
    dispatch_runtime_hotkey(app, orch, HookEvent::NamedHotkey { id: id.to_string() });
}

/// 主键位上的“说话”交给接在“听一句”后面的插件（第 4 步删主键位以后，说话和别的命令一样走具名热键）。
async fn hearing_main(orch: &Orchestrator, edge: Edge, ticket: u64) {
    if let Some((_, command)) = split_command(CMD_TALK) {
        orch.hearing_command(command, edge, Some(ticket)).await;
    }
}

/// 登记 id → 接在“听一句”后面那个插件的命令 id（`channel-cycle` → `voice-input.channel-cycle` → `channel-cycle`）；
/// 不是它的命令为 None。
fn hearing_command_of<'a>(orch: &Orchestrator, id: &'a str) -> Option<&'a str> {
    hearing_command_for(orch.hearing_owner(), id)
}

fn hearing_command_for<'a>(owner: Option<&str>, id: &'a str) -> Option<&'a str> {
    let full = legacy_by_id(id).map_or(id, |legacy| legacy.command);
    let (source, command) = split_command(full)?;
    (Some(source) == owner).then_some(command)
}

/// 事件到达时记录运行时与核心入场票据；启停、取消和重注册后拒绝旧输入。
fn dispatch_runtime_hotkey(app: &AppHandle, orch: &Arc<Orchestrator>, event: HookEvent) {
    let Some(runtime) = app.try_state::<crate::runtime::RuntimeManager>() else {
        return;
    };
    if event == HookEvent::Cancel {
        // Esc invalidates queued presses immediately. Keep admission paused until
        // cancel reaches the core, so a newer press cannot overtake that cancel.
        // This path never waits for the edge queue or an AI request to finish.
        if runtime.phase() != kotone_core::runtime::RuntimePhase::Running {
            return;
        }
        let pause = runtime.pause_inputs();
        orch.invalidate_hotkey_inputs();
        let orch = orch.clone();
        tauri::async_runtime::spawn(async move {
            let _pause = pause;
            orch.cancel().await;
        });
        return;
    }
    let Some(runtime_ticket) = runtime.input_ticket() else {
        return;
    };
    let ticket = HotkeyInputTicket {
        runtime: runtime_ticket,
        core: orch.hotkey_input_ticket(),
    };
    // Press/release must reach the core in physical order. Spawning each edge
    // separately can schedule release first, leaving a short press recording.
    // Other actions (especially Esc) stay independent so they can cancel input.
    if let HookEvent::NamedHold { id, pressed } = &event {
        if id == HK_LEGACY_REMINDER {
            let _ = app
                .state::<HotkeyManager>()
                .assistant_edges
                .send((ticket, *pressed));
        }
        return;
    }
    let app = app.clone();
    let orch = orch.clone();
    tauri::async_runtime::spawn(async move {
        if !app
            .state::<crate::runtime::RuntimeManager>()
            .accepts_input(ticket.runtime)
        {
            return;
        }
        match event {
            HookEvent::NamedHold { .. } => unreachable!("hold edges use the ordered dispatcher"),
            // 主键位上的是“说话”（插件登记的按住命令）：按住还是按一下由插件照交互模式决定（ADR 040 D1）
            HookEvent::HoldPressed | HookEvent::Toggle => {
                hearing_main(&orch, Edge::Press, ticket.core).await
            }
            HookEvent::HoldReleased => hearing_main(&orch, Edge::Release, ticket.core).await,
            HookEvent::Cancel => unreachable!("cancel bypasses queued input"),
            HookEvent::NamedHotkey { id } => match id.as_str() {
                HK_ASSISTANT => {
                    if crate::call::active() {
                        // 通话中再按一次 = 挂断（ADR 033 D1）。
                        crate::call::hang_up_by_key(&app).await;
                    } else {
                        let timeout = {
                            let state = app.state::<crate::SharedState>();
                            let settings = state.settings.read().unwrap();
                            settings.assistant.wake.no_speech_timeout_ms
                        };
                        // 她正在说时按呼叫键：先停下她，再开始听（ADR 029 §五）。
                        crate::assistant::interrupt_before_call(&app).await;
                        orch.call_assistant(Some(ticket.core), timeout).await;
                        // 接通（ADR 033）：这句说完以后一直在听，直到挂断。
                        crate::call::start(&app);
                    }
                }
                HK_LEGACY_REMINDER => orch.on_assistant_hotkey_with_ticket(ticket.core).await,
                // 自己加的、要听的（快捷指令：替你跟她说一句，不经过语音识别，ADR 040 第 3 期）
                other if crate::hotkey_run::is_custom(other) => {
                    crate::hotkey_run::press(&app, other)
                }
                // 插件登记的命令：接在“听一句”后面的那个插件的，经本体转给它（ADR 040 D1）
                other => match hearing_command_of(&orch, other) {
                    Some(command) => {
                        orch.hearing_command(command, Edge::Press, Some(ticket.core))
                            .await
                    }
                    None => kotone_core::log::log(&format!("未知具名热键: {id}")),
                },
            },
            HookEvent::MainKeyMissed { .. } | HookEvent::NamedKeyMissed { .. } => {}
        }
    });
}

#[derive(Clone, Copy)]
struct HotkeyInputTicket {
    runtime: u64,
    core: u64,
}

/// `hotkeys.status` 的 `named[]` 和设置页按 id 查状态的顺序（第 1 期起的 id；“说话”占主键位，状态在 `key` 里）。
/// 登记计划里不在这里的（插件新加的命令、自己加的 `custom.<id>`）接在后面。第 3 期以前快捷指令的
/// `quick-message-1`…`8` 固定在这里，现在每条用自己的 id（ADR 040 第 3 期，`docs/tool-service.md`）。
const KNOWN_NAMED_HOTKEYS: &[&str] = &[
    HK_ASSISTANT,
    HK_LEGACY_REMINDER,
    HK_CHANNEL_CYCLE,
    HK_RESEND_LAST,
    HK_PP_TOGGLE,
    HK_PP_CYCLE,
    HK_RUNTIME_TOGGLE,
    HK_AUTOMATION_STOP,
];

/// “说话”占的主键位（登记计划里的 id）。第 2 期第 4 步真机跑完以后删掉主键位，说话改成具名按住。
const MAIN_ID: &str = HK_RECORD;

/// 被动提示键（识别没在跑时只看不吞）的登记 id 前缀：`hint:<id>`。
const HINT_PREFIX: &str = "hint:";

/// 一条要落到后端的具名热键。
#[derive(Debug, Clone, PartialEq)]
struct Entry {
    id: String,
    combo: String,
    mode: HotkeyMode,
    /// 登记计划里的错误（撞了 Esc、被前面的占了……）：有就不登记，记进状态。
    error: Option<String>,
}

fn entry(planned: &Planned) -> Entry {
    Entry {
        id: planned.id.clone(),
        combo: planned.combo.clone(),
        mode: if planned.hold {
            HotkeyMode::Hold
        } else {
            HotkeyMode::Toggle
        },
        error: planned.error.clone(),
    }
}

/// 登记计划（表的顺序 = 谁先拿到键；冲突、Esc、插件关着都在里面算好，ADR 040 D6）。
fn plan_from(settings: &kotone_core::settings::Settings) -> Vec<Planned> {
    crate::hotkey_table_cmd::registration_plan(settings)
}

/// 随语音识别登记的具名热键（不含说话：它占主键位）。
fn runtime_entries(plan: &[Planned]) -> Vec<Entry> {
    plan.iter()
        .filter(|p| !p.always && p.id != MAIN_ID)
        .map(entry)
        .collect()
}

/// 识别没在跑也登记的（停下热键、启停本地语音识别）。
fn app_entries(plan: &[Planned]) -> Vec<Entry> {
    plan.iter().filter(|p| p.always).map(entry).collect()
}

/// 识别没在跑时只看不吞、按了提示的：(登记 id, 键, 在哪提示)。被前面的占了键的不盯。
fn hint_entries(plan: &[Planned]) -> Vec<(String, String, HintAt)> {
    plan.iter()
        .filter(|p| !p.always && p.error.is_none())
        .filter_map(|p| {
            p.hint
                .map(|at| (format!("{HINT_PREFIX}{}", p.id), p.combo.clone(), at))
        })
        .collect()
}

/// 运行期具名热键清单（状态顺序；没登记的键为空）：测试用。
#[cfg(test)]
fn named_list_from(
    settings: &kotone_core::settings::Settings,
) -> Vec<(String, String, HotkeyMode)> {
    let plan = plan_from(settings);
    let runtime = runtime_entries(&plan);
    let reminder_mode = if settings.assistant.interaction_mode
        == kotone_core::assistant::AssistantInteractionMode::PushToTalk
    {
        HotkeyMode::Hold
    } else {
        HotkeyMode::Toggle
    };
    let always: Vec<&str> = [HK_RUNTIME_TOGGLE, HK_AUTOMATION_STOP].to_vec();
    let mut list: Vec<(String, String, HotkeyMode)> = KNOWN_NAMED_HOTKEYS
        .iter()
        .filter(|id| !always.contains(id))
        .map(|id| match runtime.iter().find(|e| e.id == *id) {
            Some(e) => (e.id.clone(), e.combo.clone(), e.mode),
            None => (
                id.to_string(),
                String::new(),
                if *id == HK_LEGACY_REMINDER {
                    reminder_mode
                } else {
                    HotkeyMode::Toggle
                },
            ),
        })
        .collect();
    for e in runtime
        .iter()
        .filter(|e| !KNOWN_NAMED_HOTKEYS.contains(&e.id.as_str()))
    {
        list.push((e.id.clone(), e.combo.clone(), e.mode));
    }
    list
}

/// 测试用：运行期具名热键（id, 组合）。
#[cfg(test)]
pub(crate) fn named_list_for_tests(
    settings: &kotone_core::settings::Settings,
) -> Vec<(String, String)> {
    named_list_from(settings)
        .into_iter()
        .map(|(id, combo, _)| (id, combo))
        .collect()
}

/// 热键管理器：后端选择/回退 + 状态暴露；对上层签名不变
pub struct HotkeyManager {
    assistant_edges: tokio::sync::mpsc::UnboundedSender<(HotkeyInputTicket, bool)>,
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
    /// 已生效的具名辅助热键：id → 已注册组合（空串 = 未配置）
    named: Mutex<HashMap<String, Option<String>>>,
    /// 具名热键最近一次失败信息：id → 错误（冲突/注册失败；成功/未配置为 None）
    named_errors: Mutex<HashMap<String, Option<String>>>,
    /// 应用级具名热键（runtime 未运行也生效）：id → 已生效组合
    app_named: Mutex<HashMap<String, Option<String>>>,
    /// 应用级具名热键的失败信息（冲突/钩子与插件都注册失败）
    app_named_errors: Mutex<HashMap<String, Option<String>>>,
    /// 当前生效的被动提示键（runtime 未运行时）：登记 id（`hint:<id>`）→ (组合, 在哪提示)
    hints: Mutex<HashMap<String, (String, HintAt)>>,
}

impl HotkeyManager {
    pub fn new(app: &AppHandle, orch: Arc<Orchestrator>) -> Self {
        let (assistant_edges, mut edges) =
            tokio::sync::mpsc::unbounded_channel::<(HotkeyInputTicket, bool)>();
        let edge_app = app.clone();
        let edge_orch = orch.clone();
        tauri::async_runtime::spawn(async move {
            while let Some((ticket, pressed)) = edges.recv().await {
                if edge_app
                    .state::<crate::runtime::RuntimeManager>()
                    .accepts_input(ticket.runtime)
                {
                    edge_orch
                        .on_assistant_hotkey_hold_with_ticket(pressed, ticket.core)
                        .await;
                }
            }
        });
        Self {
            assistant_edges,
            plugin: PluginHotkeySource::new(app, orch.clone()),
            #[cfg(windows)]
            llhook: kotone_platform_windows::hotkey_ll::LlHookSource::new(make_llhook_sink(
                app.clone(),
                orch,
            )),
            backend: Mutex::new(ActiveBackend::None),
            registered_key: Mutex::new(None),
            last_error: Mutex::new(None),
            named: Mutex::new(HashMap::new()),
            named_errors: Mutex::new(HashMap::new()),
            app_named: Mutex::new(HashMap::new()),
            app_named_errors: Mutex::new(HashMap::new()),
            hints: Mutex::new(HashMap::new()),
        }
    }

    /// 语音识别没准备好时（runtime 不在 Running）盯住要“听一句”的热键（说话、呼叫键、语音输入的其他热键、
    /// 快捷指令）：被动、不吞键，按下只弹提示（D8；ADR 040 实施记录主会话定的第 2 条）。`active = false`
    /// （进入 Running）时全部撤掉。由 `runtime::refresh_hints` 在配置锁内调用。钩子不可用时不提示（RegisterHotKey 会吞键）。
    pub fn set_runtime_hints(&self, settings: &kotone_core::settings::Settings, active: bool) {
        let wanted: HashMap<String, (String, HintAt)> = if active {
            hint_entries(&plan_from(settings))
                .into_iter()
                .map(|(id, combo, at)| (id, (combo, at)))
                .collect()
        } else {
            HashMap::new()
        };
        let mut current = self.hints.lock().unwrap();
        if *current == wanted {
            return;
        }
        #[cfg(windows)]
        {
            for id in current.keys() {
                let _ = self.llhook.set_passive_hotkey(id, None);
            }
            current.clear();
            if wanted.is_empty() {
                return;
            }
            if let Err(e) = self.llhook.ensure_hook_ready() {
                kotone_core::log::log(&format!("没准备好的提示键未启用（钩子不可用）：{e}"));
                return;
            }
            for (id, (combo, at)) in wanted {
                match self.llhook.set_passive_hotkey(&id, Some(&combo)) {
                    Ok(()) => {
                        current.insert(id, (combo, at));
                    }
                    Err(e) => kotone_core::log::log(&format!("提示键 {id} 未启用：{e}")),
                }
            }
        }
        #[cfg(not(windows))]
        {
            let _ = &wanted;
            current.clear();
        }
    }

    /// 被动提示键在哪提示（`id` 是去掉 `hint:` 的登记 id）。
    fn hint_at(&self, id: &str) -> Option<HintAt> {
        self.hints
            .lock()
            .unwrap()
            .get(&format!("{HINT_PREFIX}{id}"))
            .map(|(_, at)| *at)
    }

    /// 用显式配置注册热键。设置事务在内存发布 next 之前调用此方法，避免注册过程
    /// 再从 SharedState 读到 old，导致主热键与频道/重发热键来自不同配置版本。
    ///
    /// 登记什么照登记计划（`hotkey_table::registration_plan`，ADR 040）：“说话”占主键位（`mode` 是它的交互模式），
    /// 其余随语音识别登记的是具名热键。`key` 是旧参数（就是 `hotkey.key`，热键表里说话那条的镜像），以计划为准。
    pub fn register_with_settings(
        &self,
        app: &AppHandle,
        key: &str,
        mode: HotkeyMode,
        settings: &kotone_core::settings::Settings,
    ) -> Result<(), String> {
        self.unregister(app)?;
        let _ = key;
        let plan = plan_from(settings);
        // 语音输入插件关着（ADR 038）：计划里没有说话，主键空着，呼叫键这些具名热键照常；
        // 钩子照样跑（主键停用、全部放行，会话中的 Esc 取消照常）。说话的键被前面的占了、撞了 Esc 时也不登记。
        let main = plan.iter().find(|p| p.id == MAIN_ID);
        let main_error = main.and_then(|p| p.error.clone());
        let key = main
            .filter(|p| p.error.is_none())
            .map(|p| p.combo.clone())
            .unwrap_or_default();
        let voice_on = !key.is_empty();
        let named = runtime_entries(&plan);
        let pref = settings.hotkey_backend;

        #[cfg(windows)]
        let requires_llhook = plan
            .iter()
            .any(|p| p.error.is_none() && uses_mouse_button(&p.combo));

        #[cfg(windows)]
        if pref != HotkeyBackend::Register || requires_llhook {
            if requires_llhook && pref == HotkeyBackend::Register {
                kotone_core::log::log("鼠标侧键不受 RegisterHotKey 支持，已强制使用低层键鼠钩子");
            }
            let registered = if voice_on {
                self.llhook.register(&key, mode)
            } else {
                self.llhook.register_without_main()
            };
            match registered {
                Ok(()) => {
                    kotone_core::log::log(&format!(
                        "hotkey backend=llhook registered ok: {key} ({mode:?})"
                    ));
                    *self.backend.lock().unwrap() = ActiveBackend::LlHook;
                    *self.registered_key.lock().unwrap() = voice_on.then(|| key.clone());
                    *self.last_error.lock().unwrap() = main_error;
                    self.apply_named(&named);
                    return Ok(());
                }
                Err(e) => {
                    if requires_llhook {
                        let msg = format!(
                            "鼠标侧键热键需要低层键鼠钩子，但钩子注册失败：{e}。\
                             请将 Kotone 加入安全软件信任区后重试"
                        );
                        kotone_core::log::log(&format!("hotkey register FAILED: {msg}"));
                        *self.last_error.lock().unwrap() = Some(msg.clone());
                        return Err(msg);
                    }
                    kotone_core::log::log(&format!("llhook 后端不可用，回退 RegisterHotKey: {e}"));
                }
            }
        }

        let registered = if voice_on {
            self.plugin.register(&key, mode)
        } else {
            self.plugin.unregister();
            Ok(())
        };
        match registered {
            Ok(()) => {
                kotone_core::log::log(&format!(
                    "hotkey backend=register registered ok: {key} ({mode:?})"
                ));
                *self.backend.lock().unwrap() = ActiveBackend::Plugin;
                *self.registered_key.lock().unwrap() = voice_on.then(|| key.clone());
                *self.last_error.lock().unwrap() = main_error;
                self.apply_named(&named);
                Ok(())
            }
            Err(msg) => {
                kotone_core::log::log(&format!("hotkey register FAILED: {msg}"));
                *self.last_error.lock().unwrap() = Some(msg.clone());
                Err(msg)
            }
        }
    }

    /// 注销当前热键（两个后端都停）。应用级热键不在此列：runtime 停止后
    /// 仍要能用「启动 / 停止」热键把 Kotone 拉起来，故末尾重新落回后端。
    pub fn unregister(&self, _app: &AppHandle) -> Result<(), String> {
        #[cfg(windows)]
        {
            self.llhook.unregister();
            // 也会清掉被动提示键：停止后由 runtime::refresh_hints 重新盯上
            self.llhook.clear_named_hotkeys();
            self.hints.lock().unwrap().clear();
        }
        self.plugin.unregister();
        *self.backend.lock().unwrap() = ActiveBackend::None;
        *self.registered_key.lock().unwrap() = None;
        self.named.lock().unwrap().clear();
        self.named_errors.lock().unwrap().clear();
        self.reapply_app_named();
        Ok(())
    }

    // ---------- 应用级具名热键（runtime 未运行也生效） ----------

    /// 应用识别没在跑也登记的热键（登记计划里 `always` 的：停下热键、启停本地语音识别）。与运行期具名热键不同，
    /// 它由设置事务直接驱动，不参与 `apply_named` 的清单重放。
    ///
    /// 注册失败不回滚配置（与具名热键一致）：错误记入状态由设置页展示，
    /// 用户可据此换键，而不是连保存都做不到。
    pub fn apply_app_hotkeys(&self, settings: &kotone_core::settings::Settings) {
        let entries = app_entries(&plan_from(settings));
        let mut applied: HashMap<String, Option<String>> = HashMap::new();
        let mut errors: HashMap<String, Option<String>> = HashMap::new();
        // 不再登记的（清掉了键、被前面的占了）先摘掉
        let previous: Vec<String> = self.app_named.lock().unwrap().keys().cloned().collect();
        for id in previous {
            if !entries.iter().any(|e| e.id == id && e.error.is_none()) {
                self.clear_app_hotkey(&id);
            }
        }
        for e in entries {
            if let Some(error) = e.error {
                kotone_core::log::log(&format!("app hotkey {} 没登记：{error}", e.id));
                self.clear_app_hotkey(&e.id);
                applied.insert(e.id.clone(), None);
                errors.insert(e.id, Some(error));
                continue;
            }
            match self.register_app_hotkey(&e.id, &e.combo) {
                Ok(()) => {
                    applied.insert(e.id.clone(), Some(e.combo));
                    errors.insert(e.id, None);
                }
                Err(err) => {
                    kotone_core::log::log(&format!("app hotkey {} register FAILED: {err}", e.id));
                    applied.insert(e.id.clone(), None);
                    errors.insert(e.id, Some(err));
                }
            }
        }
        *self.app_named.lock().unwrap() = applied;
        *self.app_named_errors.lock().unwrap() = errors;
    }

    /// 注册单个应用级热键：优先低层钩子（游戏前台也能收到——本热键的主要用途
    /// 正是「在游戏里启动 / 停止 Kotone」），钩子不可用时回退 RegisterHotKey。
    /// 和别的热键撞不撞在登记计划里算过了（ADR 040 D6），这里只管落到后端。
    fn register_app_hotkey(&self, id: &str, combo: &str) -> Result<(), String> {
        #[cfg(windows)]
        {
            match self.llhook.ensure_hook_ready() {
                Ok(()) => match self.llhook.set_named_hotkey(id, Some(combo)) {
                    Ok(()) => {
                        // 摘掉插件后端可能残留的同 id 注册，避免同一键在两个后端各触发一次
                        let _ = self.plugin.set_named_hotkey(id, None);
                        kotone_core::log::log(&format!(
                            "app hotkey {id} registered on llhook: {combo}"
                        ));
                        return Ok(());
                    }
                    Err(e) => kotone_core::log::log(&format!(
                        "app hotkey {id} llhook failed: {e}；回退 RegisterHotKey"
                    )),
                },
                Err(e) => kotone_core::log::log(&format!(
                    "llhook 不可用（{e}），应用级热键回退 RegisterHotKey"
                )),
            }
            // 回退路径先把钩子侧的残留摘干净（双后端同时命中 = 触发两次 = 白按）
            let _ = self.llhook.set_named_hotkey(id, None);
        }
        self.plugin
            .set_named_hotkey(id, Some(combo))
            .map_err(|e| format!("注册热键「{combo}」失败: {e}（键位可能被其他程序占用）"))
    }

    /// 从两个后端都摘掉一个应用级热键（不关心它当初落在哪个后端）
    fn clear_app_hotkey(&self, id: &str) {
        #[cfg(windows)]
        let _ = self.llhook.set_named_hotkey(id, None);
        let _ = self.plugin.set_named_hotkey(id, None);
    }

    /// 把已生效的应用级热键重新落到后端：清空具名热键 / 注销主热键之后调用
    /// （`clear_named_hotkeys` 会把它们一并清掉）。
    fn reapply_app_named(&self) {
        let combos: Vec<(String, String)> = self
            .app_named
            .lock()
            .unwrap()
            .iter()
            .filter_map(|(id, combo)| combo.as_ref().map(|c| (id.clone(), c.clone())))
            .collect();
        if combos.is_empty() {
            return;
        }
        for (id, combo) in combos {
            if let Err(e) = self.register_app_hotkey(&id, &combo) {
                kotone_core::log::log(&format!("app hotkey {id} re-register FAILED: {e}"));
                self.app_named.lock().unwrap().insert(id.clone(), None);
                self.app_named_errors.lock().unwrap().insert(id, Some(e));
            }
        }
    }

    /// 落下全部运行期具名热键（主热键注册成功后按当前生效后端应用）。谁先拿到键、撞没撞 Esc 在登记计划里
    /// 算好了（声明顺序 = 表的顺序，ADR 040 D6）：计划里有错误的不登记，错误记入对应 id（设置页展示）。
    fn apply_named(&self, named: &[Entry]) {
        let backend = *self.backend.lock().unwrap();
        let mut applied: HashMap<String, Option<String>> = HashMap::new();
        let mut errors: HashMap<String, Option<String>> = HashMap::new();
        // 先清空后端全部具名热键，再按新清单重放（幂等重注册）
        {
            #[cfg(windows)]
            {
                if backend == ActiveBackend::LlHook {
                    self.llhook.clear_named_hotkeys();
                    self.hints.lock().unwrap().clear();
                } else {
                    self.plugin.clear_named_hotkeys();
                }
            }
            #[cfg(not(windows))]
            self.plugin.clear_named_hotkeys();
        }
        for e in named {
            if let Some(error) = &e.error {
                kotone_core::log::log(&format!("named hotkey {} 没登记：{error}", e.id));
                applied.insert(e.id.clone(), None);
                errors.insert(e.id.clone(), Some(error.clone()));
                continue;
            }
            let res = {
                #[cfg(windows)]
                {
                    if backend == ActiveBackend::LlHook {
                        self.llhook
                            .set_named_hotkey_mode(&e.id, Some(&e.combo), e.mode)
                    } else {
                        self.plugin
                            .set_named_hotkey_mode(&e.id, Some(&e.combo), e.mode)
                    }
                }
                #[cfg(not(windows))]
                {
                    let _ = backend;
                    self.plugin
                        .set_named_hotkey_mode(&e.id, Some(&e.combo), e.mode)
                }
            };
            match res {
                Ok(()) => {
                    kotone_core::log::log(&format!(
                        "named hotkey {} registered: {}",
                        e.id, e.combo
                    ));
                    applied.insert(e.id.clone(), Some(e.combo.clone()));
                    errors.insert(e.id.clone(), None);
                }
                Err(err) => {
                    kotone_core::log::log(&format!("named hotkey {} register FAILED: {err}", e.id));
                    applied.insert(e.id.clone(), None);
                    errors.insert(e.id.clone(), Some(err));
                }
            }
        }
        *self.named.lock().unwrap() = applied;
        *self.named_errors.lock().unwrap() = errors;
        // 上面的 clear_named_hotkeys 会把应用级热键一并清掉，这里补回去
        self.reapply_app_named();
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
        let named_status = {
            let named = self.named.lock().unwrap();
            let errors = self.named_errors.lock().unwrap();
            let app_named = self.app_named.lock().unwrap();
            let app_errors = self.app_named_errors.lock().unwrap();
            // 状态顺序照旧（设置页按 id 查）；计划里新加、不在旧清单里的接在后面
            let mut ids: Vec<String> = KNOWN_NAMED_HOTKEYS
                .iter()
                .map(|id| id.to_string())
                .collect();
            for id in named.keys().chain(app_named.keys()) {
                if !ids.contains(id) {
                    ids.push(id.clone());
                }
            }
            ids.into_iter()
                .map(|id| {
                    // 应用级热键单独存放（不随 runtime 注销），状态合并输出给设置页
                    let (keys, errs) = if app_named.contains_key(&id) {
                        (&app_named, &app_errors)
                    } else {
                        (&named, &errors)
                    };
                    NamedHotkeyStatus {
                        key: keys.get(&id).cloned().flatten(),
                        error: errs.get(&id).cloned().flatten(),
                        id,
                    }
                })
                .collect()
        };
        HotkeyStatus {
            registered: backend != ActiveBackend::None,
            key,
            error,
            backend: backend.label().to_string(),
            named: named_status,
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
            result
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

    /// 开始热键捕获（设置页「点击录入」）：LL 钩子捕获下一个键盘组合或鼠标侧键，
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
            self.llhook
                .capture_next(cb, std::time::Duration::from_secs(10))
                .map_err(|detail| {
                    format!(
                        "低级键鼠钩子自检未通过，暂时无法录入热键。\
                         可能是 360、火绒等安全软件拦截了输入钩子或模拟输入；\
                         请将 Kotone 加入信任区后重试。检测详情：{detail}"
                    )
                })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mouse_button_in_any_hotkey_requires_llhook() {
        assert!(uses_mouse_button("Mouse4"));
        assert!(uses_mouse_button("Ctrl+Mouse5"));
        assert!(uses_mouse_button("XButton1"));
        assert!(!uses_mouse_button("Shift+F9") && !uses_mouse_button(""));
    }

    /// 几条快捷指令（跟她说一句），id 是 `q0`、`q1`……，键 Ctrl+F1……
    fn quick_messages(count: usize) -> Vec<kotone_core::assistant::QuickMessage> {
        (0..count)
            .map(|i| kotone_core::assistant::QuickMessage {
                id: format!("q{i}"),
                text: "集合".into(),
                hotkey: format!("Ctrl+F{}", i + 1),
                ..Default::default()
            })
            .collect()
    }

    /// 计划里要登记的每个具名热键都在状态清单里：设置页按 id 查注册状态，
    /// 漏一个就会出现「配置了但看不到状态」的静默失效（自己加的接在后面，id 是 `custom.<id>`）。
    /// 识别没跑也登记的正好是停下、启停识别和不用听的自己加的（运行自动化）。
    #[test]
    fn every_planned_hotkey_has_a_status_slot() {
        let mut settings = kotone_core::settings::Settings {
            channel_cycle_hotkey: "Shift+CapsLock".into(),
            resend_last_hotkey: "Alt+F6".into(),
            toggle_post_processing_hotkey: "Alt+F7".into(),
            cycle_post_processing_hotkey: "Alt+F8".into(),
            runtime_toggle_hotkey: "Ctrl+Alt+K".into(),
            ..Default::default()
        };
        settings.assistant.companion.quick_messages = quick_messages(8);
        settings.assistant.companion.quick_messages[7].automation_id = "auto-1".into();
        let plan = plan_from(&settings);
        for p in &plan {
            assert!(p.error.is_none(), "{} {:?}", p.id, p.error);
            assert!(
                p.id == MAIN_ID
                    || KNOWN_NAMED_HOTKEYS.contains(&p.id.as_str())
                    || crate::hotkey_run::is_custom(&p.id),
                "{} 不在状态清单里",
                p.id
            );
        }
        let app: Vec<String> = app_entries(&plan).into_iter().map(|e| e.id).collect();
        assert_eq!(
            app,
            vec![HK_AUTOMATION_STOP, HK_RUNTIME_TOGGLE, "custom.q7"]
        );
        assert_eq!(
            runtime_entries(&plan).len(),
            KNOWN_NAMED_HOTKEYS.len() - 3 + 7,
            "不含提醒助手和两个应用级；7 条跟她说一句随识别登记"
        );
    }

    #[test]
    fn legacy_hotkeys_keep_the_reminder_mode_and_register_no_new_bindings() {
        use kotone_core::assistant::AssistantInteractionMode;
        use kotone_core::settings::LegacyUpgrade;

        let mut settings = kotone_core::settings::Settings::default();
        settings.ui.legacy_upgrade = LegacyUpgrade::Pending;
        settings.assistant.hotkey.clear();
        let (id, key, mode) = &named_list_from(&settings)[1];
        assert_eq!(id, HK_LEGACY_REMINDER);
        assert!(key.is_empty());
        assert_eq!(*mode, HotkeyMode::Hold);
        assert!(named_list_from(&settings)[0].1.is_empty());
        assert!(app_entries(&plan_from(&settings)).is_empty());

        settings.assistant.hotkey = "F9".into();
        settings.ui.legacy_upgrade = LegacyUpgrade::Dismissed;
        assert_eq!(
            named_list_from(&settings)[1],
            (
                HK_LEGACY_REMINDER.to_string(),
                "F9".into(),
                HotkeyMode::Hold
            )
        );
        settings.assistant.interaction_mode = AssistantInteractionMode::Dictation;
        assert_eq!(
            named_list_from(&settings)[1],
            (
                HK_LEGACY_REMINDER.to_string(),
                "F9".into(),
                HotkeyMode::Toggle
            )
        );
    }

    #[test]
    fn legacy_opt_in_keeps_both_hotkey_bindings() {
        use kotone_core::assistant::CompanionBindings;
        use kotone_core::settings::LegacyUpgrade;

        let mut settings = kotone_core::settings::Settings::default();
        settings.ui.legacy_upgrade = LegacyUpgrade::OptedIn;
        settings.assistant.hotkey = "F9".into();
        settings.assistant.companion_bindings = Some(CompanionBindings {
            hotkey: "F10".into(),
            connection_id: "game".into(),
        });
        assert_eq!(
            &named_list_from(&settings)[..2],
            &[
                (HK_ASSISTANT.to_string(), "F10".into(), HotkeyMode::Toggle),
                (
                    HK_LEGACY_REMINDER.to_string(),
                    "F9".into(),
                    HotkeyMode::Hold
                ),
            ]
        );
    }

    /// 识别没在跑时盯住要“听一句”的热键（只看不吞、按了提示）：呼叫键在气泡、语音输入的在悬浮窗
    /// （ADR 040 实施记录，主会话定的第 2 条：切换频道这些也一样）。被前面的热键占了键的不盯。
    #[test]
    fn hints_watch_every_listening_key_except_ones_that_lost_their_key() {
        let mut settings = kotone_core::settings::Settings::default();
        settings.hotkey.key = "CapsLock".into();
        settings.assistant.hotkey = "Ctrl+F9".into();
        settings.channel_cycle_hotkey = "Shift+CapsLock".into();
        assert_eq!(
            hint_entries(&plan_from(&settings)),
            vec![
                (
                    "hint:assistant".to_string(),
                    "Ctrl+F9".to_string(),
                    HintAt::Bubble
                ),
                (
                    "hint:record".to_string(),
                    "CapsLock".to_string(),
                    HintAt::Overlay
                ),
                (
                    "hint:channel-cycle".to_string(),
                    "Shift+CapsLock".to_string(),
                    HintAt::Overlay
                ),
            ]
        );
        // 停下热键和说话同一个键：停下在表里靠前，说话没拿到键，也不盯
        settings.automation_stop_hotkey = "CapsLock".into();
        let ids: Vec<String> = hint_entries(&plan_from(&settings))
            .into_iter()
            .map(|(id, _, _)| id)
            .collect();
        assert_eq!(ids, vec!["hint:assistant", "hint:channel-cycle"]);
        // 0.1.x 用户还没启用琴音：没有呼叫键
        settings.automation_stop_hotkey = String::new();
        settings.ui.legacy_upgrade = kotone_core::settings::LegacyUpgrade::Dismissed;
        let ids: Vec<String> = hint_entries(&plan_from(&settings))
            .into_iter()
            .map(|(id, _, _)| id)
            .collect();
        assert_eq!(ids, vec!["hint:record", "hint:channel-cycle"]);
        settings.hotkey.key = String::new();
        settings.channel_cycle_hotkey = String::new();
        assert!(hint_entries(&plan_from(&settings)).is_empty());
    }

    #[test]
    fn calling_kotone_is_always_a_single_press_regardless_of_old_modes() {
        use kotone_core::interaction::InteractionMode;
        let mut settings = kotone_core::settings::Settings {
            interaction_mode: Some(InteractionMode::PushToTalk),
            ..Default::default()
        };
        // 呼叫琴音按一次就听：旧配置里的“对讲机”也不再按住说话
        settings.assistant.interaction_mode =
            kotone_core::assistant::AssistantInteractionMode::PushToTalk;
        let call = named_list_from(&settings)
            .into_iter()
            .find(|(id, _, _)| id == HK_ASSISTANT);
        assert_eq!(
            call,
            Some((
                HK_ASSISTANT.to_string(),
                kotone_core::assistant::DEFAULT_CALL_HOTKEY.into(),
                HotkeyMode::Toggle
            ))
        );
    }

    /// 插件登记的命令按下时交给接在“听一句”后面的那个插件（ADR 040 D1）；不是它的不转。
    #[test]
    fn plugin_commands_go_to_the_plugin_after_hearing() {
        let owner = Some("voice-input");
        assert_eq!(
            hearing_command_for(owner, HK_CHANNEL_CYCLE),
            Some("channel-cycle")
        );
        assert_eq!(
            hearing_command_for(owner, HK_PP_CYCLE),
            Some("post-process-cycle")
        );
        assert_eq!(
            hearing_command_for(owner, "voice-input.resend-last"),
            Some("resend-last")
        );
        assert_eq!(
            hearing_command_for(None, HK_CHANNEL_CYCLE),
            None,
            "没接语音输入"
        );
        assert_eq!(hearing_command_for(owner, HK_ASSISTANT), None, "本体的不转");
        assert_eq!(hearing_command_for(owner, "music.next"), None);
    }

    /// 宿主登记的每一个热键都在「设置 → 热键」那张表里、参与冲突检查（ADR 040 D2、D6）：
    /// 加了新热键忘了进表就失败。
    #[test]
    fn every_registered_hotkey_is_in_the_hotkey_table() {
        use kotone_core::hotkey_table::{bindings, PluginInfo, Sources};
        let mut settings = kotone_core::settings::Settings::default();
        // 升级后启用了她：呼叫键和提醒助手热键都在
        settings.ui.legacy_upgrade = kotone_core::settings::LegacyUpgrade::OptedIn;
        settings.assistant.companion.quick_messages = quick_messages(8);
        let manifest = kotone_plugin_voice_input::plugin::manifest();
        let plugins = [PluginInfo {
            id: &manifest.id,
            name: &manifest.name,
            enabled: true,
            commands: &manifest.commands,
        }];
        let ids: Vec<String> = bindings(&Sources {
            settings: &settings,
            profiles: &[],
            plugins: &plugins,
        })
        .into_iter()
        .map(|b| b.id)
        .collect();
        // 表里的行按命令名，状态按第 1 期的 id：对照表接得上
        for id in KNOWN_NAMED_HOTKEYS.iter().chain([&HK_RECORD]) {
            let row = legacy_by_id(id).map_or(id.to_string(), |legacy| legacy.command.to_string());
            assert!(ids.contains(&row), "{id} 不在热键表里");
        }
        // 自己加的：登记 id 就是表里的行 id
        for p in plan_from(&settings) {
            if crate::hotkey_run::is_custom(&p.id) {
                assert!(ids.contains(&p.id), "{} 不在热键表里", p.id);
            }
        }
    }
}
