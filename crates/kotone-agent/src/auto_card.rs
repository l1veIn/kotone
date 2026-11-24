//! 自动卡片（ADR 039 D3）的纯逻辑：插件看到游戏里的事，自己往聊天里发一张卡片，**不叫模型、不花钱**。
//!
//! 一张自动卡片在对话里是两条消息，都追加在末尾，前面的消息一个字不改（ADR 024“历史只追加”，前缀缓存照常命中）：
//!
//! 1. 卡片：[`Message::Notice`] 带 `card`（[`card_message`]）。`Notice` 本来就不发给模型；界面把它画成插件卡片，
//!    上面一行写“{插件名} 发来”。`card` 里的 `ui` / `structuredContent` 和工具结果的 `display` 同一个形状，
//!    聊天卡片用同一个组件。
//! 2. 给模型的那一两句：来源 [`SOURCE`] 的用户消息（[`note_message`]），说法照自动化跑完的消息（“系统消息，
//!    不是玩家说的话”）；插件给的那句里有别人起的名字，按数据附上（模型按不可信数据看，原则 9）。界面不显示它。
//!
//! 时机（[`Desk`]）：琴音空着（她没在回答、没在说话，玩家没在说话；桌面端照 ADR 029“做完主动说”的空闲判断）
//! 时两条马上记进对话；不空时先“压着”：卡片照样马上显示在聊天里（界面事件），两条都等这一轮结束再记进对话，
//! 不插进正在跑的那一轮（不像自动化跑完的消息那样插话），也不叫她开口。去重：同一个插件同一个 `key` 只发一次。
//!
//! 桌面端（`apps/desktop/src-tauri/src/agent/auto_card.rs`）管存盘、界面事件、消息提示。

use std::collections::VecDeque;

use kotone_ai::agent::{Attachment, Message};
use kotone_core::agent::AutoCardsConfig;
use kotone_plugin_api::AutoCard;
use serde_json::{json, Value};

/// 给模型的那条用户消息的来源（界面不显示它；对话标题也不拿它）。
pub const SOURCE: &str = "auto-card";
/// 记住最近多少张卡片的去重键（一局最多两张，够用很久）。
const REMEMBER: usize = 256;

/// 截到 `max` 个字，多了在末尾加“…”。
pub fn clip(text: &str, max: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// 消息提示的那一句：插件给的，没给时“{插件名}发来一张卡片”。
pub fn popup_text(plugin_name: &str, card: &AutoCard) -> String {
    let text = card.popup.trim();
    if text.is_empty() {
        format!("{plugin_name}发来一张卡片")
    } else {
        clip(text, AutoCard::POPUP_MAX)
    }
}

/// 卡片那条（只给界面）。`card` 的形状（界面 `agent.ts` 的 `AutoCardInfo`）：
/// `{plugin, pluginName, key, title, popup, ui: {uri, params?}, structuredContent}`。
pub fn card_message(plugin: &str, plugin_name: &str, card: &AutoCard, at_ms: u64) -> Message {
    let mut ui = json!({"uri": card.uri});
    if !card.params.is_null() {
        ui["params"] = card.params.clone();
    }
    Message::Notice {
        // 旧版本只认得 text：看到的是这一行
        text: format!("{plugin_name} 发来一张卡片：{}", card.title.trim()),
        at_ms,
        compaction: None,
        usage: None,
        recap: None,
        card: Some(json!({
            "plugin": plugin,
            "pluginName": plugin_name,
            "key": card.key,
            "title": card.title.trim(),
            "popup": popup_text(plugin_name, card),
            "ui": ui,
            "structuredContent": card.data,
        })),
    }
}

/// 给模型的那条（紧跟在卡片后面）。
pub fn note_message(plugin_name: &str, card: &AutoCard, at_ms: u64) -> Message {
    let title = card.title.trim();
    Message::User {
        text: format!(
            "（系统消息，不是玩家说的话：{plugin_name}插件发来一张自动卡片「{title}」，玩家在聊天里看得到；要点见附件。）"
        ),
        attachments: vec![Attachment::Data {
            label: format!("{plugin_name} · {title}"),
            text: clip(&card.note, AutoCard::NOTE_MAX),
        }],
        source: SOURCE.into(),
        at_ms,
        context: String::new(),
    }
}

/// 卡片消息里的 `card`（不是自动卡片时 None）。
pub fn card_of(message: &Message) -> Option<&Value> {
    match message {
        Message::Notice { card, .. } => card.as_ref(),
        _ => None,
    }
}

/// 来一张卡片时怎么办。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// 玩家关了自动卡片：不发、不弹。
    Off,
    /// 同一个插件同一个键发过了。
    Duplicate,
    /// 发；`popup` = 要不要弹消息提示（还要看游戏是不是独占全屏、Kotone 窗口是不是在前台，桌面端判断）。
    Post { popup: bool },
}

