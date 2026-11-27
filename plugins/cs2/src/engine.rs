//! 把一份份 GSI 的 POST 变成游戏中心的阶段、`cs2.*` 事件和推送（纯逻辑，测试直接喂）。
//!
//! - **阶段**（ADR 036 D3）：没有 `map` = 菜单（`menu`），`warmup` = 热身（`pre_match`），`live` / `intermission` = 对局中，
//!   `gameover` = 结算（`post_match`）。收到第一份就认领 `cs2`（进程那一路不再报），30 秒收不到、游戏也不在了就交回。
//! - **一局**（这里叫一张图）：地图换了、上一张打完了又开始热身 / 开打、比分回到 0 : 0 都算新的一张。
//! - **事件**只在看到变化时发（前一份和这一份不一样），第一次连上时只记底、不补以前的。
//! - **推送**只说事实：比分、回合、时间、谁赢；同样的输入同样的输出。下包倒数的卡片只在状态变了时更新（下了 / 拆了 / 炸了 /
//!   回合结束），倒数本身由卡片按 `explodesAtMs` 自己走，不每秒推一次。

use std::collections::HashSet;

use kotone_plugin_api::{Observation, Phase, Push, PushCard, PushCell, PushLine, PushTone};
use serde_json::{json, Map, Value};

use crate::gsi::{MatchStats, PlayerState, Side, Snapshot};
use crate::rules::{self, C4_SECONDS};

/// 游戏配置 id（和 `crates/kotone-core/resources/profiles/cs2.json` 一样）。
pub const GAME: &str = "cs2";
/// 报给游戏中心的来源。
pub const SOURCE: &str = "cs2/gsi";
/// 多久收不到 POST 算断了（cfg 里的心跳是 10 秒）。
pub const STALE_MS: u64 = 30_000;
/// 剩几秒时说“还剩十秒”。
pub const TEN_MS: u64 = 10_000;

/// 推送的 id（清单 `[[pushes]]`）。
pub const PUSH_BOMB: &str = "bomb";
pub const PUSH_BOMB_TEN: &str = "bomb-ten";
pub const PUSH_MATCH_POINT: &str = "match-point";
pub const PUSH_HALFTIME: &str = "halftime";
pub const PUSH_MATCH_END: &str = "match-end";
/// 页面（清单 `[[pages]]`）。
pub const PAGE_MATCH: &str = "ui://kotone/cs2/match";
pub const PAGE_BOMB: &str = "ui://kotone/cs2/bomb";

/// 事件的完整名字（清单 `[[events]]`）。
pub const EV_ROUND_STARTED: &str = "cs2.round_started";
pub const EV_ROUND_ENDED: &str = "cs2.round_ended";
pub const EV_BOMB_PLANTED: &str = "cs2.bomb_planted";
pub const EV_BOMB_DEFUSED: &str = "cs2.bomb_defused";
pub const EV_BOMB_EXPLODED: &str = "cs2.bomb_exploded";
pub const EV_HALFTIME: &str = "cs2.halftime";
pub const EV_MATCH_POINT: &str = "cs2.match_point";
pub const EV_MATCH_ENDED: &str = "cs2.match_ended";
pub const EV_MY_DEATH: &str = "cs2.my_death";

/// 引擎要宿主做的事（按顺序做）。
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// 认领（true）或交回（false）`cs2` 的阶段。
    Claim(bool),
    Observe(Observation),
    Event {
        kind: &'static str,
        data: Map<String, Value>,
    },
    Push(Push),
}

/// 一回合的结果。
#[derive(Debug, Clone, PartialEq)]
pub struct RoundResult {
    pub round: i64,
    pub winner: Side,
    /// 你这边赢了没有（不知道你在哪边时 None）。
    pub won: Option<bool>,
    pub reason: &'static str,
    pub ct: i64,
    pub t: i64,
}

