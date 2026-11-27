//! 语音输入运行时与会话命令：状态、启动 / 停止、确认发送、取消。

use super::*;

/// 全量运行时状态（相位 + restartNeeded + 当前引擎/模型/模式）
#[tauri::command]
pub(crate) fn get_runtime_status(
    app: AppHandle,
    state: tauri::State<SharedState>,
) -> RuntimeStatus {
    let rt = app.state::<RuntimeManager>();
    let settings = state.settings.read().unwrap();
    rt.status(&settings, &state.engines, None)
}

/// 启动：warmup 引擎 → 注册热键 → 显示悬浮窗（进度经 kotone://runtime 推送）。
/// Running + restartNeeded 时等价于 stop + start（重启语义）。
#[tauri::command]
pub(crate) async fn start_runtime(app: AppHandle) -> Result<RuntimeStatus, String> {
    runtime::start(&app).await
}

/// 停止：取消会话 → 注销热键 → 隐藏悬浮窗 → 卸载引擎。Stopped 时幂等。
#[tauri::command]
pub(crate) async fn stop_runtime(app: AppHandle) -> Result<RuntimeStatus, String> {
    runtime::stop(&app).await
}

// ---------- 模型目录管理 ----------

/// Preview 状态下确认发送（ADR-006：预览只读，始终发送预览文本）
#[tauri::command]
pub(crate) async fn confirm_send(state: tauri::State<'_, SharedState>) -> Result<(), String> {
    state.orchestrator.confirm_send().await
}

/// 取消当前会话（悬浮窗取消按钮 / 调试）
#[tauri::command]
pub(crate) async fn cancel_session(state: tauri::State<'_, SharedState>) -> Result<(), String> {
    state.orchestrator.cancel().await;
    Ok(())
}

#[tauri::command]
pub(crate) async fn voice_input_trial_begin(app: AppHandle) -> Result<String, String> {
    if app.state::<RuntimeManager>().phase() != RuntimePhase::Running {
        return Err("请先启动语音识别".into());
    }
    app.state::<SharedState>()
        .orchestrator
        .begin_voice_trial()
        .await
}

#[tauri::command]
pub(crate) async fn voice_input_trial_finish(
    state: tauri::State<'_, SharedState>,
    session_id: String,
) -> Result<String, String> {
    state.orchestrator.finish_voice_trial(&session_id).await
}

#[tauri::command]
pub(crate) async fn voice_input_trial_cancel(
    state: tauri::State<'_, SharedState>,
    session_id: String,
) -> Result<(), String> {
    state.orchestrator.cancel_voice_trial(&session_id).await;
    Ok(())
}
