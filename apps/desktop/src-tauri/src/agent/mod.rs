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
//! - 她正忙时插进来、要进聊天的（插件的推送、复盘、自动化的结果）：排队，不丢；怎么排只写在 `when_free.rs`。
//!
//! 不做：游戏内入口与快速模式（第 4 步）、切换到游戏窗口（第 5 步）。

mod browser_tools;
mod commands;
mod context;
mod exec_tools;
mod host;
mod push;
mod relay;
mod subagents;
mod turn;
mod voice;
mod when_free;

#[cfg(test)]
mod test_util;

use browser_tools::*;
pub use commands::*;
use context::*;
use exec_tools::*;
use host::*;
pub use push::*;
pub use relay::run_prompt;
use relay::*;
pub use subagents::*;
pub use turn::*;
use voice::*;
use when_free::*;
pub use when_free::{post_notice, resume_held_cards};

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use kotone_agent::catalog::{saved_entry, Catalog, MAX_SAVED_TOOLS, SAVED_PREFIX};
use kotone_agent::port::{BuiltinHost, KotonePort, Scope, Session};
use kotone_agent::prompt::{
    capability_overview, child_prompt, child_turn_context, system_prompt, turn_context, AsrState,
    Calling, Entry, PromptContext, TurnContext,
};
use kotone_agent::workspace::Workspace;
use kotone_agent::{Answer, Asker, PermissionRequest};
use kotone_ai::agent::model::AgentModel;
use kotone_ai::agent::store::{Conversation, ConversationStore, Subagent, SubagentStatus};
use kotone_ai::agent::{
    now_ms, run_turn, AgentEvent, AgentLimits, Attachment, Inbox, Message, StopReason, ToolOutcome,
    ToolStatus, Turn,
};
use kotone_core::automation::{Run, RunStatus};
use kotone_core::cancel::CancellationToken;
use kotone_core::log;
use kotone_core::reminders::DeliveryKind;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::oneshot;

pub const EVENT: &str = "kotone://agent-event";
pub const ASK_EVENT: &str = "kotone://agent-ask";
pub const ASK_RESOLVED_EVENT: &str = "kotone://agent-ask-resolved";

struct Running {
    conversation_id: String,
    cancel: CancellationToken,
    inbox: Arc<Inbox>,
    /// 没有模型在跑：主会话里转到后台的一个工具（[`move_tool`]，ADR 029 §一）。不占后台的名额
    /// （`MAX_RUNNING` 管的是同时在跑的模型），也插不进话。
    moved_tool: bool,
}

/// 交给后台的一件事（子代理，ADR 029 第二期）：它属于哪段主会话、叫什么、费用记到哪一轮。
#[derive(Clone)]
pub struct ChildSpec {
    pub parent: String,
    pub name: String,
    pub label: String,
    /// 派它出去（或最近一次追加要求）的那一轮：模型、搜索的费用记到主会话的这一轮。
    pub owner: kotone_core::usage::Owner,
}

struct Child {
    spec: ChildSpec,
    running: Option<Running>,
    /// 等这一轮结果的 `agent.start` / `agent.followup`（最多 4 秒），按任务消息的来源编号对应。
    waiters: HashMap<String, oneshot::Sender<(SubagentStatus, String)>>,
    /// 在等玩家确认：界面上点了按钮（`agent_subagent_answer`）就交回这里。
    ask: Option<oneshot::Sender<Answer>>,
}

#[derive(Default)]
struct Inner {
    running: Option<Running>,
    /// 后台的事：它那段对话的 id → 运行时状态（只在内存里；重启后只剩登记和记录）。
    children: HashMap<String, Child>,
    /// 主会话 id → 交给后台的事的登记（[`registry`] 第一次用时从盘上读）。
    registries: HashMap<String, Vec<Subagent>>,
    /// 主会话 id → 玩家最近一次在这段对话里是怎么说的（结果交回、要确认时照它开口或写字）。
    last_entry: HashMap<String, Entry>,
    /// 正在等主会话空下来、要把结果交回去的主会话。
    reporting: HashSet<String>,
    next_tag: u64,
    /// 每段对话跨轮保留的状态（tools.find 启用的工具、“本段对话都允许”）。只在内存里。
    sessions: HashMap<String, Arc<Session>>,
    asks: HashMap<String, oneshot::Sender<Answer>>,
    next_ask: u64,
    /// 用嘴问过、还在等的确认（最近问的在最后）：问完那几秒过了，玩家之后用嘴说“可以 / 不要”
    /// 也算回答（[`answer_by_voice`]），不用再去窗口里点。
    spoken_asks: Vec<SpokenAsk>,
    /// 执行记录 id → 发起它的对话。
    run_owners: HashMap<String, String>,
}

pub struct AgentRuntime {
    store: std::sync::OnceLock<Result<Arc<ConversationStore>, String>>,
    inner: Mutex<Inner>,
}

