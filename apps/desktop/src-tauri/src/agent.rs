//! 内置琴音 Agent 的桌面运行时（ADR 014、ADR 015）。
//!
//! - 对话存在 `~/.kotone/agent/`；游戏内与聊天窗口共用“当前对话”（ADR 014“已确认”）。
//! - 同一时间只跑一轮；运行中收到新消息算**插话**（当前工具执行完后交给模型），
//!   本轮结束后还有没处理的就接着跑下一轮。
//! - 工具：常驻工具服务的同一份实现（`tool_service::service`），加内置动作与已保存的自动化，
//!   经 `kotone_agent::KotonePort`（权限：允许 / 询问 / 禁止）。
//! - 询问：发 `kotone://agent-ask` 事件，等界面调 `agent_answer`；停止时按“拒绝”收尾。
//! - 事件：`kotone://agent-event`（按 ACP 会话更新设计），每条新消息同时存盘。
//! - 自动化由某段对话发起（`automation.run` / `saved.*`）时，结束后把结果作为一条消息
//!   放回那段对话（正在跑就作为插话交给模型）。
//!
//! 不做：游戏内入口与快速模式（第 4 步）、切换到游戏窗口（第 5 步）。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use kotone_agent::catalog::{saved_entry, Catalog, MAX_SAVED_TOOLS, SAVED_PREFIX};
use kotone_agent::port::{BuiltinHost, KotonePort, Session};
use kotone_agent::prompt::{system_prompt, Entry, PromptContext};
use kotone_agent::{Answer, Asker, PermissionRequest};
use kotone_ai::agent::model::OpenAiModel;
use kotone_ai::agent::store::{Conversation, ConversationStore};
use kotone_ai::agent::{
    now_ms, run_turn, AgentEvent, AgentLimits, Attachment, Inbox, Message, StopReason, ToolStatus,
    Turn,
};
use kotone_core::automation::{ChatAction, Run, RunStatus};
use kotone_core::cancel::CancellationToken;
use kotone_core::log;
use kotone_core::reminders::DeliveryKind;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::oneshot;

pub const EVENT: &str = "kotone://agent-event";
pub const ASK_EVENT: &str = "kotone://agent-ask";
pub const ASK_RESOLVED_EVENT: &str = "kotone://agent-ask-resolved";

/// 等待结果回到对话的执行记录最多记几条。
const MAX_RUN_OWNERS: usize = 200;

struct Running {
    conversation_id: String,
    cancel: CancellationToken,
    inbox: Arc<Inbox>,
}

#[derive(Default)]
struct Inner {
    running: Option<Running>,
    /// 每段对话跨轮保留的状态（tools.find 启用的工具、“本段对话都允许”）。只在内存里。
    sessions: HashMap<String, Arc<Session>>,
    asks: HashMap<String, oneshot::Sender<Answer>>,
    next_ask: u64,
    /// 执行记录 id → 发起它的对话。
    run_owners: HashMap<String, String>,
}

pub struct AgentRuntime {
    store: Result<Arc<ConversationStore>, String>,
    inner: Mutex<Inner>,
}

impl AgentRuntime {
    pub fn open() -> Self {
        let store = ConversationStore::open(kotone_core::settings::kotone_dir().join("agent"))
            .map(Arc::new);
        if let Err(e) = &store {
            log::log(&format!("agent: conversation store unavailable: {e}"));
        }
        Self {
            store,
            inner: Mutex::new(Inner::default()),
        }
    }
}

fn runtime(app: &AppHandle) -> Result<tauri::State<'_, AgentRuntime>, String> {
    app.try_state::<AgentRuntime>()
        .ok_or_else(|| "琴音尚未就绪".to_string())
}

fn store(app: &AppHandle) -> Result<Arc<ConversationStore>, String> {
    runtime(app)?.store.clone()
}

fn settings(app: &AppHandle) -> kotone_core::settings::Settings {
    app.state::<crate::SharedState>()
        .settings
        .read()
        .unwrap()
        .clone()
}

