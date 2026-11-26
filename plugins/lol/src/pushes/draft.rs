//! 峡谷选人（排位、匹配）的几块：卡片随着选人的进度长出来（ADR 043 B 轮，讨论记录第二节“选人（排位）”那几行）。
//! 都是固定规则（同样的输入永远得到同样的输出，ADR 043 D2），数据写明出处，不评价人：
//!
//! - **现在到哪一步**（[`stage`]）：预选 → 禁用 → 等 → 轮到你选 → 锁定了；
//! - **可以禁的**（[`ban_candidates`]，预选后、禁用结束前）：腾讯 101 里你预选的英雄这一路最难打的（你对它胜率最低的三个），
//!   再补这一路这个版本最强的（强度 T 越小越强，一样时胜率高的在前），一共最多 5 个；队友的英雄、已经禁了的不列；
//! - **你的英雄池**（[`my_pool`]、[`rank_pool`]，轮到你选时）：你自己最近的峡谷对局里玩过的英雄（局数多的在前，最多 8 个），
//!   对上对面已经锁了的英雄：在 101 这一路的“好打”里算 +1、“难打”或这一路强度榜写的克制里算 −1；按这个数、
//!   再按这一路胜率、再按你玩过的局数排，最多列 4 个；被禁、被别人选了的不列；
//! - **对线的是谁**（[`guess_opponent`]，锁定以后的对线那块）：对面已锁的英雄里，在你这一路 101 强度榜上选用率最高的那个
//!   （猜的，卡片上写明）。

use std::collections::HashMap;

use serde_json::{json, Value};

use crate::client::history::GameRecord;
use crate::stats::qq101::{Matchup, TierRow};

/// 峡谷的队列（匹配、排位、灵活、快速、冠军杯赛）：英雄池只看这些局。
pub const RIFT_QUEUES: &[i64] = &[400, 420, 430, 440, 480, 490, 700];
/// 英雄池最多看几个英雄、卡片上列几个。
pub const POOL_MAX: usize = 8;
pub const POOL_SHOWN: usize = 4;
/// 禁用建议最多几个；其中“克制你的”最多几个。
pub const BANS_MAX: usize = 5;
pub const COUNTERS_MAX: usize = 3;

/// 选人到哪一步了。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// 预选（亮英雄）。
    Planning,
    /// 禁用（自己的禁用还没做完，或者别人还在禁）。
    Banning,
    /// 禁完了、还没轮到你。
    Waiting,
    /// 轮到你选。
    Picking,
    /// 你锁定了（或者已经到了最后的准备阶段）。
    Locked,
}

impl Stage {
    pub fn id(self) -> &'static str {
        match self {
            Stage::Planning => "planning",
            Stage::Banning => "banning",
            Stage::Waiting => "waiting",
            Stage::Picking => "picking",
            Stage::Locked => "locked",
        }
    }
}

fn int(v: &Value, key: &str) -> i64 {
    v.get(key).and_then(Value::as_i64).unwrap_or(0)
}

fn flag(v: &Value, key: &str) -> bool {
    v.get(key).and_then(Value::as_bool).unwrap_or(false)
}

/// 选人接口里的所有动作（`actions` 是几组）。
fn actions(session: &Value) -> impl Iterator<Item = &Value> {
    session["actions"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_array)
        .flatten()
}

/// 这一次选人里能用的东西（只看选人接口）。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Draft {
    /// 我现在的英雄（锁了的，没锁时预选的）；还没有时 None。
    pub mine: Option<i64>,
    pub locked: bool,
    /// 我的分路（选人接口的 `assignedPosition`：`top` / `jungle` / `middle` / `bottom` / `utility`，没有时空）。
    pub position: String,
    /// 我方每个人现在的英雄（锁了的或预选的，含我；没选的不算）。
    pub allies: Vec<i64>,
    /// 对面已经锁了的英雄（排位里对面锁了才看得到）。
    pub enemies: Vec<i64>,
    /// 已经被禁的。
    pub banned: Vec<i64>,
    /// 我现在的两个召唤师技能。
    pub spells: [i64; 2],
}

