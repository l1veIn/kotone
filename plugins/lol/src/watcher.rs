//! 英雄联盟插件的后台和工具（ADR 035）：读本机的客户端接口（LCU）和局内接口（Live Client Data），都只读。
//! （原来是桌面端的 `lol.rs`，ADR 038 第 1 步搬进插件：`AppHandle` 换成了宿主 [`Host`]。）
//!
//! - 客户端开着时每 2 秒看一次对局阶段；进加载界面就在后台查这局 10 个人的战绩（同时 2 个、每人 20 局），
//!   她问“查战绩”时多半已经查好；
//! - `lol.match` / `lol.spell` 读局内接口（马上返回）；召唤师技能是玩家说了才记，这局结束清掉；
//! - 端口和令牌从 `LeagueClientUx.exe` 的命令行读（不用管理员）；令牌不进日志。
//! - 插件或它的“读客户端数据”开关（`plugins.lol.sources.client`）关着时什么都不读（D5）。
//! - 游戏中心（ADR 036）的 `lol` 适配器：客户端开着时由这里报阶段（客户端阶段 + 局内接口有没有数据），
//!   对局里每 2 秒把局内接口的新事件（龙、塔、团灭……）和“我阵亡 / 复活”报给游戏中心。
//! - 测试用（不开游戏，`scripts/manual-e2e/lol_tools_real.py`）：`KOTONE_LOL_LCU=<端口>:<令牌>` 不找客户端进程、
//!   直接连 127.0.0.1 的这个端口；`KOTONE_LOL_LIVE=<地址>` 换掉局内接口的地址（只认本机）。

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use kotone_core::log;
use kotone_plugin_api::{Host, Observation, Phase, ToolResult};
use serde_json::{json, Map, Value};

use crate::client::events;
use crate::client::gamedata::normalize;
use crate::client::history::{self, PlayerHistory};
use crate::client::http::paths;
use crate::client::http::{loopback_base, LIVE_BASE};
use crate::client::live::{self, LiveGame, LivePlayer};
use crate::client::spells::{self, SpellTracker, SpellUse};
use crate::client::text::{self, ScoutPlayer};
use crate::client::{in_game_phase, GameData, Lcu, LcuAuth, Live, LolError};
use crate::pages;
use crate::tools::{TOOL_GAME, TOOL_MATCH, TOOL_PLAYER, TOOL_SCOUT, TOOL_SPELL};
use crate::LolPlugin;

/// 客户端开着时多久看一次阶段；没开时多久看一次进程。
const POLL: Duration = Duration::from_secs(2);
const IDLE_POLL: Duration = Duration::from_secs(5);
/// 每人查几局。
const HISTORY_GAMES: u32 = 20;
/// 同时查几个人（照 LeagueAkari）。
const CONCURRENCY: usize = 2;
/// 她问的时候还没查完，最多等多久。
const SCOUT_WAIT: Duration = Duration::from_secs(12);
/// 英雄联盟的游戏配置 id（游戏中心里的 `game`）。
const LOL: &str = "lol";
/// “读客户端数据”开关（plugin.toml 的 sources）。
pub const SOURCE_CLIENT: &str = "client";

struct Client {
    pid: u32,
    lcu: Lcu,
    /// 玩家自己的 puuid（第一次用时查）。
    me: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct Member {
    puuid: String,
    champion_id: i64,
    /// 1 = teamOne，2 = teamTwo。
    team: u8,
    party: Option<i64>,
}

#[derive(Default)]
struct Match {
    game_id: i64,
    queue_id: i64,
    mode: String,
    members: Vec<Member>,
    histories: HashMap<String, PlayerHistory>,
    /// 还在查的人。
    pending: HashSet<String>,
}

#[derive(Default)]
pub(crate) struct State {
    client: Option<Client>,
    phase: String,
    data: Option<Arc<GameData>>,
    loading_data: bool,
    current: Option<Match>,
    tracker: SpellTracker,
    /// 已经向游戏中心认领了英雄联盟的阶段（客户端开着）。
    claimed: bool,
    /// 局内接口里已经报过的最大 `EventID`（第一次连上时取当时最新的，之前的不补）。
    live_seen: Option<i64>,
    /// 上一轮玩家自己是不是阵亡了。
    was_dead: Option<bool>,
}

fn live_client() -> Result<&'static Live, String> {
    static LIVE: OnceLock<Option<Live>> = OnceLock::new();
    LIVE.get_or_init(|| {
        let base = match std::env::var("KOTONE_LOL_LIVE") {
            Err(_) => LIVE_BASE.to_string(),
            Ok(value) => {
                let base = loopback_base(&value);
                log::log(&match &base {
                    Some(base) => format!("lol: KOTONE_LOL_LIVE：局内接口改连 {base}"),
                    None => "lol: KOTONE_LOL_LIVE 不是本机的 http(s) 地址，局内接口不连".into(),
                });
                base?
            }
        };
        Live::with_base(&base).ok()
    })
    .as_ref()
    .ok_or_else(|| "局内接口的客户端建不起来".to_string())
}

