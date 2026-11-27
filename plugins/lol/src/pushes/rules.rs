//! 选英雄卡片和海克斯三选一的两条判断规则，照 [AramHex](https://github.com/czgreat/aramhex)（MIT，Copyright (c) 2026 czgreat；
//! 读的是 `main` 分支 `17ef65cd`，2026-11-20）移植，许可证全文在 `licenses/aramhex-MIT.txt`：
//!
//! - **评级 S–D**（`lolhex/engine.py` 的 `GRADES`、`rank_grade`、`hero_brief`）：按国服胜率排名，前 10% S、30% A、
//!   60% B、85% C、其余 D。名次只按胜率排（腾讯英雄榜自带的名次掺了选用率）。
//! - **三选一（四选一一样）的判断**（`lolhex/recommend.py` 的 `evaluate`）：分数是这个英雄拿这个海克斯、平滑后的胜率
//!   比英雄平均高几个百分点（平滑和 AramHex 一样 k = 300，在 [`aramhex::score`]）；
//!   - 本英雄不到 1000 局的“样本偏少”；同屏最多的那张够 1000 局时，不到 1000 局的不当首选（`Few`），
//!     不到它十分之一的“样本太少”、建议刷新（`TooFew`）；
//!   - 首选在样本够的里面挑分最高的（都不够就在全部里挑）；
//!   - 其余的：比平均低 0.5 个点以上、或比首选低 2 个点以上的“可考虑刷新”，别的“可选”；
//!   - 这个英雄一局都没拿过、别的英雄拿过的，按 −3 分算（AramHex 的“该英雄几乎不选”）；谁都没拿过的是“无数据”（新海克斯）。
//!
//! 和 AramHex 不一样的地方（数据只有 aramhex-data 的 英雄 × 海克斯，没有腾讯的官方层级、分段、测试服数据）：
//! - 没有“按官方层级兜底”“按分段综合”“测试服数据”这几条；
//! - 海克斯也给一个 S–D：同稀有度里按**平滑后**的胜率排名（AramHex 的“金色胜率第 3/40”按原始胜率排，几十局全胜的会排第一；
//!   这里平滑以后再排，和显示的分数一致），再用上面同一张表换算。

use std::collections::HashMap;

use serde::Serialize;

use crate::stats::aramhex::{self, AugmentScore, Dataset};

/// 名次占比 → 评级（AramHex `GRADES`）。
const GRADES: [(f64, &str); 5] = [
    (0.10, "S"),
    (0.30, "A"),
    (0.60, "B"),
    (0.85, "C"),
    (1.01, "D"),
];
/// 比英雄平均低这么多个点以上：可考虑刷新（`REROLL_BELOW`）。
const REROLL_BELOW: f64 = -0.5;
/// 比首选低这么多个点以上：可考虑刷新（`REROLL_GAP`）。
const REROLL_GAP: f64 = 2.0;
/// 本英雄局数到这个数才能当首选（`RELIABLE_GAMES`）。
pub const RELIABLE_GAMES: i64 = 1000;
/// 局数不到同屏最多那张的这个比例：样本太少，提示刷新（`TOO_FEW_RATIO`）。
const TOO_FEW_RATIO: f64 = 0.1;
/// 这个英雄几乎不拿的海克斯按这个分算（AramHex `basis = "rare"`）。
const RARE_SCORE: f64 = -3.0;

/// 第 `rank` 名（从 1 数）、共 `total` 个 → S / A / B / C / D；没有名次时 None。
pub fn rank_grade(rank: i64, total: i64) -> Option<&'static str> {
    if rank <= 0 || total <= 0 {
        return None;
    }
    let q = rank as f64 / total as f64;
    GRADES.iter().find(|(lim, _)| q <= *lim).map(|(_, g)| *g)
}

/// 只按胜率排名次（高的在前，一样高的按 id，结果稳定）：id → 名次（从 1 数），和一共几个有胜率的。
pub fn win_ranks(rows: impl IntoIterator<Item = (i64, Option<f64>)>) -> (HashMap<i64, i64>, i64) {
    let mut order: Vec<(i64, f64)> = rows
        .into_iter()
        .filter_map(|(id, win)| win.filter(|w| w.is_finite()).map(|w| (id, w)))
        .collect();
    order.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    order.dedup_by_key(|(id, _)| *id);
    let total = order.len() as i64;
    let ranks = order
        .iter()
        .enumerate()
        .map(|(i, (id, _))| (*id, i as i64 + 1))
        .collect();
    (ranks, total)
}