pub fn draft(session: &Value) -> Draft {
    let me = session["localPlayerCellId"].as_i64();
    let champion = |m: &Value| {
        let locked = int(m, "championId");
        if locked > 0 {
            locked
        } else {
            int(m, "championPickIntent")
        }
    };
    let mut d = Draft::default();
    for m in session["myTeam"].as_array().into_iter().flatten() {
        let c = champion(m);
        if c > 0 {
            d.allies.push(c);
        }
        if me.is_some() && m["cellId"].as_i64() == me {
            d.mine = (c > 0).then_some(c);
            d.position = m["assignedPosition"].as_str().unwrap_or("").to_string();
            d.spells = [int(m, "spell1Id"), int(m, "spell2Id")];
        }
    }
    d.enemies = session["theirTeam"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|m| int(m, "championId"))
        .filter(|c| *c > 0)
        .collect();
    for key in ["myTeamBans", "theirTeamBans"] {
        d.banned.extend(
            session["bans"][key]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_i64)
                .filter(|c| *c > 0),
        );
    }
    for a in actions(session) {
        if a["type"] == "ban" && flag(a, "completed") && int(a, "championId") > 0 {
            let c = int(a, "championId");
            if !d.banned.contains(&c) {
                d.banned.push(c);
            }
        }
    }
    d.locked = me.is_some_and(|me| {
        actions(session)
            .any(|a| a["type"] == "pick" && int(a, "actorCellId") == me && flag(a, "completed"))
    });
    d
}

/// 现在到哪一步了（照选人接口的 `timer.phase` 和动作）。
pub fn stage(session: &Value) -> Stage {
    let me = session["localPlayerCellId"].as_i64().unwrap_or(-1);
    let phase = session["timer"]["phase"].as_str().unwrap_or("");
    let mine = |kind: &str| {
        actions(session)
            .filter(|a| a["type"] == kind && int(a, "actorCellId") == me)
            .collect::<Vec<_>>()
    };
    let picks = mine("pick");
    if phase == "FINALIZATION"
        || phase == "GAME_STARTING"
        || picks.iter().any(|a| flag(a, "completed"))
    {
        return Stage::Locked;
    }
    if picks.iter().any(|a| flag(a, "isInProgress")) {
        return Stage::Picking;
    }
    if phase == "PLANNING" {
        return Stage::Planning;
    }
    let bans_left = actions(session).any(|a| a["type"] == "ban" && !flag(a, "completed"));
    if bans_left {
        Stage::Banning
    } else {
        Stage::Waiting
    }
}

/// 强度榜一行的“强”：T 后面的数（T0 最强），读不懂的排最后。
fn tier_number(tier: &str) -> i64 {
    tier.trim_start_matches(['T', 't']).parse().unwrap_or(99)
}

/// 可以禁的（见文件开头）。`counters`：101 里我这个英雄这一路的“难打”（我对它们的胜率）；`rows`：这一路的强度榜；
/// `exclude`：不列的（我方的英雄、已经禁了的）。每个是 `{champion: id, reason: counter / strong, ...}`。
pub fn ban_candidates(counters: &[Matchup], rows: &[TierRow], exclude: &[i64]) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    let mut taken: Vec<i64> = exclude.to_vec();
    let mut hard: Vec<&Matchup> = counters
        .iter()
        .filter(|m| !exclude.contains(&m.champion_id))
        .collect();
    hard.sort_by(|a, b| {
        a.win_rate
            .unwrap_or(f64::MAX)
            .total_cmp(&b.win_rate.unwrap_or(f64::MAX))
            .then(a.champion_id.cmp(&b.champion_id))
    });
    for m in hard.into_iter().take(COUNTERS_MAX) {
        taken.push(m.champion_id);
        out.push(json!({"champion": m.champion_id, "reason": "counter", "winRate": m.win_rate}));
    }
    let mut strong: Vec<&TierRow> = rows
        .iter()
        .filter(|r| !taken.contains(&r.champion_id))
        .collect();
    strong.sort_by(|a, b| {
        tier_number(&a.tier)
            .cmp(&tier_number(&b.tier))
            .then(
                b.win_rate
                    .unwrap_or(0.0)
                    .total_cmp(&a.win_rate.unwrap_or(0.0)),
            )
            .then(a.champion_id.cmp(&b.champion_id))
    });
    for r in strong.into_iter().take(BANS_MAX.saturating_sub(out.len())) {
        out.push(
            json!({"champion": r.champion_id, "reason": "strong", "tier": r.tier,
                        "winRate": r.win_rate, "banRate": r.ban_rate}),
        );
    }
    out
}

