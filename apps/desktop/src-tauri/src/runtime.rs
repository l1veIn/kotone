//! 语音识别运行时的壳侧实现：RuntimeManager 持有相位与启动快照，
//! start/stop 编排（warmup → 注册热键 → 显示悬浮窗 / 反向卸载），
//! 状态变化经 `kotone://runtime` 事件推送全量状态给前端。
//!
//! ADR 028：“启动”只属于本地语音识别。在线识别打开 Kotone 就自动就绪（这里自动 start），
//! 换了识别模型也自动跟上（[`after_asr_change`]）；本地识别才需要玩家点「启动」。
//! 没准备好时按语音输入热键 / 呼叫键，只提示、不吞键（[`not_ready_hint`]）。
//!
//! 迁移合法性由 core `kotone_core::runtime` 纯函数裁决；本模块只做编排。

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use tauri::{AppHandle, Emitter, Manager};

use kotone_core::interaction::effective_hotkey_mode;
use kotone_core::profile::GameProfile;
use kotone_core::runtime::{self, RuntimePhase};
use kotone_core::settings::Settings;
use kotone_core::stt::{EngineRegistry, SessionConfig};

use crate::hotkey::HotkeyManager;
use crate::{hide_window, show_window_no_focus, SharedState};

/// 启动时快照：Running 期间与当前配置比对推导 restartNeeded
#[derive(Debug, Clone)]
pub struct StartedSnapshot {
    pub engine_id: String,
    pub model_id: String,
    /// 真正用于预热共享 recognizer 的配置。Running 期间不可偷换；
    /// 当前设置与它不同时由 restartNeeded 提示重启。
    pub session_config: SessionConfig,
}

fn session_config_from(settings: &Settings, engine_id: &str) -> SessionConfig {
    let active_profile = settings
        .active_profile_id
        .as_deref()
        .and_then(kotone_core::profile::get)
        .unwrap_or_else(GameProfile::builtin_generic);
    SessionConfig {
        language: settings.language.clone(),
        hotwords: active_profile.hotwords,
        hotwords_score: settings.hotwords_score,
        options: settings.session_options(engine_id),
    }
}

/// IPC/事件用运行时状态（kotone://runtime 全量推送）
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeStatus {
    /// stopped / starting / running / stopping
    pub phase: String,
    pub restart_needed: bool,
    /// 当前配置的引擎/模型（restartNeeded 时 ≠ 运行中的快照）
    pub engine_id: Option<String>,
    pub engine_name: Option<String>,
    pub model_id: Option<String>,
    /// 交互模式预设（null = 旧字段推导）
    pub interaction_mode: Option<String>,
    /// 过渡阶段提示（warmup / hotkey / overlay / unload），稳态为 null
    pub stage: Option<String>,
    /// 当前识别模型是本地模型（只有本地识别有“启动”的概念）
    pub asr_local: bool,
    /// 最近一次启动失败的原因（再次启动、启动成功、停止或换识别模型时清掉）
    pub error: Option<String>,
}

/// 运行时管理器：相位 + 启动快照（两个锁分开，避免事件构造时长持锁）
pub struct RuntimeManager {
    phase: Mutex<RuntimePhase>,
    started: Mutex<Option<StartedSnapshot>>,
    configuration: Mutex<()>,
    input_epoch: AtomicU64,
    input_pauses: Arc<AtomicUsize>,
    /// 最近一次启动失败的原因
    last_error: Mutex<Option<String>>,
    /// 上次推送状态时的识别引擎 / 模型：变化 = 玩家换了识别模型
    last_asr: Mutex<Option<(String, String)>>,
    /// 自动启动 / 重启串行执行（换模型、在线识别自动应用设置）
    follow: Arc<tokio::sync::Mutex<()>>,
    /// 在线识别自动重启过的会话配置（同一份配置只自动重启一次，防止推导不一致时反复重启）
    auto_restarted_for: Mutex<Option<String>>,
}

/// Pause only input admission; no mutex is held while asynchronous cancellation runs.
pub struct RuntimeInputPause(Arc<AtomicUsize>);

impl Drop for RuntimeInputPause {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl RuntimeManager {
    pub fn new() -> Self {
        Self {
            phase: Mutex::new(RuntimePhase::Stopped),
            started: Mutex::new(None),
            configuration: Mutex::new(()),
            input_epoch: AtomicU64::new(0),
            input_pauses: Arc::new(AtomicUsize::new(0)),
            last_error: Mutex::new(None),
            last_asr: Mutex::new(None),
            follow: Arc::new(tokio::sync::Mutex::new(())),
            auto_restarted_for: Mutex::new(None),
        }
    }