/// 测试用的客户端（`KOTONE_LOL_LCU=<端口>:<令牌>`）。没设 → None（照常找 `LeagueClientUx.exe`）；
/// 设了 → Some，写错了是 Some(None)：谁都不连，免得测试悄悄连上玩家真的客户端。
fn test_lcu() -> Option<Option<&'static LcuAuth>> {
    static AUTH: OnceLock<Option<Option<LcuAuth>>> = OnceLock::new();
    AUTH.get_or_init(|| {
        let value = std::env::var("KOTONE_LOL_LCU").ok()?;
        let auth = crate::client::parse_port_token(&value);
        log::log(&match &auth {
            Some(auth) => format!(
                "lol: KOTONE_LOL_LCU：不找客户端进程，连 127.0.0.1:{}",
                auth.port
            ),
            None => "lol: KOTONE_LOL_LCU 要写成 <端口>:<令牌>，客户端接口不连".into(),
        });
        Some(auth)
    })
    .as_ref()
    .map(Option::as_ref)
}

/// 插件开着、“读客户端数据”也开着。
fn enabled(host: &dyn Host) -> bool {
    host.enabled() && host.source_enabled(SOURCE_CLIENT)
}

impl LolPlugin {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap()
    }

    /// 启动时：开始盯客户端（不阻塞启动）。
    pub(crate) fn watch(self: Arc<Self>, host: Arc<dyn Host>) {
        let task_host = host.clone();
        host.spawn(Box::pin(async move {
            loop {
                let open = self.tick(&task_host).await;
                tokio::time::sleep(if open { POLL } else { IDLE_POLL }).await;
            }
        }));
    }

    fn reset_all(&self, host: &dyn Host) {
        let claimed = {
            let mut s = self.state();
            if s.client.is_some() {
                log::log("lol: 英雄联盟客户端关了");
            }
            let claimed = s.claimed;
            *s = State::default();
            claimed
        };
        if claimed {
            // 交回给游戏中心的进程那一路
            host.game_observe(
                LOL,
                Observation {
                    phase: Phase::Stopped,
                    source: "lol/client".into(),
                    ..Observation::default()
                },
            );
            host.game_claim(LOL, false);
        }
    }

    /// 连上客户端（进程换了就重新读命令行）。
    fn ensure_client(&self, host: &dyn Host, pid: u32) -> Option<Lcu> {
        if let Some(c) = self.state().client.as_ref().filter(|c| c.pid == pid) {
            return Some(c.lcu.clone());
        }
        let auth = match test_lcu() {
            Some(test) => test?.clone(),
            None => host
                .process_command_line(pid)
                .as_deref()
                .and_then(crate::client::parse_command_line)?,
        };
        let lcu = Lcu::new(&auth).ok()?;
        log::log(&format!(
            "lol: 连上英雄联盟客户端（端口 {}，{} {}）",
            auth.port, auth.region, auth.platform
        ));
        let mut s = self.state();
        *s = State::default();
        s.client = Some(Client {
            pid,
            lcu: lcu.clone(),
            me: None,
        });
        Some(lcu)
    }

    /// 一轮；返回客户端是不是开着。
    async fn tick(&self, host: &Arc<dyn Host>) -> bool {
        if !enabled(host.as_ref()) {
            self.reset_all(host.as_ref());
            return false;
        }
        let Some(pid) = client_pid(host.as_ref()) else {
            self.reset_all(host.as_ref());
            return false;
        };
        let Some(lcu) = self.ensure_client(host.as_ref(), pid) else {
            return false;
        };
        let phase = match lcu.get(paths::PHASE).await {
            Ok(Value::String(phase)) => phase,
            Ok(_) => String::new(),
            Err(error) => {
                // 客户端重启了（端口变了）：下一轮重新读命令行
                log::log(&format!("lol: 读对局阶段失败：{error}"));
                self.state().client = None;
                return true;
            }
        };
        let previous = std::mem::replace(&mut self.state().phase, phase.clone());
        self.load_data_in_background(&lcu);
        let mut live = None;
        if in_game_phase(&phase) {
            if let Ok(session) = lcu.get(paths::SESSION).await {
                self.on_session(&lcu, &session).await;
            }
            live = poll_live().await;
            if let Some((raw, game)) = &live {
                self.push_live_events(host.as_ref(), raw, game);
            }
        } else if in_game_phase(&previous) {
            // 这局结束：技能记录清掉；战绩留着，下一局开始时换掉
            let mut s = self.state();
            s.tracker = SpellTracker::default();
            s.live_seen = None;
            s.was_dead = None;
        }
        self.report_phase(host.as_ref(), &phase, live.as_ref().map(|(_, game)| game));
        true
    }

    /// 报给游戏中心：现在的阶段（`InProgress` 要局内接口有游戏时间才算“对局中”，之前是“加载”）和几项状态。
    fn report_phase(&self, host: &dyn Host, native: &str, live: Option<&LiveGame>) {
        let ready = live.is_some_and(|g| g.time > 0.0);
        let phase: Phase = serde_json::from_value(json!(events::generic_phase(native, ready)))
            .unwrap_or(Phase::Menu);
        let mut attrs = Map::new();
        {
            let mut s = self.state();
            s.claimed = true;
            if let Some(m) = s.current.as_ref().filter(|_| in_game_phase(native)) {
                attrs.insert("lol.gameId".into(), json!(m.game_id));
                attrs.insert("lol.queueId".into(), json!(m.queue_id));
                attrs.insert("lol.mode".into(), json!(m.mode));
            }
        }
        if let Some(game) = live {
            attrs.insert("lol.gameTime".into(), json!(game.time.round()));
            attrs.insert("lol.mode".into(), json!(game.mode));
        }
        host.game_claim(LOL, true);
        host.game_observe(
            LOL,
            Observation {
                phase,
                native_phase: native.to_string(),
                attrs,
                source: "lol/client".into(),
            },
        );
    }

    /// 局内接口的新事件和“我阵亡 / 复活”报给游戏中心。第一次连上时只记下起点，之前的不补。
    fn push_live_events(&self, host: &dyn Host, raw: &Value, game: &LiveGame) {
        let me = game.me().map(|p| (p.dead, p.respawn));
        let (after, was_dead) = {
            let mut s = self.state();
            let after = *s
                .live_seen
                .get_or_insert_with(|| events::last_event_id(raw));
            let was_dead = match me {
                Some((dead, _)) => s.was_dead.replace(dead),
                None => s.was_dead,
            };
            (after, was_dead)
        };
        let new = events::new_events(raw, game, after);
        if let Some(last) = new.iter().map(|e| e.0).max() {
            self.state().live_seen = Some(last);
        }
        for (_, kind, data, time) in new {
            host.game_event(LOL, kind, "lol/live-client", data, Some(time));
        }
        match (was_dead, me) {
            (Some(false), Some((true, respawn))) => {
                let mut data = Map::new();
                data.insert("respawnSeconds".into(), json!(respawn.max(0.0).round()));
                host.game_event(
                    LOL,
                    "lol.my_death",
                    "lol/live-client",
                    data,
                    Some(game.time),
                );
            }
            (Some(true), Some((false, _))) => {
                host.game_event(
                    LOL,
                    "lol.my_respawn",
                    "lol/live-client",
                    Map::new(),
                    Some(game.time),
                );
            }
            _ => {}
        }
    }

    fn load_data_in_background(&self, lcu: &Lcu) {
        {
            let mut s = self.state();
            if s.data.is_some() || s.loading_data {
                return;
            }
            s.loading_data = true;
        }
        let lcu = lcu.clone();
        let state = self.state.clone();
        tokio::spawn(async move {
            let data = load_data(&lcu).await;
            let mut s = state.lock().unwrap();
            s.loading_data = false;
            if let Some(data) = data {
                s.data = Some(Arc::new(data));
            }
        });
    }

    /// 游戏资料：还没读就现在读（工具调用时）。
    async fn ensure_data(&self) -> Option<Arc<GameData>> {
        let lcu = {
            let s = self.state();
            if let Some(data) = &s.data {
                return Some(data.clone());
            }
            s.client.as_ref()?.lcu.clone()
        };
        let data = Arc::new(load_data(&lcu).await?);
        self.state().data = Some(data.clone());
        Some(data)
    }

    async fn my_puuid(&self, lcu: &Lcu) -> Option<String> {
        // 先取出来再判断：锁不能跨过下面的 await
        let cached = self.state().client.as_ref().and_then(|c| c.me.clone());
        if cached.is_some() {
            return cached;
        }
        let me = lcu
            .get(paths::ME)
            .await
            .ok()?
            .get("puuid")?
            .as_str()?
            .to_string();
        if let Some(c) = self.state().client.as_mut() {
            c.me = Some(me.clone());
        }
        Some(me)
    }

    async fn on_session(&self, lcu: &Lcu, session: &Value) {
        let game_id = session["gameData"]["gameId"].as_i64().unwrap_or(0);
        if game_id == 0 {
            return;
        }
        let found = members(session);
        let missing: Vec<String> = {
            let mut s = self.state();
            let fresh = s.current.as_ref().map(|m| m.game_id) != Some(game_id);
            if fresh {
                s.tracker = SpellTracker::default();
                s.current = Some(Match {
                    game_id,
                    queue_id: session["gameData"]["queue"]["id"].as_i64().unwrap_or(0),
                    mode: session["gameData"]["queue"]["gameMode"]
                        .as_str()
                        .unwrap_or("")
                        .to_string(),
                    ..Match::default()
                });
            }
            let m = s.current.as_mut().unwrap();
            // 英雄可能晚一点才填上（加载界面前后），每轮都按最新的
            for member in &found {
                match m.members.iter_mut().find(|x| x.puuid == member.puuid) {
                    Some(x) => *x = member.clone(),
                    None => m.members.push(member.clone()),
                }
            }
            let missing: Vec<String> = found
                .iter()
                .map(|x| x.puuid.clone())
                .filter(|p| !m.histories.contains_key(p) && !m.pending.contains(p))
                .collect();
            // 先记成“在查”，免得下一轮重复查
            m.pending.extend(missing.iter().cloned());
            missing
        };
        if missing.is_empty() {
            return;
        }
        let _ = self.my_puuid(lcu).await;
        log::log(&format!("lol: 这局开始查 {} 个人的战绩", missing.len()));
        tokio::spawn(prefetch(self.state.clone(), lcu.clone(), game_id, missing));
    }
}