/// 我的英雄池：最近的峡谷对局里玩过的英雄 `(英雄, 局数, 胜场)`，局数多的在前（一样多时胜场多的、编号小的在前），
/// 最多 [`POOL_MAX`] 个。
pub fn my_pool(games: &[GameRecord]) -> Vec<(i64, i64, i64)> {
    let mut count: HashMap<i64, (i64, i64)> = HashMap::new();
    for g in games
        .iter()
        .filter(|g| RIFT_QUEUES.contains(&g.queue_id) && g.champion_id > 0)
    {
        let e = count.entry(g.champion_id).or_default();
        e.0 += 1;
        e.1 += i64::from(g.win);
    }
    let mut out: Vec<(i64, i64, i64)> = count.into_iter().map(|(c, (n, w))| (c, n, w)).collect();
    out.sort_by(|a, b| b.1.cmp(&a.1).then(b.2.cmp(&a.2)).then(a.0.cmp(&b.0)));
    out.truncate(POOL_MAX);
    out
}

/// 英雄池里的英雄对上对面已锁的英雄（见文件开头）。`matchups`：每个英雄 101 这一路的（好打，难打）；`rows`：这一路的
/// 强度榜（`lane_known`：知道我的分路时，不在这一路榜上的英雄不列）；`unavailable`：被禁的、被别人选了的。
pub fn rank_pool(
    pool: &[(i64, i64, i64)],
    matchups: &HashMap<i64, (Vec<Matchup>, Vec<Matchup>)>,
    rows: &[TierRow],
    lane_known: bool,
    enemies: &[i64],
    unavailable: &[i64],
) -> Vec<Value> {
    let mut ranked: Vec<(i64, f64, i64, Value)> = Vec::new();
    for (champion, games, wins) in pool {
        if unavailable.contains(champion) {
            continue;
        }
        let row = rows.iter().find(|r| r.champion_id == *champion);
        if lane_known && row.is_none() {
            continue;
        }
        let (good, bad) = matchups
            .get(champion)
            .map(|(g, b)| (g.as_slice(), b.as_slice()))
            .unwrap_or((&[], &[]));
        let mut good_vs = Vec::new();
        let mut bad_vs = Vec::new();
        for e in enemies {
            if let Some(m) = good.iter().find(|m| m.champion_id == *e) {
                good_vs.push(json!({"champion": e, "winRate": m.win_rate}));
            } else if let Some(m) = bad.iter().find(|m| m.champion_id == *e) {
                bad_vs.push(json!({"champion": e, "winRate": m.win_rate}));
            } else if row.is_some_and(|r| r.counters.contains(e)) {
                bad_vs.push(json!({"champion": e, "winRate": Value::Null}));
            }
        }
        let score = good_vs.len() as i64 - bad_vs.len() as i64;
        let rate = row.and_then(|r| r.win_rate).unwrap_or(0.0);
        ranked.push((
            score,
            rate,
            *games,
            json!({
                "champion": champion, "games": games, "wins": wins,
                "good": good_vs, "bad": bad_vs,
                "tier": row.map(|r| r.tier.clone()), "winRate": row.and_then(|r| r.win_rate),
            }),
        ));
    }
    ranked.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.total_cmp(&a.1)).then(b.2.cmp(&a.2)));
    ranked
        .into_iter()
        .take(POOL_SHOWN)
        .map(|(_, _, _, v)| v)
        .collect()
}