/// 这一回合的炸弹。
#[derive(Debug, Clone, PartialEq)]
struct Bomb {
    round: i64,
    planted_ms: u64,
    explodes_ms: u64,
    /// `planted` / `defused` / `exploded` / `over`（回合结束了还没炸）
    state: &'static str,
    ended_ms: Option<u64>,
    ten_sent: bool,
}

/// 正在打的这一张图。
#[derive(Debug, Clone)]
struct Track {
    key: String,
    map: String,
    mode: String,
    started_ms: u64,
    ct: i64,
    t: i64,
    rounds: Vec<RoundResult>,
    /// 正在打第几回合（冻结时间结束时记）
    current: Option<i64>,
    my_team: Option<Side>,
    stats: Option<MatchStats>,
    state: Option<PlayerState>,
    bomb: Option<Bomb>,
    halftimes: HashSet<i64>,
    /// 上一回合结束时哪边是赛点（连着几回合都是同一边的赛点只说一次）
    match_point: Option<Side>,
    died_in: Option<i64>,
    ended: bool,
}

impl Track {
    fn new(map: &str, mode: &str, now_ms: u64) -> Self {
        Self {
            key: format!("{map}-{now_ms}"),
            map: map.to_string(),
            mode: mode.to_string(),
            started_ms: now_ms,
            ct: 0,
            t: 0,
            rounds: Vec::new(),
            current: None,
            my_team: None,
            stats: None,
            state: None,
            bomb: None,
            halftimes: HashSet::new(),
            match_point: None,
            died_in: None,
            ended: false,
        }
    }

    fn score(&self, side: Side) -> i64 {
        match side {
            Side::Ct => self.ct,
            Side::T => self.t,
        }
    }

    /// 比分里你这边、对面（不知道你在哪边时 None）。
    fn mine(&self) -> Option<(i64, i64)> {
        self.my_team
            .map(|me| (self.score(me), self.score(me.other())))
    }

    /// 一行、一句话里的比分：知道你在哪边时“我们 7 : 5 对面”，不知道时“CT 7 : 5 T”。
    fn score_text(&self) -> String {
        match self.mine() {
            Some((mine, theirs)) => format!("我们 {mine} : {theirs} 对面"),
            None => format!("CT {} : {} T", self.ct, self.t),
        }
    }

    fn result(&self) -> &'static str {
        match self.mine() {
            Some((a, b)) if a > b => "win",
            Some((a, b)) if a < b => "loss",
            Some(_) => "tie",
            None => "",
        }
    }
}

/// 引擎。
#[derive(Debug, Default)]
pub struct Engine {
    prev: Option<Snapshot>,
    track: Option<Track>,
    /// 上一张打完的图（比赛结束卡片的数据）。
    last_match: Option<Value>,
    claimed: bool,
    last_ms: u64,
    /// 现在的阶段（游戏中心那边）。
    phase: Phase,
    native: String,
}

fn obj(v: Value) -> Map<String, Value> {
    match v {
        Value::Object(m) => m,
        _ => Map::new(),
    }
}

/// 游戏报的比分：总数没变少就照它（半场换边以后 CT / T 两边的比分跟着队换过来，不能按边取大的）。
fn sync_scores(track: &mut Track, ct: i64, t: i64) {
    if ct + t >= track.ct + track.t {
        track.ct = ct;
        track.t = t;
    }
}

/// 这份 POST 是哪个阶段。
pub fn phase_of(snap: &Snapshot) -> (Phase, String) {
    match &snap.map {
        None => (Phase::Menu, "menu".into()),
        Some(map) => {
            let phase = match map.phase.as_str() {
                "warmup" => Phase::PreMatch,
                "live" | "intermission" => Phase::InMatch,
                "gameover" => Phase::PostMatch,
                _ => Phase::Menu,
            };
            (phase, map.phase.clone())
        }
    }
}

impl Engine {
    /// 收到过数据、还没断（游戏中心里 `cs2` 归这里管）。
    pub fn live(&self) -> bool {
        self.claimed
    }

    /// 最后一次收到 POST 的时间（毫秒，0 = 没收到过）。
    pub fn last_ms(&self) -> u64 {
        self.last_ms
    }

