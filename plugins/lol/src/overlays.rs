//! 英雄联盟插件的两个覆盖层（照 aramhex：只在需要的阶段、插件和数据开关开着时才显示）：
//!
//! - **选英雄**（`ui://kotone/lol/champ-select`）：客户端接口给己方 5 个人（和大乱斗 / 海斗的备选席）的英雄，
//!   配上网上的统计（海斗：腾讯的海斗英雄榜；峡谷：那一路的强度和胜率），贴在客户端窗口右上角；
//! - **海克斯评分**（`ui://kotone/lol/augment-scores`）：海斗对局里，截游戏画面中间那块识别文字，认出三张海克斯卡片的
//!   名字（aramhex-data 的海克斯名），按这局自己英雄的 英雄 × 海克斯 胜率打分，贴在每张卡片上面。
//!   只用本地文字识别（在线识别太慢、要花钱）；覆盖层本身不进截图。
//!
//! 怎么摆、什么时候显示、置顶、点击穿透、不进截图都是本体的事（[`Host::overlay`]），这里只给数据和位置。

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

/// 认出来的卡片 → 覆盖层条目：每张卡片一个，贴在名字上面一点。
pub fn augment_items(
    found: &[(ScreenText, i64)],
    hero: i64,
    hero_name: &str,
    d: &aramhex::Dataset,
) -> Vec<OverlayItem> {
    let scores: Vec<Option<aramhex::AugmentScore>> =
        found.iter().map(|(_, id)| d.score(hero, *id)).collect();
    let best = scores
        .iter()
        .enumerate()
        .filter_map(|(i, s)| s.map(|s| (i, s.smoothed)))
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i);
    found
        .iter()
        .zip(&scores)
        .enumerate()
        .map(|(i, ((line, id), score))| {
            let [x, y, w, _] = line.rect;
            let width = w.max(0.12) + 0.02;
            let name = d.augments.get(id).map(|a| a.name.as_str()).unwrap_or("");
            OverlayItem {
                id: format!("augment-{id}"),
                uri: OVERLAY_AUGMENTS.into(),
                target: OverlayTarget::Process(GAME_EXE.into()),
                anchor: Anchor::Rect {
                    x: (x + w / 2.0 - width / 2.0).max(0.0),
                    y: (y - 0.065).max(0.0),
                    w: width,
                    h: 0.055,
                },
                data: json!({
                    "augment": name,
                    "champion": hero_name,
                    "score": score,
                    "best": best == Some(i),
                    "attribution": "aramhex-data",
                }),
            }
        })
        .collect()
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
        let Ok(session) = lcu.get("/lol-champ-select/v1/session").await else {
            return Vec::new();
        };
        let queue = match lcu.get(crate::client::http::paths::SESSION).await {
            Ok(v) => v["gameData"]["queue"]["id"].as_i64().unwrap_or(0),
            Err(_) => 0,
        };
        let me = session["localPlayerCellId"].as_i64();
        let team: Vec<(i64, String, bool)> = session["myTeam"]
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
                            m["cellId"].as_i64() == me,
                        )
                    })
                    .filter(|(id, _, _)| *id > 0)
                    .collect()
            })
            .unwrap_or_default();
        let bench: Vec<i64> = session["benchChampions"]
            .as_array()
            .map(|b| b.iter().filter_map(|c| c["championId"].as_i64()).collect())
            .unwrap_or_default();
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
            let row = |id: i64| table.iter().find(|r| r.champion_id == id);
            json!({
                "mode": "mayhem",
                "team": team.iter().map(|(id, _, me)| json!({
                    "champion": short(*id), "me": me,
                    "winRate": row(*id).and_then(|r| r.win_rate), "rank": row(*id).and_then(|r| r.rank),
                })).collect::<Vec<_>>(),
                "bench": bench.iter().map(|id| json!({
                    "champion": short(*id),
                    "winRate": row(*id).and_then(|r| r.win_rate), "rank": row(*id).and_then(|r| r.rank),
                })).collect::<Vec<_>>(),
                "source": "腾讯 101（掌盟）",
            })
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
        vec![OverlayItem {
            id: "champ-select".into(),
            uri: OVERLAY_CHAMP_SELECT.into(),
            target: OverlayTarget::Process(CLIENT_EXE.into()),
            anchor: Anchor::Corner {
                corner: "top-right".into(),
                margin: 0.01,
                w: 0.2,
                h: 0.32,
            },
            data,
        }]
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
        // 最好的是终极九头蛇（5800/10000 = 58%，平滑后也比 17000/30000 高），在最右边
        assert_eq!(items[2].data["best"], true);
        assert_eq!(items[0].data["best"], false);
        match &items[0].anchor {
            Anchor::Rect { y, .. } => assert!(*y < 0.35),
            other => panic!("{other:?}"),
        }
        assert!(items[0].data["score"]["delta"].as_f64().unwrap() > 0.0);
    }

    #[test]
    fn ordinary_text_is_not_a_card() {
        let d = dataset();
        assert!(
            match_augments(&[line("对面拿了火龙", 0.4, 0.3), line("鼓", 0.1, 0.1)], &d).is_empty()
        );
    }
}
