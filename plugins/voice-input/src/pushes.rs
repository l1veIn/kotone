//! 语音输入的推送和事件（ADR 043 D10，讨论记录第九节“语音输入按新形态”）。
//!
//! - 推送 `sent`（发出去的话，卡片）：发出去一句，聊天里一张小卡片“你向「队伍」发了：……”，写处理过的话，和原话不一样时
//!   也写原话。同一个游戏、同一个频道、上一句发出后 2 分钟内的几句并进同一张（同一个 key 再推一次 = 原地更新，D5）。
//!   给她的那句附上发出去的话：她就知道你刚才在游戏里说了什么（下次叫她时这句话随对话发给模型服务商；关掉这条推送就不给）。
//! - 推送 `failed`（没发出去，卡片）：写明原因，带一个“重发”按钮（[`TOOL_RESEND`]，只给页面）。重发是玩家点的：经本体
//!   “发到游戏”的入口（ADR 041）回到原来那个游戏窗口、用原来的频道发，切不回去就不发，不往 Kotone 自己的窗口里打字；
//!   不是替玩家补按键（ADR 032）。重发的结果原地更新这张卡片（已重发，或者新的原因）。
//! - 事件 `<插件 id>.sent`（频道、游戏、文字）和 `<插件 id>.failed`（原因），清单 `[[events]]` 登记，给自动化和她用。经游戏中心
//!   报（[`Host::game_event`]）：游戏中心里有这个游戏（在跑）时才报得出去。
//!
//! 推送和事件都经宿主（[`Host::push`]、[`Host::game_event`]，开关由宿主执行）。宿主在插件启动时交给
//! [`crate::VoiceInputPlugin`]（`Plugin::start`），存在这里；命令行、本体的测试没有宿主，就不推。推送要写对话（磁盘），
//! 不在发送的路上等：发完交给后台线程，一次一条、按先后（[`PushDesk::report`]）。
//!
//! 卡片里的数据（页面 `ui://kotone/voice-input/sent`、`…/failed` 照它画，`plugins/voice-input/ui`）：
//!
//! ```text
//! sent:   { game: {id, name}, channel: {id, name}, lines: [{ text, source?, atMs }] }      // source = 原话（和 text 不同时才有）
//! failed: { id, text, source?, reason, code, atMs, attempts, channel?: {game, id},       // channel = 当时选的频道
//!           resent?: { game: {id, name}, channel: {id, name}, atMs } }                   // 重发成功了
//! ```

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Weak};

use kotone_core::history::HistoryPlace;
use kotone_core::inject::TargetWindow;
use kotone_core::orchestrator::Orchestrator;
use kotone_core::send_to_game::ChatOutcome;
use kotone_plugin_api::{Host, Push, PushCard};
use serde_json::{json, Map, Value};

/// 清单 `[[pushes]]` 的 id。
pub const PUSH_SENT: &str = "sent";
pub const PUSH_FAILED: &str = "failed";
/// 卡片的页面（清单 `[[pages]]`）。
pub const PAGE_SENT: &str = "ui://kotone/voice-input/sent";
pub const PAGE_FAILED: &str = "ui://kotone/voice-input/failed";
/// 清单 `[[events]]` 里的名字（`voice-input.` 后面那一段）。
pub const EVENT_SENT: &str = "sent";
pub const EVENT_FAILED: &str = "failed";
/// 卡片上的“重发”（只给页面用）。
pub const TOOL_RESEND: &str = "voice-input.resend";
/// 卡片的名字（和清单里这两条推送的 title 一样，测试查）。
pub const TITLE_SENT: &str = "发出去的话";
pub const TITLE_FAILED: &str = "没发出去";
/// 上一句发出后多久以内的并进同一张卡片。
pub const MERGE_WINDOW_MS: u64 = 2 * 60 * 1000;
/// 一张卡片最多并几句（再多另起一张，卡片不至于太长）。
pub const MAX_LINES: usize = 20;
/// 记住最近多少句没发出去的（重发用；更早的、重启过的从识别记录里找）。
const MAX_FAILED: usize = 32;
/// 报事件时写的来源。
const SOURCE: &str = "voice-input";

