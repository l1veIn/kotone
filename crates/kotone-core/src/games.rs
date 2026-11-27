//! 游戏状态和事件（ADR 036）：所有游戏共用的阶段、状态、事件信封；把适配器的观察变成事件的 [`Hub`]；
//! 自动化的事件触发（`on`）、它的校验和匹配；`for` / 每局次数 / 冷却 / 每局叫她几次的记账（[`Triggers`]）。
//!
//! 这里只有纯数据和纯函数，谁来看进程、读接口（适配器）、谁来分发（游戏中心）在桌面端。
//! 照搬的做法：状态和事件分开（Overwolf GEP）；`from` / `to` / `for`（Home Assistant）；
//! 事件数据的子集匹配、数组表示“或”（AWS EventBridge）；事件信封的字段（CloudEvents）。

use std::collections::{BTreeMap, HashMap, VecDeque};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// 所有游戏共用的阶段（D3）。游戏自己的叫法放在 `nativePhase`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// 没在跑。
    #[default]
    Stopped,
    /// 在跑，不知道细节（没有适配器的游戏一直停在这）。
    Running,
    /// 大厅、主界面。
    Menu,
    /// 排队、确认。
    Queue,
    /// 选英雄、选角色、热身。
    PreMatch,
    /// 加载。
    Loading,
    /// 对局中。
    InMatch,
    /// 结算。
    PostMatch,
}

impl Phase {
    pub const ALL: [Phase; 8] = [
        Phase::Stopped,
        Phase::Running,
        Phase::Menu,
        Phase::Queue,
        Phase::PreMatch,
        Phase::Loading,
        Phase::InMatch,
        Phase::PostMatch,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Phase::Stopped => "stopped",
            Phase::Running => "running",
            Phase::Menu => "menu",
            Phase::Queue => "queue",
            Phase::PreMatch => "pre_match",
            Phase::Loading => "loading",
            Phase::InMatch => "in_match",
            Phase::PostMatch => "post_match",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Phase::Stopped => "没在跑",
            Phase::Running => "在跑",
            Phase::Menu => "大厅",
            Phase::Queue => "排队",
            Phase::PreMatch => "选英雄",
            Phase::Loading => "加载",
            Phase::InMatch => "对局中",
            Phase::PostMatch => "结算",
        }
    }

    /// 算不算在“一局”里（D3）：选英雄、加载、对局中；没有适配器的游戏在跑就算（进程从出现到退出算一局）。
    pub fn in_match(self) -> bool {
        matches!(
            self,
            Phase::Running | Phase::PreMatch | Phase::Loading | Phase::InMatch
        )
    }
}

/// 一个在跑的游戏现在的状态（D1）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameState {
    /// 游戏配置 id。
    pub game: String,
    pub phase: Phase,
    /// 游戏自己的阶段（英雄联盟的 `InProgress`……），没有为空。
    #[serde(default)]
    pub native_phase: String,
    /// 这一局的编号（Kotone 自己起的，一局里不变；游戏自己的对局 id 在 `attrs` 里）。不在一局里为空。
    #[serde(rename = "match", default)]
    pub match_id: String,
    /// 进入现在这个阶段的时间（毫秒）。
    pub since: u64,
    /// 游戏开始跑的时间（毫秒）。
    pub started: u64,
    /// 这一局开始的时间（毫秒），不在一局里为 None。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub match_started: Option<u64>,
    /// 游戏窗口在不在前台。
    #[serde(default)]
    pub focused: bool,
    /// 各游戏自己的状态，键名以适配器 id 开头（`lol.gameTime`）。
    #[serde(default)]
    pub attrs: Map<String, Value>,
}

/// 一件事（D2）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameEvent {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub game: String,
    #[serde(rename = "match", default)]
    pub match_id: String,
    /// 发生时间（毫秒）。
    pub time: u64,
    /// 游戏里的时间（秒），有的话。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_time: Option<f64>,
    /// 谁报的：`process`、`lol/client`、`lol/live-client`……
    pub source: String,
    #[serde(default)]
    pub data: Map<String, Value>,
}

/// 适配器每一轮对一个游戏的观察。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Observation {
    pub phase: Phase,
    pub native_phase: String,
    pub attrs: Map<String, Value>,
    /// 谁报的（写进由它引起的事件）。
    pub source: String,
}