/// 一张卡片的判断。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Verdict {
    /// 首选。
    Best,
    /// 可选。
    Ok,
    /// 可考虑刷新。
    Reroll,
    /// 无数据（谁都没拿过的新海克斯）。
    NoData,
}

/// 样本不够的两档（只看同屏的几张）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Thin {
    /// 同屏有够 1000 局的，这张不够：不当首选。
    Few,
    /// 不到同屏最多那张的十分之一：胜率不可信，建议刷新。
    TooFew,
}

/// 三选一里一张卡片的结果。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CardVerdict {
    pub augment_id: i64,
    pub verdict: Verdict,
    /// 这个英雄拿它的分（有局数时）。
    pub score: Option<AugmentScore>,
    /// 这个英雄一局都没拿过（别的英雄拿过）。
    pub rare: bool,
    /// silver / gold / prismatic。
    pub rarity: String,
    /// 这个英雄的同稀有度海克斯里排第几（平滑后的胜率）、一共几个。
    pub rarity_rank: Option<i64>,
    pub rarity_total: Option<i64>,
    /// 由上面的名次换算的 S–D。
    pub grade: Option<&'static str>,
    /// 本英雄不到 1000 局。
    pub sample_warning: bool,
    pub thin: Option<Thin>,
}

impl CardVerdict {
    /// 判断用的分（比英雄平均高几个百分点；这个英雄几乎不拿的按 −3 算）；无数据时 None。
    pub fn value(&self) -> Option<f64> {
        self.score
            .map(|s| s.delta)
            .or(self.rare.then_some(RARE_SCORE))
    }
}

/// 这几张都不好（“都可以刷新”）：首选的分也比英雄平均低 0.5 个点以上（别的照规则都是可考虑刷新）。AramHex 这时还是
/// 标一个首选（非要选就拿它），这里只多认出这种情况，好让她说“都一般，可以刷新”。都没有数据时不算。
pub fn all_weak(cards: &[CardVerdict]) -> bool {
    cards
        .iter()
        .find(|c| c.verdict == Verdict::Best)
        .and_then(CardVerdict::value)
        .is_some_and(|v| v < REROLL_BELOW)
}

/// 这个英雄同稀有度里按平滑后的胜率排名（一样高的名次相同）：海克斯 → (名次, 一共几个)。
fn rarity_ranks(d: &Dataset, hero: i64, base: f64) -> HashMap<i64, (i64, i64)> {
    let mut groups: HashMap<&str, Vec<(i64, f64)>> = HashMap::new();
    for (aid, cell) in d.cells.get(&hero).into_iter().flatten() {
        let Some(rarity) = d.augments.get(aid).map(|a| a.rarity.as_str()) else {
            continue;
        };
        if rarity.is_empty() || cell.games <= 0 {
            continue;
        }
        let smoothed = aramhex::score(base, *cell).smoothed;
        groups.entry(rarity).or_default().push((*aid, smoothed));
    }
    let mut out = HashMap::new();
    for rows in groups.values_mut() {
        rows.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        let total = rows.len() as i64;
        let mut rank = 0;
        let mut previous: Option<f64> = None;
        for (position, (aid, value)) in rows.iter().enumerate() {
            if previous != Some(*value) {
                rank = position as i64 + 1;
            }
            previous = Some(*value);
            out.insert(*aid, (rank, total));
        }
    }
    out
}

/// 有没有哪个英雄拿过这个海克斯。
fn anyone_took(d: &Dataset, augment: i64) -> bool {
    d.cells
        .values()
        .any(|row| row.get(&augment).is_some_and(|c| c.games > 0))
}

