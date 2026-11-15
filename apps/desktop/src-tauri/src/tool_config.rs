//! 配置类工具（`kotone_tools::catalog::HOST_TOOLS`）的桌面实现：人在引导页与设置页里能做的
//! 配置，Agent 经工具服务也能做，且调用的是与设置页**同一个** Tauri 命令函数（校验、落盘、
//! 热键重注册、运行时状态推送完全一致）。
//!
//! - 参数按工具解析（未知字段 / 类型错误 → `INVALID_ARGUMENTS`）。
//! - 工具改动设置后发 `kotone://settings-changed`，打开着的设置窗口据此刷新，不会显示过期值；
//!   历史变更沿用 `kotone://history-updated`。
//! - 下载类工具受理即返回，进度与结果经工具事件总线发布（`model.download.*`、
//!   `speech.model.download.*`）；与设置页共用唯一的下载名额。
//! - API key 只写：`connections.upsert` 把它交给系统凭据库，任何返回值与日志里都不出现。

use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter as _, Listener as _, Manager};

use kotone_core::log;
use kotone_tools::catalog as c;
use kotone_tools::host::HostError;

use crate::{tool_service, SharedState};

type HostResult = Result<Value, HostError>;

fn parse<T: for<'de> Deserialize<'de>>(tool: &str, args: Value) -> Result<T, HostError> {
    serde_json::from_value(args)
        .map_err(|e| HostError::InvalidArguments(format!("{tool} 参数无效：{e}")))
}

