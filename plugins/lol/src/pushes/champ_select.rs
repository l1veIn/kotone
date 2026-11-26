//! 选英雄卡片（ADR 043 D7，原来贴在客户端右上角的选英雄覆盖层）：清单 `[[pushes]]` 的 `champ-select`，页面
//! `ui://kotone/lol/champ-select`（插件的 MCP App，聊天里和悬浮窗的小窗里都是它）。
//!
//! - 海斗照 AramHex 的选英雄标注：每个英雄一个评级（S–D）、胜率、第几 / 共几；备选席按胜率排，比你现在的英雄高的标出来，
//!   最高的那个圈出来（值得换）。峡谷：每个人这个位置的强度和胜率。数据是腾讯 101（掌盟）的。
//! - 一次选英雄一个 key（这局的对局 id，没有时用进选英雄的时刻）：每一轮（2 秒）重读选人接口，内容变了就再推一次，
//!   宿主原地更新（重随、和备选席换英雄以后跟着变）；一样的不推。一项数据都没有时不推（不发一张全是“—”的卡片）。
//! - 选英雄结束时再推一次，卡片上标“结束了”；进了游戏（不是被秒退）才带一句给她的说明（只有事实：你和队友用什么英雄、
//!   胜率或强度）。中间的变化只更新卡片，不给她说明（ADR 043 D3）。

use kotone_core::log;
use kotone_plugin_api::{Host, Push, PushCard, PushOutcome};
use serde_json::{json, Value};

use super::{rules, MAYHEM_QUEUE, PAGE_CHAMP_SELECT, PUSH_CHAMP_SELECT, SOURCE_QQ101};
use crate::client::assets;
use crate::client::http::paths;
use crate::client::Lcu;
use crate::stats::qq101;
use crate::LolPlugin;

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
}

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

/// 卡片里有没有一项数据（胜率、强度）：一项都没有时不推。
pub fn has_data(panel: &Value) -> bool {
    let rows = |key: &str| panel[key].as_array().cloned().unwrap_or_default();
    rows("team")
        .iter()
        .chain(&rows("bench"))
        .any(|r| !r["winRate"].is_null() || !r["tier"].is_null())
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
            })
            .key
            .clone()
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
        let Some(panel) = self.champ_select_panel(host, &session, queue).await else {
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
        has_data(&panel).then_some(panel)
    }
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