/// 插件启动时宿主交来的那一份（`Plugin::start`）。
static HOST: Mutex<Option<Arc<dyn Host>>> = Mutex::new(None);
/// 接上语音输入的本体（`AfterHearing::attached`）：卡片上的重发在“听一句”以外的时候要用它。
static ATTACHED: Mutex<Option<Weak<Orchestrator>>> = Mutex::new(None);

pub(crate) fn set_host(host: Arc<dyn Host>) {
    *HOST.lock().unwrap() = Some(host);
}

pub(crate) fn attach(orch: &Arc<Orchestrator>) {
    *ATTACHED.lock().unwrap() = Some(Arc::downgrade(orch));
}

/// 接上了语音输入、还在的本体（桌面端只有一个）。
pub(crate) fn attached() -> Option<Arc<Orchestrator>> {
    ATTACHED.lock().unwrap().as_ref()?.upgrade()
}

/// 事件的完整名字（`voice-input.sent`）：清单里登记的就是它。
pub fn event_kind(name: &str) -> String {
    format!("{}.{name}", crate::plugin::manifest().id)
}

fn place(id: &str, name: &str) -> HistoryPlace {
    HistoryPlace {
        id: id.to_string(),
        name: name.to_string(),
    }
}

fn place_json(p: &HistoryPlace) -> Value {
    json!({"id": p.id, "name": p.name})
}

/// 发出去的一句（卡片里的一行）。
#[derive(Debug, Clone)]
struct SentLine {
    text: String,
    /// 原话（和处理过的话不一样时才有）
    source: Option<String>,
    at_ms: u64,
}

/// 正在并的那张“发出去的话”。
#[derive(Debug, Clone)]
struct SentCard {
    key: String,
    game: HistoryPlace,
    channel: HistoryPlace,
    /// 最后一句发出的时间
    last_ms: u64,
    lines: Vec<SentLine>,
}

impl SentCard {
    /// 这一句并不并进这张：同一个游戏、同一个频道、上一句发出后 2 分钟内、还没满。
    fn takes(&self, game: &HistoryPlace, channel: &HistoryPlace, now_ms: u64) -> bool {
        self.game.id == game.id
            && self.channel.id == channel.id
            && now_ms.saturating_sub(self.last_ms) <= MERGE_WINDOW_MS
            && self.lines.len() < MAX_LINES
    }

    fn data(&self) -> Value {
        let lines: Vec<Value> = self
            .lines
            .iter()
            .map(|l| {
                let mut line = json!({"text": l.text, "atMs": l.at_ms});
                if let Some(source) = &l.source {
                    line["source"] = json!(source);
                }
                line
            })
            .collect();
        json!({"game": place_json(&self.game), "channel": place_json(&self.channel), "lines": lines})
    }
}

/// 没发出去的一句：卡片上画的、重发要用的。
#[derive(Debug, Clone)]
pub(crate) struct Failed {
    pub(crate) id: String,
    /// 没发出去的话（发了一部分时是后面没发的几行）
    pub(crate) text: String,
    /// 原话（和 `text` 不一样时才有）
    pub(crate) source: Option<String>,
    /// 当时要发去的窗口（开始收音时的前台）；Kotone 重启过、热键重发的不知道
    pub(crate) target: Option<TargetWindow>,
    /// 当时选的频道 (游戏, 频道)；None = 游戏配置的默认频道
    pub(crate) channel: Option<(String, String)>,
    reason: String,
    code: String,
    at_ms: u64,
    attempts: u32,
    /// 重发成功了：发到了哪个游戏、哪个频道、什么时候
    pub(crate) resent: Option<(HistoryPlace, HistoryPlace, u64)>,
}

