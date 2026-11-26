//! 选英雄卡片（ADR 043 D7，原来贴在客户端右上角的选英雄覆盖层）：清单 `[[pushes]]` 的 `champ-select`，页面
//! `ui://kotone/lol/champ-select`（插件的 MCP App，聊天里和悬浮窗的小窗里都是它）。
//!
//! - 海斗照 AramHex 的选英雄标注：每个英雄一个评级（S–D）、胜率、第几 / 共几；备选席按胜率排，比你现在的英雄高的标出来，
//!   最高的那个圈出来（值得换）。峡谷：每个人这个位置的强度和胜率。数据是腾讯 101（掌盟）的。
//! - 一次选英雄一个 key（这局的对局 id，没有时用进选英雄的时刻）：每一轮（2 秒）重读选人接口，内容变了就再推一次，
//!   宿主原地更新（重随、和备选席换英雄以后跟着变）；一样的不推。一项数据都没有时不推（不发一张全是“—”的卡片）。
//! - 选英雄结束时再推一次，卡片上标“结束了”；进了游戏（不是被秒退）才带一句给她的说明（只有事实：你和队友用什么英雄、
//!   胜率或强度）。中间的变化只更新卡片，不给她说明（ADR 043 D3）。
//!
//! B 轮（讨论记录第二节“选人（排位）”“选人（海斗 / 大乱斗）”）：卡片随着选人的进度长出几块，每块一个来源标签
//! （ADR 043 D8：一块只用一个来源），都是固定规则（[`super::draft`]、[`super::comp`]、[`super::import`]）：
//!
//! - 峡谷（排位、匹配）：预选后、禁用结束前——**可以禁的**（101）；我方有 3 个人选了英雄以后——**阵容**的数（客户端的
//!   英雄资料）；轮到你选——**你的英雄池**对上对面已锁的英雄（你自己的战绩 + 101）；锁定以后——**对线**（OP.GG 的对线
//!   指南，对手是猜的、写明）；有英雄（预选也算）就有**我的英雄**：符文、出装、加点、召唤师技能（101）和“一键导入”。
//! - 海斗：原来的评级和备选席照旧；**我的英雄**是 aramhex-data 的最好的海克斯、核心三件、召唤师技能和“一键导入”（没有符文）。
//! - 每块的数据一次选人里只取一次（取不到的 15 秒后再试），不每一轮都去网上要；OP.GG 照答应的：按需、最多 2 个并发、
//!   存 24 小时（[`crate::stats::Stats::opgg`]）。

use std::collections::HashMap;
use std::time::{Duration, Instant};

use kotone_core::log;
use kotone_plugin_api::{Host, Push, PushCard, PushOutcome};
use serde_json::{json, Value};

use super::{
    comp, draft, rules, MAYHEM_QUEUE, PAGE_CHAMP_SELECT, PUSH_CHAMP_SELECT, SOURCE_CLIENT_DATA,
    SOURCE_OPGG, SOURCE_QQ101,
};
use crate::client::assets;
use crate::client::history::{self, GameRecord};
use crate::client::http::paths;
use crate::client::Lcu;
use crate::stats::{opgg, qq101};
use crate::{LolPlugin, SOURCE_CONTROL, SOURCE_OPGG as OPGG_SWITCH};

/// 卡片的名字（聊天里“英雄联盟 推送 · 选英雄”）。
pub(crate) const TITLE: &str = "选英雄";

/// 正在进行的这一次选英雄。
#[derive(Debug, Default)]
pub(crate) struct CardState {
    run: Option<Run>,
}

#[derive(Debug)]
struct Run {
    key: String,
    /// 上次推出去的卡片数据（一样就不再推）。
    last: Option<Value>,
    /// 这一次选人里各块取过的数据（[`MEMO_RETRY`]）：键是块和它的输入（英雄、分路、对面的英雄……）。
    memo: HashMap<String, (Instant, Value)>,
    /// 我自己最近的对局（英雄池用，一次选人取一次）。
    games: Option<Vec<GameRecord>>,
}

/// 取不到的块（结果是 null）多久以后再试；取到的这一次选人里一直用。
const MEMO_RETRY: Duration = Duration::from_secs(15);
/// 英雄池看自己最近多少局。
const POOL_GAMES: u32 = 40;

/// 选人接口里我方的人（英雄 id、分路、是不是我；还没选的不算）和备选席。
pub fn seats(session: &Value) -> (Vec<(i64, String, bool)>, Vec<i64>) {
    let me = session["localPlayerCellId"].as_i64();
    let team = session["myTeam"]
        .as_array()
        .map(|t| {
            t.iter()
                .map(|m| {
                    let id = m["championId"]
                        .as_i64()
                        .filter(|c| *c > 0)
                        .or_else(|| m["championPickIntent"].as_i64())
                        .unwrap_or(0);
                    (
                        id,
                        m["assignedPosition"].as_str().unwrap_or("").to_string(),
                        me.is_some() && m["cellId"].as_i64() == me,
                    )
                })
                .filter(|(id, _, _)| *id > 0)
                .collect()
        })
        .unwrap_or_default();
    let bench = session["benchChampions"]
        .as_array()
        .map(|b| {
            b.iter()
                .filter_map(|c| c["championId"].as_i64())
                .filter(|c| *c > 0)
                .collect()
        })
        .unwrap_or_default();
    (team, bench)
}

/// 这一次选英雄的 key：选人接口或对局流程里的对局 id（同一局重连、刷新都一样）；都没有时用进选英雄的时刻。
pub fn run_key(session: &Value, flow: &Value, now_ms: i64) -> String {
    let id = session["gameId"]
        .as_i64()
        .filter(|id| *id > 0)
        .or_else(|| flow["gameData"]["gameId"].as_i64().filter(|id| *id > 0));
    match id {
        Some(id) => format!("champ-select:{id}"),
        None => format!("champ-select:t{now_ms}"),
    }
}

/// 页面上的一个英雄：`{id, short, icon}`（图片照客户端的资源路径，页面自己取）。
pub fn champion(id: i64, short: String) -> Value {
    json!({"id": id, "short": short, "icon": assets::champion_icon(id)})
}