/// 等这一轮结束再记进对话的卡片（一张卡片的两条消息）。
#[derive(Debug, Clone, PartialEq)]
pub struct Held {
    pub conversation: String,
    pub messages: Vec<Message>,
}

/// 去重和“压着”的卡片。桌面端全进程一份。
#[derive(Debug, Default)]
pub struct Desk {
    seen: VecDeque<String>,
    held: Vec<Held>,
}

impl Desk {
    /// 发不发：开关、去重。决定发时就记下这个键（之后同一个键都是 [`Decision::Duplicate`]）。
    pub fn decide(&mut self, config: &AutoCardsConfig, plugin: &str, key: &str) -> Decision {
        if !config.enabled {
            return Decision::Off;
        }
        let id = format!("{plugin}:{}", key.trim());
        if self.seen.contains(&id) {
            return Decision::Duplicate;
        }
        if self.seen.len() >= REMEMBER {
            self.seen.pop_front();
        }
        self.seen.push_back(id);
        Decision::Post {
            popup: config.popup,
        }
    }

    /// 琴音不空：先压着，这一轮结束再记进对话。
    pub fn hold(&mut self, conversation: &str, messages: Vec<Message>) {
        self.held.push(Held {
            conversation: conversation.to_string(),
            messages,
        });
    }

    pub fn has_held(&self) -> bool {
        !self.held.is_empty()
    }

    /// 某段对话里压着、还没记进对话的卡片（给界面：重新读对话时照样显示出来）。
    pub fn held_cards(&self, conversation: &str) -> Vec<Message> {
        self.held
            .iter()
            .filter(|h| h.conversation == conversation)
            .flat_map(|h| h.messages.iter().filter(|m| card_of(m).is_some()).cloned())
            .collect()
    }