impl Failed {
    /// 从识别记录里找回来的（Kotone 重启过，内存里没有）：不知道当时的窗口。
    pub(crate) fn recovered(
        id: &str,
        text: String,
        source: Option<String>,
        channel: Option<(String, String)>,
    ) -> Self {
        Self {
            id: id.to_string(),
            text,
            source,
            target: None,
            channel,
            reason: String::new(),
            code: String::new(),
            at_ms: 0,
            attempts: 1,
            resent: None,
        }
    }

    fn data(&self) -> Value {
        let mut data = json!({
            "id": self.id,
            "text": self.text,
            "reason": self.reason,
            "code": self.code,
            "atMs": self.at_ms,
            "attempts": self.attempts,
        });
        if let Some(source) = &self.source {
            data["source"] = json!(source);
        }
        if let Some((game, channel)) = &self.channel {
            data["channel"] = json!({"game": game, "id": channel});
        }
        if let Some((game, channel, at)) = &self.resent {
            data["resent"] =
                json!({"game": place_json(game), "channel": place_json(channel), "atMs": at});
        }
        data
    }
}

/// 发一次的结果（发送那条路交过来，交给后台线程）。
#[derive(Debug, Clone)]
pub(crate) struct Attempt {
    /// 这一句的 id（识别记录的 sessionId；卡片的 key 用它）
    pub(crate) id: String,
    pub(crate) text: String,
    /// 识别原文
    pub(crate) source: String,
    pub(crate) target: Option<TargetWindow>,
    pub(crate) channel: Option<(String, String)>,
    /// 发出去的部分：全发了，或者多行时发了一部分（`sent_lines`）
    pub(crate) outcome: Option<ChatOutcome>,
    /// 没发出去（或者后面几行没发）：给人看的原因、代号
    pub(crate) failure: Option<(String, String)>,
    /// 没发出去时报事件用的游戏（当前选中的游戏配置）
    pub(crate) game: Option<String>,
    pub(crate) now_ms: u64,
}

/// 一件事：游戏、事件的完整名字、数据。
pub(crate) type Event = (String, String, Map<String, Value>);

/// 一次要做的：几条推送、几件事。
#[derive(Debug, Default)]
pub(crate) struct Planned {
    pub(crate) pushes: Vec<Push>,
    pub(crate) events: Vec<Event>,
}

#[derive(Default)]
struct DeskState {
    sent: Option<SentCard>,
    failed: VecDeque<Failed>,
}

/// 语音输入的推送台：记着正在并的那张卡片、最近没发出去的几句；把每一次发送的结果变成推送和事件交给宿主。
#[derive(Default)]
pub(crate) struct PushDesk {
    state: Mutex<DeskState>,
    /// 一次只交一条，按先后（卡片的内容在这把锁里算、在这把锁里推）
    serial: Mutex<()>,
    /// 测试用的宿主（没有 = 插件启动时交来的那一份）
    host: Mutex<Option<Arc<dyn Host>>>,
    /// 还在后台推的条数（测试等它推完）
    inflight: AtomicUsize,
}

impl PushDesk {
    fn host(&self) -> Option<Arc<dyn Host>> {
        self.host
            .lock()
            .unwrap()
            .clone()
            .or_else(|| HOST.lock().unwrap().clone())
    }

    #[cfg(test)]
    pub(crate) fn use_host(&self, host: Arc<dyn Host>) {
        *self.host.lock().unwrap() = Some(host);
    }

    /// 测试：等后台的推送都交完。
    #[cfg(test)]
    pub(crate) async fn settled(&self) {
        let started = std::time::Instant::now();
        while self.inflight.load(Ordering::SeqCst) > 0 {
            assert!(
                started.elapsed() < std::time::Duration::from_secs(5),
                "推送一直没交完"
            );
            tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        }
    }

    /// 内存里那句没发出去的（重发用）。
    pub(crate) fn failed(&self, id: &str) -> Option<Failed> {
        self.state
            .lock()
            .unwrap()
            .failed
            .iter()
            .find(|f| f.id == id)
            .cloned()
    }

