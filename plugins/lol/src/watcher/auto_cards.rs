//! 英雄联盟的自动卡片（ADR 039 D3；时机照调研 `docs/research/2026-11-25-lol-app-parity.md` 第五节、设计文档
//! `docs/design/lol-app.md` 第四节）。一局最多两张，不叫模型：
//!
//! - **开局战绩**：进加载（游戏开始、局内接口还没有游戏时间）以后，10 个人的战绩都查完就发；进加载 20 秒还没查完
//!   也发（卡片和那句话里写“还在查 N 人”）。页面是“这局”（`ui://kotone/lol/match`），数据和 `lol.scout` 的一样；
//!   给琴音的那句复用突破口的算法（`scout::summarize` / `weak_spots`），不另拼结论。
//! - **这局总结**：从对局里进了结算（`WaitingForStats` / `PreEndOfGame` / `EndOfGame`）就去读这局的详情，拿到就发；
//!   读不到每 5 秒再试，60 秒还没有就不发（调研 G29）。页面是“单局详情”（`ui://kotone/lol/game`，带 `justEnded`）。
//! - 不发：选人、对局中；训练模式、自定义对局（队列 ≤ 0）、观战（这局里没有自己）；Kotone 开的时候已经在对局里了
//!   （没看到加载）不发开局战绩；重开局（提前投降）不发这局总结；重连、刷新不重发（这里记着，宿主也按 key 去重）。
//! - 开关在“游戏陪玩 → 角色 → 交互”（宿主管，关了 `Host::auto_card` 回 `Ok(false)`）。

use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use kotone_core::log;
use kotone_plugin_api::{AutoCard, Host};
use serde_json::{json, Value};

use super::MatchView;
use crate::client::http::paths;
use crate::client::scout::{self, Summary};
use crate::client::text::ScoutPlayer;
use crate::pages;
use crate::LolPlugin;

/// 进加载多久还没查完也发开局战绩。
const OPENING_WAIT: Duration = Duration::from_secs(20);
/// 结算后最多等多久这局的详情。
const SUMMARY_WAIT: Duration = Duration::from_secs(60);
const SUMMARY_RETRY: Duration = Duration::from_secs(5);
/// 比这还短的一局算重开局（提前投降的“重开”在 3 分多钟）。
const REMAKE_SECONDS: i64 = 240;

pub(crate) const TITLE_OPENING: &str = "开局战绩";
pub(crate) const TITLE_SUMMARY: &str = "这局总结";

/// 发过哪局的哪种卡片、这局什么时候进的加载。客户端重连时跟着 `State` 一起清掉（宿主那边还按 key 去重）。
#[derive(Debug, Default)]
pub(crate) struct CardState {
    /// （对局 id，进加载的时刻）。
    loading: Option<(i64, Instant)>,
    /// 开局战绩已经发过（或这局不发）的对局。
    opened: HashSet<i64>,
    /// 这局总结已经排上（或这局不发）的对局。
    summarized: HashSet<i64>,
}

impl CardState {
    /// 这一轮在对局里（`in_game`）：该不该发这局的开局战绩。`live_ready` = 局内接口已经有游戏时间（不是加载了）；
    /// `fetched` = 10 个人都查完了。返回 true 时已经记成发过。
    pub(crate) fn opening_due(
        &mut self,
        game_id: i64,
        live_ready: bool,
        fetched: bool,
        now: Instant,
    ) -> bool {
        if self.opened.contains(&game_id) {
            return false;
        }
        let since = match self.loading {
            Some((id, since)) if id == game_id => since,
            _ if live_ready => {
                // 第一次看到这局时已经开打了（Kotone 晚开、重连）：不是“开局”，不发
                self.opened.insert(game_id);
                return false;
            }
            _ => {
                self.loading = Some((game_id, now));
                now
            }
        };
        if fetched || now.duration_since(since) >= OPENING_WAIT {
            self.opened.insert(game_id);
            return true;
        }
        false
    }

    /// 这局刚结束：该不该去发这局总结（一局一次）。
    pub(crate) fn summary_due(&mut self, game_id: i64) -> bool {
        self.summarized.insert(game_id)
    }
}