/// 每个游戏最多留多少件最近的事。
pub const RECENT_EVENTS: usize = 50;

/// 把适配器的观察合成状态、变化时发事件（游戏中心的纯逻辑部分，D5）。
#[derive(Debug, Default)]
pub struct Hub {
    states: BTreeMap<String, GameState>,
    recent: HashMap<String, VecDeque<GameEvent>>,
    next_id: u64,
}

fn data(pairs: &[(&str, Value)]) -> Map<String, Value> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect()
}

impl Hub {
    /// 现在在跑的游戏（按游戏配置 id 排）。
    pub fn states(&self) -> Vec<GameState> {
        self.states.values().cloned().collect()
    }

    pub fn state(&self, game: &str) -> Option<&GameState> {
        self.states.get(game)
    }

    /// 这个游戏最近的事，旧的在前，最多 `limit` 件。
    pub fn recent(&self, game: &str, limit: usize) -> Vec<GameEvent> {
        let Some(list) = self.recent.get(game) else {
            return Vec::new();
        };
        list.iter()
            .skip(list.len().saturating_sub(limit))
            .cloned()
            .collect()
    }

    fn event(
        &mut self,
        game: &str,
        kind: &str,
        source: &str,
        data: Map<String, Value>,
        game_time: Option<f64>,
        now: u64,
    ) -> GameEvent {
        self.next_id += 1;
        let event = GameEvent {
            id: format!("{game}-{}", self.next_id),
            kind: kind.to_string(),
            game: game.to_string(),
            match_id: self
                .states
                .get(game)
                .map(|s| s.match_id.clone())
                .unwrap_or_default(),
            time: now,
            game_time,
            source: source.to_string(),
            data,
        };
        let list = self.recent.entry(game.to_string()).or_default();
        list.push_back(event.clone());
        while list.len() > RECENT_EVENTS {
            list.pop_front();
        }
        event
    }

    /// 一个游戏这一轮的样子；返回由此发生的事（启动、阶段变了、一局开始 / 结束、退出）。
    /// 阶段没变时只更新状态，不发事件（值没变不通知）。
    pub fn observe(&mut self, game: &str, seen: Observation, now: u64) -> Vec<GameEvent> {
        let before = self.states.get(game).map(|s| s.phase).unwrap_or_default();
        let after = seen.phase;
        let source = if seen.source.is_empty() {
            "process".to_string()
        } else {
            seen.source.clone()
        };
        let mut out = Vec::new();
        if before == after {
            if let Some(state) = self.states.get_mut(game) {
                state.native_phase = seen.native_phase;
                state.attrs = seen.attrs;
            }
            return out;
        }
        if before == Phase::Stopped {
            self.states.insert(
                game.to_string(),
                GameState {
                    game: game.to_string(),
                    phase: Phase::Stopped,
                    native_phase: String::new(),
                    match_id: String::new(),
                    since: now,
                    started: now,
                    match_started: None,
                    focused: false,
                    attrs: Map::new(),
                },
            );
            out.push(self.event(game, "game.started", &source, Map::new(), None, now));
        }
        if before.in_match() && !after.in_match() {
            let started = self
                .states
                .get(game)
                .and_then(|s| s.match_started)
                .unwrap_or(now);
            let seconds = now.saturating_sub(started) / 1000;
            out.push(self.event(
                game,
                "game.match_ended",
                &source,
                data(&[("elapsedSeconds", seconds.into())]),
                None,
                now,
            ));
        }
        if let Some(state) = self.states.get_mut(game) {
            state.phase = after;
            state.native_phase = seen.native_phase.clone();
            state.since = now;
            state.attrs = seen.attrs;
            if !before.in_match() && after.in_match() {
                state.match_started = Some(now);
                state.match_id = format!("{game}-{now}");
            } else if !after.in_match() {
                state.match_started = None;
                state.match_id.clear();
            }
        }
        out.push(self.event(
            game,
            "game.phase_changed",
            &source,
            data(&[
                ("from", before.as_str().into()),
                ("to", after.as_str().into()),
                ("native", seen.native_phase.into()),
            ]),
            None,
            now,
        ));
        if !before.in_match() && after.in_match() {
            out.push(self.event(game, "game.match_started", &source, Map::new(), None, now));
        }
        if after == Phase::Stopped {
            let started = self.states.get(game).map(|s| s.started).unwrap_or(now);
            let seconds = now.saturating_sub(started) / 1000;
            out.push(self.event(
                game,
                "game.stopped",
                &source,
                data(&[("elapsedSeconds", seconds.into())]),
                None,
                now,
            ));
            self.states.remove(game);
        }
        out
    }

