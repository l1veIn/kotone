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
    /// 召唤师技能的名字（两个；随客户端语言变：国服“闪现”，英文“Flash”）。说给玩家听用，认技能用 [`Self::spell_keys`]。
    pub spells: Vec<String>,
    /// 召唤师技能不随语言变的 id，从 `rawDisplayName`（`GeneratedTip_SummonerSpell_SummonerFlash_DisplayName`）取：
    /// `SummonerFlash`。两个槽位各一个，认不出的槽位是空串。
    pub spell_keys: [String; 2],
    pub items: Vec<LiveItem>,
    pub bot: bool,
    /// 峡谷里的分路（`TOP` / `JUNGLE` / `MIDDLE` / `BOTTOM` / `UTILITY`）；大乱斗、海斗是 `NONE` 或空。
    pub position: String,
    /// `summonerName` 原样（隐藏了身份的人这里是英雄的称号）：只用来在两份数据之间认出同一个人（[`Self::key`]）。
    pub summoner: String,
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
/// 闪现的 id（`rawDisplayName` 里的那一段；海克斯闪现符文换过的槽位是 `SummonerFlashPerksHextechFlashtraptionV2` 一类，
/// 也算带着闪现）。
const FLASH_KEY: &str = "SummonerFlash";

/// `GeneratedTip_SummonerSpell_SummonerFlash_DisplayName` → `SummonerFlash`；不是这个样子 → 空串。
fn spell_key(raw: &str) -> String {
    raw.strip_prefix("GeneratedTip_SummonerSpell_")
        .and_then(|rest| rest.strip_suffix("_DisplayName"))
        .filter(|key| key.starts_with("Summoner"))
        .unwrap_or("")
        .to_string()
}

impl LivePlayer {
    /// 带没带闪现（“谁没带闪”只用这一个判断：`lol.match`、一键喊话）。按不随客户端语言变的 id 认，不看显示的名字
    /// （英文客户端是“Flash”）。有一个槽位是闪现 → 带着；两个槽位都认得出、都不是 → 没带；字段缺、认不出 → None：
    /// 不知道，就不说“没带闪”（宁可不说，也不把错的信息发给队友）。
    pub fn carries_flash(&self) -> Option<bool> {
        if self.spell_keys.iter().any(|k| k.starts_with(FLASH_KEY)) {
            Some(true)
        } else if self.spell_keys.iter().all(|k| !k.is_empty()) {
            Some(false)
        } else {
            None
        }
    }

    /// 隐藏了身份（主播模式，ADR 043 D9）：不是电脑、局内接口里没有名字（2026-11-26 实测 `riotId` 是 `#`、
    /// `riotIdGameName` 空、`summonerName` 是英雄名）。只用来写“隐藏了身份”，不去反查是谁。
    pub fn hidden_identity(&self) -> bool {
        !self.bot && self.name.trim().is_empty()
    }

