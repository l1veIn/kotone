//! 语音输入的启用状态独立于模型是否已载入。先注册热键，再在后台预热小模型。
//! 大模型在第一次录音时加载，由编排器在闲置时释放。
//!
//! 迁移合法性由 core `kotone_core::runtime` 纯函数裁决；本模块只做编排。

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager};

use kotone_core::interaction::effective_hotkey_mode;
use kotone_core::profile::GameProfile;
use kotone_core::runtime::{self, RuntimePhase};
use kotone_core::settings::Settings;
use kotone_core::stt::{EngineRegistry, SessionConfig};

use crate::hotkey::HotkeyManager;
use crate::{hide_window, show_window_no_focus, SharedState};

const LARGE_MODEL_BYTES: u64 = 200 * 1024 * 1024;
const LARGE_MODEL_IDLE_TIMEOUT: Duration = Duration::from_secs(5 * 60);

#[derive(Debug, PartialEq, Eq)]
enum ModelLoading {
    Online,
    Background,
    OnDemand,
}

fn model_loading(model_id: &str) -> ModelLoading {
    if kotone_stt::model::is_remote_model(model_id) {
        ModelLoading::Online
    } else if kotone_stt::model::model_size_bytes(model_id)
        .is_some_and(|bytes| bytes >= LARGE_MODEL_BYTES)
    {
        ModelLoading::OnDemand
    } else {
        ModelLoading::Background
    }
}

fn automatically_enabled(settings: &Settings) -> bool {
    if settings.ui.voice_input_paused {
        return false;
    }
    // 已开始配置的旧安装沿用原模型；首次安装继续由引导设置录音热键。
    settings.ui.first_run_completed
        || settings.ui.auto_start
        || !settings.active_model_id.trim().is_empty()
        || settings
            .engine_options
            .get(&settings.stt_engine)
            .and_then(|options| options.get("model"))
            .and_then(|model| model.as_str())
            .is_some_and(|model| !model.trim().is_empty())
}

/// 启动时快照：Running 期间与当前配置比对推导 restartNeeded
#[derive(Debug, Clone)]
pub struct StartedSnapshot {
    pub engine_id: String,
    pub model_id: String,
    /// 真正用于预热共享 recognizer 的配置。Running 期间不可偷换；
    /// 当前设置与它不同时由 restartNeeded 提示重启。
    pub session_config: SessionConfig,
}

fn session_config_from(settings: &Settings, engine_id: &str, model_id: &str) -> SessionConfig {
    let active_profile = settings
        .active_profile_id
        .as_deref()
        .and_then(kotone_core::profile::get)
        .unwrap_or_else(GameProfile::builtin_generic);
    let options = if settings.active_model_id == model_id {
        settings.session_options(engine_id)
    } else {
        // 老配置可由默认文件或引擎选项推导活动模型。锁定结果，避免识别线程
        // 回读刚变更的磁盘设置，同时让对应 modelConfigs 一并进入快照。
        let mut resolved = settings.clone();
        resolved.active_model_id = model_id.into();
        resolved.session_options(engine_id)
    };
    SessionConfig {
        language: settings.language.clone(),
        hotwords: active_profile.hotwords,
        hotwords_score: settings.hotwords_score,
        options,
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
    pub voice_input_paused: bool,
    pub error: Option<String>,
}

/// 运行时管理器：相位 + 启动快照（两个锁分开，避免事件构造时长持锁）
pub struct RuntimeManager {
    phase: Mutex<RuntimePhase>,
    started: Mutex<Option<StartedSnapshot>>,
    configuration: Mutex<()>,
    operation: tokio::sync::Mutex<()>,
    input_epoch: AtomicU64,
    input_pauses: Arc<AtomicUsize>,
    model_epoch: AtomicU64,
    warming: AtomicBool,
    error: Mutex<Option<String>>,
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
            operation: tokio::sync::Mutex::new(()),
            input_epoch: AtomicU64::new(0),
            input_pauses: Arc::new(AtomicUsize::new(0)),
            model_epoch: AtomicU64::new(0),
            warming: AtomicBool::new(false),
            error: Mutex::new(None),
        }
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
                snapshot.session_config != session_config_from(settings, &engine_id, &model_id)
            });
        let restart_needed = engine_or_model_changed || session_config_changed;
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
            stage: stage.or_else(|| {
                (phase == RuntimePhase::Running && self.warming.load(Ordering::SeqCst))
                    .then(|| "warmup".into())
            }),
            voice_input_paused: settings.ui.voice_input_paused,
            error: self.error.lock().unwrap().clone(),
        }
    }

    fn next_model_epoch(&self) -> u64 {
        self.warming.store(false, Ordering::SeqCst);
        *self.error.lock().unwrap() = None;
        self.model_epoch.fetch_add(1, Ordering::SeqCst) + 1
    }
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
    // 托盘与设置页从同一状态更新。
    crate::tray::sync_toggle_label(app, rt.phase());
    emit_status(app, &status);
    Some(status)
}

