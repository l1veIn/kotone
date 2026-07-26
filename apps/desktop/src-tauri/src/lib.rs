//! Kotone Rust 核心：模块组装、共享状态与 IPC 命令。
//! 职责划分见 docs/development.md §5.1；IPC 契约见 §5.3（类型对齐 src/lib/ipc.ts）。

mod hotkey;
mod runtime;
mod tray;

use std::sync::{Arc, RwLock};

use tauri::{AppHandle, Manager};

use hotkey::{HotkeyManager, HotkeyStatus};
use kotone_core::audio::AudioDevice;
use kotone_core::inject::{CancelToken, FocusBackend, InjectError, Injector};
use kotone_core::interaction::{effective_hotkey_mode, InteractionPolicy};
use kotone_core::orchestrator::{Emitter, Orchestrator};
use kotone_core::profile::{
    self, format_hotwords_export, merge_hotwords, parse_hotwords_import, GameProfile,
    HotwordMergeReport,
};
use kotone_core::runtime::RuntimePhase;
use kotone_core::settings::{self, OverlayVisibility, Settings};
use kotone_core::stt::{EngineInfo, EngineRegistry};
use kotone_core::log;
use kotone_platform_windows::inject::{WinFocusBackend, WindowsInjector};
use kotone_platform_windows::{audio as platform_audio, elevation, inject as platform_inject};
use kotone_stt::model;
use runtime::{RuntimeManager, RuntimeStatus};

/// 全局共享状态：settings 双端共享，orchestrator 是唯一业务状态所有者
pub struct SharedState {
    pub settings: Arc<RwLock<Settings>>,
    pub orchestrator: Arc<Orchestrator>,
    pub engines: Arc<EngineRegistry>,
    pub injector: Arc<dyn Injector>,
}

/// 生产事件出口：转发为 Tauri 事件；联动 Esc 取消键注册与 overlay 窗口显隐。
/// overlay 显隐规则（后端驱动，幂等，与前端逻辑不冲突）按 `overlay.visibility` 分档：
/// - always（常驻，默认）：会话态（Listening/…/Error）→ show；Running 期间 idle 不隐藏
///   （悬浮窗兼作运行指示）；Stopped 由 stop_runtime 显式隐藏。
/// - on_demand（用时浮现）：平时隐藏；Listening/Transcribing/Preview/Sending → show；
///   Success/Error（发送完成）延迟 ~600ms 自动隐藏（vis_gen 代际防新会话误藏）；
///   solo 连续模式保持显示直到会话停止（idle → hide）。
struct TauriEmitter {
    app: AppHandle,
    /// overlay 显隐代际：每个状态事件 +1；延迟隐藏任务只认调度时的代际
    vis_gen: Arc<std::sync::atomic::AtomicU64>,
}