/// 这局发不发卡片：自定义、训练（队列 ≤ 0）不发；观战（这局里没有自己）不发。
pub(crate) fn eligible(queue_id: i64, mode: &str, players: &[ScoutPlayer]) -> bool {
    queue_id > 0 && !mode.eq_ignore_ascii_case("PRACTICETOOL") && players.iter().any(|p| p.me)
}

fn summaries(view: &MatchView) -> Vec<Option<Summary>> {
    view.players
        .iter()
        .map(|p| {
            p.history
                .as_ref()
                .map(|h| scout::summarize(&h.games, p.champion_id, view.queue_id))
        })
        .collect()
}

/// 开局战绩给琴音的那句（约 120 字以内）：模式、对面的突破口（最多两个人、每人最多两条原因）、我方开黑、还在查几个人。
pub(crate) fn opening_note(view: &MatchView) -> String {
    let summaries = summaries(view);
    let enemies: Vec<(usize, &Summary)> = view
        .players
        .iter()
        .enumerate()
        .filter(|(_, p)| !p.ally)
        .filter_map(|(i, _)| summaries[i].as_ref().map(|s| (i, s)))
        .collect();
    let spots = scout::weak_spots(&enemies, view.random);
    let mut out = format!("{TITLE_OPENING}（{}）。", view.mode);
    if spots.is_empty() {
        out.push_str("对面几个人最近的数据差不多，看不出明显的突破口。");
    } else {
        let lines: Vec<String> = spots
            .iter()
            .map(|w| {
                let reasons: Vec<&str> = w.reasons.iter().take(2).map(String::as_str).collect();
                format!("{}，{}", view.players[w.index].short, reasons.join("、"))
            })
            .collect();
        out.push_str(&format!("对面可能的突破口：{}。", lines.join("；")));
    }
    let party = largest_party(view.players.iter().filter(|p| p.ally));
    if party >= 2 {
        out.push_str(&format!("我方有 {party} 人开黑。"));
    }
    let waiting = view.players.iter().filter(|p| p.history.is_none()).count();
    if waiting > 0 {
        out.push_str(&format!("还在查 {waiting} 人。"));
    }
    out
}

/// 一边最大的开黑小队有几个人。
fn largest_party<'a>(players: impl Iterator<Item = &'a ScoutPlayer>) -> usize {
    let mut counts = std::collections::HashMap::new();
    for p in players {
        if let Some(id) = p.party.filter(|id| *id > 0) {
            *counts.entry(id).or_insert(0usize) += 1;
        }
    }
    counts.into_values().max().unwrap_or(0)
}

/// 这局是不是重开局（提前投降、有人没进）。
pub(crate) fn remake(detail: &Value) -> bool {
    let early = detail["participants"].as_array().is_some_and(|ps| {
        ps.iter().any(|p| {
            p["stats"]["gameEndedInEarlySurrender"]
                .as_bool()
                .unwrap_or(false)
        })
    });
    let seconds = detail["gameDuration"].as_i64().unwrap_or(0);
    early || (seconds > 0 && seconds < REMAKE_SECONDS)
}

/// 详情是不是出来了（结算后一会儿才有：没有人或没有数据时还不算）。
fn detail_ready(detail: &Value) -> bool {
    detail["participants"]
        .as_array()
        .is_some_and(|ps| !ps.is_empty() && ps.iter().all(|p| p["stats"].is_object()))
}

fn rank_word(rank: usize) -> String {
    const WORDS: [&str; 5] = ["第一", "第二", "第三", "第四", "第五"];
    WORDS
        .get(rank)
        .map(|w| (*w).to_string())
        .unwrap_or_else(|| format!("第 {}", rank + 1))
}

