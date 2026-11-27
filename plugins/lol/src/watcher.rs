//! 英雄联盟插件的后台和工具（ADR 035）：读本机的客户端接口（LCU）和局内接口（Live Client Data），都只读。
//! （原来是桌面端的 `lol.rs`，ADR 038 第 1 步搬进插件：`AppHandle` 换成了宿主 [`Host`]。）
//!
//! - 客户端开着时每 2 秒看一次对局阶段；进加载界面就在后台查这局 10 个人的战绩（同时 2 个；客户端那一段最近 50 局里每人看最近 20 局，
//!   见 [`paths::matches`]），她问“查战绩”时多半已经查好；
//! - `lol.match` / `lol.spell` 读局内接口（马上返回）；召唤师技能是玩家说了才记，这局结束清掉；
//! - 端口和令牌从 `LeagueClientUx.exe` 的命令行读（不用管理员）；令牌不进日志。
//! - 插件或它的“读客户端数据”开关（`plugins.lol.sources.client`）关着时什么都不读（D5）。
//! - 游戏中心（ADR 036）的 `lol` 适配器：客户端开着时由这里报阶段（客户端阶段 + 局内接口有没有数据），
//!   对局里每 2 秒把局内接口的新事件（龙、塔、团灭……）报给游戏中心；阵亡、复活、等级、装备是比前后两份局内数据
//!   得出的，和对局中的提醒在同一处报（`pushes/alerts.rs`，ADR 046 D7）。
//! - 测试用（不开游戏，`scripts/manual-e2e/lol_tools_real.py`）：`KOTONE_LOL_LCU=<端口>:<令牌>` 不找客户端进程、
//!   直接连 127.0.0.1 的这个端口；`KOTONE_LOL_LIVE=<地址>` 换掉局内接口的地址（只认本机）；
//!   `KOTONE_LOL_CDRAGON=<地址>` 换掉素材层的 CommunityDragon 地址（只认本机，写错了就不连网）。

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use kotone_core::log;
use kotone_plugin_api::{Host, Observation, Phase, ToolResult};
use serde_json::{json, Map, Value};

use crate::client::assets::{Asset, AssetError, Assets, CommunityDragon, Offline, Origin};
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
use crate::tools::{
    TOOL_CHAMPION, TOOL_CHAMPIONS, TOOL_CLIENT, TOOL_GAME, TOOL_HISTORY, TOOL_MATCH, TOOL_PLAYER,
    TOOL_SCOUT, TOOL_SHOUT, TOOL_SOURCE, TOOL_SPELL, TOOL_TIER,
};
use crate::LolPlugin;

/// 卡片推送（ADR 039 D3、ADR 043）：进加载推“开局战绩”，结算推“这局总结”。
mod auto_cards;
/// 选人阶段的“这局”（调研 G1）。
mod champ_select;
mod history_page;
/// 隐藏了身份的人、电脑（ADR 043 D9）：名单、合并、局内接口补电脑、页面和给她的字。
#[cfg(test)]
mod identity_tests;
mod profile_background;
/// 一键喊话（ADR 041 第 3 期，`lol.shout`）。
mod shout;

/// 清单里预设命令的参数 `lol.shout` 认不认（`lib.rs` 的清单测试用）。
#[cfg(test)]
pub(crate) fn shout_args_ok(args: &Value) -> bool {
    shout::Shout::parse(args).is_ok()
}

/// 结算后多久还没拿到这局的结果就不再等（和自动卡片“这局总结”一样，调研 G29）。
const RESULT_WAIT_MS: i64 = 60_000;

