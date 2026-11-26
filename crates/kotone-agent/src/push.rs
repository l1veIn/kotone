//! 推送（ADR 043 D3–D5）的纯逻辑：插件不叫模型、自己发进聊天的一条（卡片 / 一行 / 语音），**不花钱**。
//! 以前叫“自动卡片”（ADR 039 D3），卡片现在是推送的一种样子。
//!
//! 一条推送在对话里是一或两条消息，都追加在末尾（ADR 024“历史只追加”，前缀缓存照常命中）：
//!
//! 1. 记录：[`Message::Notice`] 带 `push`（[`notice`]）。`Notice` 本来就不发给模型；界面把它画成插件卡片（有卡片时）
//!    或一条小字记录（一行、语音），上面写“{插件名} 推送”。`push` 的形状见 [`record`]；卡片的 `ui` /
//!    `structuredContent` 和工具结果的 `display` 同一个形状，聊天卡片用同一个组件。
//! 2. 给模型的那一句（推送带了 `note` 才有）：来源 [`SOURCE`] 的用户消息（[`note_message`]），说法照自动化跑完的消息
//!    （“系统消息，不是玩家说的话”）；插件给的那句里有别人起的名字，按数据附上（模型按不可信数据看，原则 9）。界面不显示它。
//!
//! **同一个 key 再推一次就是更新**（D5，[`decide`]）：内容一样什么都不做；变了就改那条 Notice 的 `push`（原地，位置不动），
//! 这次带了 `note` 就再追加一条给模型的。**发给模型的部分只追加；通知里的推送数据可以改**：Notice 不发给模型，改它不动
//! 发给模型的前缀（测试：更新前后 DeepSeek 请求体前面的每一条逐字节不变）。
//!
//! 宿主记住每个 `插件:key` 最后一次的内容（[`content_hash`]）和它在哪段对话（[`Record`]，最近 [`REMEMBER`] 个，只在内存里）；
//! Kotone 重启以后去对话里找（[`find`] + [`Record::of`]）。旧的自动卡片（rc.9 写在 Notice 的 `card` 里）也认得：当作发过、
//! 不再改它（读旧，原则 19）。
//!
//! 时机（[`Desk`] 的排队）：她空着时马上记进对话；不空时排着（推送照样马上显示在聊天里），这一轮结束再记，不插进正在跑的
//! 那一轮，也不叫她开口。**原地更新也照这条规矩**：那段对话正在跑一轮时（这一轮读了对话、跑完会整段存回去，这时写进去的
//! 会被盖掉），更新排着（[`Desk::hold_replace`]，同一条只留最新的），这一轮结束再改；还排着没记进对话的，直接换掉排着的那条
//! （[`Desk::update_held`]）。排着的存一份在盘上（[`Desk::restore`]），Kotone 退出了下次启动接着记；记进对话连着失败
//! [`MAX_FAILURES`] 次就放弃。
//!
//! 桌面端（`apps/desktop/src-tauri/src/agent/push.rs`、`when_free.rs`）管开关、存盘、界面事件、悬浮窗和语音。

use std::collections::VecDeque;

use kotone_ai::agent::{Attachment, Message};
use kotone_plugin_api::push::clip;
use kotone_plugin_api::{Push, PushSpec};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// 给模型的那条用户消息的来源（界面不显示它；对话标题也不拿它）。存盘的值，名字是只有自动卡片时起的，不改。
pub const SOURCE: &str = "auto-card";
/// 记住最近多少条推送的内容和对话。
pub const REMEMBER: usize = 256;
/// 排着的一组记进对话连着失败（读写对话出错，不算“她在忙”）几次就放弃。
pub const MAX_FAILURES: u32 = 5;

/// 一条推送的标识：插件 + key（key 前后的空白不算）。
pub fn id(plugin: &str, key: &str) -> String {
    format!("{plugin}:{}", key.trim())
}

/// Notice 里 `push` 的形状（界面 `agent.ts` 的 `PushRecord`，悬浮窗也用它）：
///
/// ```text
/// { plugin, pluginName, trigger, title, key,
///   card?: { title, ui: { uri, params? }, structuredContent },
///   line?: { cells: [{ text, tone? }] },        // tone = good / ok / bad
///   voice?: [说法…], said?: 这次说的那句,
///   updatedAtMs? }                               // 更新过才有（宿主写）
/// ```
///
/// `title` 是清单里这条推送的名字（“海克斯三选一”）；卡片有自己的名字（`card.title`）。`push` 已经截过（[`Push::shaped`]）。
pub fn record(plugin: &str, plugin_name: &str, spec: &PushSpec, push: &Push, said: &str) -> Value {
    let mut out = json!({
        "plugin": plugin,
        "pluginName": plugin_name,
        "trigger": spec.id,
        "title": spec.title,
        "key": push.key.trim(),
    });
    if let Some(card) = &push.card {
        let mut ui = json!({"uri": card.uri});
        if !card.params.is_null() {
            ui["params"] = card.params.clone();
        }
        out["card"] = json!({"title": card.title, "ui": ui, "structuredContent": card.data});
    }
    if let Some(line) = &push.line {
        out["line"] = serde_json::to_value(line).unwrap_or(Value::Null);
    }
    if !push.voice.is_empty() {
        out["voice"] = json!(push.voice);
        if !said.is_empty() {
            out["said"] = json!(said);
        }
    }
    out
}