    /// 喂一份（令牌已经对过的）POST。
    pub fn feed(&mut self, snap: Snapshot, now_ms: u64) -> Vec<Action> {
        let mut out = Vec::new();
        if !self.claimed {
            self.claimed = true;
            out.push(Action::Claim(true));
        }
        self.last_ms = now_ms;
        let prev = self.prev.take();
        self.follow_match(&snap, prev.as_ref(), now_ms, &mut out);
        let (phase, native) = phase_of(&snap);
        self.phase = phase;
        self.native = native.clone();
        out.push(Action::Observe(Observation {
            phase,
            native_phase: native,
            attrs: self.attrs(),
            source: SOURCE.into(),
        }));
        self.prev = Some(snap);
        out
    }

    /// 每 250 毫秒左右调一次：炸弹还剩 10 秒；断了就交回游戏中心。`cs2_running`：`cs2.exe` 还在不在。
    pub fn tick(&mut self, now_ms: u64, cs2_running: bool) -> Vec<Action> {
        let mut out = Vec::new();
        if let Some(track) = &mut self.track {
            if let Some(bomb) = &mut track.bomb {
                let due = bomb.explodes_ms.saturating_sub(TEN_MS);
                if bomb.state == "planted"
                    && !bomb.ten_sent
                    && now_ms >= due
                    && now_ms < bomb.explodes_ms
                {
                    bomb.ten_sent = true;
                    out.push(Action::Push(ten_push(&track.key, bomb.round)));
                }
            }
        }
        if self.claimed && now_ms.saturating_sub(self.last_ms) > STALE_MS {
            out.extend(self.release(cs2_running));
        }
        out
    }

    /// 交回游戏中心（断了、插件关了）：游戏还在就是“在跑”，不在就是“没在跑”。没打完的那张图不算结束、不推。
    pub fn release(&mut self, cs2_running: bool) -> Vec<Action> {
        if !self.claimed {
            return Vec::new();
        }
        self.claimed = false;
        self.prev = None;
        self.track = None;
        self.phase = Phase::Stopped;
        self.native.clear();
        vec![
            Action::Observe(Observation {
                phase: if cs2_running {
                    Phase::Running
                } else {
                    Phase::Stopped
                },
                source: SOURCE.into(),
                ..Observation::default()
            }),
            Action::Claim(false),
        ]
    }

    fn attrs(&self) -> Map<String, Value> {
        let mut attrs = Map::new();
        if let Some(t) = &self.track {
            attrs.insert("cs2.map".into(), json!(t.map));
            attrs.insert("cs2.mode".into(), json!(t.mode));
            attrs.insert("cs2.ctScore".into(), json!(t.ct));
            attrs.insert("cs2.tScore".into(), json!(t.t));
            if let Some(round) = t.current {
                attrs.insert("cs2.round".into(), json!(round));
            }
        }
        attrs
    }

