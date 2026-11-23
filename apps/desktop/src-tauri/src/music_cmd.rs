//! 音乐页的命令（ADR 034；接口约定见 ADR 034 和 `src/lib/music.ts`）。都是 async：要联网，不能卡主线程。

use kotone_music::Track;
use serde_json::Value;
use tauri::AppHandle;

use crate::music::{self, PlayRequest};

#[tauri::command]
pub async fn music_state(app: AppHandle) -> Result<Value, String> {
    music::state(&app)
}

#[tauri::command]
pub async fn music_login_start(app: AppHandle) -> Result<Value, String> {
    music::login_start(&app).await
}

#[tauri::command]
pub async fn music_login_cancel(app: AppHandle) -> Result<Value, String> {
    music::login_cancel(&app)
}

#[tauri::command]
pub async fn music_logout(app: AppHandle) -> Result<Value, String> {
    music::logout(&app).await
}

#[tauri::command]
pub async fn music_search(query: String, offset: Option<u32>) -> Result<Value, String> {
    music::search(&query, offset.unwrap_or(0)).await
}

#[tauri::command]
pub async fn music_home() -> Result<Value, String> {
    music::home().await
}

#[tauri::command]
pub async fn music_playlist(id: String) -> Result<Value, String> {
    music::playlist(&id).await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn music_play(
    app: AppHandle,
    tracks: Option<Vec<Track>>,
    index: Option<usize>,
    from: Option<String>,
    playlist_id: Option<String>,
    query: Option<String>,
) -> Result<Value, String> {
    music::play(
        &app,
        PlayRequest {
            tracks,
            index: index.unwrap_or(0),
            from,
            playlist_id,
            playlist_name: None,
            query,
        },
    )
    .await
}

#[tauri::command]
pub async fn music_control(
    app: AppHandle,
    action: String,
    value: Option<Value>,
) -> Result<Value, String> {
    music::control(&app, &action, value.as_ref()).await
}

#[tauri::command]
pub async fn music_queue() -> Result<Value, String> {
    music::queue()
}

#[tauri::command]
pub async fn music_set_quality(app: AppHandle, quality: String) -> Result<Value, String> {
    music::set_quality(&app, &quality)
}
