//! 选人卡片上“我的英雄”那块和一键导入（ADR 043 B 轮）：卡片上画的就是导入时写的那一份，一块只用一个来源
//! （ADR 043 D8）——峡谷是腾讯 101 这一路的（符文、出装、加点、召唤师技能），海斗是 aramhex-data 的（最好的海克斯、
//! 核心三件、召唤师技能；没有符文）。
//!
//! 一键导入是只给页面的工具 `lol.import`（卡片上的按钮调它）：要“操作客户端”开着（默认关，玩家自己在插件设置里打开，
//! 第一次打开时有风险和许可的弹窗，ADR 043 D8）；经 [`crate::client::loadout`] 写符文页、出装方案、召唤师技能，
//! 返回每一样写成了什么。开关关着时不写、不帮玩家打开，只回一句为什么（页面带玩家去设置）。

use serde_json::{json, Value};

use kotone_core::log;
use kotone_plugin_api::{Host, ToolResult};

use super::{champ_select, draft, MAYHEM_QUEUE};
use crate::client::http::paths;
use crate::client::loadout::{self, ItemBlock, Loadout, RuneSpec, Written};
use crate::client::GameData;
use crate::stats::{aramhex, qq101, statics::Statics, ChampionStats};
use crate::{LolPlugin, SOURCE_CONTROL, SOURCE_STATS};

/// 海斗最好的海克斯至少要多少局才列（和英雄页一样）。
const MIN_AUGMENT_GAMES: i64 = 300;
/// 卡片上列几个海克斯。
const AUGMENTS_SHOWN: usize = 5;

/// 名字和图（名字先用客户端的中文资料，没有时用腾讯的名字表；图只有客户端资料读过才有路径）。
pub(crate) struct Names<'a> {
    pub statics: &'a Statics,
    pub data: Option<&'a GameData>,
}

impl Names<'_> {
    pub fn item(&self, id: i64) -> Value {
        let icon = self
            .data
            .and_then(|d| d.item_icons.get(&id))
            .cloned()
            .unwrap_or_default();
        let name = self
            .data
            .and_then(|d| d.items.get(&id))
            .cloned()
            .unwrap_or_else(|| self.statics.item(id));
        json!({"id": id, "name": name, "icon": icon})
    }

    pub fn rune(&self, id: i64) -> Value {
        let perk = self
            .data
            .and_then(|d| d.perks.get(&id).or_else(|| d.perk_styles.get(&id)));
        json!({
            "id": id,
            "name": perk.map(|p| p.name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| self.statics.rune(id)),
            "icon": perk.map(|p| p.icon.clone()).unwrap_or_default(),
        })
    }

    pub fn spell(&self, id: i64) -> Value {
        let info = self.data.and_then(|d| d.spell_by_id(id));
        json!({
            "id": id,
            "name": info.map(|s| s.name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| self.statics.spell(id)),
            "icon": info.map(|s| s.icon.clone()).unwrap_or_default(),
        })
    }
}

/// 选用率最高的那一种（固定规则：一样时排在前面的）。
fn most_picked(options: &[qq101::BuildOption]) -> Vec<&qq101::BuildOption> {
    let mut list: Vec<&qq101::BuildOption> =
        options.iter().filter(|o| !o.items.is_empty()).collect();
    list.sort_by(|a, b| {
        b.pick_rate
            .unwrap_or(0.0)
            .total_cmp(&a.pick_rate.unwrap_or(0.0))
    });
    list
}

/// 101 的一种符文（局数最多的那一页）→ 客户端要的一页；不齐时 None。
fn rune_spec(stats: &ChampionStats) -> Option<(RuneSpec, &qq101::RunePage)> {
    let page = stats
        .runes
        .iter()
        .max_by(|a, b| a.games.cmp(&b.games).then(b.rank.cmp(&a.rank)))?;
    let spec = RuneSpec {
        primary_style: page.primary_style.unwrap_or(0),
        sub_style: page.secondary_style.unwrap_or(0),
        perks: page
            .primary
            .iter()
            .chain(&page.secondary)
            .chain(&page.shards)
            .copied()
            .collect(),
    };
    spec.complete().then_some((spec, page))
}

