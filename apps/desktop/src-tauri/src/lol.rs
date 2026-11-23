//! 英雄联盟工具（ADR 035）：读本机的客户端接口（LCU）和局内接口（Live Client Data），都只读。
//!
//! - 客户端开着时每 2 秒看一次对局阶段；进加载界面就在后台查这局 10 个人的战绩（同时 2 个、每人 20 局），
//!   她问“查战绩”时多半已经查好；
//! - `lol.match` / `lol.spell` 读局内接口（马上返回）；召唤师技能是玩家说了才记，这局结束清掉；
//! - 端口和令牌从 `LeagueClientUx.exe` 的命令行读（不用管理员）；令牌不进日志。
//! - 设置 `lol.enabled` 关掉时什么都不读（D5）。
//! - 测试用（不开游戏，`scripts/manual-e2e/lol_tools_real.py`）：`KOTONE_LOL_LCU=<端口>:<令牌>` 不找客户端进程、
//!   直接连 127.0.0.1 的这个端口；`KOTONE_LOL_LIVE=<地址>` 换掉局内接口的地址（只认本机）。

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use kotone_agent::lol::{TOOL_MATCH, TOOL_SCOUT, TOOL_SPELL};
use kotone_core::log;
use kotone_lol::client::paths;
use kotone_lol::client::{loopback_base, LIVE_BASE};
use kotone_lol::gamedata::normalize;
use kotone_lol::history::{self, PlayerHistory};
use kotone_lol::live::{self, LiveGame, LivePlayer};
use kotone_lol::spells::{self, SpellTracker, SpellUse};
use kotone_lol::text::{self, ScoutPlayer};
use kotone_lol::{in_game_phase, GameData, Lcu, LcuAuth, Live, LolError};
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

/// 客户端开着时多久看一次阶段；没开时多久看一次进程。
const POLL: Duration = Duration::from_secs(2);
const IDLE_POLL: Duration = Duration::from_secs(5);
/// 每人查几局。
const HISTORY_GAMES: u32 = 20;
/// 同时查几个人（照 LeagueAkari）。
const CONCURRENCY: usize = 2;
/// 她问的时候还没查完，最多等多久。
const SCOUT_WAIT: Duration = Duration::from_secs(12);

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
struct State {
    client: Option<Client>,
    phase: String,
    data: Option<Arc<GameData>>,
    loading_data: bool,
    current: Option<Match>,
    tracker: SpellTracker,
}

fn state() -> &'static Mutex<State> {
    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    STATE.get_or_init(Default::default)
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
        let auth = kotone_lol::parse_port_token(&value);
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

fn enabled(app: &AppHandle) -> bool {
    app.state::<crate::SharedState>()
        .settings
        .read()
        .unwrap()
        .lol
        .enabled
}

/// 启动时：开始盯客户端（不阻塞启动）。
pub fn init(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            let open = tick(&app).await;
            tokio::time::sleep(if open { POLL } else { IDLE_POLL }).await;
        }
    });
}

fn reset_all() {
    let mut s = state().lock().unwrap();
    if s.client.is_some() {
        log::log("lol: 英雄联盟客户端关了");
    }
    *s = State::default();
}

fn client_pid() -> Option<u32> {
    if let Some(test) = test_lcu() {
        return test.map(|_| 0);
    }
    kotone_platform_windows::app_audio::pids_by_name(kotone_lol::CLIENT_PROCESS)
        .first()
        .copied()
}

/// 连上客户端（进程换了就重新读命令行）。
fn ensure_client(pid: u32) -> Option<Lcu> {
    if let Some(c) = state()
        .lock()
        .unwrap()
        .client
        .as_ref()
        .filter(|c| c.pid == pid)
    {
        return Some(c.lcu.clone());
    }
    let auth = match test_lcu() {
        Some(test) => test?.clone(),
        None => kotone_platform_windows::process_cmdline::command_line(pid)
            .as_deref()
            .and_then(kotone_lol::parse_command_line)?,
    };
    let lcu = Lcu::new(&auth).ok()?;
    log::log(&format!(
        "lol: 连上英雄联盟客户端（端口 {}，{} {}）",
        auth.port, auth.region, auth.platform
    ));
    let mut s = state().lock().unwrap();
    *s = State::default();
    s.client = Some(Client {
        pid,
        lcu: lcu.clone(),
        me: None,
    });
    Some(lcu)
}

