//! 局内接口的事件列表 → 游戏事件（ADR 036 的 `lol.*`），以及客户端阶段 → 所有游戏共用的阶段。
//!
//! - 只报新的（`EventID` 比上次大的）；`team` 是相对玩家的 `ally` / `enemy`；
//! - 塔和水晶按建筑名里的 `T1` / `T2` 判断是谁推的（小兵推的塔没有英雄）；
//! - `data` 里只放英雄名，不放玩家名（别人起的名字不进自动化的入参）。
//! - **等级、装备**（ADR 043 B 轮）：局内接口的事件列表里没有，比前后两份局内数据得出（[`Roster`]、[`changes`]）。
//!   为了不让事件太多，只报**对面和自己**：等级——自己每升一级都报，对面只报到 6、11、16 级（大招升级的那几级）；
//!   装备——出了一件成装（不再合成别的、总价 2000 以上，客户端的装备资料里查；资料没读到时只认 [`KEY_ITEM_IDS`]），
//!   同一个人同一件只报一次。第一次看到这局时只记下当时的样子，之前的不补。
//! - **阵亡、复活**（ADR 046 D6、D7，[`death_events`]）：也是比前后两份局内数据，`isDead` 由假变真的每个人报一件
//!   `lol.death`（两边都报），同一段时间里这个人的 `ChampionKill` 补上击杀者；“我阵亡了 / 我复活了”由同一处报。

use std::collections::{HashMap, HashSet};

use serde_json::{json, Map, Value};

use crate::client::gamedata::GameData;
use crate::client::live::{LiveEvent, LiveGame, LiveItem, LivePlayer};

/// 客户端阶段（`/lol-gameflow/v1/gameflow-phase`）→ 所有游戏共用的阶段（ADR 036 D3）。
/// `live_ready` = 局内接口已经有数据（游戏时间 > 0），这时才算“对局中”，之前是“加载”。
pub fn generic_phase(native: &str, live_ready: bool) -> &'static str {
    match native {
        "Matchmaking" | "ReadyCheck" => "queue",
        "ChampSelect" => "pre_match",
        "GameStart" | "InProgress" | "Reconnect" => {
            if live_ready {
                "in_match"
            } else {
                "loading"
            }
        }
        "WaitingForStats" | "PreEndOfGame" | "EndOfGame" => "post_match",
        _ => "menu",
    }
}

/// 一件新的事：（`EventID`，类型，数据，游戏时间）。
pub type NewEvent = (i64, &'static str, Map<String, Value>, f64);

fn s<'a>(e: &'a Value, key: &str) -> &'a str {
    e.get(key).and_then(Value::as_str).unwrap_or("")
}

/// 这个名字（玩家名；隐藏了身份的人是 `summonerName`，[`LiveGame::find`]）的英雄和是哪边。
fn who(game: &LiveGame, name: &str) -> (String, &'static str) {
    let team = match game.is_ally_name(name) {
        Some(true) => "ally",
        Some(false) => "enemy",
        None => "",
    };
    let champion = game
        .find(name)
        .map(|p| p.champion.clone())
        .unwrap_or_default();
    (champion, team)
}

/// 建筑名里的 `T1`（ORDER）/ `T2`（CHAOS）→ 推掉它的是哪边。
fn structure_destroyer(game: &LiveGame, structure: &str) -> &'static str {
    let owner = if structure.contains("_T1_") {
        "ORDER"
    } else if structure.contains("_T2_") {
        "CHAOS"
    } else {
        return "";
    };
    match game.my_team() {
        "" => "",
        mine if mine == owner => "enemy",
        _ => "ally",
    }
}

fn team_side(game: &LiveGame, team: &str) -> &'static str {
    match game.my_team() {
        "" => "",
        mine if mine.eq_ignore_ascii_case(team) => "ally",
        _ if team.is_empty() => "",
        _ => "enemy",
    }
}

fn dragon(kind: &str) -> String {
    kind.to_ascii_lowercase()
}

fn object(v: Value) -> Map<String, Value> {
    v.as_object().cloned().unwrap_or_default()
}

