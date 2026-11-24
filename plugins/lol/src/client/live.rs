//! 局内接口（`/liveclientdata/allgamedata`）→ 这局现在的样子。
//! 国服实测（2026-11-23）：`championName` 是称号（“刀锋舞者”），`rawChampionName` 是
//! `game_character_displayname_Irelia`；召唤师技能是中文名；`position` 是 `NONE`；没有任何冷却字段。

use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub struct LiveItem {
    pub id: i64,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LivePlayer {
    /// `名字#编号`。
    pub riot_id: String,
    /// 名字（不带编号）。
    pub name: String,
    /// `championName`（国服是称号）。
    pub champion: String,
    /// 英文 id（从 `rawChampionName` 取），例如 `Irelia`。
    pub alias: String,
    /// `ORDER` / `CHAOS`。
    pub team: String,
    pub level: i64,
    pub kills: i64,
    pub deaths: i64,
    pub assists: i64,
    pub cs: i64,
    pub dead: bool,
    pub respawn: f64,
    /// 召唤师技能的名字（两个）。
    pub spells: Vec<String>,
    pub items: Vec<LiveItem>,
    pub bot: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LiveEvent {
    pub id: i64,
    pub name: String,
    pub time: f64,
    pub killer: String,
    pub victim: String,
    /// 龙的种类、是不是抢的等。
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LiveGame {
    /// `gameMode`：`CLASSIC`、`ARAM`、`KIWI`（海克斯大乱斗）、`PRACTICETOOL`……
    pub mode: String,
    /// 游戏时间（秒）。
    pub time: f64,
    /// 玩家自己的 `riotId`。
    pub me: String,
    pub players: Vec<LivePlayer>,
    pub events: Vec<LiveEvent>,
}

impl LiveGame {
    /// 玩家自己。
    pub fn me(&self) -> Option<&LivePlayer> {
        self.players
            .iter()
            .find(|p| !self.me.is_empty() && p.riot_id == self.me)
    }

    /// 玩家自己的队伍（`ORDER` / `CHAOS`；不知道时空）。
    pub fn my_team(&self) -> &str {
        self.me().map(|p| p.team.as_str()).unwrap_or("")
    }

    pub fn allies(&self) -> impl Iterator<Item = &LivePlayer> {
        let team = self.my_team().to_string();
        self.players.iter().filter(move |p| p.team == team)
    }

    pub fn enemies(&self) -> impl Iterator<Item = &LivePlayer> {
        let team = self.my_team().to_string();
        self.players.iter().filter(move |p| p.team != team)
    }

    /// 两边的击杀数（我方，对面）。
    pub fn score(&self) -> (i64, i64) {
        let team = self.my_team();
        self.players.iter().fold((0, 0), |(ours, theirs), p| {
            if p.team == team {
                (ours + p.kills, theirs)
            } else {
                (ours, theirs + p.kills)
            }
        })
    }

    /// 这个名字（击杀事件里的名字）属于哪边：true = 我方。
    pub fn is_ally_name(&self, name: &str) -> Option<bool> {
        let team = self.my_team();
        self.players
            .iter()
            .find(|p| p.name == name || p.riot_id == name)
            .map(|p| p.team == team)
    }
}

fn s(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

fn n(v: &Value, key: &str) -> f64 {
    v.get(key).and_then(Value::as_f64).unwrap_or(0.0)
}

/// `game_character_displayname_Irelia` → `Irelia`。
pub fn alias_from_raw(raw: &str) -> String {
    raw.rsplit('_').next().unwrap_or("").to_string()
}

fn player(p: &Value) -> LivePlayer {
    let scores = p.get("scores").cloned().unwrap_or(Value::Null);
    let spells = p.get("summonerSpells").cloned().unwrap_or(Value::Null);
    let riot_id = {
        let id = s(p, "riotId");
        if id.is_empty() {
            s(p, "summonerName")
        } else {
            id
        }
    };
    let name = {
        let name = s(p, "riotIdGameName");
        if name.is_empty() {
            riot_id.split('#').next().unwrap_or("").to_string()
        } else {
            name
        }
    };
    LivePlayer {
        name,
        riot_id,
        champion: s(p, "championName"),
        alias: alias_from_raw(&s(p, "rawChampionName")),
        team: s(p, "team"),
        level: n(p, "level") as i64,
        kills: n(&scores, "kills") as i64,
        deaths: n(&scores, "deaths") as i64,
        assists: n(&scores, "assists") as i64,
        cs: n(&scores, "creepScore") as i64,
        dead: p.get("isDead").and_then(Value::as_bool).unwrap_or(false),
        respawn: n(p, "respawnTimer"),
        spells: ["summonerSpellOne", "summonerSpellTwo"]
            .iter()
            .filter_map(|slot| spells.get(slot))
            .map(|x| s(x, "displayName"))
            .filter(|x| !x.is_empty())
            .collect(),
        items: p
            .get("items")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .map(|x| LiveItem {
                        id: n(x, "itemID") as i64,
                        name: s(x, "displayName"),
                    })
                    .collect()
            })
            .unwrap_or_default(),
        bot: p.get("isBot").and_then(Value::as_bool).unwrap_or(false),
    }
}

fn event(e: &Value) -> LiveEvent {
    let mut detail = Vec::new();
    for key in ["DragonType", "KillStreak", "AcingTeam", "Result"] {
        match e.get(key) {
            Some(Value::String(x)) if !x.is_empty() => detail.push(x.clone()),
            Some(Value::Number(x)) => detail.push(x.to_string()),
            _ => {}
        }
    }
    if e.get("Stolen").and_then(Value::as_str) == Some("True") {
        detail.push("抢的".into());
    }
    LiveEvent {
        id: n(e, "EventID") as i64,
        name: s(e, "EventName"),
        time: n(e, "EventTime"),
        killer: s(e, "KillerName"),
        victim: s(e, "VictimName"),
        detail: detail.join(" "),
    }
}

/// `allgamedata` → [`LiveGame`]；结构不对（加载中返回的错误对象）时 None。
pub fn parse_all_game_data(v: &Value) -> Option<LiveGame> {
    let game = v.get("gameData")?;
    let players: Vec<LivePlayer> = v
        .get("allPlayers")?
        .as_array()?
        .iter()
        .map(player)
        .collect();
    if players.is_empty() {
        return None;
    }
    let active = v.get("activePlayer").cloned().unwrap_or(Value::Null);
    let me = {
        let id = s(&active, "riotId");
        if id.is_empty() {
            s(&active, "summonerName")
        } else {
            id
        }
    };
    Some(LiveGame {
        mode: s(game, "gameMode"),
        time: n(game, "gameTime"),
        me,
        players,
        events: v
            .get("events")
            .and_then(|e| e.get("Events"))
            .and_then(Value::as_array)
            .map(|events| events.iter().map(event).collect())
            .unwrap_or_default(),
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::json;

    fn p(name: &str, raw: &str, title: &str, team: &str, spells: [&str; 2], kills: i64) -> Value {
        json!({
            "championName": title, "isBot": false, "isDead": false, "items": [],
            "level": 9, "position": "NONE", "rawChampionName": format!("game_character_displayname_{raw}"),
            "respawnTimer": 0.0, "riotId": format!("{name}#1"), "riotIdGameName": name, "riotIdTagLine": "1",
            "scores": {"assists": 1, "creepScore": 30, "deaths": 2, "kills": kills, "wardScore": 0.0},
            "summonerName": format!("{name}#1"),
            "summonerSpells": {
                "summonerSpellOne": {"displayName": spells[0], "rawDisplayName": "x"},
                "summonerSpellTwo": {"displayName": spells[1], "rawDisplayName": "y"}
            },
            "team": team
        })
    }

    /// 海斗一局：我方艾瑞莉娅，对面亚索（闪现 + 引燃）、阿狸（标记 + 引燃，没带闪）。
    pub(crate) fn sample_json() -> Value {
        let mut yasuo = p("敌人一", "Yasuo", "疾风剑豪", "CHAOS", ["闪现", "引燃"], 3);
        yasuo["items"] =
            json!([{"itemID": 3158, "displayName": "明朗之靴", "count": 1, "slot": 0}]);
        json!({
            "activePlayer": {"riotId": "玩家#1", "summonerName": "玩家#1"},
            "allPlayers": [
                p("玩家", "Irelia", "刀锋舞者", "ORDER", ["闪现", "标记"], 5),
                yasuo,
                p("敌人二", "Ahri", "九尾妖狐", "CHAOS", ["标记", "引燃"], 1),
            ],
            "events": {"Events": [
                {"EventID": 0, "EventName": "GameStart", "EventTime": 0.01},
                {"EventID": 7, "EventName": "DragonKill", "EventTime": 600.5, "DragonType": "Fire", "Stolen": "True", "KillerName": "敌人一"}
            ]},
            "gameData": {"gameMode": "KIWI", "gameTime": 754.2, "mapNumber": 12}
        })
    }

    #[test]
    fn parses_players_teams_and_spells() {
        let game = parse_all_game_data(&sample_json()).unwrap();
        assert_eq!(game.mode, "KIWI");
        assert_eq!(game.me().unwrap().alias, "Irelia");
        assert_eq!(game.my_team(), "ORDER");
        let enemies: Vec<&str> = game.enemies().map(|p| p.alias.as_str()).collect();
        assert_eq!(enemies, ["Yasuo", "Ahri"]);
        assert_eq!(game.score(), (5, 4));
        let yasuo = &game.players[1];
        assert_eq!(yasuo.spells, ["闪现", "引燃"]);
        assert_eq!(yasuo.items[0].id, 3158);
        assert_eq!(yasuo.name, "敌人一");
        assert_eq!(game.events[1].detail, "Fire 抢的");
        assert_eq!(game.is_ally_name("敌人一"), Some(false));
    }

    #[test]
    fn loading_error_objects_are_not_games() {
        assert!(parse_all_game_data(&json!({"errorCode": "RESOURCE_NOT_FOUND"})).is_none());
        assert!(parse_all_game_data(&json!({"gameData": {}, "allPlayers": []})).is_none());
    }

    #[test]
    fn alias_comes_from_the_raw_name() {
        assert_eq!(
            alias_from_raw("game_character_displayname_KogMaw"),
            "KogMaw"
        );
        assert_eq!(alias_from_raw(""), "");
    }
}
