//! 可切换的角色（[`kotone_agent::roles`]）的桌面端接线：列表、切换、编辑、新建、删除、导入导出、
//! 唤醒词检查；当前角色的名字和头像同步进设置 `agent.identity`（气泡、聊天、侧栏都读它），
//! 声音在合成时换成当前角色的（[`voice_for`]），唤醒词按当前角色生成（[`wake_keywords`]）。

use kotone_agent::roles::{Role, RolePatch, DEFAULT_ROLE};
use kotone_stt::wake_word;
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

use crate::SharedState;

fn workspace() -> kotone_agent::workspace::Workspace {
    crate::agent::workspace()
}

/// 当前角色与它的声音（给不拿 AppHandle 的地方用：工作区、语音合成）。设置改了就更新（[`remember`]）。
static CURRENT: std::sync::RwLock<(String, String)> =
    std::sync::RwLock::new((String::new(), String::new()));

/// 当前角色 id（缓存；还没启动完时是琴音）。
pub fn cached_id() -> String {
    let id = CURRENT.read().unwrap().0.clone();
    if id.is_empty() {
        DEFAULT_ROLE.into()
    } else {
        id
    }
}

/// 当前角色的声音（`provider/voice`，缓存；空 = 跟随全局）。
pub fn cached_voice() -> String {
    CURRENT.read().unwrap().1.clone()
}

/// 设置保存后、启动时调用：记下当前角色和它的声音。
pub fn remember(role_id: &str) {
    let ws = kotone_agent::workspace::Workspace::new(
        kotone_core::settings::kotone_dir()
            .join("agent")
            .join("workspace"),
    );
    let (id, voice) = match ws.role(role_id) {
        Some(role) => (role.id, role.voice),
        None => (
            DEFAULT_ROLE.to_string(),
            ws.role(DEFAULT_ROLE).map(|r| r.voice).unwrap_or_default(),
        ),
    };
    *CURRENT.write().unwrap() = (id, voice);
}

fn settings(app: &AppHandle) -> kotone_core::settings::Settings {
    app.state::<SharedState>().settings.read().unwrap().clone()
}

/// 当前角色 id（设置里的；那个角色已经不在了就是琴音）。
pub fn current_id(app: &AppHandle) -> String {
    let id = settings(app).agent.role_id;
    if workspace().role(&id).is_some() {
        id
    } else {
        DEFAULT_ROLE.into()
    }
}

/// 第一次启动新版本：把琴音原来的名字、头像（设置 `agent.identity`）记进她的角色卡。只做一次。
pub fn migrate(app: &AppHandle) {
    let ws = workspace();
    let card = ws
        .dir()
        .join(kotone_agent::roles::ROLES_DIR)
        .join(DEFAULT_ROLE)
        .join(kotone_agent::roles::ROLE_FILE);
    if card.exists() {
        return;
    }
    let s = settings(app);
    if s.agent.role_id != DEFAULT_ROLE {
        return;
    }
    let identity = s.agent.identity;
    let patch = RolePatch {
        name: Some(identity.name.trim().to_string()).filter(|n| !n.is_empty()),
        avatar: Some(identity.avatar.clone()).filter(|a| !a.is_empty()),
        ..Default::default()
    };
    if patch.name.is_some() || patch.avatar.is_some() {
        if let Err(e) = ws.update_role(DEFAULT_ROLE, &patch) {
            kotone_core::log::log(&format!("roles: migrate failed: {e}"));
        }
    }
}

/// 唤醒词检查结果（给界面）：文字、每个字的读音和可选读音、token、能不能用、提醒。
fn wake_check(role: &Role, preset: &str) -> wake_word::WakeWordCheck {
    let text = effective_wake_text(role, preset);
    wake_word::check(&text, &role.wake.readings)
}

/// 角色实际用的唤醒词：自己设了就用自己的；没设时琴音用「语音唤醒」里选的预置唤醒词，
/// 其他角色用名字（两个字的重复两遍）。
fn effective_wake_text(role: &Role, preset: &str) -> String {
    if !role.wake.text.trim().is_empty() {
        return role.wake.text.trim().to_string();
    }
    if role.id == DEFAULT_ROLE && role.name == kotone_agent::workspace::DEFAULT_NAME {
        if let Some((_, label, _)) = kotone_stt::wake::PHRASES
            .iter()
            .find(|(id, _, _)| *id == preset)
        {
            return (*label).to_string();
        }
    }
    wake_word::default_for_name(&role.name).unwrap_or_default()
}