/// `allgamedata` 里 `EventID` 大于 `after` 的事 → 游戏事件（旧的在前）。认不出的跳过。
pub fn new_events(all_game_data: &Value, game: &LiveGame, after: i64) -> Vec<NewEvent> {
    let Some(list) = all_game_data
        .get("events")
        .and_then(|e| e.get("Events"))
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for e in list {
        let id = e.get("EventID").and_then(Value::as_i64).unwrap_or(-1);
        if id <= after {
            continue;
        }
        let time = e.get("EventTime").and_then(Value::as_f64).unwrap_or(0.0);
        let stolen = s(e, "Stolen").eq_ignore_ascii_case("true");
        let (killer, team) = who(game, s(e, "KillerName"));
        let found: Option<(&'static str, Value)> = match s(e, "EventName") {
            "FirstBlood" => {
                let (champion, team) = who(game, s(e, "Recipient"));
                Some(("lol.first_blood", json!({"team": team, "killer": champion})))
            }
            "DragonKill" => Some((
                "lol.dragon_kill",
                json!({"team": team, "dragon": dragon(s(e, "DragonType")),
                    "dragonName": crate::client::text::dragon(s(e, "DragonType")), "stolen": stolen}),
            )),
            "HeraldKill" => Some(("lol.herald_kill", json!({"team": team, "stolen": stolen}))),
            "BaronKill" => Some(("lol.baron_kill", json!({"team": team, "stolen": stolen}))),
            "TurretKilled" => Some((
                "lol.turret_killed",
                json!({"team": structure_destroyer(game, s(e, "TurretKilled"))}),
            )),
            "InhibKilled" => Some((
                "lol.inhib_killed",
                json!({"team": structure_destroyer(game, s(e, "InhibKilled"))}),
            )),
            "Ace" => Some((
                "lol.ace",
                json!({"team": team_side(game, s(e, "AcingTeam"))}),
            )),
            "Multikill" => {
                let count = e.get("KillStreak").and_then(Value::as_i64).unwrap_or(0);
                (count >= 3).then(|| {
                    (
                        "lol.multikill",
                        json!({"team": team, "killer": killer, "count": count}),
                    )
                })
            }
            _ => None,
        };
        if let Some((kind, data)) = found {
            out.push((id, kind, object(data), time));
        }
    }
    out
}

/// 最新的 `EventID`（第一次连上时用它当起点，之前发生的不补）。
pub fn last_event_id(all_game_data: &Value) -> i64 {
    all_game_data
        .get("events")
        .and_then(|e| e.get("Events"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|e| e.get("EventID").and_then(Value::as_i64))
        .max()
        .unwrap_or(-1)
}

// ---------- 等级、装备：比前后两份局内数据 ----------

/// 一个人上一份数据里的样子。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Seen {
    pub level: i64,
    pub dead: bool,
    /// 身上装备的 id（原样，海斗的 `22xxxx` 这类也照原样）。
    pub items: Vec<i64>,
}

/// 上一份局内数据里每个人的样子（按 [`LivePlayer::key`] 认人）和游戏时间。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Roster {
    pub time: f64,
    pub players: HashMap<String, Seen>,
}

impl Roster {
    pub fn of(game: &LiveGame) -> Self {
        Self {
            time: game.time,
            players: game
                .players
                .iter()
                .map(|p| {
                    (
                        p.key(),
                        Seen {
                            level: p.level,
                            dead: p.dead,
                            items: p.items.iter().map(|i| i.id).collect(),
                        },
                    )
                })
                .collect(),
        }
    }
}

/// 升了级：上一份是几级。
#[derive(Debug, Clone)]
pub struct LevelUp<'a> {
    pub player: &'a LivePlayer,
    pub from: i64,
}

/// 身上多了一件（上一份没有这个 id）。
#[derive(Debug, Clone)]
pub struct Bought<'a> {
    pub player: &'a LivePlayer,
    pub item: &'a LiveItem,
}

/// 两份数据之间变了什么。上一份里没有的人（刚连上、换了人）不算。
#[derive(Debug, Clone, Default)]
pub struct Changes<'a> {
    pub levels: Vec<LevelUp<'a>>,
    pub items: Vec<Bought<'a>>,
}

