//! 运行时「启动」开关的壳侧实现：RuntimeManager 持有相位与启动快照，
//! start/stop 编排（warmup → 注册热键 → 显示悬浮窗 / 反向卸载），
//! 状态变化经 `kotone://runtime` 事件推送全量状态给前端。
//!
//! 迁移合法性由 core `kotone_core::runtime` 纯函数裁决；本模块只做编排。

use std::sync::{Arc, Mutex};

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
        options: settings
            .engine_options
            .get(engine_id)
            .cloned()
            .unwrap_or(serde_json::Value::Null),
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
}

/// 运行时管理器：相位 + 启动快照（两个锁分开，避免事件构造时长持锁）
pub struct RuntimeManager {
    phase: Mutex<RuntimePhase>,
    started: Mutex<Option<StartedSnapshot>>,
}

impl RuntimeManager {
    pub fn new() -> Self {
        Self {
            phase: Mutex::new(RuntimePhase::Stopped),
            started: Mutex::new(None),
        }
    }

    pub fn phase(&self) -> RuntimePhase {
        *self.phase.lock().unwrap()
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
        }
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
    // 托盘「启动引擎 / 停止引擎」文案同步：启动/停止完成都会经过本推送点
    crate::tray::sync_toggle_label(app, status.phase == "running");
    emit_status(app, &status);
    Some(status)
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
    snapshot_and_emit(app, Some("warmup".into()));

    // 每个失败分支都要回滚相位；用闭包包住主流程统一处理
    let result = start_inner(app).await;
    if let Err(ref e) = result {
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
    let rt = app.state::<RuntimeManager>();

    let (settings_snapshot, engine_id, model_id, key, mode, session_config) = {
        let settings = state.settings.read().unwrap();
        let engine_id = settings.stt_engine.clone();
        (
            settings.clone(),
            engine_id.clone(),
            kotone_stt::model::active_model_from(&settings, &engine_id),
            settings.hotkey.key.clone(),
            effective_hotkey_mode(&settings),
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
            "未注册的 STT 引擎: {engine_id}（请在「引擎与模型」页重新选择）"
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
                "模型「{model_id}」文件不齐备：{}。请在「设置 → 高级」重新下载",
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
    if let Err(e) = mgr.register(app, &key, mode) {
        state.orchestrator.deactivate_runtime_settings();
        let engines = state.engines.clone();
        let unload_engine = engine_id.clone();
        let _ = tauri::async_runtime::spawn_blocking(move || {
            if let Some(en) = engines.get(&unload_engine) {
                en.unload();
            }
        })
        .await;
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
    rt.transit(runtime::finish_start)?;
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
    rt.transit(runtime::begin_stop)?;
    snapshot_and_emit(app, Some("unload".into()));

    let state = app.state::<SharedState>();

    // 进行中的会话先取消（orchestrator 是幂等的：Idle 时 cancel 无效果）
    state.orchestrator.cancel().await;

    // 注销热键（两个后端都停；LL 钩子 matcher 置禁用，按键全放行）
    let mgr = app.state::<HotkeyManager>();
    mgr.unregister(app)?;

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