fn remember_pause(app: &AppHandle, paused: bool) -> Result<(), String> {
    let state = app.state::<SharedState>();
    let rt = app.state::<RuntimeManager>();
    let _configuration = rt.configuration_guard();
    if state.settings.read().unwrap().ui.voice_input_paused != paused {
        state.settings_repository.update(|settings| {
            settings.ui.voice_input_paused = paused;
            Ok(())
        })?;
    }
    Ok(())
}

/// 托盘和旧版启停热键共用的暂停 / 恢复操作。
pub async fn toggle(app: &AppHandle) -> Result<RuntimeStatus, String> {
    let rt = app.state::<RuntimeManager>();
    let Ok(_operation) = rt.operation.try_lock() else {
        return Err("语音输入正在调整，请稍候".into());
    };
    match rt.phase() {
        RuntimePhase::Stopped => {
            remember_pause(app, false)?;
            start_enabled(app).await
        }
        RuntimePhase::Running => {
            remember_pause(app, true)?;
            stop_enabled(app).await
        }
        _ => Err("语音输入正在调整，请稍候".into()),
    }
}

/// 供首启引导、恢复输入及显式重试使用。
pub async fn start(app: &AppHandle) -> Result<RuntimeStatus, String> {
    let rt = app.state::<RuntimeManager>();
    let _operation = rt.operation.lock().await;
    remember_pause(app, false)?;
    if rt.phase() == RuntimePhase::Running {
        refresh_running_model(app)?;
        return snapshot_and_emit(app, None).ok_or_else(|| "运行时状态未初始化".into());
    }
    start_enabled(app).await
}

/// 自动启动和配置变更共用同一路径。正在录音时继续用本轮配置，
/// 会话回到空闲后再次提交所选模型。
pub fn reconcile(app: &AppHandle) {
    let Some(rt) = app.try_state::<RuntimeManager>() else {
        return;
    };
    let Some(state) = app.try_state::<SharedState>() else {
        return;
    };
    let needs_work = {
        let settings = state.settings.read().unwrap();
        match rt.phase() {
            RuntimePhase::Running => {
                settings.ui.voice_input_paused
                    || rt.status(&settings, &state.engines, None).restart_needed
            }
            _ => automatically_enabled(&settings),
        }
    };
    if !needs_work {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        reconcile_current(&app).await;
    });
}

/// 模型选择成功返回时，让下一次录音已经能使用新选择。若有会话，
/// 热键入场锁会延迟模型切换，直到前一轮结束。
pub async fn reconcile_current(app: &AppHandle) {
    let rt = app.state::<RuntimeManager>();
    let _operation = rt.operation.lock().await;
    let state = app.state::<SharedState>();
    let (enabled, paused) = {
        let settings = state.settings.read().unwrap();
        (
            automatically_enabled(&settings),
            settings.ui.voice_input_paused,
        )
    };
    let result = match rt.phase() {
        RuntimePhase::Stopped if enabled => start_enabled(app).await.map(|_| ()),
        RuntimePhase::Running if paused => stop_enabled(app).await.map(|_| ()),
        RuntimePhase::Running => refresh_running_model(app),
        _ => Ok(()),
    };
    if let Err(error) = result {
        kotone_core::log::log(&format!("runtime reconcile: {error}"));
    }
}

async fn start_enabled(app: &AppHandle) -> Result<RuntimeStatus, String> {
    crate::record_process_event(
        app,
        &serde_json::json!({
            "caseId": kotone_core::process_log::app_session_id(),
            "activity": "runtime_start_requested",
            "data": {}
        }),
    );
    let rt = app.state::<RuntimeManager>();
    rt.next_model_epoch();
    rt.transit(runtime::begin_start)?;
    snapshot_and_emit(app, None);

    // 每个失败分支都要回滚相位；用闭包包住主流程统一处理
    let result = start_inner(app).await;
    if let Err(ref e) = result {
        *rt.error.lock().unwrap() = Some(e.clone());
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
        let model_id = kotone_stt::model::active_model_from(&settings, &engine_id);
        let session_config = session_config_from(&settings, &engine_id, &model_id);
        (settings.clone(), engine_id, model_id, session_config)
    };
    // 就绪预检（warmup 前的快速布尔检查）：失败时先记录明细（缺哪个文件）并做
    // 短重试，吸收杀软扫描/文件锁造成的瞬时误判——0.1.5 用户遇到运行中
    // stop/start 后误报「模型未下载」，被迫重新下载，布尔检查无法定位原因。
    if let Err(error) = validate_engine(&settings_snapshot, &state.engines, &engine_id, &model_id) {
        if model_loading(&model_id) == ModelLoading::Online {
            return Err(error);
        }
        kotone_core::log::log(&format!("runtime model preflight: {error}"));
        for _ in 0..3 {
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            if validate_engine(&settings_snapshot, &state.engines, &engine_id, &model_id).is_ok() {
                break;
            }
        }
        validate_engine(&settings_snapshot, &state.engines, &engine_id, &model_id)?;
    }

    finish_start(app, engine_id, model_id, session_config)
}

