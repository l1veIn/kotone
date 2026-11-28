//! 英雄联盟的三条卡片推送（ADR 039 D3、ADR 043 D3：清单 `[[pushes]]` 的 `opening`、`summary` 和 `today`；时机照调研
//! `docs/research/2026-11-25-lol-app-parity.md` 第五节、设计文档 `docs/design/lol-app.md` 第四节）。一局最多两张，不叫模型：
//!
//! - **开局战绩**：进加载（游戏开始、局内接口还没有游戏时间）以后，10 个人的战绩都查完就发；进加载 20 秒还没查完
//!   也发（卡片和那句话里写“还在查 N 人”）。页面是“这局”（`ui://kotone/lol/match`），数据和 `lol.scout` 的一样；
//!   给琴音的那句只写事实（连胜连败、这个英雄几局、开黑、隐藏了身份的人），不评价人（ADR 043 D2）。
//! - **这局总结**：从对局里进了结算（`WaitingForStats` / `PreEndOfGame` / `EndOfGame`）就去读这局的详情，拿到就发；
//!   读不到每 5 秒再试，60 秒还没有就不发（调研 G29）。页面是“单局详情”（`ui://kotone/lol/game`，带 `justEnded`）；
//!   海斗、斗魂竞技场的详情里有每个人的海克斯（`playerAugment1`–`6`），卡片上一人一行列出对面的（也能看我方的），
//!   没有海克斯的模式不显示（ADR 043 B 轮第 3 条）。卡片头部下面一行是一句预置的话（[`cheer`]，ADR 050 第 5 条：
//!   “C麻了”“没事，下把再来”），给琴音的那句里也带上（“卡片上写了：……”），免得她接着说重复的。
//!   同一时刻报一件 `lol.game_end`（ADR 050 第 6 条，[`game_end_data`]）：这条推送关着也报（事件和推送是两回事），
//!   自定义、训练、观战、重开局、60 秒还拿不到结果的不报。
//! - **今天的状态**（[`today`]）：进房间或排队（`Lobby` / `Matchmaking` / `ReadyCheck`）时，用我自己的战绩（和战绩页同一段）
//!   发一张：今天几胜几负、现在几连胜 / 连败、今天输了 2 局以上的英雄。一天一个 key（`today:<本地日期>`），打完一局
//!   回到房间时同一张原地更新、让悬浮窗再弹一次；今天还没打过不发。不给琴音留话：这张卡片一天里一直在变，
//!   留一句很快就过时了，她要今天的战绩时自己查（`lol.player`）。卡片上那句预置的话按今天的连胜连败、胜负挑
//!   （[`cheer::today_tier`]），同样不写进给她的话。
//! - 不发：选人、对局中；训练模式、自定义对局（队列 ≤ 0）、观战（这局里没有自己）；Kotone 开的时候已经在对局里了
//!   （没看到加载）不发开局战绩；重开局（提前投降）不发这局总结；重连、刷新不重发（这里记着，宿主也按 key 去重）。
//! - 开关在插件页「设置 → 推送」，每条一个（宿主管，关了 `Host::push` 回 `Off`）；同一个 key 再推一次宿主当成更新，
//!   内容一样就什么都不做。

use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use kotone_core::log;
use kotone_plugin_api::{Host, Push, PushCard, PushOutcome};
use serde_json::{json, Map, Value};

use super::MatchView;
use crate::client::history;
use crate::client::http::paths;
use crate::client::scout::{self, Summary};
use crate::client::text::{Identity, ScoutPlayer};
use crate::pages;
use crate::LolPlugin;

/// 卡片上那句预置的话（ADR 050 第 5 条）：分档、挑句子，句子在 messages 里。
pub(crate) mod cheer;
/// 今天的状态（进房间或排队时推）：只用我自己的战绩算，纯函数和页面数据在这里。
pub(crate) mod today;

