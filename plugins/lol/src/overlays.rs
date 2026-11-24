//! 英雄联盟插件的两个覆盖层（照 aramhex：只在需要的阶段、插件和数据开关开着时才显示）：
//!
//! - **选英雄**（`ui://kotone/lol/champ-select`）：客户端接口给己方 5 个人（和大乱斗 / 海斗的备选席）的英雄，
//!   配上网上的统计，贴在客户端窗口右上角。海斗照 AramHex 的选英雄标注：每个英雄一个评级（S–D）、胜率、第几 / 共几；
//!   备选席按胜率排，比你现在的英雄高的标出来，最高的那个圈出来（值得换）。峡谷：那一路的强度和胜率。
//!   每一轮（2 秒）重读选人接口：重随、和备选席换英雄以后跟着变；一项数据都没有时不显示（不拿一块“—”挡着客户端）。
//! - **海克斯评分**（`ui://kotone/lol/augment-scores`）：海斗对局里，截游戏画面中间那块识别文字，认出三张海克斯卡片的
//!   名字（aramhex-data 的海克斯名），按这局自己英雄的 英雄 × 海克斯 胜率打分，贴在每张卡片上面；照 AramHex 的三选一标签
//!   给每张一个判断（首选 / 可选 / 可考虑刷新）、评级和依据。
//!   只用本地文字识别（在线识别太慢、要花钱）；覆盖层本身不进截图。
//!
//! 评级和判断的规则在 [`rules`]（照 AramHex，MIT）。
//! 怎么摆、什么时候显示、置顶、点击穿透、不进截图都是本体的事（[`Host::overlay`]），这里只给数据和位置。

pub(crate) mod rules;

use std::sync::Arc;
use std::time::{Duration, Instant};

use kotone_plugin_api::{Anchor, Host, OverlayItem, OverlayTarget, ScreenText};
use serde_json::{json, Value};

use crate::client::live::LiveGame;
use crate::client::Lcu;
use crate::stats::{aramhex, qq101};
use crate::stats_tools::SOURCE_STATS;
use crate::LolPlugin;

pub const OVERLAY_CHAMP_SELECT: &str = "ui://kotone/lol/champ-select";
pub const OVERLAY_AUGMENTS: &str = "ui://kotone/lol/augment-scores";
const CLIENT_EXE: &str = "LeagueClientUx.exe";
const GAME_EXE: &str = "League of Legends.exe";
const MAYHEM_QUEUE: i64 = 2400;
/// 三选一的卡片在画面中间：只截这一块（比例）。
const CARD_REGION: [f64; 4] = [0.12, 0.12, 0.76, 0.62];
/// 多久看一次画面（本地识别一次约 0.1–0.3 秒）。
const CARD_POLL: Duration = Duration::from_secs(2);
/// 卡片没了多久以后收起评分。
const CARD_GONE: Duration = Duration::from_secs(4);
/// 三选一标签的高度（游戏画面高度的比例，三行字）和它的底边离卡片名字多远。
const CARD_LABEL_H: f64 = 0.085;
const CARD_LABEL_GAP: f64 = 0.01;

/// 覆盖层的状态（防止重复发同样的东西、识别叠在一起）。
#[derive(Default)]
pub(crate) struct OverlayState {
    /// 上次发出去的条目（一样就不再发）。
    last: Option<Value>,
    /// 上次看画面的时间。
    last_scan: Option<Instant>,
    /// 最近一次认出卡片的时间。
    cards_seen: Option<Instant>,
    /// 正在识别（上一次还没完就不叠一次）。
    scanning: bool,
    /// 本地识别用不了时记一次日志就够了。
    ocr_note: bool,
}

fn norm(text: &str) -> String {
    text.chars()
        .filter(|c| {
            !c.is_whitespace() && !c.is_ascii_punctuation() && !"·•“”‘’：:「」【】".contains(*c)
        })
        .collect()
}