pub fn changes<'a>(before: &Roster, game: &'a LiveGame) -> Changes<'a> {
    let mut out = Changes::default();
    for player in &game.players {
        let Some(seen) = before.players.get(&player.key()) else {
            continue;
        };
        if player.level > seen.level {
            out.levels.push(LevelUp {
                player,
                from: seen.level,
            });
        }
        let mut had: HashSet<i64> = seen.items.iter().copied().collect();
        for item in &player.items {
            if item.id > 0 && had.insert(item.id) {
                out.items.push(Bought { player, item });
            }
        }
    }
    out
}

/// 对面到了这几级才报等级事件（大招升级的那几级）。
pub const LEVEL_MARKS: [i64; 3] = [6, 11, 16];

/// 海斗、大乱斗、斗魂竞技场的装备 id 是峡谷装备前面加两位（`223157` 是海斗的中娅沙漏）：取后四位。
pub fn base_item(id: i64) -> i64 {
    if id >= 100_000 {
        id % 10_000
    } else {
        id
    }
}

/// 没读到客户端的装备资料时，装备事件只认这几件（和推送“对面的关键装备”同一张表：复活甲、中娅、水银、女妖）。
pub const KEY_ITEM_IDS: [i64; 6] = [3026, 3157, 3140, 3139, 6035, 3102];

/// 这件算不算成装（装备事件只报成装）。
pub fn completed(id: i64, data: Option<&GameData>) -> bool {
    match data.filter(|d| !d.completed_items.is_empty()) {
        Some(d) => d.completed_items.contains(&id) || d.completed_items.contains(&base_item(id)),
        None => KEY_ITEM_IDS.contains(&base_item(id)),
    }
}

fn side_of(game: &LiveGame, player: &LivePlayer) -> &'static str {
    if player.team == game.my_team() {
        "ally"
    } else {
        "enemy"
    }
}

/// 两份数据之间的等级、装备 → 游戏事件（类型，数据，游戏时间）：只报对面和自己（见模块说明）。`reported` 是这局报过的
/// 装备（`<人>:<装备 id>`）：同一个人同一件只报一次——佛耶戈附身时身上是被附身那个人的装备，附身完又变回来，不算又出了一件。
pub fn roster_events(
    changes: &Changes,
    game: &LiveGame,
    data: Option<&GameData>,
    reported: &mut HashSet<String>,
) -> Vec<(&'static str, Map<String, Value>, f64)> {
    let me = game.me().map(LivePlayer::key);
    let mine = |p: &LivePlayer| me.as_deref() == Some(p.key().as_str());
    let enemy = |p: &LivePlayer| !game.my_team().is_empty() && p.team != game.my_team();
    let champion = |p: &LivePlayer| crate::client::text::champion_names(p, data).1;
    let mut out = Vec::new();
    for up in &changes.levels {
        let p = up.player;
        let reached = LEVEL_MARKS
            .iter()
            .any(|mark| up.from < *mark && p.level >= *mark);
        if mine(p) || (enemy(p) && reached) {
            out.push((
                "lol.level",
                object(json!({"team": side_of(game, p), "champion": champion(p),
                    "level": p.level, "me": mine(p)})),
                game.time,
            ));
        }
    }
    for bought in &changes.items {
        let p = bought.player;
        if (mine(p) || enemy(p))
            && completed(bought.item.id, data)
            && reported.insert(format!("{}:{}", p.key(), bought.item.id))
        {
            let name = if bought.item.name.is_empty() {
                data.and_then(|d| d.items.get(&bought.item.id))
                    .cloned()
                    .unwrap_or_default()
            } else {
                bought.item.name.clone()
            };
            out.push((
                "lol.item",
                object(json!({"team": side_of(game, p), "champion": champion(p),
                    "itemId": bought.item.id, "item": name, "me": mine(p)})),
                game.time,
            ));
        }
    }
    out
}

// ---------- 阵亡、复活：比前后两份局内数据（ADR 046 D6、D7） ----------

/// 上一份数据之前多久的 `ChampionKill` 还算这一次阵亡（同一份数据里的事件列表和 `isDead` 是一起出来的；
/// 留一点余量给两份数据的时间对不齐。复活最快也要好几秒，不会把上一次阵亡算进来）。
const KILL_SLACK: f64 = 1.0;

