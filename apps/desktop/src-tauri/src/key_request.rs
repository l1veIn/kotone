//! `connections.request_key`：琴音请玩家粘贴某个服务商的 Key（Key 不经过模型）。
//!
//! 发 `kotone://agent-key-request` 事件，聊天窗口显示一个只给玩家看的输入框；玩家粘贴后
//! 调 [`agent_submit_key`]：建好或更新这条连接、Key 存进系统凭据库，琴音只收到“已保存”和连接 id。
//! 玩家点“不用了”、这一轮被停止或 5 分钟没填，都按“没有保存”收尾（守卫通知界面收起输入框）。

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::oneshot;

pub const REQUEST_EVENT: &str = "kotone://agent-key-request";
pub const RESOLVED_EVENT: &str = "kotone://agent-key-request-resolved";
const WAIT: Duration = Duration::from_secs(300);

type Reply = Result<String, String>;

#[derive(Default)]
pub struct KeyRequests {
    pending: Mutex<HashMap<String, oneshot::Sender<Reply>>>,
    next: Mutex<u64>,
}

struct Guard {
    app: AppHandle,
    id: String,
    conversation_id: String,
    saved: bool,
}

impl Drop for Guard {
    fn drop(&mut self) {
        if let Some(state) = self.app.try_state::<KeyRequests>() {
            state.pending.lock().unwrap().remove(&self.id);
        }
        let _ = self.app.emit(
            RESOLVED_EVENT,
            json!({"requestId": self.id, "conversationId": self.conversation_id, "saved": self.saved}),
        );
    }
}

fn preset(provider: &str) -> Option<&'static kotone_core::connection::ConnectionProviderPreset> {
    kotone_core::connection::CONNECTION_PRESETS
        .iter()
        .find(|p| p.id == provider)
}

/// 琴音调用：等玩家粘贴（或放弃）。`voice`：语音入口，提醒玩家到窗口里粘贴。
pub async fn request(
    app: &AppHandle,
    conversation_id: &str,
    voice: bool,
    arguments: &Value,
) -> Result<Value, String> {
    let text = |key: &str| {
        arguments
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string()
    };
    let provider = text("provider");
    let preset = preset(&provider)
        .ok_or_else(|| format!("没有服务商「{provider}」：用 connections.presets 查可选的 id"))?;
    let state = app.try_state::<KeyRequests>().ok_or("Kotone 还没准备好")?;
    let id = {
        let mut next = state.next.lock().unwrap();
        *next += 1;
        format!("key-{}", *next)
    };
    let (tx, rx) = oneshot::channel();
    state.pending.lock().unwrap().insert(id.clone(), tx);
    let mut guard = Guard {
        app: app.clone(),
        id: id.clone(),
        conversation_id: conversation_id.to_string(),
        saved: false,
    };
    let _ = app.emit(
        REQUEST_EVENT,
        json!({
            "requestId": id,
            "conversationId": conversation_id,
            "provider": preset.id,
            "displayName": preset.display_name,
            "apiKeyUrl": preset.api_key_url,
            "connectionId": text("connectionId"),
            "reason": text("reason"),
        }),
    );
    if voice {
        crate::bubble::note(
            app,
            &format!("请到 Kotone 窗口里粘贴 {} 的 Key", preset.display_name),
        );
    }
    kotone_core::log::log(&format!("agent: asked the player for a {} key", preset.id));
    match tokio::time::timeout(WAIT, rx).await {
        Ok(Ok(Ok(connection_id))) => {
            guard.saved = true;
            Ok(json!({"saved": true, "connectionId": connection_id, "provider": preset.id}))
        }
        Ok(Ok(Err(reason))) => Ok(json!({"saved": false, "note": reason})),
        Ok(Err(_)) => Ok(json!({"saved": false, "note": "输入框被关掉了"})),
        Err(_) => Ok(json!({"saved": false, "note": "玩家 5 分钟内没有填"})),
    }
}

/// 连接 id：指定的 → 这个服务商的已有连接 → 新建（`<provider>`，重名加序号）。
fn target_connection(
    settings: &kotone_core::settings::Settings,
    provider: &str,
    wanted: &str,
) -> (String, Option<kotone_core::connection::Connection>) {
    if let Some(found) = settings
        .connections
        .iter()
        .find(|c| !wanted.is_empty() && c.id == wanted)
        .or_else(|| settings.connections.iter().find(|c| c.provider == provider))
    {
        return (found.id.clone(), Some(found.clone()));
    }
    let mut id = provider.to_string();
    let mut n = 1;
    while settings.connections.iter().any(|c| c.id == id) {
        n += 1;
        id = format!("{provider}-{n}");
    }
    (id, None)
}

/// 聊天窗口里的输入框：`api_key` 为空 = 玩家点了“不用了”。
#[tauri::command]
pub fn agent_submit_key(
    app: AppHandle,
    request_id: String,
    provider: String,
    connection_id: Option<String>,
    api_key: Option<String>,
) -> Result<Value, String> {
    let state = app.state::<KeyRequests>();
    let Some(tx) = state.pending.lock().unwrap().remove(&request_id) else {
        return Err("这个请求已经结束了".into());
    };
    let key = api_key.unwrap_or_default().trim().to_string();
    if key.is_empty() {
        let _ = tx.send(Err("玩家选择不填".into()));
        return Ok(json!({"saved": false}));
    }
    let preset = preset(&provider).ok_or("未知的服务商")?;
    let shared = app.state::<crate::SharedState>();
    let (id, existing) = {
        let settings = shared.settings.read().unwrap();
        target_connection(&settings, preset.id, connection_id.as_deref().unwrap_or(""))
    };
    let connection = existing.unwrap_or_else(|| kotone_core::connection::Connection {
        id: id.clone(),
        display_name: preset.display_name.to_string(),
        kind: kotone_core::connection::ConnectionKind::Remote,
        provider: preset.id.to_string(),
        base_url: preset.default_base_url.to_string(),
        model: preset.default_model.to_string(),
    });
    let result = kotone_ai::connections::upsert_connection(
        &shared.settings_repository,
        shared.secrets.as_ref(),
        connection,
        Some(key),
    );
    match result {
        Ok(_) => {
            let _ = tx.send(Ok(id.clone()));
            crate::tool_config::notify_settings_changed(&app);
            Ok(json!({"saved": true, "connectionId": id}))
        }
        Err(e) => {
            // 没存上：输入框留着让玩家改（请求重新登记）。
            state.pending.lock().unwrap().insert(request_id, tx);
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_go_to_the_requested_or_existing_connection_or_a_new_one() {
        let mut settings = kotone_core::settings::Settings::default();
        let conn = |id: &str, provider: &str| kotone_core::connection::Connection {
            id: id.into(),
            display_name: id.into(),
            kind: kotone_core::connection::ConnectionKind::Remote,
            provider: provider.into(),
            base_url: String::new(),
            model: String::new(),
        };
        assert_eq!(target_connection(&settings, "openai", "").0, "openai");
        settings.connections.push(conn("openai", "custom"));
        assert_eq!(target_connection(&settings, "openai", "").0, "openai-2");
        settings.connections.push(conn("my-oa", "openai"));
        assert_eq!(target_connection(&settings, "openai", "").0, "my-oa");
        assert_eq!(target_connection(&settings, "openai", "openai").0, "openai");
    }
}
