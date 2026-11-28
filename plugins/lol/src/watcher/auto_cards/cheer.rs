//! 推送卡片上那句预置的话（ADR 050 第 5 条：“预置为主，她补长的”）。插件按固定规则从预置的句子里挑一句短的放在
//! 卡片上（ADR 043 D2 的“固定规则 + 固定说法”，插件分内的事）；打得特别好或特别差时她再说几句，那是自动化里的
//! `ask()`（`lol.game_end`，内置自动化 `builtin-lol-game-end-cheer`）。
//!
//! - **句子在 messages 里**（中英文各一套，跟角色无关）：每档一条 `plugin_lol_cheer_<档>`，几句用 `|` 隔开。页面按
//!   数据里的 `cheer: {tier, index}` 用界面的语言取；给她的那句说明用中文那套（`summary_note`）。两种语言每档句数一样。
//! - **这局总结**按这局的事实分档（[`game_tier`]）：五杀 → 赢了、伤害全队第一、KDA ≥ [`CARRY_KDA`] → 赢了 →
//!   输了但伤害全队第一 → 输了。按对局 id 取模挑；同一档连着两局时不挑上一局那句（[`pick_game`]，上一局挑的记在
//!   插件状态里，不落盘）。
//! - **今天的状态**按今天的连胜 / 连败和胜负分档（[`today_tier`]），按日期和局数挑（[`pick_today`]）：没打新的一局时
//!   挑出来的一样，卡片内容不变（宿主当成“没变”）。打平、只打了一两局没有这句。
//! - 只说她的口气，不评判队友、不嘲讽、不劝玩家休息、不说教；输了只安慰（ADR 050 第 5 条）。

use std::collections::HashMap;
use std::sync::OnceLock;

use serde_json::{json, Map, Value};

/// 这局总结“C麻了”那一档：赢了、伤害全队第一，而且 KDA（(击杀 + 助攻) ÷ 死亡，没死过按 1 算）至少这么多。
/// 用 KDA 不用死亡数：大乱斗、海斗里死得本来就多。
pub(crate) const CARRY_KDA: f64 = 5.0;
/// 今天的状态：连胜 / 连败到这么多局算“手感火热 / 连败”那一档。
pub(crate) const TODAY_STREAK_AT: i64 = 3;
/// 今天的状态：至少打了这么多局，才按胜多负少 / 负多胜少分档。
pub(crate) const TODAY_GAMES_AT: usize = 3;

/// 一档句子。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Tier {
    Penta,
    Carry,
    Win,
    LostTop,
    Loss,
    TodayHot,
    TodayCold,
    TodayUp,
    TodayDown,
}

impl Tier {
    pub(crate) const ALL: [Tier; 9] = [
        Tier::Penta,
        Tier::Carry,
        Tier::Win,
        Tier::LostTop,
        Tier::Loss,
        Tier::TodayHot,
        Tier::TodayCold,
        Tier::TodayUp,
        Tier::TodayDown,
    ];

    /// 页面数据和 messages 键里的名字。
    pub(crate) fn id(self) -> &'static str {
        match self {
            Tier::Penta => "penta",
            Tier::Carry => "carry",
            Tier::Win => "win",
            Tier::LostTop => "lost_top",
            Tier::Loss => "loss",
            Tier::TodayHot => "today_hot",
            Tier::TodayCold => "today_cold",
            Tier::TodayUp => "today_up",
            Tier::TodayDown => "today_down",
        }
    }

    pub(crate) fn key(self) -> String {
        format!("plugin_lol_cheer_{}", self.id())
    }
}

/// 一份 messages（`zh-CN.json` / `en.json`）里这一档的几句。
pub(crate) fn lines_in(messages: &Map<String, Value>, tier: Tier) -> Vec<String> {
    messages
        .get(&tier.key())
        .and_then(Value::as_str)
        .unwrap_or_default()
        .split('|')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// 中文那套（给她的说明里用；挑第几句也按它的句数，英文那套句数一样，测试查）。
fn zh_lines(tier: Tier) -> &'static [String] {
    static LINES: OnceLock<HashMap<Tier, Vec<String>>> = OnceLock::new();
    LINES
        .get_or_init(|| {
            let messages: Map<String, Value> =
                serde_json::from_str(include_str!("../../../messages/zh-CN.json"))
                    .unwrap_or_default();
            Tier::ALL
                .iter()
                .map(|t| (*t, lines_in(&messages, *t)))
                .collect()
        })
        .get(&tier)
        .map(Vec::as_slice)
        .unwrap_or_default()
}

