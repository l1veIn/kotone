//! 模型命令：列表、下载、启用、删除、模型目录与迁移。

use super::*;

#[tauri::command]
pub(crate) async fn list_models() -> Result<Vec<model::ModelInfo>, String> {
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
pub(crate) async fn download_model(
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
pub(crate) fn cancel_download() {
    model::cancel_download();
    log::log("download cancel requested");
}

/// 自动下载失败后的手动安装指引（官方页 + 目标目录 + 文件清单）
#[tauri::command]
pub(crate) fn get_model_install_guide(id: String) -> Result<model::ModelInstallGuide, String> {
    model::install_guide(&id)
}

/// 打开某个模型的目标目录（自动创建），方便用户把手动下载的文件放进去
#[tauri::command]
pub(crate) fn open_model_dest_dir(id: String) -> Result<(), String> {
    let dir = model::multi_model_dir(&id).ok_or_else(|| format!("未知模型：{id}"))?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("无法创建目录 {}：{e}", dir.display()))?;
    open_in_file_manager(&dir)
}

#[tauri::command]
pub(crate) fn set_active_model(
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

/// 当前模型目录（生效路径 + 是否默认）
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelsDirInfo {
    pub dir: String,
    pub is_default: bool,
}

#[tauri::command]
pub(crate) fn get_models_dir(state: tauri::State<SharedState>) -> ModelsDirInfo {
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
pub(crate) fn set_models_dir(
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
pub(crate) fn delete_model(
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

/// 在系统文件管理器中打开模型目录
#[tauri::command]
pub(crate) fn open_models_dir(state: tauri::State<SharedState>) -> Result<(), String> {
    let dir = {
        let settings = state.settings.read().unwrap();
        model::models_dir_from(&settings)
    };
    std::fs::create_dir_all(&dir).map_err(|e| format!("无法创建目录 {}：{e}", dir.display()))?;
    open_in_file_manager(&dir)
}