    fn set_error(&self, error: Option<String>) {
        *self.last_error.lock().unwrap() = error;
    }

    pub fn error(&self) -> Option<String> {
        self.last_error.lock().unwrap().clone()
    }

    pub fn phase(&self) -> RuntimePhase {
        *self.phase.lock().unwrap()
    }

    /// 设置提交与启动的最后一段资源注册共用此锁；不跨异步预热持有。
    pub fn configuration_guard(&self) -> MutexGuard<'_, ()> {
        self.configuration.lock().unwrap()
    }

    pub fn input_ticket(&self) -> Option<u64> {
        let phase = self.phase.lock().unwrap();
        (*phase == RuntimePhase::Running && self.input_pauses.load(Ordering::SeqCst) == 0)
            .then(|| self.input_epoch.load(Ordering::SeqCst))
    }

    pub fn accepts_input(&self, ticket: u64) -> bool {
        self.input_ticket() == Some(ticket)
    }

    /// Discard queued input and refuse new tickets until the last pause ends.
    /// Taking the phase lock makes ticket capture atomic with this transition.
    pub fn pause_inputs(&self) -> RuntimeInputPause {
        let _phase = self.phase.lock().unwrap();
        self.input_pauses.fetch_add(1, Ordering::SeqCst);
        self.input_epoch.fetch_add(1, Ordering::SeqCst);
        RuntimeInputPause(self.input_pauses.clone())
    }

    /// 迁移相位（core 纯函数裁决；非法迁移返回 Err）
    fn transit(&self, f: fn(RuntimePhase) -> Result<RuntimePhase, String>) -> Result<(), String> {
        let mut guard = self.phase.lock().unwrap();
        *guard = f(*guard)?;
        Ok(())
    }

    fn set_started(&self, snapshot: Option<StartedSnapshot>) {
        *self.started.lock().unwrap() = snapshot;
    }

    fn started(&self) -> Option<StartedSnapshot> {
        self.started.lock().unwrap().clone()
    }

    /// 组装当前全量状态（restartNeeded 现场推导：快照 vs SharedState 配置）
    pub fn status(
        &self,
        settings: &Settings,
        engines: &EngineRegistry,
        stage: Option<String>,
    ) -> RuntimeStatus {
        let phase = self.phase();
        let engine_id = settings.stt_engine.clone();
        let model_id = kotone_stt::model::active_model_from(settings, &engine_id);
        let started = self.started();
        let engine_or_model_changed = runtime::restart_needed(
            phase,
            started
                .as_ref()
                .map(|s| (s.engine_id.as_str(), s.model_id.as_str())),
            (&engine_id, &model_id),
        );
        let session_config_changed = phase == RuntimePhase::Running
            && started.as_ref().is_some_and(|snapshot| {
                snapshot.session_config != session_config_from(settings, &engine_id)
            });
        let restart_needed = engine_or_model_changed || session_config_changed;
        let asr_local = !kotone_stt::model::is_remote_model(&model_id);
        RuntimeStatus {
            phase: phase.as_str().to_string(),
            restart_needed,
            engine_name: engines
                .get(&engine_id)
                .map(|e| e.display_name().to_string()),
            engine_id: Some(engine_id),
            model_id: Some(model_id),
            interaction_mode: settings.interaction_mode.as_ref().map(|m| {
                serde_json::to_string(m)
                    .unwrap_or_default()
                    .trim_matches('"')
                    .to_string()
            }),
            stage,
            asr_local,
            error: self.error(),
        }
    }
}

/// 当前活动识别模型是不是在线模型（在线识别没有“启动”）。
pub fn asr_is_remote(settings: &Settings) -> bool {
    kotone_stt::model::is_remote_model(&kotone_stt::model::active_model_from(
        settings,
        &settings.stt_engine,
    ))
}

/// 推送全量状态（失败仅记日志；前端启动时会主动 get_runtime_status 对齐）
pub fn emit_status(app: &AppHandle, status: &RuntimeStatus) {
    if let Err(e) = app.emit("kotone://runtime", status) {
        kotone_core::log::log(&format!("kotone://runtime 事件推送失败: {e}"));
    }
}

