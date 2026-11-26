//! 选人阶段的“这局”（调研 G1，设计文档第三节 1“选人阶段”）：从 `/lol-champ-select/v1/session` 读双方的英雄、
//! 我方的分路和召唤师技能、备选席；我方 `puuid` 不空、名字没被隐藏的人马上开始查战绩（和加载阶段同一套，
//! 进加载以后接着用，不重查）。对面只有英雄（客户端不给名字），排位选人时队友的名字也看不到（`NameHidden`）；
//! 客户端标了是电脑的（`playerType` / `botDifficulty` / `isBot`）是电脑（ADR 043 D9），也不查。
//!
//! 这里只解析，不连客户端；连接和查战绩在 [`super`]。

use serde_json::Value;

use super::{flagged_bot, real_puuid, Member};
use crate::client::text::Identity;
use crate::pages::Pick;

/// 选人 session 里能用的东西。
#[derive(Debug, Default)]
pub(crate) struct ChampSelect {
    pub game_id: i64,
    /// 双方的人；看不到名字的（对面、排位里的队友）`puuid` 是空的，不查。
    pub members: Vec<Member>,
    /// 备选席（大乱斗、海斗）。
    pub bench: Vec<i64>,
    /// 我方有人名字被隐藏（排位）。
    pub names_hidden: bool,
}