/// 三选一（或四选一）：这个英雄面对这几张卡片，每张的判断（顺序和 `augments` 一样）。
pub fn evaluate(d: &Dataset, hero: i64, augments: &[i64]) -> Vec<CardVerdict> {
    let base = d.heroes.get(&hero).map(|h| h.win_rate);
    let ranks = base.map(|b| rarity_ranks(d, hero, b)).unwrap_or_default();
    // 每张的分（AramHex 的 score）和卡片
    let mut cards: Vec<(CardVerdict, Option<f64>)> = augments
        .iter()
        .map(|aid| {
            let score = d.score(hero, *aid);
            let rare = score.is_none() && base.is_some() && anyone_took(d, *aid);
            let value = score.map(|s| s.delta).or(rare.then_some(RARE_SCORE));
            let rank = ranks.get(aid).copied();
            let card = CardVerdict {
                augment_id: *aid,
                verdict: if value.is_some() {
                    Verdict::Ok
                } else {
                    Verdict::NoData
                },
                score,
                rare,
                rarity: d
                    .augments
                    .get(aid)
                    .map(|a| a.rarity.clone())
                    .unwrap_or_default(),
                rarity_rank: rank.map(|r| r.0),
                rarity_total: rank.map(|r| r.1),
                grade: rank.and_then(|(r, t)| rank_grade(r, t)),
                sample_warning: false,
                thin: None,
            };
            (card, value)
        })
        .collect();
    // 样本：只看有本英雄局数的几张
    let most = cards
        .iter()
        .filter_map(|(c, _)| c.score.map(|s| s.games))
        .max()
        .unwrap_or(0);
    for (card, _) in cards.iter_mut() {
        let Some(games) = card.score.map(|s| s.games) else {
            continue;
        };
        card.sample_warning = games < RELIABLE_GAMES;
        if most >= RELIABLE_GAMES && (games as f64) < most as f64 * TOO_FEW_RATIO {
            card.thin = Some(Thin::TooFew);
        } else if most >= RELIABLE_GAMES && games < RELIABLE_GAMES {
            card.thin = Some(Thin::Few);
        }
    }
    // 首选：样本够的里面分最高的（都不够就在全部有分的里挑；一样高取靠左的）
    let pick = |allow_thin: bool| {
        let mut best: Option<(usize, f64)> = None;
        for (i, (card, value)) in cards.iter().enumerate() {
            let Some(v) = *value else {
                continue;
            };
            let higher = !matches!(best, Some((_, b)) if b >= v);
            if (allow_thin || card.thin.is_none()) && higher {
                best = Some((i, v));
            }
        }
        best
    };
    let Some((best, best_value)) = pick(false).or_else(|| pick(true)) else {
        return cards.into_iter().map(|(c, _)| c).collect();
    };
    for (i, (card, value)) in cards.iter_mut().enumerate() {
        let Some(value) = *value else {
            continue;
        };
        card.verdict = if i == best {
            Verdict::Best
        } else if card.thin == Some(Thin::TooFew) {
            Verdict::Reroll
        } else if card.thin.is_some() {
            Verdict::Ok
        } else if value < REROLL_BELOW || value <= best_value - REROLL_GAP {
            Verdict::Reroll
        } else {
            Verdict::Ok
        };
    }
    cards.into_iter().map(|(c, _)| c).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stats::aramhex::parse_latest;
    use serde_json::json;

    #[test]
    fn grades_follow_the_aramhex_table() {
        // 前 10% S、30% A、60% B、85% C、其余 D（边界上算前一档）
        let g = |rank| rank_grade(rank, 100);
        assert_eq!(
            [
                g(1),
                g(10),
                g(11),
                g(30),
                g(31),
                g(60),
                g(61),
                g(85),
                g(86),
                g(100)
            ],
            [
                Some("S"),
                Some("S"),
                Some("A"),
                Some("A"),
                Some("B"),
                Some("B"),
                Some("C"),
                Some("C"),
                Some("D"),
                Some("D")
            ]
        );
        assert_eq!(rank_grade(1, 1), Some("D"), "只有一个时是 100%");
        assert_eq!(rank_grade(0, 100), None);
        assert_eq!(rank_grade(3, 0), None);
        assert_eq!(rank_grade(170, 170), Some("D"));
        assert_eq!(rank_grade(17, 170), Some("S"));
        assert_eq!(rank_grade(18, 170), Some("A"));
    }

    #[test]
    fn ranks_are_by_win_rate_only() {
        // 腾讯的名次掺了选用率，这里只看胜率；没有胜率的不算进总数
        let (ranks, total) = win_ranks([
            (1, Some(0.51)),
            (2, Some(0.54)),
            (3, None),
            (4, Some(0.51)),
            (5, Some(0.49)),
        ]);
        assert_eq!(total, 4);
        assert_eq!(ranks[&2], 1);
        assert_eq!(ranks[&1], 2, "一样高的按 id");
        assert_eq!(ranks[&4], 3);
        assert_eq!(ranks[&5], 4);
        assert!(!ranks.contains_key(&3));
    }

    /// 英雄 39 平均 50%；海克斯：1、2、3、5、6 金色，4 银色，7 只有别的英雄拿过，8 谁都没拿过。
    fn dataset(cells: serde_json::Value) -> Dataset {
        let latest =
            parse_latest(&json!({"path": "patches/16.23/normal", "patch": "16.23"})).unwrap();
        Dataset::from_json(
            &latest,
            &json!({"39": {"games": 100000, "wins": 50000, "win_rate": 0.5},
                    "40": {"games": 100000, "wins": 50000, "win_rate": 0.5}}),
            &json!({
                "1": {"name": "一", "rarity": "gold"}, "2": {"name": "二", "rarity": "gold"},
                "3": {"name": "三", "rarity": "gold"}, "4": {"name": "四", "rarity": "silver"},
                "5": {"name": "五", "rarity": "gold"}, "6": {"name": "六", "rarity": "gold"},
                "7": {"name": "七", "rarity": "gold"}, "8": {"name": "八", "rarity": "prismatic"}
            }),
            &cells,
        )
    }

    fn verdicts(cards: &[CardVerdict]) -> Vec<Verdict> {
        cards.iter().map(|c| c.verdict).collect()
    }

    #[test]
    fn best_ok_and_reroll() {
        let d = dataset(json!({"39": {
            "1": {"games": 20000, "wins": 11400},   // 57%：+6.9
            "2": {"games": 20000, "wins": 10900},   // 54.5%：+4.4，比首选低 2 个点以上 → 刷新
            "3": {"games": 20000, "wins": 11200},   // 56%：+5.9，低不到 2 个点 → 可选
            "4": {"games": 20000, "wins": 9800},    // 49%：−1.0
            "5": {"games": 20000, "wins": 10100},   // 50.5%：+0.5
            "6": {"games": 20000, "wins": 10500}    // 52.5%：+2.5
        }}));
        let cards = evaluate(&d, 39, &[2, 1, 3]);
        assert_eq!(
            verdicts(&cards),
            [Verdict::Reroll, Verdict::Best, Verdict::Ok]
        );
        assert!(cards.iter().all(|c| !c.sample_warning && c.thin.is_none()));
        // 金色 5 个里：1 第一、3 第二、2 第三
        assert_eq!(
            (cards[1].rarity_rank, cards[1].rarity_total),
            (Some(1), Some(5))
        );
        assert_eq!(cards[2].rarity_rank, Some(2));
        assert_eq!(cards[0].rarity_rank, Some(3));
        assert_eq!(cards[1].grade, Some("A"), "1/5 = 20%");
        assert_eq!(cards[0].grade, Some("B"), "3/5 = 60%");
        assert_eq!(cards[1].rarity, "gold");
        // 4 比平均低 0.5 个点以上、5 比首选低 2 个点以上：都是刷新。银色只有 4 一个：1/1 照表是 D
        let cards = evaluate(&d, 39, &[4, 5, 1]);
        assert_eq!(
            verdicts(&cards),
            [Verdict::Reroll, Verdict::Reroll, Verdict::Best]
        );
        assert_eq!(cards[0].grade, Some("D"));
        // 四选一：规则一样，只是多一张（AramHex 的推荐文案也按实际卡数，“四张都不理想”）
        assert_eq!(
            verdicts(&evaluate(&d, 39, &[2, 1, 3, 5])),
            [Verdict::Reroll, Verdict::Best, Verdict::Ok, Verdict::Reroll]
        );
        // 三张都比平均低：分最高的还是首选（AramHex：非要选就拿它）
        let d = dataset(json!({"39": {
            "1": {"games": 20000, "wins": 9000}, "2": {"games": 20000, "wins": 9400}, "3": {"games": 20000, "wins": 9200}
        }}));
        let weak = evaluate(&d, 39, &[1, 2, 3]);
        assert_eq!(
            verdicts(&weak),
            [Verdict::Reroll, Verdict::Best, Verdict::Reroll]
        );
        // 首选也比平均低 0.5 个点以上：都不好（她说“都一般，可以刷新”）；上面那几组有好的，不算
        assert!(all_weak(&weak));
        assert!(!all_weak(&cards));
        assert!(!all_weak(&evaluate(&d, 999, &[1, 2])), "都没数据不算");
    }

    #[test]
    fn thin_samples_are_not_picked() {
        let d = dataset(json!({"39": {
            "1": {"games": 8000, "wins": 4240},    // 53%：+2.9
            "2": {"games": 900, "wins": 620},      // 69%（+14.2）但不到 1000 局：不当首选，可选
            "3": {"games": 150, "wins": 120}       // 不到同屏最多（8000）的十分之一：样本太少，建议刷新
        }}));
        let cards = evaluate(&d, 39, &[1, 2, 3]);
        assert_eq!(
            verdicts(&cards),
            [Verdict::Best, Verdict::Ok, Verdict::Reroll]
        );
        assert_eq!(cards[1].thin, Some(Thin::Few));
        assert_eq!(cards[2].thin, Some(Thin::TooFew));
        assert!(cards[1].sample_warning && cards[2].sample_warning && !cards[0].sample_warning);
        assert!(cards[2].score.unwrap().low_sample, "不到 300 局的变淡");
        assert!(
            cards[1].score.unwrap().delta > cards[0].score.unwrap().delta,
            "分更高也不当首选"
        );
        // 三张都不够 1000 局：照常比，分最高的当首选（界面上写“仅供参考”）
        let d = dataset(json!({"39": {
            "1": {"games": 400, "wins": 220}, "2": {"games": 600, "wins": 360}, "3": {"games": 500, "wins": 250}
        }}));
        let cards = evaluate(&d, 39, &[1, 2, 3]);
        assert_eq!(cards[1].verdict, Verdict::Best);
        assert!(cards.iter().all(|c| c.sample_warning && c.thin.is_none()));
    }

    #[test]
    fn rare_and_new_augments() {
        let d = dataset(json!({
            "39": {"1": {"games": 20000, "wins": 10300}, "2": {"games": 20000, "wins": 10100}},
            "40": {"7": {"games": 5000, "wins": 2600}}
        }));
        // 7：英雄 39 没拿过、40 拿过 → 按 −3 算，可考虑刷新；8：谁都没拿过 → 无数据
        let cards = evaluate(&d, 39, &[1, 7, 8]);
        assert_eq!(
            verdicts(&cards),
            [Verdict::Best, Verdict::Reroll, Verdict::NoData]
        );
        assert!(cards[1].rare && cards[1].score.is_none() && cards[1].grade.is_none());
        assert!(!cards[2].rare && cards[2].score.is_none());
        assert_eq!(cards[2].rarity, "prismatic");
        // 不认识的英雄：没有分、没有名次，全是无数据
        let cards = evaluate(&d, 999, &[1, 2]);
        assert_eq!(verdicts(&cards), [Verdict::NoData, Verdict::NoData]);
        // 一样高的名次相同
        let d = dataset(
            json!({"39": {"1": {"games": 2000, "wins": 1000}, "2": {"games": 2000, "wins": 1000}, "3": {"games": 2000, "wins": 900}}}),
        );
        let cards = evaluate(&d, 39, &[1, 2, 3]);
        assert_eq!(
            cards.iter().map(|c| c.rarity_rank).collect::<Vec<_>>(),
            [Some(1), Some(1), Some(3)]
        );
        assert_eq!(cards[0].verdict, Verdict::Best, "一样高取靠左的");
        assert_eq!(cards[1].verdict, Verdict::Ok);
    }
}