    fn follow_match(
        &mut self,
        snap: &Snapshot,
        prev: Option<&Snapshot>,
        now_ms: u64,
        out: &mut Vec<Action>,
    ) {
        let Some(map) = &snap.map else {
            // 回到菜单：没打完的这张图就这样了（不推结束）
            self.track = None;
            return;
        };
        let fresh = match &self.track {
            None => true,
            Some(t) => {
                t.map != map.name
                    || (t.ended && map.phase != "gameover")
                    || (map.ct_score + map.t_score == 0
                        && !t.rounds.is_empty()
                        && matches!(map.phase.as_str(), "warmup" | "live"))
            }
        };
        // 新的一张图：之前的那份不算“前一份”（不拿上一张的炸弹、回合当变化）
        let prev = if fresh {
            let mut track = Track::new(&map.name, &map.mode, now_ms);
            track.ct = map.ct_score;
            track.t = map.t_score;
            // 刚连上时已经在打了：比分里的回合数就是打过的
            if map.phase == "live" && snap.round_phase == "live" {
                track.current = Some(map.ct_score + map.t_score + 1);
            }
            self.track = Some(track);
            // 第一次连上、换了一张图：只记底
            None
        } else {
            prev
        };
        let track = self.track.as_mut().expect("上面刚放进去");
        track.mode = map.mode.clone();

        // 你在哪边、你的数据：只在 player 是自己时更新；死了在看队友时，哪边照队友的（竞技里只能看队友）
        if snap.is_me() {
            if snap.player_team.is_some() {
                track.my_team = snap.player_team;
            }
            if snap.stats.is_some() {
                track.stats = snap.stats;
            }
            if snap.state.is_some() {
                track.state = snap.state.clone();
            }
        } else if track.my_team.is_none() {
            track.my_team = snap.player_team;
        }

        let Some(prev) = prev else {
            // 只记底
            sync_scores(track, map.ct_score, map.t_score);
            return;
        };
        let prev_map_phase = prev.map.as_ref().map(|m| m.phase.as_str()).unwrap_or("");
        let live = map.phase == "live";

        // 阵亡：上一份是活着的自己，这一份是 0 血的自己，或者已经换成在看别人（死了马上切过去）
        let was_alive_me = prev.is_me() && prev.state.as_ref().is_some_and(|s| s.health > 0);
        let dead_now = if snap.is_me() {
            snap.state.as_ref().is_some_and(|s| s.health <= 0)
        } else {
            !snap.player.is_empty() && snap.round_phase == "live"
        };
        if live && was_alive_me && dead_now {
            let round = track.current.unwrap_or(track.ct + track.t + 1);
            if track.died_in != Some(round) {
                track.died_in = Some(round);
                out.push(Action::Event {
                    kind: EV_MY_DEATH,
                    data: obj(json!({"round": round})),
                });
            }
        }

        // 回合开始：冻结时间结束
        if live && prev.round_phase != "live" && snap.round_phase == "live" {
            let round = (map.ct_score + map.t_score).max(track.ct + track.t) + 1;
            track.current = Some(round);
            track.bomb = None;
            let mut data = obj(json!({
                "round": round,
                "ctScore": track.ct,
                "tScore": track.t,
                "myTeam": track.my_team.map(Side::as_str).unwrap_or(""),
            }));
            data.retain(|_, v| !v.is_null());
            out.push(Action::Event {
                kind: EV_ROUND_STARTED,
                data,
            });
        }

        // 炸弹：下了 / 拆了 / 炸了
        let round_now = track.current.unwrap_or(track.ct + track.t + 1);
        if prev.round_bomb != "planted" && snap.round_bomb == "planted" {
            let seconds = snap
                .countdown
                .as_ref()
                .filter(|(phase, _)| phase == "bomb")
                .map(|(_, left)| *left)
                .unwrap_or(C4_SECONDS);
            let bomb = Bomb {
                round: round_now,
                planted_ms: now_ms,
                explodes_ms: now_ms + (seconds * 1000.0).round().max(0.0) as u64,
                state: "planted",
                ended_ms: None,
                ten_sent: false,
            };
            out.push(Action::Event {
                kind: EV_BOMB_PLANTED,
                data: obj(json!({"round": round_now, "seconds": seconds.round() as i64})),
            });
            out.push(Action::Push(bomb_push(track, &bomb)));
            track.bomb = Some(bomb);
        } else if snap.round_bomb == "planted" {
            // 观战时游戏给倒数：差得多就照它改
            if let (Some(bomb), Some((phase, left))) = (&mut track.bomb, &snap.countdown) {
                let explodes = now_ms + (left * 1000.0).round().max(0.0) as u64;
                if phase == "bomb"
                    && bomb.state == "planted"
                    && explodes.abs_diff(bomb.explodes_ms) > 2_000
                {
                    bomb.explodes_ms = explodes;
                    let bomb = bomb.clone();
                    out.push(Action::Push(bomb_push(track, &bomb)));
                }
            }
        }
        for (state, kind) in [("defused", EV_BOMB_DEFUSED), ("exploded", EV_BOMB_EXPLODED)] {
            if prev.round_bomb != state && snap.round_bomb == state {
                let round = track.bomb.as_ref().map_or(round_now, |b| b.round);
                out.push(Action::Event {
                    kind,
                    data: obj(json!({"round": round})),
                });
                if let Some(bomb) = track.bomb.as_mut().filter(|b| b.state == "planted") {
                    bomb.state = if state == "defused" {
                        "defused"
                    } else {
                        "exploded"
                    };
                    bomb.ended_ms = Some(now_ms);
                    let bomb = bomb.clone();
                    out.push(Action::Push(bomb_push(track, &bomb)));
                }
            }
        }

        // 回合结束
        if prev.round_phase != "over" && snap.round_phase == "over" {
            // 没写谁赢：看比分谁多了
            let by_score = if map.ct_score > track.ct {
                Some(Side::Ct)
            } else if map.t_score > track.t {
                Some(Side::T)
            } else {
                None
            };
            let winner = snap.round_winner.or(by_score);
            if let Some(winner) = winner {
                let round = track.current.unwrap_or(track.ct + track.t + 1);
                let (mut ct, mut t) = (track.ct, track.t);
                match winner {
                    Side::Ct => ct += 1,
                    Side::T => t += 1,
                }
                // 游戏报的比分已经算上这回合了就用游戏的
                if map.ct_score + map.t_score >= ct + t {
                    ct = map.ct_score;
                    t = map.t_score;
                }
                track.ct = ct;
                track.t = t;
                let raw = map
                    .round_wins
                    .iter()
                    .find(|(n, _)| *n as i64 == round)
                    .map(|(_, r)| rules::win_reason(r))
                    .unwrap_or("");
                let reason = match (raw, snap.round_bomb.as_str()) {
                    ("", "exploded") => "bomb",
                    ("", "defused") => "defuse",
                    (r, _) => r,
                };
                let won = track.my_team.map(|me| me == winner);
                track.rounds.retain(|r| r.round != round);
                track.rounds.push(RoundResult {
                    round,
                    winner,
                    won,
                    reason,
                    ct,
                    t,
                });
                let mut data = obj(json!({
                    "round": round,
                    "winner": winner.as_str(),
                    "reason": reason,
                    "ctScore": ct,
                    "tScore": t,
                }));
                if let (Some(won), Some((mine, theirs))) = (won, track.mine()) {
                    data.insert("won".into(), json!(won));
                    data.insert("myScore".into(), json!(mine));
                    data.insert("enemyScore".into(), json!(theirs));
                }
                out.push(Action::Event {
                    kind: EV_ROUND_ENDED,
                    data,
                });
                // 回合结束了包还没炸（CT 全灭）：卡片写回合结束
                if let Some(bomb) = track.bomb.as_mut().filter(|b| b.state == "planted") {
                    bomb.state = "over";
                    bomb.ended_ms = Some(now_ms);
                    let bomb = bomb.clone();
                    out.push(Action::Push(bomb_push(track, &bomb)));
                }
                // 赛点（这张图还没完）：一边刚到赛点时说一次，连着几回合都是它的赛点不再说
                if map.phase == "live" {
                    let side = rules::rules(&track.mode).and_then(|r| rules::match_point(r, ct, t));
                    let changed = side.is_some() && side != track.match_point;
                    track.match_point = side;
                    if let (true, Some(side)) = (changed, side) {
                        let ours = track.my_team.map(|me| me == side);
                        let mut data = obj(json!({
                            "team": side.as_str(),
                            "ctScore": ct,
                            "tScore": t,
                        }));
                        if let Some(ours) = ours {
                            data.insert("ours".into(), json!(ours));
                        }
                        out.push(Action::Event {
                            kind: EV_MATCH_POINT,
                            data,
                        });
                        out.push(Action::Push(match_point_push(track, side, ours)));
                    }
                }
            }
        }
        // 游戏后来补上的比分（不在回合结束那一份里）；半场换边以后两边的比分跟着队换过来
        sync_scores(track, map.ct_score, map.t_score);
        // round_wins 后来才写上的：补上怎么赢的
        for r in track.rounds.iter_mut().filter(|r| r.reason.is_empty()) {
            if let Some((_, raw)) = map.round_wins.iter().find(|(n, _)| *n as i64 == r.round) {
                r.reason = rules::win_reason(raw);
            }
        }

        // 半场换边
        if prev_map_phase != "intermission" && map.phase == "intermission" {
            let total = track.ct + track.t;
            if track.halftimes.insert(total) {
                let mut data = obj(json!({"ctScore": track.ct, "tScore": track.t}));
                if let Some((mine, theirs)) = track.mine() {
                    data.insert("myScore".into(), json!(mine));
                    data.insert("enemyScore".into(), json!(theirs));
                }
                out.push(Action::Event {
                    kind: EV_HALFTIME,
                    data,
                });
                out.push(Action::Push(halftime_push(track)));
            }
        }

        // 比赛结束
        if prev_map_phase != "gameover" && map.phase == "gameover" && !track.ended {
            track.ended = true;
            let stats = track.stats.unwrap_or_default();
            let mut data = obj(json!({
                "map": track.map,
                "mode": track.mode,
                "ctScore": track.ct,
                "tScore": track.t,
                "result": track.result(),
                "kills": stats.kills,
                "deaths": stats.deaths,
                "assists": stats.assists,
                "mvps": stats.mvps,
                "score": stats.score,
            }));
            if let Some((mine, theirs)) = track.mine() {
                data.insert("myScore".into(), json!(mine));
                data.insert("enemyScore".into(), json!(theirs));
            }
            out.push(Action::Event {
                kind: EV_MATCH_ENDED,
                data,
            });
            let card = match_data(track, now_ms, true);
            self.last_match = Some(card.clone());
            let track = self.track.as_ref().expect("还在");
            out.push(Action::Push(Push {
                trigger: PUSH_MATCH_END.into(),
                key: format!("{PUSH_MATCH_END}:{}", track.key),
                card: Some(PushCard {
                    title: "比赛结束".into(),
                    uri: PAGE_MATCH.into(),
                    params: Value::Null,
                    data: card,
                }),
                note: end_note(track),
                ..Push::default()
            }));
        }
    }