fn team_members(list: &Value, team: u8, hidden: &mut bool, local_cell: Option<i64>) -> Vec<Member> {
    list.as_array()
        .map(|list| {
            list.iter()
                .map(|m| {
                    let locked = m["championId"].as_i64().filter(|c| *c > 0);
                    let intent = m["championPickIntent"].as_i64().filter(|c| *c > 0);
                    let me = local_cell.is_some() && m["cellId"].as_i64() == local_cell;
                    let name_hidden = !me
                        && m["nameVisibilityType"]
                            .as_str()
                            .is_some_and(|v| v.eq_ignore_ascii_case("HIDDEN"));
                    let puuid = real_puuid(m);
                    let bot = flagged_bot(m);
                    if name_hidden && team == 1 && !bot {
                        *hidden = true;
                    }
                    let puuid = if name_hidden || bot {
                        String::new()
                    } else {
                        puuid
                    };
                    let identity = if bot {
                        Identity::Bot
                    } else if puuid.is_empty() {
                        Identity::NameHidden
                    } else {
                        Identity::Known
                    };
                    let position = m["assignedPosition"]
                        .as_str()
                        .unwrap_or("")
                        .trim()
                        .to_ascii_uppercase();
                    Member {
                        puuid,
                        champion_id: locked.or(intent).unwrap_or(0),
                        team,
                        party: None,
                        profile_icon_id: None,
                        spells: [
                            m["spell1Id"].as_i64().unwrap_or(0),
                            m["spell2Id"].as_i64().unwrap_or(0),
                        ],
                        position: if position == "NONE" {
                            String::new()
                        } else {
                            position
                        },
                        pick: match (locked, intent) {
                            (Some(_), _) => Pick::Locked,
                            (None, Some(_)) => Pick::Intent,
                            (None, None) => Pick::None,
                        },
                        identity,
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

/// `/lol-champ-select/v1/session` → 这局的人。`fallback_game_id`：session 里没有 `gameId` 时用 gameflow 的。
/// 我方的 `team` 是 1（和 gameflow 的 teamOne / teamTwo 不一定对得上：选人时只分“我方 / 对面”，进加载以后按 gameflow 重排）。
pub(crate) fn parse(session: &Value, fallback_game_id: i64) -> Option<ChampSelect> {
    let game_id = session["gameId"]
        .as_i64()
        .filter(|g| *g > 0)
        .unwrap_or(fallback_game_id);
    if game_id <= 0 {
        return None;
    }
    let local_cell = session["localPlayerCellId"].as_i64();
    let mut names_hidden = false;
    let mut members = team_members(&session["myTeam"], 1, &mut names_hidden, local_cell);
    let mut ignored = false;
    let mut theirs = team_members(&session["theirTeam"], 2, &mut ignored, local_cell);
    // 对面的名字客户端不给；就算给了也不查（选人时查对面是 LeagueAkari 也不做的事）
    for m in &mut theirs {
        m.puuid.clear();
        if m.identity == Identity::Known {
            m.identity = Identity::NameHidden;
        }
    }
    members.append(&mut theirs);
    let bench = session["benchChampions"]
        .as_array()
        .map(|b| {
            b.iter()
                .filter_map(|c| c["championId"].as_i64())
                .filter(|c| *c > 0)
                .collect()
        })
        .unwrap_or_default();
    Some(ChampSelect {
        game_id,
        members,
        bench,
        names_hidden,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn ranked_champ_select_hides_names_and_keeps_picks_lanes_and_spells() {
        let session = json!({
            "gameId": 77, "localPlayerCellId": 0,
            "myTeam": [
                {"cellId": 0, "puuid": "me", "championId": 81, "spell1Id": 4, "spell2Id": 7, "assignedPosition": "bottom", "nameVisibilityType": "HIDDEN"},
                {"cellId": 1, "puuid": "mate", "championId": 0, "championPickIntent": 99, "assignedPosition": "utility", "nameVisibilityType": "HIDDEN"},
                {"cellId": 2, "puuid": "", "championId": 0, "championPickIntent": 0, "assignedPosition": "jungle"}
            ],
            "theirTeam": [{"cellId": 5, "puuid": "enemy", "championId": 22}, {"cellId": 6, "championId": 0}],
            "benchChampions": []
        });
        let cs = parse(&session, 0).unwrap();
        assert_eq!(cs.game_id, 77);
        assert!(cs.names_hidden);
        assert_eq!(cs.members.len(), 5);
        // 我自己看得到；隐藏名字的队友不查；对面不查
        assert_eq!(cs.members[0].puuid, "me");
        assert_eq!(
            (
                cs.members[0].spells,
                cs.members[0].position.as_str(),
                cs.members[0].pick
            ),
            ([4, 7], "BOTTOM", Pick::Locked)
        );
        assert_eq!(
            (
                cs.members[1].puuid.as_str(),
                cs.members[1].champion_id,
                cs.members[1].pick
            ),
            ("", 99, Pick::Intent)
        );
        assert_eq!(
            (cs.members[2].champion_id, cs.members[2].pick),
            (0, Pick::None)
        );
        assert_eq!(
            (
                cs.members[3].team,
                cs.members[3].puuid.as_str(),
                cs.members[3].champion_id
            ),
            (2, "", 22)
        );
        assert_eq!(cs.members[4].pick, Pick::None);
    }

    /// ADR 043 D9：只有客户端标了的才是电脑；名字看不到的队友、对面照样在名单里（只有英雄），都不查。
    #[test]
    fn bots_are_only_what_the_client_flags() {
        let session = json!({
            "gameId": 5, "localPlayerCellId": 0,
            "myTeam": [
                {"cellId": 0, "puuid": "me", "championId": 81, "playerType": ""},
                {"cellId": 1, "puuid": "", "championId": 99, "nameVisibilityType": "HIDDEN", "isHumanoid": true},
                {"cellId": 2, "puuid": "00000000-0000-0000-0000-000000000000", "championId": 22},
                {"cellId": 3, "puuid": "", "championId": 86, "playerType": "BOT"}
            ],
            "theirTeam": [
                {"cellId": 5, "puuid": "", "championId": 1, "botDifficulty": "RSINTERMEDIATE"},
                {"cellId": 6, "puuid": "", "championId": 2, "botDifficulty": "NONE"}
            ]
        });
        let cs = parse(&session, 0).unwrap();
        let ids: Vec<(Identity, &str)> = cs
            .members
            .iter()
            .map(|m| (m.identity, m.puuid.as_str()))
            .collect();
        assert_eq!(
            ids,
            [
                (Identity::Known, "me"),
                (Identity::NameHidden, ""),
                (Identity::NameHidden, ""),
                (Identity::Bot, ""),
                (Identity::Bot, ""),
                (Identity::NameHidden, ""),
            ]
        );
        assert!(cs.names_hidden);
    }

    #[test]
    fn aram_champ_select_has_the_bench_and_visible_teammates() {
        let session = json!({
            "gameId": 0, "localPlayerCellId": 3,
            "myTeam": [{"cellId": 3, "puuid": "me", "championId": 157}, {"cellId": 4, "puuid": "mate", "championId": 103, "nameVisibilityType": "VISIBLE"}],
            "theirTeam": [],
            "benchChampions": [{"championId": 22}, {"championId": 99}]
        });
        let cs = parse(&session, 9).unwrap();
        assert_eq!(cs.game_id, 9, "session 里没有 gameId 时用 gameflow 的");
        assert!(!cs.names_hidden);
        assert_eq!(cs.members[1].puuid, "mate");
        assert_eq!(cs.bench, [22, 99]);
        assert!(
            parse(&json!({"myTeam": []}), 0).is_none(),
            "没有对局 id 不算"
        );
    }
}