/// 峡谷：101 这一路的一整份（符文局数最多的那一页；出门、鞋、核心三件、后期可选都按选用率；召唤师技能选用最多的一组）。
pub fn rift_loadout(
    champion: i64,
    short: &str,
    lane: Option<&str>,
    stats: &ChampionStats,
) -> Loadout {
    let mut blocks = Vec::new();
    if let Some(start) = most_picked(&stats.build.starting).first() {
        blocks.push(ItemBlock {
            title: "出门".into(),
            items: start.items.clone(),
        });
    }
    let boots: Vec<i64> = most_picked(&stats.build.boots)
        .iter()
        .take(2)
        .flat_map(|o| o.items.first().copied())
        .collect();
    if !boots.is_empty() {
        blocks.push(ItemBlock {
            title: "鞋".into(),
            items: boots,
        });
    }
    let core = most_picked(&stats.build.core);
    for (i, o) in core.iter().take(2).enumerate() {
        blocks.push(ItemBlock {
            title: if i == 0 {
                "核心三件".into()
            } else {
                "核心三件（另一种）".into()
            },
            items: o.items.clone(),
        });
    }
    let first_core: Vec<i64> = core.first().map(|o| o.items.clone()).unwrap_or_default();
    let mut late: Vec<i64> = Vec::new();
    for slot in [&stats.build.fourth, &stats.build.fifth, &stats.build.sixth] {
        for o in most_picked(slot).iter().take(2) {
            for item in &o.items {
                if !late.contains(item) && !first_core.contains(item) {
                    late.push(*item);
                }
            }
        }
    }
    if !late.is_empty() {
        blocks.push(ItemBlock {
            title: "后期可选".into(),
            items: late,
        });
    }
    let lane_name = lane.map(qq101::lane_name).unwrap_or("");
    Loadout {
        champion,
        name: [loadout::PREFIX, short, lane_name]
            .iter()
            .filter(|s| !s.is_empty())
            .copied()
            .collect::<Vec<_>>()
            .join(" "),
        uid: format!("kotone-{champion}-rift"),
        map: loadout::MAP_RIFT,
        runes: rune_spec(stats).map(|(spec, _)| spec),
        blocks,
        spells: stats.spells.first().map(|p| p.spells),
    }
}

/// 海斗的核心三件写几套、卡片上列几套（同一个数：卡片上看到的就是写进客户端的）。
const MAYHEM_CORE_SETS: usize = 2;

/// 海斗：aramhex-data 的核心三件（最多 [`MAYHEM_CORE_SETS`] 套，照文件的顺序）和用得最多的召唤师技能；没有符文。
pub fn mayhem_loadout(champion: i64, short: &str, hex: &aramhex::Dataset) -> Loadout {
    let blocks = hex
        .core_builds
        .get(&champion)
        .map(|builds| {
            builds
                .iter()
                .take(MAYHEM_CORE_SETS)
                .enumerate()
                .map(|(i, b)| ItemBlock {
                    title: format!("核心三件（第 {} 种）", i + 1),
                    items: b.items.clone(),
                })
                .collect()
        })
        .unwrap_or_default();
    Loadout {
        champion,
        name: format!("{} {short} 海斗", loadout::PREFIX),
        uid: format!("kotone-{champion}-mayhem"),
        map: loadout::MAP_ABYSS,
        runes: None,
        blocks,
        spells: hex.top_spells(champion).map(|p| p.spells),
    }
}

fn importable(loadout: &Loadout) -> Value {
    json!({
        "runes": loadout.runes.as_ref().is_some_and(RuneSpec::complete),
        "items": loadout.blocks.iter().any(|b| !b.items.is_empty()),
        "spells": loadout.spells.is_some(),
    })
}