    /// 现在这一局给页面的数据（`cs2.state` 的 `structuredContent`）：没在打时 `playing: false`，带上一张打完的。
    pub fn match_view(&self, now_ms: u64) -> Value {
        match &self.track {
            Some(track) if self.claimed => {
                let mut v = match_data(track, now_ms, track.ended);
                v["phase"] = json!(self.native);
                v["phaseName"] = json!(self.phase.label());
                v["updatedAtMs"] = json!(self.last_ms);
                v
            }
            _ => json!({
                "playing": false,
                "connected": self.claimed,
                "phase": if self.claimed { self.native.as_str() } else { "" },
                "updatedAtMs": self.last_ms,
                "lastMatch": self.last_match,
            }),
        }
    }

    /// 给她的一份（`cs2.state` 的 `content`）：短、中文、只有事实。`off` = 收不到数据时为什么（插件给的一句）。
    pub fn state_for_her(&self, now_ms: u64, off: &str) -> Value {
        if !self.claimed {
            let mut v = json!({"status": "no_data", "text": off});
            if let Some(last) = &self.last_match {
                v["lastMatch"] = brief(last);
            }
            return v;
        }
        let Some(track) = &self.track else {
            let mut v = json!({"status": "menu", "text": "CS2 开着，在主菜单，没在对局里"});
            if let Some(last) = &self.last_match {
                v["lastMatch"] = brief(last);
            }
            return v;
        };
        let mut v = json!({
            "status": "ok",
            "map": rules::map_label(&track.map),
            "mode": rules::mode_label(&track.mode),
            "phase": self.phase.label(),
            "score": track.score_text(),
            "secondsSinceUpdate": now_ms.saturating_sub(self.last_ms) / 1000,
        });
        if self.native == "intermission" {
            v["phase"] = json!("半场换边");
        }
        if let Some(round) = track.current {
            v["round"] = json!(round);
        }
        if let Some(me) = track.my_team {
            v["myTeam"] = json!(me.as_str());
        }
        if let Some(s) = &track.stats {
            v["myStats"] = json!({"kills": s.kills, "deaths": s.deaths, "assists": s.assists, "mvps": s.mvps, "score": s.score});
        }
        if let Some(s) = &track.state {
            v["me"] = json!({"health": s.health, "armor": s.armor, "helmet": s.helmet, "money": s.money, "roundKills": s.round_kills});
        }
        if let Some(bomb) = &track.bomb {
            v["bomb"] = json!(match bomb.state {
                "planted" => {
                    let left = bomb.explodes_ms.saturating_sub(now_ms) as f64 / 1000.0;
                    format!(
                        "第 {} 回合下了包，约 {:.0} 秒后爆炸",
                        bomb.round,
                        left.ceil()
                    )
                }
                "defused" => format!("第 {} 回合的包拆了", bomb.round),
                "exploded" => format!("第 {} 回合的包炸了", bomb.round),
                _ => format!("第 {} 回合下了包，回合已经结束", bomb.round),
            });
        }
        if track.ended {
            v["result"] = json!(result_label(track.result()));
        }
        v
    }

