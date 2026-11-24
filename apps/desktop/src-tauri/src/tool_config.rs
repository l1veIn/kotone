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
use kotone_core::model_catalog::ModelCategory;
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

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
struct RecapArgs {
    game: Option<String>,
    since_minutes: Option<u64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RoleUpdateArgs {
    id: String,
    patch: kotone_agent::roles::RolePatch,
}

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
    #[serde(default)]
    engine_id: Option<String>,
    model_id: String,
    #[serde(default)]
    category: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ModelIdArgs {
    id: String,
    #[serde(default)]
    category: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DescribeArgs {
    #[serde(default)]
    query: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CategoryArgs {
    #[serde(default)]
    category: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CompanionAskArgs {
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    quick_message_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AutomationRunArgs {
    /// 临时程序（与 `id` 二选一）。
    #[serde(default)]
    source: Option<String>,
    /// 已保存的自动化。
    #[serde(default)]
    id: Option<String>,
    /// 已保存自动化的入参（按它的声明校验）。
    #[serde(default)]
    args: Option<serde_json::Map<String, Value>>,
    #[serde(default)]
    name: Option<String>,
    /// 延迟几秒后开始（与 `at` 二选一；都不传 = 立即）。
    #[serde(default)]
    delay_seconds: Option<u64>,
    /// 指定开始时刻（Unix 毫秒）。
    #[serde(default)]
    at: Option<u64>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AutomationRunsArgs {}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AutomationIdArgs {
    id: String,
}

/// 自动化相关错误的分类：名额满 = BUSY；运行环境问题（没有前台游戏、存储失败等）= FAILED；
/// 其余是调用方给的内容不对 = INVALID_ARGUMENTS。
fn automation_error(e: String) -> HostError {
    if e.contains("最多同时运行") {
        HostError::Busy(e)
    } else if e.contains("前台")
        || e.contains("尚未就绪")
        || e.contains("不可用")
        || e.contains("已关闭")
        || e.contains("无法启动")
        || ["写入", "保存", "创建", "读取"]
            .iter()
            .any(|p| e.starts_with(p))
    {
        HostError::Failed(e)
    } else {
        HostError::InvalidArguments(e)
    }
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GameFocusArgs {
    #[serde(default)]
    profile_id: Option<String>,
    #[serde(default)]
    process_name: Option<String>,
    #[serde(default)]
    title_contains: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GameIdArgs {
    profile_id: String,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct GameLaunchArgs {
    #[serde(default)]
    profile_id: Option<String>,
    #[serde(default)]
    installed_id: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct GameInstalledArgs {
    #[serde(default)]
    query: Option<String>,
}

/// 游戏窗口操作会短暂阻塞（切前台要等确认），放到阻塞线程池。
async fn blocking(work: impl FnOnce() -> Result<Value, String> + Send + 'static) -> HostResult {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|e| HostError::Failed(format!("工具线程异常：{e}")))?
        .map_err(HostError::Failed)
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AutomationStopArgs {
    #[serde(default)]
    id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CompanionTypeArgs {
    text: String,
    /// 缺省 = 跟随设置 `assistant.companion.autoSend`（与热键入口一致）。
    #[serde(default)]
    send: Option<bool>,
}

fn category_of(tool: &str, value: Option<&str>) -> Result<ModelCategory, HostError> {
    ModelCategory::parse(value.unwrap_or("")).ok_or_else(|| {
        HostError::InvalidArguments(format!("{tool}: category 只能是 asr、ocr 或 tts"))
    })
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
/// 设置在工具或陪玩流程里被改动后通知打开着的设置窗口刷新。
pub(crate) fn notify_settings_changed(app: &AppHandle) {
    settings_changed(app);
}

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
    // 插件的工具（ADR 038）：外部 Agent、MCP 经工具服务调到这里，结果照 MCP（content / structuredContent / _meta.ui）
    if crate::plugins::owner(tool).is_some() {
        return crate::plugins::call(app, tool, &args, None)
            .await
            .map(|result| result.to_service_json())
            .map_err(HostError::Failed);
    }
    match tool {
        c::TOOL_SETTINGS_GET => {
            let _: NoArgs = parse(tool, args)?;
            Ok(settings_snapshot(app))
        }
        c::TOOL_SETTINGS_DESCRIBE => {
            let DescribeArgs { query } = if args.is_null() {
                DescribeArgs::default()
            } else {
                parse(tool, args)?
            };
            let fields = kotone_core::settings_guide::describe(query.as_deref());
            let dedicated: Vec<Value> = kotone_core::settings_guide::INTERNAL
                .iter()
                .map(|(path, how)| json!({"path": path, "how": how}))
                .collect();
            Ok(json!({"fields": fields, "dedicated": dedicated}))
        }
        c::TOOL_WAKE_STATUS => {
            let _: NoArgs = parse(tool, args)?;
            Ok(serde_json::to_value(crate::wake::status(app)).unwrap_or(Value::Null))
        }
        c::TOOL_WAKE_MODEL_DOWNLOAD => {
            let _: NoArgs = parse(tool, args)?;
            crate::wake::download_model(app)
                .await
                .map_err(HostError::Failed)?;
            Ok(serde_json::to_value(crate::wake::status(app)).unwrap_or(Value::Null))
        }
        c::TOOL_APP_RESTART => {
            let _: NoArgs = parse(tool, args)?;
            schedule_restart(app);
            Ok(json!({"status": "restarting"}))
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
            let before = settings_snapshot(app);
            let updated =
                crate::update_settings(app.clone(), app.state(), patch).map_err(settings_error)?;
            settings_changed(app);
            let after = serde_json::to_value(&updated).unwrap_or(Value::Null);
            let runtime = runtime_snapshot(app);
            let effects = kotone_core::settings_guide::effects(
                kotone_core::settings_guide::changed_paths(&before, &after),
                runtime["restartNeeded"] == true,
            );
            Ok(json!({"settings": updated, "runtime": runtime, "effects": effects}))
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
            let CategoryArgs { category } = if args.is_null() {
                CategoryArgs::default()
            } else {
                parse(tool, args)?
            };
            let category = category_of(tool, category.as_deref())?;
            if category != ModelCategory::Asr {
                let list_app = app.clone();
                let models = tauri::async_runtime::spawn_blocking(move || {
                    crate::model_catalog::list_blocking(&list_app, category)
                })
                .await
                .map_err(|e| HostError::Failed(e.to_string()))??;
                let dir = crate::get_models_dir(app.state());
                return Ok(json!({
                    "category": category.as_str(),
                    "models": models,
                    "dir": dir,
                    "active": {"modelId": crate::model_catalog::active_id(app, category)},
                    "downloading": kotone_stt::download::is_active(),
                }));
            }
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
            let ModelIdArgs { id, category } = parse(tool, args)?;
            let category = category_of(tool, category.as_deref())?;
            if category != ModelCategory::Asr {
                let known = list_catalog(app, category)
                    .await?
                    .into_iter()
                    .find(|m| m.id == id)
                    .ok_or_else(|| {
                        HostError::InvalidArguments(format!("未知模型 {id}（见 models.list）"))
                    })?;
                if known.backend != kotone_core::model_catalog::CatalogBackend::Local
                    || known.size_bytes == 0
                {
                    return Err(HostError::InvalidArguments(format!("{id} 无需下载")));
                }
                if kotone_stt::download::is_active() {
                    return Err(HostError::Busy(
                        "已有模型下载进行中；等它完成或先 models.download.cancel".into(),
                    ));
                }
                start_catalog_download(app, category, id.clone());
                return Ok(json!({"status": "started", "id": id, "category": category.as_str()}));
            }
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
            let ModelIdArgs { id, category } = parse(tool, args)?;
            let category = category_of(tool, category.as_deref())?;
            if category != ModelCategory::Asr {
                let was_active = crate::model_catalog::delete(app, category, &id)?;
                settings_changed(app);
                return Ok(json!({"wasActive": was_active}));
            }
            let outcome = crate::delete_model(app.clone(), app.state(), id)?;
            settings_changed(app);
            to_json(outcome)
        }
        c::TOOL_MODELS_ACTIVATE => {
            let ActivateArgs {
                engine_id,
                model_id,
                category,
            } = parse(tool, args)?;
            let category = category_of(tool, category.as_deref())?;
            if category != ModelCategory::Asr {
                let activate_app = app.clone();
                let id = model_id.clone();
                tauri::async_runtime::spawn_blocking(move || {
                    crate::model_catalog::activate(&activate_app, category, &id)
                })
                .await
                .map_err(|e| HostError::Failed(e.to_string()))??;
                settings_changed(app);
                return Ok(json!({"category": category.as_str(), "activeModelId": model_id}));
            }
            let engine_id = engine_id.ok_or_else(|| {
                HostError::InvalidArguments("models.activate: asr 需要 engineId".into())
            })?;
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
            let ModelIdArgs { id, category } = parse(tool, args)?;
            let category = category_of(tool, category.as_deref())?;
            if category != ModelCategory::Asr {
                return to_json(
                    crate::model_catalog::install_guide(app, category, &id)
                        .map_err(HostError::InvalidArguments)?,
                );
            }
            to_json(crate::get_model_install_guide(id).map_err(HostError::InvalidArguments)?)
        }
        c::TOOL_SPEECH_STATUS => {
            let _: NoArgs = parse(tool, args)?;
            let package = crate::speech_models::get_speech_model_status(app.clone());
            let (voices, voices_error) =
                match crate::assistant::list_speech_voices(app.clone(), None).await {
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
        c::TOOL_USAGE_SUMMARY => {
            let _: NoArgs = parse(tool, args)?;
            let mut summary = crate::usage::summary();
            // 本段对话：模型、语音、读屏、搜索分开（正在回答的对话，没有就是当前对话）。
            if let Some((id, since)) = crate::agent::active_conversation(app) {
                summary["conversation"] = crate::usage::conversation(&id, since);
            }
            Ok(summary)
        }
        c::TOOL_GAME_RECAP => {
            let RecapArgs {
                game,
                since_minutes,
            } = parse(tool, args)?;
            crate::recap::recap_now(app, game, since_minutes)
                .await
                .map_err(HostError::InvalidArguments)
        }
        c::TOOL_ROLES_LIST => {
            let _: NoArgs = parse(tool, args)?;
            Ok(crate::roles::list(app))
        }
        c::TOOL_ROLES_GET => {
            let IdArgs { id } = parse(tool, args)?;
            crate::roles::get(app, &id).map_err(HostError::InvalidArguments)
        }
        c::TOOL_ROLES_SWITCH => {
            let IdArgs { id } = parse(tool, args)?;
            crate::roles::switch(app, &id).map_err(HostError::InvalidArguments)
        }
        c::TOOL_ROLES_CREATE => {
            let patch: kotone_agent::roles::RolePatch = parse(tool, args)?;
            crate::roles::create(app, patch).map_err(HostError::InvalidArguments)
        }
        c::TOOL_ROLES_UPDATE => {
            let RoleUpdateArgs { id, patch } = parse(tool, args)?;
            crate::roles::update(app, &id, patch).map_err(HostError::InvalidArguments)
        }
        c::TOOL_ROLES_DELETE => {
            let IdArgs { id } = parse(tool, args)?;
            crate::roles::delete(app, &id).map_err(HostError::InvalidArguments)
        }
        c::TOOL_POSTPROCESS_PROCESSORS => {
            let _: NoArgs = parse(tool, args)?;
            Ok(json!({"processors": crate::list_post_processors(app.state())}))
        }
        c::TOOL_POSTPROCESS_TEST => {
            let PostprocessTestArgs { text, pipeline } = parse(tool, args)?;
            to_json(crate::test_post_processing(app.state(), text, pipeline).await?)
        }
        c::TOOL_COMPANION_STATUS => {
            let _: NoArgs = parse(tool, args)?;
            Ok(crate::companion::status(app))
        }
        c::TOOL_COMPANION_ASK => {
            let CompanionAskArgs {
                text,
                quick_message_id,
            } = parse(tool, args)?;
            let text = match (text, quick_message_id) {
                (Some(text), None) if !text.trim().is_empty() => text,
                (None, Some(id)) => {
                    crate::companion::quick_message_text(app, &id).ok_or_else(|| {
                        HostError::InvalidArguments(format!(
                            "没有快捷指令 {id}（见 companion.status）"
                        ))
                    })?
                }
                _ => {
                    return Err(HostError::InvalidArguments(
                        "companion.ask 需要 text 或 quickMessageId 之一".into(),
                    ))
                }
            };
            if crate::companion::is_busy(app) {
                return Err(HostError::Busy("上一条陪玩请求还在处理".into()));
            }
            // 与热键入口相同的前置条件：没有通过自检的连接时直接拒绝，不受理。
            crate::companion::connection_ready(
                app,
                &app.state::<SharedState>().settings.read().unwrap().clone(),
            )
            .map_err(HostError::Failed)?;
            let job_id = kotone_tools::ids::new_id("cjob");
            let task_app = app.clone();
            let job = job_id.clone();
            tauri::async_runtime::spawn(async move {
                let started = Instant::now();
                match crate::companion::run(&task_app, text, crate::companion::Source::Tool).await {
                    Ok(mut receipt) => {
                        receipt["jobId"] = json!(job);
                        receipt["durationMs"] = json!(started.elapsed().as_millis() as u64);
                        tool_service::publish(&task_app, "companion.finished", receipt);
                    }
                    Err(message) => tool_service::publish(
                        &task_app,
                        "companion.failed",
                        json!({"jobId": job, "message": message, "durationMs": started.elapsed().as_millis() as u64}),
                    ),
                }
            });
            Ok(json!({"status": "started", "jobId": job_id}))
        }
        c::TOOL_COMPANION_READ_CHAT => {
            let _: NoArgs = parse(tool, args)?;
            let snapshot =
                crate::companion::read_chat(app, "", crate::companion::Source::Tool).await?;
            Ok(snapshot.to_json())
        }
        c::TOOL_COMPANION_TYPE => {
            let CompanionTypeArgs { text, send } = parse(tool, args)?;
            let send = send.unwrap_or_else(|| {
                app.state::<SharedState>()
                    .settings
                    .read()
                    .unwrap()
                    .assistant
                    .companion
                    .auto_send
            });
            Ok(crate::companion::type_text(app, &text, send).await?)
        }
        c::TOOL_COMPANION_CHECK => {
            let _: NoArgs = parse(tool, args)?;
            let task_app = app.clone();
            tauri::async_runtime::spawn(async move {
                match crate::companion::self_check(&task_app).await {
                    Ok(check) => tool_service::publish(
                        &task_app,
                        "companion.check.finished",
                        json!({"check": check}),
                    ),
                    Err(message) => tool_service::publish(
                        &task_app,
                        "companion.check.failed",
                        json!({"message": message}),
                    ),
                }
            });
            Ok(json!({"status": "started"}))
        }
        c::TOOL_AUTOMATION_RUN => {
            let AutomationRunArgs {
                source,
                id,
                args: run_args,
                name,
                delay_seconds,
                at,
            } = parse(tool, args)?;
            let now = kotone_core::automation::now_ms();
            let due = match (delay_seconds, at) {
                (Some(_), Some(_)) => {
                    return Err(HostError::InvalidArguments(
                        "delaySeconds 与 at 只能给一个".into(),
                    ))
                }
                (Some(d), None) if d > kotone_core::automation::MAX_DELAY_SECONDS => {
                    return Err(HostError::InvalidArguments(
                        "delaySeconds 最多 604800（7 天）".into(),
                    ))
                }
                (Some(d), None) => now + d * 1_000,
                (None, Some(at)) => at,
                (None, None) => now,
            };
            match (id, source) {
                (Some(id), None) => crate::automation::run_saved(
                    app,
                    "tool",
                    &id,
                    &run_args.unwrap_or_default(),
                    due,
                ),
                (None, Some(source)) if run_args.is_none() => crate::automation::start(
                    app,
                    "tool",
                    crate::automation::ProgramRequest {
                        name: name.as_deref().unwrap_or("Agent 自动化"),
                        source: &source,
                        automation_id: None,
                        args: Default::default(),
                    },
                    due,
                ),
                (None, Some(_)) => Err("args 只能和 id（已保存的自动化）一起用".into()),
                _ => Err("id 与 source 要给且只给一个".into()),
            }
            .map_err(automation_error)
        }
        c::TOOL_AUTOMATION_RUNS => {
            let _: AutomationRunsArgs = if args.is_null() {
                AutomationRunsArgs::default()
            } else {
                parse(tool, args)?
            };
            crate::automation::list(app).map_err(HostError::Failed)
        }
        c::TOOL_AUTOMATION_SAVE => {
            let draft: kotone_core::automation::library::AutomationDraft = parse(tool, args)?;
            crate::automation::save(
                app,
                draft,
                kotone_core::automation::library::CreatedBy::Agent,
            )
            .map_err(automation_error)
        }
        c::TOOL_AUTOMATION_GET => {
            let AutomationIdArgs { id } = parse(tool, args)?;
            crate::automation::get(app, &id).map_err(automation_error)
        }
        c::TOOL_AUTOMATION_LIST => {
            if !args.is_null() {
                let _: NoArgs = parse(tool, args)?;
            }
            crate::automation::list_saved(app).map_err(automation_error)
        }
        c::TOOL_AUTOMATION_DELETE => {
            let AutomationIdArgs { id } = parse(tool, args)?;
            crate::automation::delete(app, &id)
                .map(|deleted| json!({"deleted": deleted}))
                .map_err(automation_error)
        }
        c::TOOL_AUTOMATION_STOP => {
            let AutomationStopArgs { id } = if args.is_null() {
                AutomationStopArgs::default()
            } else {
                parse(tool, args)?
            };
            Ok(
                json!({"stopped": crate::automation::stop(app, id.as_deref(), "工具 automation.stop")}),
            )
        }
        c::TOOL_GAME_LIST => blocking(crate::game_tools::list).await,
        c::TOOL_GAME_FOCUS => {
            let a: GameFocusArgs = if args.is_null() {
                GameFocusArgs::default()
            } else {
                parse(tool, args)?
            };
            blocking(move || {
                crate::game_tools::focus(
                    a.profile_id.as_deref(),
                    a.process_name.as_deref(),
                    a.title_contains.as_deref(),
                )
            })
            .await
        }
        c::TOOL_GAME_LAUNCH => {
            let a: GameLaunchArgs = parse(tool, args)?;
            blocking(move || {
                crate::game_tools::launch(a.profile_id.as_deref(), a.installed_id.as_deref())
            })
            .await
        }
        c::TOOL_GAME_INSTALLED => {
            let a: GameInstalledArgs = if args.is_null() {
                GameInstalledArgs::default()
            } else {
                parse(tool, args)?
            };
            blocking(move || Ok(crate::game_tools::installed(a.query.as_deref()))).await
        }
        c::TOOL_GAME_STATE => {
            let game = args
                .get("game")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|g| !g.is_empty());
            Ok(crate::games::state_json(game))
        }
        c::TOOL_GAME_CLOSE => {
            let a: GameIdArgs = parse(tool, args)?;
            blocking(move || crate::game_tools::close(&a.profile_id)).await
        }
        c::TOOL_GAME_LOOK => {
            let a: GameLookArgs = if args.is_null() {
                GameLookArgs::default()
            } else {
                parse(tool, args)?
            };
            let zoom = match a.zoom {
                None => None,
                Some(z) if z.len() == 4 => Some([z[0], z[1], z[2], z[3]]),
                Some(_) => {
                    return Err(HostError::InvalidArguments(
                        "zoom 必须是 [x0, y0, x1, y1]".into(),
                    ))
                }
            };
            let app = app.clone();
            blocking(move || crate::computer_use::look(&app, zoom)).await
        }
        c::TOOL_GAME_ACT => {
            let a: GameActArgs = parse(tool, args)?;
            kotone_tools::computer_use::actions::parse_plan(&a.actions)
                .map_err(HostError::InvalidArguments)?;
            // 调用被丢弃（本轮停止、超时）时，守卫让进行中的操作停下并松开按住的键。
            let flag = std::sync::Arc::new(std::sync::Mutex::new(None));
            let _guard = crate::computer_use::CallGuard { flag: flag.clone() };
            let app = app.clone();
            blocking(move || crate::computer_use::act(&app, &a.observation_id, &a.actions, &flag))
                .await
        }
        c::TOOL_VOICE_CHAT_STATUS => Ok(crate::voice_chat::status(app)),
        c::TOOL_VOICE_CHAT_INSTALL => {
            let app = app.clone();
            blocking(move || crate::voice_chat::install(&app)).await
        }
        c::TOOL_VOICE_CHAT_DEFAULT_MIC => {
            let a: VoiceChatMicArgs = parse(tool, args)?;
            let use_cable = match a.r#use.as_str() {
                "cable" => true,
                "restore" => false,
                _ => {
                    return Err(HostError::InvalidArguments(
                        "use 只能是 cable 或 restore".into(),
                    ))
                }
            };
            let app = app.clone();
            blocking(move || crate::voice_chat::default_mic(&app, use_cable)).await
        }
        c::TOOL_VOICE_CHAT_SAY => {
            let a: VoiceChatSayArgs = parse(tool, args)?;
            let text = a.text.trim().to_string();
            if text.is_empty() || text.chars().count() > 200 {
                return Err(HostError::InvalidArguments(
                    "text 不能为空、不超过 200 字".into(),
                ));
            }
            let app = app.clone();
            blocking(move || crate::voice_chat::say(&app, &text)).await
        }
        c::TOOL_VOICE_CHAT_TEST => {
            let app = app.clone();
            blocking(move || crate::voice_chat::test(&app)).await
        }
        other => Err(HostError::Failed(format!("桌面端未实现工具 {other}"))),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VoiceChatMicArgs {
    r#use: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VoiceChatSayArgs {
    text: String,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GameLookArgs {
    #[serde(default)]
    zoom: Option<Vec<f64>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GameActArgs {
    observation_id: String,
    actions: Value,
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

async fn list_catalog(
    app: &AppHandle,
    category: ModelCategory,
) -> Result<Vec<kotone_core::model_catalog::CatalogModel>, HostError> {
    let list_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        crate::model_catalog::list_blocking(&list_app, category)
    })
    .await
    .map_err(|e| HostError::Failed(e.to_string()))?
    .map_err(HostError::Failed)
}

/// 后台下载 OCR / TTS 本地模型：进度（至多每秒一次）与结果发布到工具事件总线，事件带 category。
fn start_catalog_download(app: &AppHandle, category: ModelCategory, id: String) {
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
                json!({"id": progress_id, "category": category.as_str(), "downloadedBytes": payload["downloaded"], "totalBytes": payload["total"]}),
            );
        });
        let started = Instant::now();
        let result = crate::model_catalog::download(&app, category, &id).await;
        app.unlisten(listener);
        let ms = started.elapsed().as_millis() as u64;
        match result {
            Ok(()) => {
                log::log(&format!(
                    "tool service: model download finished category={} id={id}",
                    category.as_str()
                ));
                tool_service::publish(
                    &app,
                    "model.download.finished",
                    json!({"id": id, "category": category.as_str(), "durationMs": ms}),
                );
                settings_changed(&app);
            }
            Err(error) => {
                let cancelled =
                    kotone_stt::download::is_cancelled(&error) || error.contains("已取消");
                tool_service::publish(
                    &app,
                    "model.download.failed",
                    json!({"id": id, "category": category.as_str(), "cancelled": cancelled, "message": error, "durationMs": ms}),
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

/// 普通重启（`app.restart`）：先让响应送出，再走正常的退出流程（RunEvent::Exit 里收尾）后重启。
fn schedule_restart(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        log::log("tool service: app.restart accepted, restarting");
        app.request_restart();
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