impl AgentRuntime {
    pub fn open(companion_enabled: bool) -> Self {
        let runtime = Self {
            store: std::sync::OnceLock::new(),
            inner: Mutex::new(Inner::default()),
        };
        if companion_enabled {
            let _ = runtime.conversation_store();
        }
        runtime
    }

    fn conversation_store(&self) -> Result<Arc<ConversationStore>, String> {
        self.store
            .get_or_init(|| {
                let store =
                    ConversationStore::open(kotone_core::settings::kotone_dir().join("agent"))
                        .map(Arc::new);
                if let Err(e) = &store {
                    log::log(&format!("agent: conversation store unavailable: {e}"));
                }
                store
            })
            .clone()
    }
}

/// 琴音的工作区（`<KOTONE_HOME>/agent/workspace`，ADR 022）。
pub fn workspace() -> Workspace {
    let ws = Workspace::new(
        kotone_core::settings::kotone_dir()
            .join("agent")
            .join("workspace"),
    )
    .with_role(&crate::roles::cached_id());
    if let Err(e) = ws.ensure() {
        log::log(&format!("agent: workspace unavailable: {e}"));
    }
    ws
}

/// 一次用嘴问过的确认：主会话的（界面卡片的 ask id）或后台的（那件事的对话 id）。
#[derive(Clone, PartialEq, Eq)]
enum SpokenAsk {
    Main(String),
    Child(String),
}

fn add_spoken_ask(app: &AppHandle, ask: SpokenAsk) {
    if let Ok(rt) = runtime(app) {
        let mut inner = rt.inner.lock().unwrap();
        inner.spoken_asks.retain(|a| *a != ask);
        inner.spoken_asks.push(ask);
    }
}

fn remove_spoken_ask(app: &AppHandle, ask: &SpokenAsk) {
    if let Ok(rt) = runtime(app) {
        rt.inner.lock().unwrap().spoken_asks.retain(|a| a != ask);
    }
}

/// 有用嘴问过、还在等回答的确认吗（通话等它时不计空闲，ADR 033）。
pub fn has_spoken_asks(app: &AppHandle) -> bool {
    runtime(app).is_ok_and(|rt| !rt.inner.lock().unwrap().spoken_asks.is_empty())
}

/// 有用嘴问过、还在等的确认，而玩家这句只是“可以 / 允许 / 不要”：交给最近问的那个。
/// 返回是否交出去了（交出去了这句就不再交给琴音）。
pub fn answer_by_voice(app: &AppHandle, text: &str) -> bool {
    let Ok(rt) = runtime(app) else {
        return false;
    };
    let name = settings(app).agent.identity.name;
    let mut inner = rt.inner.lock().unwrap();
    if inner.spoken_asks.is_empty() {
        return false;
    }
    let Some(ok) = crate::voice_ask::bare_confirmation(text, &name) else {
        return false;
    };
    let answer = if ok { Answer::AllowOnce } else { Answer::Deny };
    while let Some(ask) = inner.spoken_asks.pop() {
        let sender = match &ask {
            SpokenAsk::Main(id) => inner.asks.remove(id),
            SpokenAsk::Child(id) => inner.children.get_mut(id).and_then(|c| c.ask.take()),
        };
        if sender.is_some_and(|tx| tx.send(answer).is_ok()) {
            log::log(&format!(
                "agent: 玩家用嘴回答了还在等的确认（{}）",
                if ok { "允许" } else { "拒绝" }
            ));
            return true;
        }
    }
    false
}

fn runtime(app: &AppHandle) -> Result<tauri::State<'_, AgentRuntime>, String> {
    app.try_state::<AgentRuntime>()
        .ok_or_else(|| "还没准备好".to_string())
}

fn store(app: &AppHandle) -> Result<Arc<ConversationStore>, String> {
    if app
        .state::<crate::SharedState>()
        .settings
        .read()
        .unwrap()
        .ui
        .legacy_upgrade
        .uses_legacy_assistant()
    {
        return Err("还没启用琴音：先在聊天页添加一个 DeepSeek Key".into());
    }
    runtime(app)?.conversation_store()
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
        AgentEvent::ToolCallChunk { call_id, .. } => {
            json!({"kind": "toolCallChunk", "callId": call_id})
        }
        AgentEvent::ToolCallUpdate {
            call_id,
            status,
            content,
        } => {
            json!({"kind": "toolCallUpdate", "callId": call_id, "status": status, "content": content})
        }
        AgentEvent::Appended { message } => json!({"kind": "appended", "message": message}),
        AgentEvent::Compacted { index, message } => {
            json!({"kind": "compacted", "index": index, "message": message})
        }
        AgentEvent::Discarded { .. } => json!({"kind": "discarded"}),
        AgentEvent::Finished { stop } => {
            json!({"kind": "finished", "stop": stop, "acpStopReason": stop.acp()})
        }
    }
}

// ---------- 发送 ----------
