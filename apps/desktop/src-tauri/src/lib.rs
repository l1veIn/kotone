//! Kotone Rust 核心：模块组装、共享状态与 IPC 命令。
//! 职责划分见 docs/development.md §5.1；IPC 契约见 §5.3（类型对齐 src/lib/ipc.ts）。

mod agent;
mod assistant;
mod assistant_input;
mod automation;
mod bubble;
mod companion;
mod compatibility;
mod computer_use;
mod diagnostics;
mod events;
mod fillers;
mod follow_up;
mod game_session;
mod game_tools;
mod hotkey;
mod key_request;
mod model_catalog;
mod ocr_engines;
mod overlay;
mod overlay_window;
mod recap;
mod roles;
mod runtime;
mod speech_models;
mod speech_screen;
mod startup;
mod stop;
mod tool_config;
mod tool_service;
mod tray;
mod tts_online;
mod usage;
mod voice_ask;
mod voice_chat;
mod voice_listen;
mod voice_reply;
mod voice_timing;
mod wake;
mod web_tools;
mod window;
mod workspace_cmd;

pub(crate) use events::*;
pub(crate) use overlay_window::*;
pub(crate) use startup::*;
pub(crate) use window::*;

use std::sync::{Arc, Mutex, RwLock};

use tauri::{AppHandle, Emitter as TauriEventEmitter, Manager};

use hotkey::{HotkeyManager, HotkeyStatus, InputEnvironmentCheck};
use kotone_core::audio::AudioDevice;
use kotone_core::connection::{Connection, ConnectionResolver, SecretStore};
use kotone_core::inject::{CancelToken, FocusBackend, InjectError, Injector};
use kotone_core::interaction::effective_hotkey_mode;
use kotone_core::orchestrator::{Emitter, Orchestrator, OrchestratorState};
use kotone_core::profile::{
    self, format_hotwords_export, GameProfile, HotwordMergeReport, ProfileDeleteOutcome,
};
use kotone_core::runtime::RuntimePhase;
use kotone_core::settings::{
    self, OverlayConfig, OverlayPosition, OverlayStyle, OverlayVisibility, Settings,
    SettingsRepository,
};
use kotone_core::stt::{EngineInfo, EngineRegistry};
use kotone_core::{log, process_log};
use kotone_platform_windows::inject::{WinFocusBackend, WindowsInjector};
use kotone_platform_windows::{
    audio as platform_audio, autostart, elevation, fullscreen, playback,
};
use kotone_stt::model;
use runtime::{RuntimeManager, RuntimeStatus};

/// 全局共享状态：settings 双端共享，orchestrator 是唯一业务状态所有者
pub struct SharedState {
    pub settings: Arc<RwLock<Settings>>,
    settings_repository: Arc<SettingsRepository>,
    settings_load_warning: Mutex<Option<String>>,
    pub orchestrator: Arc<Orchestrator>,
    pub engines: Arc<EngineRegistry>,
    pub processors: Arc<kotone_core::postprocess::ProcessorRegistry>,
    pub connections: Arc<dyn ConnectionResolver>,
    pub secrets: Arc<dyn SecretStore>,
    pub injector: Arc<dyn Injector>,
}

#[cfg(test)]
mod hotkey_settings_tests {
    use super::{app_hotkey_changed, hotkey_registration_changed};
    use kotone_core::settings::Settings;

    /// 启动 / 停止热键是应用级热键：它变化时不该触发 runtime 作用域的重注册，
    /// 但仍必须被识别为「需要重新落键」。
    #[test]
    fn runtime_toggle_hotkey_tracked_separately_from_runtime_hotkeys() {
        let old = Settings::default();
        let mut next = old.clone();
        next.runtime_toggle_hotkey = "Alt+F10".into();
        assert!(app_hotkey_changed(&old, &next));
        assert!(!hotkey_registration_changed(&old, &next));

        next.hotkey.key = "F9".into();
        assert!(hotkey_registration_changed(&old, &next));
    }

    #[test]
    fn changing_assistant_input_mode_requires_registration_but_not_voice_settings() {
        let old = Settings::default();
        let mut next = old.clone();
        next.assistant.speech_volume = 30;
        assert!(!hotkey_registration_changed(&old, &next));
        next.assistant.interaction_mode =
            kotone_core::assistant::AssistantInteractionMode::Dictation;
        assert!(hotkey_registration_changed(&old, &next));
        assert!(!app_hotkey_changed(&old, &next));
    }
}

/// 冒烟测试命令：前端可 invoke("ping") 验证 IPC 通路
#[tauri::command]
fn ping() -> &'static str {
    "pong"
}

/// 导出不含识别文本、音频和热词的诊断 ZIP。`window`：24h / 7d / all。
#[tauri::command]
fn export_diagnostics(
    app: AppHandle,
    path: String,
    window: Option<String>,
) -> Result<diagnostics::DiagnosticExportResult, String> {
    let window = diagnostics::DiagnosticWindow::parse(window.as_deref().unwrap_or("all"));
    diagnostics::export(&app, std::path::Path::new(&path), window)
}

/// 清除本地流程事件与 kotone.log。`stopRecording=true` 时同时关闭后续记录。
#[tauri::command]
fn clear_diagnostics(app: AppHandle, stop_recording: bool) -> Result<(), String> {
    diagnostics::clear(&app, stop_recording)
}

/// 前端全局异常与更新器错误落入持久日志。消息先做主目录脱敏并限制长度。
#[tauri::command]
fn log_frontend_error(context: String, message: String) {
    let context: String = context
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
        .take(64)
        .collect();
    let message = diagnostics::redact_home(&message.replace(['\r', '\n'], " "));
    log::log(&format!("frontend error [{context}]: {message}"));
}

// ---------- 设置与配置（§5.3） ----------

#[tauri::command]
fn get_settings(state: tauri::State<SharedState>) -> Settings {
    state.settings.read().unwrap().clone()
}

