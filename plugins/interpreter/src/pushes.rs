//! 同传的推送和事件（ADR 043 D11，讨论记录第九节“其它插件”）。
//!
//! 后台当同传时每翻好一句调 [`crate::TOOL_SHOW`]，插件：
//!
//! - 推送 `lines`（这局的同传，卡片 + 一行）：一局一张卡片“这局的同传”，每翻好一句原地加一行（同一个 key 再推一次 =
//!   更新，D5），卡片里只留最近 [`MAX_LINES`] 句（写着一共几句）。玩家选了气泡时这一句同时是推送的一行（悬浮窗的气泡里
//!   显示）；选了弹幕时只进卡片，屏幕上那一句后台用 danmaku 显示。
//!   哪些句子算“这一局”：游戏中心知道现在是哪一局（[`Host::playing_game`] 的 `match_id`）时按局；分不出局（只看进程的
//!   游戏、在听 Discord 而没在游戏里）时，上一句翻好后 [`GAP_MS`] 内的、没有重新点开始 / 停止的并进同一张。
//! - 不给她留说明（`note` 空）：同传是她自己交给后台的，要紧的话后台会告诉她；队友说的话是别人的话，不进她的主对话（ADR 021）。
//! - 事件 `interpreter.line`（译文、原话、在听的程序）：报到现在在玩的游戏上（[`Host::game_event`]），游戏中心里没有在玩的
//!   游戏就不报。原话和译文只交给自动化（清单 `private`），她在 `game.state` 里只看到“听到一句”。
//!
//! 卡片里的数据（页面 `ui://kotone/interpreter/lines`，`plugins/interpreter/ui/LinesCard.svelte` 照它画）：
//!
//! ```text
//! { target, startedAtMs, total, lines: [{ text, heard?, app?, atMs }] }   // total = 一共几句（lines 只留最近的）
//! ```

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use kotone_plugin_api::{Host, Playing, Push, PushCard, PushCell, PushLine, PushOutcome};
use serde_json::{json, Map, Value};

use crate::Prefs;

/// 清单 `[[pushes]]` 的 id。
pub const PUSH_LINES: &str = "lines";
/// 卡片的页面（清单 `[[pages]]`）。
pub const PAGE_LINES: &str = "ui://kotone/interpreter/lines";
/// 卡片的名字（和清单里这条推送的 title 一样，测试查）。
pub const TITLE_LINES: &str = "这局的同传";
/// 清单 `[[events]]` 里的名字（`interpreter.` 后面那一段）。
pub const EVENT_LINE: &str = "line";
/// 卡片里最多留几句（最近的；再早的在后台那段对话里）。
pub const MAX_LINES: usize = 30;
/// 分不出局时：上一句翻好后多久以内的并进同一张。
pub const GAP_MS: u64 = 10 * 60 * 1000;
/// 一句最多几个字（和工具参数的上限一样）。
const TEXT_MAX: usize = 300;
/// 报事件时写的来源。
const SOURCE: &str = "interpreter";

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 事件的完整名字（`interpreter.line`）：清单里登记的就是它。
pub fn event_kind(name: &str) -> String {
    format!("{}.{name}", crate::manifest().id)
}

/// 一张卡片管哪些句子。
#[derive(Debug, Clone, PartialEq)]
enum Scope {
    /// 游戏中心里的这一局。
    Match { game: String, id: String },
    /// 分不出局：按时间（[`GAP_MS`]）和开始 / 停止。
    Loose,
}

fn scope_of(playing: Option<&Playing>) -> Scope {
    match playing {
        Some(p) if !p.match_id.is_empty() => Scope::Match {
            game: p.game.clone(),
            id: p.match_id.clone(),
        },
        _ => Scope::Loose,
    }
}

/// 翻好的一句（卡片里的一行）。
#[derive(Debug, Clone)]
struct Line {
    text: String,
    heard: Option<String>,
    app: Option<String>,
    at_ms: u64,
}

