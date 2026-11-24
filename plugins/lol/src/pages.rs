//! 页面的数据（ADR 038 D5 `structuredContent`）：这局、战绩、单局详情。她拿到的是 [`crate::client::text`]
//! 写的几行字；页面拿到的是这里的结构化数据，聊天卡片是它的快照。数字和她说的同一份（[`scout::summarize`]）。
//!
//! 照 LeagueAkari 的页面：这局 10 个人（段位、近况、标签、隐藏战绩、开黑、突破口）；点一个人看他最近的战绩；
//! 点一局看 10 个人的 KDA、伤害、经济、装备。

use serde_json::{json, Value};

use crate::client::history::{GameRecord, PlayerHistory};
use crate::client::scout::{self, Summary};
use crate::client::text::ScoutPlayer;
use crate::client::GameData;

pub const PAGE_MATCH: &str = "ui://kotone/lol/match";
pub const PAGE_PLAYER: &str = "ui://kotone/lol/player";
pub const PAGE_GAME: &str = "ui://kotone/lol/game";

/// 页面里每个人最多带几局近况（这局页面上一排小格子）。
const RECENT: usize = 10;

fn champion(data: Option<&GameData>, id: i64) -> Value {
    match data.and_then(|d| d.champion(id)) {
        Some(c) => json!({"id": id, "label": c.label(), "short": c.short(), "alias": c.alias}),
        None => {
            json!({"id": id, "label": format!("英雄 {id}"), "short": format!("英雄 {id}"), "alias": ""})
        }
    }
}

fn queue(data: Option<&GameData>, id: i64, mode: &str) -> String {
    data.and_then(|d| d.queue_name(id))
        .map(str::to_string)
        .unwrap_or_else(|| crate::client::text::mode_name(mode).to_string())
}

pub fn summary_json(s: &Summary) -> Value {
    json!({
        "games": s.games,
        "wins": s.wins,
        "losses": s.games - s.wins,
        "streak": s.streak,
        "kda": (s.kda * 10.0).round() / 10.0,
        "deaths": (s.deaths * 10.0).round() / 10.0,
        "modeGames": s.mode_games,
        "modeWins": s.mode_wins,
        "modeDpm": s.mode_dpm.round(),
        "champGames": s.champ_games,
        "champWins": s.champ_wins,
    })
}

/// 战绩里的一局（这个人自己的数据）。
pub fn game_json(g: &GameRecord, data: Option<&GameData>) -> Value {
    json!({
        "gameId": g.game_id,
        "queueId": g.queue_id,
        "queue": queue(data, g.queue_id, &g.mode),
        "createdMs": g.created_ms,
        "durationS": g.duration_s,
        "champion": champion(data, g.champion_id),
        "win": g.win,
        "kills": g.kills,
        "deaths": g.deaths,
        "assists": g.assists,
        "damage": g.damage,
        "dpm": (g.damage as f64 / g.minutes()).round(),
        "gold": g.gold,
        "cs": g.cs,
        "augments": g.augments.iter().map(|a| json!({
            "id": a,
            "name": data.and_then(|d| d.augment_name(*a)).unwrap_or(""),
        })).collect::<Vec<_>>(),
    })
}