/// 客户端自带的中文游戏资料（英雄、召唤师技能、装备、海克斯、队列），约 0.1 秒。
async fn load_data(lcu: &Lcu) -> Option<GameData> {
    let (champions, spells, items, augments, queues) = tokio::join!(
        lcu.get(paths::CHAMPIONS),
        lcu.get(paths::SPELLS),
        lcu.get(paths::ITEMS),
        lcu.get(paths::AUGMENTS),
        lcu.get(paths::QUEUES),
    );
    let data = GameData::from_json(
        &champions.ok()?,
        &spells.unwrap_or(Value::Null),
        &items.unwrap_or(Value::Null),
        &augments.unwrap_or(Value::Null),
        &queues.unwrap_or(Value::Null),
    );
    (!data.is_empty()).then_some(data)
}

/// `gameflow/session` → 这局的人（局内时两边都有）。
fn members(session: &Value) -> Vec<Member> {
    let data = &session["gameData"];
    let selections: Vec<&Value> = data["playerChampionSelections"]
        .as_array()
        .map(|a| a.iter().collect())
        .unwrap_or_default();
    let mut out = Vec::new();
    for (team, key) in [(1u8, "teamOne"), (2u8, "teamTwo")] {
        for m in data[key].as_array().into_iter().flatten() {
            let puuid = m["puuid"].as_str().unwrap_or("").to_string();
            if puuid.is_empty() {
                continue; // 电脑
            }
            let mut champion_id = m["championId"].as_i64().unwrap_or(0);
            if champion_id <= 0 {
                let internal = m["summonerInternalName"].as_str().unwrap_or("");
                champion_id = selections
                    .iter()
                    .find(|x| {
                        x["puuid"].as_str() == Some(puuid.as_str())
                            || (!internal.is_empty()
                                && x["summonerInternalName"].as_str() == Some(internal))
                    })
                    .and_then(|x| x["championId"].as_i64())
                    .unwrap_or(0);
            }
            out.push(Member {
                puuid,
                champion_id,
                team,
                party: m["teamParticipantId"].as_i64(),
            });
        }
    }
    out
}

