//! 一个人的战绩（LCU `/lol-match-history/v1/products/lol/{puuid}/matches`，新的在前；
//! 列表里每局只有他自己那一份数据）、隐私设置和单双排段位。

use serde_json::Value;

/// 一局（只算这个人自己的数据）。
#[derive(Debug, Clone, PartialEq)]
pub struct GameRecord {
    pub game_id: i64,
    pub queue_id: i64,
    /// `CLASSIC`、`ARAM`、`KIWI`……
    pub mode: String,
    /// 开局时间（毫秒）。
    pub created_ms: i64,
    pub duration_s: i64,
    pub champion_id: i64,
    pub win: bool,
    pub kills: i64,
    pub deaths: i64,
    pub assists: i64,
    /// 对英雄的伤害。
    pub damage: i64,
    pub taken: i64,
    pub gold: i64,
    pub cs: i64,
    /// 海克斯（海斗、斗魂竞技场）。
    pub augments: Vec<i64>,
}

impl GameRecord {
    pub fn minutes(&self) -> f64 {
        (self.duration_s as f64 / 60.0).max(1.0)
    }
}

/// 一个人：战绩、隐私、段位，以及查的时候出的错。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PlayerHistory {
    /// `名字#编号`（查不到时空）。
    pub riot_id: String,
    /// `PUBLIC` 以外的都算“隐藏战绩”（D6：照样读，特别标出来）。
    pub privacy: String,
    pub games: Vec<GameRecord>,
    /// 单双排段位，例如“黄金 2”；没定级时 None。
    pub rank: Option<String>,
    /// 战绩查不到的原因。
    pub error: Option<String>,
}

impl PlayerHistory {
    pub fn hidden(&self) -> bool {
        !self.privacy.is_empty() && !self.privacy.eq_ignore_ascii_case("PUBLIC")
    }
}

fn i(v: &Value, key: &str) -> i64 {
    v.get(key).and_then(Value::as_i64).unwrap_or(0)
}