fn emit(app: &AppHandle, conversation_id: &str, event: Value) {
    let _ = app.emit(
        EVENT,
        json!({"conversationId": conversation_id, "event": event}),
    );
}

/// 事件的对外形状（按 ACP `session/update` 设计，见 kotone_ai::agent 模块说明）。
fn event_json(event: &AgentEvent) -> Value {
    match event {
        AgentEvent::StepStarted { step } => json!({"kind": "step", "step": step}),
        AgentEvent::MessageChunk { text } => json!({"kind": "chunk", "text": text}),
        AgentEvent::ThoughtChunk { text } => json!({"kind": "thought", "text": text}),
        AgentEvent::ToolCall { call } => json!({"kind": "toolCall", "call": call}),
        AgentEvent::ToolCallUpdate {
            call_id,
            status,
            content,
        } => {
            json!({"kind": "toolCallUpdate", "callId": call_id, "status": status, "content": content})
        }
        AgentEvent::Appended { message } => json!({"kind": "appended", "message": message}),
        AgentEvent::Finished { stop } => {
            json!({"kind": "finished", "stop": stop, "acpStopReason": stop.acp()})
        }
    }
}

// ---------- 发送 ----------

/// 一条要交给琴音的消息。
pub struct Outgoing {
    pub text: String,
    /// `chat` / `voice` / `quick-command` ……
    pub source: String,
    pub attachments: Vec<Attachment>,
    pub entry: Entry,
}

/// 发一条消息：没有在跑就开始一轮；正在跑就作为插话。返回 `{status, conversationId}`。
pub fn send(app: &AppHandle, outgoing: Outgoing) -> Result<Value, String> {
    let text = outgoing.text.trim().to_string();
    if text.is_empty() {
        return Err("消息为空".into());
    }
    let message = Message::User {
        text,
        attachments: outgoing.attachments,
        source: outgoing.source,
        at_ms: now_ms(),
    };
    let rt = runtime(app)?;
    let store = rt.store.clone()?;
    {
        let inner = rt.inner.lock().unwrap();
        if let Some(running) = &inner.running {
            running.inbox.steer(message);
            return Ok(json!({"status": "steered", "conversationId": running.conversation_id}));
        }
    }
    let conversation = match store.current() {
        Some(id) => store.load(&id)?.unwrap_or_else(|| store.create()),
        None => store.create(),
    };
    let cancel = CancellationToken::default();
    let inbox = Arc::new(Inbox::default());
    {
        let mut inner = rt.inner.lock().unwrap();
        if let Some(running) = &inner.running {
            running.inbox.steer(message);
            return Ok(json!({"status": "steered", "conversationId": running.conversation_id}));
        }
        inner.running = Some(Running {
            conversation_id: conversation.id.clone(),
            cancel: cancel.clone(),
            inbox: inbox.clone(),
        });
    }
    store.set_current(Some(&conversation.id))?;
    let id = conversation.id.clone();
    let app = app.clone();
    let entry = outgoing.entry;
    tauri::async_runtime::spawn(async move {
        drive(app, conversation, message, entry, cancel, inbox).await;
    });
    Ok(json!({"status": "started", "conversationId": id}))
}

/// 一轮接一轮地跑，直到没有待处理的消息。
async fn drive(
    app: AppHandle,
    mut conversation: Conversation,
    first: Message,
    entry: Entry,
    cancel: CancellationToken,
    inbox: Arc<Inbox>,
) {
    let mut next = Some(first);
    loop {
        while let Some(user) = next.take() {
            let stop = turn(&app, &mut conversation, user, entry, &cancel, &inbox).await;
            if stop == StopReason::Cancelled || cancel.is_cancelled() {
                inbox.clear();
                break;
            }
            next = inbox.next_follow_up();
        }
        // 在同一把锁下确认没有新消息再收尾（send 也在这把锁下插话），不会丢消息。
        let Ok(rt) = runtime(&app) else { return };
        let mut inner = rt.inner.lock().unwrap();
        if !cancel.is_cancelled() {
            if let Some(message) = inbox.next_follow_up() {
                drop(inner);
                next = Some(message);
                continue;
            }
        }
        if inner
            .running
            .as_ref()
            .is_some_and(|r| r.conversation_id == conversation.id)
        {
            inner.running = None;
        }
        break;
    }
    emit(&app, &conversation.id, json!({"kind": "idle"}));
}