    /// 琴音空下来了：取走全部压着的（按来的先后）。
    pub fn take(&mut self) -> Vec<Held> {
        std::mem::take(&mut self.held)
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

    fn opening() -> AutoCard {
        AutoCard {
            key: "opening:42".into(),
            title: "开局战绩".into(),
            uri: "ui://kotone/lol/match".into(),
            params: Value::Null,
            data: json!({"players": [{"name": "晚安玛卡巴卡", "games": 20, "wins": 6}]}),
            note: "开局战绩（海斗）。对面可能的突破口：寒冰，近 20 局 6 胜 14 负、场均死 13 次。"
                .into(),
            popup: "对面的战绩到了".into(),
        }
    }

    fn post(desk: &mut Desk, config: &AutoCardsConfig, card: &AutoCard) -> Option<Vec<Message>> {
        match desk.decide(config, "lol", &card.key) {
            Decision::Post { .. } => Some(vec![
                card_message("lol", "英雄联盟", card, 1_000),
                note_message("英雄联盟", card, 1_000),
            ]),
            _ => None,
        }
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

    /// 发卡片本身一次模型都不调；下一轮（玩家开口）时那句话在模型看到的历史里，卡片的数据不在。
    #[tokio::test]
    async fn posting_a_card_calls_no_model_and_the_next_turn_sees_the_note() {
        let model = Counting::default();
        let mut history = Vec::new();
        player_turn(&model, &mut history, "在吗").await;
        assert_eq!(model.requests.lock().unwrap().len(), 1);

        let mut desk = Desk::default();
        let messages = post(&mut desk, &AutoCardsConfig::default(), &opening()).unwrap();
        history.extend(messages);
        assert_eq!(
            model.requests.lock().unwrap().len(),
            1,
            "发卡片不调用模型（不花钱）"
        );

        player_turn(&model, &mut history, "对面谁最菜").await;
        let requests = model.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        let seen = &requests[1].messages;
        // 模型看到：之前的一问一答、卡片的那句（按数据附上）、玩家这句；看不到卡片本身（Notice）和页面数据
        assert!(seen.iter().all(|m| !matches!(m, Message::Notice { .. })));
        let note = seen
            .iter()
            .find(|m| matches!(m, Message::User { source, .. } if source == SOURCE))
            .expect("卡片的那句话在历史里");
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

        let mut desk = Desk::default();
        history.extend(post(&mut desk, &AutoCardsConfig::default(), &opening()).unwrap());
        player_turn(&model, &mut history, "对面谁最菜").await;
        let after = deepseek_messages(&history);

        assert!(after.len() > before.len());
        assert_eq!(&after[..before.len()], &before[..], "前缀一个字节都不变");
        let rest = after[before.len()..].join("\n");
        let note = rest.find("开局战绩").expect("卡片的那句话");
        let player = rest.find("对面谁最菜").expect("玩家这句");
        assert!(note < player, "卡片的那句话在玩家这句前面：{rest}");
        // 系统提示（第一条）也没变
        assert!(after[0].contains("你是琴音"));
    }

    /// 琴音不空时压着：对话里先没有；这一轮结束后取出来，接在她这一轮的回答后面。
    #[tokio::test]
    async fn while_busy_the_card_waits_until_the_turn_ends() {
        let model = Counting::default();
        let mut history = Vec::new();
        let mut desk = Desk::default();
        // 她正在回答（这一轮开始前取的历史）时卡片来了
        let messages = post(&mut desk, &AutoCardsConfig::default(), &opening()).unwrap();
        desk.hold("c-1", messages);
        assert!(desk.has_held());
        assert_eq!(desk.held_cards("c-1").len(), 1, "卡片照样给界面显示");
        assert!(desk.held_cards("c-2").is_empty());
        player_turn(&model, &mut history, "在吗").await;
        assert!(
            history.iter().all(|m| card_of(m).is_none()),
            "这一轮里没有插进卡片"
        );
        // 这一轮结束、空下来：取出来接在末尾
        let held = desk.take();
        assert_eq!(held.len(), 1);
        assert!(!desk.has_held());
        let answered = history.len();
        history.extend(held[0].messages.clone());
        assert!(matches!(history[answered - 1], Message::Assistant { .. }));
        assert!(card_of(&history[answered]).is_some());
        assert!(matches!(&history[answered + 1], Message::User { source, .. } if source == SOURCE));
        assert_eq!(
            model.requests.lock().unwrap().len(),
            1,
            "没有为卡片多叫一次模型"
        );
    }

    #[test]
    fn the_same_key_is_posted_once_per_plugin() {
        let mut desk = Desk::default();
        let on = AutoCardsConfig::default();
        assert_eq!(
            desk.decide(&on, "lol", "opening:42"),
            Decision::Post { popup: true }
        );
        assert_eq!(desk.decide(&on, "lol", "opening:42"), Decision::Duplicate);
        assert_eq!(desk.decide(&on, "lol", " opening:42 "), Decision::Duplicate);
        assert_eq!(
            desk.decide(&on, "lol", "summary:42"),
            Decision::Post { popup: true }
        );
        assert_eq!(
            desk.decide(&on, "other", "opening:42"),
            Decision::Post { popup: true }
        );
        // 记得住的有上限，最早的先忘
        for i in 0..REMEMBER {
            desk.decide(&on, "lol", &format!("k{i}"));
        }
        assert_eq!(desk.seen.len(), REMEMBER);
    }

    #[test]
    fn switches_off_mean_no_card_and_no_popup() {
        let mut desk = Desk::default();
        let off = AutoCardsConfig {
            enabled: false,
            popup: true,
        };
        assert_eq!(desk.decide(&off, "lol", "opening:1"), Decision::Off);
        // 关着时不算发过：打开以后同一个键照样能发
        let quiet = AutoCardsConfig {
            enabled: true,
            popup: false,
        };
        assert_eq!(
            desk.decide(&quiet, "lol", "opening:1"),
            Decision::Post { popup: false }
        );
        // 旧配置（没有 autoCards）按默认：都开
        let old: kotone_core::agent::AgentConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(
            desk.decide(&old.auto_cards, "lol", "opening:2"),
            Decision::Post { popup: true }
        );
    }

    #[test]
    fn messages_have_the_card_for_the_ui_and_a_short_note_for_her() {
        let card = AutoCard {
            note: "长".repeat(400),
            popup: "这一句特别特别特别特别特别特别特别特别长的消息提示".into(),
            params: json!({"gameId": 42}),
            ..opening()
        };
        let m = card_message("lol", "英雄联盟", &card, 5);
        let info = card_of(&m).unwrap();
        assert_eq!(info["plugin"], "lol");
        assert_eq!(info["pluginName"], "英雄联盟");
        assert_eq!(info["ui"]["uri"], "ui://kotone/lol/match");
        assert_eq!(info["ui"]["params"]["gameId"], 42);
        assert_eq!(info["structuredContent"]["players"][0]["wins"], 6);
        assert!(info["popup"].as_str().unwrap().chars().count() <= AutoCard::POPUP_MAX);
        let Message::Notice { text, .. } = &m else {
            unreachable!()
        };
        assert_eq!(text, "英雄联盟 发来一张卡片：开局战绩");
        let Message::User { attachments, .. } = note_message("英雄联盟", &card, 5) else {
            unreachable!()
        };
        let Attachment::Data { text, .. } = &attachments[0] else {
            unreachable!()
        };
        assert_eq!(text.chars().count(), AutoCard::NOTE_MAX);
        assert_eq!(
            popup_text(
                "英雄联盟",
                &AutoCard {
                    popup: String::new(),
                    ..opening()
                }
            ),
            "英雄联盟发来一张卡片"
        );
        // 存盘格式：旧版本读得懂（Notice 多一个字段，User 是普通的用户消息）
        let json = serde_json::to_value(&m).unwrap();
        assert_eq!(json["role"], "notice");
        assert!(json["card"].is_object());
        let back: Message = serde_json::from_value(json).unwrap();
        assert_eq!(back, m);
    }
}