    /// 前台是哪个游戏（None = 不是游戏）；返回切进 / 切出的事。
    pub fn focus(&mut self, game: Option<&str>, now: u64) -> Vec<GameEvent> {
        let changes: Vec<(String, bool)> = self
            .states
            .values()
            .filter(|s| s.focused != (Some(s.game.as_str()) == game))
            .map(|s| (s.game.clone(), !s.focused))
            .collect();
        let mut out = Vec::new();
        for (id, focused) in changes {
            if let Some(state) = self.states.get_mut(&id) {
                state.focused = focused;
            }
            let kind = if focused {
                "game.focused"
            } else {
                "game.unfocused"
            };
            out.push(self.event(&id, kind, "process", Map::new(), None, now));
        }
        out
    }

    /// 游戏专用的事（`<适配器>.<名字>`，插件在清单里登记过的），由适配器报；游戏没在跑时不收。
    pub fn push(
        &mut self,
        game: &str,
        kind: &str,
        source: &str,
        data: Map<String, Value>,
        game_time: Option<f64>,
        now: u64,
    ) -> Option<GameEvent> {
        if !self.states.contains_key(game) {
            return None;
        }
        Some(self.event(game, kind, source, data, game_time, now))
    }

    /// 第三方插件的事（ADR 047 D5，`<插件 id>.<名字>`，宿主核对过是它清单里登记的）：**只交给自动化**，
    /// 不记进最近几件事（她看的 `game.state` 从那里取）；游戏没在跑也收（`games` 为空的插件 `game` 是空的；
    /// 游戏刚退出的宽限期里还会来）。在跑的话带上这一局的编号。
    pub fn outside_event(
        &mut self,
        game: &str,
        kind: &str,
        source: &str,
        data: Map<String, Value>,
        now: u64,
    ) -> GameEvent {
        self.next_id += 1;
        GameEvent {
            id: format!(
                "{}-{}",
                if game.is_empty() { "third-party" } else { game },
                self.next_id
            ),
            kind: kind.to_string(),
            game: game.to_string(),
            match_id: self
                .states
                .get(game)
                .map(|s| s.match_id.clone())
                .unwrap_or_default(),
            time: now,
            game_time: None,
            source: source.to_string(),
            data,
        }
    }
}

// ---------- 能给的事（目录，D4） ----------

mod catalog;
pub use catalog::{
    generic_events, CatalogEntry, EventCatalog, EventSpec, GENERIC_ADAPTER, GENERIC_SCOPE,
};

// ---------- 自动化的事件触发（D6） ----------

/// 同一个自动化还在跑时又触发了怎么办（照 Home Assistant 的 mode）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum RunMode {
    /// 正在跑就不管新的（默认）。
    #[default]
    Single,
    /// 停掉正在跑的，重新开始。
    Restart,
    /// 排队，跑完一个再跑下一个。
    Queued,
}

/// 自动化的 `on`：哪些事发生时运行。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EventTrigger {
    /// 一个或几个事件类型（“或”），也可以写前缀 `lol.*`。
    pub event: Vec<String>,
    /// 只在这个游戏配置上触发；不写 = 任何游戏。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game: Option<String>,
    /// 对事件的 `data` 做子集匹配：没写的字段不管，数组表示“或”。
    #[serde(rename = "match", default, skip_serializing_if = "Map::is_empty")]
    pub matches: Map<String, Value>,
    /// 阶段进入后保持这么多秒才触发（只用于 `game.phase_changed`）。
    #[serde(rename = "for", default)]
    pub for_seconds: u32,
    #[serde(default)]
    pub mode: RunMode,
    /// 一局最多一次。
    #[serde(default)]
    pub once_per_match: bool,
    /// 两次触发之间至少隔多少秒。
    #[serde(default)]
    pub cooldown_seconds: u32,
    #[serde(default = "yes")]
    pub enabled: bool,
}

fn yes() -> bool {
    true
}

pub const MAX_TRIGGER_EVENTS: usize = 8;
pub const MAX_MATCH_KEYS: usize = 16;
pub const MAX_FOR_SECONDS: u32 = 3600;
pub const MAX_COOLDOWN_SECONDS: u32 = 86_400;