/// 挑出来的一句。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Cheer {
    pub(crate) tier: Tier,
    pub(crate) index: usize,
    /// 中文原文（给她的说明里写“卡片上写了：……”）。
    pub(crate) text: String,
}

impl Cheer {
    fn at(tier: Tier, index: usize) -> Option<Cheer> {
        let lines = zh_lines(tier);
        let text = lines.get(index)?.clone();
        Some(Cheer { tier, index, text })
    }

    /// 页面数据里的样子：页面按界面的语言取这一句。
    pub(crate) fn page(&self) -> Value {
        json!({"tier": self.tier.id(), "index": self.index})
    }
}

/// 单局详情里我这一局的事实（页面数据 `pages::game_page_for` 里我那一行；找不到我时 None）。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MyGame {
    pub(crate) win: bool,
    pub(crate) champion: String,
    pub(crate) kills: i64,
    pub(crate) deaths: i64,
    pub(crate) assists: i64,
    pub(crate) damage: i64,
    /// 对英雄的伤害在我们队排第几（1 = 全队第一；一样多的不算比我高）。
    pub(crate) damage_rank: usize,
    /// 五杀（结算详情里有 `pentaKills` 时）。
    pub(crate) penta: Option<bool>,
}

impl MyGame {
    pub(crate) fn of(page: &Value, me: &str) -> Option<MyGame> {
        if me.is_empty() {
            return None;
        }
        page["teams"].as_array()?.iter().find_map(|team| {
            let players = team["players"].as_array()?;
            let p = players.iter().find(|p| p["puuid"].as_str() == Some(me))?;
            let damage = p["damage"].as_i64().unwrap_or(0);
            let higher = players
                .iter()
                .filter(|x| x["damage"].as_i64().unwrap_or(0) > damage)
                .count();
            Some(MyGame {
                win: team["win"].as_bool().unwrap_or(false),
                champion: p["champion"]["short"].as_str().unwrap_or("").to_string(),
                kills: p["kills"].as_i64().unwrap_or(0),
                deaths: p["deaths"].as_i64().unwrap_or(0),
                assists: p["assists"].as_i64().unwrap_or(0),
                damage,
                damage_rank: higher + 1,
                penta: p["pentaKills"].as_i64().map(|n| n > 0),
            })
        })
    }

    fn kda(&self) -> f64 {
        (self.kills + self.assists) as f64 / self.deaths.max(1) as f64
    }
}

/// 这局总结那句话的档（固定规则，从上往下第一条对得上的）。
pub(crate) fn game_tier(g: &MyGame) -> Tier {
    let top = g.damage_rank == 1;
    if g.penta == Some(true) {
        Tier::Penta
    } else if g.win && top && g.kda() >= CARRY_KDA {
        Tier::Carry
    } else if g.win {
        Tier::Win
    } else if top {
        Tier::LostTop
    } else {
        Tier::Loss
    }
}

/// 这局总结挑哪一句：对局 id 对句数取模；`last` 是上一局挑的（档，第几句），同一档又挑到同一句时换下一句。
pub(crate) fn pick_game(tier: Tier, game_id: i64, last: Option<(Tier, usize)>) -> Option<Cheer> {
    let n = zh_lines(tier).len();
    if n == 0 {
        return None;
    }
    let mut index = game_id.rem_euclid(n as i64) as usize;
    if last == Some((tier, index)) {
        index = (index + 1) % n;
    }
    Cheer::at(tier, index)
}

