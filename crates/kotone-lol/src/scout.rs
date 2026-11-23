//! 战绩统计和“突破口”（ADR 035 D3）。数字由这里算，她只负责用说信息的口吻说出来。
//!
//! - 每人最近 20 局、全部模式；和这局同一个队列的单独算（峡谷和大乱斗的伤害不放在一起比）；
//! - 突破口：在对面里比对面的平均，胜率 / KDA / 同模式分均伤害偏低、连败、第一次玩这个英雄加分，
//!   连胜、常玩这个英雄且赢得多减分；分数最高的一两个，原因一起写出来。

use crate::history::GameRecord;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Summary {
    pub games: usize,
    pub wins: usize,
    /// 正数 = 连胜几局，负数 = 连败几局（从最近一局往前数）。
    pub streak: i32,
    pub kda: f64,
    /// 场均死亡。
    pub deaths: f64,
    /// 和这局同一个队列的局数、胜场、分均伤害。
    pub mode_games: usize,
    pub mode_wins: usize,
    pub mode_dpm: f64,
    /// 这个英雄的局数、胜场。
    pub champ_games: usize,
    pub champ_wins: usize,
}

impl Summary {
    pub fn win_rate(&self) -> f64 {
        if self.games == 0 {
            0.0
        } else {
            self.wins as f64 / self.games as f64
        }
    }
}

/// `games` 新的在前。
pub fn summarize(games: &[GameRecord], champion_id: i64, queue_id: i64) -> Summary {
    let mut out = Summary {
        games: games.len(),
        wins: games.iter().filter(|g| g.win).count(),
        ..Summary::default()
    };
    if let Some(first) = games.first() {
        let n = games.iter().take_while(|g| g.win == first.win).count() as i32;
        out.streak = if first.win { n } else { -n };
    }
    let (k, d, a) = games.iter().fold((0, 0, 0), |(k, d, a), g| {
        (k + g.kills, d + g.deaths, a + g.assists)
    });
    if !games.is_empty() {
        out.kda = (k + a) as f64 / d.max(1) as f64;
        out.deaths = d as f64 / games.len() as f64;
    }
    let same: Vec<&GameRecord> = games
        .iter()
        .filter(|g| queue_id != 0 && g.queue_id == queue_id)
        .collect();
    out.mode_games = same.len();
    out.mode_wins = same.iter().filter(|g| g.win).count();
    let minutes: f64 = same.iter().map(|g| g.minutes()).sum();
    if minutes > 0.0 {
        out.mode_dpm = same.iter().map(|g| g.damage as f64).sum::<f64>() / minutes;
    }
    let champ: Vec<&GameRecord> = games
        .iter()
        .filter(|g| champion_id != 0 && g.champion_id == champion_id)
        .collect();
    out.champ_games = champ.len();
    out.champ_wins = champ.iter().filter(|g| g.win).count();
    out
}

/// 至少这么多局才拿来比。
pub const MIN_GAMES: usize = 3;

/// 第一次玩这个英雄（最近 20 局一局没有），要至少有几局才算数。
pub fn first_time(s: &Summary) -> bool {
    s.games >= 5 && s.champ_games == 0
}

/// 常玩这个英雄（最近的局里至少 5 局）。
pub fn main_champion(s: &Summary) -> bool {
    s.champ_games >= 5
}

/// 标签（给每个人那一行用）。
pub fn tags(s: &Summary) -> Vec<String> {
    let mut out = Vec::new();
    if s.streak >= 3 {
        out.push(format!("{} 连胜", s.streak));
    } else if s.streak <= -3 {
        out.push(format!("{} 连败", -s.streak));
    }
    if first_time(s) {
        out.push("这个英雄最近没玩过".into());
    } else if main_champion(s) {
        out.push(format!(
            "常玩这个英雄（{} 局 {} 胜）",
            s.champ_games, s.champ_wins
        ));
    }
    out
}

#[derive(Debug, Clone, PartialEq)]
pub struct WeakSpot {
    /// 传进来的下标。
    pub index: usize,
    pub score: f64,
    pub reasons: Vec<String>,
}

fn average(values: impl Iterator<Item = f64>) -> Option<f64> {
    let v: Vec<f64> = values.collect();
    (!v.is_empty()).then(|| v.iter().sum::<f64>() / v.len() as f64)
}

