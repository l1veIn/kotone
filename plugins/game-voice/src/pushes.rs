//! 游戏语音的推送和事件（ADR 043 D11，讨论记录第九节“其它插件”）。
//!
//! 她（或者外部 Agent）用 `voice_chat.say` 在游戏语音里替玩家对队友说了一句：
//!
//! - 推送 `said`（她对队友说了，卡片）：聊天里一张小卡片“她对队友说了：……”，和语音输入的“你向「队伍」发了”对称——
//!   凡是以玩家的名义对队友说的话，聊天里都留一条。上一句说完后 2 分钟内的几句并进同一张（同一个 key 再推一次 =
//!   原地更新，D5），满了 [`MAX_LINES`] 句另起一张。
//! - 不给她留说明：这句话是她自己说的，她的对话里本来就有这次 `voice_chat.say`。
//! - 事件 `game-voice.said`（说了什么）：给自动化用，报到现在在玩的游戏上（[`Host::playing_game`]），游戏中心里没有在玩的
//!   游戏就不报。
//!
//! 推送要写对话（磁盘），不在说话的路上等：交给后台线程，一次一条、按先后（[`SaidDesk::report`]）。
//!
//! 卡片里的数据（页面 `ui://kotone/game-voice/said`，`plugins/game-voice/ui/SaidCard.svelte` 照它画）：
//!
//! ```text
//! { lines: [{ text, atMs }] }
//! ```

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use kotone_plugin_api::{Host, Push, PushCard};
use serde_json::{json, Map, Value};

/// 清单 `[[pushes]]` 的 id。
pub const PUSH_SAID: &str = "said";
/// 卡片的页面（清单 `[[pages]]`）。
pub const PAGE_SAID: &str = "ui://kotone/game-voice/said";
/// 卡片的名字（和清单里这条推送的 title 一样，测试查）。
pub const TITLE_SAID: &str = "她对队友说了";
/// 清单 `[[events]]` 里的名字（`game-voice.` 后面那一段）。
pub const EVENT_SAID: &str = "said";
/// 上一句说完后多久以内的并进同一张卡片（和语音输入的“发出去的话”一样）。
pub const MERGE_WINDOW_MS: u64 = 2 * 60 * 1000;
/// 一张卡片最多并几句。
pub const MAX_LINES: usize = 20;
/// 报事件时写的来源。
const SOURCE: &str = "game-voice";

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 事件的完整名字（`game-voice.said`）：清单里登记的就是它。
pub fn event_kind(name: &str) -> String {
    format!("{}.{name}", crate::manifest().id)
}

/// 正在并的那张“她对队友说了”。
#[derive(Debug, Clone)]
struct SaidCard {
    key: String,
    last_ms: u64,
    lines: Vec<(String, u64)>,
}

impl SaidCard {
    fn takes(&self, now_ms: u64) -> bool {
        now_ms.saturating_sub(self.last_ms) <= MERGE_WINDOW_MS && self.lines.len() < MAX_LINES
    }

    fn data(&self) -> Value {
        let lines: Vec<Value> = self
            .lines
            .iter()
            .map(|(text, at)| json!({"text": text, "atMs": at}))
            .collect();
        json!({"lines": lines})
    }
}

/// 一件事：游戏、事件的完整名字、数据。
pub(crate) type Event = (String, String, Map<String, Value>);

