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
use kotone_agent::port::{BuiltinHost, KotonePort, Scope, Session};
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

/// 游戏内快速模式的上限（ADR 014 §4：只给常用工具、限制步数，玩家在等）。
fn game_limits() -> AgentLimits {
    AgentLimits {
        max_steps: 6,
        max_duration: std::time::Duration::from_secs(60),
        history_budget_chars: 20_000,
        ..AgentLimits::default()
    }
}

/// 消息来源 → 入口：聊天窗口里打的字是聊天，其余（热键语音、快捷指令、工具）都在游戏里。
fn entry_of(message: &Message) -> Entry {
    match message {
        Message::User { source, .. } if source == "chat" || source == "assistant-text" => {
            Entry::Chat
        }
        _ => Entry::Game,
    }
}

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
    /// `chat` / `assistant-text`（聊天窗口，回复看文字）；`voice` / `quick-command` / `tool`
    /// （游戏内：快速模式，回复用语音 / 弹幕播出）。
    pub source: String,
    pub attachments: Vec<Attachment>,
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
    tauri::async_runtime::spawn(async move {
        drive(app, conversation, message, cancel, inbox).await;
    });
    Ok(json!({"status": "started", "conversationId": id}))
}

/// 一轮接一轮地跑，直到没有待处理的消息。
async fn drive(
    app: AppHandle,
    mut conversation: Conversation,
    first: Message,
    cancel: CancellationToken,
    inbox: Arc<Inbox>,
) {
    let mut next = Some(first);
    loop {
        while let Some(user) = next.take() {
            let entry = entry_of(&user);
            let start = conversation.messages.len();
            let stop = turn(&app, &mut conversation, user, entry, &cancel, &inbox).await;
            if entry == Entry::Game {
                speak_outcome(
                    &app,
                    &conversation.messages[start.min(conversation.messages.len())..],
                    &stop,
                );
            }
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
    let mut user = user;
    if entry == Entry::Game {
        attach_screen(app, &mut user).await;
    }
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
            entry,
        },
    )
    .with_scope(if entry == Entry::Game {
        Scope::Game
    } else {
        Scope::Full
    });
    let profile = crate::companion::active_profile(&settings);
    let system = system_prompt(&PromptContext {
        entry,
        profile_name: &profile.display_name,
        profile_notes: &profile.companion_notes,
        automations: &automations,
    });
    let model = OpenAiModel::new(connection);
    let limits = if entry == Entry::Game {
        game_limits()
    } else {
        AgentLimits::default()
    };
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

/// 游戏内：请求前先读一次屏幕（截前台窗口、识别文字、更新聊天跟踪），作为数据附上，
/// 模型多数时候不用再调工具（ADR 014 §4）。读不到时附上原因，不中断。
async fn attach_screen(app: &AppHandle, user: &mut Message) {
    let Message::User {
        text, attachments, ..
    } = user
    else {
        return;
    };
    if !attachments.is_empty() {
        return;
    }
    let data = match crate::companion::read_chat(app, text, crate::companion::Source::Voice).await {
        Ok(snapshot) => {
            let input = &snapshot.input;
            let lines: Vec<Value> = input
                .ocr_lines
                .iter()
                .take(120)
                .map(|l| json!({"text": l.text, "box": l.bbox.map(|b| [b[0].round(), b[1].round(), b[2].round(), b[3].round()])}))
                .collect();
            let tracked: Vec<Value> = input
                .tracked
                .iter()
                .rev()
                .take(60)
                .rev()
                .map(|m| json!({"text": m.text, "new": m.new, "secondsAgo": m.first_seen_ago_ms / 1000}))
                .collect();
            json!({
                "window": {"title": input.window_title, "process": input.process_name,
                           "width": input.image_size.0, "height": input.image_size.1},
                "ocrLines": lines,
                "chatTracking": tracked,
                "warnings": snapshot.warnings,
            })
            .to_string()
        }
        Err(e) => format!("（没能读取画面：{e}）"),
    };
    attachments.push(Attachment::Data {
        label: "按热键时的游戏画面（文字识别）与聊天跟踪".into(),
        text: data,
    });
}

/// 游戏内：玩家看不到窗口，这一轮的结果用语音 / 弹幕播出。
/// Agent 已经用 `reply.say` 播过的不再重复；出错或被停下时播一句原因；玩家自己停止的不播。
fn speak_outcome(app: &AppHandle, appended: &[Message], stop: &StopReason) {
    if let Some(text) = game_reply(appended, stop) {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(e) = crate::assistant::deliver_text(&app, text, Vec::new()).await {
                log::log(&format!("agent: 回复未能播出：{e}"));
            }
        });
    }
}

/// 游戏内这一轮要播的话（纯函数）：出错 / 被上限停下 → 原因；正常结束 → 最后的回答，
/// 但已经用 `reply.say` 播过就不重复；玩家自己停止的不播。
fn game_reply(appended: &[Message], stop: &StopReason) -> Option<String> {
    if *stop == StopReason::Cancelled {
        return None;
    }
    let said = appended.iter().any(|m| {
        matches!(m, Message::Assistant { tool_calls, .. }
            if tool_calls.iter().any(|c| c.name == kotone_agent::catalog::TOOL_SAY))
    });
    let reply = appended.iter().rev().find_map(|m| match m {
        Message::Assistant { text, .. } if !text.trim().is_empty() => Some(text.trim().to_string()),
        _ => None,
    });
    let text = match stop.notice() {
        Some(notice) => Some(notice),
        None if said => None,
        None => reply,
    };
    text.map(|t| t.chars().take(300).collect())
}