/// 开始前就失败（没有连接、工具服务未就绪）：把用户的话和原因记进对话。
fn fail_early(
    app: &AppHandle,
    store: &ConversationStore,
    conversation: &mut Conversation,
    user: Message,
    message: String,
) -> StopReason {
    let stop = StopReason::Failed { message };
    for m in [
        user,
        Message::Notice {
            text: stop.notice().unwrap_or_default(),
            at_ms: now_ms(),
        },
    ] {
        conversation.messages.push(m.clone());
        emit(
            app,
            &conversation.id,
            event_json(&AgentEvent::Appended { message: m }),
        );
    }
    if let Err(e) = store.save(conversation) {
        log::log(&format!("agent: save failed: {e}"));
    }
    emit(
        app,
        &conversation.id,
        event_json(&AgentEvent::Finished { stop: stop.clone() }),
    );
    stop
}

/// 交给模型的已保存自动化：没被隐藏、模型能调用（没有必填图片），最多 30 个。
fn offered_automations(
    app: &AppHandle,
    hidden: &[String],
) -> Vec<kotone_core::automation::library::Automation> {
    crate::automation::saved_list(app)
        .into_iter()
        .filter(|a| !hidden.contains(&a.id))
        .filter(|a| saved_entry(a).is_some())
        .take(MAX_SAVED_TOOLS)
        .collect()
}