/// 测试用的客户端开着就当它在（pid 0）；否则找 `LeagueClientUx.exe`。
fn client_pid(host: &dyn Host) -> Option<u32> {
    if let Some(test) = test_lcu() {
        return test.map(|_| 0);
    }
    host.process_ids(crate::client::CLIENT_PROCESS)
        .first()
        .copied()
}

/// 局内接口这一轮的数据（连不上、还在加载时 None）。
async fn poll_live() -> Option<(Value, LiveGame)> {
    let raw = live_client().ok()?.all_game_data().await.ok()?;
    let game = live::parse_all_game_data(&raw)?;
    Some((raw, game))
}

async fn prefetch(state: Arc<Mutex<State>>, lcu: Lcu, game_id: i64, puuids: Vec<String>) {
    let started = Instant::now();
    let semaphore = Arc::new(tokio::sync::Semaphore::new(CONCURRENCY));
    let mut tasks = Vec::new();
    for puuid in puuids {
        let lcu = lcu.clone();
        let semaphore = semaphore.clone();
        let state = state.clone();
        tasks.push(tokio::spawn(async move {
            let _permit = semaphore.acquire_owned().await.ok();
            let history = fetch_history(&lcu, &puuid).await;
            let mut s = state.lock().unwrap();
            if let Some(m) = s.current.as_mut().filter(|m| m.game_id == game_id) {
                m.pending.remove(&puuid);
                m.histories.insert(puuid, history);
            }
        }));
    }
    let count = tasks.len();
    for task in tasks {
        let _ = task.await;
    }
    log::log(&format!(
        "lol: {count} 个人的战绩查完了，用了 {:.1} 秒",
        started.elapsed().as_secs_f64()
    ));
}

