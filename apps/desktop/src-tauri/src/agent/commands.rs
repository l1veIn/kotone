//! 界面命令（Tauri）与给其他模块用的对外接口：停下、当前对话、工具清单、状态。

use super::*;

pub fn stop(app: &AppHandle) -> Result<bool, String> {
    crate::computer_use::stop_all(app, "手动停止");
    crate::voice_chat::hush(app);
    let rt = runtime(app)?;
    let inner = rt.inner.lock().unwrap();
    match &inner.running {
        Some(running) => {
            running.cancel.cancel();
            running.inbox.clear();
            Ok(true)
        }
        None => Ok(false),
    }
}

/// 正在回答的对话，没有就是当前对话：(id, 开始的时间)。`usage.summary` 的“本段对话”用。
pub fn active_conversation(app: &AppHandle) -> Option<(String, u64)> {
    let running = runtime(app)
        .ok()?
        .inner
        .lock()
        .unwrap()
        .running
        .as_ref()
        .map(|r| r.conversation_id.clone());
    let store = store(app).ok()?;
    let id = running.or_else(|| store.current())?;
    let since = store
        .load(&id)
        .ok()
        .flatten()
        .map(|c| c.created_at_ms)
        .unwrap_or(0);
    Some((id, since))
}

/// 对话存储（对局复盘读这一局的对话用）。
pub fn conversation_store(app: &AppHandle) -> Result<Arc<ConversationStore>, String> {
    store(app)
}