fn validate_engine(
    settings: &Settings,
    engines: &EngineRegistry,
    engine_id: &str,
    model_id: &str,
) -> Result<(), String> {
    let loading = model_loading(model_id);
    if loading != ModelLoading::Online {
        // 原生词表读取不支持非 ASCII 路径。
        kotone_stt::model::validate_models_dir_path(&kotone_stt::model::models_dir_from(settings))?;
    }
    let engine = engines
        .get(engine_id)
        .ok_or_else(|| format!("未注册的语音识别引擎：{engine_id}"))?;
    if engine.is_ready() {
        return Ok(());
    }
    if loading == ModelLoading::Online {
        return Err(format!(
            "在线模型「{model_id}」未就绪，请填写 API 凭据后重试"
        ));
    }
    let missing = kotone_stt::model::multi_model_missing(model_id).join("、");
    Err(format!(
        "模型「{model_id}」文件不齐备：{missing}。请在「设置 → 高级」重新下载"
    ))
}

/// 仅在热键已可用后执行。提前捕获模型代际，避免暂停或切模型后写回旧结果。
fn prewarm(app: &AppHandle, started: &StartedSnapshot) {
    if model_loading(&started.model_id) != ModelLoading::Background {
        return;
    }
    let rt = app.state::<RuntimeManager>();
    let state = app.state::<SharedState>();
    let epoch = rt.model_epoch.load(Ordering::SeqCst);
    let loading = state
        .orchestrator
        .prewarm_runtime_engine(started.session_config.clone());
    rt.warming.store(true, Ordering::SeqCst);
    let app = app.clone();
    let engine_id = started.engine_id.clone();
    let model_id = started.model_id.clone();
    tauri::async_runtime::spawn(async move {
        let warmup_started = std::time::Instant::now();
        crate::record_process_event(
            &app,
            &serde_json::json!({
                "caseId": kotone_core::process_log::app_session_id(),
                "activity": "model_warmup_started",
                "data": {}
            }),
        );
        let result = loading.await;
        let rt = app.state::<RuntimeManager>();
        let _operation = rt.operation.lock().await;
        if rt.model_epoch.load(Ordering::SeqCst) != epoch {
            return;
        }
        rt.warming.store(false, Ordering::SeqCst);
        match result {
            Ok(()) => {
                kotone_core::log::log(&format!("runtime warmup ok: {engine_id} ({model_id})"));
                crate::record_process_event(
                    &app,
                    &serde_json::json!({
                        "caseId": kotone_core::process_log::app_session_id(),
                        "activity": "model_ready",
                        "data": {
                            "outcome": "success",
                            "durationMs": warmup_started.elapsed().as_millis() as u64
                        }
                    }),
                );
            }
            Err(error) => {
                kotone_core::log::log(&format!("runtime warmup failed: {error}"));
                *rt.error.lock().unwrap() = Some(error);
            }
        }
        snapshot_and_emit(&app, None);
    });
}

fn idle_timeout(model_id: &str) -> Option<Duration> {
    (model_loading(model_id) == ModelLoading::OnDemand).then_some(LARGE_MODEL_IDLE_TIMEOUT)
}