/// 识别出来的几行字 → 认出来的海克斯（行、id）。名字整行对上，或者一行里包含名字（至少 3 个字）。
/// 同一个海克斯只认一次（取最靠上的那行）。
pub fn match_augments(lines: &[ScreenText], d: &aramhex::Dataset) -> Vec<(ScreenText, i64)> {
    let names: Vec<(i64, String)> = d
        .augments
        .iter()
        .map(|(id, a)| (*id, norm(&a.name)))
        .filter(|(_, n)| n.chars().count() >= 2)
        .collect();
    let mut out: Vec<(ScreenText, i64)> = Vec::new();
    for line in lines {
        let text = norm(&line.text);
        if text.chars().count() < 2 {
            continue;
        }
        let hit = names.iter().find(|(_, n)| *n == text).or_else(|| {
            // 最长的名字优先（“鼓声暴起 低音轰鸣”不要被“鼓声暴起”抢走）
            let mut contained: Vec<&(i64, String)> = names
                .iter()
                .filter(|(_, n)| n.chars().count() >= 3 && text.contains(n.as_str()))
                .collect();
            contained.sort_by_key(|(_, n)| std::cmp::Reverse(n.chars().count()));
            contained.into_iter().next()
        });
        if let Some((id, _)) = hit {
            match out.iter_mut().find(|(_, existing)| existing == id) {
                Some(slot) if slot.0.rect[1] > line.rect[1] => slot.0 = line.clone(),
                Some(_) => {}
                None => out.push((line.clone(), *id)),
            }
        }
    }
    out.sort_by(|a, b| a.0.rect[0].total_cmp(&b.0.rect[0]));
    out
}

/// 认出来的卡片 → 覆盖层条目：每张卡片一个，贴在名字上面一点（盖住海克斯图标的下半截，不挡名字、说明和下面的刷新按钮，
/// 照 AramHex 的默认位置）。几张卡片一起判断（[`rules::evaluate`]），换了一张，另外几张的判断也跟着重算。
pub fn augment_items(
    found: &[(ScreenText, i64)],
    hero: i64,
    hero_name: &str,
    d: &aramhex::Dataset,
) -> Vec<OverlayItem> {
    let ids: Vec<i64> = found.iter().map(|(_, id)| *id).collect();
    let cards = rules::evaluate(d, hero, &ids);
    found
        .iter()
        .zip(&cards)
        .map(|((line, id), card)| {
            let [x, y, w, _] = line.rect;
            let width = w.max(0.12) + 0.02;
            let name = d.augments.get(id).map(|a| a.name.as_str()).unwrap_or("");
            let mut data = serde_json::to_value(card).unwrap_or_else(|_| json!({}));
            if let Some(map) = data.as_object_mut() {
                map.insert("augment".into(), json!(name));
                map.insert("champion".into(), json!(hero_name));
                map.insert("attribution".into(), json!("aramhex-data"));
            }
            OverlayItem {
                id: format!("augment-{id}"),
                uri: OVERLAY_AUGMENTS.into(),
                target: OverlayTarget::Process(GAME_EXE.into()),
                anchor: Anchor::Rect {
                    x: (x + w / 2.0 - width / 2.0).max(0.0),
                    y: (y - CARD_LABEL_GAP - CARD_LABEL_H).max(0.0),
                    w: width,
                    h: CARD_LABEL_H,
                },
                data,
            }
        })
        .collect()
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

/// 海斗的选英雄面板（照 AramHex 的选英雄标注）：每个英雄一个评级（只按胜率重排的名次换算 S–D）、胜率、第几 / 共几；
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
            "champion": short(id),
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
        "source": "腾讯 101（掌盟）",
    })
}

/// 选英雄面板 → 覆盖层条目；一项数据（胜率、强度）都没有时不显示。高度跟着行数走（只盖要盖的那块）。
pub fn champ_select_item(data: Value) -> Option<OverlayItem> {
    let rows = |key: &str| data[key].as_array().cloned().unwrap_or_default();
    let (team, bench) = (rows("team"), rows("bench"));
    let has_data = team
        .iter()
        .chain(&bench)
        .any(|r| !r["winRate"].is_null() || !r["tier"].is_null());
    if !has_data {
        return None;
    }
    // 标题、出处和上下边约 7.5%，每行约 2.9%（字号跟面板宽度走，客户端是 16:9）；备选两个一行，加一行小标题
    let lines = team.len()
        + if bench.is_empty() {
            0
        } else {
            1 + bench.len().div_ceil(2)
        };
    Some(OverlayItem {
        id: "champ-select".into(),
        uri: OVERLAY_CHAMP_SELECT.into(),
        target: OverlayTarget::Process(CLIENT_EXE.into()),
        anchor: Anchor::Corner {
            corner: "top-right".into(),
            margin: 0.01,
            w: 0.2,
            h: (0.075 + 0.029 * lines as f64).clamp(0.1, 0.5),
        },
        data,
    })
}

impl LolPlugin {
    pub(crate) fn send_overlay(&self, host: &dyn Host, items: Vec<OverlayItem>) {
        let value = serde_json::to_value(&items).unwrap_or(Value::Null);
        let mut o = self.overlay.lock().unwrap();
        if o.last.as_ref() == Some(&value) {
            return;
        }
        o.last = Some(value);
        drop(o);
        host.overlay(items);
    }

