//! 局内接口的事件列表 → 游戏事件（ADR 036 的 `lol.*`），以及客户端阶段 → 所有游戏共用的阶段。
//!
//! - 只报新的（`EventID` 比上次大的）；`team` 是相对玩家的 `ally` / `enemy`；
//! - 塔和水晶按建筑名里的 `T1` / `T2` 判断是谁推的（小兵推的塔没有英雄）；
//! - `data` 里只放英雄名，不放玩家名（别人起的名字不进自动化的入参）。

use serde_json::{json, Map, Value};

use crate::live::LiveGame;

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

/// 这个名字（玩家名）的英雄和是哪边。
fn who(game: &LiveGame, name: &str) -> (String, &'static str) {
    let team = match game.is_ally_name(name) {
        Some(true) => "ally",
        Some(false) => "enemy",
        None => "",
    };
    let champion = game
        .players
        .iter()
        .find(|p| !name.is_empty() && (p.name == name || p.riot_id == name))
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
                    "dragonName": crate::text::dragon(s(e, "DragonType")), "stolen": stolen}),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live::{parse_all_game_data, tests as l};

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
}