/// 峡谷卡片上“我的英雄”那块（101）：符文、出装、加点、召唤师技能，和导入时写的是同一份。
pub fn rift_mine(
    champion: Value,
    lane: Option<&str>,
    locked: bool,
    stats: &ChampionStats,
    loadout: &Loadout,
    names: &Names,
) -> Value {
    let runes = rune_spec(stats).map(|(spec, page)| {
        json!({
            "styles": [names.rune(spec.primary_style), names.rune(spec.sub_style)],
            "perks": spec.perks.iter().map(|p| names.rune(*p)).collect::<Vec<_>>(),
            "winRate": page.win_rate, "pickRate": page.pick_rate,
        })
    });
    let group = |kind: &str, option: Option<&&qq101::BuildOption>, take: usize| {
        option.map(|o| {
            json!({"kind": kind, "items": o.items.iter().take(take).map(|i| names.item(*i)).collect::<Vec<_>>(),
                   "winRate": o.win_rate, "pickRate": o.pick_rate})
        })
    };
    let mut items: Vec<Value> = Vec::new();
    items.extend(group(
        "starting",
        most_picked(&stats.build.starting).first(),
        6,
    ));
    let boots: Vec<Value> = most_picked(&stats.build.boots)
        .iter()
        .take(2)
        .flat_map(|o| o.items.first().map(|i| names.item(*i)))
        .collect();
    if !boots.is_empty() {
        items.push(json!({"kind": "boots", "items": boots, "winRate": Value::Null, "pickRate": Value::Null}));
    }
    items.extend(group("core", most_picked(&stats.build.core).first(), 3));
    if let Some(late) = loadout.blocks.iter().find(|b| b.title == "后期可选") {
        items.push(json!({"kind": "late", "items": late.items.iter().take(6).map(|i| names.item(*i)).collect::<Vec<_>>(),
                          "winRate": Value::Null, "pickRate": Value::Null}));
    }
    let skills = (!stats.skill_priority.is_empty() || !stats.skill_order.is_empty()).then(|| {
        json!({"priority": stats.skill_priority, "order": stats.skill_order.iter().take(6).collect::<Vec<_>>()})
    });
    let spells = stats.spells.first();
    json!({
        "champion": champion,
        "lane": lane.map(qq101::lane_name),
        "locked": locked,
        "source": super::SOURCE_QQ101,
        "patch": stats.patch,
        "runes": runes,
        "items": items,
        "skills": skills,
        "spells": spells.map(|p| p.spells.iter().map(|s| names.spell(*s)).collect::<Vec<_>>()).unwrap_or_default(),
        "spellsWinRate": spells.and_then(|p| p.win_rate),
        "augments": [],
        "importable": importable(loadout),
    })
}

/// 海斗卡片上“我的英雄”那块（aramhex-data）：最好的海克斯（比这个英雄平均高几个点）、核心三件、召唤师技能。
pub fn mayhem_mine(
    champion: Value,
    hex: &aramhex::Dataset,
    loadout: &Loadout,
    names: &Names,
) -> Value {
    let id = loadout.champion;
    let augments: Vec<Value> = hex
        .best_for(id, AUGMENTS_SHOWN, MIN_AUGMENT_GAMES)
        .into_iter()
        .map(|(aid, s)| {
            let a = hex.augments.get(&aid);
            let icon = names
                .data
                .and_then(|d| d.augments.get(&aid))
                .map(|g| g.icon.clone())
                .unwrap_or_default();
            json!({"id": aid, "name": a.map(|a| a.name.as_str()).unwrap_or(""), "rarity": a.map(|a| a.rarity.as_str()).unwrap_or(""),
                   "icon": icon, "delta": s.delta, "games": s.games})
        })
        .collect();
    let items: Vec<Value> = hex
        .core_builds
        .get(&id)
        .map(|builds| {
            builds
                .iter()
                .take(MAYHEM_CORE_SETS)
                .map(|b| {
                    json!({"kind": "core", "items": b.items.iter().map(|i| names.item(*i)).collect::<Vec<_>>(),
                           "winRate": b.win_rate, "pickRate": b.pick_rate})
                })
                .collect()
        })
        .unwrap_or_default();
    let spells = hex.top_spells(id);
    json!({
        "champion": champion,
        "lane": Value::Null,
        "locked": true,
        "source": super::SOURCE_ARAMHEX,
        "patch": hex.patch,
        "runes": Value::Null,
        "items": items,
        "skills": Value::Null,
        "spells": spells.map(|p| p.spells.iter().map(|s| names.spell(*s)).collect::<Vec<_>>()).unwrap_or_default(),
        "spellsWinRate": spells.map(|p| p.win_rate),
        "augments": augments,
        "importable": importable(loadout),
    })
}

fn status(status: &str, reason: &str, text: impl Into<String>) -> Value {
    json!({"status": status, "reason": reason, "text": text.into()})
}