/// 海斗的选英雄卡片（照 AramHex 的选英雄标注）：每个英雄一个评级（只按胜率重排的名次换算 S–D）、胜率、第几 / 共几；
/// 备选席按胜率从高到低排，胜率比我现在的英雄高的 `better`，其中最高的 `best`（值得换）。我的英雄没有胜率时不比。
pub fn mayhem_panel(
    team: &[(i64, String, bool)],
    bench: &[i64],
    table: &[qq101::MayhemChampion],
    short: impl Fn(i64) -> String,
) -> Value {
    let (ranks, total) = rules::win_ranks(table.iter().map(|r| (r.champion_id, r.win_rate)));
    let win = |id: i64| {
        table
            .iter()
            .find(|r| r.champion_id == id)
            .and_then(|r| r.win_rate)
    };
    let brief = |id: i64| {
        let rank = ranks.get(&id).copied();
        json!({
            "champion": champion(id, short(id)),
            "winRate": win(id),
            "rank": rank,
            "grade": rank.and_then(|r| rules::rank_grade(r, total)),
        })
    };
    let mine = team
        .iter()
        .find(|(_, _, me)| *me)
        .and_then(|(id, _, _)| win(*id));
    let mut seats: Vec<(i64, Option<f64>)> = bench.iter().map(|id| (*id, win(*id))).collect();
    seats.sort_by(|a, b| b.1.unwrap_or(f64::MIN).total_cmp(&a.1.unwrap_or(f64::MIN)));
    let better = |w: Option<f64>| matches!((w, mine), (Some(w), Some(m)) if w > m);
    let best = seats.iter().find(|(_, w)| better(*w)).map(|(id, _)| *id);
    json!({
        "mode": "mayhem",
        "total": total,
        "team": team.iter().map(|(id, _, me)| {
            let mut row = brief(*id);
            row["me"] = json!(me);
            row
        }).collect::<Vec<_>>(),
        "bench": seats.iter().map(|(id, w)| {
            let mut row = brief(*id);
            row["better"] = json!(better(*w));
            row["best"] = json!(best == Some(*id));
            row
        }).collect::<Vec<_>>(),
        "source": SOURCE_QQ101,
        "ended": false,
    })
}

/// 选人接口的分路 → 101 的分路代码（大乱斗、海斗里没有分路）。
pub fn lane_code(position: &str) -> Option<&'static str> {
    match position {
        "top" => Some("TOP"),
        "jungle" => Some("JUNGLE"),
        "middle" => Some("MIDDLE"),
        "bottom" => Some("BOTTOM"),
        "utility" => Some("SUPPORT"),
        _ => None,
    }
}

/// 峡谷的选英雄卡片：每个人这个位置的强度和胜率（`rows` 和 `team` 一一对应：这个英雄在这一路的 101 强度榜那一行）。
pub fn rift_panel(
    team: &[(i64, String, bool)],
    rows: &[Option<qq101::TierRow>],
    short: impl Fn(i64) -> String,
) -> Value {
    let team: Vec<Value> = team
        .iter()
        .zip(rows.iter().chain(std::iter::repeat(&None)))
        .map(|((id, position, me), row)| {
            json!({
                "champion": champion(*id, short(*id)),
                "me": me,
                "lane": lane_code(position).map(qq101::lane_name),
                "tier": row.as_ref().map(|r| r.tier.clone()),
                "winRate": row.as_ref().and_then(|r| r.win_rate),
            })
        })
        .collect();
    json!({"mode": "rift", "team": team, "bench": [], "source": SOURCE_QQ101, "ended": false})
}

/// B 轮加的几块（没有的是 null）。
pub const BLOCKS: [&str; 5] = ["bans", "comp", "pool", "matchup", "mine"];

/// 卡片里有没有一项数据（胜率、强度，或者 B 轮的一块）：一项都没有时不推。
pub fn has_data(panel: &Value) -> bool {
    let rows = |key: &str| panel[key].as_array().cloned().unwrap_or_default();
    rows("team")
        .iter()
        .chain(&rows("bench"))
        .any(|r| !r["winRate"].is_null() || !r["tier"].is_null())
        || BLOCKS.iter().any(|b| !panel[*b].is_null())
}

fn percent(rate: &Value) -> Option<String> {
    rate.as_f64().map(|r| format!("{:.1}%", r * 100.0))
}

/// 选英雄结束时给她的那句（只有事实，一句）：你用什么英雄（海斗：胜率、评级；峡谷：分路、强度），队友用什么。
pub fn ended_note(panel: &Value) -> String {
    let team = panel["team"].as_array().cloned().unwrap_or_default();
    let name = |row: &Value| row["champion"]["short"].as_str().unwrap_or("").to_string();
    let mayhem = panel["mode"] == "mayhem";
    let mine = team.iter().find(|r| r["me"] == true).map(|r| {
        let mut facts = Vec::new();
        if mayhem {
            if let Some(rate) = percent(&r["winRate"]) {
                facts.push(format!("海斗胜率 {rate}"));
            }
            if let Some(grade) = r["grade"].as_str() {
                facts.push(format!("评级 {grade}"));
            }
        } else {
            if let Some(lane) = r["lane"].as_str() {
                facts.push(lane.to_string());
            }
            if let Some(tier) = r["tier"].as_str() {
                facts.push(format!("强度 {tier}"));
            }
        }
        if facts.is_empty() {
            name(r)
        } else {
            format!("{}（{}）", name(r), facts.join("，"))
        }
    });
    let mates: Vec<String> = team
        .iter()
        .filter(|r| r["me"] != true)
        .map(name)
        .filter(|n| !n.is_empty())
        .collect();
    let mut out = format!(
        "{TITLE}结束了（{}）：",
        if mayhem { "海斗" } else { "峡谷" }
    );
    match mine {
        Some(mine) => {
            out.push_str(&format!("你用{mine}"));
            if !mates.is_empty() {
                out.push_str(&format!("，队友 {}", mates.join("、")));
            }
        }
        None => out.push_str(&format!("我方 {}", mates.join("、"))),
    }
    out.push('。');
    out
}

impl CardState {
    /// 这一轮在选英雄：这一次选英雄的 key（第一次看到时记下）。
    fn key(&mut self, make: impl FnOnce() -> String) -> String {
        self.run
            .get_or_insert_with(|| Run {
                key: make(),
                last: None,
                memo: HashMap::new(),
                games: None,
            })
            .key
            .clone()
    }