/// 这局总结给琴音的那句：输赢、模式、时长；我用什么英雄、K/D/A、伤害在全队排第几。`page` 是单局详情的页面数据。
pub(crate) fn summary_note(page: &Value, me: &str) -> String {
    let seconds = page["durationS"].as_i64().unwrap_or(0);
    let length = format!("{} 分 {} 秒", seconds / 60, seconds % 60);
    let queue = page["queue"].as_str().unwrap_or("");
    let teams = page["teams"].as_array().cloned().unwrap_or_default();
    let mine = teams.iter().find_map(|team| {
        let players = team["players"].as_array()?;
        let index = players
            .iter()
            .position(|p| !me.is_empty() && p["puuid"].as_str() == Some(me))?;
        Some((team, players, index))
    });
    let Some((team, players, index)) = mine else {
        return format!("{TITLE_SUMMARY}：这局结束了（{queue}，{length}）。");
    };
    let win = team["win"].as_bool().unwrap_or(false);
    let p = &players[index];
    let damage = p["damage"].as_i64().unwrap_or(0);
    let rank = players
        .iter()
        .filter(|x| x["damage"].as_i64().unwrap_or(0) > damage)
        .count();
    format!(
        "{TITLE_SUMMARY}：这局{}了（{queue}，{length}）。你用{} {}/{}/{}，伤害全队{}。",
        if win { "赢" } else { "输" },
        p["champion"]["short"].as_str().unwrap_or("英雄"),
        p["kills"].as_i64().unwrap_or(0),
        p["deaths"].as_i64().unwrap_or(0),
        p["assists"].as_i64().unwrap_or(0),
        rank_word(rank),
    )
}

fn report(title: &str, result: Result<bool, String>) {
    match result {
        Ok(true) => {}
        Ok(false) => log::log(&format!("lol: 自动卡片「{title}」没发（关着或发过了）")),
        Err(e) => log::log(&format!("lol: 自动卡片「{title}」发不出去：{e}")),
    }
}

impl LolPlugin {
    /// 后台每一轮的最后：按阶段决定发不发自动卡片（`previous` 是上一轮的客户端阶段）。
    pub(super) fn auto_cards(
        &self,
        host: &Arc<dyn Host>,
        previous: &str,
        phase: &str,
        live_ready: bool,
    ) {
        if crate::client::in_game_phase(phase) {
            self.opening_card(host.as_ref(), live_ready);
        } else if crate::client::in_game_phase(previous)
            && matches!(phase, "WaitingForStats" | "PreEndOfGame" | "EndOfGame")
        {
            self.summary_card(host);
        }
    }

    fn opening_card(&self, host: &dyn Host, live_ready: bool) {
        let due = {
            let mut s = self.state();
            let Some((game_id, fetched)) = s.current.as_ref().map(|m| {
                let fetched = m.members.len() >= 2
                    && m.pending.is_empty()
                    && m.members.iter().all(|x| m.histories.contains_key(&x.puuid));
                (m.game_id, fetched)
            }) else {
                return;
            };
            s.cards
                .opening_due(game_id, live_ready, fetched, Instant::now())
                .then_some(game_id)
        };
        let Some(game_id) = due else { return };
        let data = self.loaded_data();
        let Some(view) = self.match_view(data.as_deref()) else {
            return;
        };
        let native_mode = self
            .state()
            .current
            .as_ref()
            .map(|m| m.mode.clone())
            .unwrap_or_default();
        if !eligible(view.queue_id, &native_mode, &view.players) {
            log::log(&format!(
                "lol: 这局（队列 {}）不发开局战绩（自定义、训练或观战）",
                view.queue_id
            ));
            return;
        }
        let page = pages::match_page(
            &view.players,
            &view.puuids,
            view.game_id,
            view.queue_id,
            &view.mode,
            view.random,
            data.as_deref(),
        );
        let result = host.auto_card(AutoCard {
            key: format!("opening:{game_id}"),
            title: TITLE_OPENING.into(),
            uri: pages::PAGE_MATCH.into(),
            params: Value::Null,
            data: page,
            note: opening_note(&view),
            popup: "对面的战绩到了".into(),
        });
        report(TITLE_OPENING, result);
    }