async fn turn(
    app: &AppHandle,
    conversation: &mut Conversation,
    user: Message,
    entry: Entry,
    cancel: &CancellationToken,
    inbox: &Arc<Inbox>,
) -> StopReason {
    let Ok(store) = store(app) else {
        return StopReason::Failed {
            message: "对话存储不可用".into(),
        };
    };
    let settings = settings(app);
    let connection = match crate::companion::connection_ready(app, &settings) {
        Ok(c) => c,
        Err(e) => return fail_early(app, &store, conversation, user, e),
    };
    let Some(service) = crate::tool_service::service(app) else {
        return fail_early(
            app,
            &store,
            conversation,
            user,
            "工具服务尚未就绪，请稍后再试".into(),
        );
    };
    let specs = service.capabilities()["tools"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let automations = offered_automations(app, &settings.agent.hidden_automations);
    let catalog = Catalog::build(&specs, &automations);
    let session = match runtime(app) {
        Ok(rt) => rt
            .inner
            .lock()
            .unwrap()
            .sessions
            .entry(conversation.id.clone())
            .or_default()
            .clone(),
        Err(_) => Arc::new(Session::default()),
    };
    let port = KotonePort::new(
        service,
        catalog,
        settings.agent.permissions.clone(),
        session,
        DesktopBuiltins {
            app: app.clone(),
            session: format!("agent:{}", conversation.id),
        },
        DesktopAsker {
            app: app.clone(),
            conversation_id: conversation.id.clone(),
        },
    );
    let profile = crate::companion::active_profile(&settings);
    let system = system_prompt(&PromptContext {
        entry,
        profile_name: &profile.display_name,
        profile_notes: &profile.companion_notes,
        automations: &automations,
    });
    let model = OpenAiModel::new(connection);
    let limits = AgentLimits::default();
    let conversation_id = conversation.id.clone();
    let shared = Mutex::new(conversation.clone());
    let tool_names: Mutex<HashMap<String, String>> = Mutex::default();
    let on_event = |event: AgentEvent| {
        match &event {
            AgentEvent::Appended { message } => {
                let mut c = shared.lock().unwrap();
                c.messages.push(message.clone());
                if let Err(e) = store.save(&mut c) {
                    log::log(&format!("agent: save failed: {e}"));
                }
            }
            AgentEvent::ToolCall { call } => {
                tool_names
                    .lock()
                    .unwrap()
                    .insert(call.id.clone(), call.name.clone());
            }
            AgentEvent::ToolCallUpdate {
                call_id,
                status: ToolStatus::Completed,
                content: Some(content),
            } => {
                let name = tool_names.lock().unwrap().get(call_id).cloned();
                if name.is_some_and(|n| n == "automation.run" || n.starts_with(SAVED_PREFIX)) {
                    remember_run(app, content, &conversation_id);
                }
            }
            _ => {}
        }
        emit(app, &conversation_id, event_json(&event));
    };
    let mut history = conversation.messages.clone();
    let stop = run_turn(
        &model,
        &port,
        &mut history,
        Turn {
            system: &system,
            user,
            limits: &limits,
            cancel,
            inbox: Some(inbox),
        },
        &on_event,
    )
    .await;
    *conversation = shared.into_inner().unwrap();
    stop
}

/// `automation.run` / `saved.*` 的结果里有执行记录 id：记下它属于哪段对话。
fn remember_run(app: &AppHandle, content: &str, conversation_id: &str) {
    let Some(id) = serde_json::from_str::<Value>(content)
        .ok()
        .and_then(|v| v.get("id").and_then(Value::as_str).map(str::to_string))
    else {
        return;
    };
    if let Ok(rt) = runtime(app) {
        let mut inner = rt.inner.lock().unwrap();
        if inner.run_owners.len() >= MAX_RUN_OWNERS {
            inner.run_owners.clear();
        }
        inner.run_owners.insert(id, conversation_id.to_string());
    }
}

/// 调度器的结束回调：由对话发起的执行，把结果放回那段对话。
pub fn on_run_finished(app: &AppHandle, run: &Run) {
    let Ok(rt) = runtime(app) else { return };
    let Some(conversation_id) = rt.inner.lock().unwrap().run_owners.remove(&run.id) else {
        return;
    };
    let status = match run.status {
        RunStatus::Finished => "已完成",
        RunStatus::Failed => "失败",
        RunStatus::Cancelled => "已停止",
        RunStatus::Missed => "错过了",
        RunStatus::Pending | RunStatus::Running => return,
    };
    let message = Message::User {
        text: format!(
            "（系统消息，不是玩家说的话：自动化「{}」{status}。执行记录见附件；需要的话告诉玩家结果。）",
            run.title()
        ),
        attachments: vec![Attachment::Data {
            label: "执行记录".into(),
            text: crate::automation::run_json(run, false).to_string(),
        }],
        source: "automation".into(),
        at_ms: now_ms(),
    };
    {
        let inner = rt.inner.lock().unwrap();
        if let Some(running) = inner
            .running
            .as_ref()
            .filter(|r| r.conversation_id == conversation_id)
        {
            running.inbox.steer(message);
            return;
        }
    }
    // 没在跑：只记进对话，不自动调用模型（不花钱）；玩家下次说话时模型能看到。
    let Ok(store) = rt.store.clone() else { return };
    match store.load(&conversation_id) {
        Ok(Some(mut conversation)) => {
            conversation.messages.push(message.clone());
            if let Err(e) = store.save(&mut conversation) {
                log::log(&format!("agent: save failed: {e}"));
            }
            emit(
                app,
                &conversation_id,
                event_json(&AgentEvent::Appended { message }),
            );
        }
        Ok(None) => {}
        Err(e) => log::log(&format!("agent: load failed: {e}")),
    }
}

// ---------- 内置动作与询问 ----------

struct DesktopBuiltins {
    app: AppHandle,
    /// 写进执行记录的发起方：`agent:<对话 id>`。
    session: String,
}

fn delivery_names(kinds: &[DeliveryKind]) -> Vec<&'static str> {
    kinds
        .iter()
        .map(|k| match k {
            DeliveryKind::Voice => "voice",
            DeliveryKind::Danmaku => "danmaku",
        })
        .collect()
}