/// 界面上这条推送叫什么：有卡片用卡片的名字，否则用清单里这条推送的名字。
pub fn shown_title(record: &Value) -> &str {
    record["card"]["title"]
        .as_str()
        .or_else(|| record["title"].as_str())
        .unwrap_or_default()
}

/// 推送那条（只给界面）。`at_ms` 是第一次推的时间（原地更新时不变：聊天里的位置和标识跟着它）。
pub fn notice(plugin_name: &str, record: Value, at_ms: u64) -> Message {
    Message::Notice {
        // 旧版本只认得 text：看到的是这一行
        text: format!("{plugin_name} 推送：{}", shown_title(&record)),
        at_ms,
        compaction: None,
        usage: None,
        recap: None,
        card: None,
        push: Some(record),
    }
}

/// 给模型的那条（推送带了 `note` 才有）：新推的跟在推送后面；更新的单独追加在对话末尾（`updated`）。
pub fn note_message(
    plugin_name: &str,
    title: &str,
    note: &str,
    updated: bool,
    at_ms: u64,
) -> Message {
    let what = if updated {
        format!("更新了推送「{title}」")
    } else {
        format!("发来一条推送「{title}」")
    };
    Message::User {
        text: format!(
            "（系统消息，不是玩家说的话：{plugin_name}插件{what}，玩家在聊天里看得到；要点见附件。）"
        ),
        attachments: vec![Attachment::Data {
            label: format!("{plugin_name} · {title}"),
            text: clip(note, Push::NOTE_MAX),
        }],
        source: SOURCE.into(),
        at_ms,
        context: String::new(),
    }
}

/// 推送消息里的 `push`（不是推送时 None；旧的自动卡片也是 None，见 [`legacy_card_of`]）。
pub fn push_of(message: &Message) -> Option<&Value> {
    match message {
        Message::Notice { push, .. } => push.as_ref(),
        _ => None,
    }
}

/// 旧的自动卡片（rc.9 写在 Notice 的 `card` 里：`{plugin, pluginName, key, title, popup, ui, structuredContent}`）。
pub fn legacy_card_of(message: &Message) -> Option<&Value> {
    match message {
        Message::Notice { card, .. } => card.as_ref(),
        _ => None,
    }
}

/// 一条推送消息（新的，或旧的自动卡片）的标识（[`id`]）；不是时 None。
pub fn push_id(message: &Message) -> Option<String> {
    let value = push_of(message).or_else(|| legacy_card_of(message))?;
    Some(id(value["plugin"].as_str()?, value["key"].as_str()?))
}

/// 这些消息里最后一条标识是 `id` 的推送（Kotone 重启以后去对话里找）。
pub fn find<'a>(messages: &'a [Message], id: &str) -> Option<&'a Message> {
    messages
        .iter()
        .rev()
        .find(|m| push_id(m).as_deref() == Some(id))
}

/// 内容的指纹：比“同一个 key 这次和上次一样不一样”（[`decide`]）。算的是 `trigger`、卡片、一行、语音的说法，不算插件名、
/// 这次说的哪句（轮着用）、更新时间。FNV-1a（照 <http://www.isthe.com/chongo/tech/comp/fnv/>，公有领域）：
/// 换 Rust 版本也不变，存进对话的内容重启后算出来的还是同一个。
pub fn content_hash(record: &Value) -> u64 {
    let content = json!({
        "trigger": record["trigger"],
        "card": record["card"],
        "line": record["line"],
        "voice": record["voice"],
    });
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in serde_json::to_string(&content).unwrap_or_default().bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

/// 几个说法轮着用：这一条推送第 `n` 次开口说哪句。
pub fn pick(variants: &[String], n: usize) -> &str {
    if variants.is_empty() {
        ""
    } else {
        &variants[n % variants.len()]
    }
}

/// 宿主记住的一条推送：最后一次的内容、在哪段对话。
#[derive(Debug, Clone, PartialEq)]
pub struct Record {
    pub conversation: String,
    /// 第一次推的时间（Notice 的 `at_ms`，原地更新时不变）。
    pub at_ms: u64,
    /// [`content_hash`]。
    pub hash: u64,
    /// 语音的说法：更新时说法没变就不再说（结论没变不重复，ADR 043 D7）。
    pub voice: Vec<String>,
    /// 上次说的那句（这次说法没变时记录里照旧写它）。
    pub said: String,
    /// 旧的自动卡片（rc.9 写的）：当作发过，不再改它。
    pub legacy: bool,
}

impl Record {
    /// 从对话里的一条推送消息（或旧的自动卡片）认出来（Kotone 重启以后）。
    pub fn of(conversation: &str, message: &Message) -> Option<Self> {
        let Message::Notice { at_ms, .. } = message else {
            return None;
        };
        if let Some(push) = push_of(message) {
            return Some(Self {
                conversation: conversation.to_string(),
                at_ms: *at_ms,
                hash: content_hash(push),
                voice: serde_json::from_value(push["voice"].clone()).unwrap_or_default(),
                said: push["said"].as_str().unwrap_or_default().to_string(),
                legacy: false,
            });
        }
        legacy_card_of(message).map(|_| Self {
            conversation: conversation.to_string(),
            at_ms: *at_ms,
            hash: 0,
            voice: Vec::new(),
            said: String::new(),
            legacy: true,
        })
    }
}

/// 来一条推送时怎么办（开关由宿主先查过）。
#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    /// 这个 key 第一次推：追加一条新的。
    Post,
    /// 同一个 key、内容变了：原地改上一次那条（`Record` 是上一次的）。
    Update(Record),
    /// 同一个 key、内容一样（或者是旧的自动卡片）：什么都不做。
    Unchanged,
}