    /// 这一块这一次选人里取过没有：取到的给它；取不到（null）的 [`MEMO_RETRY`] 内也给 null（不再去要）。
    fn memo(&self, key: &str) -> Option<Value> {
        let (at, value) = self.run.as_ref()?.memo.get(key)?;
        (!value.is_null() || at.elapsed() < MEMO_RETRY).then(|| value.clone())
    }

    fn remember(&mut self, key: String, value: Value) {
        if let Some(run) = self.run.as_mut() {
            run.memo.insert(key, (Instant::now(), value));
        }
    }

    /// 这份卡片数据要不要推（和上次推出去的不一样）。
    fn changed(&self, panel: &Value) -> bool {
        self.run
            .as_ref()
            .is_some_and(|r| r.last.as_ref() != Some(panel))
    }

    fn pushed(&mut self, panel: Value) {
        if let Some(run) = self.run.as_mut() {
            run.last = Some(panel);
        }
    }

    /// 选英雄结束了：这一次的 key 和最后推出去的卡片（标上结束了）；没推过时 None。之后清掉。
    fn end(&mut self) -> Option<(String, Value)> {
        let run = self.run.take()?;
        let mut panel = run.last?;
        panel["ended"] = json!(true);
        Some((run.key, panel))
    }
}

fn report(result: Result<PushOutcome, String>) -> bool {
    match result {
        Ok(PushOutcome::Posted | PushOutcome::Updated | PushOutcome::Unchanged) => true,
        Ok(PushOutcome::Off) => false,
        Err(e) => {
            log::log(&format!("lol: 推送「{TITLE}」发不出去：{e}"));
            false
        }
    }
}

fn card_push(key: String, panel: Value, note: String) -> Push {
    Push {
        trigger: PUSH_CHAMP_SELECT.into(),
        key,
        card: Some(PushCard {
            title: TITLE.into(),
            uri: PAGE_CHAMP_SELECT.into(),
            params: Value::Null,
            data: panel,
        }),
        note,
        ..Push::default()
    }
}

impl LolPlugin {
    /// 每一轮：选英雄时推 / 更新卡片；离开选英雄时推最后一次（结束了）。
    pub(super) async fn champ_select_card(&self, host: &dyn Host, lcu: &Lcu, phase: &str) {
        if phase != "ChampSelect" {
            let ended = self.pushes.lock().unwrap().champ_select.end();
            if let Some((key, panel)) = ended {
                // 进了游戏才给她说明；被秒退、回到房间的只把卡片标成结束了
                let note = if crate::client::in_game_phase(phase) {
                    ended_note(&panel)
                } else {
                    String::new()
                };
                report(host.push(card_push(key, panel, note)));
            }
            return;
        }
        if !host.push_enabled(PUSH_CHAMP_SELECT) && !host.push_enabled(super::PUSH_AUGMENT_PICK) {
            return;
        }
        let Ok(session) = lcu.get(paths::CHAMP_SELECT).await else {
            return;
        };
        let flow = lcu.get(paths::SESSION).await.unwrap_or(Value::Null);
        let queue = flow["gameData"]["queue"]["id"].as_i64().unwrap_or(0);
        if queue == MAYHEM_QUEUE {
            // 海斗：三选一要本地文字识别，一个都没装时趁选英雄问一句要不要下载（进了游戏就来不及点了）
            self.ask_for_local_ocr(host);
        }
        if !host.push_enabled(PUSH_CHAMP_SELECT) {
            return;
        }
        let key = self
            .pushes
            .lock()
            .unwrap()
            .champ_select
            .key(|| run_key(&session, &flow, super::now_ms()));
        let Some(panel) = self.champ_select_panel(host, lcu, &session, queue).await else {
            return;
        };
        if !self.pushes.lock().unwrap().champ_select.changed(&panel) {
            return;
        }
        if report(host.push(card_push(key, panel.clone(), String::new()))) {
            self.pushes.lock().unwrap().champ_select.pushed(panel);
        }
    }

    /// 选英雄卡片的数据；还没人选英雄、一项数据都没有时 None。
    async fn champ_select_panel(
        &self,
        host: &dyn Host,
        lcu: &Lcu,
        session: &Value,
        queue: i64,
    ) -> Option<Value> {
        let (team, bench) = seats(session);
        if team.is_empty() && bench.is_empty() {
            return None;
        }
        let stats = self.stats_for(host);
        let names = stats.nicknames().await.ok();
        let data = self.loaded_data();
        let short = |id: i64| {
            names
                .as_ref()
                .map(|n| n.short(id))
                .or_else(|| {
                    data.as_deref()
                        .and_then(|d| d.champion(id))
                        .map(|c| c.short().to_string())
                })
                .unwrap_or_else(|| format!("英雄 {id}"))
        };
        let panel = if queue == MAYHEM_QUEUE {
            let days: Vec<String> = (0..7).map(crate::stats_tools::beijing_date).collect();
            let table = stats
                .mayhem_champions(&days)
                .await
                .map(|t| t.value.1)
                .unwrap_or_default();
            mayhem_panel(&team, &bench, &table, short)
        } else {
            let mut rows = Vec::new();
            for (id, position, _) in &team {
                let row = match lane_code(position) {
                    Some(lane) => stats
                        .tier_list(lane)
                        .await
                        .ok()
                        .and_then(|t| t.value.1.into_iter().find(|r| r.champion_id == *id)),
                    None => None,
                };
                rows.push(row);
            }
            rift_panel(&team, &rows, short)
        };
        let mut panel = panel;
        for block in BLOCKS {
            panel[block] = Value::Null;
        }
        if queue == MAYHEM_QUEUE {
            panel["mine"] = self.memo_mine(host, session, queue).await;
        } else if draft::RIFT_QUEUES.contains(&queue) {
            self.rift_blocks(host, lcu, session, queue, &mut panel, &short)
                .await;
        }
        // 一键导入的按钮：“操作客户端”关着时按钮上写为什么，带玩家去设置（开关一变卡片跟着变）
        panel["control"] = json!(host.source_enabled(SOURCE_CONTROL));
        has_data(&panel).then_some(panel)
    }

