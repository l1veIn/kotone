//! 今天的状态（ADR 043 B 轮第 3 条；讨论记录第二节推送清单“排队 / 房间”那一行：“今天的状态（几连胜 / 连败、
//! 哪个英雄一直输）”）。只用我自己的战绩（客户端 `/lol-match-history/v1/products/lol/{我}/matches`，最近 20 局），
//! 只说事实（D2）：今天几局、几胜几负、现在几连胜 / 连败、今天输了 2 局以上的英雄、各个模式几局。
//!
//! - **哪一天**：按每局的开局时间（`gameCreation`）、本地时间 0 点分界；推送的 key 是 `today:<本地日期>`，
//!   一天一张，打完一局回到房间时同一张原地更新。
//! - **算哪些局**：和战绩页一样（[`crate::client::history::parse_match_list`]）：只算匹配到的对局，自定义、训练、
//!   重开局不算。
//! - **连胜连败**：只数今天的，从最近一局往前数到第一局不一样的；跨过 0 点的不接着数（这张卡片说的是今天）。

use serde_json::{json, Value};

use crate::client::assets;
use crate::client::history::GameRecord;
use crate::client::GameData;
use crate::pages;

/// 清单 `[[pushes]]` 里这条推送的 id、卡片的名字。
pub(crate) const PUSH_TODAY: &str = "today";
pub(crate) const TITLE_TODAY: &str = "今天的状态";
/// 今天输了几局以上的英雄写出来（固定的规则，同样的战绩永远得到同样的结果）。
pub(crate) const LOST_AT_LEAST: usize = 2;
/// 查最近几局（够一天用；这些都是今天的时，卡片写“只看了最近 20 局”）。
pub(crate) const GAMES: u32 = 20;

/// 客户端的这几个阶段算“在房间里或排队”：从别的阶段进来时推一次今天的状态。
pub(crate) fn lobby_phase(phase: &str) -> bool {
    matches!(phase, "Lobby" | "Matchmaking" | "ReadyCheck")
}

/// 某个时刻（Unix 毫秒）是本地时间的哪一天：`2026-11-26`。用 kotone-core 已有的本地日期（`chrono::Local`，
/// 给 DeepSeek 的日期提示也是它：`2026-11-26,星期四`），取逗号前面那段。
pub(crate) fn local_date(ms: i64) -> String {
    kotone_core::agent::reminder_date(ms.max(0) as u64)
        .split(',')
        .next()
        .unwrap_or_default()
        .to_string()
}

/// 战绩列表里每一局的 id（重开局、自定义也在里面）：结算后等这局出现在战绩里用。
pub(crate) fn listed_ids(list: &Value) -> Vec<i64> {
    list["games"]["games"]
        .as_array()
        .map(|games| games.iter().filter_map(|g| g["gameId"].as_i64()).collect())
        .unwrap_or_default()
}

/// 今天的几局（新的在前，和 `games` 的顺序一样）：开局时间是 `today` 这一天的。`date_of` 把毫秒换成本地日期
/// （正式的是 [`local_date`]，测试里换成固定时区）。
pub(crate) fn today_games<'a>(
    games: &'a [GameRecord],
    today: &str,
    date_of: impl Fn(i64) -> String,
) -> Vec<&'a GameRecord> {
    games
        .iter()
        .filter(|g| g.created_ms > 0 && date_of(g.created_ms) == today)
        .collect()
}

/// 连胜（正数）/ 连败（负数）：从最近一局往前数；没有局是 0。`games` 新的在前。
pub(crate) fn streak(games: &[&GameRecord]) -> i64 {
    let Some(first) = games.first() else {
        return 0;
    };
    let n = games.iter().take_while(|g| g.win == first.win).count() as i64;
    if first.win {
        n
    } else {
        -n
    }
}

/// 一个英雄今天的局数和胜负。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ChampionDay {
    pub(crate) champion_id: i64,
    pub(crate) games: usize,
    pub(crate) wins: usize,
}

impl ChampionDay {
    pub(crate) fn losses(&self) -> usize {
        self.games - self.wins
    }
}

/// 今天输了 [`LOST_AT_LEAST`] 局以上的英雄：输得多的在前；一样多时局数多的在前，再一样按最近玩的在前。
pub(crate) fn lost_often(games: &[&GameRecord]) -> Vec<ChampionDay> {
    let mut days: Vec<ChampionDay> = Vec::new();
    // games 新的在前：第一次见到的就是最近玩的，排序稳定，同样多时保持这个顺序
    for g in games {
        match days.iter_mut().find(|d| d.champion_id == g.champion_id) {
            Some(d) => {
                d.games += 1;
                d.wins += usize::from(g.win);
            }
            None => days.push(ChampionDay {
                champion_id: g.champion_id,
                games: 1,
                wins: usize::from(g.win),
            }),
        }
    }
    days.retain(|d| d.losses() >= LOST_AT_LEAST);
    days.sort_by(|a, b| b.losses().cmp(&a.losses()).then(b.games.cmp(&a.games)));
    days
}