/// 一个人：隐私设置、最近 20 局、单双排段位。隐藏战绩的照样读（D6）。
async fn fetch_history(lcu: &Lcu, puuid: &str) -> PlayerHistory {
    let mut h = PlayerHistory::default();
    if let Ok(summoner) = lcu.get(&paths::summoner(puuid)).await {
        h.privacy = summoner["privacy"].as_str().unwrap_or("").to_string();
        let name = summoner["gameName"].as_str().unwrap_or("");
        let tag = summoner["tagLine"].as_str().unwrap_or("");
        if !name.is_empty() {
            h.riot_id = if tag.is_empty() {
                name.to_string()
            } else {
                format!("{name}#{tag}")
            };
        }
    }
    match lcu.get(&paths::matches(puuid, HISTORY_GAMES)).await {
        Ok(list) => {
            h.games = history::parse_match_list(&list);
            if h.riot_id.is_empty() {
                h.riot_id = history::riot_id_from_match_list(&list).unwrap_or_default();
            }
        }
        Err(error) => {
            h.error = Some(if h.hidden() {
                format!("设成了隐藏，{error}")
            } else {
                error.to_string()
            })
        }
    }
    if h.games.is_empty() && h.error.is_none() && h.hidden() {
        h.error = Some("设成了隐藏，查不到".into());
    }
    if let Ok(ranked) = lcu.get(&paths::ranked(puuid)).await {
        h.rank = history::parse_solo_rank(&ranked);
    }
    h
}

// ---------- 工具 ----------

fn str_arg<'a>(arguments: &'a Value, key: &str) -> Option<&'a str> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn status(status: &str, text: impl Into<String>) -> Value {
    json!({"status": status, "text": text.into()})
}

impl LolPlugin {
    /// 不在对局里时怎么说。
    fn not_in_game(&self, host: &dyn Host) -> Value {
        let (phase, client) = {
            let s = self.state();
            (s.phase.clone(), s.client.is_some())
        };
        let text = if in_game_phase(&phase) {
            "游戏还在加载，进了游戏再问"
        } else if phase == "ChampSelect" {
            "还在选英雄；进了加载界面才查得到这局的人"
        } else if client || client_pid(host).is_some() {
            "现在不在英雄联盟的对局里"
        } else {
            "英雄联盟没开"
        };
        status("not_in_game", text)
    }

    /// 局内数据；不在对局里时 Err(给模型的结果)。
    async fn live_game(&self, host: &dyn Host) -> Result<LiveGame, Value> {
        let live = live_client().map_err(|e| status("error", e))?;
        match live.all_game_data().await {
            Ok(v) => live::parse_all_game_data(&v)
                .ok_or_else(|| status("not_in_game", "游戏还在加载，进了游戏再问")),
            Err(LolError::Unavailable) => Err(self.not_in_game(host)),
            Err(LolError::Status(404, _)) => {
                Err(status("not_in_game", "游戏还在加载，进了游戏再问"))
            }
            Err(error) => Err(status("error", format!("读局内数据失败：{error}"))),
        }
    }

    /// `lol.match` / `lol.spell` / `lol.scout`。结果只有给她的那份（页面在第 2 步加）。
    pub(crate) async fn tool(
        &self,
        host: &Arc<dyn Host>,
        name: &str,
        arguments: &Value,
    ) -> Result<ToolResult, String> {
        if !enabled(host.as_ref()) {
            return Ok(ToolResult::content(status(
                "disabled",
                "英雄联盟插件的“读客户端数据”已关闭（插件 → 英雄联盟 里可以打开）",
            )));
        }
        Ok(match name {
            TOOL_MATCH => ToolResult::content(self.match_tool(host.as_ref()).await),
            TOOL_SPELL => ToolResult::content(self.spell_tool(host.as_ref(), arguments).await),
            TOOL_SCOUT => self.scout_tool(host, arguments).await,
            TOOL_PLAYER => self.player_tool(host.as_ref(), arguments).await,
            TOOL_GAME => self.game_tool(host.as_ref(), arguments).await,
            other => return Err(format!("{other} 不是英雄联盟工具")),
        })
    }