    /// 取过就用取过的（这一次选人里），不然现取一次记下。
    async fn memoized<F>(&self, key: String, fetch: F) -> Value
    where
        F: std::future::Future<Output = Value>,
    {
        if let Some(v) = self.pushes.lock().unwrap().champ_select.memo(&key) {
            return v;
        }
        let v = fetch.await;
        self.pushes
            .lock()
            .unwrap()
            .champ_select
            .remember(key, v.clone());
        v
    }

    /// “我的英雄”那块（[`super::import`]）：英雄、分路、锁没锁变了才重算。
    async fn memo_mine(&self, host: &dyn Host, session: &Value, queue: i64) -> Value {
        let d = draft::draft(session);
        let Some(mine) = d.mine else {
            return Value::Null;
        };
        let key = format!("mine:{queue}:{mine}:{}:{}", d.position, d.locked);
        self.memoized(key, async {
            self.my_loadout(host, session, queue)
                .await
                .map(|(_, block)| block)
                .unwrap_or(Value::Null)
        })
        .await
    }

    /// 峡谷的几块（见文件开头）。
    async fn rift_blocks(
        &self,
        host: &dyn Host,
        lcu: &Lcu,
        session: &Value,
        queue: i64,
        panel: &mut Value,
        short: &(dyn Fn(i64) -> String + Sync),
    ) {
        let d = draft::draft(session);
        let stage = draft::stage(session);
        panel["stage"] = json!(stage.id());
        let stats = self.stats_for(host);
        let lane = match lane_code(&d.position) {
            Some(lane) => Some(lane),
            None => match d.mine {
                Some(mine) => {
                    let key = format!("lane:{mine}");
                    self.memoized(key, async { json!(self.main_lane(host, mine).await) })
                        .await
                        .as_str()
                        .and_then(qq101::lane_code)
                }
                None => None,
            },
        };
        let lane_known = lane_code(&d.position).is_some();
        let rows: Vec<qq101::TierRow> = match lane {
            Some(lane) => {
                let v = self
                    .memoized(format!("rows:{lane}"), async {
                        stats
                            .tier_list(lane)
                            .await
                            .ok()
                            .map(|t| json!(t.value.1))
                            .unwrap_or(Value::Null)
                    })
                    .await;
                serde_json::from_value(v).unwrap_or_default()
            }
            None => Vec::new(),
        };
        let lane_name = lane.map(qq101::lane_name);
        let champ = |id: i64| champion(id, short(id));

        // 可以禁的：预选后、禁用结束前
        if matches!(stage, draft::Stage::Planning | draft::Stage::Banning) && lane.is_some() {
            let mut exclude = d.allies.clone();
            exclude.extend(&d.banned);
            let counters = match (d.mine, lane) {
                (Some(mine), Some(lane)) => {
                    let v = self
                        .memoized(format!("mu:{mine}:{lane}"), async {
                            stats
                                .matchups(mine, lane)
                                .await
                                .map(|(good, bad)| json!([good, bad]))
                                .unwrap_or(Value::Null)
                        })
                        .await;
                    serde_json::from_value::<(Vec<qq101::Matchup>, Vec<qq101::Matchup>)>(v)
                        .map(|(_, bad)| bad)
                        .unwrap_or_default()
                }
                _ => Vec::new(),
            };
            let list: Vec<Value> = draft::ban_candidates(&counters, &rows, &exclude)
                .into_iter()
                .map(|mut v| {
                    v["champion"] = champ(v["champion"].as_i64().unwrap_or(0));
                    v
                })
                .collect();
            if !list.is_empty() {
                panel["bans"] = json!({
                    "lane": lane_name,
                    "against": d.mine.map(champ),
                    "list": list,
                    "source": SOURCE_QQ101,
                });
            }
        }

        // 阵容：客户端的英雄资料（我方有 3 个人选了英雄以后；对面锁了 3 个以后）
        let mut ally = Vec::new();
        let mut enemy = Vec::new();
        for (list, out) in [(&d.allies, &mut ally), (&d.enemies, &mut enemy)] {
            for id in list.iter() {
                let v = self
                    .memoized(format!("traits:{id}"), async {
                        lcu.get(&crate::client::abilities::client_path(*id))
                            .await
                            .ok()
                            .and_then(|c| comp::traits(&c))
                            .map(|t| {
                                json!([
                                    t.damage == comp::Damage::Physical,
                                    t.damage == comp::Damage::Magic,
                                    t.ranged,
                                    t.frontline,
                                    t.strong_cc
                                ])
                            })
                            .unwrap_or(Value::Null)
                    })
                    .await;
                if let Some(t) = traits_of(&v) {
                    out.push(t);
                }
            }
        }
        let (ally, enemy) = (comp::side(&ally), comp::side(&enemy));
        if ally.is_some() || enemy.is_some() {
            panel["comp"] = json!({"ally": ally, "enemy": enemy, "source": SOURCE_CLIENT_DATA});
        }

        // 你的英雄池：轮到你选时
        if stage == draft::Stage::Picking {
            let games = self.my_games(lcu).await;
            let pool = draft::my_pool(&games);
            let mut matchups = HashMap::new();
            if let Some(lane) = lane {
                for (c, _, _) in &pool {
                    let v = self
                        .memoized(format!("mu:{c}:{lane}"), async {
                            stats
                                .matchups(*c, lane)
                                .await
                                .map(|(good, bad)| json!([good, bad]))
                                .unwrap_or(Value::Null)
                        })
                        .await;
                    if let Ok(pair) = serde_json::from_value(v) {
                        matchups.insert(*c, pair);
                    }
                }
            }
            let mut unavailable = d.banned.clone();
            unavailable.extend(d.enemies.iter());
            unavailable.extend(d.allies.iter().filter(|c| Some(**c) != d.mine));
            let list: Vec<Value> = draft::rank_pool(
                &pool,
                &matchups,
                &rows,
                lane_known,
                &d.enemies,
                &unavailable,
            )
            .into_iter()
            .map(|mut v| {
                v["champion"] = champ(v["champion"].as_i64().unwrap_or(0));
                for side in ["good", "bad"] {
                    if let Some(list) = v[side].as_array_mut() {
                        for m in list {
                            m["champion"] = champ(m["champion"].as_i64().unwrap_or(0));
                        }
                    }
                }
                v
            })
            .collect();
            if !list.is_empty() {
                panel["pool"] = json!({
                    "lane": lane_name,
                    "against": d.enemies.iter().map(|e| champ(*e)).collect::<Vec<_>>(),
                    "list": list,
                    "source": SOURCE_QQ101,
                });
            }
        }

        // 对线：锁定以后，OP.GG 的对线指南（OP.GG 开着时）
        if stage == draft::Stage::Locked && host.source_enabled(OPGG_SWITCH) {
            if let (Some(mine), Some(lane), Some(opponent)) =
                (d.mine, lane, draft::guess_opponent(&d.enemies, &rows))
            {
                let key = format!("matchup:{mine}:{opponent}:{lane}");
                let facts = self
                    .memoized(key, async {
                        self.lane_matchup(host, mine, opponent, lane).await
                    })
                    .await;
                if !facts.is_null() {
                    let mut block = facts;
                    block["me"] = champ(mine);
                    block["opponent"] = champ(opponent);
                    block["lane"] = json!(lane_name);
                    block["guessed"] = json!(true);
                    block["source"] = json!(SOURCE_OPGG);
                    block["url"] = json!(opgg::HOME);
                    panel["matchup"] = block;
                }
            }
        }

        panel["mine"] = self.memo_mine(host, session, queue).await;
    }