    fn summary_card(&self, host: &Arc<dyn Host>) {
        let (game_id, lcu, me) = {
            let mut s = self.state();
            let Some(m) = s.current.as_ref() else { return };
            let game_id = m.game_id;
            let me = s
                .client
                .as_ref()
                .and_then(|c| c.me.clone())
                .unwrap_or_default();
            let custom = m.queue_id <= 0 || m.mode.eq_ignore_ascii_case("PRACTICETOOL");
            let watching = !me.is_empty() && !m.members.iter().any(|x| x.puuid == me);
            let Some(lcu) = s.client.as_ref().map(|c| c.lcu.clone()) else {
                return;
            };
            if !s.cards.summary_due(game_id) {
                return;
            }
            if custom || watching {
                log::log(&format!(
                    "lol: 这局（{game_id}）不发这局总结（自定义、训练或观战）"
                ));
                return;
            }
            (game_id, lcu, me)
        };
        let data = self.loaded_data();
        let task_host = host.clone();
        host.spawn(Box::pin(async move {
            let deadline = Instant::now() + SUMMARY_WAIT;
            let detail = loop {
                match lcu.get(&paths::game(game_id)).await {
                    Ok(detail) if detail_ready(&detail) => break detail,
                    _ => {}
                }
                if Instant::now() >= deadline {
                    log::log(&format!(
                        "lol: 这局（{game_id}）结算后 60 秒还拿不到结果，不发这局总结"
                    ));
                    return;
                }
                tokio::time::sleep(SUMMARY_RETRY).await;
            };
            if remake(&detail) {
                log::log(&format!("lol: 这局（{game_id}）是重开局，不发这局总结"));
                return;
            }
            let Some(mut page) = pages::game_page(&detail, data.as_deref()) else {
                return;
            };
            // 单局详情带“刚结束”的头部（设计文档第三节 3）
            page["justEnded"] = json!(true);
            let note = summary_note(&page, &me);
            let result = task_host.auto_card(AutoCard {
                key: format!("summary:{game_id}"),
                title: TITLE_SUMMARY.into(),
                uri: pages::PAGE_GAME.into(),
                params: json!({"gameId": page["gameId"]}),
                data: page,
                note,
                popup: "这局的总结好了".into(),
            });
            report(TITLE_SUMMARY, result);
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::history::{GameRecord, PlayerHistory};

    fn player(
        short: &str,
        ally: bool,
        me: bool,
        party: Option<i64>,
        games: &[bool],
    ) -> ScoutPlayer {
        ScoutPlayer {
            champion: short.into(),
            alias: String::new(),
            short: short.into(),
            champion_id: 1,
            name: format!("{short}的玩家"),
            ally,
            me,
            party,
            history: Some(PlayerHistory {
                privacy: "PUBLIC".into(),
                games: games
                    .iter()
                    .enumerate()
                    .map(|(i, win)| GameRecord {
                        game_id: i as i64,
                        queue_id: 2400,
                        champion_id: 2,
                        win: *win,
                        kills: if *win { 8 } else { 1 },
                        deaths: if *win { 3 } else { 12 },
                        assists: 10,
                        damage: if *win { 40_000 } else { 9_000 },
                        duration_s: 1200,
                        ..GameRecord::default()
                    })
                    .collect(),
                ..PlayerHistory::default()
            }),
            profile_icon_id: None,
            spells: [0, 0],
            position: String::new(),
        }
    }

    fn view(players: Vec<ScoutPlayer>) -> MatchView {
        MatchView {
            puuids: (0..players.len()).map(|i| format!("p{i}")).collect(),
            players,
            game_id: 42,
            queue_id: 2400,
            mode: "海克斯大乱斗".into(),
            random: true,
        }
    }

    #[test]
    fn opening_waits_for_the_histories_or_twenty_seconds_and_only_once() {
        let t0 = Instant::now();
        let mut s = CardState::default();
        // 进加载，还在查：不发
        assert!(!s.opening_due(42, false, false, t0));
        assert!(!s.opening_due(42, false, false, t0 + Duration::from_secs(10)));
        // 查完了：发；之后（对局中、重连）不再发
        assert!(s.opening_due(42, false, true, t0 + Duration::from_secs(12)));
        assert!(!s.opening_due(42, true, true, t0 + Duration::from_secs(30)));
        // 下一局：20 秒还没查完也发（开打了也算，进加载时看到过）
        assert!(!s.opening_due(43, false, false, t0));
        assert!(s.opening_due(43, true, false, t0 + OPENING_WAIT));
        // Kotone 开的时候已经开打了：不发
        assert!(!s.opening_due(44, true, true, t0));
        assert!(!s.opening_due(44, true, true, t0 + Duration::from_secs(60)));
        // 这局总结一局一次
        assert!(s.summary_due(42));
        assert!(!s.summary_due(42));
    }

    #[test]
    fn customs_practice_and_spectating_get_no_cards() {
        let me = vec![player("寒冰", true, true, None, &[true])];
        let watcher = vec![player("寒冰", true, false, None, &[true])];
        assert!(eligible(2400, "KIWI", &me));
        assert!(!eligible(0, "CLASSIC", &me));
        assert!(!eligible(-1, "CLASSIC", &me));
        assert!(!eligible(2400, "PRACTICETOOL", &me));
        assert!(!eligible(2400, "KIWI", &watcher));
    }

    #[test]
    fn the_opening_note_names_the_weak_spot_parties_and_who_is_still_loading() {
        let mut players = vec![
            player(
                "伊泽瑞尔",
                true,
                true,
                Some(7),
                &[true, true, false, true, true],
            ),
            player(
                "锤石",
                true,
                false,
                Some(7),
                &[true, false, true, true, false],
            ),
            player(
                "璐璐",
                true,
                false,
                Some(7),
                &[true, true, true, false, false],
            ),
            player("盖伦", true, false, None, &[false, true, true, false, true]),
            player(
                "拉克丝",
                true,
                false,
                None,
                &[true, false, true, true, true],
            ),
            player(
                "寒冰",
                false,
                false,
                None,
                &[false, false, false, false, false, true],
            ),
            player("亚索", false, false, None, &[true, true, false, true, true]),
            player(
                "金克丝",
                false,
                false,
                None,
                &[true, true, true, false, true],
            ),
            player(
                "莫甘娜",
                false,
                false,
                None,
                &[true, false, true, true, true],
            ),
            player(
                "布兰德",
                false,
                false,
                None,
                &[false, true, true, true, true],
            ),
        ];
        players[9].history = None;
        let note = opening_note(&view(players));
        assert!(note.starts_with("开局战绩（海克斯大乱斗）。"), "{note}");
        assert!(note.contains("对面可能的突破口：寒冰，"), "{note}");
        assert!(note.contains("我方有 3 人开黑"), "{note}");
        assert!(note.contains("还在查 1 人"), "{note}");
        assert!(!note.contains("的玩家"), "不写玩家名：{note}");
        assert!(note.chars().count() <= AutoCard::NOTE_MAX, "{note}");
    }

    fn page(my_damage: i64, win: bool) -> Value {
        json!({
            "gameId": 42, "queue": "海克斯大乱斗", "durationS": 1112,
            "teams": [
                {"teamId": 100, "win": win, "players": [
                    {"puuid": "me", "champion": {"short": "寒冰"}, "kills": 12, "deaths": 3, "assists": 20, "damage": my_damage},
                    {"puuid": "a", "champion": {"short": "锤石"}, "kills": 1, "deaths": 5, "assists": 25, "damage": 30_000},
                ]},
                {"teamId": 200, "win": !win, "players": [
                    {"puuid": "b", "champion": {"short": "亚索"}, "kills": 9, "deaths": 9, "assists": 2, "damage": 90_000},
                ]},
            ],
        })
    }

    #[test]
    fn the_summary_note_has_the_result_my_line_and_damage_rank() {
        assert_eq!(
            summary_note(&page(50_000, true), "me"),
            "这局总结：这局赢了（海克斯大乱斗，18 分 32 秒）。你用寒冰 12/3/20，伤害全队第一。"
        );
        let lost = summary_note(&page(10_000, false), "me");
        assert!(
            lost.contains("这局输了") && lost.contains("伤害全队第二"),
            "{lost}"
        );
        // 找不到自己（录的详情不是这一局）：只说结束了
        assert_eq!(
            summary_note(&page(1, true), "someone"),
            "这局总结：这局结束了（海克斯大乱斗，18 分 32 秒）。"
        );
    }

    #[test]
    fn remakes_and_unfinished_details_are_recognized() {
        let full = json!({"gameDuration": 1112, "participants": [{"stats": {"win": true}}]});
        assert!(detail_ready(&full) && !remake(&full));
        assert!(remake(
            &json!({"gameDuration": 200, "participants": [{"stats": {}}]})
        ));
        assert!(remake(&json!({"gameDuration": 900, "participants": [
            {"stats": {"gameEndedInEarlySurrender": true}}
        ]})));
        assert!(!detail_ready(&json!({"participants": []})));
        assert!(!detail_ready(&json!({"httpStatus": 404})));
    }
}