impl Emitter for TauriEmitter {
    fn emit(&self, event: &str, payload: serde_json::Value) {
        use tauri::Emitter as _;
        let _ = self.app.emit(event, payload.clone());
        if event == "kotone://state" {
            let state = payload.get("state").and_then(|s| s.as_str()).unwrap_or("");
            log::log(&format!("state -> {state} {payload}"));
            // 会话激活期间（含 Preview）临时注册 Esc 全局取消键，回 Idle 即注销。
            // Preview 态同样需要 Esc：overlay 不抢焦点，Esc 是预览确认的主要键盘出口。
            if let Some(mgr) = self.app.try_state::<HotkeyManager>() {
                mgr.set_cancel_enabled(&self.app, state != "idle" && !state.is_empty());
            }
            // 后端驱动 overlay 显隐（显隐一律走原始 Win32 SW_SHOWNA/SW_HIDE 路径，
            // 不碰 tao set_visible——tao 缓存 diff 短路坑，见 hide_window 注释）
            if let Some(win) = self.app.get_webview_window("overlay") {
                let (visibility, continuous) = self
                    .app
                    .try_state::<SharedState>()
                    .map(|s| {
                        let g = s.settings.read().unwrap();
                        (g.overlay.visibility, InteractionPolicy::from_settings(&g).continuous)
                    })
                    .unwrap_or((OverlayVisibility::Always, false));
                let gen = self
                    .vis_gen
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                    + 1;
                match visibility {
                    OverlayVisibility::Always => {
                        let running = self
                            .app
                            .try_state::<RuntimeManager>()
                            .map(|rt| rt.phase() == RuntimePhase::Running)
                            .unwrap_or(false);
                        if state == "idle" && !running {
                            hide_window(&win);
                        } else if state != "idle" {
                            show_window_no_focus(&win);
                        }
                    }
                    OverlayVisibility::OnDemand => match state {
                        "listening" | "transcribing" | "preview" | "sending" => {
                            show_window_no_focus(&win);
                        }
                        "success" | "error" if !continuous => {
                            let app = self.app.clone();
                            let vis_gen = self.vis_gen.clone();
                            tauri::async_runtime::spawn(async move {
                                tokio::time::sleep(std::time::Duration::from_millis(600)).await;
                                if vis_gen.load(std::sync::atomic::Ordering::SeqCst) != gen {
                                    return; // 600ms 内已有新状态事件（新会话/停止），不藏
                                }
                                if let Some(win) = app.get_webview_window("overlay") {
                                    hide_window(&win);
                                }
                            });
                        }
                        "idle" => hide_window(&win),
                        // continuous（solo）的 success/error：会话未停，保持显示
                        _ => {}
                    },
                }
            }
        }
    }
}

/// 显示窗口但不抢焦点（焦点必须留在游戏/目标窗口，否则注入前台校验会失败）。
/// Windows 上用 SW_SHOWNA；其他平台回退普通 show。
///
/// 注意：本函数绕开 tao 的 window_flags 缓存直接操作 Win32 状态，因此隐藏必须
/// 走对称的 `hide_window`（见下），不能混用 Tauri 的 hide()/show()。
#[cfg(windows)]
fn show_window_no_focus<R: tauri::Runtime>(win: &tauri::WebviewWindow<R>) {
    use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_SHOWNA};
    match win.hwnd() {
        Ok(hwnd) => unsafe {
            let _ = ShowWindow(hwnd, SW_SHOWNA);
        },
        Err(_) => {
            let _ = win.show();
        }
    }
}

/// 隐藏窗口（与 show_window_no_focus 对称的原始 Win32 路径）。
///
/// 为什么不能用 Tauri 的 hide()：窗口经 SW_SHOWNA 显示后，tao 的 window_flags
/// 缓存仍停留在「不可见」（创建时 visible:false）；tao 的 set_visible(false)
/// 走 apply_diff，对缓存新旧值做 diff——diff 为空则提前返回，根本到不了
/// ShowWindow(SW_HIDE)（tao 0.35 window_state.rs）。这正是「停止后悬浮窗
/// 不隐藏」的根因：显隐机制不对称，与 runtime 编排无关。
#[cfg(windows)]
fn hide_window<R: tauri::Runtime>(win: &tauri::WebviewWindow<R>) {
    use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE};
    match win.hwnd() {
        Ok(hwnd) => unsafe {
            let _ = ShowWindow(hwnd, SW_HIDE);
        },
        Err(_) => {
            let _ = win.hide();
        }
    }
}

#[cfg(not(windows))]
fn show_window_no_focus<R: tauri::Runtime>(win: &tauri::WebviewWindow<R>) {
    let _ = win.show();
}

#[cfg(not(windows))]
fn hide_window<R: tauri::Runtime>(win: &tauri::WebviewWindow<R>) {
    let _ = win.hide();
}

/// 冒烟测试命令：前端可 invoke("ping") 验证 IPC 通路
#[tauri::command]
fn ping() -> &'static str {
    "pong"
}

// ---------- 设置与配置（§5.3） ----------

#[tauri::command]
fn get_settings(state: tauri::State<SharedState>) -> Settings {
    state.settings.read().unwrap().clone()
}