/// 这个人这一次阵亡的 `ChampionKill`：上一份数据以后、死的是他（[`LiveGame::find`]，隐藏了身份的人按
/// `summonerName`）的里面最晚的那件。没有时 None（接口没给，或者事件列表还没跟上）。
fn kill_of<'a>(game: &'a LiveGame, victim: &LivePlayer, after: f64) -> Option<&'a LiveEvent> {
    let key = victim.key();
    game.events
        .iter()
        .filter(|e| e.name == "ChampionKill" && e.time > after - KILL_SLACK)
        .filter(|e| game.find(&e.victim).is_some_and(|p| p.key() == key))
        .max_by(|a, b| a.time.total_cmp(&b.time))
}

/// 阵亡、复活 → 游戏事件（类型，数据，游戏时间）（ADR 046 D6、D7）。和“你阵亡时”的卡片、“对面阵亡”的提醒是同一个
/// 判断：上一份数据里活着、这一份 `isDead` 的人。上一份里没有的人（刚连上、换了人）不算；中间没读到接口那段死了又
/// 复活的不补；看不出自己在哪边（观战）时什么都不报。
///
/// - `lol.death`：一人一件，按击杀时间排（没有 `ChampionKill` 的排在最后，时间算这一份的），`alliesAlive`、
///   `enemiesAlive`、`meAlive` 是“这一个死了之后”的：同一份数据里倒下的最后一个队友只有那一件是 `alliesAlive = 1`。
///   `killer` 是击杀者的英雄，塔、小兵、野怪杀的或对不上时是空的。
/// - `lol.my_death`：自己那件 `lol.death` 后面跟一件（字段照旧，`respawnSeconds` 一样）；`lol.my_respawn`：自己上一份
///   死着、这一份活了。
///
/// `first` 是每个人这局第一次看到时的样子（按 [`LivePlayer::key`]）：英雄名都按它，佛耶戈附身时阵亡或击杀也还是佛耶戈。
pub fn death_events(
    before: &Roster,
    game: &LiveGame,
    first: &HashMap<String, LivePlayer>,
    data: Option<&GameData>,
) -> Vec<(&'static str, Map<String, Value>, f64)> {
    let Some(me) = game.me() else {
        return Vec::new();
    };
    let mine = me.team.as_str();
    if mine.is_empty() {
        return Vec::new();
    }
    let me_key = me.key();
    let was_dead = |p: &LivePlayer| before.players.get(&p.key()).map(|s| s.dead);
    let champion = |p: &LivePlayer| {
        let seen = first.get(&p.key()).unwrap_or(p);
        crate::client::text::champion_names(seen, data).1
    };
    let mut out = Vec::new();
    if !me.dead && was_dead(me) == Some(true) {
        out.push(("lol.my_respawn", Map::new(), game.time));
    }
    let mut died: Vec<(&LivePlayer, Option<&LiveEvent>)> = game
        .players
        .iter()
        .filter(|p| p.dead && was_dead(p) == Some(false))
        .map(|p| (p, kill_of(game, p, before.time)))
        .collect();
    let at = |kill: Option<&LiveEvent>| kill.map_or(game.time, |e| e.time);
    // 稳定排序：同一时刻的照局内接口的顺序
    died.sort_by(|a, b| at(a.1).total_cmp(&at(b.1)));
    let alive_now = |ours: bool| {
        game.players
            .iter()
            .filter(|p| (p.team == mine) == ours && !p.dead)
            .count()
    };
    for (i, &(p, kill)) in died.iter().enumerate() {
        // 排在后面的那几个这时候还活着
        let later = &died[i + 1..];
        let alive = |ours: bool| {
            alive_now(ours)
                + later
                    .iter()
                    .filter(|(q, _)| (q.team == mine) == ours)
                    .count()
        };
        let is_me = p.key() == me_key;
        let respawn = p.respawn.max(0.0).round() as i64;
        let killer = kill
            .and_then(|e| game.find(&e.killer))
            .map(champion)
            .unwrap_or_default();
        out.push((
            "lol.death",
            object(json!({
                "team": if p.team == mine { "ally" } else { "enemy" },
                "champion": champion(p),
                "me": is_me,
                "killer": killer,
                "respawnSeconds": respawn,
                "alliesAlive": alive(true),
                "enemiesAlive": alive(false),
                "meAlive": !me.dead || later.iter().any(|(q, _)| q.key() == me_key),
            })),
            at(kill),
        ));
        if is_me {
            out.push((
                "lol.my_death",
                object(json!({"respawnSeconds": respawn})),
                at(kill),
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::live::{parse_all_game_data, tests as l};

    #[test]
    fn client_phases_map_to_the_shared_ones() {
        assert_eq!(generic_phase("None", false), "menu");
        assert_eq!(generic_phase("Lobby", false), "menu");
        assert_eq!(generic_phase("ReadyCheck", false), "queue");
        assert_eq!(generic_phase("ChampSelect", false), "pre_match");
        assert_eq!(generic_phase("InProgress", false), "loading");
        assert_eq!(generic_phase("InProgress", true), "in_match");
        assert_eq!(generic_phase("Reconnect", true), "in_match");
        assert_eq!(generic_phase("EndOfGame", false), "post_match");
        assert_eq!(generic_phase("TerminatedInError", false), "menu");
    }

    #[test]
    fn events_are_relative_to_the_player_and_only_new_ones() {
        // 我方 ORDER（艾瑞莉娅），对面 CHAOS（亚索「敌人一」、阿狸「敌人二」）
        let mut v = l::sample_json();
        v["events"]["Events"] = json!([
            {"EventID": 0, "EventName": "GameStart", "EventTime": 0.0},
            {"EventID": 1, "EventName": "FirstBlood", "EventTime": 90.0, "Recipient": "玩家"},
            {"EventID": 2, "EventName": "DragonKill", "EventTime": 300.0, "DragonType": "Fire", "Stolen": "True", "KillerName": "敌人一"},
            {"EventID": 3, "EventName": "TurretKilled", "EventTime": 400.0, "TurretKilled": "Turret_T2_L_03_A", "KillerName": "Minion_T1L1S04N0001"},
            {"EventID": 4, "EventName": "InhibKilled", "EventTime": 800.0, "InhibKilled": "Barracks_T1_C1", "KillerName": "敌人二"},
            {"EventID": 5, "EventName": "Multikill", "EventTime": 810.0, "KillerName": "敌人二", "KillStreak": 2},
            {"EventID": 6, "EventName": "Multikill", "EventTime": 812.0, "KillerName": "敌人二", "KillStreak": 3},
            {"EventID": 7, "EventName": "Ace", "EventTime": 815.0, "Acer": "敌人二", "AcingTeam": "CHAOS"},
            {"EventID": 8, "EventName": "BaronKill", "EventTime": 900.0, "Stolen": "False", "KillerName": "玩家"},
            {"EventID": 9, "EventName": "ChampionKill", "EventTime": 901.0, "KillerName": "玩家", "VictimName": "敌人一"}
        ]);
        let game = parse_all_game_data(&v).unwrap();
        let events = new_events(&v, &game, 0);
        let kinds: Vec<&str> = events.iter().map(|e| e.1).collect();
        assert_eq!(
            kinds,
            [
                "lol.first_blood",
                "lol.dragon_kill",
                "lol.turret_killed",
                "lol.inhib_killed",
                "lol.multikill",
                "lol.ace",
                "lol.baron_kill"
            ]
        );
        assert_eq!(events[0].2["team"], "ally");
        assert_eq!(events[0].2["killer"], "刀锋舞者");
        assert_eq!(
            Value::Object(events[1].2.clone()),
            json!({"team": "enemy", "dragon": "fire", "dragonName": "火龙", "stolen": true})
        );
        assert_eq!(events[1].3, 300.0);
        assert_eq!(events[2].2["team"], "ally", "推的是 T2（对面）的塔");
        assert_eq!(events[3].2["team"], "enemy", "我们的水晶被推了");
        assert_eq!(events[4].2["count"], 3);
        assert_eq!(events[4].2["killer"], "九尾妖狐");
        assert_eq!(events[5].2["team"], "enemy");
        assert_eq!(events[6].2["stolen"], false);
        // 玩家名不进数据
        assert!(!format!("{events:?}").contains("敌人"));

        assert_eq!(last_event_id(&v), 9);
        assert!(new_events(&v, &game, 9).is_empty());
        assert_eq!(new_events(&v, &game, 7).len(), 1);
    }

    /// 前后两份局内数据：按人认（不按英雄、不按顺序）；佛耶戈附身换了英雄还是同一个人；隐藏了身份的人按队伍 + 称号认；
    /// 上一份没有的人不算；装备按 id 比，海斗的 `22xxxx` 取后四位查表。
    #[test]
    fn changes_follow_the_person_not_the_champion() {
        let before_json = l::mayhem_hidden_json();
        let before = parse_all_game_data(&before_json).unwrap();
        let roster = Roster::of(&before);
        assert_eq!(roster.players.len(), 10);
        assert!(
            roster.players.contains_key("ORDER/卡牌大师"),
            "隐藏了身份的人按队伍 + 称号认"
        );
        let mut v = before_json.clone();
        let players = v["allPlayers"].as_array_mut().unwrap();
        players.reverse();
        // 我（佛耶戈）附身成了蕾欧娜、升到 10 级；卡牌（隐藏了身份）出了中娅（海斗的 id）
        let me = players
            .iter_mut()
            .find(|p| p["riotIdGameName"] == "玩家甲")
            .unwrap();
        me["rawChampionName"] = json!("game_character_displayname_Leona");
        me["level"] = json!(10);
        let tf = players
            .iter_mut()
            .find(|p| p["summonerName"] == "卡牌大师")
            .unwrap();
        tf["items"] = json!([{"itemID": 223157, "displayName": "中娅沙漏"}]);
        v["gameData"]["gameTime"] = json!(8.0);
        let after = parse_all_game_data(&v).unwrap();
        let c = changes(&roster, &after);
        assert_eq!(c.levels.len(), 1);
        assert_eq!(c.levels[0].player.alias, "Leona");
        assert_eq!(c.levels[0].from, 9);
        assert_eq!(c.items.len(), 1);
        assert_eq!(c.items[0].item.id, 223157);
        assert_eq!(base_item(223157), 3157);
        assert_eq!(base_item(3157), 3157);
        assert!(completed(223157, None), "没有装备资料：认关键装备");
        assert!(!completed(1001, None));
        // 等级：大家都从 1 级起：我升到 10 级报（自己每升一级都报），对面 1 → 9 过了 6 级报，队友不报；队友出的装备不报
        let mut low = before_json.clone();
        for p in low["allPlayers"].as_array_mut().unwrap() {
            p["level"] = json!(1);
        }
        let roster = Roster::of(&parse_all_game_data(&low).unwrap());
        let c = changes(&roster, &after);
        let mut reported = HashSet::new();
        let events = roster_events(&c, &after, None, &mut reported);
        assert!(events.iter().all(|e| e.0 == "lol.level"), "{events:?}");
        assert_eq!(events.len(), 6, "对面 5 个 + 我：{events:?}");
        let mine: Vec<_> = events.iter().filter(|e| e.1["me"] == true).collect();
        assert_eq!(mine.len(), 1);
        assert_eq!(mine[0].1["level"], 10);
        assert_eq!(mine[0].1["team"], "ally");
        assert!(events
            .iter()
            .filter(|e| e.1["me"] == false)
            .all(|e| e.1["team"] == "enemy" && e.1["level"] == 9));
        // 对面出的装备报一次：同一个人同一件再出现（佛耶戈附身完变回来）不再报
        let mut enemy_item = after.clone();
        let viktor = enemy_item
            .players
            .iter_mut()
            .find(|p| p.alias == "Viktor")
            .unwrap();
        viktor.items = vec![LiveItem {
            id: 3157,
            name: "中娅沙漏".into(),
        }];
        let c = changes(&Roster::of(&after), &enemy_item);
        let first = roster_events(&c, &enemy_item, None, &mut reported);
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].0, "lol.item");
        assert_eq!(first[0].1["itemId"], 3157);
        assert_eq!(first[0].1["item"], "中娅沙漏");
        assert!(roster_events(&c, &enemy_item, None, &mut reported).is_empty());
        // 上一份里没有的人（刚连上）不算
        let mut fewer = before_json.clone();
        fewer["allPlayers"].as_array_mut().unwrap().truncate(1);
        let c = changes(&Roster::of(&parse_all_game_data(&fewer).unwrap()), &after);
        assert!(c.items.is_empty(), "卡牌不在上一份里");
    }

    /// 阵亡测试用的一份：照录的那把海斗（[`l::mayhem_hidden_json`]：我是佛耶戈，卡牌、剑圣隐藏了身份），
    /// `dead` 里的人（英文 id）死着，`kills` 是局内接口的事件列表。
    fn mayhem_at(time: f64, dead: &[(&str, f64)], kills: Value) -> Value {
        let mut v = l::mayhem_hidden_json();
        v["gameData"]["gameTime"] = json!(time);
        v["events"]["Events"] = kills;
        for p in v["allPlayers"].as_array_mut().unwrap() {
            let raw = p["rawChampionName"].as_str().unwrap().to_string();
            if let Some((_, respawn)) = dead.iter().find(|(a, _)| raw.ends_with(a)) {
                p["isDead"] = json!(true);
                p["respawnTimer"] = json!(respawn);
            }
        }
        v
    }

    fn first_seen(game: &LiveGame) -> HashMap<String, LivePlayer> {
        game.players.iter().map(|p| (p.key(), p.clone())).collect()
    }

    fn kill(id: i64, time: f64, killer: &str, victim: &str) -> Value {
        json!({"EventID": id, "EventName": "ChampionKill", "EventTime": time,
            "KillerName": killer, "VictimName": victim, "Assisters": []})
    }

    /// ADR 046 D6：一份数据里我方倒了四个（我还活着）、对面倒了一个：按击杀时间排，一人一件；人数是“这一个死了之后”的，
    /// 逐个递减，只有最后倒下的那个队友 `alliesAlive = 1`。击杀者是隐藏了身份的人也对得上（`summonerName`）；塔、小兵杀的
    /// `killer` 是空的；上一次阵亡的 `ChampionKill` 不算进来；数据里没有玩家名。
    #[test]
    fn deaths_in_one_poll_are_ordered_and_count_down() {
        let before = parse_all_game_data(&mayhem_at(300.0, &[], json!([]))).unwrap();
        let roster = Roster::of(&before);
        let kills = json!([
            kill(5, 250.0, "对手子", "玩家乙"),
            kill(10, 300.5, "对手子", "玩家丙"),
            kill(11, 301.6, "Turret_T2_C_05_A", "卡牌大师"),
            kill(12, 300.9, "对手丑", "无极剑圣"),
            kill(13, 301.9, "Minion_T2L1S04N0001", "玩家乙"),
            kill(14, 300.7, "无极剑圣", "对手寅"),
        ]);
        let dead = [
            ("Lucian", 14.6),
            ("Ashe", 12.0),
            ("TwistedFate", 13.0),
            ("MasterYi", 11.0),
            ("Taliyah", 9.4),
        ];
        let game = parse_all_game_data(&mayhem_at(302.0, &dead, kills)).unwrap();
        let out = death_events(&roster, &game, &first_seen(&before), None);
        assert!(out.iter().all(|e| e.0 == "lol.death"), "我没死：{out:?}");
        let rows: Vec<(String, String, String, i64, i64)> = out
            .iter()
            .map(|(_, d, _)| {
                (
                    d["team"].as_str().unwrap().to_string(),
                    d["champion"].as_str().unwrap().to_string(),
                    d["killer"].as_str().unwrap().to_string(),
                    d["alliesAlive"].as_i64().unwrap(),
                    d["enemiesAlive"].as_i64().unwrap(),
                )
            })
            .collect();
        let row = |team: &str, champion: &str, killer: &str, allies: i64, enemies: i64| {
            (
                team.to_string(),
                champion.to_string(),
                killer.to_string(),
                allies,
                enemies,
            )
        };
        assert_eq!(
            rows,
            [
                row("ally", "Ashe", "Locke", 4, 5),
                row("enemy", "Taliyah", "MasterYi", 4, 4),
                row("ally", "MasterYi", "Ambessa", 3, 4),
                row("ally", "TwistedFate", "", 2, 4),
                row("ally", "Lucian", "", 1, 4),
            ]
        );
        let times: Vec<f64> = out.iter().map(|e| e.2).collect();
        assert_eq!(times, [300.5, 300.7, 300.9, 301.6, 301.9]);
        assert!(out
            .iter()
            .all(|(_, d, _)| d["meAlive"] == true && d["me"] == false));
        assert_eq!(out[4].1["respawnSeconds"], 15);
        // 验收场景（队友都倒了、我还活着）只对得上一件
        let last_ally = out
            .iter()
            .filter(|(_, d, _)| {
                d["team"] == "ally" && d["meAlive"] == true && d["alliesAlive"] == 1
            })
            .count();
        assert_eq!(last_ally, 1);
        let text = format!("{out:?}");
        assert!(
            !text.contains("玩家") && !text.contains("对手") && !text.contains("卡牌大师"),
            "{text}"
        );
        // 下一份还死着：不再报
        let again = parse_all_game_data(&mayhem_at(304.0, &dead, json!([]))).unwrap();
        assert!(death_events(&Roster::of(&game), &again, &first_seen(&before), None).is_empty());
    }

    /// 我阵亡：`lol.death`（`me = true`）后面跟一件 `lol.my_death`（同一个 `respawnSeconds`）；没有 `ChampionKill` 时
    /// 击杀者空、时间算这一份的；复活报 `lol.my_respawn`。佛耶戈附身成蕾欧娜时阵亡，英雄还是佛耶戈；附身时击杀也是。
    /// 看不出自己在哪边（观战）、上一份里没有这个人时不报。
    #[test]
    fn my_death_follows_my_death_event_and_viego_stays_viego() {
        let before_json = mayhem_at(400.0, &[], json!([]));
        let before = parse_all_game_data(&before_json).unwrap();
        let first = first_seen(&before);
        // 我（佛耶戈）附身成了蕾欧娜，杀了维克托，然后被塔打死
        let mut v = mayhem_at(
            402.0,
            &[("Viego", 20.4), ("Viktor", 8.0)],
            json!([
                kill(20, 400.6, "玩家甲", "对手辰"),
                kill(21, 401.2, "Turret_T2_L_03_A", "玩家甲")
            ]),
        );
        v["allPlayers"][0]["rawChampionName"] = json!("game_character_displayname_Leona");
        v["allPlayers"][0]["championName"] = json!("Leona");
        let game = parse_all_game_data(&v).unwrap();
        let out = death_events(&Roster::of(&before), &game, &first, None);
        let kinds: Vec<&str> = out.iter().map(|e| e.0).collect();
        assert_eq!(kinds, ["lol.death", "lol.death", "lol.my_death"]);
        assert_eq!(out[0].1["champion"], "Viktor");
        assert_eq!(out[0].1["killer"], "Viego", "附身时击杀也是佛耶戈");
        assert_eq!(out[0].1["meAlive"], true, "维克托先死，那时我还活着");
        assert_eq!(
            Value::Object(out[1].1.clone()),
            json!({"team": "ally", "champion": "Viego", "me": true, "killer": "", "respawnSeconds": 20,
                "alliesAlive": 4, "enemiesAlive": 4, "meAlive": false})
        );
        assert_eq!(
            Value::Object(out[2].1.clone()),
            json!({"respawnSeconds": 20})
        );
        assert_eq!(out[1].2, 401.2);
        assert_eq!(out[2].2, 401.2);
        // 没有 ChampionKill：击杀者空，时间是这一份的
        let bare = parse_all_game_data(&mayhem_at(402.0, &[("Viego", 20.0)], json!([]))).unwrap();
        let out = death_events(&Roster::of(&before), &bare, &first, None);
        assert_eq!(out[0].1["killer"], "");
        assert_eq!(out[0].2, 402.0);
        // 复活
        let back = parse_all_game_data(&mayhem_at(424.0, &[], json!([]))).unwrap();
        let out = death_events(&Roster::of(&bare), &back, &first, None);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].0, "lol.my_respawn");
        assert!(out[0].1.is_empty());
        // 观战：看不出自己是谁、在哪边
        let mut watching = mayhem_at(402.0, &[("Viego", 20.0), ("Viktor", 8.0)], json!([]));
        watching["activePlayer"] = json!({});
        let watching = parse_all_game_data(&watching).unwrap();
        assert!(death_events(&Roster::of(&before), &watching, &first, None).is_empty());
        // 上一份里没有的人（刚连上）不算
        let mut fewer = before_json.clone();
        fewer["allPlayers"].as_array_mut().unwrap().truncate(1);
        let out = death_events(
            &Roster::of(&parse_all_game_data(&fewer).unwrap()),
            &game,
            &first,
            None,
        );
        assert!(
            out.iter()
                .all(|e| e.1.get("champion") != Some(&json!("Viktor"))),
            "{out:?}"
        );
    }
}