fn s(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

/// 战绩列表 → 每局。只留匹配到的对局（照 LeagueAkari：自定义、人机、训练不算），
/// 重开的（5 分钟内结束或提前投降）也不算。新的在前。
pub fn parse_match_list(v: &Value) -> Vec<GameRecord> {
    let games = v
        .get("games")
        .and_then(|g| g.get("games"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut out: Vec<GameRecord> = games
        .iter()
        .filter(|g| {
            let kind = s(g, "gameType");
            kind.is_empty() || kind == "MATCHED_GAME"
        })
        .filter_map(|g| {
            let p = g.get("participants")?.as_array()?.first()?;
            let st = p.get("stats")?;
            let flag = |key: &str| st.get(key).and_then(Value::as_bool).unwrap_or(false);
            let duration = i(g, "gameDuration");
            if duration < 300 || flag("gameEndedInEarlySurrender") {
                return None;
            }
            Some(GameRecord {
                game_id: i(g, "gameId"),
                queue_id: i(g, "queueId"),
                mode: s(g, "gameMode"),
                created_ms: i(g, "gameCreation"),
                duration_s: duration,
                champion_id: i(p, "championId"),
                win: flag("win"),
                kills: i(st, "kills"),
                deaths: i(st, "deaths"),
                assists: i(st, "assists"),
                damage: i(st, "totalDamageDealtToChampions"),
                taken: i(st, "totalDamageTaken"),
                gold: i(st, "goldEarned"),
                cs: i(st, "totalMinionsKilled") + i(st, "neutralMinionsKilled"),
                augments: (1..=6)
                    .map(|n| i(st, &format!("playerAugment{n}")))
                    .filter(|a| *a > 0)
                    .collect(),
            })
        })
        .collect();
    out.sort_by_key(|g| std::cmp::Reverse(g.created_ms));
    out
}

/// 战绩列表里这个人自己的 `名字#编号`。
pub fn riot_id_from_match_list(v: &Value) -> Option<String> {
    let player = v
        .get("games")?
        .get("games")?
        .as_array()?
        .first()?
        .get("participantIdentities")?
        .as_array()?
        .first()?
        .get("player")?;
    let name = s(player, "gameName");
    let tag = s(player, "tagLine");
    (!name.is_empty()).then(|| {
        if tag.is_empty() {
            name
        } else {
            format!("{name}#{tag}")
        }
    })
}

/// 段位的中文名。
fn tier_name(tier: &str) -> Option<&'static str> {
    Some(match tier.to_ascii_uppercase().as_str() {
        "IRON" => "黑铁",
        "BRONZE" => "黄铜",
        "SILVER" => "白银",
        "GOLD" => "黄金",
        "PLATINUM" => "铂金",
        "EMERALD" => "翡翠",
        "DIAMOND" => "钻石",
        "MASTER" => "大师",
        "GRANDMASTER" => "宗师",
        "CHALLENGER" => "王者",
        _ => return None,
    })
}

/// `/lol-ranked/v1/ranked-stats/{puuid}` → 单双排段位（“黄金 2”；大师以上没有小段）。
pub fn parse_solo_rank(v: &Value) -> Option<String> {
    let solo = v
        .get("queueMap")
        .and_then(|m| m.get("RANKED_SOLO_5x5"))
        .cloned()
        .or_else(|| {
            v.get("queues")?
                .as_array()?
                .iter()
                .find(|q| s(q, "queueType") == "RANKED_SOLO_5x5")
                .cloned()
        })?;
    let tier = tier_name(&s(&solo, "tier"))?;
    let division = match s(&solo, "division").as_str() {
        "I" => "1",
        "II" => "2",
        "III" => "3",
        "IV" => "4",
        _ => "",
    };
    Some(
        if division.is_empty() || matches!(tier, "大师" | "宗师" | "王者") {
            tier.to_string()
        } else {
            format!("{tier} {division}")
        },
    )
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::json;

    /// 一局（列表里的样子）。`ago` 是第几新的（0 = 最新）。
    pub(crate) fn game(
        ago: i64,
        queue: i64,
        champion: i64,
        win: bool,
        kda: (i64, i64, i64),
        damage: i64,
    ) -> Value {
        json!({
            "gameId": 1000 - ago, "gameCreation": 1_795_357_452_903i64 - ago * 1_800_000,
            "gameDuration": 900, "gameMode": if queue == 2400 { "KIWI" } else { "CLASSIC" },
            "gameType": "MATCHED_GAME", "queueId": queue,
            "participantIdentities": [{"participantId": 1, "player": {"gameName": "某人", "tagLine": "123", "puuid": "p"}}],
            "participants": [{"championId": champion, "participantId": 1, "teamId": 100, "stats": {
                "win": win, "kills": kda.0, "deaths": kda.1, "assists": kda.2,
                "totalDamageDealtToChampions": damage, "totalDamageTaken": 20000, "goldEarned": 9000,
                "totalMinionsKilled": 40, "neutralMinionsKilled": 0,
                "gameEndedInEarlySurrender": false, "playerAugment1": 1205, "playerAugment2": 0
            }}]
        })
    }

    pub(crate) fn list(games: Vec<Value>) -> Value {
        json!({"games": {"games": games}})
    }

    #[test]
    fn keeps_matched_games_newest_first_and_drops_remakes() {
        let mut remake = game(1, 2400, 39, false, (0, 0, 0), 100);
        remake["gameDuration"] = json!(200);
        let mut custom = game(2, 0, 39, true, (1, 1, 1), 100);
        custom["gameType"] = json!("CUSTOM_GAME");
        let v = list(vec![
            game(3, 2400, 157, true, (5, 2, 7), 30000),
            remake,
            custom,
            game(0, 420, 39, false, (2, 6, 3), 12000),
        ]);
        let games = parse_match_list(&v);
        assert_eq!(games.len(), 2);
        assert_eq!(games[0].queue_id, 420, "最新的在前");
        assert_eq!(games[1].champion_id, 157);
        assert_eq!(games[1].augments, [1205]);
        assert_eq!(games[1].cs, 40);
        assert_eq!(riot_id_from_match_list(&v).as_deref(), Some("某人#123"));
    }

    #[test]
    fn solo_rank_in_chinese() {
        let v = json!({"queueMap": {"RANKED_SOLO_5x5": {"tier": "GOLD", "division": "II"}}});
        assert_eq!(parse_solo_rank(&v).as_deref(), Some("黄金 2"));
        let v = json!({"queues": [{"queueType": "RANKED_SOLO_5x5", "tier": "MASTER", "division": "I"}]});
        assert_eq!(parse_solo_rank(&v).as_deref(), Some("大师"));
        let v = json!({"queueMap": {"RANKED_SOLO_5x5": {"tier": "", "division": "NA"}}});
        assert_eq!(parse_solo_rank(&v), None);
    }

    #[test]
    fn privacy_other_than_public_is_hidden() {
        let mut h = PlayerHistory::default();
        assert!(!h.hidden(), "不知道时不算隐藏");
        h.privacy = "PUBLIC".into();
        assert!(!h.hidden());
        h.privacy = "PRIVATE".into();
        assert!(h.hidden());
    }
}