/// 说了一句 → 一条推送和（有在玩的游戏时）一件事（纯逻辑，测试直接调）。上一句说完后 2 分钟内、不满 20 句的并进
/// 正在并的那张，否则另起一张。
fn plan(
    card: &mut Option<SaidCard>,
    text: &str,
    now_ms: u64,
    game: Option<String>,
) -> (Push, Option<Event>) {
    if !card.as_ref().is_some_and(|c| c.takes(now_ms)) {
        *card = Some(SaidCard {
            key: format!("{PUSH_SAID}:{now_ms}"),
            last_ms: now_ms,
            lines: Vec::new(),
        });
    }
    let card = card.as_mut().expect("刚放进去");
    card.lines.push((text.to_string(), now_ms));
    // 后台线程偶尔不按先后跑：卡片里照说的时间排
    card.lines.sort_by_key(|(_, at)| *at);
    card.last_ms = card.last_ms.max(now_ms);
    let push = Push {
        trigger: PUSH_SAID.into(),
        key: card.key.clone(),
        card: Some(PushCard {
            title: TITLE_SAID.into(),
            uri: PAGE_SAID.into(),
            params: Value::Null,
            data: card.data(),
        }),
        ..Push::default()
    };
    let event = game.map(|game| {
        let mut data = Map::new();
        data.insert("text".into(), json!(text));
        (game, event_kind(EVENT_SAID), data)
    });
    (push, event)
}

/// 游戏语音的推送台：记着正在并的那张卡片。
#[derive(Default)]
pub(crate) struct SaidDesk {
    card: Mutex<Option<SaidCard>>,
    /// 一次只交一条，按先后
    serial: Mutex<()>,
    /// 还在后台推的条数（测试等它推完）
    inflight: AtomicUsize,
}

impl SaidDesk {
    /// 她对队友说了一句：交给后台线程推送、报事件（说话那条路不等它）。
    pub(crate) fn report(self: &Arc<Self>, host: Arc<dyn Host>, text: &str) {
        let desk = self.clone();
        let text = text.to_string();
        let now = now_ms();
        self.inflight.fetch_add(1, Ordering::SeqCst);
        let deliver = move || {
            desk.deliver(host.as_ref(), &text, now);
            desk.inflight.fetch_sub(1, Ordering::SeqCst);
        };
        match tokio::runtime::Handle::try_current() {
            Ok(runtime) => {
                runtime.spawn_blocking(deliver);
            }
            Err(_) => deliver(),
        }
    }