/// 给她的那一句（导入了什么），页面照结构化的结果画。
fn summary(loadout: &Loadout, runes: &Written, items: &Written, spells: &Written) -> String {
    let mut parts = Vec::new();
    if runes.ok() {
        parts.push(match &runes.old {
            Some(old) => format!("符文页「{}」（换掉了「{old}」）", loadout.name),
            None => format!("符文页「{}」", loadout.name),
        });
    }
    if items.ok() {
        parts.push(format!("出装方案「{}」", loadout.name));
    }
    if spells.ok() {
        parts.push("召唤师技能".to_string());
    }
    let failed: Vec<&str> = [
        ("符文页", runes),
        ("出装方案", items),
        ("召唤师技能", spells),
    ]
    .iter()
    .filter(|(_, w)| {
        w.result == loadout::WriteResult::Error || w.result == loadout::WriteResult::NoRoom
    })
    .map(|(n, _)| *n)
    .collect();
    let mut out = if parts.is_empty() {
        "什么都没写".to_string()
    } else {
        format!("写好了：{}", parts.join("、"))
    };
    if !failed.is_empty() {
        out.push_str(&format!("；没写成：{}", failed.join("、")));
    }
    out
}

impl LolPlugin {
    /// 这一次选人里我的英雄的一整份（导入和卡片用同一份）：峡谷是 101 这一路的，海斗是 aramhex-data 的。
    /// 返回（这一份，卡片上那块）；没有英雄、数据拿不到时 Err（给页面的原因）。
    pub(crate) async fn my_loadout(
        &self,
        host: &dyn Host,
        session: &Value,
        queue: i64,
    ) -> Result<(Loadout, Value), Value> {
        let stats = self.stats_for(host);
        let names_table = stats.nicknames().await.ok();
        let data = self.loaded_data();
        let short = |id: i64| {
            names_table
                .as_ref()
                .map(|n| n.short(id))
                .or_else(|| {
                    data.as_deref()
                        .and_then(|d| d.champion(id))
                        .map(|c| c.short().to_string())
                })
                .unwrap_or_else(|| format!("英雄 {id}"))
        };
        let d = draft::draft(session);
        let Some(mine) = d.mine else {
            return Err(status("not_now", "no_champion", "你还没选英雄"));
        };
        let statics = stats.statics().await;
        let names = Names {
            statics: &statics,
            data: data.as_deref(),
        };
        let champion = champ_select::champion(mine, short(mine));
        if queue == MAYHEM_QUEUE {
            let hex = stats
                .aramhex()
                .await
                .map_err(|e| status("error", "no_data", format!("海斗的数据拿不到：{e}")))?;
            let loadout = mayhem_loadout(mine, &short(mine), &hex);
            let block = mayhem_mine(champion, &hex, &loadout, &names);
            return Ok((loadout, block));
        }
        let lane = match champ_select::lane_code(&d.position) {
            Some(lane) => Some(lane),
            None => self.main_lane(host, mine).await,
        };
        let Some(lane) = lane else {
            return Err(status(
                "error",
                "no_data",
                "腾讯 101 里没有这个英雄的分路数据",
            ));
        };
        let detail = stats
            .champion(mine, lane)
            .await
            .map_err(|e| status("error", "no_data", format!("统计查不到：{e}")))?;
        let loadout = rift_loadout(mine, &short(mine), Some(lane), &detail);
        let block = rift_mine(champion, Some(lane), d.locked, &detail, &loadout, &names);
        Ok((loadout, block))
    }

