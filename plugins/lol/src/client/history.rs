//! 一个人的战绩（LCU `/lol-match-history/v1/products/lol/{puuid}/matches`，新的在前；
//! 列表里每局只有他自己那一份数据）、隐私设置和段位（给她的是单双排一句话，页面拿结构化的两个队列）。

use serde_json::Value;

/// 一局（只算这个人自己的数据）。
#[derive(Debug, Clone, Default, PartialEq)]
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
    /// 以下只给页面（给她的字里不用）。
    /// 召唤师技能（`spell1Id`、`spell2Id`）。
    pub spells: [i64; 2],
    /// 装备，按格子（`item0`–`item6`，最后一格是饰品；空格是 0）。
    pub items: [i64; 7],
    /// 基石符文（`perk0`）、主系、副系（`perkPrimaryStyle`、`perkSubStyle`）；海斗、斗魂竞技场是 0。
    pub keystone: i64,
    pub primary_style: i64,
    pub sub_style: i64,
    /// 英雄等级（`champLevel`）。
    pub level: i64,
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
    /// 以下只给页面（给她的字里不用）。
    /// 召唤师头像（`profileIconId`；没查到时 None）。
    pub profile_icon_id: Option<i64>,
    /// 召唤师等级（没查到时 0）。
    pub summoner_level: i64,
    /// 单双排、灵活组排（[`parse_ranked`]；没查到时 None）。
    pub solo: Option<Ranked>,
    pub flex: Option<Ranked>,
}

/// 一个排位队列的段位（`/lol-ranked/v1/ranked-stats/{puuid}` 的 `queueMap` / `queues`）。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Ranked {
    /// `RANKED_SOLO_5x5` / `RANKED_FLEX_SR`。
    pub queue: String,
    /// `GOLD`……（大写）；没定级时空。
    pub tier: String,
    /// `I`–`IV`；大师以上、没定级时空。
    pub division: String,
    /// 胜点。
    pub lp: i64,
    pub wins: i64,
    pub losses: i64,
    /// 历史最高（`highestTier`，大写；没有时空）。
    pub highest_tier: String,
}

/// 单双排、灵活组排的队列名。
pub const SOLO: &str = "RANKED_SOLO_5x5";
pub const FLEX: &str = "RANKED_FLEX_SR";

impl Ranked {
    /// “黄金 2”“大师”；没定级是“未定级”。
    pub fn label(&self) -> String {
        let Some(tier) = tier_name(&self.tier) else {
            return "未定级".into();
        };
        let division = match self.division.as_str() {
            "I" => "1",
            "II" => "2",
            "III" => "3",
            "IV" => "4",
            _ => "",
        };
        if division.is_empty() {
            tier.to_string()
        } else {
            format!("{tier} {division}")
        }
    }
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
                spells: [i(p, "spell1Id"), i(p, "spell2Id")],
                items: std::array::from_fn(|n| i(st, &format!("item{n}"))),
                keystone: i(st, "perk0"),
                primary_style: i(st, "perkPrimaryStyle"),
                sub_style: i(st, "perkSubStyle"),
                level: i(st, "champLevel"),
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

/// `/lol-ranked/v1/ranked-stats/{puuid}` 里一个队列（[`SOLO`]、[`FLEX`]）的段位；这个队列没有数据时 None，
/// 有数据但没定级时 `tier` 是空的。
pub fn parse_ranked(v: &Value, queue: &str) -> Option<Ranked> {
    let q = v
        .get("queueMap")
        .and_then(|m| m.get(queue))
        .cloned()
        .or_else(|| {
            v.get("queues")?
                .as_array()?
                .iter()
                .find(|q| s(q, "queueType") == queue)
                .cloned()
        })?;
    let tier = |key: &str| {
        let t = s(&q, key).to_ascii_uppercase();
        if tier_name(&t).is_some() {
            t
        } else {
            String::new()
        }
    };
    let tier_now = tier("tier");
    let apex = matches!(tier_now.as_str(), "MASTER" | "GRANDMASTER" | "CHALLENGER");
    let division = s(&q, "division");
    Some(Ranked {
        queue: queue.to_string(),
        division: if tier_now.is_empty()
            || apex
            || !matches!(division.as_str(), "I" | "II" | "III" | "IV")
        {
            String::new()
        } else {
            division
        },
        tier: tier_now,
        lp: i(&q, "leaguePoints"),
        wins: i(&q, "wins"),
        losses: i(&q, "losses"),
        highest_tier: tier("highestTier"),
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
    fn ranked_queues_for_the_page() {
        let v = json!({"queueMap": {
            "RANKED_SOLO_5x5": {"tier": "GOLD", "division": "II", "leaguePoints": 45, "wins": 10,
                                "losses": 8, "highestTier": "platinum"},
            "RANKED_FLEX_SR": {"tier": "MASTER", "division": "I", "leaguePoints": 120},
        }});
        let solo = parse_ranked(&v, SOLO).unwrap();
        assert_eq!(
            solo,
            Ranked {
                queue: SOLO.into(),
                tier: "GOLD".into(),
                division: "II".into(),
                lp: 45,
                wins: 10,
                losses: 8,
                highest_tier: "PLATINUM".into(),
            }
        );
        assert_eq!(solo.label(), "黄金 2");
        let flex = parse_ranked(&v, FLEX).unwrap();
        assert_eq!(
            (flex.division.as_str(), flex.label()),
            ("", "大师".to_string())
        );
        // 有这个队列但没定级；列表的写法
        let v =
            json!({"queues": [{"queueType": "RANKED_SOLO_5x5", "tier": "NONE", "division": "NA"}]});
        let solo = parse_ranked(&v, SOLO).unwrap();
        assert_eq!(
            (solo.tier.as_str(), solo.label()),
            ("", "未定级".to_string())
        );
        assert!(parse_ranked(&v, FLEX).is_none());
        assert!(parse_ranked(&json!({"queues": []}), SOLO).is_none());
    }

    #[test]
    fn page_fields_of_a_game() {
        let mut g = game(0, 420, 39, true, (1, 2, 3), 100);
        g["participants"][0]["spell1Id"] = json!(4);
        g["participants"][0]["spell2Id"] = json!(14);
        let st = &mut g["participants"][0]["stats"];
        st["item0"] = json!(3031);
        st["item6"] = json!(3340);
        st["perk0"] = json!(8005);
        st["perkPrimaryStyle"] = json!(8000);
        st["perkSubStyle"] = json!(8100);
        st["champLevel"] = json!(16);
        let r = &parse_match_list(&list(vec![g]))[0];
        assert_eq!(r.spells, [4, 14]);
        assert_eq!(r.items, [3031, 0, 0, 0, 0, 0, 3340]);
        assert_eq!(
            (r.keystone, r.primary_style, r.sub_style),
            (8005, 8000, 8100)
        );
        assert_eq!(r.level, 16);
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