/// 对面谁是突破口：传对面每个人的（下标，统计），返回最多两个，分数 ≥ 1 才算。
pub fn weak_spots(enemies: &[(usize, &Summary)]) -> Vec<WeakSpot> {
    let known: Vec<&(usize, &Summary)> = enemies
        .iter()
        .filter(|(_, s)| s.games >= MIN_GAMES)
        .collect();
    if known.len() < 2 {
        return Vec::new();
    }
    let wr_avg = average(known.iter().map(|(_, s)| s.win_rate())).unwrap_or(0.5);
    let kda_avg = average(known.iter().map(|(_, s)| s.kda)).unwrap_or(0.0);
    let deaths_avg = average(known.iter().map(|(_, s)| s.deaths)).unwrap_or(0.0);
    let dpm_avg = average(
        known
            .iter()
            .filter(|(_, s)| s.mode_games >= MIN_GAMES)
            .map(|(_, s)| s.mode_dpm),
    );
    let dpm_min = known
        .iter()
        .filter(|(_, s)| s.mode_games >= MIN_GAMES)
        .map(|(_, s)| s.mode_dpm)
        .fold(f64::INFINITY, f64::min);

    let mut out: Vec<WeakSpot> = known
        .iter()
        .map(|(index, s)| {
            let mut score = 0.0;
            let mut reasons = Vec::new();
            let wr = s.win_rate();
            if wr < wr_avg - 0.05 {
                score += (wr_avg - wr) * 5.0;
                reasons.push(format!("最近 {} 局只赢了 {} 局", s.games, s.wins));
            }
            if kda_avg > 0.0 && s.kda < kda_avg * 0.85 {
                score += (1.0 - s.kda / kda_avg) * 2.0;
                reasons.push(format!("KDA {:.1}（对面平均 {:.1}）", s.kda, kda_avg));
            }
            if deaths_avg > 0.0 && s.deaths > deaths_avg * 1.2 {
                score += 0.5;
                reasons.push(format!("场均死 {:.1} 次", s.deaths));
            }
            if let Some(avg) = dpm_avg {
                if s.mode_games >= MIN_GAMES && avg > 0.0 && s.mode_dpm < avg * 0.85 {
                    score += (1.0 - s.mode_dpm / avg) * 2.0;
                    let lowest = if (s.mode_dpm - dpm_min).abs() < 0.5 {
                        "，对面最低"
                    } else {
                        ""
                    };
                    reasons.push(format!(
                        "这个模式分均伤害 {:.0}（对面平均 {:.0}{lowest}）",
                        s.mode_dpm, avg
                    ));
                }
            }
            if s.streak <= -3 {
                score += 1.0 + 0.25 * f64::from(-s.streak - 3);
                reasons.push(format!("{} 连败", -s.streak));
            } else if s.streak >= 3 {
                score -= 1.0;
            }
            if first_time(s) {
                score += 1.0;
                reasons.push("这个英雄最近 20 局没玩过".into());
            } else if main_champion(s) && s.champ_wins * 10 >= s.champ_games * 6 {
                score -= 0.5;
            }
            WeakSpot {
                index: *index,
                score,
                reasons,
            }
        })
        .filter(|w| w.score >= 1.0 && !w.reasons.is_empty())
        .collect();
    out.sort_by(|a, b| b.score.total_cmp(&a.score));
    out.truncate(2);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(
        ago: i64,
        queue: i64,
        champion: i64,
        win: bool,
        kda: (i64, i64, i64),
        damage: i64,
    ) -> GameRecord {
        GameRecord {
            game_id: 1000 - ago,
            queue_id: queue,
            mode: String::new(),
            created_ms: 1_000_000 - ago,
            duration_s: 600,
            champion_id: champion,
            win,
            kills: kda.0,
            deaths: kda.1,
            assists: kda.2,
            damage,
            taken: 0,
            gold: 0,
            cs: 0,
            augments: vec![],
        }
    }

    #[test]
    fn summary_counts_streak_kda_mode_and_champion() {
        let games = vec![
            rec(0, 2400, 39, false, (2, 8, 4), 20000),
            rec(1, 2400, 157, false, (3, 7, 5), 15000),
            rec(2, 2400, 39, false, (1, 9, 6), 10000),
            rec(3, 420, 39, true, (10, 2, 3), 30000),
        ];
        let s = summarize(&games, 39, 2400);
        assert_eq!((s.games, s.wins, s.streak), (4, 1, -3));
        assert!((s.kda - 34.0 / 26.0).abs() < 1e-9);
        assert_eq!((s.mode_games, s.mode_wins), (3, 0));
        // 45000 伤害 / 30 分钟
        assert!((s.mode_dpm - 1500.0).abs() < 1e-9);
        assert_eq!((s.champ_games, s.champ_wins), (3, 1));
        assert_eq!(tags(&s), ["3 连败"]);
        assert_eq!(summarize(&[], 39, 2400), Summary::default());
    }

    fn summary(
        games: usize,
        wins: usize,
        streak: i32,
        kda: f64,
        deaths: f64,
        dpm: f64,
        champ: usize,
    ) -> Summary {
        Summary {
            games,
            wins,
            streak,
            kda,
            deaths,
            mode_games: games,
            mode_wins: wins,
            mode_dpm: dpm,
            champ_games: champ,
            champ_wins: champ / 2,
        }
    }

    #[test]
    fn the_losing_unfamiliar_low_damage_player_is_the_weak_spot() {
        let strong = summary(20, 14, 4, 4.0, 4.0, 2400.0, 8);
        let normal = summary(20, 10, 1, 3.0, 5.0, 2000.0, 2);
        let weak = summary(20, 6, -4, 1.6, 7.5, 1300.0, 0);
        let tiny = summary(1, 0, -1, 0.5, 9.0, 100.0, 0);
        let enemies = [
            (0, &strong),
            (1, &normal),
            (2, &weak),
            (3, &normal),
            (4, &tiny),
        ];
        let spots = weak_spots(&enemies);
        assert_eq!(spots[0].index, 2, "{spots:?}");
        let reasons = spots[0].reasons.join("；");
        for want in [
            "4 连败",
            "这个英雄最近 20 局没玩过",
            "对面最低",
            "最近 20 局只赢了 6 局",
        ] {
            assert!(reasons.contains(want), "{reasons}");
        }
        // 只有一局的不拿来比；连胜的不是突破口
        assert!(spots.iter().all(|w| w.index != 4 && w.index != 0));
    }

    #[test]
    fn no_weak_spot_when_everyone_is_alike() {
        let a = summary(20, 10, 1, 3.0, 5.0, 2000.0, 2);
        assert!(weak_spots(&[(0, &a), (1, &a), (2, &a)]).is_empty());
        assert!(weak_spots(&[(0, &a)]).is_empty(), "一个人没法比");
    }
}