/// 从 AppHandle 取齐三个状态并组装 status + 推送
pub fn snapshot_and_emit(app: &AppHandle, stage: Option<String>) -> Option<RuntimeStatus> {
    let state = app.try_state::<SharedState>()?;
    let rt = app.try_state::<RuntimeManager>()?;
    let status = {
        let settings = state.settings.read().unwrap();
        rt.status(&settings, &state.engines, stage)
    };
    // 托盘「启动 / 停止本地语音识别」：只在本地识别时出现，文案随相位变（启动/停止完成都经过这里）
    crate::tray::sync(app, &status);
    // 语音唤醒跟随语音识别的启停与设置（设置保存的最后一步也经过这里）。
    crate::wake::sync(app);
    // 插件跟着设置启停（例如游戏语音的转发，ADR 020、038）。
    crate::plugins::settings_changed(app);
    emit_status(app, &status);
    // 反向通道：Agent 可订阅运行时相位变化（工具服务未开启时为空操作）
    if let Ok(value) = serde_json::to_value(&status) {
        crate::tool_service::publish(app, "runtime.phase", value);
    }
    follow_asr(app, &rt, &status);
    Some(status)
}

/// 每次推送状态时检查：识别模型换了 → 跟着启动 / 重启；在线识别改了会话设置 → 自动重启生效。
/// 只调度，不在这里等（推送点可能在启动流程或设置事务里）。
fn follow_asr(app: &AppHandle, rt: &RuntimeManager, status: &RuntimeStatus) {
    let current = (
        status.engine_id.clone().unwrap_or_default(),
        status.model_id.clone().unwrap_or_default(),
    );
    let previous = {
        let mut last = rt.last_asr.lock().unwrap();
        let previous = last.clone();
        *last = Some(current.clone());
        previous
    };
    match previous {
        Some(previous) if previous != current => {
            rt.set_error(None);
            let previous_was_remote = kotone_stt::model::is_remote_model(&previous.1);
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                after_asr_change(&app, previous_was_remote).await;
            });
        }
        Some(_)
            if !status.asr_local
                && status.phase == "running"
                && status.restart_needed
                && status.stage.is_none() =>
        {
            // 在线识别没有「重启生效」按钮：热词、语言等会话设置变了就自动重启一次。
            let signature = {
                let state = app.state::<SharedState>();
                let settings = state.settings.read().unwrap();
                format!(
                    "{}|{}|{:?}",
                    current.0,
                    current.1,
                    session_config_from(&settings, &current.0)
                )
            };
            {
                let mut done = rt.auto_restarted_for.lock().unwrap();
                if done.as_deref() == Some(signature.as_str()) {
                    return;
                }
                *done = Some(signature);
            }
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                let follow = app.state::<RuntimeManager>().follow.clone();
                let _serial = follow.lock().await;
                let still = {
                    let state = app.state::<SharedState>();
                    let rt = app.state::<RuntimeManager>();
                    let settings = state.settings.read().unwrap();
                    asr_is_remote(&settings)
                        && rt.phase() == RuntimePhase::Running
                        && rt.status(&settings, &state.engines, None).restart_needed
                };
                if still {
                    if let Err(e) = start(&app).await {
                        kotone_core::log::log(&format!("在线识别自动应用设置失败: {e}"));
                    }
                }
            });
        }
        _ => {}
    }
}