/// 局部更新配置；热键变化时触发重注册
#[tauri::command]
fn update_settings(
    app: AppHandle,
    state: tauri::State<SharedState>,
    patch: serde_json::Value,
) -> Result<Settings, String> {
    let (old_hotkey, updated) = {
        let mut guard = state.settings.write().unwrap();
        let old_hotkey = (
            guard.hotkey.key.clone(),
            effective_hotkey_mode(&guard),
            guard.hotkey_backend,
        );
        let mut merged =
            serde_json::to_value(&*guard).map_err(|e| format!("序列化配置失败: {e}"))?;
        settings::merge_json(&mut merged, &patch);
        let next: Settings =
            serde_json::from_value(merged).map_err(|e| format!("配置项不合法: {e}"))?;
        *guard = next.clone();
        (old_hotkey, next)
    };
    settings::save(&updated)?;

    // 热键键位/生效模式/后端变化 → 重注册。生效模式由 interactionMode 预设推导
    // （effective_hotkey_mode），所以切预设（如 push-to-talk）也会走到这里。
    // 仅 Running 时注册热键：Stopped 语义就是「按热键无反应」，
    // 配置变更会在下次 start_runtime 时生效。
    let next_mode = effective_hotkey_mode(&updated);
    let hotkey_changed = old_hotkey.0 != updated.hotkey.key
        || old_hotkey.1 != next_mode
        || old_hotkey.2 != updated.hotkey_backend;
    let running = app
        .try_state::<RuntimeManager>()
        .map(|rt| rt.phase() == RuntimePhase::Running)
        .unwrap_or(false);
    if hotkey_changed && running {
        if let Some(mgr) = app.try_state::<HotkeyManager>() {
            mgr.register(&app, &updated.hotkey.key, next_mode)?;
        }
    }
    // 引擎/模型/模式可能经 patch 变更 → restartNeeded 推导依赖最新配置，推送全量状态
    runtime::snapshot_and_emit(&app, None);
    Ok(updated)
}

#[tauri::command]
fn list_audio_devices() -> Vec<AudioDevice> {
    platform_audio::list_devices()
}

#[tauri::command]
fn set_audio_device(state: tauri::State<SharedState>, id: String) -> Result<(), String> {
    let updated = {
        let mut guard = state.settings.write().unwrap();
        guard.audio_device_id = id;
        guard.clone()
    };
    settings::save(&updated)
}

// ---------- STT 引擎（§5.3） ----------

#[tauri::command]
fn list_stt_engines(state: tauri::State<SharedState>) -> Vec<EngineInfo> {
    state.engines.list_info()
}

#[tauri::command]
fn set_stt_engine(app: AppHandle, state: tauri::State<SharedState>, id: String) -> Result<(), String> {
    if state.engines.get(&id).is_none() {
        return Err(format!("未注册的 STT 引擎: {id}"));
    }
    let updated = {
        let mut guard = state.settings.write().unwrap();
        guard.stt_engine = id;
        guard.clone()
    };
    settings::save(&updated)?;
    // Running 期间换引擎 → restartNeeded 推导变化，推送全量状态
    runtime::snapshot_and_emit(&app, None);
    Ok(())
}

#[tauri::command]
fn get_engine_options(state: tauri::State<SharedState>, id: String) -> serde_json::Value {
    let guard = state.settings.read().unwrap();
    guard
        .engine_options
        .get(&id)
        .cloned()
        .unwrap_or(serde_json::Value::Null)
}

// ---------- 游戏 profile（§5.3） ----------

#[tauri::command]
fn list_profiles() -> Vec<GameProfile> {
    profile::list()
}

#[tauri::command]
fn save_profile(profile: GameProfile) -> Result<(), String> {
    profile::save(&profile)
}

/// 导出 profile 热词到 UTF-8 文本（每行一词条，无权重），返回条数。
#[tauri::command]
fn export_hotwords(profile_id: String, path: String) -> Result<u32, String> {
    let p = profile::get(&profile_id).ok_or_else(|| format!("profile 不存在：{profile_id}"))?;
    let text = format_hotwords_export(&p.hotwords);
    std::fs::write(&path, text).map_err(|e| format!("写入 {path} 失败：{e}"))?;
    Ok(p.hotwords.len() as u32)
}