fn scalar(v: &Value) -> bool {
    matches!(v, Value::String(_) | Value::Number(_) | Value::Bool(_))
}

/// 保存前检查并整理（去空白）。事件名按合并后的目录 `events` 认（本体的 `game.*` 加插件登记的，插件关着的也认得；
/// 现在不会发生的由 [`EventCatalog::notes`] 提醒，不算错）。
pub fn validate_trigger(
    trigger: &EventTrigger,
    events: &EventCatalog,
) -> Result<EventTrigger, String> {
    let mut out = trigger.clone();
    out.event = trigger
        .event
        .iter()
        .map(|e| e.trim().to_string())
        .filter(|e| !e.is_empty())
        .collect();
    if out.event.is_empty() {
        return Err("on.event 至少要写一种游戏事件".into());
    }
    if out.event.len() > MAX_TRIGGER_EVENTS {
        return Err(format!("on.event 最多 {MAX_TRIGGER_EVENTS} 种"));
    }
    if let Some(bad) = out.event.iter().find(|e| !events.knows(e)) {
        return Err(format!(
            "没有“{bad}”这种游戏事件（用 game.state 看每个游戏能给哪些事）"
        ));
    }
    out.game = trigger
        .game
        .as_deref()
        .map(str::trim)
        .filter(|g| !g.is_empty())
        .map(str::to_string);
    if out.game.as_ref().is_some_and(|g| g.chars().count() > 64) {
        return Err("on.game 太长".into());
    }
    if out.matches.len() > MAX_MATCH_KEYS {
        return Err(format!("on.match 最多 {MAX_MATCH_KEYS} 个字段"));
    }
    for (key, value) in &out.matches {
        let ok = key.chars().count() <= 32
            && match value {
                Value::Array(list) => {
                    !list.is_empty() && list.len() <= 16 && list.iter().all(scalar)
                }
                v => scalar(v),
            };
        if !ok {
            return Err(format!(
                "on.match.{key} 只能是文字、数字、true / false，或它们的数组（表示“或”）"
            ));
        }
    }
    if out.for_seconds > MAX_FOR_SECONDS {
        return Err(format!("on.for 最多 {MAX_FOR_SECONDS} 秒"));
    }
    if out.for_seconds > 0 && out.event.iter().any(|e| e != "game.phase_changed") {
        return Err("on.for 只能和 game.phase_changed 一起用（阶段保持多久）".into());
    }
    if out.cooldown_seconds > MAX_COOLDOWN_SECONDS {
        return Err(format!("on.cooldownSeconds 最多 {MAX_COOLDOWN_SECONDS} 秒"));
    }
    Ok(out)
}

fn kind_matches(pattern: &str, kind: &str) -> bool {
    match pattern.strip_suffix(".*") {
        Some(prefix) => kind
            .strip_prefix(prefix)
            .is_some_and(|rest| rest.starts_with('.')),
        None => pattern == kind,
    }
}

fn value_matches(want: &Value, got: Option<&Value>) -> bool {
    let Some(got) = got else {
        return false;
    };
    match want {
        Value::Array(any) => any.iter().any(|w| value_matches(w, Some(got))),
        // 数字按数值比（1 和 1.0 一样）
        Value::Number(w) => got.as_f64().is_some_and(|g| w.as_f64() == Some(g)),
        w => w == got,
    }
}

/// 这件事对得上这个触发吗（不看 `for`、次数、冷却，那些见 [`Triggers`]）。
pub fn trigger_matches(trigger: &EventTrigger, event: &GameEvent) -> bool {
    trigger.enabled
        && trigger.event.iter().any(|p| kind_matches(p, &event.kind))
        && trigger.game.as_deref().map_or(true, |g| g == event.game)
        && trigger
            .matches
            .iter()
            .all(|(k, want)| value_matches(want, event.data.get(k)))
}

/// 一个自动化的事件触发。每局叫她几次不挂在这里，看程序里的 `ask()`（[`Triggers::take_ask`]，ADR 045 D2）。
#[derive(Debug, Clone, PartialEq)]
pub struct Armed<'a> {
    pub id: &'a str,
    pub trigger: &'a EventTrigger,
}