/// 等运行时离开过渡态（最多约 30 秒），返回稳定后的相位。
async fn settled_phase(app: &AppHandle) -> RuntimePhase {
    for _ in 0..150 {
        let phase = app.state::<RuntimeManager>().phase();
        if matches!(phase, RuntimePhase::Stopped | RuntimePhase::Running) {
            return phase;
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    app.state::<RuntimeManager>().phase()
}

/// 换了识别模型 / 引擎之后（设置页、工具、settings.update 等所有入口都经 snapshot_and_emit 到这里）：
/// - 新模型在线：直接就绪（停着就启动，运行中按重启语义换过去）；
/// - 从在线换成本地：立即启动一次，热键不会突然失灵（D9）；
/// - 本地换本地：运行中保持「重启生效」提示，停着不动。
pub async fn after_asr_change(app: &AppHandle, previous_was_remote: bool) {
    let follow = app.state::<RuntimeManager>().follow.clone();
    let _serial = follow.lock().await;
    let phase = settled_phase(app).await;
    let (now_remote, restart_needed) = {
        let state = app.state::<SharedState>();
        let settings = state.settings.read().unwrap();
        (
            asr_is_remote(&settings),
            app.state::<RuntimeManager>()
                .status(&settings, &state.engines, None)
                .restart_needed,
        )
    };
    if !(now_remote || previous_was_remote) {
        return;
    }
    if phase == RuntimePhase::Running && !restart_needed {
        return;
    }
    kotone_core::log::log(&format!(
        "识别模型已切换（{}），自动{}",
        if now_remote { "在线" } else { "本地" },
        if phase == RuntimePhase::Running {
            "重启"
        } else {
            "启动"
        }
    ));
    if let Err(e) = start(app).await {
        kotone_core::log::log(&format!("切换识别模型后自动启动失败: {e}"));
    }
}

/// 启动时（lib.rs setup）：在线识别打开即就绪；本地识别看「打开时自动启动」。
pub fn should_start_on_launch(settings: &Settings) -> bool {
    settings.ui.auto_start || asr_is_remote(settings)
}

/// 没准备好时按下的是哪个键。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HintKind {
    /// 语音输入热键：只在悬浮窗提示
    VoiceInput,
    /// 呼叫键：只在气泡提示
    Call,
}

/// 语音识别没准备好时的提示文案：(标题, 说明)。Running 返回 None。
pub fn not_ready_text(
    phase: RuntimePhase,
    remote: bool,
    error: Option<&str>,
    toggle_hotkey: &str,
) -> Option<(String, String)> {
    let what = if remote {
        "语音识别"
    } else {
        "本地语音识别"
    };
    match phase {
        RuntimePhase::Running => None,
        RuntimePhase::Starting => Some((format!("{what}正在准备…"), "准备好就能用了".into())),
        RuntimePhase::Stopping => Some((format!("{what}正在停止"), "稍后再试".into())),
        RuntimePhase::Stopped => {
            if let Some(error) = error.map(str::trim).filter(|e| !e.is_empty()) {
                let mut short: String = error.chars().take(60).collect();
                if error.chars().count() > 60 {
                    short.push('…');
                }
                return Some((format!("{what}出问题了"), short));
            }
            if remote {
                return Some((
                    "语音识别还没准备好".into(),
                    "打开 Kotone，看看「语音输入」页".into(),
                ));
            }
            let toggle = toggle_hotkey.trim();
            let how = if toggle.is_empty() {
                "打开 Kotone，点「启动」".to_string()
            } else {
                format!("按 {toggle} 启动，或在 Kotone 里点「启动」")
            };
            Some(("本地语音识别未启动".into(), how))
        }
    }
}

/// 被动提示热键触发（只观察、不吞键）：语音输入热键 → 悬浮窗；呼叫键 → 气泡。
/// 两者不混用：气泡关着时呼叫键就不提示。
pub fn not_ready_hint(app: &AppHandle, kind: HintKind) {
    let (Some(rt), Some(state)) = (
        app.try_state::<RuntimeManager>(),
        app.try_state::<SharedState>(),
    ) else {
        return;
    };
    let (remote, toggle) = {
        let settings = state.settings.read().unwrap();
        (
            asr_is_remote(&settings),
            settings.runtime_toggle_hotkey.clone(),
        )
    };
    let Some((title, detail)) = not_ready_text(rt.phase(), remote, rt.error().as_deref(), &toggle)
    else {
        return;
    };
    kotone_core::log::log(&format!("not-ready hint ({kind:?}): {title}"));
    match kind {
        HintKind::VoiceInput => crate::overlay::show_hint(app, &title, &detail),
        HintKind::Call => crate::bubble::hint(app, &title, &detail),
    }
}

/// runtime 不在 Running 时盯住语音输入热键 / 呼叫键（被动、不吞键）；Running 时撤掉。
/// 在配置锁内检查相位，避免与 finish_start 的热键注册交错。
pub fn refresh_hints(app: &AppHandle) {
    let (Some(rt), Some(state), Some(mgr)) = (
        app.try_state::<RuntimeManager>(),
        app.try_state::<SharedState>(),
        app.try_state::<HotkeyManager>(),
    ) else {
        return;
    };
    let _configuration = rt.configuration_guard();
    let settings = state.settings.read().unwrap().clone();
    mgr.set_runtime_hints(&settings, rt.phase() != RuntimePhase::Running);
}

/// 热键触发的启动 / 停止切换（「启动 / 停止 Kotone」热键）。
/// 语义与标题栏 / Hero 的主按钮一致：Stopped → start；Running → stop；
/// Running + restartNeeded → start（壳内做 stop + start 重启）；过渡态忽略，
/// 避免热键连按把状态机推到非法相位。
pub async fn toggle(app: &AppHandle) -> Result<RuntimeStatus, String> {
    let rt = app.state::<RuntimeManager>();
    {
        let state = app.state::<SharedState>();
        if asr_is_remote(&state.settings.read().unwrap()) {
            return Err("在线语音识别打开就能用，不需要启动或停止".into());
        }
    }
    match rt.phase() {
        RuntimePhase::Stopped => start(app).await,
        RuntimePhase::Running => {
            let state = app.state::<SharedState>();
            let restart_needed = {
                let settings = state.settings.read().unwrap();
                rt.status(&settings, &state.engines, None).restart_needed
            };
            if restart_needed {
                start(app).await // 与按钮一致：重启生效
            } else {
                stop(app).await
            }
        }
        p => Err(format!("运行时正在{}，请稍候", p.as_str())),
    }
}

/// 启动：warmup 当前引擎 → 注册热键 → 显示悬浮窗。
/// Running + restartNeeded 时等价于 stop + start（用户显式点「重启」走这里）。
pub async fn start(app: &AppHandle) -> Result<RuntimeStatus, String> {
    crate::record_process_event(
        app,
        &serde_json::json!({
            "caseId": kotone_core::process_log::app_session_id(),
            "activity": "runtime_start_requested",
            "data": {}
        }),
    );
    let rt = app.state::<RuntimeManager>();
    // Running + restartNeeded → 先停再启（restart 语义）；Running 且无变更 → 幂等返回
    match rt.phase() {
        RuntimePhase::Running => {
            let state = app.state::<SharedState>();
            let restart_needed = {
                let settings = state.settings.read().unwrap();
                rt.status(&settings, &state.engines, None).restart_needed
            };
            if restart_needed {
                stop(app).await?;
            } else {
                return snapshot_and_emit(app, None)
                    .ok_or_else(|| "运行时状态未初始化".to_string());
            }
        }
        p if runtime::can_start(p) => {}
        p => {
            return Err(format!(
                "运行时正在{}，请稍候",
                if p == RuntimePhase::Starting {
                    "启动"
                } else {
                    "停止"
                }
            ))
        }
    }

    rt.transit(runtime::begin_start)?;
    rt.set_error(None);
    snapshot_and_emit(app, Some("warmup".into()));

    // 每个失败分支都要回滚相位；用闭包包住主流程统一处理
    let result = start_inner(app).await;
    if let Err(ref e) = result {
        rt.set_error(Some(e.clone()));
        kotone_core::log::log(&format!("runtime start failed: {e}"));
        crate::record_process_event(
            app,
            &serde_json::json!({
                "caseId": kotone_core::process_log::app_session_id(),
                "activity": "runtime_start_failed",
                "data": { "outcome": "error", "errorCode": "RUNTIME_START_FAILED" }
            }),
        );
        let _ = rt.transit(runtime::fail_start);
        // 热键注册可能已被拆掉：没准备好的提示重新盯上
        refresh_hints(app);
    }
    let status = snapshot_and_emit(app, None);
    match (result, status) {
        (Ok(()), Some(s)) => Ok(s),
        (Ok(()), None) => Err("运行时状态未初始化".into()),
        (Err(e), _) => Err(e),
    }
}

async fn start_inner(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<SharedState>();

    let (settings_snapshot, engine_id, model_id, session_config) = {
        let settings = state.settings.read().unwrap();
        let engine_id = settings.stt_engine.clone();
        (
            settings.clone(),
            engine_id.clone(),
            kotone_stt::model::active_model_from(&settings, &engine_id),
            session_config_from(&settings, &engine_id),
        )
    };
    // 兼容旧配置或手工修改 config.json 的场景：在进入任何 sherpa 原生代码前
    // 拒绝非 ASCII 模型路径，避免词表读取失败后直接终止整个桌面进程。
    kotone_stt::model::validate_models_dir_path(&kotone_stt::model::models_dir_from(
        &settings_snapshot,
    ))?;
    if state.engines.get(&engine_id).is_none() {
        return Err(format!(
            "未注册的 STT 引擎: {engine_id}（请在「设置 → 模型 → 语音识别」重新选择）"
        ));
    }

    // 就绪预检（warmup 前的快速布尔检查）：失败时先记录明细（缺哪个文件）并做
    // 短重试，吸收杀软扫描/文件锁造成的瞬时误判——0.1.5 用户遇到运行中
    // stop/start 后误报「模型未下载」，被迫重新下载，布尔检查无法定位原因。
    let engine_ready = || {
        state
            .engines
            .get(&engine_id)
            .map(|e| e.is_ready())
            .unwrap_or(false)
    };
    if !engine_ready() {
        if kotone_stt::model::is_remote_model(&model_id) {
            return Err(format!(
                "在线模型「{model_id}」未就绪：请在「设置 → 模型 → 语音识别」填好它要的 Key（DeepSeek 用 DeepSeek 连接的 Key）后重试"
            ));
        }
        let missing = kotone_stt::model::multi_model_missing(&model_id);
        kotone_core::log::log(&format!(
            "runtime start: 模型就绪检查未通过（{model_id}）：{}；重试中",
            missing.join("、")
        ));
        let mut ready = false;
        for _ in 0..3 {
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            if engine_ready() {
                ready = true;
                break;
            }
        }
        if !ready {
            let missing = kotone_stt::model::multi_model_missing(&model_id);
            return Err(format!(
                "模型「{model_id}」文件不齐备：{}。请在「设置 → 模型 → 语音识别」重新下载",
                missing.join("、")
            ));
        }
        kotone_core::log::log("runtime start: 模型就绪检查重试后通过（前次为瞬时误判）");
    }

    // 阶段 1：warmup（模型入内存；sherpa 百毫秒级，放阻塞线程不卡 UI）
    crate::record_process_event(
        app,
        &serde_json::json!({
            "caseId": kotone_core::process_log::app_session_id(),
            "activity": "model_warmup_started",
            "data": {}
        }),
    );
    let warmup_started = std::time::Instant::now();
    let engines: Arc<EngineRegistry> = state.engines.clone();
    let warm_engine = engine_id.clone();
    let warm_config = session_config.clone();
    tauri::async_runtime::spawn_blocking(move || {
        engines
            .get(&warm_engine)
            .ok_or_else(|| format!("未注册的 STT 引擎: {warm_engine}"))?
            .warmup(&warm_config)
    })
    .await
    .map_err(|e| format!("warmup 任务异常：{e}"))??;
    kotone_core::log::log(&format!("runtime warmup ok: {engine_id} ({model_id})"));
    crate::record_process_event(
        app,
        &serde_json::json!({
            "caseId": kotone_core::process_log::app_session_id(),
            "activity": "model_ready",
            "data": {
                "outcome": "success",
                "durationMs": warmup_started.elapsed().as_millis() as u64
            }
        }),
    );

    let result = finish_start(
        app,
        settings_snapshot,
        engine_id.clone(),
        model_id,
        session_config,
    );
    if result.is_err() {
        let engines = state.engines.clone();
        let _ = tauri::async_runtime::spawn_blocking(move || {
            if let Some(engine) = engines.get(&engine_id) {
                engine.unload();
            }
        })
        .await;
    }
    result
}

/// 此段无 await：与配置事务串行提交最新热键及运行态，避免注册旧的启动快照。
fn finish_start(
    app: &AppHandle,
    settings_snapshot: Settings,
    engine_id: String,
    model_id: String,
    session_config: SessionConfig,
) -> Result<(), String> {
    let state = app.state::<SharedState>();
    let rt = app.state::<RuntimeManager>();
    let _configuration = rt.configuration_guard();
    let current_settings = state.settings.read().unwrap().clone();
    // 阶段 2：注册全局热键（失败回滚：卸载引擎，不半截启动）
    snapshot_and_emit(app, Some("hotkey".into()));
    crate::record_process_event(
        app,
        &serde_json::json!({
            "caseId": kotone_core::process_log::app_session_id(),
            "activity": "hotkey_register_started",
            "data": {}
        }),
    );
    // 注册热键前先锁定已预热快照，消除「热键已可用但 begin 读到
    // 设置页新值」的竞态窗口。注册失败时与引擎一起回滚。
    state
        .orchestrator
        .activate_runtime_settings(settings_snapshot);
    let mgr = app.state::<HotkeyManager>();
    if let Err(e) = mgr.register_with_settings(
        app,
        &current_settings.hotkey.key,
        effective_hotkey_mode(&current_settings),
        &current_settings,
    ) {
        state.orchestrator.deactivate_runtime_settings();
        return Err(e);
    }

    // 阶段 3：显示悬浮窗（不抢焦点；会话态显隐仍由 TauriEmitter 驱动）。
    // on_demand（用时浮现）与 never（完全隐藏）档位启动时不显示；前者在
    // 收音/转写时由 TauriEmitter 浮现，后者始终保持隐藏；always 启动即显示。
    // 每次启动重新应用尺寸、固定/自定义位置与点击穿透（显示器/DPI 可能已变）。
    snapshot_and_emit(app, Some("overlay".into()));
    let overlay_config = {
        let g = state.settings.read().unwrap();
        g.overlay.clone()
    };
    if let Some(win) = app.get_webview_window("overlay") {
        crate::apply_overlay_window_config(&win, &overlay_config);
    }
    // overlay WebView 启动时可能早于前端读取配置；运行态启动时重发一次全量配置，
    // 保证样式、拖动和点击穿透与后端已应用的窗口几何保持一致。
    let _ = app.emit("kotone://overlay-config", &overlay_config);
    if let Some(win) = app.get_webview_window("overlay") {
        match overlay_config.visibility {
            kotone_core::settings::OverlayVisibility::Always
                if kotone_platform_windows::fullscreen::is_exclusive_fullscreen_active()
                    == Some(true) =>
            {
                // 启动时游戏已在独占全屏：不激活置顶浮窗，否则可能
                // 直接将游戏最小化。设置页会持久记住该提示。
                hide_window(&win);
                let _ = app.emit(
                    "kotone://fullscreen-warning",
                    serde_json::json!({ "exclusiveFullscreen": true }),
                );
            }
            kotone_core::settings::OverlayVisibility::Always => show_window_no_focus(&win),
            kotone_core::settings::OverlayVisibility::OnDemand
            | kotone_core::settings::OverlayVisibility::Never => hide_window(&win),
        }
    }

    rt.set_started(Some(StartedSnapshot {
        engine_id,
        model_id,
        session_config,
    }));
    rt.input_epoch.fetch_add(1, Ordering::SeqCst);
    rt.transit(runtime::finish_start)?;
    // Creation is optional and asynchronous; reconcile again now that Always
    // can observe Running even if its WebView was not ready during stage 3.
    crate::overlay::reconcile(app, false);
    kotone_core::log::log("runtime started");
    crate::record_process_event(
        app,
        &serde_json::json!({
            "caseId": kotone_core::process_log::app_session_id(),
            "activity": "runtime_ready",
            "data": { "outcome": "success" }
        }),
    );
    Ok(())
}

/// 停止：取消进行中的会话 → 注销热键 → 隐藏悬浮窗 → 卸载引擎。
/// Stopped 时幂等返回（不报错）。
pub async fn stop(app: &AppHandle) -> Result<RuntimeStatus, String> {
    let rt = app.state::<RuntimeManager>();
    match rt.phase() {
        RuntimePhase::Stopped => {
            return snapshot_and_emit(app, None).ok_or_else(|| "运行时状态未初始化".to_string())
        }
        p if runtime::can_stop(p) => {}
        p => {
            return Err(format!(
                "运行时正在{}，请稍候",
                if p == RuntimePhase::Starting {
                    "启动"
                } else {
                    "停止"
                }
            ))
        }
    }
    let state = app.state::<SharedState>();

    {
        let _configuration = rt.configuration_guard();
        rt.transit(runtime::begin_stop)?;
        rt.input_epoch.fetch_add(1, Ordering::SeqCst);
        // 在首次 await 前关闭 admission 并注销来源；已排队的异步热键凭票据失效。
        state.orchestrator.suspend_runtime_inputs();
        app.state::<HotkeyManager>().unregister(app)?;
    }
    snapshot_and_emit(app, Some("unload".into()));

    // 进行中的会话先取消（orchestrator 是幂等的：Idle 时 cancel 无效果）
    state.orchestrator.cancel().await;

    // 隐藏悬浮窗（原始 SW_HIDE 路径——Tauri hide() 与我们的 SW_SHOWNA 显示
    // 不对称，会因 tao 可见性缓存 diff 为空被短路，详见 lib.rs hide_window）
    if let Some(win) = app.get_webview_window("overlay") {
        hide_window(&win);
    }

    // 卸载引擎（释放模型内存）
    let engines = state.engines.clone();
    // 设置页可在 Running 期间先选中另一引擎。停止时必须卸载
    // 真正已预热的快照引擎，不能卸载尚未启动的「当前选择」。
    let engine_id = rt
        .started()
        .map(|snapshot| snapshot.engine_id)
        .unwrap_or_else(|| state.settings.read().unwrap().stt_engine.clone());
    let _ = tauri::async_runtime::spawn_blocking(move || {
        if let Some(en) = engines.get(&engine_id) {
            en.unload();
        }
    })
    .await;

    state.orchestrator.deactivate_runtime_settings();
    rt.set_started(None);
    rt.set_error(None);
    rt.transit(runtime::finish_stop)?;
    // 停下以后：按语音输入热键 / 呼叫键只提示怎么启动，键照常传给游戏（D8）
    refresh_hints(app);
    kotone_core::log::log("runtime stopped");
    crate::record_process_event(
        app,
        &serde_json::json!({
            "caseId": kotone_core::process_log::app_session_id(),
            "activity": "runtime_stopped",
            "data": { "outcome": "success" }
        }),
    );
    snapshot_and_emit(app, None).ok_or_else(|| "运行时状态未初始化".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn running() -> RuntimeManager {
        let runtime = RuntimeManager::new();
        *runtime.phase.lock().unwrap() = RuntimePhase::Running;
        runtime
    }

    #[test]
    fn reconfiguration_discards_queued_input_and_resumes_after_rollback() {
        let runtime = running();
        let queued = runtime.input_ticket().unwrap();
        let reconfigure = || -> Result<(), ()> {
            let _pause = runtime.pause_inputs();
            assert!(runtime.input_ticket().is_none());
            assert!(!runtime.accepts_input(queued));
            Err(()) // A failed settings transaction still releases its pause.
        };
        assert!(reconfigure().is_err());
        assert!(!runtime.accepts_input(queued));
        let fresh = runtime.input_ticket().unwrap();
        assert!(runtime.accepts_input(fresh));
    }

    #[tokio::test]
    async fn cancellation_pause_rejects_old_and_new_edges_until_completion() {
        let runtime = running();
        let queued = runtime.input_ticket().unwrap();
        let pause = runtime.pause_inputs();
        let (done, finished) = tokio::sync::oneshot::channel::<()>();
        let task = tokio::spawn(async move {
            let _pause = pause;
            let _ = finished.await;
        });
        assert!(!runtime.accepts_input(queued));
        assert!(runtime.input_ticket().is_none());
        done.send(()).unwrap();
        task.await.unwrap();
        assert!(!runtime.accepts_input(queued));
        assert!(runtime.input_ticket().is_some());
    }

    #[test]
    fn not_ready_hint_text_follows_phase_error_and_toggle_hotkey() {
        assert_eq!(not_ready_text(RuntimePhase::Running, false, None, ""), None);
        assert_eq!(
            not_ready_text(RuntimePhase::Stopped, false, None, "Ctrl+Alt+K"),
            Some((
                "本地语音识别未启动".into(),
                "按 Ctrl+Alt+K 启动，或在 Kotone 里点「启动」".into()
            ))
        );
        assert_eq!(
            not_ready_text(RuntimePhase::Stopped, false, None, " ")
                .unwrap()
                .1,
            "打开 Kotone，点「启动」"
        );
        assert_eq!(
            not_ready_text(RuntimePhase::Starting, false, None, "")
                .unwrap()
                .0,
            "本地语音识别正在准备…"
        );
        assert_eq!(
            not_ready_text(RuntimePhase::Starting, true, None, "")
                .unwrap()
                .0,
            "语音识别正在准备…"
        );
        let (title, detail) =
            not_ready_text(RuntimePhase::Stopped, false, Some("模型文件不齐备"), "F10").unwrap();
        assert_eq!(title, "本地语音识别出问题了");
        assert_eq!(detail, "模型文件不齐备");
        for (title, detail) in [
            not_ready_text(RuntimePhase::Stopped, false, None, "").unwrap(),
            not_ready_text(RuntimePhase::Stopped, true, None, "").unwrap(),
        ] {
            assert!(!title.contains("右上角") && !detail.contains("右上角"));
        }
    }

    #[test]
    fn launch_starts_online_recognition_or_local_with_auto_start() {
        let mut settings = Settings::default();
        assert!(
            !should_start_on_launch(&settings),
            "默认是本地识别，且没开自动启动"
        );
        settings.ui.auto_start = true;
        assert!(should_start_on_launch(&settings));
        settings.ui.auto_start = false;
        kotone_stt::model::set_active_in(&mut settings, "deepseek-asr", "deepseek-asr").unwrap();
        assert!(asr_is_remote(&settings));
        assert!(should_start_on_launch(&settings), "在线识别打开就能用");
    }

    #[test]
    fn overlapping_pauses_never_reopen_input_early() {
        let runtime = running();
        let cancel = runtime.pause_inputs();
        let settings = runtime.pause_inputs();
        drop(cancel);
        assert!(runtime.input_ticket().is_none());
        drop(settings);
        assert!(runtime.input_ticket().is_some());
        let pause = runtime.pause_inputs();
        *runtime.phase.lock().unwrap() = RuntimePhase::Stopped;
        drop(pause);
        assert!(runtime.input_ticket().is_none());
    }
}