/// 照上一次的记录（内存里的，或重启后从对话里认出来的）和这次内容的指纹决定。
pub fn decide(previous: Option<Record>, hash: u64) -> Decision {
    match previous {
        None => Decision::Post,
        Some(r) if r.legacy || r.hash == hash => Decision::Unchanged,
        Some(r) => Decision::Update(r),
    }
}

/// 等这一轮结束再记进对话的一组消息。会存到盘上（桌面端 `agent/auto-cards-held.json`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Held {
    pub conversation: String,
    pub messages: Vec<Message>,
    /// 记进对话已经连着失败了几次（[`Desk::failed`]）。
    #[serde(default)]
    pub failures: u32,
    /// 原地更新（ADR 043 D5）：`messages` 里那条推送换掉对话里同一个标识的那条（找不到就追加），不是追加在末尾。
    /// 旧版本读到会当成追加（多出一张，不丢）。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub replace: bool,
}

/// 记住的推送和“排着”的消息。桌面端全进程一份。
#[derive(Debug, Default)]
pub struct Desk {
    records: VecDeque<(String, Record)>,
    held: Vec<Held>,
}

impl Desk {
    /// 内存里记着的这条推送（Kotone 重启以后是空的，要去对话里找）。
    pub fn record(&self, id: &str) -> Option<&Record> {
        self.records.iter().find(|(i, _)| i == id).map(|(_, r)| r)
    }

    /// 记下这条推送最后一次的内容和对话（最近的放最后；满了先忘最早的）。
    pub fn remember(&mut self, id: String, record: Record) {
        self.records.retain(|(i, _)| *i != id);
        if self.records.len() >= REMEMBER {
            self.records.pop_front();
        }
        self.records.push_back((id, record));
    }

    /// 忘掉这条（记进对话失败、那段对话没了）：下次同一个 key 当成新的。
    pub fn forget(&mut self, id: &str) {
        self.records.retain(|(i, _)| i != id);
    }

    /// 启动时接上盘上存着的（上次 Kotone 退出时还没记进对话的），里面的推送也记上。
    pub fn restore(&mut self, held: Vec<Held>) {
        for h in &held {
            for m in &h.messages {
                if let (Some(id), Some(record)) = (push_id(m), Record::of(&h.conversation, m)) {
                    self.remember(id, record);
                }
            }
        }
        let newer = std::mem::replace(&mut self.held, held);
        self.held.extend(newer);
    }

    /// 她不空：先排着，这一轮结束再记进对话。
    pub fn hold(&mut self, conversation: &str, messages: Vec<Message>) {
        self.held.push(Held {
            conversation: conversation.to_string(),
            messages,
            failures: 0,
            replace: false,
        });
    }

    /// 已经记进对话的那条推送要原地更新，但那段对话正在跑一轮：排着，这一轮结束再改。同一条排着的更新只留最新的。
    pub fn hold_replace(&mut self, conversation: &str, notice: Message) {
        if self.update_held(&notice) {
            return;
        }
        self.held.push(Held {
            conversation: conversation.to_string(),
            messages: vec![notice],
            failures: 0,
            replace: true,
        });
    }

    /// 这条推送还排着（没记进对话，或者排着一个更新）：直接换掉排着的那条，返回 true。
    pub fn update_held(&mut self, notice: &Message) -> bool {
        let Some(id) = push_id(notice) else {
            return false;
        };
        for h in &mut self.held {
            if let Some(m) = h
                .messages
                .iter_mut()
                .find(|m| push_id(m).as_deref() == Some(id.as_str()))
            {
                *m = notice.clone();
                return true;
            }
        }
        false
    }

    /// 取出来还没记成的放回去，排在取出以后新压进来的前面（保持来的先后）。
    pub fn put_back(&mut self, held: Vec<Held>) {
        let newer = std::mem::replace(&mut self.held, held);
        self.held.extend(newer);
    }