/// 从 UTF-8 文本导入热词（合并去重，追加到现有列表末尾），返回合并报告。
#[tauri::command]
fn import_hotwords(profile_id: String, path: String) -> Result<HotwordMergeReport, String> {
    let text = std::fs::read_to_string(&path).map_err(|e| format!("读取 {path} 失败：{e}"))?;
    let mut p = profile::get(&profile_id).ok_or_else(|| format!("profile 不存在：{profile_id}"))?;
    let incoming = parse_hotwords_import(&text);
    let (merged, report) = merge_hotwords(&p.hotwords, &incoming);
    p.hotwords = merged;
    profile::save(&p)?;
    Ok(report)
}

// ---------- 提权（UIPI 方案，§10 R-1） ----------

/// 提权状态：自身是否提权 + 当前激活 profile 的游戏进程是否提权（null = 无法判断）
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ElevationStatus {
    pub elevated: bool,
    pub active_game_elevated: Option<bool>,
}

#[tauri::command]
fn get_elevation_status(state: tauri::State<SharedState>) -> ElevationStatus {
    let elevated = elevation::is_elevated();
    // 激活 profile 的进程名 → 运行中的 pid → TokenElevation
    // 链路修复（resolve_active_game_pid）：profile 文件缺失时回退内置 profile，
    // 可用列表 = 磁盘 profile + 未覆盖的内置 profile
    let active_game_elevated = {
        let guard = state.settings.read().unwrap();
        let mut available = profile::list();
        for b in [
            GameProfile::builtin_lol(),
            GameProfile::builtin_generic(),
        ] {
            if !available.iter().any(|p| p.id == b.id) {
                available.push(b);
            }
        }
        elevation::resolve_active_game_pid(
            guard.active_profile_id.as_deref(),
            &available,
            &mut |name| platform_inject::find_pid_by_name(name),
        )
        .and_then(elevation::is_process_elevated)
    };
    ElevationStatus {
        elevated,
        active_game_elevated,
    }
}

/// 热键注册状态：设置页热键分区展示「注册失败，可能被其他程序/其他 Kotone 实例占用」
#[tauri::command]
fn get_hotkey_status(app: AppHandle) -> HotkeyStatus {
    app.state::<HotkeyManager>().status()
}

/// 开始热键捕获（设置页「点击录入」）：结果经 `kotone://hotkey-capture` 事件推送
#[tauri::command]
fn start_hotkey_capture(app: AppHandle) -> Result<(), String> {
    app.state::<HotkeyManager>().start_capture(app.clone())
}

/// 取消进行中的热键捕获
#[tauri::command]
fn cancel_hotkey_capture(app: AppHandle) {
    app.state::<HotkeyManager>().cancel_capture();
}

/// 一键管理员重启：ShellExecuteExW "runas" 拉起新进程后退出当前进程。
/// 用户在 UAC 弹窗点「否」会返回错误，当前进程继续运行。
#[tauri::command]
fn restart_as_admin(app: AppHandle) -> Result<(), String> {
    elevation::restart_as_admin()?;
    app.exit(0);
    Ok(())
}

// ---------- 模型 / 评测 ----------

#[tauri::command]
fn list_models() -> Result<Vec<model::ModelInfo>, String> {
    model::list()
}

/// 下载模型（id = 清单内任意模型 / silero-vad；镜像策略见 settings.download）。
/// 进度经 "kotone://download" 事件外发：{ id, downloaded, total }。
/// async 命令 + spawn_blocking：大模型下载不阻塞 UI 线程；IPC 签名不变。
#[tauri::command]
async fn download_model(app: AppHandle, id: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let app2 = app.clone();
        let id2 = id.clone();
        model::download(&id, &move |downloaded, total| {
            use tauri::Emitter as _;
            let _ = app2.emit(
                "kotone://download",
                serde_json::json!({ "id": id2, "downloaded": downloaded, "total": total }),
            );
        })
    })
    .await
    .map_err(|e| format!("下载任务异常：{e}"))?
}