    async fn match_tool(&self, host: &dyn Host) -> Value {
        let game = match self.live_game(host).await {
            Ok(game) => game,
            Err(result) => return result,
        };
        let data = self.ensure_data().await;
        let tracker = self.state().tracker.clone();
        json!({"status": "ok", "text": text::render_match(&game, data.as_deref(), &tracker)})
    }
}

/// 玩家说的技能名 → 客户端里的名字。
fn spell_name(query: &str) -> &str {
    match query.trim() {
        "" | "闪" | "闪现" | "f" | "F" => "闪现",
        "点" | "点燃" | "引燃" => "引燃",
        "治疗" | "治疗术" | "奶" => "治疗术",
        "传送" | "TP" | "tp" => "传送",
        "屏障" | "盾" => "屏障",
        "幽灵" | "疾跑" | "幽灵疾步" => "幽灵疾步",
        "雪球" | "标记" => "标记",
        "清晰" | "清晰术" => "清晰术",
        "惩戒" | "惩击" => "惩戒",
        other => other,
    }
}

/// 没有游戏资料时的基础冷却（秒）。
fn fallback_cooldown(spell: &str) -> Option<f64> {
    Some(match spell {
        "闪现" => 300.0,
        "传送" => 300.0,
        "治疗术" | "虚弱" | "净化" | "幽灵疾步" | "清晰术" => 240.0,
        "引燃" | "屏障" => 180.0,
        "标记" => 80.0,
        _ => return None,
    })
}

fn champion_matches(p: &LivePlayer, data: Option<&GameData>, query: &str) -> bool {
    if let Some(c) = data.and_then(|d| d.champion_by_alias(&p.alias)) {
        return c.matches(query);
    }
    let q = normalize(query);
    let names = [normalize(&p.champion), normalize(&p.alias)];
    !q.is_empty()
        && names
            .iter()
            .any(|n| !n.is_empty() && (n.contains(&q) || q.contains(n)))
}

impl LolPlugin {
    async fn spell_tool(&self, host: &dyn Host, arguments: &Value) -> Value {
        let clear = arguments.get("clear").and_then(Value::as_bool) == Some(true);
        let champion = str_arg(arguments, "champion");
        let spell = spell_name(str_arg(arguments, "spell").unwrap_or("闪现")).to_string();
        if clear && champion.is_none() {
            let n = self.state().tracker.clear(None, None);
            return status("ok", format!("记下的 {n} 条技能都清掉了"));
        }
        let Some(champion) = champion else {
            return status("error", "要说是哪个英雄交的（champion）");
        };
        let game = match self.live_game(host).await {
            Ok(game) => game,
            Err(result) => return result,
        };
        let data = self.ensure_data().await;
        let data = data.as_deref();
        let enemies: Vec<&LivePlayer> = game
            .enemies()
            .filter(|p| champion_matches(p, data, champion))
            .collect();
        let candidates: Vec<&LivePlayer> = if enemies.is_empty() {
            game.players
                .iter()
                .filter(|p| champion_matches(p, data, champion))
                .collect()
        } else {
            enemies
        };
        let short = |p: &LivePlayer| text::champion_names(p, data).1;
        let player = match candidates.as_slice() {
            [one] => *one,
            [] => {
                let names: Vec<String> = game.enemies().map(short).collect();
                return status(
                    "not_found",
                    format!("这局没有“{champion}”。对面是：{}", names.join("、")),
                );
            }
            many => {
                let names: Vec<String> = many.iter().map(|p| short(p)).collect();
                return status("ambiguous", format!("是哪个：{}？", names.join("、")));
            }
        };
        let name = short(player);
        if clear {
            let n = self.state().tracker.clear(
                Some(&player.alias),
                str_arg(arguments, "spell").map(|_| spell.as_str()),
            );
            return status("ok", format!("{name} 记下的 {n} 条清掉了"));
        }
        let Some(have) = player
            .spells
            .iter()
            .find(|s| **s == spell || s.contains(&spell) || spell.contains(s.as_str()))
            .cloned()
        else {
            return status(
                "not_found",
                format!("{name} 没带{spell}（带的是 {}）", player.spells.join("、")),
            );
        };
        let Some(base) = data
            .and_then(|d| d.spell(&have, &game.mode))
            .map(|s| s.cooldown)
            .or_else(|| fallback_cooldown(&have))
        else {
            return status("error", format!("不知道{have}的冷却"));
        };
        let ago = arguments
            .get("ago")
            .and_then(Value::as_f64)
            .unwrap_or(0.0)
            .clamp(0.0, 300.0);
        let used_at = (game.time - ago).max(0.0);
        let (ready_at, notes) = spells::ready_time(base, used_at, &game.mode, &player.items);
        let entry = SpellUse {
            alias: player.alias.clone(),
            champion: name.clone(),
            spell: have.clone(),
            used_at,
            ready_at,
            notes: notes.clone(),
        };
        let chat = spells::chat_line(&entry);
        self.state().tracker.record(entry);
        let how = if notes.is_empty() {
            String::new()
        } else {
            format!("（{}）", notes.join("，"))
        };
        json!({
            "status": "ok",
            "text": format!(
                "记下了：{name} {} 交的{have}，{} 好{how}",
                crate::client::clock(used_at),
                crate::client::clock(ready_at)
            ),
            "chat": chat,
            "ready": crate::client::clock(ready_at),
        })
    }

