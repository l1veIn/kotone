//! Kotone Rust 核心：模块组装、共享状态与 IPC 命令。
//! 职责划分见 docs/development.md §5.1；IPC 契约见 §5.3（类型对齐 src/lib/ipc.ts）。

mod hotkey;
mod tray;

use std::sync::{Arc, RwLock};

use tauri::{AppHandle, Manager};

use hotkey::{HotkeyManager, HotkeyStatus};
use kotone_core::audio::AudioDevice;
use kotone_core::inject::{CancelToken, FocusBackend, InjectError, Injector};
use kotone_core::orchestrator::{Emitter, Orchestrator};
use kotone_core::profile::{self, GameProfile};
use kotone_core::settings::{self, Settings};
use kotone_core::stt::{EngineInfo, EngineRegistry};
use kotone_core::{eval, log};
use kotone_platform_windows::inject::{WinFocusBackend, WindowsInjector};
use kotone_platform_windows::{audio as platform_audio, elevation, inject as platform_inject};
use kotone_stt::model;

/// 全局共享状态：settings 双端共享，orchestrator 是唯一业务状态所有者
pub struct SharedState {
    pub settings: Arc<RwLock<Settings>>,
    pub orchestrator: Arc<Orchestrator>,
    pub engines: Arc<EngineRegistry>,
    pub injector: Arc<dyn Injector>,
}

/// 生产事件出口：转发为 Tauri 事件；联动 Esc 取消键注册与 overlay 窗口显隐。
/// overlay 显隐规则（后端驱动，幂等，与前端逻辑不冲突）：
/// - Listening/Transcribing/Preview/Sending/Success/Error → show（不抢焦点）
/// - Idle → hide
struct TauriEmitter {
    app: AppHandle,
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
            // 后端驱动 overlay 显隐
            if let Some(win) = self.app.get_webview_window("overlay") {
                if state == "idle" {
                    let _ = win.hide();
                } else {
                    show_window_no_focus(&win);
                }
            }
        }
    }
}

/// 显示窗口但不抢焦点（焦点必须留在游戏/目标窗口，否则注入前台校验会失败）。
/// Windows 上用 SW_SHOWNA；其他平台回退普通 show。
#[cfg(windows)]
fn show_window_no_focus(win: &tauri::WebviewWindow) {
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

#[cfg(not(windows))]
fn show_window_no_focus(win: &tauri::WebviewWindow) {
    let _ = win.show();
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
            guard.hotkey.mode,
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

    // 热键键位/模式/后端变化 → 重注册
    if old_hotkey.0 != updated.hotkey.key
        || old_hotkey.1 != updated.hotkey.mode
        || old_hotkey.2 != updated.hotkey_backend
    {
        if let Some(mgr) = app.try_state::<HotkeyManager>() {
            mgr.register(&app, &updated.hotkey.key, updated.hotkey.mode)?;
        }
    }
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
fn set_stt_engine(state: tauri::State<SharedState>, id: String) -> Result<(), String> {
    if state.engines.get(&id).is_none() {
        return Err(format!("未注册的 STT 引擎: {id}"));
    }
    let updated = {
        let mut guard = state.settings.write().unwrap();
        guard.stt_engine = id;
        guard.clone()
    };
    settings::save(&updated)
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

/// 检测当前前台游戏并匹配 profile（inject::foreground_process_name → find_by_process），
/// 附带目标进程提权状态（UIPI 诊断用，§10 R-1；null = 无法判断）
#[tauri::command]
fn detect_foreground_game() -> Option<ForegroundGameInfo> {
    let pid = platform_inject::foreground_pid()?;
    let name = platform_inject::process_name_from_pid(pid)?;
    let profile = profile::find_by_process(&profile::list(), &name)?;
    Some(ForegroundGameInfo {
        profile,
        target_elevated: elevation::is_process_elevated(pid),
    })
}

/// detect_foreground_game 返回值：profile 字段平铺 + targetElevated
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForegroundGameInfo {
    #[serde(flatten)]
    pub profile: GameProfile,
    pub target_elevated: Option<bool>,
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

/// 下载模型或 whisper-cli 运行时（id = ggml-* / whisper-cli）。
/// 进度经 "kotone://download" 事件外发：{ id, downloaded, total }。
/// async 命令 + spawn_blocking：466MB 模型下载不阻塞 UI 线程；IPC 签名不变。
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
fn set_active_model(engine_id: String, model_id: String) -> Result<(), String> {
    model::set_active(&engine_id, &model_id)
}

#[tauri::command]
fn eval_list_sessions() -> Result<Vec<eval::EvalSession>, String> {
    eval::list_sessions()
}

/// 回放是重计算（whisper finalize 走子进程可达数秒）：spawn_blocking 不阻塞 UI；
/// invoke 签名不变（sessionId, engineId -> EvalResult），引擎注册表由共享状态注入。
#[tauri::command]
async fn eval_replay(
    state: tauri::State<'_, SharedState>,
    session_id: String,
    engine_id: String,
) -> Result<eval::EvalResult, String> {
    let engines = state.engines.clone();
    tauri::async_runtime::spawn_blocking(move || eval::replay(&session_id, &engine_id, &engines))
        .await
        .map_err(|e| format!("回放任务异常：{e}"))?
}

#[tauri::command]
fn eval_export() -> Result<String, String> {
    eval::export()
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
            });
            let injector: Arc<dyn Injector> = Arc::new(WindowsInjector);
            let focus: Arc<dyn FocusBackend> = Arc::new(WinFocusBackend);
            let audio_backend: Arc<dyn kotone_core::audio::AudioBackend> =
                Arc::new(platform_audio::CpalBackend);

            let orchestrator = Arc::new(Orchestrator::new(
                settings.clone(),
                engines.clone(),
                audio_backend,
                injector.clone(),
                focus,
                emitter,
            ));

            app.manage(SharedState {
                settings: settings.clone(),
                orchestrator: orchestrator.clone(),
                engines,
                injector,
            });
            app.manage(HotkeyManager::new(app.handle(), orchestrator.clone()));

            // 按配置注册全局热键（失败不致命，设置页可改键重试）
            let mgr = app.state::<HotkeyManager>();
            let (key, mode) = {
                let s = settings.read().unwrap();
                (s.hotkey.key.clone(), s.hotkey.mode)
            };
            if let Err(e) = mgr.register(app.handle(), &key, mode) {
                eprintln!("[kotone] {e}");
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
            detect_foreground_game,
            get_elevation_status,
            get_hotkey_status,
            restart_as_admin,
            list_models,
            download_model,
            set_active_model,
            eval_list_sessions,
            eval_replay,
            eval_export,
            confirm_send,
            cancel_session,
            simulate_send,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Kotone application");
}