    /// 这张图每回合的结果（`cs2.rounds`）。
    pub fn rounds_for_her(&self, off: &str) -> Value {
        let Some(track) = self.track.as_ref().filter(|_| self.claimed) else {
            return json!({"status": "no_data", "text": if self.claimed { "CS2 开着，在主菜单，没在对局里" } else { off }});
        };
        json!({
            "status": "ok",
            "map": rules::map_label(&track.map),
            "score": track.score_text(),
            "myTeam": track.my_team.map(Side::as_str),
            "rounds": rounds_json(track),
        })
    }
}

fn result_label(result: &str) -> &'static str {
    match result {
        "win" => "赢了",
        "loss" => "输了",
        "tie" => "平了",
        _ => "",
    }
}

/// 上一张打完的图，给她的一句。
fn brief(card: &Value) -> Value {
    json!({
        "map": card["mapName"],
        "score": card["scoreText"],
        "result": result_label(card["result"].as_str().unwrap_or("")),
    })
}

fn rounds_json(track: &Track) -> Vec<Value> {
    track
        .rounds
        .iter()
        .map(|r| {
            let mut v = json!({"round": r.round, "winner": r.winner.as_str(), "reason": r.reason, "ctScore": r.ct, "tScore": r.t});
            if let Some(won) = r.won {
                v["won"] = json!(won);
            }
            v
        })
        .collect()
}

