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

/// 补丁会不会改到热键（热键表、旧的热键字段、呼叫键和快捷指令所在的 `assistant`、插件开关、0.1.x 升级状态）。
fn touches_hotkeys(patch: &serde_json::Value) -> bool {
    const KEYS: &[&str] = &[
        "hotkeys",
        "hotkey",
        "assistant",
        "plugins",
        "ui",
        "channelCycleHotkey",
        "resendLastHotkey",
        "togglePostProcessingHotkey",
        "cyclePostProcessingHotkey",
        "runtimeToggleHotkey",
        "automationStopHotkey",
    ];
    patch
        .as_object()
        .is_some_and(|object| KEYS.iter().any(|key| object.contains_key(*key)))
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
        // 语音输入插件开关：语音输入热键跟着登记 / 撤掉（ADR 038）
        || crate::plugins::voice_input_on(old) != crate::plugins::voice_input_on(next)
        // 热键表（ADR 040 D8）：上面的旧字段都是它的镜像，表里别的条目（更新的版本写的）变了也重新算一遍；
        // 自己加的（快捷指令……，第 3 期）键变了、加了 / 删了一条、换了种类都要重新登记（共享资源评估第二节第 13 条），
        // 说的话、自动化的入参按下时才读，不用
        || hotkey_table_changed(old, next)
}

/// 热键表里登记要看的那部分变了没有（[`kotone_core::hotkey_custom::registration_view`]）。
fn hotkey_table_changed(old: &Settings, next: &Settings) -> bool {
    use kotone_core::hotkey_custom::registration_view;
    registration_view(&old.hotkeys) != registration_view(&next.hotkeys)
}

/// 应用级热键（启动 / 停止 Kotone）变化：与 runtime 是否运行无关，必须重注册。
fn app_hotkey_changed(old: &Settings, next: &Settings) -> bool {
    old.runtime_toggle_hotkey != next.runtime_toggle_hotkey
        || old.automation_stop_hotkey != next.automation_stop_hotkey
        || old.ui.legacy_upgrade.uses_legacy_assistant()
            != next.ui.legacy_upgrade.uses_legacy_assistant()
        // 谁先拿到键按表的顺序算（ADR 040 D6）：呼叫键变了，停下热键可能从“被占了”变成能登记；
        // 自己加的不用听的（运行自动化、调工具、打开页面，第 3 期）也随应用登记
        || hotkey_table_changed(old, next)
        // 它们能不能按看权限和插件开关（含第三方插件的信任）：变了重新算谁登记
        || (crate::hotkey_bind::has_tool_bindings(next)
            && (old.agent.permissions != next.agent.permissions || old.plugins != next.plugins))
}

/// 「设置 → 热键」的确认（ADR 040 D9）：哪一条、玩家看到的工具和参数、这个工具这时的副作用。
pub(crate) struct Confirmation<'a> {
    pub row: &'a str,
    pub call: (String, serde_json::Value),
    pub effects: Vec<String>,
}

/// 局部更新配置；热键变化时触发重注册
#[tauri::command]
pub(crate) fn update_settings(
    app: AppHandle,
    state: tauri::State<SharedState>,
    patch: serde_json::Value,
) -> Result<Settings, String> {
    apply_update(app, state, patch, None)
}