/// 热键和本轮模型快照串行提交。之后的小模型预热不会拦住录音入口。
fn finish_start(
    app: &AppHandle,
    engine_id: String,
    model_id: String,
    session_config: SessionConfig,
) -> Result<(), String> {
    let state = app.state::<SharedState>();
    let rt = app.state::<RuntimeManager>();
    let _configuration = rt.configuration_guard();
    let mut current_settings = state.settings.read().unwrap().clone();
    if current_settings.ui.voice_input_paused {
        return Err("语音输入已暂停".into());
    }
    let current_model =
        kotone_stt::model::active_model_from(&current_settings, &current_settings.stt_engine);
    if current_settings.stt_engine != engine_id
        || current_model != model_id
        || session_config_from(&current_settings, &engine_id, &model_id) != session_config
    {
        return Err("语音模型配置已更改，请重试".into());
    }
    snapshot_and_emit(app, Some("hotkey".into()));
    crate::record_process_event(
        app,
        &serde_json::json!({
            "caseId": kotone_core::process_log::app_session_id(),
            "activity": "hotkey_register_started",
            "data": {}
        }),
    );
    current_settings.active_model_id.clone_from(&model_id);
    state
        .orchestrator
        .activate_runtime_settings_with_idle_timeout(
            current_settings.clone(),
            idle_timeout(&model_id),
        );
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

    let started = StartedSnapshot {
        engine_id,
        model_id,
        session_config,
    };
    rt.set_started(Some(started.clone()));
    rt.input_epoch.fetch_add(1, Ordering::SeqCst);
    rt.transit(runtime::finish_start)?;
    kotone_core::log::log("runtime started");
    prewarm(app, &started);

    // 显示悬浮窗（不抢焦点；会话态显隐仍由 TauriEmitter 驱动）。
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

    crate::overlay::reconcile(app, false);
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

fn refresh_running_model(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<SharedState>();
    let rt = app.state::<RuntimeManager>();
    let _configuration = rt.configuration_guard();
    let mut settings = state.settings.read().unwrap().clone();
    let engine_id = settings.stt_engine.clone();
    let model_id = kotone_stt::model::active_model_from(&settings, &engine_id);
    let session_config = session_config_from(&settings, &engine_id, &model_id);
    if let Some(current) = rt.started().filter(|current| {
        current.engine_id == engine_id
            && current.model_id == model_id
            && current.session_config == session_config
    }) {
        let failed_warmup = rt.error.lock().unwrap().is_some();
        if failed_warmup && !rt.warming.load(Ordering::SeqCst) {
            rt.next_model_epoch();
            prewarm(app, &current);
        }
        return Ok(());
    }
    let _input = state.orchestrator.try_input_configuration_guard()?;
    if let Err(error) = validate_engine(&settings, &state.engines, &engine_id, &model_id) {
        *rt.error.lock().unwrap() = Some(error.clone());
        snapshot_and_emit(app, None);
        return Err(error);
    }
    rt.next_model_epoch();
    settings.active_model_id.clone_from(&model_id);
    state
        .orchestrator
        .activate_runtime_settings_with_idle_timeout(settings, idle_timeout(&model_id));
    let started = StartedSnapshot {
        engine_id,
        model_id,
        session_config,
    };
    rt.set_started(Some(started.clone()));
    prewarm(app, &started);
    snapshot_and_emit(app, None);
    Ok(())
}

/// 显式暂停保持到用户恢复，不影响已经安排的提醒播报。
pub async fn stop(app: &AppHandle) -> Result<RuntimeStatus, String> {
    let rt = app.state::<RuntimeManager>();
    let _operation = rt.operation.lock().await;
    remember_pause(app, true)?;
    stop_enabled(app).await
}

async fn stop_enabled(app: &AppHandle) -> Result<RuntimeStatus, String> {
    let rt = app.state::<RuntimeManager>();
    rt.next_model_epoch();
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

    state.orchestrator.deactivate_runtime_settings_wait().await;
    rt.set_started(None);
    rt.transit(runtime::finish_stop)?;
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
    fn configured_installations_are_enabled_unless_explicitly_paused() {
        let mut settings = Settings::default();
        assert!(!automatically_enabled(&settings));
        settings.ui.first_run_completed = true;
        assert!(!settings.ui.auto_start);
        assert!(automatically_enabled(&settings));
        settings.ui.voice_input_paused = true;
        assert!(!automatically_enabled(&settings));
        settings.ui.voice_input_paused = false;
        settings.ui.first_run_completed = false;
        settings.active_model_id = "iflytek-asr".into();
        assert!(automatically_enabled(&settings));
    }

    #[test]
    fn schedules_local_models_by_size_without_prewarming_online_engines() {
        assert_eq!(model_loading("iflytek-asr"), ModelLoading::Online);
        let light = "x-asr-480ms-streaming-zh-en-punct-int8-2026-06-05";
        let large = "sense-voice-zh-en-ja-ko-yue-2024-07-17";
        assert_eq!(model_loading(light), ModelLoading::Background);
        assert_eq!(idle_timeout(light), None);
        assert_eq!(model_loading(large), ModelLoading::OnDemand);
        assert_eq!(idle_timeout(large), Some(Duration::from_secs(300)));
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