/// 该运行了：哪个自动化、因为哪件事。
#[derive(Debug, Clone, PartialEq)]
pub struct Fire {
    pub automation: String,
    pub event: GameEvent,
}

#[derive(Debug, Clone)]
struct Waiting {
    automation: String,
    event: GameEvent,
    due: u64,
}

/// 按游戏事件触发的执行里，每个自动化每局最多叫她几次（`ask()`，每次花一次 AI 调用，ADR 045 D2）。
pub const ASKS_PER_MATCH: u32 = 3;

/// `for` 的等待、每局次数、冷却、每局叫她几次的记账。一局结束（或游戏退出）时这个游戏的次数清零。
#[derive(Debug, Default)]
pub struct Triggers {
    waiting: Vec<Waiting>,
    /// （自动化，游戏）→ 这一局触发了几次。
    counts: HashMap<(String, String), u32>,
    /// （自动化，游戏）→ 这一局按事件触发的执行里叫了她几次。
    asks: HashMap<(String, String), u32>,
    /// 自动化 → 上次触发的时间（毫秒）。
    last: HashMap<String, u64>,
}

impl Triggers {
    fn allowed(&self, armed: &Armed, game: &str, now: u64) -> bool {
        let used = self
            .counts
            .get(&(armed.id.to_string(), game.to_string()))
            .copied()
            .unwrap_or(0);
        let cooled = self.last.get(armed.id).map_or(true, |t| {
            now.saturating_sub(*t) >= u64::from(armed.trigger.cooldown_seconds) * 1000
        });
        (!armed.trigger.once_per_match || used < 1) && cooled
    }

    /// 按游戏事件触发的执行要叫她一次（`ask()`）：这一局这个自动化在这个游戏上还没到 `limit` 次就记一次、
    /// 返回 true；到了返回 false（这次不交给她）。手动、热键、定时运行的不走这里（不限）。和触发次数同处清零。
    pub fn take_ask(&mut self, automation: &str, game: &str, limit: u32) -> bool {
        let used = self
            .asks
            .entry((automation.to_string(), game.to_string()))
            .or_default();
        if *used >= limit {
            return false;
        }
        *used += 1;
        true
    }

    fn fired(&mut self, automation: &str, event: &GameEvent, now: u64) -> Fire {
        *self
            .counts
            .entry((automation.to_string(), event.game.clone()))
            .or_default() += 1;
        self.last.insert(automation.to_string(), now);
        Fire {
            automation: automation.to_string(),
            event: event.clone(),
        }
    }

    /// 一件事来了：返回马上要运行的；有 `for` 的先记下等着（[`Triggers::tick`]）。
    pub fn on_event(&mut self, armed: &[Armed], event: &GameEvent, now: u64) -> Vec<Fire> {
        if event.kind == "game.phase_changed" {
            // 阶段又变了：这个游戏还在等的都作废
            self.waiting.retain(|w| w.event.game != event.game);
        }
        let mut out = Vec::new();
        for a in armed {
            if !trigger_matches(a.trigger, event) {
                continue;
            }
            if a.trigger.for_seconds > 0 {
                self.waiting.push(Waiting {
                    automation: a.id.to_string(),
                    event: event.clone(),
                    due: now + u64::from(a.trigger.for_seconds) * 1000,
                });
            } else if self.allowed(a, &event.game, now) {
                out.push(self.fired(a.id, event, now));
            }
        }
        if matches!(event.kind.as_str(), "game.match_ended" | "game.stopped") {
            self.counts.retain(|(_, game), _| *game != event.game);
            self.asks.retain(|(_, game), _| *game != event.game);
        }
        out
    }

    /// 到点的 `for`：阶段一直没变（等的时候变了会在 [`Triggers::on_event`] 作废）就运行。
    pub fn tick(&mut self, armed: &[Armed], now: u64) -> Vec<Fire> {
        let (due, rest): (Vec<Waiting>, Vec<Waiting>) =
            self.waiting.drain(..).partition(|w| w.due <= now);
        self.waiting = rest;
        let mut out = Vec::new();
        for w in due {
            // 等的时候自动化被删了或改了就不跑
            let Some(a) = armed
                .iter()
                .find(|a| a.id == w.automation && trigger_matches(a.trigger, &w.event))
            else {
                continue;
            };
            if self.allowed(a, &w.event.game, now) {
                out.push(self.fired(&w.automation, &w.event, now));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests;