/// 已注册后处理模块的只读发现接口；设置页不维护硬编码模块清单。
#[tauri::command]
fn list_post_processors(
    state: tauri::State<SharedState>,
) -> Vec<kotone_core::postprocess::ProcessorDescriptor> {
    state.processors.list_info()
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ConnectionPresetInfo {
    id: String,
    display_name: String,
    /// `recommended` / `china` / `global` / `aggregator` / `custom`
    group: String,
    description: String,
    default_base_url: String,
    default_model: String,
    models: Vec<String>,
    api_key_url: Option<String>,
}

#[tauri::command]
fn list_connection_presets() -> Vec<ConnectionPresetInfo> {
    kotone_core::connection::CONNECTION_PRESETS
        .iter()
        .map(|preset| ConnectionPresetInfo {
            id: preset.id.into(),
            display_name: preset.display_name.into(),
            group: preset.group.into(),
            description: preset.description.into(),
            default_base_url: preset.default_base_url.into(),
            default_model: preset.default_model.into(),
            models: preset.models.iter().map(|m| m.to_string()).collect(),
            api_key_url: preset.api_key_url.map(str::to_string),
        })
        .collect()
}

#[tauri::command]
fn list_connections(
    state: tauri::State<SharedState>,
) -> Result<Vec<kotone_ai::connections::ConnectionInfo>, String> {
    let settings = state.settings.read().unwrap();
    kotone_ai::connections::list_connection_info(&settings, state.secrets.as_ref())
}

#[tauri::command]
fn upsert_connection(
    state: tauri::State<SharedState>,
    connection: Connection,
    api_key: Option<String>,
) -> Result<Settings, String> {
    kotone_ai::connections::upsert_connection(
        &state.settings_repository,
        state.secrets.as_ref(),
        connection,
        api_key,
    )
}

#[tauri::command]
fn delete_connection(state: tauri::State<SharedState>, id: String) -> Result<Settings, String> {
    kotone_ai::connections::delete_connection(
        &state.settings_repository,
        state.secrets.as_ref(),
        &id,
    )
}

/// 独立试跑一条后处理 pipeline；不经过 orchestrator，因此不会写历史或触发注入。
/// 未传 pipeline 时使用当前设置的 pipeline 快照，即使总开关关闭也会执行其步骤。
#[tauri::command]
async fn test_post_processing(
    state: tauri::State<'_, SharedState>,
    text: String,
    pipeline: Option<kotone_core::postprocess::PipelineConfig>,
) -> Result<kotone_core::postprocess::PostProcessingTestResult, String> {
    let pipeline = pipeline.unwrap_or_else(|| {
        state
            .settings
            .read()
            .unwrap()
            .post_processing
            .active_pipeline()
            .cloned()
            .unwrap_or_default()
    });
    kotone_core::postprocess::test_post_processing(text, pipeline, &state.processors).await
}

/// 启动时配置恢复诊断；主设置页读取后以 toast 明确告知用户备份路径。
#[tauri::command]
fn get_settings_load_warning(state: tauri::State<SharedState>) -> Option<String> {
    state.settings_load_warning.lock().unwrap().take()
}

fn hotkey_registration_changed(old: &Settings, next: &Settings) -> bool {
    old.hotkey.key != next.hotkey.key
        || effective_hotkey_mode(old) != effective_hotkey_mode(next)
        || old.hotkey_backend != next.hotkey_backend
        || old.channel_cycle_hotkey != next.channel_cycle_hotkey
        || old.resend_last_hotkey != next.resend_last_hotkey
        || old.toggle_post_processing_hotkey != next.toggle_post_processing_hotkey
        || old.cycle_post_processing_hotkey != next.cycle_post_processing_hotkey
        || old.call_hotkey() != next.call_hotkey()
        || old.legacy_reminder_hotkey() != next.legacy_reminder_hotkey()
        || old.assistant.interaction_mode != next.assistant.interaction_mode
        || old.ui.legacy_upgrade.uses_legacy_assistant()
            != next.ui.legacy_upgrade.uses_legacy_assistant()
}

/// 应用级热键（启动 / 停止 Kotone）变化：与 runtime 是否运行无关，必须重注册。
fn app_hotkey_changed(old: &Settings, next: &Settings) -> bool {
    old.runtime_toggle_hotkey != next.runtime_toggle_hotkey
        || old.automation_stop_hotkey != next.automation_stop_hotkey
        || old.ui.legacy_upgrade.uses_legacy_assistant()
            != next.ui.legacy_upgrade.uses_legacy_assistant()
}

/// 局部更新配置；热键变化时触发重注册
#[tauri::command]
fn update_settings(
    app: AppHandle,
    state: tauri::State<SharedState>,
    patch: serde_json::Value,
) -> Result<Settings, String> {
    let runtime_manager = app.try_state::<RuntimeManager>();
    let configuration_guard = runtime_manager.as_ref().map(|rt| rt.configuration_guard());
    let running = runtime_manager
        .as_ref()
        .is_some_and(|rt| rt.phase() == RuntimePhase::Running);
    let manager = app.try_state::<HotkeyManager>();
    if patch.get("connections").is_some() {
        return Err("连接请使用专用接口保存，不能写入通用设置".into());
    }
    if patch
        .get("models")
        .and_then(|models| models.get("dir"))
        .is_some()
    {
        return Err("模型目录请使用目录迁移接口修改".into());
    }
    // Drop the core admission lock before accepting fresh source events, including
    // error/rollback exits. The runtime pause owns no mutex and may outlive apply.
    let mut input_pause = None;
    let mut input_configuration = None;
    let (old, updated, ()) = state.settings_repository.update_with(
        |next| {
            let previous = next.clone();
            let mut merged =
                serde_json::to_value(&*next).map_err(|e| format!("序列化配置失败: {e}"))?;
            settings::merge_json(&mut merged, &patch);
            *next = serde_json::from_value(merged).map_err(|e| format!("配置项不合法: {e}"))?;
            next.overlay.normalize_interaction();
            next.assistant.normalize_voices();
            next.assistant.validate()?;
            let call_key = kotone_core::hotkey::parse_hotkey(next.call_hotkey());
            if call_key.is_some()
                && call_key == kotone_core::hotkey::parse_hotkey(next.legacy_reminder_hotkey())
            {
                return Err("呼叫琴音和提醒助手不能共用热键".into());
            }
            if previous.overlay.visibility != next.overlay.visibility
                && next.overlay.visibility != OverlayVisibility::Never
            {
                overlay::prepare_enable(&app);
            }
            if running && hotkey_registration_changed(&previous, next) {
                input_configuration = Some(state.orchestrator.try_input_configuration_guard()?);
                input_pause = runtime_manager.as_ref().map(|rt| rt.pause_inputs());
                state.orchestrator.invalidate_hotkey_inputs();
            }
            Ok(())
        },
        |old, next| {
            if running && hotkey_registration_changed(old, next) {
                if let Some(manager) = manager.as_ref() {
                    manager.register_with_settings(
                        &app,
                        &next.hotkey.key,
                        effective_hotkey_mode(next),
                        next,
                    )?;
                }
            }
            // 应用级热键不受 running 限制：runtime 停止时也必须能一键启动
            if app_hotkey_changed(old, next) {
                if let Some(manager) = manager.as_ref() {
                    manager.apply_app_hotkeys(next);
                }
            }
            Ok(())
        },
        |old, next| {
            if running && hotkey_registration_changed(old, next) {
                if let Some(manager) = manager.as_ref() {
                    manager.register_with_settings(
                        &app,
                        &old.hotkey.key,
                        effective_hotkey_mode(old),
                        old,
                    )?;
                }
            }
            if app_hotkey_changed(old, next) {
                if let Some(manager) = manager.as_ref() {
                    manager.apply_app_hotkeys(old);
                }
            }
            Ok(())
        },
    )?;
    drop(input_configuration);
    drop(input_pause);
    drop(configuration_guard);
    // 语音识别没准备好时盯着的提示键（语音输入热键 / 呼叫键）跟着设置走
    if hotkey_registration_changed(&old, &updated) || app_hotkey_changed(&old, &updated) {
        runtime::refresh_hints(&app);
    }
    if hotkey_registration_changed(&old, &updated) {
        use tauri::Emitter as _;
        let _ = app.emit(
            "kotone://input-config",
            serde_json::json!({
                "hotkey": updated.hotkey.key,
                "assistantHotkey": updated.call_hotkey(),
                "legacyReminderHotkey": updated.legacy_reminder_hotkey(),
                "assistantMode": updated.assistant.interaction_mode,
                "legacyAssistant": updated.ui.legacy_upgrade.uses_legacy_assistant(),
            }),
        );
    }
    process_log::set_recording_enabled(updated.diagnostics.recording);
    let old_overlay = old.overlay;

    // overlay 配置变化 → 立即重排几何/点击穿透/显隐 + 通知前端（无需重启）
    if old_overlay != updated.overlay {
        overlay::reconcile(
            &app,
            old_overlay.visibility != updated.overlay.visibility
                && updated.overlay.visibility != OverlayVisibility::Never,
        );
        use tauri::Emitter as _;
        let _ = app.emit("kotone://overlay-config", &updated.overlay);
    }

    if !updated.ui.legacy_upgrade.uses_legacy_assistant() {
        if old.ui.legacy_upgrade.uses_legacy_assistant() {
            automation::ensure_builtin(&app);
            roles::migrate(&app);
            // 同步命令跑在主线程的 IPC 回调里：在这里直接建 WebView（气泡窗）会让 WebView2
            // 在回调里嵌套创建控制器而死锁（2026-11-20 真机：老用户填 Key 启用琴音后整个应用卡住）。
            // 交给后台任务，等这次 IPC 返回后再由事件循环建窗口。
            let windows_app = app.clone();
            tauri::async_runtime::spawn(async move {
                create_startup_windows(&windows_app, true);
            });
        }
        // 当前角色（名字、声音、唤醒词跟着它）
        roles::remember(&updated.agent.role_id);
        // 换了声音 / 角色 / 引擎：填充话按新声音重新合成（ADR 029 §四；没变就只是过一遍缓存）。
        fillers::refresh(&app);
    }
    // 引擎/模型/模式可能经 patch 变更 → restartNeeded 推导依赖最新配置，推送全量状态
    runtime::snapshot_and_emit(&app, None);
    Ok(updated)
}

/// 后端自己改设置（例如换角色）：和设置页保存一样生效，并通知各窗口刷新。
pub(crate) fn update_settings_from_backend(
    app: &AppHandle,
    patch: serde_json::Value,
) -> Result<Settings, String> {
    use tauri::Emitter as _;
    let updated = update_settings(app.clone(), app.state(), patch)?;
    let _ = app.emit("kotone://settings-changed", &updated);
    Ok(updated)
}

#[tauri::command]
fn list_audio_devices() -> Vec<AudioDevice> {
    platform_audio::list_devices()
}

#[tauri::command]
fn set_audio_device(state: tauri::State<SharedState>, id: String) -> Result<(), String> {
    state
        .settings_repository
        .update(|next| {
            next.audio_device_id = id;
            Ok(())
        })
        .map(|_| ())
}

// ---------- STT 引擎（§5.3） ----------

#[tauri::command]
async fn list_stt_engines(state: tauri::State<'_, SharedState>) -> Result<Vec<EngineInfo>, String> {
    let engines = state.engines.clone();
    tauri::async_runtime::spawn_blocking(move || engines.list_info())
        .await
        .map_err(|error| format!("列举引擎失败：{error}"))
}

#[tauri::command]
fn set_stt_engine(
    app: AppHandle,
    state: tauri::State<SharedState>,
    id: String,
) -> Result<(), String> {
    if state.engines.get(&id).is_none() {
        return Err(format!("未注册的 STT 引擎: {id}"));
    }
    let canonical = kotone_core::stt::canonical_stt_engine(&id).to_string();
    state.settings_repository.update(|next| {
        next.stt_engine = canonical;
        Ok(())
    })?;
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

/// 保存 profile。内置 profile（lol / dota2 / generic）会同步推导 removedBuiltinHotwords：
/// 内置热词全量列表 − 传入热词列表 = 用户主动删除的内置热词（供后续合并逻辑排除）。
/// 非内置 id 不触碰该字段。
#[tauri::command]
fn save_profile(mut profile: GameProfile) -> Result<(), String> {
    let builtin = match profile.id.as_str() {
        "lol" => Some(GameProfile::builtin_lol()),
        "dota2" => Some(GameProfile::builtin_dota2()),
        "generic" => Some(GameProfile::builtin_generic()),
        _ => None,
    };
    if let Some(builtin) = builtin {
        profile.removed_builtin_hotwords = builtin
            .hotwords
            .iter()
            .filter(|w| !profile.hotwords.contains(w))
            .cloned()
            .collect();
    }
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

/// 导出内置屏蔽词表到 UTF-8 CSV（两列：屏蔽词,替换词），返回规则条数。
#[tauri::command]
fn export_builtin_blocklist(path: String) -> Result<usize, String> {
    std::fs::write(&path, kotone_postprocess::blocklist::builtin_csv())
        .map_err(|error| format!("写入 {path} 失败：{error}"))?;
    Ok(kotone_postprocess::blocklist::builtin_rule_count())
}

/// 从 UTF-8 文本导入热词（合并去重，追加到现有列表末尾），返回合并报告。
#[tauri::command]
fn import_hotwords(profile_id: String, path: String) -> Result<HotwordMergeReport, String> {
    profile::import_hotwords_from_file(&profile_id, std::path::Path::new(&path))
}

/// 导出整包 profile 为 .kprofile ZIP（profile.json + icon.*）。
#[tauri::command]
fn export_profile(profile_id: String, path: String) -> Result<(), String> {
    profile::export_profile(&profile_id, std::path::Path::new(&path))
}

/// 导入 .kprofile ZIP 包：生成新随机 id 落盘，返回导入后的 profile。
#[tauri::command]
fn import_profile(path: String) -> Result<GameProfile, String> {
    profile::import_profile(std::path::Path::new(&path))
}

/// 删除 profile：内置 = 恢复出厂；导入的 = 永久删除（含 icon）。
#[tauri::command]
fn delete_profile(profile_id: String) -> Result<ProfileDeleteOutcome, String> {
    profile::delete_profile(&profile_id)
}

/// 读取 profile 图标字节（无图标 → None）。前端按扩展名推断 mime 建 Blob URL。
#[tauri::command]
fn get_profile_icon(profile_id: String) -> Result<Option<Vec<u8>>, String> {
    Ok(profile::profile_icon_bytes(&profile_id))
}

// ---------- 提权（UIPI 方案，§10 R-1） ----------

#[tauri::command]
fn get_elevation_status(state: tauri::State<SharedState>) -> compatibility::ElevationStatus {
    compatibility::elevation_status(&state)
}

/// 热键注册状态：设置页热键分区展示「注册失败，可能被其他程序/其他 Kotone 实例占用」
#[tauri::command]
fn get_hotkey_status(app: AppHandle) -> HotkeyStatus {
    app.state::<HotkeyManager>().status()
}

/// 主动检测低级键盘钩子与 SendInput 环境；不要求先开始录入或启动运行时。
#[tauri::command]
async fn check_input_environment(app: AppHandle) -> Result<InputEnvironmentCheck, String> {
    let worker_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        worker_app
            .state::<HotkeyManager>()
            .check_input_environment()
    })
    .await
    .map_err(|e| format!("输入环境自检任务失败：{e}"))
}

/// 静态键位冲突提示（P2-⑩：向导热键步骤录入后展示；常见游戏键位表）
#[tauri::command]
fn detect_hotkey_conflicts(key: String) -> Vec<String> {
    crate::hotkey::detect_conflicts(&key)
}

/// 开始热键捕获（设置页「点击录入」）：结果经 `kotone://hotkey-capture` 事件推送
#[tauri::command]
async fn start_hotkey_capture(app: AppHandle) -> Result<(), String> {
    let worker_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        worker_app
            .state::<HotkeyManager>()
            .start_capture(worker_app.clone())
    })
    .await
    .map_err(|e| format!("启动热键自检任务失败：{e}"))?
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