fn to_json<T: serde::Serialize>(value: T) -> HostResult {
    serde_json::to_value(value).map_err(|e| HostError::Failed(format!("序列化结果失败：{e}")))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NoArgs {}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct IdArgs {
    id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PatchArgs {
    patch: Value,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ActivateArgs {
    engine_id: String,
    model_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DirArgs {
    dir: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UpsertConnectionArgs {
    connection: kotone_core::connection::Connection,
    api_key: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProfileArgs {
    profile: kotone_core::profile::GameProfile,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct KeyArgs {
    key: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EnabledArgs {
    enabled: bool,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HistoryListArgs {
    limit: Option<usize>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HistoryDeleteArgs {
    session_id: String,
    ts: String,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WindowArgs {
    window: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ClearDiagnosticsArgs {
    stop_recording: Option<bool>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PostprocessTestArgs {
    text: String,
    pipeline: Option<kotone_core::postprocess::PipelineConfig>,
}

fn settings_snapshot(app: &AppHandle) -> Value {
    serde_json::to_value(crate::get_settings(app.state())).unwrap_or(Value::Null)
}

/// 工具改动设置后通知设置窗口刷新（窗口未打开时无人接收）。
fn settings_changed(app: &AppHandle) {
    let _ = app.emit("kotone://settings-changed", settings_snapshot(app));
}

fn runtime_snapshot(app: &AppHandle) -> Value {
    let state = app.state::<SharedState>();
    let rt = app.state::<crate::runtime::RuntimeManager>();
    let settings = state.settings.read().unwrap();
    serde_json::to_value(rt.status(&settings, &state.engines, None)).unwrap_or(Value::Null)
}

/// 设置页命令的校验错误（反序列化 / 取值不合法）归为参数错误，其余是执行失败。
fn settings_error(message: String) -> HostError {
    if message.starts_with("配置项不合法") {
        HostError::InvalidArguments(message)
    } else {
        HostError::Failed(message)
    }
}

pub async fn invoke(app: &AppHandle, tool: &str, args: Value) -> HostResult {
    match tool {
        c::TOOL_SETTINGS_GET => {
            let _: NoArgs = parse(tool, args)?;
            Ok(settings_snapshot(app))
        }
        c::TOOL_SETTINGS_UPDATE => {
            let PatchArgs { patch } = parse(tool, args)?;
            if !patch.is_object() {
                return Err(HostError::InvalidArguments("patch 必须是对象".into()));
            }
            if patch.get("connections").is_some() {
                return Err(HostError::InvalidArguments(
                    "连接请用 connections.upsert / connections.delete 修改".into(),
                ));
            }
            if patch.get("models").and_then(|m| m.get("dir")).is_some() {
                return Err(HostError::InvalidArguments(
                    "模型目录请用 models.dir.set 修改（会迁移已有模型）".into(),
                ));
            }
            let updated =
                crate::update_settings(app.clone(), app.state(), patch).map_err(settings_error)?;
            settings_changed(app);
            Ok(json!({"settings": updated, "runtime": runtime_snapshot(app)}))
        }
        c::TOOL_AUDIO_DEVICES => {
            let _: NoArgs = parse(tool, args)?;
            let selected = app
                .state::<SharedState>()
                .settings
                .read()
                .unwrap()
                .audio_device_id
                .clone();
            Ok(json!({"devices": crate::list_audio_devices(), "selected": selected}))
        }
        c::TOOL_STT_ENGINES => {
            let _: NoArgs = parse(tool, args)?;
            let engines = crate::list_stt_engines(app.state()).await?;
            let current = app
                .state::<SharedState>()
                .settings
                .read()
                .unwrap()
                .stt_engine
                .clone();
            Ok(json!({"engines": engines, "current": current}))
        }
        c::TOOL_MODELS_LIST => {
            let _: NoArgs = parse(tool, args)?;
            let models = crate::list_models().await?;
            let dir = crate::get_models_dir(app.state());
            let (engine, model_id) = {
                let state = app.state::<SharedState>();
                let settings = state.settings.read().unwrap();
                let engine = settings.stt_engine.clone();
                let model_id = kotone_stt::model::active_model_from(&settings, &engine);
                (engine, model_id)
            };
            Ok(json!({
                "models": models,
                "dir": dir,
                "active": {"engine": engine, "modelId": model_id},
                "downloading": kotone_stt::download::is_active(),
            }))
        }
        c::TOOL_MODELS_DOWNLOAD => {
            let IdArgs { id } = parse(tool, args)?;
            if kotone_stt::model::multi_model_dir(&id).is_none()
                && id != kotone_stt::model::VAD_MODEL_ID
            {
                return Err(HostError::InvalidArguments(format!(
                    "未知模型 {id}（见 models.list）"
                )));
            }
            if kotone_stt::download::is_active() {
                return Err(HostError::Busy(
                    "已有模型下载进行中；等它完成或先 models.download.cancel".into(),
                ));
            }
            start_model_download(app, id.clone());
            Ok(json!({"status": "started", "id": id}))
        }
        c::TOOL_MODELS_DOWNLOAD_CANCEL => {
            let _: NoArgs = parse(tool, args)?;
            let active = kotone_stt::download::is_active();
            let speech = app.state::<crate::speech_models::SpeechModelManager>();
            if speech.is_downloading() {
                crate::speech_models::cancel_speech_model_download(speech);
            } else {
                crate::cancel_download();
            }
            Ok(json!({"requested": active}))
        }
        c::TOOL_MODELS_DELETE => {
            let IdArgs { id } = parse(tool, args)?;
            let outcome = crate::delete_model(app.clone(), app.state(), id)?;
            settings_changed(app);
            to_json(outcome)
        }
        c::TOOL_MODELS_ACTIVATE => {
            let ActivateArgs {
                engine_id,
                model_id,
            } = parse(tool, args)?;
            crate::set_active_model(app.clone(), app.state(), engine_id, model_id)?;
            settings_changed(app);
            Ok(runtime_snapshot(app))
        }
        c::TOOL_MODELS_DIR_SET => {
            let DirArgs { dir } = parse(tool, args)?;
            let migration = crate::set_models_dir(app.clone(), app.state(), dir)?;
            settings_changed(app);
            to_json(migration)
        }
        c::TOOL_MODELS_INSTALL_GUIDE => {
            let IdArgs { id } = parse(tool, args)?;
            to_json(crate::get_model_install_guide(id).map_err(HostError::InvalidArguments)?)
        }
        c::TOOL_SPEECH_STATUS => {
            let _: NoArgs = parse(tool, args)?;
            let package = crate::speech_models::get_speech_model_status(app.clone());
            let (voices, voices_error) =
                match crate::assistant::list_speech_voices(app.clone()).await {
                    Ok(voices) => (
                        serde_json::to_value(voices).unwrap_or(json!([])),
                        Value::Null,
                    ),
                    Err(error) => (json!([]), json!(error)),
                };
            Ok(json!({"package": package, "voices": voices, "voicesError": voices_error}))
        }
        c::TOOL_SPEECH_MODEL_DOWNLOAD => {
            let _: NoArgs = parse(tool, args)?;
            if crate::speech_models::get_speech_model_status(app.clone()).installed {
                return Ok(json!({"status": "installed"}));
            }
            if kotone_stt::download::is_active() {
                return Err(HostError::Busy(
                    "已有模型下载进行中；等它完成或先 models.download.cancel".into(),
                ));
            }
            start_speech_download(app);
            Ok(json!({"status": "started"}))
        }
        c::TOOL_CONNECTIONS_PRESETS => {
            let _: NoArgs = parse(tool, args)?;
            Ok(json!({"presets": crate::list_connection_presets()}))
        }
        c::TOOL_CONNECTIONS_LIST => {
            let _: NoArgs = parse(tool, args)?;
            Ok(json!({"connections": crate::list_connections(app.state())?}))
        }
        c::TOOL_CONNECTIONS_UPSERT => {
            let UpsertConnectionArgs {
                connection,
                api_key,
            } = parse(tool, args)?;
            let api_key = api_key.filter(|key| !key.trim().is_empty());
            crate::upsert_connection(app.state(), connection, api_key).map_err(settings_error)?;
            settings_changed(app);
            Ok(json!({"connections": crate::list_connections(app.state())?}))
        }
        c::TOOL_CONNECTIONS_DELETE => {
            let IdArgs { id } = parse(tool, args)?;
            crate::delete_connection(app.state(), id)?;
            settings_changed(app);
            Ok(json!({"connections": crate::list_connections(app.state())?}))
        }
        c::TOOL_PROFILES_LIST => {
            let _: NoArgs = parse(tool, args)?;
            let active = app
                .state::<SharedState>()
                .settings
                .read()
                .unwrap()
                .active_profile_id
                .clone();
            Ok(json!({"profiles": crate::list_profiles(), "activeProfileId": active}))
        }
        c::TOOL_PROFILES_SAVE => {
            let ProfileArgs { profile } = parse(tool, args)?;
            let id = profile.id.clone();
            crate::save_profile(profile)?;
            settings_changed(app);
            Ok(json!({"saved": id}))
        }
        c::TOOL_PROFILES_DELETE => {
            let IdArgs { id } = parse(tool, args)?;
            let outcome = crate::delete_profile(id)?;
            settings_changed(app);
            to_json(outcome)
        }
        c::TOOL_HOTKEYS_STATUS => {
            let _: NoArgs = parse(tool, args)?;
            to_json(crate::get_hotkey_status(app.clone()))
        }
        c::TOOL_HOTKEYS_CONFLICTS => {
            let KeyArgs { key } = parse(tool, args)?;
            let warnings = crate::detect_hotkey_conflicts(key.clone());
            Ok(json!({"key": key, "warnings": warnings}))
        }
        c::TOOL_INPUT_CHECK => {
            let _: NoArgs = parse(tool, args)?;
            to_json(crate::check_input_environment(app.clone()).await?)
        }
        c::TOOL_ELEVATION_STATUS => {
            let _: NoArgs = parse(tool, args)?;
            to_json(crate::get_elevation_status(app.state()))
        }
        c::TOOL_ELEVATION_RESTART => {
            let _: NoArgs = parse(tool, args)?;
            schedule_elevated_restart(app);
            Ok(json!({"status": "restarting"}))
        }
        c::TOOL_AUTOSTART_STATUS => {
            let _: NoArgs = parse(tool, args)?;
            Ok(json!({"enabled": crate::get_autostart_enabled()}))
        }
        c::TOOL_AUTOSTART_SET => {
            let EnabledArgs { enabled } = parse(tool, args)?;
            crate::set_autostart_enabled(enabled)?;
            Ok(json!({"enabled": crate::get_autostart_enabled()}))
        }
        c::TOOL_HISTORY_LIST => {
            let HistoryListArgs { limit } = parse(tool, args)?;
            let limit = limit.unwrap_or(50);
            if !(1..=1000).contains(&limit) {
                return Err(HostError::InvalidArguments(
                    "limit 必须在 1-1000 之间".into(),
                ));
            }
            let mut records = crate::get_history()?;
            let total = records.len();
            records.truncate(limit);
            Ok(json!({"records": records, "total": total, "truncated": total > limit}))
        }
        c::TOOL_HISTORY_DELETE => {
            let HistoryDeleteArgs { session_id, ts } = parse(tool, args)?;
            crate::delete_history_record(session_id, ts)?;
            let _ = app.emit("kotone://history-updated", ());
            Ok(json!({"deleted": true}))
        }
        c::TOOL_HISTORY_CLEAR => {
            let _: NoArgs = parse(tool, args)?;
            crate::clear_history()?;
            let _ = app.emit("kotone://history-updated", ());
            Ok(json!({"cleared": true}))
        }
        c::TOOL_DIAGNOSTICS_EXPORT => {
            let WindowArgs { window } = parse(tool, args)?;
            if let Some(w) = window.as_deref() {
                if !matches!(w, "24h" | "7d" | "all") {
                    return Err(HostError::InvalidArguments(
                        "window 只接受 24h / 7d / all".into(),
                    ));
                }
            }
            let dir = kotone_core::settings::kotone_dir().join("exports");
            std::fs::create_dir_all(&dir)
                .map_err(|e| HostError::Failed(format!("创建导出目录失败：{e}")))?;
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or_default();
            let path = dir.join(format!("kotone-diagnostics-{stamp}.zip"));
            let result = crate::export_diagnostics(
                app.clone(),
                path.to_string_lossy().into_owned(),
                window,
            )?;
            let mut value = to_json(result)?;
            if let Some(map) = value.as_object_mut() {
                map.insert("path".into(), json!(path.to_string_lossy()));
            }
            Ok(value)
        }
        c::TOOL_DIAGNOSTICS_CLEAR => {
            let ClearDiagnosticsArgs { stop_recording } = parse(tool, args)?;
            let stop = stop_recording.unwrap_or(false);
            crate::clear_diagnostics(app.clone(), stop)?;
            if stop {
                settings_changed(app);
            }
            Ok(json!({"cleared": true}))
        }
        c::TOOL_APP_RESOURCES => {
            let _: NoArgs = parse(tool, args)?;
            to_json(crate::get_resource_usage(app.state()))
        }
        c::TOOL_POSTPROCESS_PROCESSORS => {
            let _: NoArgs = parse(tool, args)?;
            Ok(json!({"processors": crate::list_post_processors(app.state())}))
        }
        c::TOOL_POSTPROCESS_TEST => {
            let PostprocessTestArgs { text, pipeline } = parse(tool, args)?;
            to_json(crate::test_post_processing(app.state(), text, pipeline).await?)
        }
        other => Err(HostError::Failed(format!("桌面端未实现工具 {other}"))),
    }
}

/// 后台下载模型：进度（至多每秒一次）与结果发布到工具事件总线。
fn start_model_download(app: &AppHandle, id: String) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let progress_app = app.clone();
        let progress_id = id.clone();
        let last = std::sync::Mutex::new(Instant::now() - Duration::from_secs(1));
        let listener = app.listen("kotone://download", move |event| {
            let Ok(payload) = serde_json::from_str::<Value>(event.payload()) else {
                return;
            };
            if payload["id"] != progress_id.as_str() {
                return;
            }
            let mut last = last.lock().unwrap();
            if last.elapsed() < Duration::from_secs(1) {
                return;
            }
            *last = Instant::now();
            tool_service::publish(
                &progress_app,
                "model.download.progress",
                json!({"id": progress_id, "downloadedBytes": payload["downloaded"], "totalBytes": payload["total"]}),
            );
        });
        let started = Instant::now();
        let result = crate::download_model(app.clone(), app.state(), id.clone()).await;
        app.unlisten(listener);
        let ms = started.elapsed().as_millis() as u64;
        match result {
            Ok(()) => {
                log::log(&format!("tool service: model download finished id={id}"));
                tool_service::publish(
                    &app,
                    "model.download.finished",
                    json!({"id": id, "durationMs": ms}),
                );
                settings_changed(&app);
            }
            Err(error) => {
                let cancelled = kotone_stt::download::is_cancelled(&error);
                tool_service::publish(
                    &app,
                    "model.download.failed",
                    json!({"id": id, "cancelled": cancelled, "message": error, "durationMs": ms}),
                );
            }
        }
    });
}

/// 后台下载 Kokoro 语音包：结果发布到工具事件总线（进度见 speech.status）。
fn start_speech_download(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let started = Instant::now();
        let result = crate::speech_models::download_speech_model(app.clone()).await;
        let ms = started.elapsed().as_millis() as u64;
        match result {
            Ok(status) => tool_service::publish(
                &app,
                "speech.model.download.finished",
                json!({"status": status, "durationMs": ms}),
            ),
            Err(error) => tool_service::publish(
                &app,
                "speech.model.download.failed",
                json!({"cancelled": kotone_stt::download::is_cancelled(&error) || error.contains("已取消"), "message": error, "durationMs": ms}),
            ),
        }
    });
}

/// 管理员重启：先让响应送出，再弹 UAC（等待用户不受工具期限限制）。用户同意后与设置页
/// 「以管理员权限重启」相同——立即退出，由新的提权实例接替；拒绝则发布
/// `elevation.restart.failed`，本实例继续运行。
fn schedule_elevated_restart(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        let result = tauri::async_runtime::spawn_blocking(
            kotone_platform_windows::elevation::restart_as_admin,
        )
        .await
        .map_err(|e| e.to_string())
        .and_then(|r| r);
        match result {
            Ok(()) => {
                log::log("tool service: elevation.restart accepted, exiting");
                app.exit(0);
            }
            Err(error) => {
                log::log(&format!("tool service: elevation.restart failed: {error}"));
                tool_service::publish(&app, "elevation.restart.failed", json!({"message": error}));
            }
        }
    });
}