/// 等琴音空下来（最多 3 分钟），把一条说明加到当前对话末尾（没有当前对话就加到最近的主对话）；
/// 不发给模型。返回加到了哪段对话。
pub async fn append_notice_when_idle(app: &AppHandle, notice: Message) -> Option<String> {
    for _ in 0..90 {
        if !is_running(app) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
    if is_running(app) {
        return None;
    }
    let store = store(app).ok()?;
    let id = store.current().or_else(|| {
        store
            .list()
            .into_iter()
            .find(|c| c.parent.is_none())
            .map(|c| c.id)
    })?;
    let mut conversation = store.load(&id).ok()??;
    conversation.messages.push(notice.clone());
    if let Err(e) = store.save(&mut conversation) {
        log::log(&format!("agent: save failed: {e}"));
        return None;
    }
    emit(app, &id, json!({"kind": "appended", "message": notice}));
    Some(id)
}

/// 主会话正在回答吗。
pub fn is_running(app: &AppHandle) -> bool {
    runtime(app).is_ok_and(|rt| rt.inner.lock().unwrap().running.is_some())
}

/// 下一句话开始一段新对话（换角色时用）。
pub fn start_new_conversation(app: &AppHandle) -> Result<(), String> {
    ensure_idle(app)?;
    store(app)?.set_current(None)?;
    emit(app, "", json!({"kind": "idle"}));
    Ok(())
}

fn ensure_idle(app: &AppHandle) -> Result<(), String> {
    if runtime(app)?.inner.lock().unwrap().running.is_some() {
        Err("正在回答，请先停止".into())
    } else {
        Ok(())
    }
}

/// 琴音能用的全部工具（设置页按工具设规则用）：名字、分组、说明、默认规则、用户规则、
/// 是否一开始就提供（其余要经 tools.find 查到）。
pub fn tools(app: &AppHandle) -> Result<Value, String> {
    let settings = settings(app);
    let service = crate::tool_service::service(app).ok_or("工具服务尚未就绪，请稍后再试")?;
    let specs = service.capabilities()["tools"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let automations = offered_automations(app, &settings.agent.hidden_automations);
    let catalog = Catalog::build(&specs, &automations);
    let rules = &settings.agent.permissions.rules;
    let tools: Vec<Value> = catalog
        .entries()
        .iter()
        .map(|e| {
            let default = match kotone_agent::permissions::default_decision(e, &Value::Null) {
                kotone_agent::permissions::Decision::Allow => "allow",
                _ => "ask",
            };
            let description: String = e.description.chars().take(200).collect();
            json!({
                "name": e.name,
                "group": e.group,
                "description": description,
                "sideEffects": e.side_effects,
                "defaultRule": default,
                "rule": rules.get(&e.name),
                "core": Catalog::is_core(e),
            })
        })
        .collect();
    Ok(json!({"tools": tools}))
}

pub fn state(app: &AppHandle) -> Result<Value, String> {
    let rt = runtime(app)?;
    let store = store(app)?;
    let running = rt
        .inner
        .lock()
        .unwrap()
        .running
        .as_ref()
        .map(|r| r.conversation_id.clone());
    let settings = settings(app);
    let ready = crate::companion::connection_ready(app, &settings).err();
    // 交给后台的事的记录（`parent` 不为空）不进对话列表，只在主会话里是一张卡片（ADR 029 第二期）。
    let conversations: Vec<_> = store
        .list()
        .into_iter()
        .filter(|c| c.parent.is_none())
        .collect();
    Ok(json!({
        "current": store.current(),
        "running": running,
        "conversations": conversations,
        "notReadyReason": ready,
        "permissions": settings.agent.permissions,
    }))
}

// ---------- Tauri 命令 ----------

#[tauri::command]
pub fn agent_send(app: AppHandle, text: String) -> Result<Value, String> {
    send(
        &app,
        Outgoing {
            text,
            source: "chat".into(),
            attachments: Vec::new(),
        },
    )
}

#[tauri::command]
pub fn agent_stop(app: AppHandle) -> Result<bool, String> {
    stop(&app)
}

#[tauri::command]
pub fn agent_answer(app: AppHandle, ask_id: String, answer: Answer) -> Result<bool, String> {
    let sender = runtime(&app)?.inner.lock().unwrap().asks.remove(&ask_id);
    Ok(sender.is_some_and(|tx| tx.send(answer).is_ok()))
}

#[tauri::command]
pub fn agent_tools(app: AppHandle) -> Result<Value, String> {
    tools(&app)
}

#[tauri::command]
pub fn agent_state(app: AppHandle) -> Result<Value, String> {
    state(&app)
}

#[tauri::command]
pub fn agent_conversation(app: AppHandle, id: String) -> Result<Option<Conversation>, String> {
    let mut conversation = store(&app)?.load(&id)?;
    if let Some(c) = &mut conversation {
        c.subagents = subagents_of(&app, &id);
    }
    Ok(conversation)
}

/// 新对话：下一条消息开始一段新的对话。
#[tauri::command]
pub fn agent_new_conversation(app: AppHandle) -> Result<(), String> {
    ensure_idle(&app)?;
    store(&app)?.set_current(None)
}

#[tauri::command]
pub fn agent_select_conversation(app: AppHandle, id: String) -> Result<(), String> {
    ensure_idle(&app)?;
    let store = store(&app)?;
    if store.load(&id)?.is_none() {
        return Err("找不到这段对话".into());
    }
    store.set_current(Some(&id))
}

/// 整体替换权限设置（模式 + 按工具的规则）。设置补丁是递归合并的，删不掉单条规则，
/// 所以界面改规则时走这里。
#[tauri::command]
pub fn agent_set_permissions(
    app: AppHandle,
    permissions: kotone_core::agent::Policy,
) -> Result<kotone_core::agent::Policy, String> {
    let updated = app
        .state::<crate::SharedState>()
        .settings_repository
        .update(move |next| {
            next.agent.permissions = permissions;
            Ok(())
        })?;
    crate::tool_config::notify_settings_changed(&app);
    Ok(updated.agent.permissions)
}

/// 某个已保存的自动化交不交给 AI（`agent.hiddenAutomations`）。返回新的隐藏列表。
#[tauri::command]
pub fn agent_set_automation_hidden(
    app: AppHandle,
    id: String,
    hidden: bool,
) -> Result<Vec<String>, String> {
    let id = id.trim().to_string();
    if id.is_empty() {
        return Err("自动化 id 为空".into());
    }
    let updated = app
        .state::<crate::SharedState>()
        .settings_repository
        .update(move |next| {
            let list = &mut next.agent.hidden_automations;
            list.retain(|x| x != &id);
            if hidden {
                list.push(id);
            }
            Ok(())
        })?;
    crate::tool_config::notify_settings_changed(&app);
    Ok(updated.agent.hidden_automations)
}

#[tauri::command]
pub fn agent_delete_conversation(app: AppHandle, id: String) -> Result<bool, String> {
    let rt = runtime(&app)?;
    if rt
        .inner
        .lock()
        .unwrap()
        .running
        .as_ref()
        .is_some_and(|r| r.conversation_id == id)
    {
        return Err("这段对话正在进行，请先停止".into());
    }
    rt.inner.lock().unwrap().sessions.remove(&id);
    // 交给后台的事一起停、一起删（它们只在这段对话里看得到）。
    let store = store(&app)?;
    for agent in subagents_of(&app, &id) {
        stop_subagent(&app, &id, &agent.name, "对话删除了");
        rt.inner
            .lock()
            .unwrap()
            .sessions
            .remove(&agent.conversation_id);
        if let Err(e) = store.delete(&agent.conversation_id) {
            log::log(&format!("agent: delete background record failed: {e}"));
        }
    }
    {
        let mut inner = rt.inner.lock().unwrap();
        inner.registries.remove(&id);
        inner.last_entry.remove(&id);
    }
    store.delete(&id)
}

/// 一件后台的事一步步做了什么（卡片展开时看；同时间线一样的消息）。
#[tauri::command]
pub fn agent_subagent_messages(
    app: AppHandle,
    conversation_id: String,
    name: String,
) -> Result<Value, String> {
    let agent = subagents_of(&app, &conversation_id)
        .into_iter()
        .find(|a| a.name == name)
        .ok_or("找不到这件事")?;
    let messages = store(&app)?
        .load(&agent.conversation_id)?
        .map(|c| c.messages)
        .unwrap_or_default();
    Ok(json!({"messages": messages}))
}

/// 卡片上的“停止”。
#[tauri::command]
pub fn agent_subagent_stop(app: AppHandle, conversation_id: String, name: String) -> bool {
    stop_subagent(&app, &conversation_id, &name, "玩家点了停止")
}

/// 卡片上的“继续”（上次没做完 Kotone 就关了，或者被停了）：相当于玩家说“继续刚才的事”。
#[tauri::command]
pub async fn agent_subagent_continue(
    app: AppHandle,
    conversation_id: String,
    name: String,
) -> Result<Value, String> {
    resume(
        &app,
        &conversation_id,
        &name,
        "（玩家在聊天里点了“继续”）",
        now_ms(),
        "继续刚才没做完的事。",
        None,
        std::time::Duration::ZERO,
    )
    .await
}

/// 卡片上的“允许 / 本次对话都允许 / 拒绝”。返回是否交到了在等的那次确认。
#[tauri::command]
pub fn agent_subagent_answer(
    app: AppHandle,
    conversation_id: String,
    name: String,
    answer: String,
) -> Result<bool, String> {
    let answer = match answer.as_str() {
        "allow-once" => Answer::AllowOnce,
        "allow-conversation" | "allow-for-conversation" => Answer::AllowForConversation,
        "deny" => Answer::Deny,
        other => return Err(format!("不认识的回答：{other}")),
    };
    let Some(agent) = subagents_of(&app, &conversation_id)
        .into_iter()
        .find(|a| a.name == name)
    else {
        return Ok(false);
    };
    let sender = runtime(&app)?
        .inner
        .lock()
        .unwrap()
        .children
        .get_mut(&agent.conversation_id)
        .and_then(|c| c.ask.take());
    Ok(sender.is_some_and(|tx| tx.send(answer).is_ok()))
}

// ---------- 交给后台的事（子代理，ADR 029 第二期；照 Codex / OpenCode / Goose） ----------
//
// - 主会话用 `agent.start` / `agent.followup` 派活，最多等 4 秒：做完了当同步结果，没做完就让主会话
//   说“好了告诉你”结束这一轮（`kotone_agent::subagents`）；
// - 一件事是一段隐藏的对话（`parent` = 主会话），登记在主会话的 `subagents/<id>.json`；
// - 一轮做完先存盘登记（`reported = false`）再交回：主会话空下来（她没在回答、玩家没在说话）就插一条
//   `agent-report` 消息并唤醒主会话说一句；玩家先开口了就在那一轮前面带上；
// - 要玩家确认的事经主会话问：玩家最近是用嘴说的就开口问，打字的就在卡片上点。