// ---------- 开机自启 ----------

/// 读取「开机自动启动」系统真实状态（HKCU Run 键 Kotone 值是否存在）。
/// 用户在任务管理器「启动」里禁用后这里返回 false，设置页据此与系统同步。
#[tauri::command]
fn get_autostart_enabled() -> bool {
    autostart::autostart_enabled()
}

/// 设置「开机自动启动」：写入 / 删除 HKCU Run 键（无参数启动；
/// 值名与 NSIS 卸载清理共用，卸载时自动移除）。
#[tauri::command]
fn set_autostart_enabled(enabled: bool) -> Result<(), String> {
    autostart::set_autostart(enabled)
}

// ---------- 音效提示 ----------

/// 试听内置提示音（设置页「音效」tab）：kind = record/send；soundId 缺失时按事件默认。
#[tauri::command]
fn preview_sfx(kind: String, sound_id: Option<String>) {
    let is_record = kind != "send";
    let id = sound_id
        .as_deref()
        .and_then(playback::SfxId::from_id)
        .unwrap_or_else(|| playback::SfxId::default_for(is_record));
    playback::play_sfx(id, 0.8);
}

// ---------- 模型 / 评测 ----------

#[tauri::command]
async fn list_models() -> Result<Vec<model::ModelInfo>, String> {
    tauri::async_runtime::spawn_blocking(model::list)
        .await
        .map_err(|error| format!("列举模型失败：{error}"))?
}