    /// 在前后两份局内数据里认出同一个人用的 key（不看英雄：佛耶戈附身时 `rawChampionName` 会变；不看顺序：
    /// 2026-11-26 录的那把海斗里 `allPlayers` 的顺序变过）。有名字时是 `riotId`；隐藏了身份的人（`riotId` 是 `#`）
    /// 用队伍 + `summonerName`（录的样子是英雄的称号）。
    pub fn key(&self) -> String {
        if self.name.trim().is_empty() {
            format!("{}/{}", self.team, self.summoner)
        } else {
            self.riot_id.clone()
        }
    }
}

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
        spell_keys: ["summonerSpellOne", "summonerSpellTwo"].map(|slot| {
            spell_key(&s(
                spells.get(slot).unwrap_or(&Value::Null),
                "rawDisplayName",
            ))
        }),
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
        position: s(p, "position"),
        summoner: s(p, "summonerName"),
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

    /// 局内接口里召唤师技能的 `rawDisplayName`（照实录的样子）。
    fn raw_spell(name: &str) -> String {
        let key = match name {
            "闪现" | "Flash" => "SummonerFlash",
            "引燃" | "Ignite" => "SummonerDot",
            "标记" | "Mark" => "SummonerSnowball",
            "幽灵疾步" | "Ghost" => "SummonerHaste",
            _ => "SummonerBarrier",
        };
        format!("GeneratedTip_SummonerSpell_{key}_DisplayName")
    }

    fn p(name: &str, raw: &str, title: &str, team: &str, spells: [&str; 2], kills: i64) -> Value {
        json!({
            "championName": title, "isBot": false, "isDead": false, "items": [],
            "level": 9, "position": "NONE", "rawChampionName": format!("game_character_displayname_{raw}"),
            "respawnTimer": 0.0, "riotId": format!("{name}#1"), "riotIdGameName": name, "riotIdTagLine": "1",
            "scores": {"assists": 1, "creepScore": 30, "deaths": 2, "kills": kills, "wardScore": 0.0},
            "summonerName": format!("{name}#1"),
            "summonerSpells": {
                "summonerSpellOne": {"displayName": spells[0], "rawDisplayName": raw_spell(spells[0])},
                "summonerSpellTwo": {"displayName": spells[1], "rawDisplayName": raw_spell(spells[1])}
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

    /// 照 2026-11-26 录的那一把海斗（`target/lol-probe/20261126-mayhem-midgame` 的 `allgamedata-t0000.json`）：
    /// ORDER 五个人里卡牌大师、无极剑圣隐藏了身份（`riotId` 是 `#`、`riotIdGameName` 空、`summonerName` 是英雄名），
    /// 谁都不是电脑。名字换成了编的（录的是真人）。
    pub(crate) fn mayhem_hidden_json() -> Value {
        let named = |name: &str, raw: &str, team: &str| {
            let mut v = p(name, raw, raw, team, ["闪现", "标记"], 0);
            v["position"] = json!("NONE");
            v
        };
        let hidden = |raw: &str, title: &str| {
            let mut v = named("", raw, "ORDER");
            v["riotId"] = json!("#");
            v["riotIdGameName"] = json!("");
            v["riotIdTagLine"] = json!("");
            v["summonerName"] = json!(title);
            v
        };
        json!({
            "activePlayer": {"riotId": "玩家甲#1", "summonerName": "玩家甲#1"},
            "allPlayers": [
                named("玩家甲", "Viego", "ORDER"),
                named("玩家乙", "Lucian", "ORDER"),
                named("玩家丙", "Ashe", "ORDER"),
                hidden("TwistedFate", "卡牌大师"),
                hidden("MasterYi", "无极剑圣"),
                named("对手子", "Locke", "CHAOS"),
                named("对手丑", "Ambessa", "CHAOS"),
                named("对手寅", "Taliyah", "CHAOS"),
                named("对手卯", "Leona", "CHAOS"),
                named("对手辰", "Viktor", "CHAOS"),
            ],
            "events": {"Events": []},
            "gameData": {"gameMode": "KIWI", "gameTime": 0.02, "mapNumber": 12}
        })
    }

    #[test]
    fn hidden_identity_is_a_human_without_a_name() {
        let game = parse_all_game_data(&mayhem_hidden_json()).unwrap();
        let hidden: Vec<&str> = game
            .players
            .iter()
            .filter(|p| p.hidden_identity())
            .map(|p| p.alias.as_str())
            .collect();
        assert_eq!(hidden, ["TwistedFate", "MasterYi"]);
        assert_eq!(game.allies().count(), 5, "ORDER 五个人都在");
        assert!(game.players.iter().all(|p| !p.bot));
        // 电脑看 isBot，不看名字
        let mut v = mayhem_hidden_json();
        v["allPlayers"][3]["isBot"] = json!(true);
        let game = parse_all_game_data(&v).unwrap();
        assert!(game.players[3].bot && !game.players[3].hidden_identity());
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

    /// 带没带闪现按 `rawDisplayName` 认（ADR 041 第 3 期修）：中文、英文客户端一样；字段缺、认不出时不知道（不说没带）。
    #[test]
    fn flash_is_recognized_by_the_raw_name_not_the_language() {
        let flash_of = |spells: Value| {
            let mut v = sample_json();
            v["allPlayers"][2]["summonerSpells"] = spells;
            let game = parse_all_game_data(&v).unwrap();
            (
                game.players[2].carries_flash(),
                game.players[2].spell_keys.clone(),
            )
        };
        let slot =
            |display: &str, raw: &str| json!({"displayName": display, "rawDisplayName": raw});
        // 中文客户端（实录的样子）
        let (zh, keys) = flash_of(json!({
            "summonerSpellOne": slot("闪现", "GeneratedTip_SummonerSpell_SummonerFlash_DisplayName"),
            "summonerSpellTwo": slot("幽灵疾步", "GeneratedTip_SummonerSpell_SummonerHaste_DisplayName"),
        }));
        assert_eq!(zh, Some(true));
        assert_eq!(keys, ["SummonerFlash", "SummonerHaste"]);
        // 英文客户端：显示的名字是 Flash，照样认得
        assert_eq!(flash_of(json!({
            "summonerSpellOne": slot("Ignite", "GeneratedTip_SummonerSpell_SummonerDot_DisplayName"),
            "summonerSpellTwo": slot("Flash", "GeneratedTip_SummonerSpell_SummonerFlash_DisplayName"),
        })).0, Some(true));
        assert_eq!(flash_of(json!({
            "summonerSpellOne": slot("Ignite", "GeneratedTip_SummonerSpell_SummonerDot_DisplayName"),
            "summonerSpellTwo": slot("Ghost", "GeneratedTip_SummonerSpell_SummonerHaste_DisplayName"),
        })).0, Some(false), "两个都认得出、都不是闪现：没带");
        // 海克斯闪现符文换过的槽位也算带着
        assert_eq!(flash_of(json!({
            "summonerSpellOne": slot("海克斯闪现", "GeneratedTip_SummonerSpell_SummonerFlashPerksHextechFlashtraptionV2_DisplayName"),
            "summonerSpellTwo": slot("引燃", "GeneratedTip_SummonerSpell_SummonerDot_DisplayName"),
        })).0, Some(true));
        // 字段缺、认不出：不知道（只有显示的名字也不猜）
        assert_eq!(
            flash_of(json!({
                "summonerSpellOne": {"displayName": "Ignite"},
                "summonerSpellTwo": {"displayName": "Ghost"},
            }))
            .0,
            None
        );
        assert_eq!(flash_of(json!({
            "summonerSpellOne": slot("引燃", "GeneratedTip_SummonerSpell_SummonerDot_DisplayName"),
            "summonerSpellTwo": slot("?", "x"),
        })).0, None);
        assert_eq!(flash_of(Value::Null).0, None);
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