    /// 每一轮（客户端开着时）：按阶段决定显示哪个覆盖层。
    pub(crate) async fn update_overlays(
        &self,
        host: &Arc<dyn Host>,
        lcu: &Lcu,
        phase: &str,
        live: Option<&LiveGame>,
    ) {
        if !host.source_enabled(SOURCE_STATS) {
            self.send_overlay(host.as_ref(), Vec::new());
            return;
        }
        match phase {
            "ChampSelect" => {
                let items = self.champ_select_items(host.as_ref(), lcu).await;
                self.send_overlay(host.as_ref(), items);
            }
            _ => match live {
                Some(game) if self.current_queue() == Some(MAYHEM_QUEUE) => {
                    self.scan_cards(host, game).await;
                }
                _ => self.send_overlay(host.as_ref(), Vec::new()),
            },
        }
    }

    async fn champ_select_items(&self, host: &dyn Host, lcu: &Lcu) -> Vec<OverlayItem> {
        let Ok(session) = lcu.get(crate::client::http::paths::CHAMP_SELECT).await else {
            return Vec::new();
        };
        let queue = match lcu.get(crate::client::http::paths::SESSION).await {
            Ok(v) => v["gameData"]["queue"]["id"].as_i64().unwrap_or(0),
            Err(_) => 0,
        };
        let (team, bench) = seats(&session);
        if team.is_empty() && bench.is_empty() {
            return Vec::new();
        }
        let stats = self.stats_for(host);
        let names = stats.nicknames().await.ok();
        let short = |id: i64| {
            names
                .as_ref()
                .map(|n| n.short(id))
                .unwrap_or_else(|| format!("英雄 {id}"))
        };
        let data = if queue == MAYHEM_QUEUE {
            let days: Vec<String> = (0..7).map(crate::stats_tools::beijing_date).collect();
            let table = stats
                .mayhem_champions(&days)
                .await
                .map(|t| t.value.1)
                .unwrap_or_default();
            mayhem_panel(&team, &bench, &table, short)
        } else {
            let mut team_rows = Vec::new();
            for (id, position, me) in &team {
                let lane = match position.as_str() {
                    "top" => Some("TOP"),
                    "jungle" => Some("JUNGLE"),
                    "middle" => Some("MIDDLE"),
                    "bottom" => Some("BOTTOM"),
                    "utility" => Some("SUPPORT"),
                    _ => None,
                };
                let row = match lane {
                    Some(lane) => stats
                        .tier_list(lane)
                        .await
                        .ok()
                        .and_then(|t| t.value.1.into_iter().find(|r| r.champion_id == *id)),
                    None => None,
                };
                team_rows.push(json!({
                    "champion": short(*id), "me": me,
                    "lane": lane.map(qq101::lane_name),
                    "tier": row.as_ref().map(|r| r.tier.clone()),
                    "winRate": row.as_ref().and_then(|r| r.win_rate),
                }));
            }
            json!({"mode": "rift", "team": team_rows, "bench": [], "source": "腾讯 101（掌盟）"})
        };
        champ_select_item(data).into_iter().collect()
    }

    /// 海斗对局里：看画面中间有没有三选一的卡片；有就贴评分，没了一会儿就收起。
    async fn scan_cards(&self, host: &Arc<dyn Host>, game: &LiveGame) {
        {
            let mut o = self.overlay.lock().unwrap();
            if o.scanning || o.last_scan.is_some_and(|t| t.elapsed() < CARD_POLL) {
                return;
            }
            o.scanning = true;
            o.last_scan = Some(Instant::now());
        }
        let result = host.screen_text(GAME_EXE, CARD_REGION, true).await;
        let lines = match result {
            Ok(lines) => lines,
            Err(e) => {
                let mut o = self.overlay.lock().unwrap();
                o.scanning = false;
                if !o.ocr_note && !e.starts_with("前台不是") {
                    o.ocr_note = true;
                    kotone_core::log::log(&format!("lol: 海克斯评分看不了画面：{e}"));
                }
                drop(o);
                self.hide_cards_if_gone(host.as_ref());
                return;
            }
        };
        let hex = self.stats_for(host.as_ref()).aramhex().await;
        let me = game.me().map(|p| p.alias.clone()).unwrap_or_default();
        let data = self.loaded_data();
        let hero = data
            .as_deref()
            .and_then(|d| d.champion_by_alias(&me))
            .map(|c| (c.id, c.short().to_string()));
        let items = match (&hex, hero) {
            (Ok(d), Some((id, name))) => {
                let found = match_augments(&lines, d);
                if found.len() >= 2 {
                    augment_items(&found, id, &name, d)
                } else {
                    Vec::new()
                }
            }
            _ => Vec::new(),
        };
        self.overlay.lock().unwrap().scanning = false;
        if items.is_empty() {
            self.hide_cards_if_gone(host.as_ref());
        } else {
            self.overlay.lock().unwrap().cards_seen = Some(Instant::now());
            self.send_overlay(host.as_ref(), items);
        }
    }