/// 这局（`lol.scout`）：两边各 5 个人、开黑、突破口。`puuids` 和 `players` 一一对应。
pub fn match_page(
    players: &[ScoutPlayer],
    puuids: &[String],
    game_id: i64,
    queue_id: i64,
    mode: &str,
    random_pick: bool,
    data: Option<&GameData>,
) -> Value {
    let summaries: Vec<Option<Summary>> = players
        .iter()
        .map(|p| {
            p.history
                .as_ref()
                .map(|h| scout::summarize(&h.games, p.champion_id, queue_id))
        })
        .collect();
    let enemies: Vec<(usize, &Summary)> = players
        .iter()
        .enumerate()
        .filter(|(_, p)| !p.ally)
        .filter_map(|(i, _)| summaries[i].as_ref().map(|s| (i, s)))
        .collect();
    let spots = scout::weak_spots(&enemies, random_pick);
    let person = |i: usize| {
        let p = &players[i];
        let h = p.history.as_ref();
        let s = summaries[i].as_ref().filter(|s| s.games > 0);
        let spot = spots.iter().find(|w| w.index == i);
        json!({
            "puuid": puuids.get(i).cloned().unwrap_or_default(),
            "champion": champion(data, p.champion_id),
            "name": p.name,
            "riotId": h.map(|h| h.riot_id.clone()).unwrap_or_default(),
            "me": p.me,
            "ally": p.ally,
            "party": p.party,
            "pending": h.is_none(),
            "hidden": h.is_some_and(PlayerHistory::hidden),
            "rank": h.and_then(|h| h.rank.clone()),
            "error": h.and_then(|h| h.error.clone()),
            "summary": s.map(summary_json),
            "tags": s.map(|s| scout::tags(s, random_pick)).unwrap_or_default(),
            "recent": h.map(|h| h.games.iter().take(RECENT).map(|g| json!({
                "gameId": g.game_id,
                "win": g.win,
                "championId": g.champion_id,
                "kills": g.kills,
                "deaths": g.deaths,
                "assists": g.assists,
            })).collect::<Vec<_>>()).unwrap_or_default(),
            "weak": spot.map(|w| json!(w.reasons)),
        })
    };
    let team = |ally: bool| -> Vec<Value> {
        (0..players.len())
            .filter(|i| players[*i].ally == ally)
            .map(person)
            .collect()
    };
    json!({
        "gameId": game_id,
        "queueId": queue_id,
        "mode": mode,
        "random": random_pick,
        "allies": team(true),
        "enemies": team(false),
        "waiting": players.iter().filter(|p| p.history.is_none()).count(),
    })
}

/// 一个人的战绩（`lol.player`）。`queue_id`：和这局同一个模式的单独算（不在对局里时 0）。
pub fn player_page(
    puuid: &str,
    history: &PlayerHistory,
    queue_id: i64,
    data: Option<&GameData>,
) -> Value {
    let all = scout::summarize(&history.games, 0, queue_id);
    // 常玩的英雄：按局数，最多 5 个
    let mut champions: Vec<(i64, usize, usize)> = Vec::new();
    for g in &history.games {
        match champions.iter_mut().find(|(id, _, _)| *id == g.champion_id) {
            Some(c) => {
                c.1 += 1;
                c.2 += usize::from(g.win);
            }
            None => champions.push((g.champion_id, 1, usize::from(g.win))),
        }
    }
    champions.sort_by_key(|(_, games, _)| std::cmp::Reverse(*games));
    champions.truncate(5);
    json!({
        "puuid": puuid,
        "riotId": history.riot_id,
        "hidden": history.hidden(),
        "rank": history.rank,
        "error": history.error,
        "summary": (all.games > 0).then(|| summary_json(&all)),
        "champions": champions.iter().map(|(id, games, wins)| json!({
            "champion": champion(data, *id), "games": games, "wins": wins,
        })).collect::<Vec<_>>(),
        "games": history.games.iter().map(|g| game_json(g, data)).collect::<Vec<_>>(),
    })
}

/// `lol.player` 给她的几行字：段位、近况、常玩、最近 5 局（页面上有全部）。
pub fn player_text(history: &PlayerHistory, data: Option<&GameData>) -> String {
    let name = if history.riot_id.is_empty() {
        "这个玩家".to_string()
    } else {
        format!("「{}」", history.riot_id)
    };
    let mut parts = vec![name];
    if let Some(rank) = &history.rank {
        parts.push(format!("单双排 {rank}"));
    }
    if history.hidden() {
        parts.push("隐藏战绩".into());
    }
    if history.games.is_empty() {
        parts.push(match &history.error {
            Some(e) => format!("战绩查不到（{e}）"),
            None => "最近没有对局".into(),
        });
        return parts.join(" · ");
    }
    let s = scout::summarize(&history.games, 0, 0);
    parts.push(format!(
        "近 {} 局 {} 胜 {} 负",
        s.games,
        s.wins,
        s.games - s.wins
    ));
    parts.push(format!("KDA {:.1}，场均死 {:.1}", s.kda, s.deaths));
    if s.streak >= 3 {
        parts.push(format!("{} 连胜", s.streak));
    } else if s.streak <= -3 {
        parts.push(format!("{} 连败", -s.streak));
    }
    let mut out = vec![parts.join(" · ")];
    let short = |id: i64| {
        data.and_then(|d| d.champion(id))
            .map(|c| c.short().to_string())
            .unwrap_or_else(|| format!("英雄 {id}"))
    };
    let recent: Vec<String> = history
        .games
        .iter()
        .take(5)
        .map(|g| {
            format!(
                "{} {} {}/{}/{}（{}）",
                short(g.champion_id),
                if g.win { "胜" } else { "负" },
                g.kills,
                g.deaths,
                g.assists,
                queue(data, g.queue_id, &g.mode)
            )
        })
        .collect();
    out.push(format!("最近 5 局：{}", recent.join("；")));
    out.join("\n")
}