    /// 我自己最近的对局（一次选人取一次；取不到是空的）。
    async fn my_games(&self, lcu: &Lcu) -> Vec<GameRecord> {
        if let Some(games) = self
            .pushes
            .lock()
            .unwrap()
            .champ_select
            .run
            .as_ref()
            .and_then(|r| r.games.clone())
        {
            return games;
        }
        let puuid = lcu
            .get(paths::ME)
            .await
            .ok()
            .and_then(|me| me["puuid"].as_str().map(str::to_string))
            .unwrap_or_default();
        let games = if puuid.is_empty() {
            Vec::new()
        } else {
            lcu.get(&paths::matches(&puuid, POOL_GAMES))
                .await
                .map(|list| history::parse_match_list(&list))
                .unwrap_or_default()
        };
        if let Some(run) = self.pushes.lock().unwrap().champ_select.run.as_mut() {
            run.games = Some(games.clone());
        }
        games
    }

    /// OP.GG 的对线指南里的几条事实（英文 id 不知道、OP.GG 连不上、读不懂时 null）。
    async fn lane_matchup(&self, host: &dyn Host, mine: i64, opponent: i64, lane: &str) -> Value {
        let stats = self.stats_for(host);
        let names = stats.nicknames().await.ok();
        let data = self.loaded_data();
        let alias = |id: i64| {
            names
                .as_ref()
                .and_then(|n| n.aliases.get(&id).cloned())
                .or_else(|| {
                    data.as_deref()
                        .and_then(|d| d.champion(id).map(|c| c.alias.clone()))
                })
                .filter(|a| !a.is_empty())
        };
        let (Some(me), Some(them)) = (alias(mine), alias(opponent)) else {
            return Value::Null;
        };
        let args = opgg::lane_matchup_arguments(
            &opgg::champion_key(&me),
            &opgg::champion_key(&them),
            opgg::position(lane),
        );
        match stats
            .opgg(opgg::LANE_MATCHUP_TOOL, args, &host.app_version())
            .await
        {
            Ok(f) => opgg::parse_lane_matchup(&f.value)
                .filter(|m| !m.is_empty())
                .map(|m| json!(m))
                .unwrap_or(Value::Null),
            Err(e) => {
                log::log(&format!("lol: OP.GG 的对线指南查不到：{e}"));
                Value::Null
            }
        }
    }
}