/// 这一张图给页面、卡片的数据。
fn match_data(track: &Track, now_ms: u64, over: bool) -> Value {
    let mut v = json!({
        "playing": true,
        "over": over,
        "map": track.map,
        "mapName": rules::map_name(&track.map),
        "mode": track.mode,
        "modeName": rules::mode_name(&track.mode),
        "startedAtMs": track.started_ms,
        "atMs": now_ms,
        "ctScore": track.ct,
        "tScore": track.t,
        "scoreText": track.score_text(),
        "result": if over { track.result() } else { "" },
        "rounds": rounds_json(track),
    });
    if let Some(round) = track.current {
        v["round"] = json!(round);
    }
    if let Some(me) = track.my_team {
        v["myTeam"] = json!(me.as_str());
        let (mine, theirs) = track.mine().unwrap_or_default();
        v["myScore"] = json!(mine);
        v["enemyScore"] = json!(theirs);
    }
    if let Some(s) = &track.stats {
        v["stats"] = json!({"kills": s.kills, "deaths": s.deaths, "assists": s.assists, "mvps": s.mvps, "score": s.score});
    }
    if let Some(s) = &track.state {
        v["me"] = json!({"health": s.health, "armor": s.armor, "helmet": s.helmet, "money": s.money, "roundKills": s.round_kills, "equipValue": s.equip_value});
    }
    if let Some(b) = &track.bomb {
        v["bomb"] = bomb_data(track, b);
    }
    v
}

