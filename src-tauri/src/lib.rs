//! Kotone Rust 核心：模块组装、共享状态与 IPC 命令。
//! 职责划分见 docs/development.md §5.1；IPC 契约见 §5.3（类型对齐 src/lib/ipc.ts）。

mod audio;
mod eval;
mod hotkey;
pub mod inject;
mod model;
mod orchestrator;
pub mod profile;
mod settings;
mod stt;
mod tray;

use std::sync::{Arc, RwLock};

use tauri::{AppHandle, Manager};

use audio::AudioDevice;
use hotkey::HotkeyManager;
use inject::{InjectError, Injector, WindowsInjector};
use orchestrator::{Emitter, Orchestrator};
use profile::GameProfile;
use settings::Settings;
use stt::{EngineInfo, EngineRegistry};

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
            #[cfg(debug_assertions)]
            eprintln!("[kotone state] {state} {payload}");
            // 录音期间临时注册 Esc 全局取消键，会话结束注销
            if let Some(mgr) = self.app.try_state::<HotkeyManager>() {
                mgr.set_cancel_enabled(&self.app, state == "listening");
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
        let old_hotkey = (guard.hotkey.key.clone(), guard.hotkey.mode);
        let mut merged =
            serde_json::to_value(&*guard).map_err(|e| format!("序列化配置失败: {e}"))?;
        settings::merge_json(&mut merged, &patch);
        let next: Settings =
            serde_json::from_value(merged).map_err(|e| format!("配置项不合法: {e}"))?;
        *guard = next.clone();
        (old_hotkey, next)
    };
    settings::save(&updated)?;

    // 热键键位/模式变化 → 重注册
    if old_hotkey.0 != updated.hotkey.key || old_hotkey.1 != updated.hotkey.mode {
        if let Some(mgr) = app.try_state::<HotkeyManager>() {
            mgr.register(&app, &updated.hotkey.key, updated.hotkey.mode)?;
        }
    }
    Ok(updated)
}

#[tauri::command]
fn list_audio_devices() -> Vec<AudioDevice> {
    audio::list_devices()
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

/// 检测当前前台游戏并匹配 profile（inject::foreground_process_name → find_by_process）
#[tauri::command]
fn detect_foreground_game() -> Option<GameProfile> {
    profile::detect_foreground()
}

// ---------- 模型 / 评测（未实现，返回错误） ----------

#[tauri::command]
fn list_models() -> Result<Vec<model::ModelInfo>, String> {
    model::list()
}

#[tauri::command]
fn download_model(id: String) -> Result<(), String> {
    model::download(&id)
}

#[tauri::command]
fn set_active_model(engine_id: String, model_id: String) -> Result<(), String> {
    model::set_active(&engine_id, &model_id)
}

#[tauri::command]
fn eval_list_sessions() -> Result<Vec<eval::EvalSession>, String> {
    eval::list_sessions()
}

#[tauri::command]
fn eval_replay(session_id: String, engine_id: String) -> Result<eval::EvalSession, String> {
    eval::replay(&session_id, &engine_id)
}

#[tauri::command]
fn eval_export() -> Result<String, String> {
    eval::export()
}

// ---------- 会话控制 / 调试 ----------

/// Preview 状态下确认（可带编辑后文本）发送
#[tauri::command]
async fn confirm_send(
    state: tauri::State<'_, SharedState>,
    text: Option<String>,
) -> Result<(), String> {
    state.orchestrator.confirm_send(text).await
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
        .send(&text, &profile, inject::CancelToken::default())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
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
            tray::setup_tray(app.handle())?;

            // 首次运行：默认配置 + 内置 profile 落盘（~/.kotone/）
            let settings = settings::load();
            let _ = settings::save(&settings);
            if let Err(e) = profile::ensure_builtin() {
                eprintln!("[kotone] 内置 profile 落盘失败: {e}");
            }

            let settings = Arc::new(RwLock::new(settings));
            let engines = Arc::new(EngineRegistry::new());
            let emitter: Arc<dyn Emitter> = Arc::new(TauriEmitter {
                app: app.handle().clone(),
            });
            let injector: Arc<dyn Injector> = Arc::new(WindowsInjector);
            let audio_backend: Arc<dyn audio::AudioBackend> = Arc::new(audio::CpalBackend);

            let orchestrator = Arc::new(Orchestrator::new(
                settings.clone(),
                engines.clone(),
                audio_backend,
                injector.clone(),
                emitter,
            ));

            app.manage(SharedState {
                settings: settings.clone(),
                orchestrator,
                engines,
                injector,
            });
            app.manage(HotkeyManager::new());

            // 按配置注册全局热键（失败不致命，设置页可改键重试）
            let mgr = app.state::<HotkeyManager>();
            let (key, mode) = {
                let s = settings.read().unwrap();
                (s.hotkey.key.clone(), s.hotkey.mode)
            };
            if let Err(e) = mgr.register(app.handle(), &key, mode) {
                eprintln!("[kotone] {e}");
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