/// 陪玩热键 / 快捷指令 / 工具入口交给琴音（`agent.inGame`）。立即返回，结果在后台播出。
pub fn submit_from_assistant(
    app: &AppHandle,
    text: String,
    source: &str,
) -> Result<String, String> {
    let result = send(
        app,
        Outgoing {
            text,
            source: source.to_string(),
            attachments: Vec::new(),
        },
    )?;
    Ok(match result["status"].as_str() {
        Some("steered") => "琴音正在处理，已把这句话交给她".into(),
        _ => "已交给琴音".into(),
    })
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
    entry: Entry,
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
        if self.entry == Entry::Game {
            // 游戏内：先用语音 / 弹幕告诉玩家，再等窗口里的回答（语音确认以后再做）。
            let text = format!(
                "需要你确认：{}。{}。请到琴音窗口里点允许或拒绝。",
                request.title, request.reason
            );
            let app = self.app.clone();
            tauri::async_runtime::spawn(async move {
                let _ = crate::assistant::deliver_text(&app, text, Vec::new()).await;
            });
        }
        let _ = self.app.emit(
            ASK_EVENT,
            json!({"askId": ask_id, "conversationId": self.conversation_id, "request": request}),
        );
        // 这一轮被停止或超时时，循环会直接丢掉这个 future：由守卫在任何结束方式下
        // 清掉登记并通知界面收起按钮。
        let mut guard = AskGuard {
            app: self.app.clone(),
            ask_id,
            conversation_id: self.conversation_id.clone(),
            answer: Answer::Deny,
        };
        guard.answer = tokio::select! {
            biased;
            _ = cancel.cancelled() => Answer::Deny,
            answer = rx => answer.unwrap_or(Answer::Deny),
        };
        guard.answer
    }
}

struct AskGuard {
    app: AppHandle,
    ask_id: String,
    conversation_id: String,
    answer: Answer,
}

impl Drop for AskGuard {
    fn drop(&mut self) {
        if let Ok(rt) = runtime(&self.app) {
            rt.inner.lock().unwrap().asks.remove(&self.ask_id);
        }
        let _ = self.app.emit(
            ASK_RESOLVED_EVENT,
            json!({"askId": self.ask_id, "conversationId": self.conversation_id, "answer": self.answer}),
        );
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
    rt.store.clone()?.delete(&id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kotone_ai::agent::ToolCall;

    fn user(source: &str) -> Message {
        Message::User {
            text: "嗨".into(),
            attachments: Vec::new(),
            source: source.into(),
            at_ms: 0,
        }
    }

    fn assistant(text: &str, calls: &[&str]) -> Message {
        Message::Assistant {
            text: text.into(),
            tool_calls: calls
                .iter()
                .map(|name| ToolCall {
                    id: "c".into(),
                    name: (*name).into(),
                    arguments: "{}".into(),
                })
                .collect(),
            reasoning: String::new(),
            usage: None,
            at_ms: 0,
        }
    }

    #[test]
    fn chat_window_text_is_chat_everything_else_is_in_game() {
        assert_eq!(entry_of(&user("chat")), Entry::Chat);
        assert_eq!(entry_of(&user("assistant-text")), Entry::Chat);
        for source in ["voice", "quick-command", "tool", "automation", ""] {
            assert_eq!(entry_of(&user(source)), Entry::Game, "{source}");
        }
    }

    #[test]
    fn in_game_reply_is_spoken_once_and_errors_are_spoken() {
        let done = StopReason::Completed;
        let turn = [
            user("voice"),
            assistant("", &["automation.list"]),
            assistant("你有两个自动化", &[]),
        ];
        assert_eq!(game_reply(&turn, &done).as_deref(), Some("你有两个自动化"));
        // 已经用 reply.say 播过：不重复。
        let said = [
            user("voice"),
            assistant("", &["reply.say"]),
            assistant("好了", &[]),
        ];
        assert_eq!(game_reply(&said, &done), None);
        // 出错：播原因；玩家自己停止：不播。
        let failed = StopReason::Failed {
            message: "AI 连接失败".into(),
        };
        assert!(game_reply(&turn, &failed).unwrap().contains("AI 连接失败"));
        assert_eq!(game_reply(&turn, &StopReason::Cancelled), None);
        assert!(game_reply(&turn, &StopReason::StepLimit).is_some());
        // 没有任何回答文字：不播空话。
        assert_eq!(game_reply(&[user("voice")], &done), None);
        let long = [assistant(&"字".repeat(400), &[])];
        assert_eq!(game_reply(&long, &done).unwrap().chars().count(), 300);
    }

    #[test]
    fn game_limits_are_tighter_than_chat() {
        let game = game_limits();
        let chat = AgentLimits::default();
        assert!(game.max_steps < chat.max_steps);
        assert!(game.max_duration < chat.max_duration);
    }
}