/// 对线的多半是谁：对面已锁的英雄里，在我这一路强度榜上选用率最高的那个；一个都不在榜上时 None。
pub fn guess_opponent(enemies: &[i64], rows: &[TierRow]) -> Option<i64> {
    enemies
        .iter()
        .filter_map(|e| {
            rows.iter()
                .find(|r| r.champion_id == *e)
                .map(|r| (*e, r.pick_rate.unwrap_or(0.0)))
        })
        .max_by(|a, b| a.1.total_cmp(&b.1).then(b.0.cmp(&a.0)))
        .map(|(e, _)| e)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// 一次排位选人（和 `scripts/manual-e2e/lol_tools_real.py` 的峡谷选人一样的形状）：我是 2 号位中路。
    /// `stage`：planning / ban / wait / pick / locked。
    pub(crate) fn ranked_session(stage: &str) -> Value {
        let (phase, my_ban_done, my_pick) = match stage {
            "planning" => ("PLANNING", false, (false, false)),
            "ban" => ("BAN_PICK", false, (false, false)),
            "wait" => ("BAN_PICK", true, (false, false)),
            "pick" => ("BAN_PICK", true, (true, false)),
            _ => ("BAN_PICK", true, (false, true)),
        };
        let enemies_locked = !matches!(stage, "planning" | "ban");
        json!({
            "gameId": 9200001,
            "localPlayerCellId": 2,
            "timer": {"phase": phase},
            "myTeam": [
                {"cellId": 0, "championId": 0, "championPickIntent": 266, "assignedPosition": "top", "spell1Id": 4, "spell2Id": 12},
                {"cellId": 1, "championId": if enemies_locked { 64 } else { 0 }, "championPickIntent": 64, "assignedPosition": "jungle"},
                {"cellId": 2, "championId": if my_pick.1 { 157 } else { 0 }, "championPickIntent": 157, "assignedPosition": "middle", "spell1Id": 12, "spell2Id": 4},
                {"cellId": 3, "championId": 0, "championPickIntent": 222, "assignedPosition": "bottom"},
                {"cellId": 4, "championId": 0, "championPickIntent": 0, "assignedPosition": "utility"}
            ],
            "theirTeam": [
                {"cellId": 5, "championId": if enemies_locked { 238 } else { 0 }},
                {"cellId": 6, "championId": if enemies_locked { 86 } else { 0 }},
                {"cellId": 7, "championId": 0}
            ],
            "bans": {"myTeamBans": [], "theirTeamBans": []},
            "actions": [
                [
                    {"id": 1, "actorCellId": 2, "type": "ban", "championId": if my_ban_done { 555 } else { 0 }, "completed": my_ban_done, "isInProgress": !my_ban_done && phase == "BAN_PICK"},
                    {"id": 2, "actorCellId": 7, "type": "ban", "championId": if my_ban_done { 103 } else { 0 }, "completed": my_ban_done, "isInProgress": !my_ban_done && phase == "BAN_PICK"}
                ],
                [
                    {"id": 3, "actorCellId": 5, "type": "pick", "championId": 238, "completed": enemies_locked, "isInProgress": false},
                    {"id": 4, "actorCellId": 2, "type": "pick", "championId": 157, "completed": my_pick.1, "isInProgress": my_pick.0}
                ]
            ],
            "benchChampions": [],
            "isCustomGame": false
        })
    }

    #[test]
    fn stages_follow_the_draft() {
        assert_eq!(stage(&ranked_session("planning")), Stage::Planning);
        assert_eq!(stage(&ranked_session("ban")), Stage::Banning);
        assert_eq!(stage(&ranked_session("wait")), Stage::Waiting);
        assert_eq!(stage(&ranked_session("pick")), Stage::Picking);
        assert_eq!(stage(&ranked_session("locked")), Stage::Locked);
        let mut fin = ranked_session("pick");
        fin["timer"]["phase"] = json!("FINALIZATION");
        assert_eq!(stage(&fin), Stage::Locked);
        assert_eq!(stage(&json!({})), Stage::Waiting);
    }

    #[test]
    fn the_draft_reads_mine_allies_enemies_and_bans() {
        let d = draft(&ranked_session("planning"));
        assert_eq!(d.mine, Some(157));
        assert!(!d.locked);
        assert_eq!(d.position, "middle");
        assert_eq!(d.allies, [266, 64, 157, 222]);
        assert!(d.enemies.is_empty(), "预选时对面看不到");
        assert_eq!(d.spells, [12, 4]);
        let d = draft(&ranked_session("locked"));
        assert!(d.locked);
        assert_eq!(d.enemies, [238, 86]);
        assert_eq!(d.banned, [555, 103]);
    }

    pub(crate) fn row(id: i64, tier: &str, win: f64, pick: f64, counters: &[i64]) -> TierRow {
        TierRow {
            rank: None,
            champion_id: id,
            tier: tier.into(),
            lane: "MIDDLE".into(),
            win_rate: Some(win),
            pick_rate: Some(pick),
            ban_rate: Some(0.1),
            counters: counters.to_vec(),
            rank_change: None,
        }
    }

    pub(crate) fn matchup(id: i64, win: f64) -> Matchup {
        Matchup {
            champion_id: id,
            win_rate: Some(win),
        }
    }

    #[test]
    fn ban_candidates_counter_me_first_then_strong_this_patch() {
        let counters = [
            matchup(1, 0.4455),
            matchup(58, 0.4552),
            matchup(62, 0.4612),
            matchup(240, 0.4641),
            matchup(64, 0.40),
        ];
        let rows = [
            row(103, "T1", 0.51, 0.05, &[]),
            row(238, "T0", 0.505, 0.08, &[]),
            row(7, "T0", 0.52, 0.02, &[]),
            row(1, "T0", 0.53, 0.03, &[]),
        ];
        // 64 是队友的英雄：不禁
        let list = ban_candidates(&counters, &rows, &[157, 64]);
        let ids: Vec<i64> = list
            .iter()
            .map(|v| v["champion"].as_i64().unwrap())
            .collect();
        // 克制我的 3 个（胜率最低的在前），再补强度 T0 的（胜率高的在前），1 已经列过
        assert_eq!(ids, [1, 58, 62, 7, 238]);
        assert_eq!(list[0]["reason"], "counter");
        assert_eq!(list[3]["reason"], "strong");
        assert_eq!(list[3]["tier"], "T0");
        // 还没预选：只有这一路最强的
        let strong = ban_candidates(&[], &rows, &[]);
        assert_eq!(strong.len(), 4);
        assert!(strong.iter().all(|v| v["reason"] == "strong"));
        assert_eq!(strong[0]["champion"], 1);
    }

    pub(crate) fn game(champion: i64, queue: i64, win: bool) -> GameRecord {
        GameRecord {
            champion_id: champion,
            queue_id: queue,
            win,
            ..GameRecord::default()
        }
    }

    #[test]
    fn my_pool_is_my_recent_rift_champions_ranked_against_their_picks() {
        let games = vec![
            game(157, 420, true),
            game(157, 420, false),
            game(157, 420, true),
            game(103, 420, true),
            game(103, 440, true),
            game(7, 420, false),
            game(99, 450, true), // 大乱斗：不算
            game(99, 2400, true),
            game(4, 420, false),
        ];
        let pool = my_pool(&games);
        assert_eq!(pool[0], (157, 3, 2));
        assert_eq!(pool[1], (103, 2, 2));
        assert!(pool.iter().all(|(c, ..)| *c != 99));
        assert_eq!(pool.len(), 4);
        // 对面锁了劫（238）、盖伦（86）
        let mut matchups = HashMap::new();
        matchups.insert(157, (vec![matchup(86, 0.55)], vec![matchup(238, 0.47)]));
        matchups.insert(103, (vec![matchup(238, 0.53), matchup(86, 0.58)], vec![]));
        matchups.insert(7, (vec![], vec![]));
        let rows = [
            row(157, "T1", 0.50, 0.1, &[]),
            row(103, "T2", 0.49, 0.05, &[]),
            row(7, "T1", 0.52, 0.04, &[238]),
            // 4 不在中路的榜上
        ];
        let ranked = rank_pool(&pool, &matchups, &rows, true, &[238, 86], &[]);
        let ids: Vec<i64> = ranked
            .iter()
            .map(|v| v["champion"].as_i64().unwrap())
            .collect();
        // 阿狸 +2；亚索 +1 −1 = 0；乐芙兰 −1（榜上写劫克制它）；崔丝塔娜不在中路榜上
        assert_eq!(ids, [103, 157, 7]);
        assert_eq!(ranked[0]["good"].as_array().unwrap().len(), 2);
        assert_eq!(ranked[2]["bad"][0]["champion"], 238);
        assert!(ranked[2]["bad"][0]["winRate"].is_null());
        assert_eq!(ranked[1]["games"], 3);
        // 阿狸被禁了：不列
        let ranked = rank_pool(&pool, &matchups, &rows, true, &[238, 86], &[103]);
        assert_eq!(ranked[0]["champion"], 157);
        // 不知道分路：不按榜筛
        let ranked = rank_pool(&pool, &matchups, &rows, false, &[238, 86], &[]);
        assert_eq!(ranked.len(), 4);
    }

    #[test]
    fn the_opponent_is_the_enemy_most_picked_in_my_lane() {
        let rows = [
            row(238, "T1", 0.5, 0.08, &[]),
            row(103, "T1", 0.5, 0.12, &[]),
        ];
        assert_eq!(guess_opponent(&[86, 238, 103], &rows), Some(103));
        assert_eq!(guess_opponent(&[86], &rows), None);
        assert_eq!(guess_opponent(&[], &rows), None);
    }
}
