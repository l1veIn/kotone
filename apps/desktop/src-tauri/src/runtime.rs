//! 运行时「启动」开关的壳侧实现：RuntimeManager 持有相位与启动快照，
//! start/stop 编排（warmup → 注册热键 → 显示悬浮窗 / 反向卸载），
//! 状态变化经 `kotone://runtime` 事件推送全量状态给前端。
//!
//! 迁移合法性由 core `kotone_core::runtime` 纯函数裁决；本模块只做编排。

use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Emitter, Manager};

use kotone_core::interaction::effective_hotkey_mode;
use kotone_core::runtime::{self, RuntimePhase};
use kotone_core::settings::Settings;
use kotone_core::stt::EngineRegistry;

use crate::hotkey::HotkeyManager;
use crate::{hide_window, show_window_no_focus, SharedState};

/// 启动时快照：Running 期间与当前配置比对推导 restartNeeded
#[derive(Debug, Clone)]
pub struct StartedSnapshot {
    pub engine_id: String,
    pub model_id: String,
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
        let restart_needed = runtime::restart_needed(
            phase,
            started
                .as_ref()
                .map(|s| (s.engine_id.as_str(), s.model_id.as_str())),
            (&engine_id, &model_id),
        );
        RuntimeStatus {
            phase: phase.as_str().to_string(),
            restart_needed,
            engine_name: engines.get(&engine_id).map(|e| e.display_name().to_string()),
            engine_id: Some(engine_id),
            model_id: Some(model_id),
            interaction_mode: settings
                .interaction_mode
                .as_ref()
                .map(|m| serde_json::to_string(m).unwrap_or_default().trim_matches('"').to_string()),
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
    emit_status(app, &status);
    Some(status)
}

/// 启动：warmup 当前引擎 → 注册热键 → 显示悬浮窗。
/// Running + restartNeeded 时等价于 stop + start（用户显式点「重启」走这里）。
pub async fn start(app: &AppHandle) -> Result<RuntimeStatus, String> {
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
        p => return Err(format!("运行时正在{}，请稍候", if p == RuntimePhase::Starting { "启动" } else { "停止" })),
    }

    rt.transit(runtime::begin_start)?;
    snapshot_and_emit(app, Some("warmup".into()));

    // 每个失败分支都要回滚相位；用闭包包住主流程统一处理
    let result = start_inner(app).await;
    if let Err(ref e) = result {
        kotone_core::log::log(&format!("runtime start failed: {e}"));
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

    let (engine_id, model_id, key, mode) = {
        let settings = state.settings.read().unwrap();
        (
            settings.stt_engine.clone(),
            kotone_stt::model::active_model_from(&settings, &settings.stt_engine),
            settings.hotkey.key.clone(),
            effective_hotkey_mode(&settings),
        )
    };
    if state.engines.get(&engine_id).is_none() {
        return Err(format!("未注册的 STT 引擎: {engine_id}（请在「引擎与模型」页重新选择）"));
    }

    // 阶段 1：warmup（模型入内存；sherpa 百毫秒级，放阻塞线程不卡 UI）
    let engines: Arc<EngineRegistry> = state.engines.clone();
    let warm_engine = engine_id.clone();
    tauri::async_runtime::spawn_blocking(move || {
        engines
            .get(&warm_engine)
            .ok_or_else(|| format!("未注册的 STT 引擎: {warm_engine}"))?
            .warmup()
    })
    .await
    .map_err(|e| format!("warmup 任务异常：{e}"))??;
    kotone_core::log::log(&format!("runtime warmup ok: {engine_id} ({model_id})"));

    // 阶段 2：注册全局热键（失败回滚：卸载引擎，不半截启动）
    snapshot_and_emit(app, Some("hotkey".into()));
    let mgr = app.state::<HotkeyManager>();
    if let Err(e) = mgr.register(app, &key, mode) {
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
    // on_demand（用时浮现）档位启动时不显示——平时隐藏，收音/转写时由
    // TauriEmitter 的状态事件浮现；always（常驻）维持启动即显示。
    // 每次启动重新应用尺寸、固定/自定义位置与点击穿透（显示器/DPI 可能已变）。
    snapshot_and_emit(app, Some("overlay".into()));
    let (overlay_on_demand, overlay_config) = {
        let g = state.settings.read().unwrap();
        (
            g.overlay.visibility == kotone_core::settings::OverlayVisibility::OnDemand,
            g.overlay.clone(),
        )
    };
    if let Some(win) = app.get_webview_window("overlay") {
        crate::apply_overlay_window_config(&win, &overlay_config);
    }
    // overlay WebView 启动时可能早于前端读取配置；运行态启动时重发一次全量配置，
    // 保证样式、拖动和点击穿透与后端已应用的窗口几何保持一致。
    let _ = app.emit("kotone://overlay-config", &overlay_config);
    if !overlay_on_demand {
        if let Some(win) = app.get_webview_window("overlay") {
            show_window_no_focus(&win);
        }
    }

    rt.set_started(Some(StartedSnapshot { engine_id, model_id }));
    rt.transit(runtime::finish_start)?;
    kotone_core::log::log("runtime started");
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
        p => return Err(format!("运行时正在{}，请稍候", if p == RuntimePhase::Starting { "启动" } else { "停止" })),
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
    let engine_id = state.settings.read().unwrap().stt_engine.clone();
    let _ = tauri::async_runtime::spawn_blocking(move || {
        if let Some(en) = engines.get(&engine_id) {
            en.unload();
        }
    })
    .await;

    rt.set_started(None);
    rt.transit(runtime::finish_stop)?;
    kotone_core::log::log("runtime stopped");
    snapshot_and_emit(app, None).ok_or_else(|| "运行时状态未初始化".to_string())
}