    /// 发了一次：算好推送和事件，交给后台线程推（没有宿主就什么都不做）。
    pub(crate) fn report(self: &Arc<Self>, attempt: Attempt) {
        let Some(host) = self.host() else {
            return;
        };
        let desk = self.clone();
        self.inflight.fetch_add(1, Ordering::SeqCst);
        let deliver = move || {
            desk.deliver(host.as_ref(), attempt);
            desk.inflight.fetch_sub(1, Ordering::SeqCst);
        };
        match tokio::runtime::Handle::try_current() {
            Ok(runtime) => {
                runtime.spawn_blocking(deliver);
            }
            Err(_) => deliver(),
        }
    }

    fn deliver(&self, host: &dyn Host, attempt: Attempt) {
        let _one = self.serial.lock().unwrap();
        let planned = plan(&mut self.state.lock().unwrap(), attempt);
        for push in planned.pushes {
            let key = push.key.clone();
            match host.push(push) {
                Ok(outcome) => {
                    kotone_core::log::log(&format!("voice-input push {key}: {outcome:?}"))
                }
                Err(e) => kotone_core::log::log(&format!("voice-input push {key} 没推出去：{e}")),
            }
        }
        for (game, kind, data) in planned.events {
            host.game_event(&game, &kind, SOURCE, data, None);
        }
    }
}

/// 给她的那句（推送的 note）：玩家在游戏里说了什么、对谁说的。
fn sent_note(game: &HistoryPlace, channel: &HistoryPlace, text: &str) -> String {
    format!(
        "玩家刚用语音输入在{}的「{}」频道发了：{text}",
        game.name, channel.name
    )
}

fn sent_push(card: &SentCard, note: String) -> Push {
    Push {
        trigger: PUSH_SENT.into(),
        key: card.key.clone(),
        card: Some(PushCard {
            title: TITLE_SENT.into(),
            uri: PAGE_SENT.into(),
            params: Value::Null,
            data: card.data(),
        }),
        note,
        ..Push::default()
    }
}

fn failed_push(failed: &Failed) -> Push {
    Push {
        trigger: PUSH_FAILED.into(),
        key: format!("failed:{}", failed.id),
        card: Some(PushCard {
            title: TITLE_FAILED.into(),
            uri: PAGE_FAILED.into(),
            params: Value::Null,
            data: failed.data(),
        }),
        ..Push::default()
    }
}

fn data(pairs: &[(&str, &str)]) -> Map<String, Value> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), json!(v)))
        .collect()
}