fn preset_phrase(app: &AppHandle) -> String {
    serde_json::to_value(settings(app).assistant.wake.phrase)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// 当前角色的唤醒词给 KWS 的关键词行。不能用时退回预置的“琴音琴音”，并说明原因。
pub fn wake_keywords(app: &AppHandle) -> (String, String, Option<String>) {
    let preset = preset_phrase(app);
    let fallback = || {
        kotone_stt::wake::keywords_text(&preset)
            .unwrap_or_else(|_| "q ín y īn q ín y īn @琴音琴音\n".into())
    };
    let Some(role) = workspace().role(&current_id(app)) else {
        return (fallback(), "琴音琴音".into(), None);
    };
    let check = wake_check(&role, &preset);
    if check.ok {
        let line = wake_word::keyword_line(&check.tokens, &check.text);
        (format!("{line}\n"), check.text, None)
    } else {
        (
            fallback(),
            "琴音琴音".into(),
            Some(format!(
                "{}的唤醒词「{}」不能用（{}），先用“琴音琴音”",
                role.name,
                check.text,
                check.problems.join("；")
            )),
        )
    }
}

fn role_json(role: &Role, current: &str, preset: &str) -> Value {
    let check = wake_check(role, preset);
    json!({
        "id": role.id,
        "name": role.name,
        "summary": role.summary,
        "builtin": role.builtin,
        "avatar": role.avatar,
        "voice": role.voice,
        "wake": role.wake,
        "wakeCheck": check,
        "greeting": role.greeting,
        "current": role.id == current,
    })
}

/// 全部角色（工具 `roles.list` 和界面共用）。
pub fn list(app: &AppHandle) -> Value {
    let current = current_id(app);
    let preset = preset_phrase(app);
    let ws = workspace();
    let roles: Vec<Value> = ws
        .roles()
        .iter()
        .map(|r| role_json(r, &current, &preset))
        .collect();
    json!({"current": current, "roles": roles})
}

/// 一个角色的详情（含性格全文）。
pub fn get(app: &AppHandle, id: &str) -> Result<Value, String> {
    let ws = workspace();
    let role = ws.role(id).ok_or_else(|| format!("没有角色「{id}」"))?;
    let mut value = role_json(&role, &current_id(app), &preset_phrase(app));
    value["soul"] = json!(ws.role_soul(id));
    Ok(value)
}

/// 把角色的名字、头像写进设置（界面各处读这里），并按需换唤醒词。
fn sync_identity(app: &AppHandle, role: &Role) -> Result<(), String> {
    let patch = json!({"agent": {"roleId": role.id, "identity": {"name": role.name, "avatar": role.avatar}}});
    crate::update_settings_from_backend(app, patch)?;
    Ok(())
}

/// 换角色：名字、头像、性格、声音、唤醒词跟着换；开一段新对话（系统提示在一段对话里不变）。
pub fn switch(app: &AppHandle, id: &str) -> Result<Value, String> {
    let role = workspace()
        .role(id)
        .ok_or_else(|| format!("没有角色「{id}」"))?;
    if crate::agent::is_running(app) {
        return Err("正在回答，等说完再换角色".into());
    }
    sync_identity(app, &role)?;
    crate::agent::start_new_conversation(app)?;
    kotone_core::log::log(&format!("roles: switched to {}", role.id));
    Ok(json!({"current": role.id, "name": role.name, "note": "已换角色，下一句话开始是新对话"}))
}

pub fn update(app: &AppHandle, id: &str, patch: RolePatch) -> Result<Value, String> {
    let role = workspace().update_role(id, &patch)?;
    if current_id(app) == role.id {
        sync_identity(app, &role)?;
    }
    remember(&current_id(app));
    get(app, &role.id)
}

pub fn create(app: &AppHandle, patch: RolePatch) -> Result<Value, String> {
    let role = workspace().create_role(&patch)?;
    get(app, &role.id)
}

pub fn delete(app: &AppHandle, id: &str) -> Result<Value, String> {
    if id == DEFAULT_ROLE {
        // 琴音：只是改回默认（名字、声音、唤醒词），性格和头像不动。
        workspace().update_role(
            DEFAULT_ROLE,
            &RolePatch {
                name: Some(kotone_agent::workspace::DEFAULT_NAME.into()),
                voice: Some(String::new()),
                wake: Some(Default::default()),
                ..Default::default()
            },
        )?;
    } else {
        let was_current = current_id(app) == id;
        workspace().delete_role(id)?;
        if was_current {
            switch(app, DEFAULT_ROLE)?;
        }
    }
    if current_id(app) == id {
        if let Some(role) = workspace().role(id) {
            sync_identity(app, &role)?;
        }
    }
    Ok(list(app))
}

// ---------- Tauri 命令 ----------

#[tauri::command]
pub fn roles_list(app: AppHandle) -> Value {
    list(&app)
}

#[tauri::command]
pub fn roles_get(app: AppHandle, id: String) -> Result<Value, String> {
    get(&app, &id)
}

#[tauri::command]
pub fn roles_switch(app: AppHandle, id: String) -> Result<Value, String> {
    switch(&app, &id)
}

#[tauri::command]
pub fn roles_update(app: AppHandle, id: String, patch: RolePatch) -> Result<Value, String> {
    update(&app, &id, patch)
}

#[tauri::command]
pub fn roles_create(app: AppHandle, patch: RolePatch) -> Result<Value, String> {
    create(&app, patch)
}

#[tauri::command]
pub fn roles_delete(app: AppHandle, id: String) -> Result<Value, String> {
    delete(&app, &id)
}

#[tauri::command]
pub fn roles_export(id: String) -> Result<Value, String> {
    workspace().export_role(&id)
}

#[tauri::command]
pub fn roles_import(app: AppHandle, card: Value) -> Result<Value, String> {
    let role = workspace().import_role(card)?;
    get(&app, &role.id)
}

/// 检查一个唤醒词（界面边打字边检查）。
#[tauri::command]
pub fn roles_check_wake(text: String, readings: Vec<String>) -> wake_word::WakeWordCheck {
    wake_word::check(&text, &readings)
}

#[cfg(test)]
mod tests {
    #[test]
    fn builtin_default_wake_words_pass_the_model_check() {
        for role in kotone_agent::roles::BUILTIN {
            let text = if role.wake.is_empty() {
                kotone_stt::wake_word::default_for_name(role.name).unwrap_or_default()
            } else {
                role.wake.to_string()
            };
            let check = kotone_stt::wake_word::check(&text, &[]);
            assert!(check.ok, "{}「{text}」：{:?}", role.name, check.problems);
        }
    }
}