/// 今天的状态那句话的档：`streak` 正数连胜、负数连败（只数今天的）。打平、只打了一两局（又没连上 3 局）时 None。
pub(crate) fn today_tier(games: usize, wins: usize, streak: i64) -> Option<Tier> {
    let losses = games.saturating_sub(wins);
    if streak >= TODAY_STREAK_AT {
        Some(Tier::TodayHot)
    } else if streak <= -TODAY_STREAK_AT {
        Some(Tier::TodayCold)
    } else if games < TODAY_GAMES_AT || wins == losses {
        None
    } else if wins > losses {
        Some(Tier::TodayUp)
    } else {
        Some(Tier::TodayDown)
    }
}

/// 今天的状态挑哪一句：日期里的数字（`2026-11-28` → 20261128）加今天的局数，对句数取模。
/// 同一天、同样的局数挑出来的一样（卡片内容不变）；多打一局换一句。
pub(crate) fn pick_today(tier: Tier, date: &str, games: usize) -> Option<Cheer> {
    let n = zh_lines(tier).len() as u64;
    if n == 0 {
        return None;
    }
    let day = date
        .bytes()
        .filter(u8::is_ascii_digit)
        .fold(0u64, |acc, b| {
            acc.wrapping_mul(10).wrapping_add(u64::from(b - b'0'))
        });
    Cheer::at(tier, (day.wrapping_add(games as u64) % n) as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog(text: &str) -> Map<String, Value> {
        serde_json::from_str(text).unwrap()
    }

    fn game(win: bool, kda: (i64, i64, i64), rank: usize, penta: Option<bool>) -> MyGame {
        MyGame {
            win,
            champion: "寒冰".into(),
            kills: kda.0,
            deaths: kda.1,
            assists: kda.2,
            damage: 30_000,
            damage_rank: rank,
            penta,
        }
    }

    /// 句子在 messages 里：每档 6–10 句、中英文句数一样（按句数取模，两种语言要挑到同一个意思）；中文以短为主
    /// （超过 12 个字的不到三成）；不劝休息、不嘲讽、不评判队友。
    #[test]
    fn every_tier_has_six_to_ten_short_lines_in_both_languages() {
        let zh = catalog(include_str!("../../../messages/zh-CN.json"));
        let en = catalog(include_str!("../../../messages/en.json"));
        for tier in Tier::ALL {
            let z = lines_in(&zh, tier);
            let e = lines_in(&en, tier);
            assert!((6..=10).contains(&z.len()), "{tier:?}: {}", z.len());
            assert_eq!(z.len(), e.len(), "{tier:?} 中英文句数要一样");
            assert_eq!(zh_lines(tier), z.as_slice());
            let long = z.iter().filter(|s| s.chars().count() > 12).count();
            assert!(long * 10 <= z.len() * 3, "{tier:?} 太长：{z:?}");
            for line in &z {
                for word in [
                    "休息",
                    "睡",
                    "别打了",
                    "队友",
                    "坑",
                    "菜",
                    "垃圾",
                    "应该",
                    "要学会",
                ] {
                    assert!(!line.contains(word), "{tier:?}：{line}");
                }
            }
        }
    }

    #[test]
    fn the_game_tier_follows_the_facts_from_the_top() {
        // 五杀不看输赢
        assert_eq!(
            game_tier(&game(false, (9, 9, 3), 2, Some(true))),
            Tier::Penta
        );
        // 赢了、伤害第一、KDA 5：C 麻了；KDA 不到 5 就只是赢了
        assert_eq!(
            game_tier(&game(true, (10, 4, 10), 1, Some(false))),
            Tier::Carry
        );
        assert_eq!(game_tier(&game(true, (12, 0, 3), 1, None)), Tier::Carry);
        assert_eq!(game_tier(&game(true, (8, 4, 10), 1, None)), Tier::Win);
        assert_eq!(game_tier(&game(true, (20, 1, 20), 2, None)), Tier::Win);
        // 输了：伤害第一是“尽力了”
        assert_eq!(game_tier(&game(false, (10, 8, 5), 1, None)), Tier::LostTop);
        assert_eq!(game_tier(&game(false, (2, 8, 5), 3, None)), Tier::Loss);
    }

    #[test]
    fn the_game_line_is_picked_by_game_id_and_not_repeated_back_to_back() {
        let n = zh_lines(Tier::Carry).len() as i64;
        let a = pick_game(Tier::Carry, 3 * n + 2, None).unwrap();
        assert_eq!(a.index, 2);
        assert_eq!(a.text, zh_lines(Tier::Carry)[2]);
        assert_eq!(a.page(), json!({"tier": "carry", "index": 2}));
        // 同样的数据挑出同一句
        assert_eq!(pick_game(Tier::Carry, 3 * n + 2, None), Some(a.clone()));
        // 上一局同一档同一句：换下一句；上一局别的档、或同一档别的句：不管
        let next = pick_game(Tier::Carry, 4 * n + 2, Some((Tier::Carry, 2))).unwrap();
        assert_eq!(next.index, 3);
        assert_eq!(
            pick_game(Tier::Carry, 4 * n + 2, Some((Tier::Win, 2)))
                .unwrap()
                .index,
            2
        );
        assert_eq!(
            pick_game(Tier::Carry, 4 * n + 2, Some((Tier::Carry, 5)))
                .unwrap()
                .index,
            2
        );
        // 最后一句再往下是第一句；负的 id 也取得到
        let last = (n - 1) as usize;
        assert_eq!(
            pick_game(Tier::Carry, n - 1, Some((Tier::Carry, last)))
                .unwrap()
                .index,
            0
        );
        assert!(pick_game(Tier::Carry, -7, None).is_some());
        assert!(zh_lines(Tier::Carry).iter().any(|s| s == "C麻了"));
    }

    #[test]
    fn the_today_tier_needs_a_streak_or_a_clear_lean() {
        assert_eq!(today_tier(3, 3, 3), Some(Tier::TodayHot));
        assert_eq!(today_tier(5, 1, -3), Some(Tier::TodayCold));
        assert_eq!(today_tier(5, 3, 1), Some(Tier::TodayUp));
        assert_eq!(today_tier(5, 2, -2), Some(Tier::TodayDown));
        // 打平、只打了一两局：没有这句
        assert_eq!(today_tier(4, 2, 2), None);
        assert_eq!(today_tier(2, 2, 2), None);
        assert_eq!(today_tier(1, 0, -1), None);
        assert_eq!(today_tier(0, 0, 0), None);
    }

    #[test]
    fn the_today_line_is_picked_by_date_and_game_count() {
        let n = zh_lines(Tier::TodayCold).len();
        let a = pick_today(Tier::TodayCold, "2026-11-28", 4).unwrap();
        assert_eq!(a.index, ((20_261_128 + 4) % n as u64) as usize);
        assert_eq!(
            pick_today(Tier::TodayCold, "2026-11-28", 4),
            Some(a.clone())
        );
        let b = pick_today(Tier::TodayCold, "2026-11-28", 5).unwrap();
        assert_eq!(b.index, (a.index + 1) % n, "多打一局换一句");
        assert_ne!(pick_today(Tier::TodayCold, "2026-11-29", 4), Some(a));
    }

    #[test]
    fn my_game_reads_my_row_rank_and_penta() {
        let page = json!({"teams": [
            {"win": true, "players": [
                {"puuid": "me", "champion": {"short": "寒冰"}, "kills": 12, "deaths": 3, "assists": 20, "damage": 50_000, "pentaKills": 1},
                {"puuid": "a", "damage": 50_000},
                {"puuid": "b", "damage": 60_000},
            ]},
            {"win": false, "players": [{"puuid": "c", "damage": 90_000}]},
        ]});
        let g = MyGame::of(&page, "me").unwrap();
        assert_eq!(
            (g.win, g.champion.as_str(), g.kills, g.deaths, g.assists),
            (true, "寒冰", 12, 3, 20)
        );
        assert_eq!(g.damage_rank, 2, "一样多的不算比我高，对面的不算");
        assert_eq!(g.penta, Some(true));
        let mut old = page.clone();
        old["teams"][0]["players"][0]
            .as_object_mut()
            .unwrap()
            .remove("pentaKills");
        assert_eq!(MyGame::of(&old, "me").unwrap().penta, None);
        assert!(MyGame::of(&page, "someone").is_none());
        assert!(MyGame::of(&page, "").is_none());
    }
}