#[tauri::command]
fn set_active_model(
    app: AppHandle,
    state: tauri::State<SharedState>,
    engine_id: String,
    model_id: String,
) -> Result<(), String> {
    model::set_active(&engine_id, &model_id)?;
    // model::set_active 直接读写 config.json；同步 SharedState 内存副本，
    // restartNeeded 推导（快照 vs 当前配置）以内存为准
    {
        let mut guard = state.settings.write().unwrap();
        if let Some(opts) = guard.engine_options.as_object_mut() {
            let entry = opts
                .entry(engine_id.clone())
                .or_insert_with(|| serde_json::json!({}));
            entry["model"] = serde_json::Value::String(model_id);
        }
    }
    // Running 期间换活动模型 → restartNeeded = true（不自动重启，用户显式重启生效）
    runtime::snapshot_and_emit(&app, None);
    Ok(())
}

// ---------- 运行时「启动」开关 ----------

/// 全量运行时状态（相位 + restartNeeded + 当前引擎/模型/模式）
#[tauri::command]
fn get_runtime_status(app: AppHandle, state: tauri::State<SharedState>) -> RuntimeStatus {
    let rt = app.state::<RuntimeManager>();
    let settings = state.settings.read().unwrap();
    rt.status(&settings, &state.engines, None)
}

/// 启动：warmup 引擎 → 注册热键 → 显示悬浮窗（进度经 kotone://runtime 推送）。
/// Running + restartNeeded 时等价于 stop + start（重启语义）。
#[tauri::command]
async fn start_runtime(app: AppHandle) -> Result<RuntimeStatus, String> {
    runtime::start(&app).await
}

/// 停止：取消会话 → 注销热键 → 隐藏悬浮窗 → 卸载引擎。Stopped 时幂等。
#[tauri::command]
async fn stop_runtime(app: AppHandle) -> Result<RuntimeStatus, String> {
    runtime::stop(&app).await
}

// ---------- 模型目录管理 ----------

/// 当前模型目录（生效路径 + 是否默认）
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelsDirInfo {
    pub dir: String,
    pub is_default: bool,
}

#[tauri::command]
fn get_models_dir(state: tauri::State<SharedState>) -> ModelsDirInfo {
    let settings = state.settings.read().unwrap();
    ModelsDirInfo {
        dir: model::models_dir_from(&settings).to_string_lossy().into_owned(),
        is_default: settings.models.dir.trim().is_empty(),
    }
}

/// set_models_dir 返回：新目录 + 迁移报告（moved / failed 条目名）
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelsDirMigration {
    pub dir: String,
    pub moved: Vec<String>,
    pub failed: Vec<String>,
}

/// 切换模型存储目录：先把旧目录内容移动到新目录（跨卷回退复制），
/// 迁移完成后才写配置——迁移失败过半也不丢配置一致性（failed 条目需重新下载）。
/// 传空字符串 = 恢复默认 ~/.kotone/models。
#[tauri::command]
fn set_models_dir(
    app: AppHandle,
    state: tauri::State<SharedState>,
    dir: String,
) -> Result<ModelsDirMigration, String> {
    let (old_dir, new_dir) = {
        let settings = state.settings.read().unwrap();
        let old = model::models_dir_from(&settings);
        let mut next = settings.clone();
        next.models.dir = dir.trim().to_string();
        (old, model::models_dir_from(&next))
    };
    if old_dir == new_dir {
        return Err("新目录与当前目录相同".into());
    }
    let report = model::migrate_dir_contents(&old_dir, &new_dir)?;

    let updated = {
        let mut guard = state.settings.write().unwrap();
        guard.models.dir = dir.trim().to_string();
        guard.clone()
    };
    settings::save(&updated)?;
    // 模型路径变化影响就绪判断与 restartNeeded 推导，推送状态
    runtime::snapshot_and_emit(&app, None);
    Ok(ModelsDirMigration {
        dir: new_dir.to_string_lossy().into_owned(),
        moved: report.moved,
        failed: report.failed,
    })
}