/// 一次发送的结果 → 推送和事件（纯逻辑，测试直接调）：
///
/// - 发出去了（全部或前几行）：并进正在并的那张“发出去的话”（规则见 [`SentCard::takes`]），否则另起一张；报 `sent`；
///   这一句以前没发出去过（输入条上点了重试、卡片上点了重发），那张“没发出去”标成已重发、原地更新。
/// - 没发出去（或者后面几行没发）：记下这一句（同一句再失败就更新同一张卡片，次数加一），推“没发出去”，报 `failed`。
fn plan(state: &mut DeskState, a: Attempt) -> Planned {
    let mut out = Planned::default();
    if let Some(outcome) = a.outcome.as_ref().filter(|o| o.sent_lines > 0) {
        let sent = outcome.lines[..outcome.sent_lines.min(outcome.lines.len())].join("\n");
        let game = place(&outcome.game, &outcome.game_name);
        let channel = place(&outcome.channel, &outcome.channel_name);
        // 原话：整句都发了、处理过的话和原话不一样时才写
        let source = Some(a.source.trim())
            .filter(|s| a.failure.is_none() && !s.is_empty() && *s != sent.trim())
            .map(str::to_string);
        if !state
            .sent
            .as_ref()
            .is_some_and(|card| card.takes(&game, &channel, a.now_ms))
        {
            state.sent = Some(SentCard {
                key: format!("sent:{}:{}", a.id, a.now_ms),
                game: game.clone(),
                channel: channel.clone(),
                last_ms: a.now_ms,
                lines: Vec::new(),
            });
        }
        let card = state.sent.as_mut().expect("刚放进去");
        card.lines.push(SentLine {
            text: sent.clone(),
            source,
            at_ms: a.now_ms,
        });
        card.last_ms = a.now_ms;
        out.pushes
            .push(sent_push(card, sent_note(&game, &channel, &sent)));
        out.events.push((
            game.id.clone(),
            event_kind(EVENT_SENT),
            data(&[
                ("text", &sent),
                ("channel", &channel.id),
                ("channelName", &channel.name),
            ]),
        ));
        if a.failure.is_none() {
            if let Some(failed) = state
                .failed
                .iter_mut()
                .find(|f| f.id == a.id && f.resent.is_none())
            {
                failed.resent = Some((game, channel, a.now_ms));
                out.pushes.push(failed_push(failed));
            }
        }
    }
    if let Some((reason, code)) = a.failure {
        let partial = a.outcome.as_ref().filter(|o| o.sent_lines > 0);
        let text = match partial {
            Some(o) => o.lines[o.sent_lines.min(o.lines.len())..].join("\n"),
            None => a.text.clone(),
        };
        let source = Some(a.source.trim())
            .filter(|s| partial.is_none() && !s.is_empty() && *s != text.trim())
            .map(str::to_string);
        let attempts = match state.failed.iter().position(|f| f.id == a.id) {
            Some(i) => state.failed.remove(i).map_or(1, |f| f.attempts + 1),
            None => 1,
        };
        let failed = Failed {
            id: a.id.clone(),
            text,
            source,
            target: a.target,
            channel: a.channel.clone(),
            reason: reason.clone(),
            code: code.clone(),
            at_ms: a.now_ms,
            attempts,
            resent: None,
        };
        out.pushes.push(failed_push(&failed));
        state.failed.push_back(failed);
        while state.failed.len() > MAX_FAILED {
            state.failed.pop_front();
        }
        if let Some(game) = a.game.clone().or_else(|| a.channel.clone().map(|(g, _)| g)) {
            out.events.push((
                game,
                event_kind(EVENT_FAILED),
                data(&[("reason", &reason), ("code", &code)]),
            ));
        }
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use kotone_core::send_to_game::{ChatFailure, SendRoute};

    pub(crate) fn outcome(game: (&str, &str), channel: (&str, &str), text: &str) -> ChatOutcome {
        let lines: Vec<String> = text.lines().map(String::from).collect();
        ChatOutcome {
            route: SendRoute::Keys,
            plugin: None,
            game: game.0.into(),
            game_name: game.1.into(),
            channel: channel.0.into(),
            channel_name: channel.1.into(),
            channel_fell_back: false,
            sent_lines: lines.len(),
            lines,
            sent: true,
            stopped: None,
            waited_ms: 0,
        }
    }

    const LOL: (&str, &str) = ("lol", "英雄联盟");
    const TEAM: (&str, &str) = ("team", "队伍");
    const ALL: (&str, &str) = ("all", "所有人");

    fn sent(id: &str, channel: (&str, &str), text: &str, source: &str, now_ms: u64) -> Attempt {
        Attempt {
            id: id.into(),
            text: text.into(),
            source: source.into(),
            target: Some(TargetWindow(42)),
            channel: Some(("lol".into(), channel.0.into())),
            outcome: Some(outcome(LOL, channel, text)),
            failure: None,
            game: Some("lol".into()),
            now_ms,
        }
    }

    fn failed(id: &str, text: &str, reason: &str, now_ms: u64) -> Attempt {
        Attempt {
            id: id.into(),
            text: text.into(),
            source: text.into(),
            target: Some(TargetWindow(42)),
            channel: Some(("lol".into(), "all".into())),
            outcome: None,
            failure: Some((reason.into(), "MODIFIERS_STILL_DOWN".into())),
            game: Some("lol".into()),
            now_ms,
        }
    }

    fn card(push: &Push) -> &Value {
        &push.card.as_ref().unwrap().data
    }

    /// 连着发的几句（同一个游戏、同一个频道、上一句发出后 2 分钟内）并成同一张卡片：同一个 key，卡片里一句一行；
    /// 换了频道、隔了 2 分钟以上、满了，另起一张（新的 key）。
    #[test]
    fn consecutive_sends_in_the_same_channel_within_two_minutes_share_a_card() {
        let mut state = DeskState::default();
        let t = 1_000_000;
        let first = plan(&mut state, sent("a", TEAM, "打野来中路", "打野来中路吧", t));
        let second = plan(
            &mut state,
            sent("b", TEAM, "小龙刷了", "小龙刷了", t + 90_000),
        );
        let (k1, k2) = (&first.pushes[0].key, &second.pushes[0].key);
        assert_eq!(k1, k2, "2 分钟内同一个频道：同一张");
        let lines = card(&second.pushes[0])["lines"].as_array().unwrap().clone();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["text"], "打野来中路");
        assert_eq!(lines[0]["source"], "打野来中路吧", "原话不一样时也写原话");
        assert!(lines[1].get("source").is_none(), "一样就不写");
        assert_eq!(card(&second.pushes[0])["channel"]["name"], "队伍");
        assert_eq!(card(&second.pushes[0])["game"]["name"], "英雄联盟");
        assert_eq!(second.pushes[0].card.as_ref().unwrap().uri, PAGE_SENT);
        // 给她的那句是这一次发的那句
        assert!(
            second.pushes[0].note.contains("小龙刷了"),
            "{}",
            second.pushes[0].note
        );
        assert!(second.pushes[0].note.contains("队伍"));

        // 换了频道：另起一张
        let other = plan(&mut state, sent("c", ALL, "gg", "gg", t + 100_000));
        assert_ne!(&other.pushes[0].key, k1);
        // 回到队伍：上一张已经不是“正在并的”了，也另起一张
        let back = plan(&mut state, sent("d", TEAM, "撤", "撤", t + 110_000));
        assert_ne!(&back.pushes[0].key, k1);

        // 上一句发出后过了 2 分钟以上：另起一张（刚好 2 分钟还算）
        let mut state = DeskState::default();
        let a = plan(&mut state, sent("a", TEAM, "一", "一", t));
        let b = plan(&mut state, sent("b", TEAM, "二", "二", t + MERGE_WINDOW_MS));
        let c = plan(
            &mut state,
            sent("c", TEAM, "三", "三", t + 2 * MERGE_WINDOW_MS + 1),
        );
        assert_eq!(a.pushes[0].key, b.pushes[0].key);
        assert_ne!(b.pushes[0].key, c.pushes[0].key);

        // 满了：另起一张
        let mut state = DeskState::default();
        let keys: Vec<String> = (0..=MAX_LINES)
            .map(|i| {
                plan(
                    &mut state,
                    sent(&format!("s{i}"), TEAM, "x", "x", t + i as u64),
                )
                .pushes[0]
                    .key
                    .clone()
            })
            .collect();
        assert!(keys[..MAX_LINES].iter().all(|k| *k == keys[0]));
        assert_ne!(keys[MAX_LINES], keys[0]);
    }

    /// 发出去一句报 `sent`（文字、频道；游戏是事件的游戏），没发出去报 `failed`（原因、代号）；名字都是清单里登记的。
    #[test]
    fn sends_and_failures_become_declared_events() {
        let mut state = DeskState::default();
        let ok = plan(&mut state, sent("a", TEAM, "打野来中路", "打野来中路", 1));
        let (game, kind, data) = &ok.events[0];
        assert_eq!(game, "lol");
        assert_eq!(kind, "voice-input.sent");
        assert_eq!(data["text"], "打野来中路");
        assert_eq!(data["channel"], "team");
        assert_eq!(data["channelName"], "队伍");
        let bad = plan(&mut state, failed("b", "gg", "Ctrl 一直按着", 2));
        let (game, kind, data) = &bad.events[0];
        assert_eq!(
            (game.as_str(), kind.as_str()),
            ("lol", "voice-input.failed")
        );
        assert_eq!(data["reason"], "Ctrl 一直按着");
        assert_eq!(data["code"], "MODIFIERS_STILL_DOWN");
        let m = crate::plugin::manifest();
        for (_, kind, _) in ok.events.iter().chain(&bad.events) {
            assert!(m.declares_event(kind), "{kind} 要在清单 [[events]] 里");
        }
        // 不知道是哪个游戏（通用、没选）：不报
        let mut nowhere = failed("c", "gg", "x", 3);
        nowhere.game = None;
        nowhere.channel = None;
        assert!(plan(&mut state, nowhere).events.is_empty());
    }

    /// 没发出去：一张卡片（原因 + 那句话 + 重发要的 id），没有给她的那句；同一句再失败更新同一张（次数加一）；之后发出去了，
    /// 那张标成已重发（原地更新），另外照常进“发出去的话”。发了一部分（多行）：发出去的进“发出去的话”，没发的进“没发出去”。
    #[test]
    fn a_failure_is_a_card_that_a_later_success_marks_as_resent() {
        let mut state = DeskState::default();
        let first = plan(&mut state, failed("s1", "打野来中路", "Ctrl 一直按着", 10));
        assert_eq!(first.pushes.len(), 1);
        let push = &first.pushes[0];
        assert_eq!(push.trigger, PUSH_FAILED);
        assert_eq!(push.key, "failed:s1");
        assert!(push.note.is_empty(), "没发出去不用告诉她");
        assert_eq!(card(push)["reason"], "Ctrl 一直按着");
        assert_eq!(card(push)["text"], "打野来中路");
        assert_eq!(card(push)["id"], "s1");
        assert_eq!(card(push)["attempts"], 1);
        assert_eq!(card(push)["channel"], json!({"game": "lol", "id": "all"}));
        let kept = state.failed.back().unwrap();
        assert_eq!(kept.target, Some(TargetWindow(42)));

        let again = plan(&mut state, failed("s1", "打野来中路", "键盘正被占着", 20));
        assert_eq!(again.pushes[0].key, "failed:s1");
        assert_eq!(card(&again.pushes[0])["attempts"], 2);
        assert_eq!(card(&again.pushes[0])["reason"], "键盘正被占着");
        assert_eq!(state.failed.len(), 1);

        let ok = plan(&mut state, sent("s1", ALL, "打野来中路", "打野来中路", 30));
        let keys: Vec<&str> = ok.pushes.iter().map(|p| p.key.as_str()).collect();
        assert!(keys[0].starts_with("sent:"));
        assert_eq!(keys[1], "failed:s1");
        assert_eq!(card(&ok.pushes[1])["resent"]["channel"]["name"], "所有人");
        // 已重发的不再标第二次
        let ok2 = plan(&mut state, sent("s1", ALL, "打野来中路", "打野来中路", 40));
        assert_eq!(ok2.pushes.len(), 1);

        // 多行发了一部分
        let mut state = DeskState::default();
        let mut partial = failed(
            "m",
            "第一行\n第二行",
            "发了 1 行，后面 1 行没发：前台换了窗口",
            50,
        );
        let mut o = outcome(LOL, TEAM, "第一行\n第二行");
        o.sent_lines = 1;
        o.sent = false;
        o.stopped = Some(ChatFailure::ForegroundChanged);
        partial.outcome = Some(o);
        let planned = plan(&mut state, partial);
        assert_eq!(planned.pushes.len(), 2);
        assert_eq!(card(&planned.pushes[0])["lines"][0]["text"], "第一行");
        assert_eq!(
            card(&planned.pushes[1])["text"],
            "第二行",
            "重发只发没发的那几行"
        );
    }
}