/// 记下来的英雄资料 `[物理, 魔法, 远程, 前排, 强控]` → 几项。
fn traits_of(v: &Value) -> Option<comp::Traits> {
    let f: Vec<bool> = serde_json::from_value(v.clone()).ok()?;
    let [physical, magic, ranged, frontline, strong_cc] = f[..] else {
        return None;
    };
    Some(comp::Traits {
        damage: if physical {
            comp::Damage::Physical
        } else if magic {
            comp::Damage::Magic
        } else {
            comp::Damage::Mixed
        },
        ranged,
        frontline,
        strong_cc,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mayhem_row(id: i64, rank: i64, win: f64) -> qq101::MayhemChampion {
        qq101::MayhemChampion {
            champion_id: id,
            rank: Some(rank),
            rank_change: None,
            win_rate: Some(win),
            pick_rate: Some(0.01),
            partners: Vec::new(),
            worst_augments: Vec::new(),
        }
    }

    /// 20 个英雄，id 1..=20，胜率 55% 往下每个低 0.5 个点（id 1 最高）；腾讯的名次故意倒过来（掺了选用率）。
    fn mayhem_table() -> Vec<qq101::MayhemChampion> {
        (1..=20)
            .map(|id| mayhem_row(id, 21 - id, 0.55 - (id - 1) as f64 * 0.005))
            .collect()
    }

    /// 和 `scripts/manual-e2e/lol_tools_real.py` 的 `synth_champ_select` 一样的形状：我方 5 人 + 备选席。
    fn session(me: i64, team: &[i64], bench: &[i64]) -> Value {
        json!({
            "localPlayerCellId": 0,
            "myTeam": team.iter().enumerate().map(|(cell, id)| json!({
                "cellId": cell, "championId": if cell == 0 { me } else { *id },
                "championPickIntent": 0, "assignedPosition": ""
            })).collect::<Vec<_>>(),
            "theirTeam": [], "actions": [],
            "benchChampions": bench.iter().map(|id| json!({"championId": id})).collect::<Vec<_>>(),
            "isCustomGame": false,
        })
    }

    fn name(id: i64) -> String {
        format!("英雄{id}")
    }

    #[test]
    fn mayhem_card_grades_and_bench_comparison() {
        // 我是 8（第 8 名 → 8/20 = 40% → B），队友 2、15、19、20；备选 6、12、3、9
        let (team, bench) = seats(&session(8, &[8, 2, 15, 19, 20], &[6, 12, 3, 9]));
        assert_eq!(team.len(), 5);
        assert!(team[0].2 && !team[1].2);
        let panel = mayhem_panel(&team, &bench, &mayhem_table(), name);
        assert_eq!(panel["mode"], "mayhem");
        assert_eq!(panel["total"], 20);
        assert_eq!(panel["ended"], false);
        assert_eq!(panel["source"], SOURCE_QQ101);
        let grades: Vec<&str> = panel["team"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["grade"].as_str().unwrap())
            .collect();
        // 名次只按胜率（腾讯给的名次倒过来也不管）：2 → 10% S、8 → 40% B、15 → 75% C、19、20 → D
        assert_eq!(grades, ["B", "S", "C", "D", "D"]);
        assert_eq!(panel["team"][0]["me"], true);
        assert_eq!(panel["team"][0]["rank"], 8);
        // 英雄：名字 + id + 客户端的头像路径（页面自己取图）
        assert_eq!(
            panel["team"][0]["champion"],
            json!({"id": 8, "short": "英雄8", "icon": "/lol-game-data/assets/v1/champion-icons/8.png"})
        );
        // 备选按胜率从高到低；比我（8）高的 3、6 标出来，最高的 3 圈出来
        let bench = panel["bench"].as_array().unwrap();
        let order: Vec<&str> = bench
            .iter()
            .map(|r| r["champion"]["short"].as_str().unwrap())
            .collect();
        assert_eq!(order, ["英雄3", "英雄6", "英雄9", "英雄12"]);
        let flags: Vec<(bool, bool)> = bench
            .iter()
            .map(|r| (r["better"] == true, r["best"] == true))
            .collect();
        assert_eq!(
            flags,
            [(true, true), (true, false), (false, false), (false, false)]
        );
        assert_eq!(bench[0]["grade"], "A", "3/20 = 15%：不在前 10%");
        assert!(has_data(&panel));
    }

    /// 一次选英雄：重随以后内容变了要再推（同一个 key，宿主原地更新），一样的不推；结束时标上结束了、给她一句事实。
    #[test]
    fn one_champ_select_is_one_card_updated_in_place() {
        let table = mayhem_table();
        let mut state = CardState::default();
        let flow = json!({"gameData": {"gameId": 9100001, "queue": {"id": 2400}}});
        let key = state.key(|| run_key(&json!({}), &flow, 1));
        assert_eq!(key, "champ-select:9100001");
        let (team, bench) = seats(&session(8, &[8, 2, 15, 19, 20], &[6, 12]));
        let before = mayhem_panel(&team, &bench, &table, name);
        assert!(state.changed(&before), "第一次：推");
        state.pushed(before.clone());
        assert!(
            !state.changed(&mayhem_panel(&team, &bench, &table, name)),
            "一样的 session 两次：不推"
        );
        // 我重随：8 进了备选席，换成 1（全场最高）；备选里没有比我高的了
        let (team, bench) = seats(&session(1, &[1, 2, 15, 19, 20], &[6, 12, 8]));
        let after = mayhem_panel(&team, &bench, &table, name);
        assert!(state.changed(&after), "重随以后内容变了：再推");
        assert_eq!(before["bench"][0]["best"], true);
        assert_eq!(after["team"][0]["grade"], "S");
        assert!(after["bench"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["better"] == false && r["best"] == false));
        state.pushed(after.clone());
        // key 不变（同一次选英雄）
        assert_eq!(state.key(|| "别的".into()), "champ-select:9100001");
        // 结束：最后一份标上 ended，之后清掉
        let (end_key, ended) = state.end().unwrap();
        assert_eq!(end_key, "champ-select:9100001");
        assert_eq!(ended["ended"], true);
        assert_eq!(ended["team"], after["team"]);
        assert!(state.end().is_none());
        let note = ended_note(&ended);
        assert_eq!(
            note,
            "选英雄结束了（海斗）：你用英雄1（海斗胜率 55.0%，评级 S），队友 英雄2、英雄15、英雄19、英雄20。"
        );
        assert!(note.chars().count() <= Push::NOTE_MAX);
        // 推过以前就结束（一项数据都没有、还没推）：什么都不推
        let mut empty = CardState::default();
        empty.key(|| "champ-select:t5".into());
        assert!(empty.end().is_none());
    }

    #[test]
    fn the_key_is_the_game_or_the_moment_champ_select_started() {
        assert_eq!(
            run_key(&json!({"gameId": 77}), &Value::Null, 5),
            "champ-select:77"
        );
        assert_eq!(
            run_key(
                &json!({"gameId": 0}),
                &json!({"gameData": {"gameId": 0}}),
                5
            ),
            "champ-select:t5"
        );
    }

    #[test]
    fn rift_card_shows_lane_tiers_and_hides_without_data() {
        let session = json!({
            "localPlayerCellId": 1,
            "myTeam": [
                {"cellId": 0, "championId": 157, "assignedPosition": "middle"},
                {"cellId": 1, "championId": 0, "championPickIntent": 39, "assignedPosition": "top"},
                {"cellId": 2, "championId": 64, "assignedPosition": "jungle"},
                {"cellId": 3, "championId": 0, "championPickIntent": 0, "assignedPosition": "bottom"}
            ],
        });
        let (team, bench) = seats(&session);
        assert_eq!(team.len(), 3, "还没选的不算");
        assert!(bench.is_empty());
        assert!(team[1].2, "预选的也算我");
        let row = |id: i64, tier: &str, win: f64| qq101::TierRow {
            rank: None,
            champion_id: id,
            tier: tier.into(),
            lane: String::new(),
            win_rate: Some(win),
            pick_rate: None,
            ban_rate: None,
            counters: Vec::new(),
            rank_change: None,
        };
        let panel = rift_panel(
            &team,
            &[Some(row(157, "T1", 0.51)), Some(row(39, "T0", 0.53)), None],
            name,
        );
        assert_eq!(panel["mode"], "rift");
        assert_eq!(panel["team"][0]["lane"], "中路");
        assert_eq!(panel["team"][1]["tier"], "T0");
        assert_eq!(panel["team"][1]["me"], true);
        assert!(panel["team"][2]["tier"].is_null());
        assert!(has_data(&panel));
        assert_eq!(
            ended_note(&panel),
            "选英雄结束了（峡谷）：你用英雄39（上路，强度 T0），队友 英雄157、英雄64。"
        );
        // 海斗榜拿不到 / 峡谷里谁都没有分路：一项数据都没有，不推
        assert!(!has_data(&mayhem_panel(&team, &[], &[], name)));
        assert!(!has_data(&rift_panel(&team, &[], name)));
        // 没有 localPlayerCellId 时谁都不是“我”（以前 None == None 会把每个人都当成我）
        let (team, _) = seats(&json!({"myTeam": [{"championId": 5}, {"championId": 6}]}));
        assert!(team.iter().all(|(_, _, me)| !me));
    }

    /// 测试用的宿主：只有开关（“操作客户端”、OP.GG 可以关）。
    struct SwitchHost {
        control: bool,
        opgg: bool,
    }

    impl Host for SwitchHost {
        fn plugin(&self) -> &str {
            "lol"
        }
        fn enabled(&self) -> bool {
            true
        }
        fn source_enabled(&self, source: &str) -> bool {
            match source {
                SOURCE_CONTROL => self.control,
                OPGG_SWITCH => self.opgg,
                _ => true,
            }
        }
        fn settings(&self) -> Value {
            json!({})
        }
        fn data_dir(&self) -> std::path::PathBuf {
            std::env::temp_dir().join("kotone-lol-cs-test")
        }
        fn spawn(&self, _task: kotone_plugin_api::BoxFuture<'static, ()>) {}
        fn process_ids(&self, _exe: &str) -> Vec<u32> {
            Vec::new()
        }
        fn process_command_line(&self, _pid: u32) -> Option<String> {
            None
        }
        fn game_observe(&self, _game: &str, _observation: kotone_plugin_api::Observation) {}
        fn game_claim(&self, _game: &str, _claimed: bool) {}
        fn game_event(
            &self,
            _game: &str,
            _kind: &str,
            _source: &str,
            _data: serde_json::Map<String, Value>,
            _game_time: Option<f64>,
        ) {
        }
    }

    /// 一次排位选人里各块要的数据先记好（和取过一次一样），这样每一步都不连网、不连客户端（客户端的地址是连不上的端口）。
    fn prefilled() -> LolPlugin {
        use draft::tests::{game, matchup, row};
        let plugin = LolPlugin::new();
        {
            let mut p = plugin.pushes.lock().unwrap();
            let s = &mut p.champ_select;
            s.key(|| "champ-select:9200001".into());
            // 中路的强度榜：亚索、劫、阿狸（排位里被对面禁了）、辛德拉、乐芙兰、维克托；盖伦不在中路榜上
            let rows = [
                row(157, "T1", 0.50, 0.10, &[]),
                row(238, "T1", 0.505, 0.08, &[]),
                row(103, "T2", 0.49, 0.06, &[]),
                row(134, "T2", 0.495, 0.05, &[]),
                row(7, "T1", 0.52, 0.04, &[238]),
                row(112, "T0", 0.53, 0.05, &[]),
            ];
            s.remember("rows:MIDDLE".into(), json!(rows));
            s.remember(
                "mu:157:MIDDLE".into(),
                json!([
                    [matchup(86, 0.55)],
                    [matchup(1, 0.4455), matchup(238, 0.47)]
                ]),
            );
            s.remember("mu:134:MIDDLE".into(), json!([[matchup(238, 0.53)], []]));
            s.remember("mu:7:MIDDLE".into(), json!([[], []]));
            // 客户端的英雄资料：[物理, 魔法, 远程, 前排, 强控]
            for (id, t) in [
                (266, [true, false, false, true, false]),
                (64, [true, false, false, false, true]),
                (157, [true, false, false, false, false]),
                (222, [true, false, true, false, false]),
                (238, [true, false, false, false, false]),
                (86, [true, false, false, true, false]),
            ] {
                s.remember(format!("traits:{id}"), json!(t));
            }
            s.run.as_mut().unwrap().games = Some(vec![
                game(157, 420, true),
                game(157, 420, false),
                game(134, 420, true),
                game(103, 420, true),
                game(7, 420, false),
            ]);
            for locked in [false, true] {
                s.remember(
                    format!("mine:420:157:middle:{locked}"),
                    json!({"champion": {"id": 157}, "locked": locked, "source": "qq101",
                           "importable": {"runes": true, "items": true, "spells": true}}),
                );
            }
            s.remember(
                "matchup:157:238:MIDDLE".into(),
                json!({"laneAdvantage": "opponent", "soloKillAdvantage": "opponent", "playStyle": "even", "tip": "Block Q with W"}),
            );
        }
        plugin
    }

    fn blocks(panel: &Value) -> Vec<&'static str> {
        BLOCKS
            .iter()
            .copied()
            .filter(|b| !panel[*b].is_null())
            .collect()
    }

    /// 峡谷的卡片一步一步长出来（ADR 043 B 轮）：预选 → 可以禁的 + 我的英雄（预选）；禁用；还没轮到你；轮到你 → 英雄池；
    /// 锁定 → 对线（OP.GG）+ 我的英雄（锁了）。阵容：我方 3 个人选了就有，对面锁了 3 个才有对面那一边。
    #[tokio::test]
    async fn the_rift_card_grows_with_the_draft() {
        let plugin = prefilled();
        let host = SwitchHost {
            control: false,
            opgg: true,
        };
        let lcu = Lcu::with_base("http://127.0.0.1:1", "tok");
        let mut seen = Vec::new();
        for stage in ["planning", "ban", "wait", "pick", "locked"] {
            let session = draft::tests::ranked_session(stage);
            let mut panel = json!({});
            for b in BLOCKS {
                panel[b] = Value::Null;
            }
            plugin
                .rift_blocks(&host, &lcu, &session, 420, &mut panel, &name)
                .await;
            seen.push((stage, panel["stage"].clone(), blocks(&panel), panel));
        }
        let at = |stage: &str| &seen.iter().find(|s| s.0 == stage).unwrap().3;
        let kinds: Vec<(&str, Value, Vec<&str>)> = seen
            .iter()
            .map(|(s, st, b, _)| (*s, st.clone(), b.clone()))
            .collect();
        assert_eq!(
            kinds,
            [
                ("planning", json!("planning"), vec!["bans", "comp", "mine"]),
                ("ban", json!("banning"), vec!["bans", "comp", "mine"]),
                ("wait", json!("waiting"), vec!["comp", "mine"]),
                ("pick", json!("picking"), vec!["comp", "pool", "mine"]),
                ("locked", json!("locked"), vec!["comp", "matchup", "mine"]),
            ]
        );
        // 可以禁的：克制亚索的（胜率最低的在前；劫不是队友，列；盖伦是“好打”不列），再补中路最强的
        let bans = &at("planning")["bans"];
        assert_eq!(bans["lane"], "中路");
        assert_eq!(bans["against"]["short"], "英雄157");
        let ids: Vec<i64> = bans["list"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| b["champion"]["id"].as_i64().unwrap())
            .collect();
        assert_eq!(ids, [1, 238, 112, 7, 134], "队友的英雄（157）不列");
        assert_eq!(bans["source"], "qq101");
        // 阵容：我方 4 个人选了（266 64 157 222），全是物理、有前排、有强控；预选时对面看不到
        let comp = &at("planning")["comp"];
        assert_eq!(comp["ally"]["count"], 4);
        assert_eq!(comp["ally"]["flags"], json!(["all_physical"]));
        assert!(comp["enemy"].is_null());
        assert_eq!(comp["source"], "client");
        // 英雄池：辛德拉对劫好打 → 第一；亚索是我预选的，照样列；乐芙兰被劫克制；阿狸被禁了不列
        let pool = &at("pick")["pool"];
        let ids: Vec<i64> = pool["list"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["champion"]["id"].as_i64().unwrap())
            .collect();
        assert_eq!(ids, [134, 157, 7]);
        assert_eq!(pool["list"][0]["good"][0]["champion"]["short"], "英雄238");
        assert_eq!(pool["against"].as_array().unwrap().len(), 2);
        // 对线：猜的对手是劫（对面在中路榜上选用率最高的），OP.GG 的几条事实原样
        let matchup = &at("locked")["matchup"];
        assert_eq!(matchup["opponent"]["id"], 238);
        assert_eq!(matchup["guessed"], true);
        assert_eq!(matchup["laneAdvantage"], "opponent");
        assert_eq!(matchup["source"], "opgg");
        assert_eq!(at("locked")["mine"]["locked"], true);
        assert_eq!(at("planning")["mine"]["locked"], false);

        // OP.GG 关着：锁定以后没有对线那块，别的照旧
        let off = SwitchHost {
            control: false,
            opgg: false,
        };
        let mut panel = json!({});
        plugin
            .rift_blocks(
                &off,
                &lcu,
                &draft::tests::ranked_session("locked"),
                420,
                &mut panel,
                &name,
            )
            .await;
        assert!(panel["matchup"].is_null());
        assert!(!panel["mine"].is_null());
    }

    /// 一键导入要“操作客户端”开着：关着时不连客户端、什么都不写，回一句为什么（页面带玩家去设置）；开着但客户端没开时说没开。
    #[tokio::test]
    async fn import_needs_the_control_switch() {
        let plugin = LolPlugin::new();
        let off = SwitchHost {
            control: false,
            opgg: true,
        };
        let result = plugin.import_tool(&off, &json!({})).await;
        assert_eq!(result.content["status"], "disabled");
        assert_eq!(result.content["reason"], "control_off");
        let on = SwitchHost {
            control: true,
            opgg: true,
        };
        let result = plugin.import_tool(&on, &json!({})).await;
        assert_eq!(result.content["status"], "not_in_client");
        // 卡片上带着开关：关着时按钮写为什么
        let spec = crate::tools::specs()
            .into_iter()
            .find(|s| s.name == crate::tools::TOOL_IMPORT)
            .unwrap();
        assert_eq!(spec.needs, ["client", "control"]);
    }

    /// 真数据核对（要连网，连不上时用 `--example stats_probe` 留在 `target/lol-stats-probe/` 的缓存）：腾讯的海斗英雄榜
    /// 换算的评级分布、一张选英雄卡片；艾瑞莉娅面对她最好的、中间的、最差的三张海克斯的判断。
    /// `cargo test -p kotone-plugin-lol real_data_champ_select -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn real_data_champ_select_and_augments() {
        let dir =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/lol-stats-probe");
        let stats = crate::stats::Stats::new(dir);
        let days: Vec<String> = (0..7).map(crate::stats_tools::beijing_date).collect();
        let table = stats
            .mayhem_champions(&days)
            .await
            .expect("海斗英雄榜")
            .value
            .1;
        let names = stats.nicknames().await.expect("外号");
        let ids: Vec<i64> = table.iter().map(|r| r.champion_id).collect();
        let team = vec![
            (ids[0], String::new(), true),
            (ids[ids.len() / 2], String::new(), false),
        ];
        let panel = mayhem_panel(&team, &ids[1..6], &table, |id| names.short(id));
        let mut counts = std::collections::BTreeMap::new();
        let (ranks, total) = rules::win_ranks(table.iter().map(|r| (r.champion_id, r.win_rate)));
        for rank in ranks.values() {
            *counts
                .entry(rules::rank_grade(*rank, total).unwrap())
                .or_insert(0) += 1;
        }
        println!("海斗英雄榜 {total} 个，评级分布 {counts:?}");
        println!("{}", serde_json::to_string_pretty(&panel).unwrap());
        assert!(has_data(&panel));

        let hex = stats.aramhex().await.expect("aramhex-data");
        let irelia = 39;
        let mut all = hex.best_for(irelia, 500, 1);
        assert!(all.len() > 10, "艾瑞莉娅的海克斯太少：{}", all.len());
        let picks = [
            all.remove(0).0,
            all.remove(all.len() / 2).0,
            all.pop().unwrap().0,
        ];
        for card in rules::evaluate(&hex, irelia, &picks) {
            println!(
                "{}：{:?} {:?} {}第 {:?}/{:?} 样本偏少={} {:?} 分={:?}",
                hex.augments[&card.augment_id].name,
                card.verdict,
                card.grade,
                card.rarity,
                card.rarity_rank,
                card.rarity_total,
                card.sample_warning,
                card.thin,
                card.score.map(|s| (s.delta, s.games)),
            );
        }
    }
}