fn bomb_data(track: &Track, bomb: &Bomb) -> Value {
    let mut v = json!({
        "round": bomb.round,
        "state": bomb.state,
        "plantedAtMs": bomb.planted_ms,
        "explodesAtMs": bomb.explodes_ms,
        "map": track.map,
        "mapName": rules::map_name(&track.map),
    });
    if let Some(at) = bomb.ended_ms {
        v["endedAtMs"] = json!(at);
    }
    v
}

fn bomb_push(track: &Track, bomb: &Bomb) -> Push {
    Push {
        trigger: PUSH_BOMB.into(),
        key: format!("{PUSH_BOMB}:{}:{}", track.key, bomb.round),
        card: Some(PushCard {
            title: "下包倒数".into(),
            uri: PAGE_BOMB.into(),
            params: Value::Null,
            data: bomb_data(track, bomb),
        }),
        ..Push::default()
    }
}

fn ten_push(key: &str, round: i64) -> Push {
    Push {
        trigger: PUSH_BOMB_TEN.into(),
        key: format!("{PUSH_BOMB_TEN}:{key}:{round}"),
        line: Some(PushLine {
            cells: vec![PushCell {
                text: "炸弹还剩 10 秒".into(),
                tone: None,
            }],
        }),
        voice: vec!["包还有十秒".into(), "还剩十秒".into(), "十秒后爆炸".into()],
        ..Push::default()
    }
}

fn match_point_push(track: &Track, side: Side, ours: Option<bool>) -> Push {
    let (voice, tone): (Vec<String>, Option<PushTone>) = match ours {
        Some(true) => (
            vec!["我们赛点了".into(), "我们拿到赛点了".into()],
            Some(PushTone::Good),
        ),
        Some(false) => (
            vec!["对面赛点了".into(), "对面拿到赛点了".into()],
            Some(PushTone::Bad),
        ),
        None => (vec![format!("{} 赛点了", side.as_str())], None),
    };
    Push {
        trigger: PUSH_MATCH_POINT.into(),
        key: format!("{PUSH_MATCH_POINT}:{}:{}-{}", track.key, track.ct, track.t),
        line: Some(PushLine {
            cells: vec![PushCell {
                text: format!("赛点 · {}", track.score_text()),
                tone,
            }],
        }),
        voice,
        ..Push::default()
    }
}

fn halftime_push(track: &Track) -> Push {
    let numbers = match track.mine() {
        Some((mine, theirs)) => format!("{mine} 比 {theirs}"),
        None => format!("CT {} 比 {}", track.ct, track.t),
    };
    Push {
        trigger: PUSH_HALFTIME.into(),
        key: format!("{PUSH_HALFTIME}:{}:{}", track.key, track.ct + track.t),
        line: Some(PushLine {
            cells: vec![PushCell {
                text: format!("半场 · {}", track.score_text()),
                tone: None,
            }],
        }),
        voice: vec![format!("半场了，{numbers}"), format!("半场换边，{numbers}")],
        ..Push::default()
    }
}

/// 比赛结束给她的一句（只说事实）。
fn end_note(track: &Track) -> String {
    let mut note = format!(
        "CS2 这张图打完了：{}（{}）{}",
        rules::map_label(&track.map),
        rules::mode_label(&track.mode),
        track.score_text()
    );
    match result_label(track.result()) {
        "" => {}
        r => note.push_str(&format!("，{r}")),
    }
    if let Some(s) = &track.stats {
        note.push_str(&format!(
            "；玩家 {} 杀 {} 死 {} 助攻，MVP {} 次",
            s.kills, s.deaths, s.assists, s.mvps
        ));
    }
    note.push('。');
    note
}

#[cfg(test)]
#[path = "engine_tests.rs"]
mod tests;