/// 正在加的那张“这局的同传”。
#[derive(Debug, Clone)]
struct Card {
    key: String,
    scope: Scope,
    target: String,
    started_ms: u64,
    last_ms: u64,
    /// 一共几句（`lines` 只留最近 [`MAX_LINES`] 句）
    total: usize,
    lines: VecDeque<Line>,
    /// 玩家点了开始 / 停止：分不出局时下一句另起一张
    closed: bool,
}

impl Card {
    /// 这一句加不加进这张：同一局（游戏中心知道是哪一局时）；分不出局时，没有重新点开始 / 停止、上一句翻好后 10 分钟内。
    fn takes(&self, scope: &Scope, now_ms: u64) -> bool {
        match (&self.scope, scope) {
            (Scope::Match { .. }, _) => self.scope == *scope,
            (Scope::Loose, Scope::Loose) => {
                !self.closed && now_ms.saturating_sub(self.last_ms) <= GAP_MS
            }
            (Scope::Loose, Scope::Match { .. }) => false,
        }
    }

    fn data(&self) -> Value {
        let lines: Vec<Value> = self
            .lines
            .iter()
            .map(|l| {
                let mut line = json!({"text": l.text, "atMs": l.at_ms});
                if let Some(heard) = &l.heard {
                    line["heard"] = json!(heard);
                }
                if let Some(app) = &l.app {
                    line["app"] = json!(app);
                }
                line
            })
            .collect();
        json!({
            "target": self.target,
            "startedAtMs": self.started_ms,
            "total": self.total,
            "lines": lines,
        })
    }
}

/// 后台交来的一句。
#[derive(Debug, Clone)]
pub(crate) struct Translated {
    /// 译文
    pub(crate) text: String,
    /// 听到的原话
    pub(crate) heard: Option<String>,
    /// 在听的程序
    pub(crate) app: Option<String>,
    pub(crate) at_ms: u64,
    /// 现在在玩的游戏（游戏中心）
    pub(crate) playing: Option<Playing>,
    /// 译成什么（页面上的设置）
    pub(crate) target: String,
    /// 玩家选了气泡（这一句同时是推送的一行）
    pub(crate) bubble: bool,
}

/// 一件事：游戏、事件的完整名字、数据。
pub(crate) type Event = (String, String, Map<String, Value>);

/// 一句要做的：一条推送、（有在玩的游戏时）一件事。
#[derive(Debug)]
pub(crate) struct Planned {
    pub(crate) push: Push,
    pub(crate) event: Option<Event>,
}

/// 同传的推送台：记着正在加的那张卡片。
#[derive(Debug, Default)]
pub struct Desk {
    card: Option<Card>,
}

impl Desk {
    /// 玩家在页面上点了开始 / 停止：分不出局时下一句另起一张（同一局的照旧在一张里）。
    pub fn reset(&mut self) {
        if let Some(card) = &mut self.card {
            card.closed = true;
        }
    }