/// 下载模型（id = 清单内任意模型 / silero-vad；镜像策略见 settings.download）。
/// 进度经 "kotone://download" 事件外发：{ id, downloaded, total }。
/// async 命令 + spawn_blocking：大模型下载不阻塞 UI 线程；IPC 签名不变。
/// 下载前做磁盘空间预检（P2-⑦）：目标目录所在卷剩余空间不足时直接拒绝，
/// 避免下载到一半才发现 C 盘/目标盘已满。
#[tauri::command]
async fn download_model(
    app: AppHandle,
    state: tauri::State<'_, SharedState>,
    id: String,
) -> Result<(), String> {
    // 磁盘空间预检：模型体积 vs 模型目录所在卷可用空间
    if let (Some(need), Some(dir)) = (model::model_size_bytes(&id), Some(model_dir_now(&state))) {
        if let Some(avail) = disk_available_space(&dir) {
            if avail < need {
                let mb = |b: u64| b / 1_000_000;
                return Err(format!(
                    "磁盘空间不足：模型需要约 {} MB，但「{}」所在磁盘仅剩 {} MB。\
                     请清理磁盘或在设置中更换模型存储位置",
                    mb(need),
                    dir.display(),
                    mb(avail)
                ));
            }
        }
    }
    let case_id = format!(
        "model-download-{}-{}",
        id,
        kotone_core::eval::new_session_id()
    );
    record_process_event(
        &app,
        &serde_json::json!({
            "caseId": case_id.clone(),
            "activity": "model_download_started",
            "data": {}
        }),
    );
    let started = std::time::Instant::now();
    let app_for_download = app.clone();
    let case_for_result = case_id.clone();
    let result = match tauri::async_runtime::spawn_blocking(move || {
        let app2 = app_for_download.clone();
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
    {
        Ok(result) => result,
        Err(error) => Err(format!("下载任务异常：{error}")),
    };
    let (activity, outcome, error_code) = if result.is_ok() {
        ("model_download_succeeded", "success", None)
    } else {
        (
            "model_download_failed",
            "error",
            Some("MODEL_DOWNLOAD_FAILED"),
        )
    };
    record_process_event(
        &app,
        &serde_json::json!({
            "caseId": case_for_result,
            "activity": activity,
            "data": {
                "outcome": outcome,
                "errorCode": error_code,
                "durationMs": started.elapsed().as_millis() as u64
            }
        }),
    );
    result
}

/// 当前生效模型目录（磁盘预检用）
fn model_dir_now(state: &tauri::State<'_, SharedState>) -> std::path::PathBuf {
    let settings = state.settings.read().unwrap();
    model::models_dir_from(&settings)
}

/// 路径所在卷的可用空间（sysinfo Disks；取最长挂载前缀匹配；失败返回 None 不阻断下载）
fn disk_available_space(path: &std::path::Path) -> Option<u64> {
    use sysinfo::Disks;
    let disks = Disks::new_with_refreshed_list();
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    disks
        .iter()
        .filter(|d| canonical.starts_with(d.mount_point()))
        .max_by_key(|d| d.mount_point().as_os_str().len())
        .map(|d| d.available_space())
}

/// 请求取消进行中的模型下载（幂等；.part 保留可续传）
#[tauri::command]
fn cancel_download() {
    model::cancel_download();
    log::log("download cancel requested");
}

/// 自动下载失败后的手动安装指引（官方页 + 目标目录 + 文件清单）
#[tauri::command]
fn get_model_install_guide(id: String) -> Result<model::ModelInstallGuide, String> {
    model::install_guide(&id)
}

/// 打开某个模型的目标目录（自动创建），方便用户把手动下载的文件放进去
#[tauri::command]
fn open_model_dest_dir(id: String) -> Result<(), String> {
    let dir = model::multi_model_dir(&id).ok_or_else(|| format!("未知模型：{id}"))?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("无法创建目录 {}：{e}", dir.display()))?;
    open_in_file_manager(&dir)
}

#[tauri::command]
fn set_active_model(
    app: AppHandle,
    state: tauri::State<SharedState>,
    engine_id: String,
    model_id: String,
) -> Result<(), String> {
    state
        .settings_repository
        .update(|next| model::set_active_in(next, &engine_id, &model_id))?;
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
        dir: model::models_dir_from(&settings)
            .to_string_lossy()
            .into_owned(),
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
/// 运行时 Running 期间拒绝迁移：模型文件被引擎占用（Windows 文件锁），
/// 迁移必然失败且产生困惑的 failed 列表（P2-⑧）。
#[tauri::command]
fn set_models_dir(
    app: AppHandle,
    state: tauri::State<SharedState>,
    dir: String,
) -> Result<ModelsDirMigration, String> {
    let runtime_manager = app.state::<RuntimeManager>();
    let _configuration = runtime_manager.configuration_guard();
    // 只有本地识别会占用模型文件（Windows 文件锁）；在线识别一直就绪，不影响迁移。
    let local_running = runtime_manager.phase() != RuntimePhase::Stopped
        && runtime_manager
            .status(&state.settings.read().unwrap(), &state.engines, None)
            .asr_local;
    if local_running {
        return Err(
            "本地语音识别正在运行，请先停止再迁移模型目录（「语音输入」页或托盘「停止本地语音识别」）"
                .into(),
        );
    }
    if app
        .state::<speech_models::SpeechModelManager>()
        .is_downloading()
    {
        return Err("语音模型正在下载，请等待完成或取消后再迁移目录".into());
    }
    if kotone_stt::download::is_active() {
        return Err("有模型正在下载，请等待完成或取消后再迁移目录".into());
    }
    let assistant_state = app.state::<assistant::AssistantRuntime>();
    let _output = assistant::prepare_model_migration(&assistant_state)?;
    let requested_dir = dir.trim().to_string();
    let (_, updated, report) = state.settings_repository.update_with(
        |next| {
            next.models.dir = requested_dir;
            model::validate_models_dir_path(&model::models_dir_from(next))
        },
        |old, next| {
            let old_dir = model::models_dir_from(old);
            let new_dir = model::models_dir_from(next);
            if old_dir == new_dir {
                return Err("新目录与当前目录相同".into());
            }
            model::migrate_dir_contents(&old_dir, &new_dir)
        },
        |_, _| Ok(()),
    )?;
    let new_dir = model::models_dir_from(&updated);
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
    let was_active = std::cell::Cell::new(false);
    state.settings_repository.update_with(
        |next| {
            was_active.set(model::clear_active_model(next, &id)?);
            Ok(())
        },
        |_, next| model::delete_files_from(next, &id),
        |_, _| Ok(()),
    )?;
    let outcome = model::DeleteOutcome {
        was_active: was_active.get(),
    };
    runtime::snapshot_and_emit(&app, None);
    Ok(outcome)
}

/// 在系统文件管理器中打开历史记录目录（P2-⑨：历史目录可自定义后仍需可视入口）
#[tauri::command]
fn open_history_dir() -> Result<(), String> {
    let dir = kotone_core::history::history_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("无法创建目录 {}：{e}", dir.display()))?;
    open_in_file_manager(&dir)
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

/// 在系统默认浏览器中打开外部链接（关于页 GitHub 等）。
/// webview 内 target=_blank 不会调起系统浏览器；rundll32 方案零新增依赖，
/// 白名单只允许 http/https，避免被当成任意协议执行入口。
#[tauri::command]
fn open_external(url: String) -> Result<(), String> {
    if !url.starts_with("https://") && !url.starts_with("http://") {
        return Err("仅支持打开 http/https 链接".into());
    }
    std::process::Command::new("rundll32")
        .args(["url.dll,FileProtocolHandler", &url])
        .spawn()
        .map_err(|e| format!("打开浏览器失败：{e}"))?;
    Ok(())
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

/// 删除单条识别历史（按 sessionId + ts 精确匹配；带录音且不再被其他记录
/// 引用时一并删除对应 wav；记录不存在 = 静默成功，幂等）
#[tauri::command]
fn delete_history_record(session_id: String, ts: String) -> Result<(), String> {
    kotone_core::history::delete(&session_id, &ts)
}

/// 读取历史记录的音频文件字节（~/.kotone/history/audio/<file_name>）。
/// file_name 只允许纯文件名：含 /、\\、.. 或为空直接拒绝（防路径穿越）。
#[tauri::command]
fn read_history_audio(file_name: String) -> Result<Vec<u8>, String> {
    if file_name.is_empty()
        || file_name.contains('/')
        || file_name.contains('\\')
        || file_name.contains("..")
    {
        return Err(format!("非法的音频文件名：{file_name}"));
    }
    let path = kotone_core::history::history_dir()
        .join("audio")
        .join(&file_name);
    std::fs::read(&path).map_err(|e| format!("读取音频 {} 失败：{e}", path.display()))
}

// ---------- 进程资源占用 ----------

/// sysinfo System 常驻进程内存：CPU 百分比依赖两次 refresh 的间隔采样，
/// 复用同一实例才能保证前端每次轮询拿到增量而非 0。
struct ResourceMonitor(Mutex<sysinfo::System>);

/// 当前进程资源占用（前端每 2s 轮询一次）
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceUsage {
    /// CPU 占用百分比（保留 1 位小数）。sysinfo 语义：自上次 refresh 以来
    /// 的增量采样，首次调用可能为 0；前端 2s 轮询正好提供采样间隔。
    pub cpu_percent: f32,
    /// 常驻内存（字节）
    pub memory_bytes: u64,
}

#[tauri::command]
fn get_resource_usage(monitor: tauri::State<ResourceMonitor>) -> ResourceUsage {
    let pid = sysinfo::Pid::from_u32(std::process::id());
    let mut sys = monitor.0.lock().unwrap();
    // sysinfo 0.30 无按 pid 过滤的 refresh（ProcessesToUpdate 是 0.32+ 的 API），
    // 全量刷新后只取本进程；2s 轮询频率下开销可接受。
    sys.refresh_processes();
    let (cpu_percent, memory_bytes) = sys
        .process(pid)
        .map(|p| ((p.cpu_usage() * 10.0).round() / 10.0, p.memory()))
        .unwrap_or((0.0, 0));
    ResourceUsage {
        cpu_percent,
        memory_bytes,
    }
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
/// （openChatKey → Unicode/剪贴板 → sendKey，直发当前前台窗口）。
///
/// P0（用户反馈「SendInput 被 360 拦截后卡死」）：此前是同步命令，在主线程
/// 直接调 SendInput——被安全软件钩住挂起时整个主进程（UI/托盘/IPC）冻结。
/// 改为 async + spawn_blocking + 10s 超时：挂起只阻塞工作线程，超时后返回
/// 清晰错误，UI 保持响应。
#[tauri::command]
async fn simulate_send(
    state: tauri::State<'_, SharedState>,
    text: String,
    profile_id: Option<String>,
) -> Result<(), InjectError> {
    let profile = profile_id
        .as_deref()
        .and_then(profile::get)
        .unwrap_or_else(GameProfile::builtin_generic);
    let injector = state.injector.clone();
    let task = tauri::async_runtime::spawn_blocking(move || {
        injector.send(&text, &profile, CancelToken::default())
    });
    const SEND_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
    match tokio::time::timeout(SEND_TIMEOUT, task).await {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => Err(InjectError::new("发送线程异常".to_string())),
        Err(_) => Err(InjectError::new(
            "发送超时（10s）：模拟输入未在限时内完成，通常是被安全软件或游戏反作弊拦截".to_string(),
        )),
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // 见函数注释：自启动提权必须先于单实例插件的互斥锁创建。
    #[cfg(windows)]
    auto_elevate_before_tauri_init();

    let startup_options = StartupOptions {
        onboarding: parse_onboarding_launch_mode(std::env::args()),
        background: has_background_flag(std::env::args()),
    };
    // 隔离实例（设了 KOTONE_HOME：开发、测试、外部 Agent 的独立实例）不参与单实例：
    // 否则正式安装在运行时，隔离实例一启动就把正式实例的窗口唤到前台然后退出。
    // 隔离实例的配置要自己关掉热键，免得和正式实例抢按键（见 scripts/manual-e2e/README.md）。
    let mut builder = tauri::Builder::default();
    if std::env::var_os("KOTONE_HOME").is_none() {
        builder = builder
            // 单实例：第二实例启动时唤起已有实例的设置窗口并退出自身，
            // 避免旧进程未退出导致热键「already registered」与 WebView2 类注册冲突
            .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
                // Agent 以后台方式再次启动：实例已在运行，保持静默，不唤起窗口、不抢焦点
                if has_background_flag(args.iter().map(String::as_str)) {
                    return;
                }
                if let Some(win) = app.get_webview_window("main") {
                    let _ = win.show();
                    let _ = win.unminimize();
                    let _ = win.set_focus();
                }
                // 测试时常从已在托盘运行的实例再次执行
                // `kotone.exe --onboarding=always`。第二实例不会进入 setup，
                // 因此必须把强制打开请求转发给现有设置窗口。
                if parse_onboarding_launch_mode(args.iter().map(String::as_str))
                    == OnboardingLaunchMode::Always
                {
                    let _ = app.emit_to("main", "kotone://open-onboarding", ());
                }
            }));
    }
    builder
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        // 关闭按钮不退出应用：main / overlay 窗口 CloseRequested 一律转 hide（托盘常驻）；
        // 仅托盘菜单「退出」（app.exit）真正结束进程。
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if matches!(window.label(), "overlay" | "danmaku") {
                    if let Some(win) = window.app_handle().get_webview_window(window.label()) {
                        hide_window(&win);
                    }
                } else {
                    let _ = window.hide();
                }
                api.prevent_close();
            }
        })
        .setup(move |app| {
            log::init();
            let previous_panic_hook = std::panic::take_hook();
            std::panic::set_hook(Box::new(move |info| {
                log::log(&format!("panic: {info}"));
                previous_panic_hook(info);
            }));
            log::log(&format!(
                "startup: version={} elevated={} arch={}",
                env!("CARGO_PKG_VERSION"),
                elevation::is_elevated(),
                std::env::consts::ARCH
            ));
            tray::setup_tray(app.handle())?;
            app.manage(startup_options.clone());

            // 首次运行：默认配置 + 内置 profile 落盘（~/.kotone/）
            let loaded_settings = settings::load_with_diagnostic();
            let settings_load_warning = loaded_settings.warning;
            let settings = loaded_settings.settings;
            process_log::set_recording_enabled(settings.diagnostics.recording);
            let mut app_started =
                process_log::ProcessEvent::new(process_log::app_session_id(), "app_started");
            app_started.context.engine_id = Some(settings.stt_engine.clone());
            app_started.context.model_id =
                Some(model::active_model_from(&settings, &settings.stt_engine));
            app_started.context.profile_id = settings.active_profile_id.clone();
            app_started.context.elevated = Some(elevation::is_elevated());
            let _ = process_log::record(app_started);

            // 自启动提权已前移到 run() 顶部（auto_elevate_before_tauri_init）：
            // 单实例插件初始化早于 setup，父进程若在此才 runas 退出，提权子进程
            // 会被单实例锁误判为第二实例而自杀，导致「双击无反应」。

            if let Err(e) = profile::ensure_builtin() {
                eprintln!("[kotone] 内置 profile 落盘失败: {e}");
            }

            // silero VAD 模型随二进制打包，首次启动解压到模型目录；
            // 失败仅记日志不阻断启动（one-shot 判停缺失时 begin 会报清晰错误）
            match model::ensure_vad_model() {
                Ok(written) => {
                    if written {
                        log::log("silero VAD 模型已写入模型目录");
                    }
                }
                Err(e) => log::log(&format!("silero VAD 模型写入失败: {e}")),
            }

            // 用量账本（usage.jsonl）：之后的在线调用都记进去。
            usage::start();
            // 旧版提醒沿用自己的音色；琴音角色在选择体验后才初始化。
            let companion_enabled = !settings.ui.legacy_upgrade.uses_legacy_assistant();
            if companion_enabled {
                roles::remember(&settings.agent.role_id);
            }
            let settings = Arc::new(RwLock::new(settings));
            let settings_repository = Arc::new(SettingsRepository::new(settings.clone()));
            let mut registry = EngineRegistry::new();
            kotone_stt::register_builtin(&mut registry);
            let engines = Arc::new(registry);
            let mut processor_registry = kotone_core::postprocess::ProcessorRegistry::new();
            let secrets: Arc<dyn SecretStore> = Arc::new(kotone_ai::secrets::KeyringSecretStore);
            let connections: Arc<dyn ConnectionResolver> = Arc::new(
                kotone_ai::secrets::SecretBackedResolver::new(settings.clone(), secrets.clone()),
            );
            if let Err(error) = kotone_postprocess::register_with_connections(
                &mut processor_registry,
                connections.clone(),
            ) {
                return Err(Box::<dyn std::error::Error>::from(error));
            }
            let processors = Arc::new(processor_registry);
            let emitter: Arc<dyn Emitter> = Arc::new(TauriEmitter {
                app: app.handle().clone(),
                vis_gen: Arc::new(std::sync::atomic::AtomicU64::new(0)),
                hidden_for_companion: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            });
            let injector: Arc<dyn Injector> = Arc::new(WindowsInjector);
            let focus: Arc<dyn FocusBackend> = Arc::new(WinFocusBackend);
            // 语音唤醒（ADR 017）：唤醒后的那次会话改用唤醒线程转来的音频，其余照常开麦克风。
            let wake_runtime = wake::WakeRuntime::default();
            let audio_backend: Arc<dyn kotone_core::audio::AudioBackend> =
                Arc::new(wake::WakeAudioBackend {
                    inner: Arc::new(platform_audio::CpalBackend),
                    handoff: wake_runtime.handoff(),
                });
            app.manage(wake_runtime);
            app.manage(voice_ask::VoiceAsk::default());
            app.manage(bubble::BubbleRuntime::default());
            app.manage(speech_screen::SpeechScreen::default());
            app.manage(computer_use::ComputerUse::default());
            app.manage(voice_chat::VoiceChat::default());
            app.manage(voice_listen::VoiceListen::default());
            app.manage(key_request::KeyRequests::default());
            let assistant_runtime =
                assistant::initialize(app.handle(), settings.clone(), connections.clone());

            #[allow(unused_mut)] // vad-silero feature 关闭时无可变接线
            let mut orchestrator = Orchestrator::new(
                settings.clone(),
                engines.clone(),
                audio_backend,
                injector.clone(),
                focus,
                emitter,
            )
            .with_processors(processors.clone())
            .with_secrets(secrets.clone())
            .with_settings_repository(settings_repository.clone());
            if let Ok(scheduler) = &assistant_runtime.scheduler {
                orchestrator = orchestrator
                    .with_assistant(assistant_runtime.handler.clone(), scheduler.clone());
            }
            let capture_app = app.handle().clone();
            orchestrator = orchestrator
                .with_before_capture(Arc::new(move || assistant::before_capture(&capture_app)));
            let capture_app = app.handle().clone();
            orchestrator = orchestrator
                .with_after_capture(Arc::new(move || assistant::after_capture(&capture_app)));
            // VAD 接线（ADR-007）：vad-silero feature 开启时注入 silero 工厂；
            // 默认构建不接入——one-shot 模式 begin 会报清晰错误
            #[cfg(feature = "vad-silero")]
            {
                orchestrator.vad_factory = Some(kotone_stt::vad::silero_factory());
            }
            let orchestrator = orchestrator.into_arc();
            orchestrator.suspend_runtime_inputs();

            app.manage(SharedState {
                settings: settings.clone(),
                settings_repository,
                settings_load_warning: Mutex::new(settings_load_warning),
                orchestrator: orchestrator.clone(),
                engines,
                processors,
                connections,
                secrets,
                injector,
            });
            app.manage(HotkeyManager::new(app.handle(), orchestrator.clone()));
            app.manage(RuntimeManager::new());
            app.manage(ResourceMonitor(Mutex::new(sysinfo::System::new())));
            app.manage(overlay::OverlayManager::default());
            app.manage(assistant_runtime);
            app.manage(speech_models::SpeechModelManager::default());
            app.manage(ocr_engines::OcrEngines::new(app.handle()));
            app.manage(companion::CompanionRuntime::default());
            app.manage(automation::AutomationManager::open());
            if companion_enabled {
                automation::ensure_builtin(app.handle());
            }
            automation::start_schedules(app.handle());
            // 内置琴音 Agent：对话存储与运行状态（ADR 014 / 015）
            app.manage(agent::AgentRuntime::open(companion_enabled));
            if companion_enabled {
                // 琴音原来的名字和头像记进她的角色卡（只做一次）。
                roles::migrate(app.handle());
                // 填充话（“我看看”“稍等哈”……）先合成好（ADR 029 §四）。等启动的事忙完再做。
                let fillers_app = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    fillers::refresh(&fillers_app);
                });
            }
            // 对局复盘：盯着游戏进程，一局结束就整理（agent.matchRecap，默认开）。
            recap::start(app.handle());
            // 先接上程序执行端口，再启动调度器（到点的程序才有人执行）。
            automation::attach(app.handle());
            assistant::start(app.handle());
            // 游戏语音（ADR 020）：设置里开着就开始转发（之后跟着设置保存启停）。
            voice_chat::sync(app.handle());
            // 可选本机工具服务：独立于 ASR 运行时，只在启动时读取开关（docs/tool-service.md）
            let tool_service_enabled = settings.read().unwrap().tool_service.enabled;
            tool_service::start(app.handle(), tool_service_enabled);
            compatibility::start_fullscreen_monitor(app.handle().clone());

            // 启动时的窗口（设置、弹幕、气泡）在全部状态 manage() 之后才建：页面一加载就会发 IPC，
            // 走快速 IPC（ipc.localhost）时可能早于 setup 结束到达，命令取不到状态会崩溃。
            create_startup_windows(app.handle(), companion_enabled);
            for label in ["main", "overlay", "danmaku"] {
                if let Some(win) = app.get_webview_window(label) {
                    harden_webview(&win);
                }
            }
            // 开发调试：`pnpm tauri dev -- --console` 启动后自动打开 webview 控制台。
            // `open_devtools` 只在 Tauri 的开发构建中可用，必须从 release 编译中移除。
            #[cfg(debug_assertions)]
            {
                if std::env::args().any(|a| a == "--console") {
                    if let Some(win) = app.get_webview_window("main") {
                        win.open_devtools();
                    }
                }
            }
            overlay::reconcile(app.handle(), false);

            // 应用级热键（启动 / 停止 Kotone）：runtime 未运行时也必须生效，
            // 故在启动阶段就落到低级钩子上（未配置时空操作，不安装钩子）。
            {
                let manager = app.state::<HotkeyManager>();
                let app_hotkeys = settings.read().unwrap().clone();
                manager.apply_app_hotkeys(&app_hotkeys);
            }
            // 语音识别还没就绪：按语音输入热键 / 呼叫键只提示怎么启动，键照常传给游戏（D8）。
            runtime::refresh_hints(app.handle());
            tray::refresh(app.handle());

            // 语音识别运行时（core runtime 状态机）默认 Stopped。ADR 028：在线识别打开就能用，
            // 直接 start；本地识别只有开了「打开时自动启动」（ui.autoStart）才自动启动
            // （warmup → 注册热键 → 显示悬浮窗，进度经 kotone://runtime 推送）。
            if runtime::should_start_on_launch(&settings.read().unwrap()) {
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
            // `--background`（Agent 启动）只驻留托盘，不显示窗口、不抢焦点
            let background = app.state::<StartupOptions>().background;
            if !background {
                if let Some(win) = app.get_webview_window("main") {
                    let _ = win.show();
                    let _ = win.set_focus();
                }
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            ping,
            export_diagnostics,
            clear_diagnostics,
            log_frontend_error,
            get_startup_options,
            get_settings,
            get_overlay_health,
            get_overlay_state,
            list_post_processors,
            list_connection_presets,
            list_connections,
            upsert_connection,
            delete_connection,
            test_post_processing,
            get_settings_load_warning,
            update_settings,
            list_audio_devices,
            set_audio_device,
            list_stt_engines,
            set_stt_engine,
            get_engine_options,
            list_profiles,
            save_profile,
            export_hotwords,
            export_builtin_blocklist,
            import_hotwords,
            export_profile,
            import_profile,
            delete_profile,
            get_profile_icon,
            save_overlay_position,
            get_elevation_status,
            get_hotkey_status,
            check_input_environment,
            detect_hotkey_conflicts,
            start_hotkey_capture,
            cancel_hotkey_capture,
            restart_as_admin,
            get_autostart_enabled,
            set_autostart_enabled,
            preview_sfx,
            list_models,
            download_model,
            cancel_download,
            get_model_install_guide,
            open_model_dest_dir,
            set_active_model,
            get_runtime_status,
            start_runtime,
            stop_runtime,
            get_models_dir,
            set_models_dir,
            delete_model,
            open_models_dir,
            open_history_dir,
            open_external,
            get_history,
            clear_history,
            delete_history_record,
            read_history_audio,
            get_resource_usage,
            confirm_send,
            cancel_session,
            simulate_send,
            assistant::list_reminders,
            assistant::cancel_reminder,
            assistant::assistant_submit_text,
            assistant::cancel_assistant_request,
            assistant_input::assistant_voice_begin,
            assistant_input::assistant_voice_end,
            assistant_input::assistant_voice_confirm,
            assistant_input::assistant_voice_cancel,
            assistant::list_speech_voices,
            assistant::preview_reminder,
            assistant::assistant_call,
            assistant::stop_speech,
            assistant::get_danmaku,
            assistant::finish_danmaku,
            speech_models::get_speech_model_status,
            speech_models::download_speech_model,
            wake::wake_status,
            bubble::bubble_snapshot,
            bubble::bubble_preview,
            wake::wake_download_model,
            speech_models::cancel_speech_model_download,
            automation::automation_runs,
            automation::automation_saved_list,
            automation::automation_get,
            automation::automation_save,
            automation::automation_delete,
            automation::automation_run_saved,
            automation::automation_stop,
            usage::usage_summary,
            usage::usage_conversation,
            roles::roles_list,
            roles::roles_get,
            roles::roles_switch,
            roles::roles_update,
            roles::roles_create,
            roles::roles_delete,
            roles::roles_export,
            roles::roles_import,
            roles::roles_check_wake,
            agent::agent_send,
            workspace_cmd::workspace_overview,
            key_request::agent_submit_key,
            game_tools::game_icon,
            game_tools::launch_installed_game,
            workspace_cmd::workspace_read,
            workspace_cmd::workspace_write,
            workspace_cmd::workspace_delete_game,
            workspace_cmd::workspace_set_avatar,
            workspace_cmd::workspace_avatar,
            workspace_cmd::workspace_delete_skill,
            workspace_cmd::workspace_import_skill,
            workspace_cmd::workspace_open_dir,
            agent::agent_stop,
            agent::agent_answer,
            agent::agent_state,
            agent::agent_tools,
            agent::agent_conversation,
            agent::agent_new_conversation,
            agent::agent_select_conversation,
            agent::agent_delete_conversation,
            agent::agent_subagent_messages,
            agent::agent_subagent_stop,
            agent::agent_subagent_continue,
            agent::agent_subagent_answer,
            agent::agent_set_permissions,
            agent::agent_set_automation_hidden,
            companion::companion_self_check,
            companion::connection_self_check,
            companion::connection_models,
            game_tools::installed_games,
            companion::companion_status,
            companion::companion_clear_chat_region,
            model_catalog::list_catalog_models,
            model_catalog::download_catalog_model,
            model_catalog::import_catalog_model,
            model_catalog::delete_catalog_model,
            model_catalog::activate_catalog_model,
            model_catalog::get_catalog_install_guide,
            model_catalog::open_catalog_model_dir,
            voice_chat::voice_chat_status,
            voice_chat::voice_chat_install,
            voice_chat::voice_chat_default_mic,
            voice_chat::voice_chat_test,
            voice_chat::voice_chat_say,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Kotone application")
        .run(|app, event| {
            if matches!(event, tauri::RunEvent::Exit) {
                tool_service::shutdown(app);
                automation::shutdown(app);
                voice_listen::stop_all(app);
                assistant::shutdown(app);
                wake::shutdown(app);
            }
        });
}