/// 删除已下载模型；active 模型被删时回退默认并同步 SharedState
#[tauri::command]
fn delete_model(
    app: AppHandle,
    state: tauri::State<SharedState>,
    id: String,
) -> Result<model::DeleteOutcome, String> {
    let outcome = model::delete(&id)?;
    if outcome.was_active {
        // model::delete 清了磁盘配置的 engineOptions[engine].model；同步内存副本
        let mut guard = state.settings.write().unwrap();
        if let Some(opts) = guard.engine_options.as_object_mut() {
            for entry in opts.values_mut().filter_map(|e| e.as_object_mut()) {
                if entry.get("model").and_then(|m| m.as_str()) == Some(id.as_str()) {
                    entry.remove("model");
                }
            }
        }
    }
    runtime::snapshot_and_emit(&app, None);
    Ok(outcome)
}

/// 在系统文件管理器中打开模型目录
#[tauri::command]
fn open_models_dir(state: tauri::State<SharedState>) -> Result<(), String> {
    let dir = {
        let settings = state.settings.read().unwrap();
        model::models_dir_from(&settings)
    };
    std::fs::create_dir_all(&dir).map_err(|e| format!("无法创建目录 {}：{e}", dir.display()))?;
    open_in_file_manager(&dir)
}

#[cfg(windows)]
fn open_in_file_manager(dir: &std::path::Path) -> Result<(), String> {
    std::process::Command::new("explorer.exe")
        .arg(dir)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("打开目录失败：{e}"))
}

#[cfg(not(windows))]
fn open_in_file_manager(dir: &std::path::Path) -> Result<(), String> {
    std::process::Command::new("xdg-open")
        .arg(dir)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("打开目录失败：{e}"))
}

// ---------- 识别历史（core history 薄转发；参考 CLI log list/clear，§8.1 历史面板） ----------

/// 识别历史列表（新→旧；模式/容量配置走通用 update_settings patch）
#[tauri::command]
fn get_history() -> Result<Vec<kotone_core::history::HistoryRecord>, String> {
    kotone_core::history::list()
}

/// 清空全部识别历史（含音频文件；前端做二次确认，后端不再拦截）
#[tauri::command]
fn clear_history() -> Result<(), String> {
    kotone_core::history::clear()
}

// ---------- 会话控制 / 调试 ----------

/// Preview 状态下确认发送（ADR-006：预览只读，始终发送预览文本）
#[tauri::command]
async fn confirm_send(state: tauri::State<'_, SharedState>) -> Result<(), String> {
    state.orchestrator.confirm_send().await
}

/// 取消当前会话（悬浮窗取消按钮 / 调试）
#[tauri::command]
async fn cancel_session(state: tauri::State<'_, SharedState>) -> Result<(), String> {
    state.orchestrator.cancel().await;
    Ok(())
}

