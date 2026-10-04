//! Settings-page voice controls use a session receipt, never the generic game input commands.

use kotone_core::assistant::AssistantVoiceSession;
use kotone_core::runtime::RuntimePhase;
use tauri::{AppHandle, Manager};

use crate::{runtime::RuntimeManager, SharedState};

#[tauri::command]
pub async fn assistant_voice_begin(app: AppHandle) -> Result<AssistantVoiceSession, String> {
    let runtime = app.state::<RuntimeManager>();
    if runtime.phase() != RuntimePhase::Running {
        return Err("请先启动 Kotone，再使用语音输入".into());
    }
    let runtime_ticket = runtime.input_ticket().ok_or("输入正在切换，请稍后重试")?;
    let state = app.state::<SharedState>();
    let input_ticket = state.orchestrator.hotkey_input_ticket();
    if !runtime.accepts_input(runtime_ticket) {
        return Err("运行时正在切换，请稍后重试".into());
    }
    state
        .orchestrator
        .start_assistant_voice_input(input_ticket)
        .await
}

#[tauri::command]
pub async fn assistant_voice_end(
    state: tauri::State<'_, SharedState>,
    session_id: String,
) -> Result<(), String> {
    state
        .orchestrator
        .finish_assistant_voice_input(&session_id)
        .await
}

#[tauri::command]
pub async fn assistant_voice_confirm(
    state: tauri::State<'_, SharedState>,
    session_id: String,
) -> Result<(), String> {
    state
        .orchestrator
        .confirm_assistant_voice_input(&session_id)
        .await
}

#[tauri::command]
pub async fn assistant_voice_cancel(
    state: tauri::State<'_, SharedState>,
    session_id: String,
) -> Result<(), String> {
    state
        .orchestrator
        .cancel_assistant_voice_input(&session_id)
        .await
}
