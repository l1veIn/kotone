//! 战绩统计（ADR 035 D3）：每个人最近的胜负、连胜连败、KDA、这个英雄打过几局。只数数，不评价人
//! （ADR 043 D2：谁是突破口、谁会坑是她的判断，插件不推测；原来的 `weak_spots` 去掉了）。
//!
//! - 每人最近 20 局、全部模式；和这局同一个队列的单独算（峡谷和大乱斗的伤害不放在一起比）；
//! - 随机英雄的模式（大乱斗、海斗，见 [`crate::client::gamedata::random_pick`]）不看这个英雄玩没玩过：英雄是随机的，
//!   实测海斗里几乎每个人都“最近没玩过”，是噪音。

use crate::client::history::GameRecord;

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

/// 第一次玩这个英雄（最近 20 局一局没有），要至少有几局才算数。
pub fn first_time(s: &Summary) -> bool {
    s.games >= 5 && s.champ_games == 0
}

/// 常玩这个英雄（最近的局里至少 5 局）。
pub fn main_champion(s: &Summary) -> bool {
    s.champ_games >= 5
}

/// 标签（给每个人那一行用）。`random_pick` = 这局是随机英雄的模式。
pub fn tags(s: &Summary, random_pick: bool) -> Vec<String> {
    tagged(s, random_pick)
        .into_iter()
        .map(|(_, text)| text)
        .collect()
}

/// 同 [`tags`]，每条带种类（页面按种类上色）：`win_streak` / `loss_streak` / `first_time` / `main`。
pub fn tagged(s: &Summary, random_pick: bool) -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    if s.streak >= 3 {
        out.push(("win_streak", format!("{} 连胜", s.streak)));
    } else if s.streak <= -3 {
        out.push(("loss_streak", format!("{} 连败", -s.streak)));
    }
    if random_pick {
        // 英雄是随机的，玩没玩过不说明什么
    } else if first_time(s) {
        out.push(("first_time", "这个英雄最近没玩过".into()));
    } else if main_champion(s) {
        out.push((
            "main",
            format!("常玩这个英雄（{} 局 {} 胜）", s.champ_games, s.champ_wins),
        ));
    }
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
            gold: 0,
            cs: 0,
            augments: vec![],
            ..GameRecord::default()
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
        assert_eq!(tags(&s, false), ["3 连败"]);
        assert_eq!(summarize(&[], 39, 2400), Summary::default());
    }

    fn summary(games: usize, wins: usize, streak: i32, champ: usize) -> Summary {
        Summary {
            games,
            wins,
            streak,
            kda: 3.0,
            deaths: 5.0,
            mode_games: games,
            mode_wins: wins,
            mode_dpm: 2000.0,
            champ_games: champ,
            champ_wins: champ / 2,
        }
    }

    #[test]
    fn random_pick_modes_ignore_champion_familiarity() {
        // 海斗实测：英雄是随机的，几乎每个人这个英雄都是 0 局
        let fresh = summary(20, 10, 1, 0);
        assert_eq!(tags(&fresh, false), ["这个英雄最近没玩过"]);
        assert!(tags(&fresh, true).is_empty());
        let main = summary(20, 10, 1, 6);
        assert!(tags(&main, false)[0].starts_with("常玩这个英雄"));
        assert!(tags(&main, true).is_empty());
        // 连胜连败两种模式都标
        assert_eq!(tags(&summary(20, 6, -4, 2), true), ["4 连败"]);
        assert_eq!(tags(&summary(20, 14, 3, 2), false), ["3 连胜"]);
    }
}