/// 补丁（界面的通用保存、她和外部 Agent 的 `settings.update`）和「设置 → 热键」的确认共用这一条路：校验、对齐热键表、
/// 冲突、登记。补丁写进来的热键确认一律抹掉（D9）；只有 `confirmation`（确认命令）在那之后写确认。
fn apply_update(
    app: AppHandle,
    state: tauri::State<SharedState>,
    patch: serde_json::Value,
    confirmation: Option<Confirmation<'_>>,
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
    // 改到热键的补丁要查新冲突（和游戏配置的聊天键也比），游戏配置在设置事务外先读好（锁里不读磁盘）
    let profiles = touches_hotkeys(&patch).then(kotone_core::profile::list);
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
            // 补丁里写进模型配置的 Key（火山 Access Token、讯飞 APISecret……）存进凭据库，不进 config.json（ADR 048）
            kotone_core::model_secrets::take(next, &previous, state.secrets.as_ref())?;
            // 快捷指令的旧写法照旧受旧的限制（最多 8 条、话 1 到 200 字……）：对齐以后旧字段只是表的镜像，先查
            if next.assistant.companion.quick_messages
                != previous.assistant.companion.quick_messages
            {
                next.assistant.companion.validate()?;
            }
            // 热键：旧字段（hotkey.key、快捷指令……）改了就改表，再照表写回（ADR 040 D8）；下面的检查看的是对齐以后的值
            next.reconcile_hotkeys(&previous);
            // 绑的时候问权限（ADR 040 D9）：补丁写进来的确认不算，只有「设置 → 热键」的确认命令能写
            next.drop_unconfirmed(&previous);
            if let Some(confirm) = &confirmation {
                let index = kotone_core::hotkey_custom::find_row(&next.hotkeys, confirm.row)
                    .ok_or("这条自己加的热键已经没有了，刷新一下再确认")?;
                let (tool, args) = &confirm.call;
                if !crate::hotkey_bind::same_call(&next.hotkeys.bindings[index], tool, args) {
                    return Err("这条热键刚被改过，刷新一下再确认".into());
                }
                next.confirm_custom(confirm.row, &confirm.effects)?;
            }
            next.hotkeys.validate_custom()?;
            next.overlay.normalize_interaction();
            next.assistant.normalize_voices();
            next.ui.sidebar.normalize();
            next.assistant.validate()?;
            // 新造成的热键冲突不让存（和别的热键、Esc、游戏配置的聊天键；已经冲突着、这次没动的不拦，ADR 040 D6）
            if let Some(error) = crate::hotkey_table_cmd::new_conflict(
                &previous,
                next,
                profiles.as_deref().unwrap_or_default(),
            ) {
                return Err(error);
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
    // 悬浮窗的小窗改了宽度 / 最高：马上按新的大小重画（显示着的话窗口跟着变）
    crate::floating::settings_changed(&app, &old.assistant.bubble, &updated.assistant.bubble);
    // 设置里关了内置浏览器（ADR 038）：开着的网页都关掉（界面上右侧栏的“网页”跟着设置收起来）
    if old.browser.enabled && !updated.browser.enabled {
        crate::browser::switched_off(&app);
    }
    let voice_on = crate::plugins::voice_input_on(&updated);
    let voice_switched = crate::plugins::voice_input_on(&old) != voice_on;
    let old_overlay = old.overlay;

    // overlay 配置变化（含语音输入插件开关：悬浮窗只属于语音输入）→ 立即重排几何/点击穿透/显隐
    // + 通知前端（无需重启）
    if old_overlay != updated.overlay || voice_switched {
        if voice_switched && voice_on && updated.overlay.visibility != OverlayVisibility::Never {
            overlay::prepare_enable(&app);
        }
        overlay::reconcile(
            &app,
            (old_overlay.visibility != updated.overlay.visibility || voice_switched)
                && updated.overlay.visibility != OverlayVisibility::Never
                && voice_on,
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

/// 「设置 → 热键」的确认命令（[`crate::hotkey_bind::confirm_hotkey`]）：记下确认，重新登记，通知各窗口刷新。
pub(crate) fn confirm_from_backend(
    app: &AppHandle,
    row: &str,
    call: (String, serde_json::Value),
    effects: Vec<String>,
) -> Result<Settings, String> {
    use tauri::Emitter as _;
    let updated = apply_update(
        app.clone(),
        app.state(),
        serde_json::json!({}),
        Some(Confirmation { row, call, effects }),
    )?;
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

    fn quick(id: &str, hotkey: &str) -> kotone_core::assistant::QuickMessage {
        kotone_core::assistant::QuickMessage {
            id: id.into(),
            text: "帮我看看队友说了什么".into(),
            hotkey: hotkey.into(),
            ..Default::default()
        }
    }

    /// 旧写法改快捷指令（`settings.update` / 唤醒页）：和桌面端 `update_settings` 一样对齐进热键表。
    fn with_quick(
        old: &Settings,
        edit: impl FnOnce(&mut Vec<kotone_core::assistant::QuickMessage>),
    ) -> Settings {
        let mut next = old.clone();
        edit(&mut next.assistant.companion.quick_messages);
        next.reconcile_hotkeys(old);
        next
    }

    /// 共享资源评估第二节第 13 条：语音识别开着时改快捷指令的键，要立即重新登记。快捷指令在热键表里（第 3 期），
    /// 每条用自己的 id 登记：键变了、加了一条带键的、删了一条、从“说一句”换成“运行自动化”（随识别 → 随应用）都算；
    /// 加了一条没绑键的、只改说的话、只改自动化的入参不算（按下时才读）。
    #[test]
    fn changing_a_quick_message_hotkey_requires_registration() {
        let mut old = Settings::default();
        old.assistant.companion.quick_messages = vec![quick("q1", ""), quick("q2", "F7")];
        old.reconcile_hotkeys(&Settings::default());
        let next = with_quick(&old, |q| q[0].hotkey = "F6".into());
        assert!(hotkey_registration_changed(&old, &next), "绑了一个键");
        let next = with_quick(&old, |q| q[1].hotkey = "F8".into());
        assert!(hotkey_registration_changed(&old, &next), "换了一个键");
        let next = with_quick(&old, |q| {
            q.remove(1);
        });
        assert!(hotkey_registration_changed(&old, &next), "删了一条带键的");
        let next = with_quick(&old, |q| q.push(quick("q3", "F9")));
        assert!(hotkey_registration_changed(&old, &next), "加了一条带键的");
        let next = with_quick(&old, |q| q[1].automation_id = "auto-1".into());
        assert!(
            hotkey_registration_changed(&old, &next) && app_hotkey_changed(&old, &next),
            "换成运行自动化：识别没在跑也登记"
        );
        let next = with_quick(&old, |q| q.push(quick("q3", "")));
        assert!(
            !hotkey_registration_changed(&old, &next),
            "加了一条没绑键的"
        );
        let next = with_quick(&old, |q| q[1].text = "我们推塔".into());
        assert_ne!(next.hotkeys, old.hotkeys, "话在表里改了");
        assert!(!hotkey_registration_changed(&old, &next), "只改了话");
        assert!(!app_hotkey_changed(&old, &next));
    }
}
