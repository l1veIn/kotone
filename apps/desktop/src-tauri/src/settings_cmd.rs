//! 设置命令：读取 / 更新设置（含热键重新注册）、麦克风、语音识别引擎与选项。

use super::*;

#[tauri::command]
pub(crate) fn get_settings(state: tauri::State<SharedState>) -> Settings {
    state.settings.read().unwrap().clone()
}

/// 启动时配置恢复诊断；主设置页读取后以 toast 明确告知用户备份路径。
#[tauri::command]
pub(crate) fn get_settings_load_warning(state: tauri::State<SharedState>) -> Option<String> {
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
pub(crate) fn update_settings(
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
            next.ui.sidebar.normalize();
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

    crate::music::settings_changed(&old.music, &updated.music);

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
pub(crate) fn list_audio_devices() -> Vec<AudioDevice> {
    platform_audio::list_devices()
}

#[tauri::command]
pub(crate) fn set_audio_device(state: tauri::State<SharedState>, id: String) -> Result<(), String> {
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
pub(crate) async fn list_stt_engines(
    state: tauri::State<'_, SharedState>,
) -> Result<Vec<EngineInfo>, String> {
    let engines = state.engines.clone();
    tauri::async_runtime::spawn_blocking(move || engines.list_info())
        .await
        .map_err(|error| format!("列举引擎失败：{error}"))
}

#[tauri::command]
pub(crate) fn set_stt_engine(
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
pub(crate) fn get_engine_options(
    state: tauri::State<SharedState>,
    id: String,
) -> serde_json::Value {
    let guard = state.settings.read().unwrap();
    guard
        .engine_options
        .get(&id)
        .cloned()
        .unwrap_or(serde_json::Value::Null)
}

// ---------- 游戏 profile（§5.3） ----------

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