    /// `lol.import`（只给页面：选人卡片上的“一键导入”）。
    pub(crate) async fn import_tool(&self, host: &dyn Host, _arguments: &Value) -> ToolResult {
        if !host.source_enabled(SOURCE_CONTROL) {
            return ToolResult::content(status(
                "disabled",
                "control_off",
                "英雄联盟插件的“操作客户端”关着（默认关）：在插件设置里打开以后才能一键导入",
            ));
        }
        if !host.source_enabled(crate::SOURCE_CLIENT) || !host.source_enabled(SOURCE_STATS) {
            return ToolResult::content(status(
                "disabled",
                "sources_off",
                "要“读客户端数据”和“用网上的统计”都开着",
            ));
        }
        let lcu = match self.lcu_or_closed(host) {
            Ok(lcu) => lcu,
            Err(result) => return ToolResult::content(result),
        };
        let Ok(session) = lcu.get(paths::CHAMP_SELECT).await else {
            return ToolResult::content(status("not_now", "not_champ_select", "现在不在选英雄"));
        };
        let flow = lcu.get(paths::SESSION).await.unwrap_or(Value::Null);
        let queue = flow["gameData"]["queue"]["id"].as_i64().unwrap_or(0);
        let (plan, _) = match self.my_loadout(host, &session, queue).await {
            Ok(found) => found,
            Err(result) => return ToolResult::content(result),
        };
        let d = draft::draft(&session);
        let runes = match &plan.runes {
            Some(spec) => loadout::write_runes(&lcu, &plan.name, spec).await,
            None => Written::no_data(),
        };
        let items = loadout::write_items(&lcu, &plan, super::now_ms()).await;
        let spells = match plan.spells {
            Some(spells) => loadout::write_spells(&lcu, spells, d.spells).await,
            None => Written::no_data(),
        };
        let text = summary(&plan, &runes, &items, &spells);
        log::log(&format!("lol: 一键导入（英雄 {}）：{text}", plan.champion));
        let any = runes.ok() || items.ok() || spells.ok();
        let broken = [&runes, &items, &spells].iter().any(|w| {
            matches!(
                w.result,
                loadout::WriteResult::Error | loadout::WriteResult::NoRoom
            )
        });
        let word = match (any, broken) {
            (true, false) => "ok",
            (true, true) => "partial",
            (false, _) => "error",
        };
        ToolResult::content(json!({
            "status": word, "text": text, "champion": plan.champion, "name": plan.name,
            "runes": runes, "items": items, "spells": spells,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stats::qq101::{Build, BuildOption, RunePage, SpellPair};

    fn option(items: &[i64], pick: f64, win: f64) -> BuildOption {
        BuildOption {
            items: items.to_vec(),
            rank: None,
            pick_rate: Some(pick),
            win_rate: Some(win),
        }
    }

    /// 照 `target/lol-stats-probe` 里 101 的亚索中路（16.23）那几行拼的。
    pub(crate) fn yasuo_mid() -> ChampionStats {
        ChampionStats {
            patch: "16.23".into(),
            lane: "MIDDLE".into(),
            good_against: vec![],
            bad_against: vec![],
            build: Build {
                starting: vec![
                    option(&[1086, 2003, 2003], 0.17, 0.47),
                    option(&[1055, 2003], 0.80, 0.51),
                ],
                boots: vec![option(&[3111], 0.34, 0.51), option(&[3047], 0.65, 0.50)],
                core: vec![
                    option(&[3153, 3091, 6333], 0.08, 0.54),
                    option(&[6673, 3031, 6333], 0.18, 0.58),
                ],
                fourth: vec![option(&[6333], 0.24, 0.58), option(&[3026], 0.15, 0.60)],
                fifth: vec![option(&[3026], 0.31, 0.59)],
                sixth: vec![option(&[3143], 0.10, 0.45)],
            },
            runes: vec![
                RunePage {
                    rank: 10,
                    primary_style: Some(8000),
                    secondary_style: Some(8400),
                    primary: vec![8008, 9111, 9104, 8299],
                    secondary: vec![8242, 8473],
                    shards: vec![5005, 5008, 5001],
                    pick_rate: Some(0.0127),
                    win_rate: Some(0.4928),
                    games: 621,
                },
                RunePage {
                    rank: 1,
                    primary_style: Some(8000),
                    secondary_style: Some(8400),
                    primary: vec![8010, 9111, 9104, 8299],
                    secondary: vec![8242, 8473],
                    shards: vec![5005, 5008, 5001],
                    pick_rate: Some(0.4527),
                    win_rate: Some(0.4758),
                    games: 22172,
                },
            ],
            skill_priority: vec!["Q".into(), "E".into(), "W".into()],
            skill_order: ["Q", "E", "W", "Q", "Q", "R", "Q", "E"]
                .map(String::from)
                .to_vec(),
            spells: vec![SpellPair {
                spells: [4, 14],
                win_rate: Some(0.5053),
                pick_rate: Some(0.8583),
            }],
            stale: false,
        }
    }

    #[test]
    fn the_rift_loadout_is_the_most_played_of_each() {
        let l = rift_loadout(157, "亚索", Some("MIDDLE"), &yasuo_mid());
        assert_eq!(l.name, "Kotone 亚索 中路");
        assert_eq!(l.uid, "kotone-157-rift");
        assert_eq!(l.map, 11);
        // 符文：局数最多的那一页（101 的名次不按顺序排，第 10 名排在前面）
        let runes = l.runes.clone().unwrap();
        assert_eq!(runes.perks[0], 8010);
        assert_eq!(runes.perks.len(), 9);
        let titles: Vec<&str> = l.blocks.iter().map(|b| b.title.as_str()).collect();
        assert_eq!(
            titles,
            ["出门", "鞋", "核心三件", "核心三件（另一种）", "后期可选"]
        );
        assert_eq!(l.blocks[0].items, [1055, 2003], "出门：选用率最高的");
        assert_eq!(l.blocks[1].items, [3047, 3111]);
        assert_eq!(l.blocks[2].items, [6673, 3031, 6333]);
        // 后期：第四到第六件各前两种，去掉核心里已经有的 6333
        assert_eq!(l.blocks[4].items, [3026, 3143]);
        assert_eq!(l.spells, Some([4, 14]));

        let names = Names {
            statics: &Statics {
                items: Default::default(),
                runes: Default::default(),
                spells: Default::default(),
            },
            data: None,
        };
        let block = rift_mine(
            json!({"id": 157}),
            Some("MIDDLE"),
            true,
            &yasuo_mid(),
            &l,
            &names,
        );
        assert_eq!(block["lane"], "中路");
        assert_eq!(block["source"], "qq101");
        assert_eq!(block["runes"]["perks"].as_array().unwrap().len(), 9);
        assert_eq!(block["runes"]["styles"][0]["id"], 8000);
        let kinds: Vec<&str> = block["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|g| g["kind"].as_str().unwrap())
            .collect();
        assert_eq!(kinds, ["starting", "boots", "core", "late"]);
        assert_eq!(block["items"][2]["items"][0]["name"], "装备 6673");
        assert_eq!(block["skills"]["priority"], json!(["Q", "E", "W"]));
        assert_eq!(block["skills"]["order"].as_array().unwrap().len(), 6);
        assert_eq!(block["spells"][1]["id"], 14);
        assert_eq!(
            block["importable"],
            json!({"runes": true, "items": true, "spells": true})
        );
    }

    #[test]
    fn the_mayhem_loadout_has_core_builds_and_spells_but_no_runes() {
        let latest = aramhex::parse_latest(
            &json!({"path": "patches/16.23/normal", "patch": "16.23", "files": {}}),
        )
        .unwrap();
        let mut hex = aramhex::Dataset::from_json(
            &latest,
            &json!({"39": {"games": 100000, "wins": 50000, "win_rate": 0.5}}),
            &json!({"1001": {"name": "泰坦的坚决", "rarity": "prismatic"}}),
            &json!({"39": {"1001": {"games": 3000, "wins": 1800}}}),
        );
        hex.add_builds(
            &json!({"39": [{"items": ["3153", "6333", "2517"], "games": 33719, "wins": 19313, "win": 0.5727, "pick": 0.0296},
                           {"items": ["3078", "6333", "3053"], "games": 20000, "wins": 10500, "win": 0.525, "pick": 0.02},
                           {"items": ["3153", "3078", "3053"], "games": 9000, "wins": 4600, "win": 0.511, "pick": 0.01}]}),
            &json!({"39": {"4+32": {"games": 52000, "wins": 26525, "win_rate": 0.5101}}}),
        );
        let l = mayhem_loadout(39, "艾瑞莉娅", &hex);
        assert_eq!(l.name, "Kotone 艾瑞莉娅 海斗");
        assert_eq!(l.map, 12);
        assert!(l.runes.is_none());
        assert_eq!(l.blocks[0].title, "核心三件（第 1 种）");
        assert_eq!(l.spells, Some([4, 32]));
        let statics = Statics {
            items: Default::default(),
            runes: Default::default(),
            spells: Default::default(),
        };
        let names = Names {
            statics: &statics,
            data: None,
        };
        let block = mayhem_mine(json!({"id": 39}), &hex, &l, &names);
        assert_eq!(block["source"], "aramhex");
        // 卡片上列的核心三件就是写进客户端的那几套
        assert_eq!(l.blocks.len(), MAYHEM_CORE_SETS);
        assert_eq!(block["items"].as_array().unwrap().len(), l.blocks.len());
        assert_eq!(block["augments"][0]["name"], "泰坦的坚决");
        assert!(block["runes"].is_null());
        assert_eq!(
            block["importable"],
            json!({"runes": false, "items": true, "spells": true})
        );
        // 没有出装数据的英雄：只有召唤师技能也行，什么都没有时三样都是 false
        let empty = mayhem_loadout(1, "安妮", &hex);
        assert_eq!(
            importable(&empty),
            json!({"runes": false, "items": false, "spells": false})
        );
    }
}