/// 进加载多久还没查完也发开局战绩。
const OPENING_WAIT: Duration = Duration::from_secs(20);
/// 结算后最多等多久这局的详情。
const SUMMARY_WAIT: Duration = Duration::from_secs(60);
const SUMMARY_RETRY: Duration = Duration::from_secs(5);
/// 比这还短的一局算重开局（提前投降的“重开”在 3 分多钟）。
const REMAKE_SECONDS: i64 = 240;

pub(crate) const TITLE_OPENING: &str = "开局战绩";
pub(crate) const TITLE_SUMMARY: &str = "这局总结";
/// 清单 `[[pushes]]` 里这两条推送的 id。
pub(crate) const PUSH_OPENING: &str = "opening";
pub(crate) const PUSH_SUMMARY: &str = "summary";

/// 发过哪局的哪种卡片、这局什么时候进的加载。客户端重连时跟着 `State` 一起清掉（宿主那边还按 key 去重）。
#[derive(Debug, Default)]
pub(crate) struct CardState {
    /// （对局 id，进加载的时刻）。
    loading: Option<(i64, Instant)>,
    /// 开局战绩已经发过（或这局不发）的对局。
    opened: HashSet<i64>,
    /// 这局总结已经排上（或这局不发）的对局。
    summarized: HashSet<i64>,
    /// 最近一局打完的（不算自定义、训练、观战）：回到房间推今天的状态时，等它出现在战绩里再算。
    ended: Option<i64>,
    /// 上一局这局总结挑的那句预置的话（档，第几句）：同一档连着两局时不挑同一句（ADR 050 第 5 条）。不落盘。
    last_cheer: Option<(cheer::Tier, usize)>,
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

/// 开局战绩给琴音的那句（不超过 [`Push::NOTE_MAX`] 字）：只写事实（ADR 043 D2：插件说“是什么”，“意味着什么”归她）——
/// 两边谁连胜连败（3 局以上）、这个英雄最近没玩过 / 常玩（自己选英雄的模式）、谁和谁开黑、隐藏了身份的几个人、还在查几个人。
/// 放不下时先省掉不要紧的（我方的、开黑），句子不截断。
pub(crate) fn opening_note(view: &MatchView) -> String {
    let summaries = summaries(view);
    let facts = |ally: bool| -> Vec<String> {
        view.players
            .iter()
            .enumerate()
            .filter(|(_, p)| p.ally == ally)
            .filter_map(|(i, p)| {
                let s = summaries[i].as_ref().filter(|s| s.games > 0)?;
                let words: Vec<String> = scout::tagged(s, view.random)
                    .into_iter()
                    .map(|(kind, text)| match kind {
                        "main" => format!("这个英雄 {} 局", s.champ_games),
                        _ => text,
                    })
                    .collect();
                (!words.is_empty()).then(|| format!("{} {}", p.short, words.join("、")))
            })
            .take(3)
            .collect()
    };
    let side = |name: &str, list: Vec<String>| {
        (!list.is_empty()).then(|| format!("{name}：{}。", list.join("，")))
    };
    let parties = crate::client::text::parties(&view.players);
    let hidden: Vec<String> = [("我方", true), ("对面", false)]
        .into_iter()
        .filter_map(|(name, ally)| {
            let n = view
                .players
                .iter()
                .filter(|p| p.ally == ally && p.identity == Identity::Hidden)
                .count();
            (n > 0).then(|| format!("{name} {n} 人"))
        })
        .collect();
    let waiting = view.players.iter().filter(|p| p.pending()).count();
    // （要紧的程度，这句）：按顺序写，放不下时从最不要紧的开始省
    let parts: Vec<(u8, Option<String>)> = vec![
        (0, Some(format!("{TITLE_OPENING}（{}）。", view.mode))),
        (1, side("对面", facts(false))),
        (4, side("我方", facts(true))),
        (
            3,
            (!parties.is_empty()).then(|| format!("开黑：{}。", parties.join("；"))),
        ),
        (
            2,
            (!hidden.is_empty()).then(|| format!("隐藏了身份：{}。", hidden.join("、"))),
        ),
        (0, (waiting > 0).then(|| format!("还在查 {waiting} 人。"))),
    ];
    let mut keep: Vec<(u8, String)> = parts
        .into_iter()
        .filter_map(|(rank, text)| text.map(|t| (rank, t)))
        .collect();
    let len = |k: &[(u8, String)]| k.iter().map(|(_, t)| t.chars().count()).sum::<usize>();
    while len(&keep) > Push::NOTE_MAX {
        let Some(worst) = keep
            .iter()
            .enumerate()
            .filter(|(_, (rank, _))| *rank > 0)
            .max_by_key(|(_, (rank, _))| *rank)
            .map(|(i, _)| i)
        else {
            break;
        };
        keep.remove(worst);
    }
    keep.into_iter().map(|(_, t)| t).collect()
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

/// 这局总结给琴音的那句：输赢、模式、时长；我用什么英雄、K/D/A、伤害在全队排第几；卡片上那句预置的话（ADR 050
/// 第 5 条：她知道卡片上说了什么，免得接着说重复的）。`page` 是单局详情的页面数据。
pub(crate) fn summary_note(page: &Value, me: &str, cheer: Option<&cheer::Cheer>) -> String {
    let seconds = page["durationS"].as_i64().unwrap_or(0);
    let length = format!("{} 分 {} 秒", seconds / 60, seconds % 60);
    let queue = page["queue"].as_str().unwrap_or("");
    let Some(g) = cheer::MyGame::of(page, me) else {
        return format!("{TITLE_SUMMARY}：这局结束了（{queue}，{length}）。");
    };
    let line = cheer
        .map(|c| format!("卡片上写了：“{}”。", c.text))
        .unwrap_or_default();
    format!(
        "{TITLE_SUMMARY}：这局{}了（{queue}，{length}）。你用{} {}/{}/{}，伤害全队{}。{line}",
        if g.win { "赢" } else { "输" },
        if g.champion.is_empty() {
            "英雄"
        } else {
            &g.champion
        },
        g.kills,
        g.deaths,
        g.assists,
        rank_word(g.damage_rank - 1),
    )
}

/// `lol.game_end` 的数据（ADR 050 第 6 条；清单 `[[events]]` 里登记的字段，camelCase）：只放事实。`streak` 是算上这局
/// 今天现在几连胜（正）/ 几连败（负），读不到今天的战绩时 None（两个字段都不给）；结算详情里没有五杀这一项时不给 `pentaKill`。
pub(crate) fn game_end_data(
    page: &Value,
    g: &cheer::MyGame,
    streak: Option<i64>,
) -> Map<String, Value> {
    let mut data = Map::new();
    data.insert("win".into(), json!(g.win));
    data.insert("queue".into(), json!(page["queue"].as_str().unwrap_or("")));
    data.insert(
        "durationSeconds".into(),
        json!(page["durationS"].as_i64().unwrap_or(0)),
    );
    data.insert("champion".into(), json!(g.champion));
    data.insert("kills".into(), json!(g.kills));
    data.insert("deaths".into(), json!(g.deaths));
    data.insert("assists".into(), json!(g.assists));
    data.insert("damageRank".into(), json!(g.damage_rank));
    if let Some(penta) = g.penta {
        data.insert("pentaKill".into(), json!(penta));
    }
    if let Some(streak) = streak {
        data.insert("todayWinStreak".into(), json!(streak.max(0)));
        data.insert("todayLossStreak".into(), json!((-streak).max(0)));
    }
    data
}

fn report(title: &str, result: Result<PushOutcome, String>) {
    match result {
        Ok(PushOutcome::Posted | PushOutcome::Updated) => {}
        Ok(outcome) => log::log(&format!(
            "lol: 推送「{title}」没发（{outcome:?}：关着或发过了）"
        )),
        Err(e) => log::log(&format!("lol: 推送「{title}」发不出去：{e}")),
    }
}

impl LolPlugin {
    /// 后台每一轮的最后：按阶段决定推不推开局战绩、这局总结（`previous` 是上一轮的客户端阶段）。
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
        // 进房间或排队（从别的阶段进来：打完一局回来、Kotone 刚连上客户端）：今天的状态
        if today::lobby_phase(phase) && !today::lobby_phase(previous) {
            self.today_card(host);
        }
    }

    fn opening_card(&self, host: &dyn Host, live_ready: bool) {
        let due = {
            let mut s = self.state();
            let Some((game_id, fetched)) = s.current.as_ref().map(|m| {
                // 只等查得到的人：隐藏了身份的、电脑不查（ADR 043 D9）
                let fetched = m.members.len() >= 2
                    && m.pending.is_empty()
                    && m.members
                        .iter()
                        .filter(|x| x.identity == Identity::Known)
                        .all(|x| m.histories.contains_key(&x.puuid));
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
        let page = pages::match_page_in(
            &view.players,
            &view.puuids,
            view.game_id,
            view.queue_id,
            &view.mode,
            view.random,
            data.as_deref(),
            &view.stage,
        );
        let result = host.push(Push {
            trigger: PUSH_OPENING.into(),
            key: format!("opening:{game_id}"),
            card: Some(PushCard {
                title: TITLE_OPENING.into(),
                uri: pages::PAGE_MATCH.into(),
                params: Value::Null,
                data: page,
            }),
            note: opening_note(&view),
            ..Push::default()
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
            s.cards.ended = Some(game_id);
            (game_id, lcu, me)
        };
        let data = self.loaded_data();
        let task_host = host.clone();
        let state = self.state.clone();
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
            let Some(mut page) = pages::game_page_for(&detail, data.as_deref(), Some(&me)) else {
                return;
            };
            // “这局”页面结算时的“刚结束”（调研 G29）：我那一队赢没赢、时长
            if let Some(mine) = page["teams"]
                .as_array()
                .and_then(|t| t.iter().find(|t| t["ally"] == json!(true)))
            {
                let result = pages::MatchResult {
                    win: mine["win"].as_bool().unwrap_or(false),
                    duration_s: page["durationS"].as_i64().unwrap_or(0),
                    game_id,
                };
                if let Some(m) = state
                    .lock()
                    .unwrap()
                    .current
                    .as_mut()
                    .filter(|m| m.game_id == game_id)
                {
                    m.result = Some(result);
                }
            }
            // 单局详情带“刚结束”的头部（设计文档第三节 3）
            page["justEnded"] = json!(true);
            // 卡片上那句预置的话（ADR 050 第 5 条）：按这局的事实分档，上一局挑的那句记在插件状态里
            let mine = cheer::MyGame::of(&page, &me);
            let line = mine.as_ref().and_then(|g| {
                let mut s = state.lock().unwrap();
                let picked = cheer::pick_game(cheer::game_tier(g), game_id, s.cards.last_cheer)?;
                s.cards.last_cheer = Some((picked.tier, picked.index));
                Some(picked)
            });
            if let Some(line) = &line {
                page["cheer"] = line.page();
            }
            let note = summary_note(&page, &me, line.as_ref());
            // lol.game_end 的“今天几连胜 / 连败”：读一次我的战绩（这一局多半还没进去，按这局的结果接上）
            let streak = match &mine {
                Some(g) => match lcu.get(&paths::matches(&me)).await {
                    Ok(list) => Some(today::streak_after(
                        &list,
                        game_id,
                        page["createdMs"].as_i64().unwrap_or(0),
                        g.win,
                        today::local_date,
                    )),
                    Err(error) => {
                        log::log(&format!(
                            "lol: 读不到自己的战绩，这局结算的事里没有今天的连胜连败：{error}"
                        ));
                        None
                    }
                },
                None => None,
            };
            let event = mine.as_ref().map(|g| game_end_data(&page, g, streak));
            let result = task_host.push(Push {
                trigger: PUSH_SUMMARY.into(),
                key: format!("summary:{game_id}"),
                card: Some(PushCard {
                    title: TITLE_SUMMARY.into(),
                    uri: pages::PAGE_GAME.into(),
                    params: json!({"gameId": page["gameId"]}),
                    data: page,
                }),
                note,
                ..Push::default()
            });
            report(TITLE_SUMMARY, result);
            // 推送关着也报（事件和推送是两回事）；先推后报：她被自动化叫起来时，卡片和给她的那句已经在对话里了
            match event {
                Some(data) => {
                    task_host.game_event(super::LOL, "lol.game_end", "lol/client", data, None)
                }
                None => log::log(&format!(
                    "lol: 这局（{game_id}）的结算详情里找不到自己，不报 lol.game_end"
                )),
            }
        }));
    }

    /// 进房间或排队：推（或原地更新）今天的状态。读我自己的战绩（和战绩页同一段）；刚打完一局时等它出现在战绩里（每 5 秒看一次，
    /// 最多 60 秒，到了照样用读到的发）。今天还没打过不发；推送关着时连战绩都不读。
    fn today_card(&self, host: &Arc<dyn Host>) {
        if !host.push_enabled(today::PUSH_TODAY) {
            return;
        }
        let (lcu, me, ended) = {
            let s = self.state();
            let Some(c) = s.client.as_ref() else { return };
            (c.lcu.clone(), c.me.clone(), s.cards.ended)
        };
        let task_host = host.clone();
        let state = self.state.clone();
        host.spawn(Box::pin(async move {
            let me = match me {
                Some(me) => me,
                None => {
                    let read = lcu.get(paths::ME).await.ok();
                    let Some(me) = read
                        .as_ref()
                        .and_then(|v| v["puuid"].as_str())
                        .filter(|p| !p.is_empty())
                        .map(str::to_string)
                    else {
                        log::log("lol: 读不到自己是谁，不发今天的状态");
                        return;
                    };
                    if let Some(c) = state.lock().unwrap().client.as_mut() {
                        c.me = Some(me.clone());
                    }
                    me
                }
            };
            let deadline = Instant::now() + SUMMARY_WAIT;
            let list = loop {
                let late = Instant::now() >= deadline;
                match lcu.get(&paths::matches(&me)).await {
                    // 刚打完的那一局已经在战绩里了（或者这次没有刚打完的）
                    Ok(list)
                        if late
                            || !matches!(ended, Some(id) if !today::listed_ids(&list).contains(&id)) =>
                    {
                        break list
                    }
                    Err(error) if late => {
                        log::log(&format!("lol: 读不到自己的战绩，不发今天的状态：{error}"));
                        return;
                    }
                    _ => {}
                }
                tokio::time::sleep(SUMMARY_RETRY).await;
            };
            // 英雄的名字、模式的名字要客户端的资料（刚连上时还在后台读，最多等 10 秒，读不到就用 id）
            let mut data = state.lock().unwrap().data.clone();
            for _ in 0..10 {
                if data.is_some() {
                    break;
                }
                tokio::time::sleep(Duration::from_secs(1)).await;
                data = state.lock().unwrap().data.clone();
            }
            let date = today::local_date(crate::pushes::now_ms());
            let Some(push) = today_push(&list, &date, today::local_date, data.as_deref()) else {
                log::log("lol: 今天还没打过，不发今天的状态");
                return;
            };
            report(today::TITLE_TODAY, task_host.push(push));
        }));
    }
}

/// 今天的状态那条推送：`list` 是我的战绩列表（客户端的原样）。今天还没打过时 None。一天一个 key，
/// 同一天再推就是更新（内容一样宿主什么都不做）；更新时让悬浮窗再弹一次（它上面已经有这一局的两张卡片了）。
/// 不给琴音留话（见模块说明）。
pub(crate) fn today_push(
    list: &Value,
    date: &str,
    date_of: impl Fn(i64) -> String,
    data: Option<&crate::client::GameData>,
) -> Option<Push> {
    let games = history::parse_match_list(list);
    let todays = today::today_games(&games, date, date_of);
    if todays.is_empty() {
        return None;
    }
    // 客户端给的这几局全是今天的：今天可能还有更早的
    let listed = today::listed_ids(list).len();
    let capped = (todays.len() == games.len() && listed >= today::CAPPED_FROM).then_some(listed);
    Some(Push {
        trigger: today::PUSH_TODAY.into(),
        key: format!("today:{date}"),
        card: Some(PushCard {
            title: today::TITLE_TODAY.into(),
            uri: pages::PAGE_TODAY.into(),
            params: Value::Null,
            data: today::today_page(date, &todays, capped, data),
        }),
        alert: true,
        ..Push::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ADR 043 D4：推的三条都在清单 `[[pushes]]` 里（宿主不收没声明的），样子是卡片，要“读客户端数据”，页面是清单里的。
    #[test]
    fn the_card_pushes_are_declared_in_the_manifest() {
        let m = crate::manifest();
        for (id, title) in [
            (PUSH_OPENING, TITLE_OPENING),
            (PUSH_SUMMARY, TITLE_SUMMARY),
            (today::PUSH_TODAY, today::TITLE_TODAY),
        ] {
            let spec = m.push_spec(id).expect(id);
            assert_eq!(spec.title, title);
            assert_eq!(spec.forms, [kotone_plugin_api::PushForm::Card]);
            assert_eq!(spec.needs, ["client"]);
            assert!(spec.enabled_by_default);
        }
        for page in [pages::PAGE_MATCH, pages::PAGE_GAME, pages::PAGE_TODAY] {
            assert!(
                m.page(page).is_some_and(|p| p.needs == ["client"]),
                "{page}"
            );
        }
    }

    /// 战绩列表（客户端的原样，新的在前）：`(id, 开局时间, 英雄, 赢没赢)`；每局 20 分钟。
    fn match_list(games: &[(i64, i64, i64, bool)]) -> Value {
        json!({"games": {"games": games.iter().map(|(id, at, champion, win)| json!({
            "gameId": id, "queueId": 2400, "gameMode": "KIWI", "gameType": "MATCHED_GAME",
            "gameCreation": at, "gameDuration": 1200,
            "participants": [{"championId": champion, "stats": {"win": win, "kills": 4, "deaths": 2, "assists": 9}}],
        })).collect::<Vec<_>>()}})
    }

    /// 日期按 UTC（测试不依赖本机时区；分界的细节见 `today::tests`）。
    fn utc(ms: i64) -> String {
        if ms >= 86_400_000 {
            "2026-11-26".into()
        } else {
            "2026-11-25".into()
        }
    }

    /// 今天的状态：一天一个 key、卡片、不给她留话、更新时再弹；同样的战绩推出来一模一样（宿主当成“没变”），
    /// 多打一局内容才变、key 不变（原地更新）；今天还没打过不推；客户端给的 20 局以上全是今天的时标出来。
    #[test]
    fn the_today_push_is_one_card_per_day_updated_in_place() {
        let day = 86_400_000;
        let list = match_list(&[
            (3, day + 7_200_000, 157, false),
            (2, day + 3_600_000, 157, false),
            (1, day - 3_600_000, 103, true),
        ]);
        let push = today_push(&list, "2026-11-26", utc, None).expect("今天打过");
        assert_eq!(push.trigger, "today");
        assert_eq!(push.key, "today:2026-11-26");
        assert!(push.note.is_empty(), "不给她留话");
        assert!(push.alert && push.line.is_none() && push.voice.is_empty());
        push.check(crate::manifest()).unwrap();
        let card = push.card.as_ref().unwrap();
        assert_eq!(card.uri, pages::PAGE_TODAY);
        assert_eq!(card.title, "今天的状态");
        assert_eq!(card.data["games"], 2, "昨天那一局不算");
        assert_eq!(card.data["streak"], -2);
        assert_eq!(card.data["lostOften"][0]["champion"]["id"], 157);
        assert_eq!(card.data["capped"], false);
        // 同样的战绩：一模一样
        assert_eq!(
            today_push(&list, "2026-11-26", utc, None).unwrap().card,
            push.card
        );
        // 又打了一局：同一个 key，内容变了
        let more = match_list(&[
            (4, day + 9_000_000, 103, true),
            (3, day + 7_200_000, 157, false),
            (2, day + 3_600_000, 157, false),
        ]);
        let next = today_push(&more, "2026-11-26", utc, None).unwrap();
        assert_eq!(next.key, push.key);
        assert_ne!(next.card, push.card);
        assert_eq!(next.card.as_ref().unwrap().data["streak"], 1);
        // 第二天还没打：不推
        assert!(today_push(&list, "2026-11-27", utc, None).is_none());
        assert!(today_push(&json!({}), "2026-11-26", utc, None).is_none());
        // 客户端给的 20 局全是今天的：只看了最近 20 局
        let busy: Vec<(i64, i64, i64, bool)> = (0..20)
            .map(|i| (100 - i, day + 3_000_000 - i * 60_000, 22, i % 2 == 0))
            .collect();
        let capped = today_push(&match_list(&busy), "2026-11-26", utc, None).unwrap();
        let data = &capped.card.as_ref().unwrap().data;
        assert_eq!(
            (data["capped"].as_bool(), data["limit"].as_u64()),
            (Some(true), Some(20))
        );
    }
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
            identity: Identity::Known,
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
            stage: pages::MatchStage::default(),
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
    fn the_opening_note_states_streaks_parties_hidden_players_and_who_is_still_loading() {
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
        // 隐藏了身份（ADR 043 D9）：不查，也不算还在查
        players[8].history = None;
        players[8].identity = Identity::Hidden;
        let note = opening_note(&view(players));
        assert!(note.starts_with("开局战绩（海克斯大乱斗）。"), "{note}");
        // ADR 043 D2：只说事实，不说谁是突破口
        assert!(note.contains("对面：寒冰 5 连败"), "{note}");
        assert!(!note.contains("突破口"), "{note}");
        assert!(note.contains("开黑：我方 伊泽瑞尔 + 锤石 + 璐璐"), "{note}");
        assert!(note.contains("隐藏了身份：对面 1 人"), "{note}");
        assert!(note.contains("还在查 1 人"), "{note}");
        assert!(!note.contains("的玩家"), "不写玩家名：{note}");
        assert!(note.chars().count() <= Push::NOTE_MAX, "{note}");
    }

    /// 十个人都有事可说时，那句话照样不超过 160 字，先省掉我方的、开黑；对面和“还在查”留着。
    #[test]
    fn a_crowded_opening_note_drops_the_least_important_sentences() {
        let streak = [false, false, false, false, false];
        let players: Vec<ScoutPlayer> = (0..10)
            .map(|i| {
                let mut p = player(
                    &format!("名字很长的英雄{i}"),
                    i < 5,
                    i == 0,
                    Some(i64::from(i < 5)),
                    &streak,
                );
                if i == 9 {
                    p.history = None;
                }
                p
            })
            .collect();
        let note = opening_note(&view(players));
        assert!(note.chars().count() <= Push::NOTE_MAX, "{note}");
        assert!(note.contains("对面："), "{note}");
        assert!(note.ends_with("还在查 1 人。"), "{note}");
        assert!(!note.contains("我方："), "先省掉我方的：{note}");
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
            summary_note(&page(50_000, true), "me", None),
            "这局总结：这局赢了（海克斯大乱斗，18 分 32 秒）。你用寒冰 12/3/20，伤害全队第一。"
        );
        let lost = summary_note(&page(10_000, false), "me", None);
        assert!(
            lost.contains("这局输了") && lost.contains("伤害全队第二"),
            "{lost}"
        );
        // 找不到自己（录的详情不是这一局）：只说结束了
        assert_eq!(
            summary_note(&page(1, true), "someone", None),
            "这局总结：这局结束了（海克斯大乱斗，18 分 32 秒）。"
        );
    }

    /// ADR 050 第 5 条：这局总结的那句预置的话放进页面数据（页面按语言取）、给她的那句里也写上；
    /// 赢了、伤害第一、KDA 5 以上是“C麻了”那一档。说明照样不超过 160 字。
    #[test]
    fn the_summary_carries_the_preset_line_on_the_card_and_in_her_note() {
        let won = page(50_000, true);
        let g = cheer::MyGame::of(&won, "me").unwrap();
        assert_eq!(cheer::game_tier(&g), cheer::Tier::Carry);
        let line = cheer::pick_game(cheer::game_tier(&g), 42, None).unwrap();
        let note = summary_note(&won, "me", Some(&line));
        assert!(
            note.ends_with(&format!("伤害全队第一。卡片上写了：“{}”。", line.text)),
            "{note}"
        );
        assert!(note.chars().count() <= Push::NOTE_MAX, "{note}");
        assert_eq!(line.page()["tier"], "carry");
        // 输了、伤害第二：“输了”那一档
        let lost = page(10_000, false);
        let g = cheer::MyGame::of(&lost, "me").unwrap();
        assert_eq!(cheer::game_tier(&g), cheer::Tier::Loss);
    }

    /// ADR 050 第 6 条：`lol.game_end` 只放事实，字段都在清单里登记过；读不到今天的战绩时不给连胜连败，
    /// 详情里没有五杀这一项时不给 `pentaKill`。
    #[test]
    fn the_game_end_event_states_the_facts_registered_in_the_manifest() {
        let mut won = page(50_000, true);
        won["teams"][0]["players"][0]["pentaKills"] = json!(1);
        let g = cheer::MyGame::of(&won, "me").unwrap();
        let data = game_end_data(&won, &g, Some(-3));
        assert_eq!(
            Value::Object(data.clone()),
            json!({"win": true, "queue": "海克斯大乱斗", "durationSeconds": 1112, "champion": "寒冰",
                   "kills": 12, "deaths": 3, "assists": 20, "damageRank": 1, "pentaKill": true,
                   "todayWinStreak": 0, "todayLossStreak": 3})
        );
        let spec = crate::manifest()
            .events
            .iter()
            .find(|e| e.kind == "lol.game_end")
            .expect("lol.game_end 登记了");
        assert_eq!(spec.needs, ["client"]);
        for field in data.keys() {
            assert!(spec.data.contains_key(field), "data.{field} 没登记");
        }
        for field in spec.data.keys() {
            assert!(data.contains_key(field), "登记了 data.{field} 却不报");
        }
        // 读不到今天的战绩、详情里没有五杀
        let plain = page(10_000, false);
        let g = cheer::MyGame::of(&plain, "me").unwrap();
        let data = game_end_data(&plain, &g, None);
        assert_eq!(data["win"], false);
        assert_eq!(data["damageRank"], 2);
        for missing in ["pentaKill", "todayWinStreak", "todayLossStreak"] {
            assert!(!data.contains_key(missing), "{missing}");
        }
        assert_eq!(
            game_end_data(&plain, &g, Some(2))["todayWinStreak"],
            2,
            "连胜时连败是 0"
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