impl BuiltinHost for DesktopBuiltins {
    async fn say(&self, text: String, delivery: Vec<DeliveryKind>) -> Result<Value, String> {
        let delivered = crate::assistant::deliver_text(&self.app, text, delivery).await?;
        Ok(json!({"delivered": delivery_names(&delivered)}))
    }

    async fn remind(
        &self,
        delay_seconds: u64,
        text: String,
        delivery: Vec<DeliveryKind>,
    ) -> Result<Value, String> {
        crate::automation::schedule_reminder(
            &self.app,
            &self.session,
            text,
            delivery,
            delay_seconds,
        )
    }

    async fn send_to_game_chat(&self, text: String, send: Option<bool>) -> Result<Value, String> {
        // 前台是 Kotone 自己（例如玩家正在聊天窗口里）时不能打字，否则字会打进 Kotone。
        crate::automation::foreground_target()
            .map_err(|_| "前台不是游戏窗口：请先切到游戏，再让我替你发言".to_string())?;
        let settings = settings(&self.app);
        let send = send.unwrap_or(settings.assistant.companion.auto_send);
        let profile = crate::companion::active_profile(&settings).id;
        let result = crate::companion::type_text(&self.app, &text, send).await;
        let record = crate::automation::record_chat(
            &self.app,
            &self.session,
            ChatAction {
                text,
                sent: send && result.is_ok(),
                profile,
            },
            result.as_ref().map(|_| ()).map_err(Clone::clone),
        );
        if let Err(e) = &record {
            log::log(&format!("agent: chat record failed: {e}"));
        }
        let mut value = result?;
        if let Ok(run) = record {
            value["runId"] = json!(run.id);
        }
        Ok(value)
    }
}

struct DesktopAsker {
    app: AppHandle,
    conversation_id: String,
}

impl Asker for DesktopAsker {
    async fn ask(&self, request: PermissionRequest, cancel: &CancellationToken) -> Answer {
        let (tx, rx) = oneshot::channel();
        let ask_id = {
            let Ok(rt) = runtime(&self.app) else {
                return Answer::Deny;
            };
            let mut inner = rt.inner.lock().unwrap();
            inner.next_ask += 1;
            let id = format!("ask-{}", inner.next_ask);
            inner.asks.insert(id.clone(), tx);
            id
        };
        let _ = self.app.emit(
            ASK_EVENT,
            json!({"askId": ask_id, "conversationId": self.conversation_id, "request": request}),
        );
        let answer = tokio::select! {
            biased;
            _ = cancel.cancelled() => Answer::Deny,
            answer = rx => answer.unwrap_or(Answer::Deny),
        };
        if let Ok(rt) = runtime(&self.app) {
            rt.inner.lock().unwrap().asks.remove(&ask_id);
        }
        let _ = self.app.emit(
            ASK_RESOLVED_EVENT,
            json!({"askId": ask_id, "conversationId": self.conversation_id, "answer": answer}),
        );
        answer
    }
}

// ---------- 对话管理 ----------

pub fn stop(app: &AppHandle) -> Result<bool, String> {
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

fn ensure_idle(app: &AppHandle) -> Result<(), String> {
    if runtime(app)?.inner.lock().unwrap().running.is_some() {
        Err("琴音正在回答，请先停止".into())
    } else {
        Ok(())
    }
}

pub fn state(app: &AppHandle) -> Result<Value, String> {
    let rt = runtime(app)?;
    let store = rt.store.clone()?;
    let running = rt
        .inner
        .lock()
        .unwrap()
        .running
        .as_ref()
        .map(|r| r.conversation_id.clone());
    let settings = settings(app);
    let ready = crate::companion::connection_ready(app, &settings).err();
    Ok(json!({
        "current": store.current(),
        "running": running,
        "conversations": store.list(),
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
            entry: Entry::Chat,
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
pub fn agent_state(app: AppHandle) -> Result<Value, String> {
    state(&app)
}

#[tauri::command]
pub fn agent_conversation(app: AppHandle, id: String) -> Result<Option<Conversation>, String> {
    store(&app)?.load(&id)
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
    rt.store.clone()?.delete(&id)
}