    fn deliver(&self, host: &dyn Host, text: &str, now_ms: u64) {
        let _one = self.serial.lock().unwrap();
        let game = host.playing_game().map(|p| p.game);
        let (push, event) = plan(&mut self.card.lock().unwrap(), text, now_ms, game);
        let key = push.key.clone();
        match host.push(push) {
            Ok(outcome) => kotone_core::log::log(&format!("game-voice push {key}: {outcome:?}")),
            Err(e) => kotone_core::log::log(&format!("game-voice push {key} 没推出去：{e}")),
        }
        if let Some((game, kind, data)) = event {
            host.game_event(&game, &kind, SOURCE, data, None);
        }
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    use kotone_plugin_api::{BoxFuture, Observation, Playing, PushOutcome};

    const T: u64 = 1_800_000_000_000;

    fn lines(push: &Push) -> Vec<Value> {
        push.card.as_ref().unwrap().data["lines"]
            .as_array()
            .unwrap()
            .clone()
    }

    /// 上一句说完后 2 分钟内的并进同一张（刚好 2 分钟还算），一句一行；隔得更久、满了 20 句另起一张；不给她留说明。
    #[test]
    fn sentences_within_two_minutes_share_a_card() {
        let mut card = None;
        let (a, _) = plan(&mut card, "我去拿龙", T, None);
        let (b, _) = plan(&mut card, "等我一下", T + MERGE_WINDOW_MS, None);
        assert_eq!(a.key, b.key);
        let both = lines(&b);
        assert_eq!(both.len(), 2);
        assert_eq!(both[0]["text"], "我去拿龙");
        assert_eq!(both[1]["text"], "等我一下");
        assert_eq!(both[1]["atMs"], T + MERGE_WINDOW_MS);
        assert_eq!(b.trigger, PUSH_SAID);
        let shown = b.card.as_ref().unwrap();
        assert_eq!(
            (shown.title.as_str(), shown.uri.as_str()),
            (TITLE_SAID, PAGE_SAID)
        );
        assert!(b.note.is_empty(), "她自己说的：她的对话里本来就有");
        let (c, _) = plan(&mut card, "撤", T + 2 * MERGE_WINDOW_MS + 1, None);
        assert_ne!(b.key, c.key, "隔了 2 分钟以上");
        assert_eq!(lines(&c).len(), 1);

        let mut card = None;
        let keys: Vec<String> = (0..=MAX_LINES)
            .map(|i| plan(&mut card, "x", T + i as u64, None).0.key)
            .collect();
        assert!(keys[..MAX_LINES].iter().all(|k| *k == keys[0]));
        assert_ne!(keys[MAX_LINES], keys[0], "满了另起一张");
    }

    /// 有在玩的游戏：报 `game-voice.said`（说了什么；名字在清单里）；没有：不报。
    #[test]
    fn a_sentence_becomes_a_declared_event_on_the_playing_game() {
        let mut card = None;
        let (_, event) = plan(&mut card, "我去拿龙", T, Some("lol".into()));
        let (game, kind, data) = event.unwrap();
        assert_eq!((game.as_str(), kind.as_str()), ("lol", "game-voice.said"));
        assert_eq!(data["text"], "我去拿龙");
        assert!(crate::manifest().declares_event(&kind));
        assert!(plan(&mut card, "撤", T + 1, None).1.is_none());
    }

    #[derive(Default)]
    struct RecordingHost {
        pushes: Mutex<Vec<Push>>,
        events: Mutex<Vec<(String, String)>>,
    }

    impl Host for RecordingHost {
        fn plugin(&self) -> &str {
            "game-voice"
        }
        fn enabled(&self) -> bool {
            true
        }
        fn source_enabled(&self, _: &str) -> bool {
            true
        }
        fn settings(&self) -> Value {
            Value::Null
        }
        fn data_dir(&self) -> PathBuf {
            PathBuf::new()
        }
        fn spawn(&self, _: BoxFuture<'static, ()>) {}
        fn process_ids(&self, _: &str) -> Vec<u32> {
            Vec::new()
        }
        fn process_command_line(&self, _: u32) -> Option<String> {
            None
        }
        fn game_observe(&self, _: &str, _: Observation) {}
        fn game_claim(&self, _: &str, _: bool) {}
        fn game_event(
            &self,
            game: &str,
            kind: &str,
            _: &str,
            _: Map<String, Value>,
            _: Option<f64>,
        ) {
            self.events.lock().unwrap().push((game.into(), kind.into()));
        }
        fn playing_game(&self) -> Option<Playing> {
            Some(Playing {
                game: "pubg".into(),
                match_id: String::new(),
            })
        }
        fn push(&self, push: Push) -> Result<PushOutcome, String> {
            self.pushes.lock().unwrap().push(push);
            Ok(PushOutcome::Posted)
        }
    }

    /// 说了两句：后台按先后推两次同一张卡片（第二次带着两句）、报两件事，说话那条路不等它。
    #[tokio::test]
    async fn reports_go_out_in_order_off_the_speaking_path() {
        let desk = Arc::new(SaidDesk::default());
        let host = Arc::new(RecordingHost::default());
        desk.report(host.clone(), "我去拿龙");
        desk.report(host.clone(), "等我一下");
        desk.settled().await;
        let pushes = host.pushes.lock().unwrap().clone();
        assert_eq!(pushes.len(), 2);
        assert_eq!(pushes[0].key, pushes[1].key);
        let texts: Vec<Value> = lines(&pushes[1])
            .iter()
            .map(|l| l["text"].clone())
            .collect();
        assert_eq!(
            texts,
            [json!("我去拿龙"), json!("等我一下")],
            "照说的时间排"
        );
        let events = host.events.lock().unwrap().clone();
        assert_eq!(
            events,
            vec![("pubg".to_string(), "game-voice.said".to_string()); 2]
        );
    }
}