fn i(v: &Value, key: &str) -> i64 {
    v.get(key).and_then(Value::as_i64).unwrap_or(0)
}

fn s(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

/// 单局详情（`/lol-match-history/v1/games/{gameId}`，`lol.game`）：两边 10 个人。
pub fn game_page(detail: &Value, data: Option<&GameData>) -> Option<Value> {
    let participants = detail.get("participants")?.as_array()?;
    let identities = detail
        .get("participantIdentities")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let duration = i(detail, "gameDuration");
    let minutes = (duration as f64 / 60.0).max(1.0);
    let mut teams: Vec<(i64, bool, Vec<Value>)> = Vec::new();
    for p in participants {
        let st = p.get("stats").cloned().unwrap_or(Value::Null);
        let id = i(p, "participantId");
        let player = identities
            .iter()
            .find(|x| i(x, "participantId") == id)
            .and_then(|x| x.get("player"))
            .cloned()
            .unwrap_or(Value::Null);
        let name = s(&player, "gameName");
        let tag = s(&player, "tagLine");
        let riot_id = match (name.is_empty(), tag.is_empty()) {
            (true, _) => s(&player, "summonerName"),
            (false, true) => name.clone(),
            (false, false) => format!("{name}#{tag}"),
        };
        let items: Vec<Value> = (0..=6)
            .map(|n| i(&st, &format!("item{n}")))
            .filter(|x| *x > 0)
            .map(|x| json!({"id": x, "name": data.and_then(|d| d.items.get(&x)).cloned().unwrap_or_default()}))
            .collect();
        let augments: Vec<Value> = (1..=6)
            .map(|n| i(&st, &format!("playerAugment{n}")))
            .filter(|a| *a > 0)
            .map(|a| json!({"id": a, "name": data.and_then(|d| d.augment_name(a)).unwrap_or("")}))
            .collect();
        let damage = i(&st, "totalDamageDealtToChampions");
        let team_id = i(p, "teamId");
        let win = st.get("win").and_then(Value::as_bool).unwrap_or(false);
        let row = json!({
            "puuid": s(&player, "puuid"),
            "riotId": riot_id,
            "champion": champion(data, i(p, "championId")),
            "level": i(&st, "champLevel"),
            "kills": i(&st, "kills"),
            "deaths": i(&st, "deaths"),
            "assists": i(&st, "assists"),
            "damage": damage,
            "dpm": (damage as f64 / minutes).round(),
            "taken": i(&st, "totalDamageTaken"),
            "gold": i(&st, "goldEarned"),
            "cs": i(&st, "totalMinionsKilled") + i(&st, "neutralMinionsKilled"),
            "items": items,
            "augments": augments,
        });
        match teams.iter_mut().find(|(t, _, _)| *t == team_id) {
            Some(team) => team.2.push(row),
            None => teams.push((team_id, win, vec![row])),
        }
    }
    teams.sort_by_key(|(t, _, _)| *t);
    Some(json!({
        "gameId": i(detail, "gameId"),
        "queueId": i(detail, "queueId"),
        "queue": queue(data, i(detail, "queueId"), &s(detail, "gameMode")),
        "createdMs": i(detail, "gameCreation"),
        "durationS": duration,
        "teams": teams.into_iter().map(|(id, win, players)| json!({
            "teamId": id, "win": win, "players": players,
        })).collect::<Vec<_>>(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::history::GameRecord;

    fn game(id: i64, win: bool, champion: i64) -> GameRecord {
        GameRecord {
            game_id: id,
            queue_id: 2400,
            mode: "KIWI".into(),
            created_ms: 1_000 - id,
            duration_s: 1200,
            champion_id: champion,
            win,
            kills: 5,
            deaths: 3,
            assists: 10,
            damage: 30_000,
            taken: 20_000,
            gold: 12_000,
            cs: 40,
            augments: vec![1, 2],
        }
    }

    fn history(wins: &[bool]) -> PlayerHistory {
        PlayerHistory {
            riot_id: "某人#123".into(),
            privacy: "PUBLIC".into(),
            games: wins
                .iter()
                .enumerate()
                .map(|(i, w)| game(i as i64 + 1, *w, if i < 3 { 39 } else { 157 }))
                .collect(),
            rank: Some("黄金 2".into()),
            error: None,
        }
    }

    #[test]
    fn player_page_has_summary_champions_and_games() {
        let data = crate::client::gamedata::tests::sample();
        let h = history(&[false, false, false, true, true]);
        let v = player_page("p1", &h, 2400, Some(&data));
        assert_eq!(v["riotId"], "某人#123");
        assert_eq!(v["summary"]["games"], 5);
        assert_eq!(v["summary"]["streak"], -3);
        assert_eq!(v["summary"]["modeGames"], 5);
        assert_eq!(v["champions"][0]["champion"]["short"], "艾瑞莉娅");
        assert_eq!(v["champions"][0]["games"], 3);
        assert_eq!(v["games"].as_array().unwrap().len(), 5);
        assert_eq!(v["games"][0]["dpm"], 1500.0);
        assert_eq!(v["games"][0]["kills"], 5);
    }

    #[test]
    fn match_page_splits_teams_and_marks_weak_spots() {
        let data = crate::client::gamedata::tests::sample();
        let mut players = Vec::new();
        let mut puuids = Vec::new();
        for n in 0..10 {
            let ally = n < 5;
            // 对面第一个连败、其余赢得多
            let wins: Vec<bool> = if n == 5 {
                vec![false; 10]
            } else {
                (0..10).map(|g| g % 3 != 0).collect()
            };
            players.push(ScoutPlayer {
                champion: "刀锋舞者 艾瑞莉娅".into(),
                short: "艾瑞莉娅".into(),
                champion_id: 39,
                name: format!("p{n}"),
                ally,
                me: n == 0,
                history: (n != 9).then(|| history(&wins)),
                ..ScoutPlayer::default()
            });
            puuids.push(format!("puuid-{n}"));
        }
        let v = match_page(
            &players,
            &puuids,
            77,
            2400,
            "海克斯大乱斗",
            true,
            Some(&data),
        );
        assert_eq!(v["allies"].as_array().unwrap().len(), 5);
        assert_eq!(v["enemies"].as_array().unwrap().len(), 5);
        assert_eq!(v["waiting"], 1);
        assert_eq!(v["allies"][0]["me"], true);
        assert_eq!(v["enemies"][0]["puuid"], "puuid-5");
        assert!(v["enemies"][0]["weak"]
            .as_array()
            .is_some_and(|r| !r.is_empty()));
        assert_eq!(v["enemies"][4]["pending"], true);
        assert_eq!(v["enemies"][0]["recent"].as_array().unwrap().len(), 10);
    }

    #[test]
    fn game_page_reads_the_recorded_detail_shape() {
        let detail = json!({
            "gameId": 9, "queueId": 2400, "gameMode": "KIWI", "gameDuration": 600, "gameCreation": 5,
            "participantIdentities": [
                {"participantId": 1, "player": {"gameName": "甲", "tagLine": "1", "puuid": "a"}},
                {"participantId": 2, "player": {"gameName": "乙", "tagLine": "", "puuid": "b"}}
            ],
            "participants": [
                {"participantId": 1, "teamId": 100, "championId": 39,
                 "stats": {"win": true, "kills": 3, "deaths": 1, "assists": 2, "totalDamageDealtToChampions": 6000,
                           "item0": 3031, "item1": 0, "playerAugment1": 7, "champLevel": 12}},
                {"participantId": 2, "teamId": 200, "championId": 157, "stats": {"win": false}}
            ]
        });
        let v = game_page(&detail, None).unwrap();
        assert_eq!(v["teams"][0]["teamId"], 100);
        assert_eq!(v["teams"][0]["win"], true);
        assert_eq!(v["teams"][0]["players"][0]["riotId"], "甲#1");
        assert_eq!(v["teams"][0]["players"][0]["dpm"], 600.0);
        assert_eq!(v["teams"][0]["players"][0]["items"][0]["id"], 3031);
        assert_eq!(v["teams"][1]["players"][0]["riotId"], "乙");
        assert!(game_page(&json!({}), None).is_none());
    }
}