/// 一轮；返回客户端是不是开着。
async fn tick(app: &AppHandle) -> bool {
    if !enabled(app) {
        reset_all();
        return false;
    }
    let Some(pid) = client_pid() else {
        reset_all();
        return false;
    };
    let Some(lcu) = ensure_client(pid) else {
        return false;
    };
    let phase = match lcu.get(paths::PHASE).await {
        Ok(Value::String(phase)) => phase,
        Ok(_) => String::new(),
        Err(error) => {
            // 客户端重启了（端口变了）：下一轮重新读命令行
            log::log(&format!("lol: 读对局阶段失败：{error}"));
            state().lock().unwrap().client = None;
            return true;
        }
    };
    let previous = std::mem::replace(&mut state().lock().unwrap().phase, phase.clone());
    load_data_in_background(&lcu);
    if in_game_phase(&phase) {
        if let Ok(session) = lcu.get(paths::SESSION).await {
            on_session(&lcu, &session).await;
        }
    } else if in_game_phase(&previous) {
        // 这局结束：技能记录清掉；战绩留着，下一局开始时换掉
        state().lock().unwrap().tracker = SpellTracker::default();
    }
    true
}

fn load_data_in_background(lcu: &Lcu) {
    {
        let mut s = state().lock().unwrap();
        if s.data.is_some() || s.loading_data {
            return;
        }
        s.loading_data = true;
    }
    let lcu = lcu.clone();
    tauri::async_runtime::spawn(async move {
        let data = load_data(&lcu).await;
        let mut s = state().lock().unwrap();
        s.loading_data = false;
        if let Some(data) = data {
            s.data = Some(Arc::new(data));
        }
    });
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

/// 游戏资料：还没读就现在读（工具调用时）。
async fn ensure_data() -> Option<Arc<GameData>> {
    let lcu = {
        let s = state().lock().unwrap();
        if let Some(data) = &s.data {
            return Some(data.clone());
        }
        s.client.as_ref()?.lcu.clone()
    };
    let data = Arc::new(load_data(&lcu).await?);
    state().lock().unwrap().data = Some(data.clone());
    Some(data)
}

async fn my_puuid(lcu: &Lcu) -> Option<String> {
    // 先取出来再判断：锁不能跨过下面的 await
    let cached = state()
        .lock()
        .unwrap()
        .client
        .as_ref()
        .and_then(|c| c.me.clone());
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
    if let Some(c) = state().lock().unwrap().client.as_mut() {
        c.me = Some(me.clone());
    }
    Some(me)
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

async fn on_session(lcu: &Lcu, session: &Value) {
    let game_id = session["gameData"]["gameId"].as_i64().unwrap_or(0);
    if game_id == 0 {
        return;
    }
    let found = members(session);
    let missing: Vec<String> = {
        let mut s = state().lock().unwrap();
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
    let _ = my_puuid(lcu).await;
    log::log(&format!("lol: 这局开始查 {} 个人的战绩", missing.len()));
    tauri::async_runtime::spawn(prefetch(lcu.clone(), game_id, missing));
}

async fn prefetch(lcu: Lcu, game_id: i64, puuids: Vec<String>) {
    let started = Instant::now();
    let semaphore = Arc::new(tokio::sync::Semaphore::new(CONCURRENCY));
    let mut tasks = Vec::new();
    for puuid in puuids {
        let lcu = lcu.clone();
        let semaphore = semaphore.clone();
        tasks.push(tauri::async_runtime::spawn(async move {
            let _permit = semaphore.acquire_owned().await.ok();
            let history = fetch_history(&lcu, &puuid).await;
            let mut s = state().lock().unwrap();
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

/// 不在对局里时怎么说。
fn not_in_game() -> Value {
    let (phase, client) = {
        let s = state().lock().unwrap();
        (s.phase.clone(), s.client.is_some())
    };
    let text = if in_game_phase(&phase) {
        "游戏还在加载，进了游戏再问"
    } else if phase == "ChampSelect" {
        "还在选英雄；进了加载界面才查得到这局的人"
    } else if client || client_pid().is_some() {
        "现在不在英雄联盟的对局里"
    } else {
        "英雄联盟没开"
    };
    status("not_in_game", text)
}

/// 局内数据；不在对局里时 Err(给模型的结果)。
async fn live_game() -> Result<LiveGame, Value> {
    let live = live_client().map_err(|e| status("error", e))?;
    match live.all_game_data().await {
        Ok(v) => live::parse_all_game_data(&v)
            .ok_or_else(|| status("not_in_game", "游戏还在加载，进了游戏再问")),
        Err(LolError::Unavailable) => Err(not_in_game()),
        Err(LolError::Status(404, _)) => Err(status("not_in_game", "游戏还在加载，进了游戏再问")),
        Err(error) => Err(status("error", format!("读局内数据失败：{error}"))),
    }
}

/// `lol.match` / `lol.spell` / `lol.scout`。
pub async fn tool(app: &AppHandle, name: &str, arguments: &Value) -> Result<Value, String> {
    if !enabled(app) {
        return Ok(status(
            "disabled",
            "英雄联盟对局数据已关闭（游戏列表 → 英雄联盟 里可以打开）",
        ));
    }
    Ok(match name {
        TOOL_MATCH => match_tool().await,
        TOOL_SPELL => spell_tool(arguments).await,
        TOOL_SCOUT => scout_tool(app, arguments).await,
        other => return Err(format!("{other} 不是英雄联盟工具")),
    })
}

async fn match_tool() -> Value {
    let game = match live_game().await {
        Ok(game) => game,
        Err(result) => return result,
    };
    let data = ensure_data().await;
    let tracker = state().lock().unwrap().tracker.clone();
    json!({"status": "ok", "text": text::render_match(&game, data.as_deref(), &tracker)})
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

async fn spell_tool(arguments: &Value) -> Value {
    let clear = arguments.get("clear").and_then(Value::as_bool) == Some(true);
    let champion = str_arg(arguments, "champion");
    let spell = spell_name(str_arg(arguments, "spell").unwrap_or("闪现")).to_string();
    if clear && champion.is_none() {
        let n = state().lock().unwrap().tracker.clear(None, None);
        return status("ok", format!("记下的 {n} 条技能都清掉了"));
    }
    let Some(champion) = champion else {
        return status("error", "要说是哪个英雄交的（champion）");
    };
    let game = match live_game().await {
        Ok(game) => game,
        Err(result) => return result,
    };
    let data = ensure_data().await;
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
        let n = state().lock().unwrap().tracker.clear(
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
    state().lock().unwrap().tracker.record(entry);
    let how = if notes.is_empty() {
        String::new()
    } else {
        format!("（{}）", notes.join("，"))
    };
    json!({
        "status": "ok",
        "text": format!(
            "记下了：{name} {} 交的{have}，{} 好{how}",
            kotone_lol::clock(used_at),
            kotone_lol::clock(ready_at)
        ),
        "chat": chat,
        "ready": kotone_lol::clock(ready_at),
    })
}

async fn scout_tool(app: &AppHandle, arguments: &Value) -> Value {
    let only = str_arg(arguments, "player");
    let has_match = state().lock().unwrap().current.is_some();
    if !has_match {
        // 工具可能比后台那一轮先到：现在看一次
        tick(app).await;
    }
    let deadline = Instant::now() + SCOUT_WAIT;
    loop {
        // 先把数取出来、放开锁，再决定（not_in_game 自己也要拿锁）
        let waiting = state()
            .lock()
            .unwrap()
            .current
            .as_ref()
            .map(|m| m.pending.len());
        match waiting {
            None => return not_in_game(),
            Some(0) => break,
            Some(_) if Instant::now() >= deadline => break,
            Some(_) => tokio::time::sleep(Duration::from_millis(300)).await,
        }
    }
    let data = ensure_data().await;
    if state().lock().unwrap().current.is_none() {
        return not_in_game();
    }
    let s = state().lock().unwrap();
    let Some(m) = s.current.as_ref() else {
        return status("not_in_game", "这局刚结束");
    };
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
            let champion = data.as_deref().and_then(|d| d.champion(x.champion_id));
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
        .as_deref()
        .and_then(|d| d.queue_name(m.queue_id))
        .map(str::to_string)
        .unwrap_or_else(|| text::mode_name(&m.mode).to_string());
    let random = kotone_lol::gamedata::random_pick(data.as_deref(), m.queue_id);
    json!({"status": "ok", "text": text::render_scout(&players, m.queue_id, &mode, random, only)})
}