    async fn scout_tool(&self, host: &Arc<dyn Host>, arguments: &Value) -> ToolResult {
        let only = str_arg(arguments, "player");
        let has_match = self.state().current.is_some();
        if !has_match {
            // 工具可能比后台那一轮先到：现在看一次
            self.tick(host).await;
        }
        let deadline = Instant::now() + SCOUT_WAIT;
        loop {
            // 先把数取出来、放开锁，再决定（not_in_game 自己也要拿锁）
            let waiting = self.state().current.as_ref().map(|m| m.pending.len());
            match waiting {
                None => return ToolResult::content(self.not_in_game(host.as_ref())),
                Some(0) => break,
                Some(_) if Instant::now() >= deadline => break,
                Some(_) => tokio::time::sleep(Duration::from_millis(300)).await,
            }
        }
        let data = self.ensure_data().await;
        if self.state().current.is_none() {
            return ToolResult::content(self.not_in_game(host.as_ref()));
        }
        let Some(view) = self.match_view(data.as_deref()) else {
            return ToolResult::content(status("not_in_game", "这局刚结束"));
        };
        let text = text::render_scout(&view.players, view.queue_id, &view.mode, view.random, only);
        ToolResult::content(json!({"status": "ok", "text": text})).with_page(
            pages::PAGE_MATCH,
            pages::match_page(
                &view.players,
                &view.puuids,
                view.game_id,
                view.queue_id,
                &view.mode,
                view.random,
                data.as_deref(),
            ),
        )
    }

    /// 这局的 10 个人（和 `puuids` 一一对应）、模式。没有这局时 None。
    fn match_view(&self, data: Option<&GameData>) -> Option<MatchView> {
        let s = self.state();
        let m = s.current.as_ref()?;
        let me = s
            .client
            .as_ref()
            .and_then(|c| c.me.clone())
            .unwrap_or_default();
        let my_team = m
            .members
            .iter()
            .find(|x| x.puuid == me)
            .map(|x| x.team)
            .unwrap_or(1);
        let players: Vec<ScoutPlayer> = m
            .members
            .iter()
            .map(|x| {
                let champion = data.and_then(|d| d.champion(x.champion_id));
                let history = m.histories.get(&x.puuid).cloned();
                let name = history
                    .as_ref()
                    .map(|h| h.riot_id.split('#').next().unwrap_or("").to_string())
                    .unwrap_or_default();
                ScoutPlayer {
                    champion: champion
                        .map(|c| c.label())
                        .unwrap_or_else(|| format!("英雄 {}", x.champion_id)),
                    alias: champion.map(|c| c.alias.clone()).unwrap_or_default(),
                    short: champion
                        .map(|c| c.short().to_string())
                        .unwrap_or_else(|| format!("英雄 {}", x.champion_id)),
                    champion_id: x.champion_id,
                    name,
                    ally: x.team == my_team,
                    me: x.puuid == me,
                    party: x.party,
                    history,
                }
            })
            .collect();
        let mode = data
            .and_then(|d| d.queue_name(m.queue_id))
            .map(str::to_string)
            .unwrap_or_else(|| text::mode_name(&m.mode).to_string());
        Some(MatchView {
            puuids: m.members.iter().map(|x| x.puuid.clone()).collect(),
            players,
            game_id: m.game_id,
            queue_id: m.queue_id,
            random: crate::client::gamedata::random_pick(data, m.queue_id),
            mode,
        })
    }

    /// 连着的客户端（没开时给她的话）。
    fn lcu_or_closed(&self, host: &dyn Host) -> Result<Lcu, Value> {
        if let Some(lcu) = self.state().client.as_ref().map(|c| c.lcu.clone()) {
            return Ok(lcu);
        }
        match client_pid(host).and_then(|pid| self.ensure_client(host, pid)) {
            Some(lcu) => Ok(lcu),
            None => Err(status(
                "not_in_client",
                "英雄联盟客户端没开（查战绩要开着客户端）",
            )),
        }
    }