    fn hide_cards_if_gone(&self, host: &dyn Host) {
        let gone = self
            .overlay
            .lock()
            .unwrap()
            .cards_seen
            .map_or(true, |t| t.elapsed() >= CARD_GONE);
        if gone {
            self.send_overlay(host, Vec::new());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stats::aramhex::{parse_latest, Dataset};

    fn dataset() -> Dataset {
        let latest =
            parse_latest(&json!({"path": "patches/16.23/normal", "patch": "16.23"})).unwrap();
        Dataset::from_json(
            &latest,
            &json!({"39": {"games": 100000, "wins": 50000, "win_rate": 0.5}}),
            &json!({
                "1": {"name": "鼓声暴起", "rarity": "gold"},
                "2": {"name": "鼓声暴起 低音轰鸣", "rarity": "prismatic"},
                "3": {"name": "终极九头蛇", "rarity": "prismatic"},
                "4": {"name": "弹跳蜜蜂", "rarity": "silver"}
            }),
            &json!({"39": {
                "2": {"games": 30000, "wins": 17000},
                "3": {"games": 10000, "wins": 5800},
                "4": {"games": 6000, "wins": 2900}
            }}),
        )
    }

    fn line(text: &str, x: f64, y: f64) -> ScreenText {
        ScreenText {
            text: text.into(),
            rect: [x, y, 0.1, 0.03],
        }
    }

    #[test]
    fn three_cards_are_found_and_scored() {
        let d = dataset();
        let lines = vec![
            line("选择一个强化", 0.4, 0.1),
            line("终极九头蛇", 0.6, 0.35),
            line("鼓声暴起·低音轰鸣", 0.2, 0.35),
            line("弹跳 蜜蜂", 0.4, 0.36),
            line("每次攻击……", 0.4, 0.5),
        ];
        let found = match_augments(&lines, &d);
        let ids: Vec<i64> = found.iter().map(|(_, id)| *id).collect();
        assert_eq!(ids, vec![2, 4, 3], "从左到右，长名字优先");
        let items = augment_items(&found, 39, "艾瑞莉娅", &d);
        assert_eq!(items.len(), 3);
        assert!(items.iter().all(|i| i.uri == OVERLAY_AUGMENTS));
        // 首选是终极九头蛇（5800/10000 = 58%，平滑后 +7.8，也比 17000/30000 的 +6.6 高），在最右边；
        // 鼓声暴起 低音轰鸣低不到 2 个点：可选；弹跳蜜蜂比平均低：可考虑刷新
        let verdicts: Vec<&str> = items
            .iter()
            .map(|i| i.data["verdict"].as_str().unwrap())
            .collect();
        assert_eq!(verdicts, ["ok", "reroll", "best"]);
        assert_eq!(items[2].data["augment"], "终极九头蛇");
        assert_eq!(items[2].data["augmentId"], 3);
        assert_eq!(items[2].data["champion"], "艾瑞莉娅");
        assert_eq!(items[2].data["attribution"], "aramhex-data");
        // 棱彩两个里排第一 → 1/2 = 50% → B；评级、名次、样本都给页面
        assert_eq!(items[2].data["grade"], "B");
        assert_eq!(items[2].data["rarity"], "prismatic");
        assert_eq!(
            (&items[2].data["rarityRank"], &items[2].data["rarityTotal"]),
            (&json!(1), &json!(2))
        );
        assert_eq!(items[2].data["sampleWarning"], false);
        assert!(items[2].data["thin"].is_null());
        match &items[0].anchor {
            Anchor::Rect { y, h, .. } => {
                assert!(*y + *h < 0.35, "标签在名字上面");
                assert!((*h - CARD_LABEL_H).abs() < 1e-9);
            }
            other => panic!("{other:?}"),
        }
        assert!(items[0].data["score"]["delta"].as_f64().unwrap() > 0.0);
    }

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

    fn session(me: i64, team: &[i64], bench: &[i64]) -> Value {
        json!({
            "localPlayerCellId": 0,
            "myTeam": team.iter().enumerate().map(|(cell, id)| json!({
                "cellId": cell, "championId": if cell == 0 { me } else { *id }, "assignedPosition": ""
            })).collect::<Vec<_>>(),
            "benchChampions": bench.iter().map(|id| json!({"championId": id})).collect::<Vec<_>>(),
        })
    }

    fn name(id: i64) -> String {
        format!("英雄{id}")
    }

    #[test]
    fn champ_select_grades_and_bench_comparison() {
        // 我是 8（第 8 名 → 8/20 = 40% → B），队友 2、15、19、20；备选 6、12、3、9
        let (team, bench) = seats(&session(8, &[8, 2, 15, 19, 20], &[6, 12, 3, 9]));
        assert_eq!(team.len(), 5);
        assert!(team[0].2 && !team[1].2);
        let panel = mayhem_panel(&team, &bench, &mayhem_table(), name);
        assert_eq!(panel["mode"], "mayhem");
        assert_eq!(panel["total"], 20);
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
        assert_eq!(panel["team"][0]["champion"], "英雄8");
        // 备选按胜率从高到低；比我（8）高的 3、6 标出来，最高的 3 圈出来
        let bench = panel["bench"].as_array().unwrap();
        let order: Vec<&str> = bench
            .iter()
            .map(|r| r["champion"].as_str().unwrap())
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
    }

    #[test]
    fn the_panel_follows_a_reroll() {
        // 我重随：8 进了备选席，换成 1（全场最高）；备选里没有比我高的了
        let table = mayhem_table();
        let before = mayhem_panel(
            &seats(&session(8, &[8, 2, 15, 19, 20], &[6, 12])).0,
            &seats(&session(8, &[8, 2, 15, 19, 20], &[6, 12])).1,
            &table,
            name,
        );
        let (team, bench) = seats(&session(1, &[1, 2, 15, 19, 20], &[6, 12, 8]));
        let after = mayhem_panel(&team, &bench, &table, name);
        assert_ne!(before, after, "内容变了才会重发（send_overlay 比较整份）");
        assert_eq!(before["bench"][0]["best"], true);
        assert_eq!(after["team"][0]["grade"], "S");
        assert!(after["bench"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["better"] == false && r["best"] == false));
        assert_eq!(after["bench"][2]["champion"], "英雄12");
        // 一样的 session 两次：一模一样（不重发）
        assert_eq!(after, mayhem_panel(&team, &bench, &table, name));
    }

    #[test]
    fn the_panel_hides_without_data_and_grows_with_the_bench() {
        let (team, bench) = seats(&session(8, &[8, 2, 15, 19, 20], &[6, 12, 3, 9]));
        // 腾讯的海斗榜拿不到：一项数据都没有，不显示
        assert!(champ_select_item(mayhem_panel(&team, &bench, &[], name)).is_none());
        // 峡谷里谁都没有分路（例如大乱斗）：也不显示
        let rift = json!({"mode": "rift", "team": [{"champion": "甲", "me": true, "lane": null, "tier": null, "winRate": null}], "bench": []});
        assert!(champ_select_item(rift).is_none());
        let height = |bench: &[i64]| match champ_select_item(mayhem_panel(
            &team,
            bench,
            &mayhem_table(),
            name,
        ))
        .unwrap()
        .anchor
        {
            Anchor::Corner { h, w, corner, .. } => {
                assert_eq!((w, corner.as_str()), (0.2, "top-right"));
                h
            }
            other => panic!("{other:?}"),
        };
        let (none, four, ten) = (
            height(&[]),
            height(&bench),
            height(&[3, 4, 5, 6, 7, 9, 10, 11, 12, 13]),
        );
        assert!(none < four && four < ten, "{none} {four} {ten}");
        assert!(none > 0.15 && ten <= 0.5);
        // 没有 localPlayerCellId 时谁都不是“我”（以前 None == None 会把每个人都当成我）
        let (team, _) = seats(&json!({"myTeam": [{"championId": 5}, {"championId": 6}]}));
        assert!(team.iter().all(|(_, _, me)| !me));
    }

    #[test]
    fn ordinary_text_is_not_a_card() {
        let d = dataset();
        assert!(
            match_augments(&[line("对面拿了火龙", 0.4, 0.3), line("鼓", 0.1, 0.1)], &d).is_empty()
        );
    }

    /// 真数据核对（要连网，连不上时用 `--example stats_probe` 留在 `target/lol-stats-probe/` 的缓存）：腾讯的海斗英雄榜
    /// 换算的评级分布、艾瑞莉娅面对她最好的、中间的、最差的三张海克斯的判断。
    /// `cargo test -p kotone-plugin-lol real_data_overlay -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn real_data_overlay() {
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
        assert!(champ_select_item(panel).is_some());

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