/// 客户端开着时多久看一次阶段；没开时多久看一次进程。
const POLL: Duration = Duration::from_secs(2);
const IDLE_POLL: Duration = Duration::from_secs(5);
/// 每人看几局（她的字、战绩卡片、这局页面都是“近 20 局”；向客户端要的是同一段最近 50 局，见 [`paths::matches`]）。
const HISTORY_GAMES: usize = 20;
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
    /// 只给页面：召唤师头像、召唤师技能、分路（[`ScoutPlayer`] 里同名的字段）。
    profile_icon_id: Option<i64>,
    spells: [i64; 2],
    position: String,
    /// 选人时选好了没有（别的阶段都是锁了）。
    pick: pages::Pick,
    /// 看得到是谁没有（ADR 043 D9）：只查 `Known` 的；隐藏了身份的真人、电脑照样列出来，只显示英雄。
    identity: text::Identity,
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
    /// 进过加载（不只是选人；选人被秒退的不算这局）。
    started: bool,
    /// 选人时的备选席、名字被隐藏（页面用）。
    bench: Vec<i64>,
    names_hidden: bool,
    /// 结算后这局的结果（自动卡片读到这局的详情时写，调研 G29）。
    result: Option<pages::MatchResult>,
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
    /// 推送过哪局的哪种卡片（[`auto_cards`]）。
    cards: auto_cards::CardState,
    /// “这局”页面的阶段和进这个阶段的时间（Unix 毫秒）。
    stage: Option<(pages::MatchPhase, i64)>,
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

    /// 启动时：开始盯客户端（不阻塞启动）。海克斯三选一自己一个循环（卡片在屏幕上时每秒看一次画面，ADR 043 D7）。
    pub(crate) fn watch(self: Arc<Self>, host: Arc<dyn Host>) {
        self.clone().watch_augment_picks(host.clone());
        let task_host = host.clone();
        host.spawn(Box::pin(async move {
            loop {
                let open = self.tick(&task_host).await;
                tokio::time::sleep(if open { POLL } else { IDLE_POLL }).await;
            }
        }));
    }

    fn reset_all(&self, host: &dyn Host) {
        self.reset_pushes(host);
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
        // 操作客户端（ADR 038 第 3.5 步，开关默认关）：自动接受对局、自动选 / 禁英雄
        self.run_control(host.as_ref(), &lcu, &phase).await;
        self.load_data_in_background(&lcu);
        let mut live = None;
        if phase == "ChampSelect" {
            // 选人（调研 G1）：双方的英雄、我方看得到名字的人马上查战绩
            if let Ok(session) = lcu.get(paths::CHAMP_SELECT).await {
                let flow = lcu.get(paths::SESSION).await.unwrap_or(Value::Null);
                self.on_champ_select(&lcu, &session, &flow).await;
            }
        } else if in_game_phase(&phase) {
            if let Ok(session) = lcu.get(paths::SESSION).await {
                self.on_session(&lcu, &session).await;
            }
            live = poll_live().await;
            if let Some((raw, game)) = &live {
                // 局内接口的 isBot：客户端没标的电脑在这里补上（ADR 043 D9）
                self.mark_live_bots(game);
                self.push_live_events(host.as_ref(), raw, game);
            }
        } else if in_game_phase(&previous) {
            // 这局结束：技能记录清掉；战绩留着，下一局开始时换掉
            let mut s = self.state();
            s.tracker = SpellTracker::default();
            s.live_seen = None;
        }
        self.report_phase(host.as_ref(), &phase, live.as_ref().map(|(_, game)| game));
        // 卡片推送（ADR 039 D3、ADR 043）：进加载查完战绩推“开局战绩”，结算推“这局总结”
        let live_ready = live.as_ref().is_some_and(|(_, game)| game.time > 0.0);
        self.update_stage(&phase, live_ready);
        self.auto_cards(host, &previous, &phase, live_ready);
        // 推送：选英雄卡片、海斗三选一（ADR 043 D7，原来的两个覆盖层）
        self.update_pushes(host, &lcu, &phase, live.as_ref().map(|(_, game)| game))
            .await;
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

    /// 局内接口事件列表里的新事报给游戏中心。第一次连上时只记下起点，之前的不补。阵亡、复活不在这里
    /// （[`crate::pushes`] 的对局中的提醒那一处报，ADR 046 D7）。
    fn push_live_events(&self, host: &dyn Host, raw: &Value, game: &LiveGame) {
        let after = *self
            .state()
            .live_seen
            .get_or_insert_with(|| events::last_event_id(raw));
        let new = events::new_events(raw, game, after);
        if let Some(last) = new.iter().map(|e| e.0).max() {
            self.state().live_seen = Some(last);
        }
        for (_, kind, data, time) in new {
            host.game_event(LOL, kind, "lol/live-client", data, Some(time));
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
        let (missing, hidden, bots) = {
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
            m.started = true;
            let count = |identity| found.iter().filter(|x| x.identity == identity).count();
            let (hidden, bots) = (count(text::Identity::Hidden), count(text::Identity::Bot));
            (merge_session(m, found), hidden, bots)
        };
        if missing.is_empty() {
            return;
        }
        let me = self.my_puuid(lcu).await.unwrap_or_default();
        let mine = if missing.contains(&me) {
            "（含你自己）"
        } else {
            ""
        };
        log::log(&format!(
            "lol: 这局开始查 {} 人的战绩{mine}；隐藏了身份的 {hidden} 人、电脑 {bots} 个不查",
            missing.len()
        ));
        tokio::spawn(prefetch(self.state.clone(), lcu.clone(), game_id, missing));
    }

    /// 局内接口说是电脑的（`isBot`），而客户端没给 puuid、我们当成“隐藏了身份”的人：改成电脑（按队伍和英雄对上）。
    fn mark_live_bots(&self, game: &LiveGame) {
        let data = self.loaded_data();
        if let Some(m) = self.state().current.as_mut() {
            mark_bots(&mut m.members, game, data.as_deref());
        }
    }

    /// 选人（调研 G1）：双方的英雄、我方的分路和召唤师技能、备选席；我方看得到名字的人马上开始查战绩
    /// （和加载同一套：同一局进了加载接着用，不重查）。`flow` 是 gameflow 的 session（队列、模式、对局 id 的后备）。
    async fn on_champ_select(&self, lcu: &Lcu, session: &Value, flow: &Value) {
        let Some(cs) =
            champ_select::parse(session, flow["gameData"]["gameId"].as_i64().unwrap_or(0))
        else {
            return;
        };
        let game_id = cs.game_id;
        let missing: Vec<String> = {
            let mut s = self.state();
            if s.current.as_ref().map(|m| m.game_id) != Some(game_id) {
                s.tracker = SpellTracker::default();
                s.current = Some(Match {
                    game_id,
                    queue_id: flow["gameData"]["queue"]["id"].as_i64().unwrap_or(0),
                    mode: flow["gameData"]["queue"]["gameMode"]
                        .as_str()
                        .unwrap_or("")
                        .to_string(),
                    ..Match::default()
                });
            }
            let m = s.current.as_mut().unwrap();
            if m.started {
                return;
            }
            // 选人时的名单每轮整个换（英雄、预选一直在变）
            m.members = cs.members;
            m.bench = cs.bench;
            m.names_hidden = cs.names_hidden;
            let missing: Vec<String> = m
                .members
                .iter()
                .filter(|x| x.identity == text::Identity::Known)
                .map(|x| x.puuid.clone())
                .filter(|p| !p.is_empty() && !m.histories.contains_key(p) && !m.pending.contains(p))
                .collect();
            m.pending.extend(missing.iter().cloned());
            missing
        };
        if missing.is_empty() {
            return;
        }
        // 我方看得到名字的人，自己也在里面（以前写成“N 个队友”，把自己也算进去了）
        let me = self.my_puuid(lcu).await.unwrap_or_default();
        let mine = if missing.contains(&me) {
            "（含你自己）"
        } else {
            ""
        };
        log::log(&format!(
            "lol: 选人时开始查我方 {} 人的战绩{mine}",
            missing.len()
        ));
        tokio::spawn(prefetch(self.state.clone(), lcu.clone(), game_id, missing));
    }

    /// 记下“这局”页面的阶段；变了就记进这个阶段的时间（阶段牌上“进加载 N 秒”、结算后等结果最多 60 秒）。
    fn update_stage(&self, native: &str, live_ready: bool) {
        let mut s = self.state();
        let started = s.current.as_ref().is_some_and(|m| m.started);
        let stage = stage_of(native, live_ready, started);
        if s.stage.map(|(p, _)| p) != stage {
            s.stage = stage.map(|p| (p, now_ms()));
        }
    }
}

/// 客户端自带的中文游戏资料（英雄、召唤师技能、装备、海克斯、队列，页面用的符文），约 0.1 秒。
async fn load_data(lcu: &Lcu) -> Option<GameData> {
    let (champions, spells, items, augments, queues, perks, styles) = tokio::join!(
        lcu.get(paths::CHAMPIONS),
        lcu.get(paths::SPELLS),
        lcu.get(paths::ITEMS),
        lcu.get(paths::AUGMENTS),
        lcu.get(paths::QUEUES),
        lcu.get(paths::PERKS),
        lcu.get(paths::PERK_STYLES),
    );
    let mut data = GameData::from_json(
        &champions.ok()?,
        &spells.unwrap_or(Value::Null),
        &items.unwrap_or(Value::Null),
        &augments.unwrap_or(Value::Null),
        &queues.unwrap_or(Value::Null),
    );
    data.add_perks(
        &perks.unwrap_or(Value::Null),
        &styles.unwrap_or(Value::Null),
    );
    (!data.is_empty()).then_some(data)
}

/// 客户端说这个人是电脑（gameflow / 选人 session 里的 `botDifficulty`、`isBot`、`playerType`）。没有这些标记的
/// 都当真人：没有 puuid 的真人是隐藏了身份（ADR 043 D9，2026-11-26 那一把海斗里我方两个人就是这样被当成电脑漏掉的）。
pub(crate) fn flagged_bot(m: &Value) -> bool {
    let difficulty = m["botDifficulty"].as_str().unwrap_or("").trim();
    m["isBot"].as_bool() == Some(true)
        || (!difficulty.is_empty() && !difficulty.eq_ignore_ascii_case("NONE"))
        || m["playerType"]
            .as_str()
            .is_some_and(|t| t.trim().eq_ignore_ascii_case("BOT"))
}

/// 客户端给的 puuid；空的、全是 0 的（LeagueAkari 的 `EMPTY_PUUID`）算没有。
pub(crate) fn real_puuid(m: &Value) -> String {
    let puuid = m["puuid"].as_str().unwrap_or("").trim();
    if puuid.chars().all(|c| c == '0' || c == '-') {
        String::new()
    } else {
        puuid.to_string()
    }
}

/// `gameflow/session` → 这局的人（局内时两边都有）。没有 puuid 的真人（隐藏了身份）和电脑也在里面，不查。
fn members(session: &Value) -> Vec<Member> {
    let data = &session["gameData"];
    let selections: Vec<&Value> = data["playerChampionSelections"]
        .as_array()
        .map(|a| a.iter().collect())
        .unwrap_or_default();
    let mut out = Vec::new();
    for (team, key) in [(1u8, "teamOne"), (2u8, "teamTwo")] {
        for m in data[key].as_array().into_iter().flatten() {
            let puuid = real_puuid(m);
            let identity = if flagged_bot(m) {
                text::Identity::Bot
            } else if puuid.is_empty() {
                text::Identity::Hidden
            } else {
                text::Identity::Known
            };
            let internal = m["summonerInternalName"].as_str().unwrap_or("");
            let selection = selections.iter().find(|x| {
                (!puuid.is_empty() && x["puuid"].as_str() == Some(puuid.as_str()))
                    || (!internal.is_empty()
                        && x["summonerInternalName"].as_str() == Some(internal))
            });
            let mut champion_id = m["championId"].as_i64().unwrap_or(0);
            if champion_id <= 0 {
                champion_id = selection
                    .and_then(|x| x["championId"].as_i64())
                    .unwrap_or(0);
            }
            let spell = |key: &str| selection.and_then(|x| x[key].as_i64()).unwrap_or(0);
            let position = m["selectedPosition"].as_str().unwrap_or("").trim();
            out.push(Member {
                champion_id,
                team,
                party: m["teamParticipantId"].as_i64(),
                profile_icon_id: m["profileIconId"].as_i64(),
                spells: [spell("spell1Id"), spell("spell2Id")],
                position: if position.eq_ignore_ascii_case("NONE") {
                    String::new()
                } else {
                    position.to_ascii_uppercase()
                },
                // 电脑的 puuid 不查（有的模式里电脑也带一个）
                puuid: if identity == text::Identity::Bot {
                    String::new()
                } else {
                    puuid
                },
                pick: pages::Pick::Locked,
                identity,
            });
        }
    }
    out
}

/// 进了加载以后每一轮把 gameflow 的名单 `found` 合进这局（[`LolPlugin::on_session`]），返回要开始查的 puuid（已经记成“在查”）。
/// 顺序跟着 gameflow；英雄可能晚一点才填上（加载界面前后），每轮都按最新的；之前有、这一轮没出现的查得到的人留着
/// （查过的战绩接着用）。选人时看不到名字的人（对面、排位里的队友）按 gameflow 重来；局内接口补上的“电脑”留着。
fn merge_session(m: &mut Match, found: Vec<Member>) -> Vec<String> {
    let old = std::mem::take(&mut m.members);
    let mut next = found;
    for member in next.iter_mut() {
        let was_bot = old.iter().any(|x| {
            x.identity == text::Identity::Bot
                && x.team == member.team
                && x.champion_id == member.champion_id
        });
        if member.identity == text::Identity::Hidden && was_bot {
            member.identity = text::Identity::Bot;
        }
    }
    for x in old {
        if x.identity == text::Identity::Known
            && !x.puuid.is_empty()
            && !next.iter().any(|n| n.puuid == x.puuid)
        {
            next.push(x);
        }
    }
    m.members = next;
    let missing: Vec<String> = m
        .members
        .iter()
        .filter(|x| x.identity == text::Identity::Known)
        .map(|x| x.puuid.clone())
        .filter(|p| !p.is_empty() && !m.histories.contains_key(p) && !m.pending.contains(p))
        .collect();
    // 先记成“在查”，免得下一轮重复查
    m.pending.extend(missing.iter().cloned());
    missing
}

/// 局内接口的 `isBot`（[`LivePlayer::bot`]）：没有 puuid 的人在局内接口里是电脑，就改成电脑。按队伍（teamOne = ORDER）
/// 和英雄（英文 id）对上；对不上的不动。
fn mark_bots(members: &mut [Member], game: &LiveGame, data: Option<&GameData>) {
    let Some(data) = data else { return };
    for member in members
        .iter_mut()
        .filter(|x| x.identity == text::Identity::Hidden)
    {
        let side = if member.team == 1 { "ORDER" } else { "CHAOS" };
        let Some(alias) = data.champion(member.champion_id).map(|c| c.alias.as_str()) else {
            continue;
        };
        if game
            .players
            .iter()
            .any(|p| p.bot && p.team == side && p.alias.eq_ignore_ascii_case(alias))
        {
            member.identity = text::Identity::Bot;
        }
    }
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

/// 一个人：隐私设置、最近 20 局（重开、自定义不算）、单双排段位。隐藏战绩的照样读（D6）。
async fn fetch_history(lcu: &Lcu, puuid: &str) -> PlayerHistory {
    let mut h = PlayerHistory::default();
    if let Ok(summoner) = lcu.get(&paths::summoner(puuid)).await {
        h.privacy = summoner["privacy"].as_str().unwrap_or("").to_string();
        h.profile_icon_id = summoner["profileIconId"].as_i64();
        h.summoner_level = summoner["summonerLevel"].as_i64().unwrap_or(0);
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
    match lcu.get(&paths::matches(puuid)).await {
        Ok(list) => {
            h.games = history::parse_match_list(&list);
            h.games.truncate(HISTORY_GAMES);
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
        h.solo = history::parse_ranked(&ranked, history::SOLO);
        h.flex = history::parse_ranked(&ranked, history::FLEX);
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

/// 局内接口还没有游戏（加载界面）。
fn loading() -> Value {
    let mut out = status("not_in_game", "游戏还在加载，进了游戏再问");
    out["reason"] = json!("loading");
    out
}

/// 现在的 Unix 毫秒。
fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// “这局”页面的阶段：客户端阶段 + 局内接口有没有游戏时间 + 这局进没进过加载。不在这几个阶段、也没打过这局时是 None。
fn stage_of(native: &str, live_ready: bool, started: bool) -> Option<pages::MatchPhase> {
    use pages::MatchPhase as P;
    if native == "ChampSelect" {
        Some(P::ChampSelect)
    } else if in_game_phase(native) {
        Some(if live_ready { P::InGame } else { P::Loading })
    } else if matches!(native, "WaitingForStats" | "PreEndOfGame" | "EndOfGame") || started {
        // 打完回到大厅：页面留着上一局（设计第四节 1）
        started.then_some(P::PostMatch)
    } else {
        None
    }
}

impl LolPlugin {
    /// 不在对局里时怎么说。
    fn not_in_game(&self, host: &dyn Host) -> Value {
        let (phase, client) = {
            let s = self.state();
            (s.phase.clone(), s.client.is_some())
        };
        // `reason` 给页面选翻译好的文案（App 的 status.ts）；给她的话不变
        let (reason, text) = if in_game_phase(&phase) {
            ("loading", "游戏还在加载，进了游戏再问")
        } else if phase == "ChampSelect" {
            ("champ_select", "还在选英雄；进了加载界面才查得到这局的人")
        } else if client || client_pid(host).is_some() {
            ("not_in_game", "现在不在英雄联盟的对局里")
        } else {
            ("client_closed", "英雄联盟没开")
        };
        let mut out = status("not_in_game", text);
        out["reason"] = json!(reason);
        out
    }

    /// 局内数据；不在对局里时 Err(给模型的结果)。
    async fn live_game(&self, host: &dyn Host) -> Result<LiveGame, Value> {
        let live = live_client().map_err(|e| status("error", e))?;
        match live.all_game_data().await {
            Ok(v) => live::parse_all_game_data(&v).ok_or_else(loading),
            Err(LolError::Unavailable) => Err(self.not_in_game(host)),
            Err(LolError::Status(404, _)) => Err(loading()),
            Err(error) => Err(status("error", format!("读局内数据失败：{error}"))),
        }
    }

    /// `lol.match` / `lol.spell` / `lol.scout` / `lol.shout`……。结果只有给她的那份（页面在第 2 步加）。
    pub(crate) async fn tool(
        &self,
        host: &Arc<dyn Host>,
        name: &str,
        arguments: &Value,
    ) -> Result<ToolResult, String> {
        // 网上的统计不要客户端（开关各管各的）
        match name {
            TOOL_CHAMPIONS => return Ok(self.champions_tool(host.as_ref()).await),
            TOOL_CHAMPION => return Ok(self.champion_tool(host.as_ref(), arguments).await),
            TOOL_TIER => return Ok(self.tier_tool(host.as_ref(), arguments).await),
            // 来源标签选的数据源（ADR 043 D8）：只记设置，不要客户端
            TOOL_SOURCE => return Ok(crate::stats_tools::source_tool(host.as_ref(), arguments)),
            // 海克斯三选一要的本地文字识别模型（下载 / 不用）：不要客户端
            crate::tools::TOOL_AUGMENT_OCR => {
                return Ok(self.augment_ocr_tool(host, arguments).await)
            }
            // 选人卡片上的一键导入（ADR 043 B 轮）：自己查“操作客户端”和“读客户端数据”
            crate::tools::TOOL_IMPORT => {
                return Ok(self.import_tool(host.as_ref(), arguments).await)
            }
            _ => {}
        }
        if !enabled(host.as_ref()) {
            let off = "英雄联盟插件的“读客户端数据”已关闭（插件 → 英雄联盟 里可以打开）";
            if name == TOOL_SHOUT {
                // 喊话没喊出去就是没成（热键那边照“没成”告诉玩家）
                return Err(format!("{off}：这次没发"));
            }
            return Ok(ToolResult::content(status("disabled", off)));
        }
        if name == TOOL_SHOUT {
            return self.shout_tool(host.as_ref(), arguments).await;
        }
        Ok(match name {
            TOOL_MATCH => self.match_tool(host.as_ref()).await,
            TOOL_SPELL => ToolResult::content(self.spell_tool(host.as_ref(), arguments).await),
            TOOL_SCOUT => self.scout_tool(host, arguments).await,
            TOOL_PLAYER => self.player_tool(host.as_ref(), arguments).await,
            TOOL_CLIENT => match self.lcu_or_closed(host.as_ref()) {
                Ok(lcu) => self.client_tool(host.as_ref(), &lcu, arguments).await,
                Err(result) => ToolResult::content(result),
            },
            TOOL_GAME => self.game_tool(host.as_ref(), arguments).await,
            TOOL_HISTORY => self.history_page_tool(host.as_ref(), arguments).await,
            other => return Err(format!("{other} 不是英雄联盟工具")),
        })
    }

    /// `lol.match`：给她的几行字（不带玩家名），加上给程序的结构化数据（ADR 046 D8，字段见 [`text::match_data`]；
    /// 没有页面，宿主不把它放进她的上下文、也不出卡片）。
    async fn match_tool(&self, host: &dyn Host) -> ToolResult {
        let game = match self.live_game(host).await {
            Ok(game) => game,
            Err(result) => return with_program_data(result),
        };
        let data = self.ensure_data().await;
        let tracker = self.state().tracker.clone();
        ToolResult {
            content: json!({"status": "ok", "text": text::render_match(&game, data.as_deref(), &tracker)}),
            structured_content: Some(text::match_data(&game, data.as_deref(), &tracker)),
            ui: None,
        }
    }
}

/// 没拿到这局（不在对局里、读不到）时给程序的那份：`{status, reason}`。不在对局里的 `reason` 是
/// `loading` / `champ_select` / `not_in_game` / `client_closed`；别的（`status = "error"`）是给她的那句原因。
fn with_program_data(content: Value) -> ToolResult {
    let reason = content
        .get("reason")
        .or_else(|| content.get("text"))
        .cloned()
        .unwrap_or(Value::Null);
    let program = json!({"status": content["status"], "reason": reason});
    ToolResult {
        content,
        structured_content: Some(program),
        ui: None,
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
        // 选人被秒退、只剩上一次选人的名单：不算这局
        if self.state().current.is_none() || self.state().stage.is_none() {
            return ToolResult::content(self.not_in_game(host.as_ref()));
        }
        let Some(view) = self.match_view(data.as_deref()) else {
            let mut out = status("not_in_game", "这局刚结束");
            out["reason"] = json!("ended");
            return ToolResult::content(out);
        };
        // 选人时只说我方（对面只有英雄，查不到人）
        let text = if view.stage.phase == Some(pages::MatchPhase::ChampSelect) {
            let allies: Vec<ScoutPlayer> =
                view.players.iter().filter(|p| p.ally).cloned().collect();
            text::render_scout(&allies, view.queue_id, &view.mode, view.random, only)
        } else {
            text::render_scout(&view.players, view.queue_id, &view.mode, view.random, only)
        };
        ToolResult::content(json!({"status": "ok", "text": text})).with_page(
            pages::PAGE_MATCH,
            pages::match_page_in(
                &view.players,
                &view.puuids,
                view.game_id,
                view.queue_id,
                &view.mode,
                view.random,
                data.as_deref(),
                &view.stage,
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
            .find(|x| !me.is_empty() && x.puuid == me)
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
                    me: !me.is_empty() && x.puuid == me,
                    party: x.party,
                    history,
                    profile_icon_id: x.profile_icon_id,
                    spells: x.spells,
                    position: x.position.clone(),
                    identity: x.identity,
                }
            })
            .collect();
        let mode = data
            .and_then(|d| d.queue_name(m.queue_id))
            .map(str::to_string)
            .unwrap_or_else(|| text::mode_name(&m.mode).to_string());
        // “这局”页面的阶段（设计第三节 1 的四种状态）；结算后最多等 60 秒结果
        let (phase, since) = s
            .stage
            .map(|(p, t)| (Some(p), Some(t)))
            .unwrap_or((None, None));
        let result_pending = phase == Some(pages::MatchPhase::PostMatch)
            && m.result.is_none()
            && since.is_some_and(|t| now_ms() - t < RESULT_WAIT_MS);
        let stage = pages::MatchStage {
            phase,
            since_ms: since,
            names_hidden: m.names_hidden,
            bench: if phase == Some(pages::MatchPhase::ChampSelect) {
                m.bench.clone()
            } else {
                Vec::new()
            },
            picks: m.members.iter().map(|x| x.pick).collect(),
            result: m.result,
            result_pending,
        };
        Some(MatchView {
            puuids: m.members.iter().map(|x| x.puuid.clone()).collect(),
            players,
            game_id: m.game_id,
            queue_id: m.queue_id,
            random: crate::client::gamedata::random_pick(data, m.queue_id),
            mode,
            stage,
        })
    }

    /// 素材层的入口（ADR 039 D6，[`crate::client::assets`]）：页面要的一张图（英雄头像、段位徽章……），
    /// `path` 是页面数据里的客户端路径（前面的 `/` 可有可无）。插件和“读客户端数据”开着、后台已经连上客户端时
    /// 先问客户端，不然问 CommunityDragon；磁盘缓存在插件目录下的 `asset-cache/`。一页几十张图：这里不去找客户端
    /// 进程（后台每 2–5 秒找一次），只用已经连上的。
    pub async fn fetch_asset(&self, host: &dyn Host, path: &str) -> Result<Asset, AssetError> {
        static ASSETS: OnceLock<Assets> = OnceLock::new();
        let assets = ASSETS.get_or_init(|| {
            let assets = Assets::new(host.data_dir().join("asset-cache"));
            // 测试用（KOTONE_LOL_CDRAGON）：换成本机的假 CommunityDragon
            let Ok(value) = std::env::var("KOTONE_LOL_CDRAGON") else {
                return assets;
            };
            match loopback_base(&value) {
                Some(base) => {
                    log::log(&format!("lol: KOTONE_LOL_CDRAGON：素材改问 {base}"));
                    assets.with_online(CommunityDragon::with_base(&base))
                }
                None => {
                    log::log("lol: KOTONE_LOL_CDRAGON 不是本机的 http(s) 地址，素材不连网");
                    assets.with_online(Offline)
                }
            }
        });
        let lcu = self.connected_lcu(host);
        assets
            .fetch(lcu.as_ref().map(|l| l as &dyn Origin), path)
            .await
    }

    /// 后台已经连上的客户端（插件和“读客户端数据”开着时；不去找进程）。素材层、英雄技能用。
    pub(crate) fn connected_lcu(&self, host: &dyn Host) -> Option<Lcu> {
        if enabled(host) {
            self.state().client.as_ref().map(|c| c.lcu.clone())
        } else {
            None
        }
    }

    /// 这局的队列（不在对局里时 None）。
    pub(crate) fn current_queue(&self) -> Option<i64> {
        self.state().current.as_ref().map(|m| m.queue_id)
    }

    /// 这局的对局 id（不在对局里时 None）。
    /// 记下的召唤师技能（`lol.spell`，你或她说了才记）：对局中的提醒看哪个转好了（ADR 043 B 轮）。
    pub(crate) fn spell_uses(&self) -> Vec<SpellUse> {
        self.state().tracker.all().to_vec()
    }

    pub(crate) fn current_game_id(&self) -> Option<i64> {
        self.state().current.as_ref().map(|m| m.game_id)
    }

    /// 已经读好的客户端游戏资料（没读时 None，不去读）。
    pub(crate) fn loaded_data(&self) -> Option<Arc<GameData>> {
        self.state().data.clone()
    }

    /// 连着的客户端（没开时给她的话）。
    pub(crate) fn lcu_or_closed(&self, host: &dyn Host) -> Result<Lcu, Value> {
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
        let include_background = arguments["includeProfileBackground"].as_bool() == Some(true)
            && str_arg(arguments, "puuid").is_none()
            && str_arg(arguments, "player").is_none();
        let (history, background) = tokio::join!(
            async {
                match cached {
                    Some(h) => h,
                    None => fetch_history(&lcu, &puuid).await,
                }
            },
            async {
                if include_background {
                    // 装饰图查询和资料并行；客户端忙碌时仍及时显示战绩。
                    tokio::time::timeout(
                        Duration::from_secs(3),
                        profile_background::resolve(&lcu, &puuid),
                    )
                    .await
                    .ok()
                    .flatten()
                } else {
                    None
                }
            }
        );
        let queue_id = self
            .state()
            .current
            .as_ref()
            .map(|m| m.queue_id)
            .unwrap_or(0);
        let text = pages::player_text(&history, data.as_deref());
        let mut page = pages::player_page(&puuid, &history, queue_id, data.as_deref());
        if include_background {
            page["profileBackground"] = json!(background);
        }
        ToolResult::content(json!({"status": "ok", "text": text}))
            .with_page(pages::PAGE_PLAYER, page)
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
        let me = self.state().client.as_ref().and_then(|c| c.me.clone());
        match pages::game_page_for(&detail, data.as_deref(), me.as_deref()) {
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
    /// 阶段和那个阶段才有的东西（页面用）。
    stage: pages::MatchStage,
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
    use super::*;

    /// “这局”页面的四种状态（设计第三节 1）从客户端阶段、局内接口有没有游戏时间、这局进没进过加载来。
    #[test]
    fn the_page_stage_follows_the_client_phase() {
        use pages::MatchPhase as P;
        assert_eq!(stage_of("ChampSelect", false, false), Some(P::ChampSelect));
        assert_eq!(stage_of("InProgress", false, true), Some(P::Loading));
        assert_eq!(stage_of("GameStart", false, true), Some(P::Loading));
        assert_eq!(stage_of("InProgress", true, true), Some(P::InGame));
        assert_eq!(stage_of("EndOfGame", false, true), Some(P::PostMatch));
        // 打完回到大厅：页面留着上一局；选人被秒退（没进过加载）：不算这局
        assert_eq!(stage_of("Lobby", false, true), Some(P::PostMatch));
        assert_eq!(stage_of("Lobby", false, false), None);
        assert_eq!(stage_of("None", false, false), None);
    }

    /// `lol.match` 没拿到这局时（ADR 046 D8）：给她的照旧，给程序的是 `{status, reason}`；读不到时 reason 是原因。
    #[test]
    fn match_without_a_game_gives_programs_status_and_reason() {
        let out = with_program_data(loading());
        assert_eq!(out.content, loading());
        assert_eq!(
            out.structured_content,
            Some(json!({"status": "not_in_game", "reason": "loading"}))
        );
        assert!(out.ui.is_none(), "没有页面：不进她的上下文、不出卡片");
        let mut closed = status("not_in_game", "英雄联盟没开");
        closed["reason"] = json!("client_closed");
        assert_eq!(
            with_program_data(closed).structured_content,
            Some(json!({"status": "not_in_game", "reason": "client_closed"}))
        );
        assert_eq!(
            with_program_data(status("error", "读局内数据失败：超时")).structured_content,
            Some(json!({"status": "error", "reason": "读局内数据失败：超时"}))
        );
    }

    #[test]
    fn query_values_are_percent_encoded() {
        assert_eq!(encode("abc-1"), "abc-1");
        assert_eq!(encode("琴音 #"), "%E7%90%B4%E9%9F%B3%20%23");
    }

    /// 素材层的入口能放进宿主要的 `BoxFuture`（`Send`）：编译过就算过。
    #[allow(dead_code)]
    fn fetch_asset_is_send<'a>(
        plugin: &'a LolPlugin,
        host: &'a dyn Host,
    ) -> kotone_plugin_api::BoxFuture<'a, Result<Asset, AssetError>> {
        Box::pin(plugin.fetch_asset(host, "lol-game-data/assets/v1/champion-icons/1.png"))
    }
}