    /// 记进对话失败了一次（读写对话出错）：没到 [`MAX_FAILURES`] 次就返回它（放回去下次再试），到了就放弃，
    /// 返回 None（调用方记一次日志）。
    pub fn failed(mut held: Held) -> Option<Held> {
        held.failures += 1;
        (held.failures < MAX_FAILURES).then_some(held)
    }

    /// 现在排着的（存盘用）。
    pub fn held(&self) -> &[Held] {
        &self.held
    }

    pub fn has_held(&self) -> bool {
        !self.held.is_empty()
    }

    /// 某段对话里排着、还没记进对话的推送（还没记的新推送，和排着的更新）：给界面，重新读对话时照样显示——
    /// 对话里已经有同一个标识的就原地换成这条，没有就放在最后面。
    pub fn held_pushes(&self, conversation: &str) -> Vec<Message> {
        self.held
            .iter()
            .filter(|h| h.conversation == conversation)
            .flat_map(|h| h.messages.iter().filter(|m| push_id(m).is_some()).cloned())
            .collect()
    }

    /// 她空下来了：取走全部排着的（按来的先后）。
    pub fn take(&mut self) -> Vec<Held> {
        std::mem::take(&mut self.held)
    }
}

/// 把一组排着的更新放进对话的消息里：换掉同一个标识的最后一条推送，找不到就追加（那一条不在了，也别丢掉更新）。
pub fn apply_replace(messages: &mut Vec<Message>, notice: &Message) {
    let id = push_id(notice);
    match messages
        .iter_mut()
        .rev()
        .find(|m| id.is_some() && push_id(m) == id)
    {
        Some(m) => *m = notice.clone(),
        None => messages.push(notice.clone()),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use kotone_ai::agent::deepseek::DeepSeekModel;
    use kotone_ai::agent::{
        run_turn, AgentEvent, AgentLimits, Delta, FinishReason, ModelClient, ModelReply,
        ModelRequest, ToolDef, ToolOutcome, ToolPort, Turn,
    };
    use kotone_ai::transport::AiError;
    use kotone_core::cancel::CancellationToken;
    use kotone_core::connection::{ConnectionKind, ResolvedConnection};
    use kotone_plugin_api::{PushCard, PushCell, PushForm, PushLine, PushPriority, PushTone};

    use super::*;

    /// 数调用次数的假模型：每次回一句话。
    #[derive(Clone, Default)]
    struct Counting {
        requests: Arc<Mutex<Vec<ModelRequest>>>,
    }

    impl ModelClient for Counting {
        async fn complete(
            &self,
            request: &ModelRequest,
            _on_delta: &(dyn Fn(Delta<'_>) + Send + Sync),
            _cancel: &CancellationToken,
        ) -> Result<ModelReply, AiError> {
            self.requests.lock().unwrap().push(request.clone());
            Ok(ModelReply {
                text: "好".into(),
                reasoning: String::new(),
                tool_calls: Vec::new(),
                finish: FinishReason::Stop,
                usage: None,
            })
        }
    }

    struct NoTools;

    impl ToolPort for NoTools {
        fn definitions(&self) -> Vec<ToolDef> {
            Vec::new()
        }
        async fn call(
            &self,
            _call_id: &str,
            _name: &str,
            _arguments: Value,
            _cancel: &CancellationToken,
        ) -> ToolOutcome {
            ToolOutcome::error("没有工具")
        }
    }

    fn spec(id: &str, forms: &[PushForm]) -> PushSpec {
        PushSpec {
            id: id.into(),
            title: if id == "opening" {
                "开局战绩"
            } else {
                "海克斯三选一"
            }
            .into(),
            when: "进加载".into(),
            what: "卡片".into(),
            forms: forms.to_vec(),
            priority: PushPriority::Normal,
            enabled_by_default: true,
            until_done: false,
            needs: Vec::new(),
        }
    }

    fn opening() -> Push {
        Push {
            trigger: "opening".into(),
            key: "opening:42".into(),
            card: Some(PushCard {
                title: "开局战绩".into(),
                uri: "ui://kotone/lol/match".into(),
                params: Value::Null,
                data: json!({"players": [{"name": "晚安玛卡巴卡", "games": 20, "wins": 6}]}),
            }),
            note: "开局战绩（海斗）。对面寒冰近 20 局 6 胜 14 负、场均死 13 次。".into(),
            ..Push::default()
        }
    }

    /// 新推一条：推送那条 + 给她的那句（有 note 才有）。
    fn posted(push: &Push, at_ms: u64) -> Vec<Message> {
        let record = record(
            "lol",
            "英雄联盟",
            &spec("opening", &[PushForm::Card]),
            push,
            "",
        );
        let mut out = vec![notice("英雄联盟", record, at_ms)];
        if !push.note.is_empty() {
            out.push(note_message(
                "英雄联盟",
                "开局战绩",
                &push.note,
                false,
                at_ms,
            ));
        }
        out
    }

    async fn player_turn(model: &Counting, history: &mut Vec<Message>, text: &str) {
        let limits = AgentLimits::default();
        let cancel = CancellationToken::default();
        let events: Mutex<Vec<AgentEvent>> = Mutex::default();
        run_turn(
            model,
            &NoTools,
            history,
            Turn {
                system: "你是琴音",
                user: Message::user(text),
                limits: &limits,
                cancel: &cancel,
                inbox: None,
            },
            &|e| events.lock().unwrap().push(e),
        )
        .await;
    }

    fn deepseek_messages(history: &[Message]) -> Vec<String> {
        let model = DeepSeekModel::new(ResolvedConnection {
            id: "c".into(),
            display_name: "DeepSeek".into(),
            kind: ConnectionKind::Remote,
            provider: "deepseek".into(),
            base_url: "https://api.deepseek.com".into(),
            model: "deepseek-chat".into(),
            api_key: None,
        });
        let request = ModelRequest {
            system: "你是琴音".into(),
            messages: history
                .iter()
                .filter(|m| !matches!(m, Message::Notice { .. }))
                .cloned()
                .collect(),
            tools: Vec::new(),
        };
        let (body, _) = model.body(&request);
        body["messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| serde_json::to_string(m).unwrap())
            .collect()
    }

    /// 推送本身一次模型都不调；下一轮（玩家开口）时那句话在模型看到的历史里，卡片的数据不在。
    #[tokio::test]
    async fn posting_calls_no_model_and_the_next_turn_sees_the_note() {
        let model = Counting::default();
        let mut history = Vec::new();
        player_turn(&model, &mut history, "在吗").await;
        assert_eq!(model.requests.lock().unwrap().len(), 1);

        history.extend(posted(&opening(), 1_000));
        assert_eq!(
            model.requests.lock().unwrap().len(),
            1,
            "推送不调用模型（不花钱）"
        );

        player_turn(&model, &mut history, "对面谁最菜").await;
        let requests = model.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        let seen = &requests[1].messages;
        // 模型看到：之前的一问一答、推送的那句（按数据附上）、玩家这句；看不到推送本身（Notice）和页面数据
        assert!(seen.iter().all(|m| !matches!(m, Message::Notice { .. })));
        let note = seen
            .iter()
            .find(|m| matches!(m, Message::User { source, .. } if source == SOURCE))
            .expect("推送的那句话在历史里");
        let Message::User {
            text, attachments, ..
        } = note
        else {
            unreachable!()
        };
        assert!(
            text.contains("英雄联盟") && text.contains("开局战绩") && text.contains("系统消息")
        );
        assert!(matches!(&attachments[0], Attachment::Data { text, .. } if text.contains("寒冰")));
        let wire = serde_json::to_string(seen).unwrap();
        assert!(!wire.contains("晚安玛卡巴卡"), "页面数据不发给模型");
    }

    /// 那句话追加在末尾：之前发给模型的每一条一个字节都不变（前缀缓存照常命中，ADR 024）。
    #[tokio::test]
    async fn the_note_is_appended_and_the_prefix_bytes_stay() {
        let model = Counting::default();
        let mut history = Vec::new();
        player_turn(&model, &mut history, "查一下这把").await;
        let before = deepseek_messages(&history);

        history.extend(posted(&opening(), 1_000));
        player_turn(&model, &mut history, "对面谁最菜").await;
        let after = deepseek_messages(&history);

        assert!(after.len() > before.len());
        assert_eq!(&after[..before.len()], &before[..], "前缀一个字节都不变");
        let rest = after[before.len()..].join("\n");
        let note = rest.find("开局战绩").expect("推送的那句话");
        let player = rest.find("对面谁最菜").expect("玩家这句");
        assert!(note < player, "推送的那句话在玩家这句前面：{rest}");
        // 系统提示（第一条）也没变
        assert!(after[0].contains("你是琴音"));
    }

    /// ADR 043 D5：原地更新卡片（改 Notice 的 push）以后，发给模型的请求体前面的每一条逐字节不变；更新带的那句话
    /// 追加在末尾。没带 note 的更新，模型那边一个字节都不多。
    #[tokio::test]
    async fn an_in_place_update_keeps_the_model_prefix_byte_identical() {
        let model = Counting::default();
        let mut history = Vec::new();
        player_turn(&model, &mut history, "查一下这把").await;
        history.extend(posted(&opening(), 1_000));
        player_turn(&model, &mut history, "对面谁最菜").await;
        let before = deepseek_messages(&history);
        let position = history
            .iter()
            .position(|m| push_id(m).as_deref() == Some("lol:opening:42"))
            .unwrap();
        let length = history.len();

        // 同一个 key、内容变了，没带 note：原地改
        let mut changed = opening();
        changed.card.as_mut().unwrap().data =
            json!({"players": [{"name": "晚安玛卡巴卡", "games": 20, "wins": 7}]});
        changed.note = String::new();
        let record = record(
            "lol",
            "英雄联盟",
            &spec("opening", &[PushForm::Card]),
            &changed,
            "",
        );
        let updated = notice("英雄联盟", record, 1_000);
        apply_replace(&mut history, &updated);
        assert_eq!(history.len(), length, "原地改，不多一条");
        assert_eq!(history[position], updated, "位置不动");
        assert_eq!(
            deepseek_messages(&history),
            before,
            "没带 note 的更新：发给模型的一个字节都不变"
        );

        // 再更新一次，带 note：追加一条给模型的，前面的不变
        changed.note = "选人结束：对面换了打野。".into();
        history.push(note_message(
            "英雄联盟",
            "开局战绩",
            &changed.note,
            true,
            2_000,
        ));
        player_turn(&model, &mut history, "那现在呢").await;
        let after = deepseek_messages(&history);
        assert_eq!(&after[..before.len()], &before[..], "前缀一个字节都不变");
        let rest = after[before.len()..].join("\n");
        assert!(
            rest.contains("更新了推送") && rest.contains("对面换了打野"),
            "{rest}"
        );
        assert!(
            !serde_json::to_string(&model.requests.lock().unwrap().last().unwrap().messages)
                .unwrap()
                .contains("\"wins\":7")
        );
    }

    /// 她不空时排着：对话里先没有；这一轮结束后取出来，接在她这一轮的回答后面。
    #[tokio::test]
    async fn while_busy_the_push_waits_until_the_turn_ends() {
        let model = Counting::default();
        let mut history = Vec::new();
        let mut desk = Desk::default();
        desk.hold("c-1", posted(&opening(), 1_000));
        assert!(desk.has_held());
        assert_eq!(desk.held_pushes("c-1").len(), 1, "推送照样给界面显示");
        assert!(desk.held_pushes("c-2").is_empty());
        player_turn(&model, &mut history, "在吗").await;
        assert!(
            history.iter().all(|m| push_of(m).is_none()),
            "这一轮里没有插进推送"
        );
        let held = desk.take();
        assert_eq!(held.len(), 1);
        assert!(!desk.has_held());
        let answered = history.len();
        history.extend(held[0].messages.clone());
        assert!(matches!(history[answered - 1], Message::Assistant { .. }));
        assert!(push_of(&history[answered]).is_some());
        assert!(matches!(&history[answered + 1], Message::User { source, .. } if source == SOURCE));
        assert_eq!(
            model.requests.lock().unwrap().len(),
            1,
            "没有为推送多叫一次模型"
        );
    }

    /// 同一个 key：内容一样什么都不做，变了更新上一次那条；不同插件、不同 key 是新的；旧的自动卡片当作发过。
    #[test]
    fn the_same_key_again_is_an_update_or_nothing() {
        let first = record(
            "lol",
            "英雄联盟",
            &spec("opening", &[PushForm::Card]),
            &opening(),
            "",
        );
        let hash = content_hash(&first);
        assert_eq!(decide(None, hash), Decision::Post);
        let mut desk = Desk::default();
        let rec = Record {
            conversation: "c-1".into(),
            at_ms: 1_000,
            hash,
            voice: Vec::new(),
            said: String::new(),
            legacy: false,
        };
        desk.remember(id("lol", "opening:42"), rec.clone());
        assert_eq!(
            desk.record(&id("lol", " opening:42 ")),
            Some(&rec),
            "key 前后的空白不算"
        );
        assert!(desk.record(&id("other", "opening:42")).is_none());
        assert_eq!(
            decide(desk.record("lol:opening:42").cloned(), hash),
            Decision::Unchanged
        );
        // 插件名、这次说的哪句、更新时间不算内容
        let mut renamed = record(
            "lol",
            "LoL",
            &spec("opening", &[PushForm::Card]),
            &opening(),
            "x",
        );
        renamed["updatedAtMs"] = json!(5);
        assert_eq!(content_hash(&renamed), hash);
        // 页面数据变了：更新
        let mut changed = opening();
        changed.card.as_mut().unwrap().data = json!({"players": []});
        let changed = record(
            "lol",
            "英雄联盟",
            &spec("opening", &[PushForm::Card]),
            &changed,
            "",
        );
        assert_eq!(
            decide(
                desk.record("lol:opening:42").cloned(),
                content_hash(&changed)
            ),
            Decision::Update(rec)
        );
        // 旧的自动卡片：当作发过，不动它
        let legacy = Record {
            legacy: true,
            ..Record::of("c-1", &posted(&opening(), 1)[0]).unwrap()
        };
        assert_eq!(decide(Some(legacy), 123), Decision::Unchanged);
        // 记得住的有上限，最早的先忘
        for i in 0..REMEMBER {
            desk.remember(
                format!("lol:k{i}"),
                Record::of("c", &posted(&opening(), 1)[0]).unwrap(),
            );
        }
        assert_eq!(desk.records.len(), REMEMBER);
        assert!(desk.record("lol:opening:42").is_none());
        desk.forget("lol:k5");
        assert!(desk.record("lol:k5").is_none());
    }

    /// Kotone 重启（内存里的记录空了）：去对话里找——新的推送、旧的自动卡片都认得；下一局、别的插件不算。
    #[test]
    fn after_a_restart_the_conversation_tells_what_was_pushed() {
        let history = posted(&opening(), 1_000);
        let found = find(&history, "lol:opening:42").expect("对话里有这一条");
        let rec = Record::of("c-1", found).unwrap();
        assert_eq!(rec.at_ms, 1_000);
        assert!(!rec.legacy);
        assert_eq!(rec.hash, content_hash(push_of(found).unwrap()));
        assert!(find(&history, "lol:opening:43").is_none(), "下一局照发");
        assert!(find(&history, "other:opening:42").is_none());
        assert!(find(&[Message::user("opening:42")], "lol:opening:42").is_none());
        // rc.9 写的自动卡片（Notice 的 card）
        let legacy = Message::Notice {
            text: "英雄联盟 发来一张卡片：开局战绩".into(),
            at_ms: 7,
            compaction: None,
            usage: None,
            recap: None,
            card: Some(
                json!({"plugin": "lol", "pluginName": "英雄联盟", "key": "opening:42", "title": "开局战绩",
                "ui": {"uri": "ui://kotone/lol/match"}, "structuredContent": {}}),
            ),
            push: None,
        };
        let old: Message = serde_json::from_value(serde_json::to_value(&legacy).unwrap()).unwrap();
        assert_eq!(push_id(&old).as_deref(), Some("lol:opening:42"));
        assert!(Record::of("c-1", &old).unwrap().legacy);
        // rc.9 的存盘格式（没有 push 字段）读得进来
        let rc9: Message = serde_json::from_str(
            r#"{"role":"notice","text":"英雄联盟 发来一张卡片：开局战绩","atMs":7,"card":{"plugin":"lol","key":"opening:42"}}"#,
        )
        .unwrap();
        assert_eq!(push_id(&rc9).as_deref(), Some("lol:opening:42"));
    }

    /// 还排着的推送更新了：直接换掉排着的那条（最新的为准）；已经记进对话、那段对话在跑：排一个“原地更新”，
    /// 同一条只留最新的；界面重新读对话时这些都拿得到。
    #[test]
    fn updates_replace_what_is_held() {
        let mut desk = Desk::default();
        desk.hold("c-1", posted(&opening(), 1_000));
        let mut changed = opening();
        changed.card.as_mut().unwrap().data = json!({"players": []});
        let rec = record(
            "lol",
            "英雄联盟",
            &spec("opening", &[PushForm::Card]),
            &changed,
            "",
        );
        let updated = notice("英雄联盟", rec, 1_000);
        assert!(desk.update_held(&updated), "排着的那条直接换掉");
        assert_eq!(desk.held().len(), 1);
        assert_eq!(desk.held()[0].messages[0], updated);
        assert!(!desk.held()[0].replace);
        assert_eq!(desk.held()[0].messages.len(), 2, "给她的那句还在");

        // 另一条已经记进对话，那段对话在跑：排一个原地更新
        let mut summary = opening();
        summary.key = "summary:42".into();
        let s1 = notice(
            "英雄联盟",
            record(
                "lol",
                "英雄联盟",
                &spec("opening", &[PushForm::Card]),
                &summary,
                "",
            ),
            2_000,
        );
        assert!(!desk.update_held(&s1), "没排着");
        desk.hold_replace("c-1", s1);
        summary.card.as_mut().unwrap().title = "这局总结".into();
        let s2 = notice(
            "英雄联盟",
            record(
                "lol",
                "英雄联盟",
                &spec("opening", &[PushForm::Card]),
                &summary,
                "",
            ),
            2_000,
        );
        desk.hold_replace("c-1", s2.clone());
        assert_eq!(desk.held().len(), 2, "同一条排着的更新只留一个");
        assert!(desk.held()[1].replace);
        assert_eq!(
            desk.held()[1].messages,
            std::slice::from_ref(&s2),
            "最新的为准"
        );
        assert_eq!(desk.held_pushes("c-1"), [updated, s2.clone()]);

        // 这一轮结束：原地换掉对话里那条，位置不动；找不到就追加
        let mut conversation = vec![
            Message::user("在吗"),
            posted(&summary, 2_000)[0].clone(),
            Message::user("后来的"),
        ];
        apply_replace(&mut conversation, &s2);
        assert_eq!(conversation.len(), 3);
        assert_eq!(conversation[1], s2);
        let mut empty = vec![Message::user("在吗")];
        apply_replace(&mut empty, &s2);
        assert_eq!(empty.len(), 2);

        // 存盘格式：replace 只在是的时候写；旧格式（没有 replace、failures）读得进来
        let saved = serde_json::to_value(desk.held()).unwrap();
        assert!(saved[0].get("replace").is_none());
        assert_eq!(saved[1]["replace"], true);
        let old: Vec<Held> =
            serde_json::from_str(r#"[{"conversation":"c-1","messages":[]}]"#).unwrap();
        assert!(!old[0].replace && old[0].failures == 0);
    }

    /// 排着的存盘读回来（下次启动接着记），里面的推送记上；新压进来的排在后面。记进对话连着失败几次就放弃。
    #[test]
    fn held_pushes_survive_a_restart_and_give_up_after_repeated_failures() {
        let mut desk = Desk::default();
        desk.hold("c-1", posted(&opening(), 1_000));
        let saved = serde_json::to_string(desk.held()).unwrap();

        let mut restarted = Desk::default();
        let mut summary = opening();
        summary.key = "summary:42".into();
        restarted.hold("c-1", posted(&summary, 2_000));
        restarted.restore(serde_json::from_str(&saved).unwrap());
        let rec = restarted
            .record("lol:opening:42")
            .expect("排着的推送记上了");
        assert_eq!(rec.conversation, "c-1");
        let held = restarted.take();
        let keys: Vec<&str> = held
            .iter()
            .map(|h| push_of(&h.messages[0]).unwrap()["key"].as_str().unwrap())
            .collect();
        assert_eq!(keys, ["opening:42", "summary:42"], "按来的先后");

        // 取出来、第一组没记成：放回去，排在取出以后新来的前面
        let mut desk = Desk::default();
        desk.put_back(held);
        desk.hold("c-2", Vec::new());
        assert_eq!(desk.held().len(), 3);
        assert_eq!(desk.held()[2].conversation, "c-2");
        // 连着失败：第 MAX_FAILURES 次放弃
        let mut one = desk.take().remove(0);
        for _ in 1..MAX_FAILURES {
            one = Desk::failed(one).expect("还没到次数");
        }
        assert_eq!(one.failures, MAX_FAILURES - 1);
        assert!(Desk::failed(one).is_none());
    }

    #[test]
    fn records_carry_what_the_ui_and_the_floating_window_need() {
        let offer = Push {
            trigger: "augment-pick".into(),
            key: "pick:1".into(),
            line: Some(PushLine {
                cells: vec![
                    PushCell {
                        text: "首选".into(),
                        tone: Some(PushTone::Good),
                    },
                    PushCell {
                        text: "可刷新".into(),
                        tone: Some(PushTone::Bad),
                    },
                ],
            }),
            voice: vec!["左边那个".into(), "选左边".into()],
            ..Push::default()
        };
        let pick_spec = spec("augment-pick", &[PushForm::Line, PushForm::Voice]);
        let rec = record("lol", "英雄联盟", &pick_spec, &offer, pick(&offer.voice, 1));
        assert_eq!(rec["trigger"], "augment-pick");
        assert_eq!(rec["title"], "海克斯三选一");
        assert_eq!(rec["line"]["cells"][0]["tone"], "good");
        assert_eq!(rec["voice"][1], "选左边");
        assert_eq!(rec["said"], "选左边");
        assert!(rec.get("card").is_none());
        assert_eq!(shown_title(&rec), "海克斯三选一");
        let m = notice("英雄联盟", rec, 5);
        let Message::Notice { text, .. } = &m else {
            unreachable!()
        };
        assert_eq!(text, "英雄联盟 推送：海克斯三选一");
        assert_eq!(push_id(&m).as_deref(), Some("lol:pick:1"));
        // 轮着用
        assert_eq!(pick(&offer.voice, 0), "左边那个");
        assert_eq!(pick(&offer.voice, 3), "选左边");
        assert_eq!(pick(&[], 3), "");

        // 卡片：和工具结果的 display 同一个形状；给她的那句截到 160 字
        let card = Push {
            card: Some(PushCard {
                params: json!({"gameId": 42}),
                ..opening().card.unwrap()
            }),
            ..opening()
        };
        let rec = record(
            "lol",
            "英雄联盟",
            &spec("opening", &[PushForm::Card]),
            &card,
            "",
        );
        assert_eq!(rec["card"]["ui"]["uri"], "ui://kotone/lol/match");
        assert_eq!(rec["card"]["ui"]["params"]["gameId"], 42);
        assert_eq!(rec["card"]["structuredContent"]["players"][0]["wins"], 6);
        assert!(rec.get("line").is_none() && rec.get("voice").is_none());
        let Message::User { attachments, .. } =
            note_message("英雄联盟", "开局战绩", &"长".repeat(400), false, 5)
        else {
            unreachable!()
        };
        let Attachment::Data { text, .. } = &attachments[0] else {
            unreachable!()
        };
        assert_eq!(text.chars().count(), Push::NOTE_MAX);
        // 存盘格式：旧版本读得懂（Notice 多一个字段，User 是普通的用户消息）
        let m = notice("英雄联盟", rec, 5);
        let json = serde_json::to_value(&m).unwrap();
        assert_eq!(json["role"], "notice");
        assert!(json["push"].is_object() && json.get("card").is_none());
        let back: Message = serde_json::from_value(json).unwrap();
        assert_eq!(back, m);
    }
}