    /// 翻好的一句 → 一条推送和一件事（纯逻辑，测试直接调）。规则见模块说明。
    pub(crate) fn plan(&mut self, t: Translated) -> Planned {
        let scope = scope_of(t.playing.as_ref());
        if !self
            .card
            .as_ref()
            .is_some_and(|card| card.takes(&scope, t.at_ms))
        {
            self.card = Some(Card {
                // 带上开始的时间：Kotone 重启以后游戏中心的局号会重来，不能和以前那张撞上
                key: format!("{PUSH_LINES}:{}", t.at_ms),
                scope,
                target: t.target.clone(),
                started_ms: t.at_ms,
                last_ms: t.at_ms,
                total: 0,
                lines: VecDeque::new(),
                closed: false,
            });
        }
        let card = self.card.as_mut().expect("刚放进去");
        card.target = t.target.clone();
        card.last_ms = t.at_ms;
        card.total += 1;
        card.lines.push_back(Line {
            text: t.text.clone(),
            heard: t.heard.clone(),
            app: t.app.clone(),
            at_ms: t.at_ms,
        });
        while card.lines.len() > MAX_LINES {
            card.lines.pop_front();
        }
        let push = Push {
            trigger: PUSH_LINES.into(),
            key: card.key.clone(),
            card: Some(PushCard {
                title: TITLE_LINES.into(),
                uri: PAGE_LINES.into(),
                params: Value::Null,
                data: card.data(),
            }),
            // 选了气泡：这一句显示在悬浮窗的气泡里（只有一格：一整句，宿主最多留 40 个字）
            line: t.bubble.then(|| PushLine {
                cells: vec![PushCell {
                    text: t.text.clone(),
                    tone: None,
                }],
            }),
            ..Push::default()
        };
        let event = t.playing.as_ref().map(|p| {
            let data: Map<String, Value> = [
                ("text", t.text.as_str()),
                ("heard", t.heard.as_deref().unwrap_or("")),
                ("app", t.app.as_deref().unwrap_or("")),
            ]
            .into_iter()
            .map(|(k, v)| (k.to_string(), json!(v)))
            .collect();
            (p.game.clone(), event_kind(EVENT_LINE), data)
        });
        Planned { push, event }
    }
}

fn text_arg(args: &Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.chars().take(TEXT_MAX).collect())
}