/// 今天各个模式（队列）几局几胜：局数多的在前，一样多按最近玩的在前。
fn modes(games: &[&GameRecord], data: Option<&GameData>) -> Vec<Value> {
    let mut rows: Vec<(i64, String, usize, usize)> = Vec::new();
    for g in games {
        match rows.iter_mut().find(|(queue, ..)| *queue == g.queue_id) {
            Some(row) => {
                row.2 += 1;
                row.3 += usize::from(g.win);
            }
            None => rows.push((
                g.queue_id,
                pages::queue(data, g.queue_id, &g.mode),
                1,
                usize::from(g.win),
            )),
        }
    }
    rows.sort_by_key(|row| std::cmp::Reverse(row.2));
    rows.into_iter()
        .map(|(id, queue, games, wins)| json!({"queueId": id, "queue": queue, "games": games, "wins": wins}))
        .collect()
}

/// 页面数据（`ui://kotone/lol/today`）。`games` 是今天的（新的在前）；`capped`：查到的最近 [`GAMES`] 局全是今天的，
/// 今天可能还有更早的。里面没有时间戳之类每次都变的东西：没打新的一局时内容一样，宿主什么都不做。
pub(crate) fn today_page(
    date: &str,
    games: &[&GameRecord],
    capped: bool,
    data: Option<&GameData>,
) -> Value {
    let wins = games.iter().filter(|g| g.win).count();
    json!({
        "date": date,
        "games": games.len(),
        "wins": wins,
        "losses": games.len() - wins,
        "streak": streak(games),
        "capped": capped,
        "limit": GAMES,
        "lostAtLeast": LOST_AT_LEAST,
        "lostOften": lost_often(games).iter().map(|d| json!({
            "champion": pages::champion(data, d.champion_id),
            "games": d.games,
            "wins": d.wins,
            "losses": d.losses(),
        })).collect::<Vec<_>>(),
        "modes": modes(games, data),
        // 和“这局”页面每个人的近况一样的形状（页面的 Recent 小头像）
        "recent": games.iter().map(|g| json!({
            "gameId": g.game_id,
            "win": g.win,
            "championId": g.champion_id,
            "championIcon": assets::champion_icon(g.champion_id),
            "kills": g.kills,
            "deaths": g.deaths,
            "assists": g.assists,
        })).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 北京时间（UTC+8）的日期：测试不依赖这台电脑的时区。
    fn beijing(ms: i64) -> String {
        let days = (ms + 8 * 3_600_000).div_euclid(86_400_000);
        // 1970-01-01 起的天数 → 年月日（Howard Hinnant 的 civil_from_days）
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = if mp < 10 { mp + 3 } else { mp - 9 };
        let y = yoe + era * 400 + i64::from(m <= 2);
        format!("{y:04}-{m:02}-{d:02}")
    }

    /// 2026-11-26 00:00 北京时间（= 2026-11-25 16:00 UTC）。
    const MIDNIGHT: i64 = 1_795_622_400_000;
    const HOUR: i64 = 3_600_000;

    fn game(id: i64, at: i64, champion: i64, win: bool) -> GameRecord {
        GameRecord {
            game_id: id,
            queue_id: if champion == 157 { 420 } else { 2400 },
            mode: if champion == 157 { "CLASSIC" } else { "KIWI" }.into(),
            created_ms: at,
            duration_s: 1200,
            champion_id: champion,
            win,
            kills: 5,
            deaths: 3,
            assists: 7,
            ..GameRecord::default()
        }
    }

    /// 新的在前（和战绩列表一样）：今天 6 局（亚索 0 胜 2 负、阿狸 1 胜 2 负、艾希 1 胜），最后两局连败；
    /// 昨天 23:50 开的那一局（跨过了 0 点）算昨天。
    fn history() -> Vec<GameRecord> {
        vec![
            game(9, MIDNIGHT + 22 * HOUR, 103, false),
            game(8, MIDNIGHT + 21 * HOUR, 157, false),
            game(7, MIDNIGHT + 20 * HOUR, 103, true),
            game(6, MIDNIGHT + 3 * HOUR, 157, false),
            game(5, MIDNIGHT + 2 * HOUR, 103, false),
            game(4, MIDNIGHT + 60_000, 22, true),
            game(3, MIDNIGHT - 10 * 60_000, 22, false),
            game(2, MIDNIGHT - 5 * HOUR, 22, false),
        ]
    }

    #[test]
    fn the_day_is_split_at_local_midnight() {
        assert_eq!(beijing(MIDNIGHT), "2026-11-26");
        assert_eq!(beijing(MIDNIGHT - 1), "2026-11-25");
        let all = history();
        let today = today_games(&all, "2026-11-26", beijing);
        assert_eq!(
            today.iter().map(|g| g.game_id).collect::<Vec<_>>(),
            [9, 8, 7, 6, 5, 4]
        );
        let yesterday = today_games(&all, "2026-11-25", beijing);
        assert_eq!(
            yesterday.iter().map(|g| g.game_id).collect::<Vec<_>>(),
            [3, 2],
            "23:50 开的那一局算昨天"
        );
        assert!(today_games(&all, "2026-11-27", beijing).is_empty());
        // 没有开局时间的（读不到）不算
        let broken = [game(1, 0, 22, true)];
        assert!(today_games(&broken, "1970-01-01", beijing).is_empty());
        // 正式的日期是本机的本地时间，写法一样
        let real = local_date(MIDNIGHT);
        assert_eq!(real.len(), 10, "{real}");
        assert_eq!(real.as_bytes()[4], b'-');
    }

    #[test]
    fn streaks_count_back_from_the_latest_game_of_today() {
        let all = history();
        let today = today_games(&all, "2026-11-26", beijing);
        assert_eq!(streak(&today), -2);
        assert_eq!(streak(&today[2..]), 1);
        assert_eq!(streak(&[]), 0);
        // 昨天的两连败不接着今天的数：今天第一局赢了，今天只有它时是 1 连胜
        let only_first = today_games(&all[5..], "2026-11-26", beijing);
        assert_eq!(streak(&only_first), 1);
        let wins = [
            game(1, 1, 22, true),
            game(2, 2, 22, true),
            game(3, 3, 22, true),
        ];
        let refs: Vec<&GameRecord> = wins.iter().collect();
        assert_eq!(streak(&refs), 3);
    }

    #[test]
    fn champions_that_lost_twice_or_more_today_are_listed_most_losses_first() {
        let all = history();
        let today = today_games(&all, "2026-11-26", beijing);
        let lost = lost_often(&today);
        // 亚索 2 局 2 负、阿狸 3 局 2 负：一样输 2 局，局数多的阿狸在前；艾希只输 0 局（昨天的不算）
        assert_eq!(
            lost,
            [
                ChampionDay {
                    champion_id: 103,
                    games: 3,
                    wins: 1
                },
                ChampionDay {
                    champion_id: 157,
                    games: 2,
                    wins: 0
                },
            ]
        );
        // 输一局的不写
        assert!(lost_often(&today[..2]).is_empty());
    }

    #[test]
    fn the_page_states_the_day_and_is_the_same_until_a_new_game() {
        let all = history();
        let today = today_games(&all, "2026-11-26", beijing);
        let v = today_page("2026-11-26", &today, false, None);
        assert_eq!(v["date"], "2026-11-26");
        assert_eq!(
            (
                v["games"].as_u64(),
                v["wins"].as_u64(),
                v["losses"].as_u64()
            ),
            (Some(6), Some(2), Some(4))
        );
        assert_eq!(v["streak"], -2);
        assert_eq!(v["lostAtLeast"], 2);
        assert_eq!(v["lostOften"][0]["champion"]["id"], 103);
        assert_eq!(v["lostOften"][0]["losses"], 2);
        assert_eq!(v["lostOften"][1]["champion"]["id"], 157);
        assert_eq!(v["lostOften"][1]["wins"], 0);
        // 模式：海斗 4 局 2 胜、单双排 2 局 0 胜（没有客户端的数据时用模式的中文名）
        assert_eq!(v["modes"][0]["queueId"], 2400);
        assert_eq!(v["modes"][0]["games"], 4);
        assert_eq!(v["modes"][0]["wins"], 2);
        assert_eq!(v["modes"][1]["games"], 2);
        assert!(!v["modes"][1]["queue"].as_str().unwrap().is_empty());
        // 近况：今天每一局，新的在前，形状和“这局”的近况一样
        let recent = v["recent"].as_array().unwrap();
        assert_eq!(recent.len(), 6);
        assert_eq!(recent[0]["gameId"], 9);
        assert_eq!(recent[0]["win"], false);
        assert_eq!(
            recent[0]["championIcon"],
            "/lol-game-data/assets/v1/champion-icons/103.png"
        );
        // 同样的战绩 → 同样的数据（宿主按内容判断“没变”）；多了一局才不一样
        assert_eq!(v, today_page("2026-11-26", &today, false, None));
        let mut more = all.clone();
        more.insert(0, game(10, MIDNIGHT + 23 * HOUR, 22, true));
        let after = today_games(&more, "2026-11-26", beijing);
        let v2 = today_page("2026-11-26", &after, false, None);
        assert_ne!(v, v2);
        assert_eq!(v2["streak"], 1);
        assert_eq!(v2["games"], 7);
    }

    #[test]
    fn listed_ids_include_games_the_history_skips() {
        let list = json!({"games": {"games": [
            {"gameId": 3, "gameType": "MATCHED_GAME", "gameDuration": 200},
            {"gameId": 2, "gameType": "CUSTOM_GAME"},
        ]}});
        assert_eq!(listed_ids(&list), [3, 2]);
        assert!(listed_ids(&json!({"httpStatus": 404})).is_empty());
    }

    #[test]
    fn lobby_and_queue_are_where_the_card_comes() {
        for p in ["Lobby", "Matchmaking", "ReadyCheck"] {
            assert!(lobby_phase(p), "{p}");
        }
        for p in [
            "",
            "None",
            "ChampSelect",
            "InProgress",
            "EndOfGame",
            "WaitingForStats",
        ] {
            assert!(!lobby_phase(p), "{p}");
        }
    }
}