    /// `lol.player`：某个人最近的战绩（不写 = 自己；这局里的人按英雄名或玩家名；别的人写“名字#编号”）。
    async fn player_tool(&self, host: &dyn Host, arguments: &Value) -> ToolResult {
        let lcu = match self.lcu_or_closed(host) {
            Ok(lcu) => lcu,
            Err(result) => return ToolResult::content(result),
        };
        let data = self.ensure_data().await;
        let puuid = match self.find_player(&lcu, data.as_deref(), arguments).await {
            Ok(puuid) => puuid,
            Err(result) => return ToolResult::content(result),
        };
        // 这局查过的直接用
        let cached = self
            .state()
            .current
            .as_ref()
            .and_then(|m| m.histories.get(&puuid).cloned());
        let history = match cached {
            Some(h) => h,
            None => fetch_history(&lcu, &puuid).await,
        };
        let queue_id = self
            .state()
            .current
            .as_ref()
            .map(|m| m.queue_id)
            .unwrap_or(0);
        let text = pages::player_text(&history, data.as_deref());
        ToolResult::content(json!({"status": "ok", "text": text})).with_page(
            pages::PAGE_PLAYER,
            pages::player_page(&puuid, &history, queue_id, data.as_deref()),
        )
    }

    async fn find_player(
        &self,
        lcu: &Lcu,
        data: Option<&GameData>,
        arguments: &Value,
    ) -> Result<String, Value> {
        if let Some(puuid) = str_arg(arguments, "puuid") {
            return Ok(puuid.to_string());
        }
        let Some(query) = str_arg(arguments, "player") else {
            return self
                .my_puuid(lcu)
                .await
                .ok_or_else(|| status("error", "读不到你自己的账号"));
        };
        // 先在这局里找（英雄名、玩家名）
        if let Some(view) = self.match_view(data) {
            let hits: Vec<usize> = (0..view.players.len())
                .filter(|i| view.players[*i].matches(query))
                .collect();
            match hits.as_slice() {
                [one] => return Ok(view.puuids[*one].clone()),
                [] => {}
                many => {
                    let names: Vec<&str> = many
                        .iter()
                        .map(|i| view.players[*i].short.as_str())
                        .collect();
                    return Err(status(
                        "ambiguous",
                        format!("是哪个：{}？", names.join("、")),
                    ));
                }
            }
        }
        // 名字#编号：按 Riot ID 找
        if let Some((name, tag)) = query.split_once('#') {
            let path = format!(
                "/lol-summoner/v1/alias/lookup?gameName={}&tagLine={}",
                encode(name.trim()),
                encode(tag.trim())
            );
            if let Ok(found) = lcu.get(&path).await {
                if let Some(puuid) = found
                    .get("puuid")
                    .and_then(Value::as_str)
                    .filter(|p| !p.is_empty())
                {
                    return Ok(puuid.to_string());
                }
            }
            return Err(status(
                "not_found",
                format!("没找到 {query}（要写完整的 名字#编号）"),
            ));
        }
        Err(status(
            "not_found",
            format!("这局没有“{query}”；查别人要写完整的 名字#编号"),
        ))
    }

    /// `lol.game`（只给页面）：一局的详情，10 个人的 KDA、伤害、经济、装备。
    async fn game_tool(&self, host: &dyn Host, arguments: &Value) -> ToolResult {
        let lcu = match self.lcu_or_closed(host) {
            Ok(lcu) => lcu,
            Err(result) => return ToolResult::content(result),
        };
        let Some(game_id) = arguments
            .get("gameId")
            .and_then(Value::as_i64)
            .filter(|g| *g > 0)
        else {
            return ToolResult::content(status("error", "要写 gameId"));
        };
        let data = self.ensure_data().await;
        let detail = match lcu.get(&paths::game(game_id)).await {
            Ok(v) => v,
            Err(error) => {
                return ToolResult::content(status("error", format!("这局的详情读不到：{error}")))
            }
        };
        match pages::game_page(&detail, data.as_deref()) {
            Some(page) => ToolResult::content(json!({"status": "ok", "gameId": game_id}))
                .with_page(pages::PAGE_GAME, page),
            None => ToolResult::content(status("error", "这局的详情是空的")),
        }
    }
}

/// [`LolPlugin::match_view`]。
struct MatchView {
    players: Vec<ScoutPlayer>,
    puuids: Vec<String>,
    game_id: i64,
    queue_id: i64,
    mode: String,
    random: bool,
}

/// 查询参数里的值（UTF-8 按字节百分号编码，只留字母数字和 -_.~）。
fn encode(value: &str) -> String {
    value
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::encode;

    #[test]
    fn query_values_are_percent_encoded() {
        assert_eq!(encode("abc-1"), "abc-1");
        assert_eq!(encode("琴音 #"), "%E7%90%B4%E9%9F%B3%20%23");
    }
}