/// [`crate::TOOL_SHOW`]：后台翻好了一句。推送、报事件（在台子的锁里按先后：两句同时来时，后推的那次带着两句），
/// 回 `shown`（false = 这条推送关着或推不出去：后台改用 danmaku）。
pub(crate) fn show(
    desk: &Mutex<Desk>,
    host: &dyn Host,
    prefs: &Prefs,
    args: &Value,
    now_ms: u64,
) -> Result<Value, String> {
    let text = text_arg(args, "text").ok_or("text 不能为空：写译文")?;
    // 没写在听哪个程序：只在听一个时就是它
    let app = text_arg(args, "app").or_else(|| {
        let listening = host.listening();
        (listening.len() == 1).then(|| listening[0].clone())
    });
    let translated = Translated {
        text,
        heard: text_arg(args, "heard"),
        app,
        at_ms: now_ms,
        playing: host.playing_game(),
        target: prefs.target.clone(),
        bubble: prefs.bubble,
    };
    let mut desk = desk.lock().unwrap();
    let planned = desk.plan(translated);
    let outcome = host.push(planned.push);
    if let Some((game, kind, data)) = planned.event {
        host.game_event(&game, &kind, SOURCE, data, None);
    }
    Ok(match outcome {
        Ok(PushOutcome::Off) => {
            json!({"shown": false, "note": "同传的推送关着（插件页「设置 → 推送」）：这一句用 danmaku 显示"})
        }
        Ok(_) => json!({"shown": true}),
        // 宿主收推送时出的错宿主自己记日志（`DesktopHost::push`）；这里只告诉后台改用弹幕
        Err(e) => {
            json!({"shown": false, "note": format!("没显示出来（{e}）：这一句用 danmaku 显示")})
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::Arc;

    use kotone_plugin_api::{BoxFuture, Observation};

    const T: u64 = 1_800_000_000_000;

    fn lol(match_id: &str) -> Option<Playing> {
        Some(Playing {
            game: "lol".into(),
            match_id: match_id.into(),
        })
    }

    fn line(text: &str, at_ms: u64, playing: Option<Playing>) -> Translated {
        Translated {
            text: text.into(),
            heard: Some(format!("heard {text}")),
            app: Some("League of Legends.exe".into()),
            at_ms,
            playing,
            target: "中文".into(),
            bubble: true,
        }
    }

    fn card(planned: &Planned) -> &Value {
        &planned.push.card.as_ref().unwrap().data
    }

    /// 一局一张卡片：同一局的句子同一个 key，新的在下面；换了一局另起一张；卡片里最多留最近 30 句、写着一共几句。
    #[test]
    fn lines_of_one_match_share_one_card() {
        let mut desk = Desk::default();
        let a = desk.plan(line("小心左边", T, lol("m1")));
        let b = desk.plan(line("我去拿龙", T + 30 * 60 * 1000, lol("m1")));
        assert_eq!(a.push.key, b.push.key, "同一局：隔得再久也是同一张");
        assert!(a.push.key.starts_with("lines:"));
        let lines = card(&b)["lines"].as_array().unwrap();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["text"], "小心左边");
        assert_eq!(lines[1]["text"], "我去拿龙");
        assert_eq!(lines[1]["heard"], "heard 我去拿龙");
        assert_eq!(lines[1]["app"], "League of Legends.exe");
        assert_eq!(card(&b)["total"], 2);
        assert_eq!(card(&b)["target"], "中文");
        assert_eq!(b.push.card.as_ref().unwrap().title, TITLE_LINES);
        assert_eq!(b.push.card.as_ref().unwrap().uri, PAGE_LINES);
        assert!(
            b.push.note.is_empty(),
            "不给她留说明：别人的话不进她的主对话"
        );
        assert!(!b.push.alert);

        // 点了开始 / 停止：同一局的照旧在这张里
        desk.reset();
        let c = desk.plan(line("撤", T + 31 * 60 * 1000, lol("m1")));
        assert_eq!(c.push.key, a.push.key);

        // 下一局：另起一张
        let next = desk.plan(line("你好", T + 40 * 60 * 1000, lol("m2")));
        assert_ne!(next.push.key, a.push.key);
        assert_eq!(card(&next)["total"], 1);

        // 满了只留最近 30 句
        let mut desk = Desk::default();
        let last = (0..MAX_LINES + 5)
            .map(|i| desk.plan(line(&format!("第{i}句"), T + i as u64, lol("m1"))))
            .last()
            .unwrap();
        let lines = card(&last)["lines"].as_array().unwrap();
        assert_eq!(lines.len(), MAX_LINES);
        assert_eq!(lines[0]["text"], "第5句");
        assert_eq!(card(&last)["total"], MAX_LINES + 5);
    }

    /// 分不出局（没在游戏里、只看进程的游戏）：上一句翻好后 10 分钟内并进同一张（刚好 10 分钟还算）；隔得更久、
    /// 点了开始 / 停止，另起一张；从分不出局到进了一局也另起一张。
    #[test]
    fn without_a_match_lines_merge_by_time_until_start_or_stop() {
        let mut desk = Desk::default();
        let a = desk.plan(line("一", T, None));
        let b = desk.plan(line("二", T + GAP_MS, None));
        assert_eq!(a.push.key, b.push.key);
        let c = desk.plan(line("三", T + 2 * GAP_MS + 1, None));
        assert_ne!(b.push.key, c.push.key, "隔了 10 分钟以上");
        desk.reset();
        let d = desk.plan(line("四", T + 2 * GAP_MS + 2, None));
        assert_ne!(c.push.key, d.push.key, "重新点了开始");
        // 只看进程的游戏（局号为空）也是按时间
        let pubg = Some(Playing {
            game: "pubg".into(),
            match_id: String::new(),
        });
        let e = desk.plan(line("五", T + 2 * GAP_MS + 3, pubg));
        assert_eq!(d.push.key, e.push.key);
        let f = desk.plan(line("六", T + 2 * GAP_MS + 4, lol("m9")));
        assert_ne!(e.push.key, f.push.key, "进了一局：这局一张");
        let g = desk.plan(line("七", T + 2 * GAP_MS + 5, None));
        assert_ne!(f.push.key, g.push.key, "这局结束了");
    }

    /// 选了气泡：这一句也是推送的一行（一格、一整句）；选了弹幕：只进卡片。
    #[test]
    fn the_line_is_the_translation_only_when_the_player_chose_the_bubble() {
        let mut desk = Desk::default();
        let bubble = desk.plan(line("他们三个人从左边绕过来了", T, None));
        let cells = &bubble.push.line.as_ref().unwrap().cells;
        assert_eq!(cells.len(), 1);
        assert_eq!(cells[0].text, "他们三个人从左边绕过来了");
        assert_eq!(cells[0].tone, None);
        let mut danmaku = line("撤", T + 1, None);
        danmaku.bubble = false;
        let planned = desk.plan(danmaku);
        assert!(planned.push.line.is_none());
        assert_eq!(card(&planned)["lines"].as_array().unwrap().len(), 2);
        // 宿主照清单截字：一整句的一行留得下
        let spec = crate::manifest().push_spec(PUSH_LINES).unwrap();
        let shaped = bubble.push.clone().shaped(spec);
        assert!(shaped.check(crate::manifest()).is_ok());
        assert_eq!(
            shaped.line.as_ref().unwrap().cells[0].text,
            "他们三个人从左边绕过来了"
        );
        assert!(shaped.card.is_some());
    }

    /// 有在玩的游戏：报 `interpreter.line`（译文、原话、在听的程序；名字在清单里）；没有：不报。
    #[test]
    fn a_line_becomes_a_declared_event_on_the_playing_game() {
        let mut desk = Desk::default();
        let planned = desk.plan(line("小心", T, lol("m1")));
        let (game, kind, data) = planned.event.unwrap();
        assert_eq!((game.as_str(), kind.as_str()), ("lol", "interpreter.line"));
        assert_eq!(data["text"], "小心");
        assert_eq!(data["heard"], "heard 小心");
        assert_eq!(data["app"], "League of Legends.exe");
        assert!(crate::manifest().declares_event(&kind));
        let mut nowhere = line("小心", T + 1, None);
        nowhere.heard = None;
        assert!(desk.plan(nowhere).event.is_none());
    }

    /// 测试用的宿主：记下推送和事件；推送关着时回 Off。
    #[derive(Default)]
    struct FakeHost {
        pushes: Mutex<Vec<Push>>,
        events: Mutex<Vec<Event>>,
        off: bool,
        listening: Vec<String>,
        playing: Option<Playing>,
    }

    impl Host for FakeHost {
        fn plugin(&self) -> &str {
            "interpreter"
        }
        fn enabled(&self) -> bool {
            true
        }
        fn source_enabled(&self, _source: &str) -> bool {
            true
        }
        fn settings(&self) -> Value {
            json!({})
        }
        fn data_dir(&self) -> PathBuf {
            PathBuf::new()
        }
        fn spawn(&self, _task: BoxFuture<'static, ()>) {}
        fn process_ids(&self, _exe: &str) -> Vec<u32> {
            Vec::new()
        }
        fn process_command_line(&self, _pid: u32) -> Option<String> {
            None
        }
        fn game_observe(&self, _game: &str, _observation: Observation) {}
        fn game_claim(&self, _game: &str, _claimed: bool) {}
        fn game_event(
            &self,
            game: &str,
            kind: &str,
            _source: &str,
            data: Map<String, Value>,
            _game_time: Option<f64>,
        ) {
            self.events
                .lock()
                .unwrap()
                .push((game.into(), kind.into(), data));
        }
        fn listening(&self) -> Vec<String> {
            self.listening.clone()
        }
        fn playing_game(&self) -> Option<Playing> {
            self.playing.clone()
        }
        fn push(&self, push: Push) -> Result<PushOutcome, String> {
            let first = !self
                .pushes
                .lock()
                .unwrap()
                .iter()
                .any(|p| p.key == push.key);
            self.pushes.lock().unwrap().push(push);
            Ok(if self.off {
                PushOutcome::Off
            } else if first {
                PushOutcome::Posted
            } else {
                PushOutcome::Updated
            })
        }
    }

    /// 工具：推送这一句（没写程序时用正在听的那一个）、报事件、回 shown；推送关着时回 shown=false 让后台改用弹幕；
    /// 没有译文直接说。
    #[test]
    fn the_show_tool_pushes_reports_and_answers() {
        let desk = Mutex::new(Desk::default());
        let host = Arc::new(FakeHost {
            listening: vec!["Discord.exe".into()],
            playing: lol("m1"),
            ..FakeHost::default()
        });
        let prefs = Prefs::from_settings(&json!({"target": "中文"}));
        let out = show(
            &desk,
            host.as_ref(),
            &prefs,
            &json!({"text": " 我需要医疗包 ", "heard": "I need a medkit"}),
            T,
        )
        .unwrap();
        assert_eq!(out, json!({"shown": true}));
        let out = show(&desk, host.as_ref(), &prefs, &json!({"text": "撤"}), T + 1).unwrap();
        assert_eq!(out["shown"], true);
        let pushes = host.pushes.lock().unwrap().clone();
        assert_eq!(pushes.len(), 2);
        assert_eq!(pushes[0].key, pushes[1].key, "同一局同一张");
        assert_eq!(pushes[0].trigger, PUSH_LINES);
        let lines = pushes[1].card.as_ref().unwrap().data["lines"].clone();
        assert_eq!(lines[0]["text"], "我需要医疗包");
        assert_eq!(lines[0]["heard"], "I need a medkit");
        assert_eq!(lines[0]["app"], "Discord.exe", "没写程序：正在听的那一个");
        assert!(lines[1].get("heard").is_none());
        assert_eq!(pushes[1].line.as_ref().unwrap().cells[0].text, "撤");
        let events = host.events.lock().unwrap().clone();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].0, "lol");
        assert_eq!(events[0].1, "interpreter.line");

        let off = FakeHost {
            off: true,
            ..FakeHost::default()
        };
        let out = show(&desk, &off, &prefs, &json!({"text": "撤"}), T + 2).unwrap();
        assert_eq!(out["shown"], false);
        assert!(out["note"].as_str().unwrap().contains("danmaku"));
        assert!(
            off.events.lock().unwrap().is_empty(),
            "没有在玩的游戏：不报"
        );
        assert!(show(&desk, &off, &prefs, &json!({"text": "  "}), T + 3).is_err());
    }

    /// 清单：推送是卡片 + 一行、默认开、不要数据开关，卡片的页面在清单里；事件以插件 id 开头、原话和译文只给自动化。
    /// 她的设置说明里写着这条推送的开关在哪，由桌面端的 `plugins::tests` 对所有插件一起查（插件只依赖插件契约）。
    #[test]
    fn the_push_and_the_event_are_declared() {
        use kotone_plugin_api::PushForm;
        let m = crate::manifest();
        let spec = m.push_spec(PUSH_LINES).expect("清单里要有推送 lines");
        assert_eq!(spec.title, TITLE_LINES);
        assert_eq!(spec.forms, [PushForm::Card, PushForm::Line]);
        assert!(spec.enabled_by_default && spec.needs.is_empty() && !spec.until_done);
        assert!(m.page(PAGE_LINES).is_some());
        assert!(m.adapter.is_none());
        // 事件：以插件 id 开头、不要数据开关（目录里不会因为开关关着标原因）、不属于哪个游戏，原话和译文只给自动化
        let entry = m
            .events
            .iter()
            .find(|e| e.kind == event_kind(EVENT_LINE))
            .expect("清单里要有事件 line");
        assert_eq!(m.event_scope(), m.id);
        assert!(m.games.is_empty() && entry.needs.is_empty());
        for field in ["text", "heard", "app"] {
            assert!(entry.data.contains_key(field), "{field}");
        }
        assert_eq!(entry.private, ["text", "heard"]);
    }
}