/// 手动触发发送（调试/记事本测试用）：走真实 WindowsInjector 时序
/// （前台校验 → openChatKey → Unicode/剪贴板 → sendKey）
#[tauri::command]
fn simulate_send(
    state: tauri::State<SharedState>,
    text: String,
    profile_id: Option<String>,
) -> Result<(), InjectError> {
    let profile = profile_id
        .as_deref()
        .and_then(profile::get)
        .unwrap_or_else(GameProfile::builtin_generic);
    state
        .injector
        .send(&text, &profile, CancelToken::default())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // 单实例：第二实例启动时唤起已有实例的设置窗口并退出自身，
        // 避免旧进程未退出导致热键「already registered」与 WebView2 类注册冲突
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.show();
                let _ = win.unminimize();
                let _ = win.set_focus();
            }
        }))
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        // 关闭按钮不退出应用：main / overlay 窗口 CloseRequested 一律转 hide（托盘常驻）；
        // 仅托盘菜单「退出」（app.exit）真正结束进程。
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let _ = window.hide();
                api.prevent_close();
            }
        })
        .setup(|app| {
            log::init();
            log::log(&format!(
                "startup: args={:?} elevated={}",
                std::env::args().collect::<Vec<_>>(),
                elevation::is_elevated()
            ));
            tray::setup_tray(app.handle())?;

            // 首次运行：默认配置 + 内置 profile 落盘（~/.kotone/）
            let settings = settings::load();
            let _ = settings::save(&settings);

            // 自启动提权（§10 R-1）：设置开启且当前未提权时 runas 重启自身。
            // 防循环：重启子进程带 ELEVATED_RETRY_ARG 标记，若用户取消 UAC
            // （子进程仍未提权）则本次会话放弃重试。成功拉起后本进程直接退出。
            #[cfg(windows)]
            if elevation::should_auto_elevate(
                settings.run_as_admin_on_start,
                elevation::is_elevated(),
                elevation::retry_marker_present(),
            ) {
                log::log("auto-elevate: attempting runas restart");
                match elevation::restart_for_auto_elevate() {
                    Ok(()) => {
                        log::log("auto-elevate: spawned elevated copy, exiting this process");
                        app.handle().exit(0);
                        return Ok(());
                    }
                    Err(e) => {
                        log::log(&format!("auto-elevate failed: {e}"));
                        eprintln!("[kotone] 自动提权重启失败: {e}");
                    }
                }
            }

            if let Err(e) = profile::ensure_builtin() {
                eprintln!("[kotone] 内置 profile 落盘失败: {e}");
            }

            let settings = Arc::new(RwLock::new(settings));
            let mut registry = EngineRegistry::new();
            kotone_stt::register_builtin(&mut registry);
            let engines = Arc::new(registry);
            let emitter: Arc<dyn Emitter> = Arc::new(TauriEmitter {
                app: app.handle().clone(),
                vis_gen: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            });
            let injector: Arc<dyn Injector> = Arc::new(WindowsInjector);
            let focus: Arc<dyn FocusBackend> = Arc::new(WinFocusBackend);
            let audio_backend: Arc<dyn kotone_core::audio::AudioBackend> =
                Arc::new(platform_audio::CpalBackend);

            #[allow(unused_mut)] // vad-silero feature 关闭时无可变接线
            let mut orchestrator = Orchestrator::new(
                settings.clone(),
                engines.clone(),
                audio_backend,
                injector.clone(),
                focus,
                emitter,
            );
            // VAD 接线（ADR-007）：vad-silero feature 开启时注入 silero 工厂；
            // 默认构建不接入——one-shot 模式 begin 会报清晰错误
            #[cfg(feature = "vad-silero")]
            {
                orchestrator.vad_factory = Some(kotone_stt::vad::silero_factory());
            }
            let orchestrator = orchestrator.into_arc();

            app.manage(SharedState {
                settings: settings.clone(),
                orchestrator: orchestrator.clone(),
                engines,
                injector,
            });
            app.manage(HotkeyManager::new(app.handle(), orchestrator.clone()));
            app.manage(RuntimeManager::new());

            // 「启动」开关（core runtime 状态机）：默认 Stopped——不注册热键、
            // 不 warmup 引擎、悬浮窗隐藏。ui.autoStart = true 时自动 start_runtime
            // （warmup → 注册热键 → 显示悬浮窗，进度经 kotone://runtime 推送）。
            if settings.read().unwrap().ui.auto_start {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = runtime::start(&handle).await {
                        log::log(&format!("auto-start failed: {e}"));
                    }
                });
            }

            // 启动即显示设置窗口：全部窗口默认隐藏时，应用看起来像「没起来」。
            // 托盘常驻语义不变——关闭此窗口只是隐藏，进程仍在托盘。
            // 自动提权重启的场合此分支在提权子进程中执行，同样弹窗，符合预期。
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.show();
                let _ = win.set_focus();
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            ping,
            get_settings,
            update_settings,
            list_audio_devices,
            set_audio_device,
            list_stt_engines,
            set_stt_engine,
            get_engine_options,
            list_profiles,
            save_profile,
            export_hotwords,
            import_hotwords,
            get_elevation_status,
            get_hotkey_status,
            start_hotkey_capture,
            cancel_hotkey_capture,
            restart_as_admin,
            list_models,
            download_model,
            set_active_model,
            get_runtime_status,
            start_runtime,
            stop_runtime,
            get_models_dir,
            set_models_dir,
            delete_model,
            open_models_dir,
            get_history,
            clear_history,
            confirm_send,
            cancel_session,
            simulate_send,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Kotone application");
}
