//! 英雄联盟插件“用网上的统计”的工具（ADR 038 第 3 步，数据开关 `stats`）：
//! - `lol.champion`：一个英雄的强度、克制、出装、符文、加点、召唤师技能（峡谷，腾讯 101）；
//!   海斗时是英雄榜上的名次、胜率、搭档和最好的海克斯（腾讯 101 + aramhex-data）；
//! - `lol.tier`：强度榜（峡谷各路 / 海斗）。
//!
//! 给她的是几行字（[`ToolResult::content`]），页面拿完整的数据。数据来源写在结果里（页面上署名）。

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use kotone_plugin_api::{Host, ToolResult};
use serde_json::{json, Value};

use crate::stats::{self, aramhex, qq101, Stats};
use crate::LolPlugin;

pub const PAGE_CHAMPION: &str = "ui://kotone/lol/champion";
pub const PAGE_TIER: &str = "ui://kotone/lol/tier";
/// “用网上的统计”开关（plugin.toml 的 sources）。
pub const SOURCE_STATS: &str = "stats";
/// OP.GG（默认关，等 OP.GG 回复 issue #26）。
pub const SOURCE_OPGG: &str = "opgg";
/// 海克斯大乱斗的队列。
const MAYHEM_QUEUE: i64 = 2400;
/// 海克斯评分至少要多少局才列进“最好的海克斯”。
const MIN_AUGMENT_GAMES: i64 = 300;

const TENCENT: &str = "腾讯 101（掌盟，国服）";

fn status(status: &str, text: impl Into<String>) -> Value {
    json!({"status": status, "text": text.into()})
}

fn pct(v: Option<f64>) -> String {
    v.map(|x| format!("{:.1}%", x * 100.0))
        .unwrap_or_else(|| "—".into())
}

/// 北京时间往前数 `days` 天的日期 `YYYYMMDD`（腾讯的海斗数据按天出）。
pub fn beijing_date(days_back: u64) -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
        + 8 * 3600;
    let day = (secs / 86_400).saturating_sub(days_back) as i64;
    // Howard Hinnant 的 civil_from_days
    let z = day + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}{m:02}{d:02}")
}

/// 今天往前 7 天（海斗数据当天的常常还没出）。
fn recent_days() -> Vec<String> {
    (0..7).map(beijing_date).collect()
}

impl LolPlugin {
    pub(crate) fn stats_for(&self, host: &dyn Host) -> &Stats {
        self.stats
            .get_or_init(|| Stats::new(host.data_dir().join("stats")))
    }

    /// 玩家说的英雄（名字、称号、外号、英文名）→ 英雄 id。
    async fn champion_id(&self, host: &dyn Host, query: &str) -> Result<i64, Value> {
        if let Ok(names) = self.stats_for(host).nicknames().await {
            if let Some(id) = names.find(query) {
                return Ok(id);
            }
        }
        // 外号表拿不到时用客户端的资料
        if let Some(data) = self.loaded_data() {
            if let Some(c) = data.champions.iter().find(|c| c.matches(query)) {
                return Ok(c.id);
            }
        }
        Err(status("not_found", format!("不知道“{query}”是哪个英雄")))
    }

    async fn names(&self, host: &dyn Host) -> Option<Arc<stats::nicknames::Nicknames>> {
        self.stats_for(host).nicknames().await.ok()
    }

    fn champion_json(names: Option<&stats::nicknames::Nicknames>, id: i64) -> Value {
        match names {
            Some(n) => json!({"id": id, "label": n.label(id), "short": n.short(id)}),
            None => json!({"id": id, "label": format!("英雄 {id}"), "short": format!("英雄 {id}")}),
        }
    }

    /// 这一局是不是海斗（不在对局里时 false）。
    fn in_mayhem(&self) -> bool {
        self.current_queue() == Some(MAYHEM_QUEUE)
    }

    /// `lol.champion`。
    pub(crate) async fn champion_tool(&self, host: &dyn Host, arguments: &Value) -> ToolResult {
        if !host.source_enabled(SOURCE_STATS) {
            return ToolResult::content(status(
                "disabled",
                "英雄联盟插件的“用网上的统计”已关闭（插件 → 英雄联盟 里可以打开）",
            ));
        }
        let Some(query) = arguments
            .get("champion")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|q| !q.is_empty())
        else {
            return ToolResult::content(status("error", "要写 champion（英雄名、称号或外号）"));
        };
        let id = match self.champion_id(host, query).await {
            Ok(id) => id,
            Err(result) => return ToolResult::content(result),
        };
        let mode = arguments.get("mode").and_then(Value::as_str).unwrap_or("");
        let mayhem = match mode {
            "mayhem" => true,
            "rift" => false,
            _ => self.in_mayhem(),
        };
        if mayhem {
            self.mayhem_champion(host, id).await
        } else {
            let lane = arguments
                .get("lane")
                .and_then(Value::as_str)
                .and_then(qq101::lane_code);
            self.rift_champion(host, id, lane).await
        }
    }

    /// 这个英雄最常走的路（五路强度榜里选用率最高的那一路）。
    async fn main_lane(&self, host: &dyn Host, id: i64) -> Option<&'static str> {
        let mut best: Option<(&'static str, f64)> = None;
        for lane in qq101::LANES {
            if let Ok(list) = self.stats_for(host).tier_list(lane).await {
                if let Some(row) = list.value.1.iter().find(|r| r.champion_id == id) {
                    let pick = row.pick_rate.unwrap_or(0.0);
                    if best.map_or(true, |(_, p)| pick > p) {
                        best = Some((lane, pick));
                    }
                }
            }
        }
        best.map(|(lane, _)| lane)
    }

    async fn rift_champion(
        &self,
        host: &dyn Host,
        id: i64,
        lane: Option<&'static str>,
    ) -> ToolResult {
        let lane = match lane {
            Some(lane) => lane,
            None => match self.main_lane(host, id).await {
                Some(lane) => lane,
                None => {
                    return ToolResult::content(status(
                        "not_found",
                        "腾讯的峡谷强度榜里没有这个英雄（或者连不上）",
                    ))
                }
            },
        };
        let stats = self.stats_for(host);
        let (detail, tier, statics, names) = tokio::join!(
            stats.champion(id, lane),
            stats.tier_list(lane),
            stats.statics(),
            self.names(host)
        );
        let detail = match detail {
            Ok(d) => d,
            Err(e) => return ToolResult::content(status("error", format!("统计查不到：{e}"))),
        };
        let names = names.as_deref();
        let (date, rows) = tier.map(|t| t.value).unwrap_or_default();
        let row = rows.iter().find(|r| r.champion_id == id);
        let short = |cid: i64| {
            names
                .map(|n| n.short(cid))
                .unwrap_or_else(|| format!("英雄 {cid}"))
        };
        let me = short(id);
        let lane_name = qq101::lane_name(lane);
        // 给她的几行
        let mut lines = vec![match row {
            Some(r) => format!(
                "{me} {lane_name}（{} 版本，国服全段位）：{}，胜率 {}，选用率 {}，禁用率 {}",
                detail.patch,
                r.tier,
                pct(r.win_rate),
                pct(r.pick_rate),
                pct(r.ban_rate)
            ),
            None => format!("{me} {lane_name}（{} 版本，国服全段位）", detail.patch),
        }];
        let matchups = |list: &[qq101::Matchup]| -> String {
            list.iter()
                .take(5)
                .map(|m| format!("{} {}", short(m.champion_id), pct(m.win_rate)))
                .collect::<Vec<_>>()
                .join("、")
        };
        if !detail.bad_against.is_empty() {
            lines.push(format!(
                "难打（它对这些英雄胜率低）：{}",
                matchups(&detail.bad_against)
            ));
        }
        if !detail.good_against.is_empty() {
            lines.push(format!("好打：{}", matchups(&detail.good_against)));
        }
        if let Some(core) = detail.build.core.first() {
            let items: Vec<String> = core.items.iter().map(|i| statics.item(*i)).collect();
            lines.push(format!(
                "核心三件：{}（胜率 {}）",
                items.join(" → "),
                pct(core.win_rate)
            ));
        }
        if let Some(boots) = detail.build.boots.first() {
            let items: Vec<String> = boots.items.iter().map(|i| statics.item(*i)).collect();
            lines.push(format!("鞋：{}", items.join("、")));
        }
        if let Some(page) = detail.runes.first() {
            if let Some(keystone) = page.primary.first() {
                lines.push(format!(
                    "符文：{}（胜率 {}）",
                    statics.rune(*keystone),
                    pct(page.win_rate)
                ));
            }
        }
        if !detail.skill_priority.is_empty() {
            lines.push(format!("加点：主升 {}", detail.skill_priority.join(" > ")));
        }
        if let Some(pair) = detail.spells.first() {
            lines.push(format!(
                "召唤师技能：{} + {}",
                statics.spell(pair.spells[0]),
                statics.spell(pair.spells[1])
            ));
        }
        // OP.GG（全球数据，开关默认关）
        let mut opgg_block = Value::Null;
        if host.source_enabled(SOURCE_OPGG) {
            if let Some(alias) = names.and_then(|n| n.aliases.get(&id)) {
                let args = json!({
                    "champion": stats::opgg::champion_key(alias),
                    "position": stats::opgg::position(lane),
                    "game_mode": "ranked",
                    "lang": "zh_CN",
                });
                match stats
                    .opgg("lol_get_champion_analysis", args, &host.app_version())
                    .await
                {
                    Ok(text) => {
                        let short: String = text.value.chars().take(1200).collect();
                        lines.push(format!("{}：{short}", stats::opgg::ATTRIBUTION));
                        opgg_block = json!({"text": text.value, "attribution": stats::opgg::ATTRIBUTION,
                                            "url": stats::opgg::HOME, "stale": text.stale});
                    }
                    Err(e) => {
                        opgg_block = json!({"error": e, "attribution": stats::opgg::ATTRIBUTION, "url": stats::opgg::HOME})
                    }
                }
            }
        }
        lines.push(format!(
            "数据：{TENCENT}{}",
            if detail.stale {
                "，网上没连上，用的是之前存的"
            } else {
                ""
            }
        ));
        let champion = |cid: i64| Self::champion_json(names, cid);
        // 图（页面用，ADR 039 D6）：客户端资料读过才有路径，没读过是空字符串（页面显示名字的第一个字）
        let data = self.loaded_data();
        let data = data.as_deref();
        let item_icon = |i: i64| {
            data.and_then(|d| d.item_icons.get(&i))
                .cloned()
                .unwrap_or_default()
        };
        let rune_icon = |i: i64| {
            data.and_then(|d| d.perks.get(&i).or_else(|| d.perk_styles.get(&i)))
                .map(|p| p.icon.clone())
                .unwrap_or_default()
        };
        let spell_icon = |i: i64| {
            data.and_then(|d| d.spell_by_id(i))
                .map(|s| s.icon.clone())
                .unwrap_or_default()
        };
        let named_items = |ids: &[i64]| -> Vec<Value> {
            ids.iter()
                .map(|i| json!({"id": i, "name": statics.item(*i), "icon": item_icon(*i)}))
                .collect()
        };
        let options = |list: &[qq101::BuildOption]| -> Vec<Value> {
            list.iter()
                .take(4)
                .map(|o| json!({"items": named_items(&o.items), "winRate": o.win_rate, "pickRate": o.pick_rate}))
                .collect()
        };
        let page = json!({
            "mode": "rift",
            "champion": champion(id),
            "lane": lane,
            "laneName": lane_name,
            "patch": detail.patch,
            "date": date,
            "tier": row,
            "goodAgainst": detail.good_against.iter().map(|m| json!({"champion": champion(m.champion_id), "winRate": m.win_rate})).collect::<Vec<_>>(),
            "badAgainst": detail.bad_against.iter().map(|m| json!({"champion": champion(m.champion_id), "winRate": m.win_rate})).collect::<Vec<_>>(),
            "build": {
                "starting": options(&detail.build.starting),
                "boots": options(&detail.build.boots),
                "core": options(&detail.build.core),
                "fourth": options(&detail.build.fourth),
                "fifth": options(&detail.build.fifth),
                "sixth": options(&detail.build.sixth),
            },
            "runes": detail.runes.iter().take(3).map(|r| json!({
                "primary": r.primary.iter().map(|x| json!({"id": x, "name": statics.rune(*x), "icon": rune_icon(*x)})).collect::<Vec<_>>(),
                "secondary": r.secondary.iter().map(|x| json!({"id": x, "name": statics.rune(*x), "icon": rune_icon(*x)})).collect::<Vec<_>>(),
                "shards": r.shards.iter().map(|x| json!({"id": x, "name": statics.rune(*x), "icon": rune_icon(*x)})).collect::<Vec<_>>(),
                "winRate": r.win_rate, "pickRate": r.pick_rate, "games": r.games,
            })).collect::<Vec<_>>(),
            "skillPriority": detail.skill_priority,
            "skillOrder": detail.skill_order,
            "spells": detail.spells.iter().map(|p| json!({
                "spells": p.spells.iter().map(|x| json!({"id": x, "name": statics.spell(*x), "icon": spell_icon(*x)})).collect::<Vec<_>>(),
                "winRate": p.win_rate, "pickRate": p.pick_rate,
            })).collect::<Vec<_>>(),
                        "sources": [{"name": TENCENT, "date": date}],
            "stale": detail.stale,
            "opgg": opgg_block,
        });
        ToolResult::content(json!({"status": "ok", "text": lines.join("\n")}))
            .with_page(PAGE_CHAMPION, page)
    }

    async fn mayhem_champion(&self, host: &dyn Host, id: i64) -> ToolResult {
        let stats = self.stats_for(host);
        let days = recent_days();
        let (table, hex, names) = tokio::join!(
            stats.mayhem_champions(&days),
            stats.aramhex(),
            self.names(host)
        );
        let names = names.as_deref();
        let short = |cid: i64| {
            names
                .map(|n| n.short(cid))
                .unwrap_or_else(|| format!("英雄 {cid}"))
        };
        let champion = |cid: i64| Self::champion_json(names, cid);
        let me = short(id);
        let mut lines = Vec::new();
        let mut sources = Vec::new();
        let row = match &table {
            Ok(t) => {
                sources.push(json!({"name": TENCENT, "date": t.value.0}));
                t.value.1.iter().find(|r| r.champion_id == id).cloned()
            }
            Err(_) => None,
        };
        if let Some(r) = &row {
            lines.push(format!(
                "{me} 在海斗：第 {} 名，胜率 {}，选用率 {}",
                r.rank.map(|x| x.to_string()).unwrap_or_else(|| "—".into()),
                pct(r.win_rate),
                pct(r.pick_rate)
            ));
            if !r.partners.is_empty() {
                let partners: Vec<String> = r.partners.iter().take(3).map(|p| short(*p)).collect();
                lines.push(format!("最搭的队友：{}", partners.join("、")));
            }
        }
        let mut augments = Vec::new();
        let mut attribution = None;
        let data = self.loaded_data();
        if let Ok(d) = &hex {
            attribution = Some(aramhex::attribution(d));
            for (aid, s) in d.best_for(id, 10, MIN_AUGMENT_GAMES) {
                let a = d.augments.get(&aid);
                augments.push(json!({
                    "id": aid,
                    "name": a.map(|a| a.name.as_str()).unwrap_or(""),
                    "rarity": a.map(|a| a.rarity.as_str()).unwrap_or(""),
                    // 图标：客户端资料里同一个 id 的海克斯（读过才有）
                    "icon": data.as_ref().and_then(|g| g.augments.get(&aid)).map(|g| g.icon.as_str()).unwrap_or(""),
                    "games": s.games, "winRate": s.win_rate, "delta": s.delta, "lowSample": s.low_sample,
                }));
            }
            if !augments.is_empty() {
                let top: Vec<String> = augments
                    .iter()
                    .take(5)
                    .map(|a| {
                        format!(
                            "{}（{:+.1}）",
                            a["name"].as_str().unwrap_or(""),
                            a["delta"].as_f64().unwrap_or(0.0)
                        )
                    })
                    .collect();
                lines.push(format!(
                    "最好的海克斯（比这个英雄平均胜率高几个百分点）：{}",
                    top.join("、")
                ));
            }
            sources.push(
                json!({"name": aramhex::REPO, "license": aramhex::LICENSE, "date": d.snapshot_at}),
            );
        }
        if lines.is_empty() {
            let why = [table.err(), hex.err()]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join("；");
            return ToolResult::content(status("error", format!("海斗的统计查不到：{why}")));
        }
        lines.push(format!(
            "数据：{}",
            sources
                .iter()
                .filter_map(|s| s["name"].as_str())
                .collect::<Vec<_>>()
                .join("、")
        ));
        let page = json!({
            "mode": "mayhem",
            "champion": champion(id),
            "rank": row.as_ref().and_then(|r| r.rank),
            "winRate": row.as_ref().and_then(|r| r.win_rate),
            "pickRate": row.as_ref().and_then(|r| r.pick_rate),
            "partners": row.as_ref().map(|r| r.partners.iter().take(5).map(|p| champion(*p)).collect::<Vec<_>>()).unwrap_or_default(),
            "augments": augments,
            "attribution": attribution,
            "sources": sources,
        });
        ToolResult::content(json!({"status": "ok", "text": lines.join("\n")}))
            .with_page(PAGE_CHAMPION, page)
    }

    /// `lol.tier`。
    pub(crate) async fn tier_tool(&self, host: &dyn Host, arguments: &Value) -> ToolResult {
        if !host.source_enabled(SOURCE_STATS) {
            return ToolResult::content(status(
                "disabled",
                "英雄联盟插件的“用网上的统计”已关闭（插件 → 英雄联盟 里可以打开）",
            ));
        }
        let mode = arguments.get("mode").and_then(Value::as_str).unwrap_or("");
        let mayhem = match mode {
            "mayhem" => true,
            "rift" => false,
            _ => self.in_mayhem(),
        };
        let names = self.names(host).await;
        let names = names.as_deref();
        let short = |cid: i64| {
            names
                .map(|n| n.short(cid))
                .unwrap_or_else(|| format!("英雄 {cid}"))
        };
        let champion = |cid: i64| Self::champion_json(names, cid);
        if mayhem {
            let table = match self.stats_for(host).mayhem_champions(&recent_days()).await {
                Ok(t) => t,
                Err(e) => {
                    return ToolResult::content(status("error", format!("海斗英雄榜查不到：{e}")))
                }
            };
            let (date, rows) = table.value;
            let top: Vec<String> = rows
                .iter()
                .take(10)
                .map(|r| format!("{} {}", short(r.champion_id), pct(r.win_rate)))
                .collect();
            let text = format!("海斗英雄榜（{TENCENT}，{date}）前 10：{}", top.join("、"));
            let page = json!({
                "mode": "mayhem",
                "date": date,
                "rows": rows.iter().map(|r| json!({
                    "champion": champion(r.champion_id), "rank": r.rank, "rankChange": r.rank_change,
                    "winRate": r.win_rate, "pickRate": r.pick_rate,
                })).collect::<Vec<_>>(),
                "sources": [{"name": TENCENT, "date": date}],
            });
            return ToolResult::content(json!({"status": "ok", "text": text}))
                .with_page(PAGE_TIER, page);
        }
        let lanes: Vec<&'static str> = match arguments
            .get("lane")
            .and_then(Value::as_str)
            .and_then(qq101::lane_code)
        {
            Some(lane) => vec![lane],
            None => qq101::LANES.to_vec(),
        };
        let mut lines = Vec::new();
        let mut tables = serde_json::Map::new();
        let mut date = String::new();
        for lane in &lanes {
            let Ok(list) = self.stats_for(host).tier_list(lane).await else {
                continue;
            };
            let (d, rows) = list.value;
            date = d;
            let take = if lanes.len() == 1 { 10 } else { 3 };
            let top: Vec<String> = rows
                .iter()
                .take(take)
                .map(|r| format!("{} {}（{}）", short(r.champion_id), r.tier, pct(r.win_rate)))
                .collect();
            lines.push(format!("{}：{}", qq101::lane_name(lane), top.join("、")));
            tables.insert(
                lane.to_string(),
                Value::Array(
                    rows.iter()
                        .map(|r| json!({
                            "champion": champion(r.champion_id), "tier": r.tier, "rank": r.rank,
                            "winRate": r.win_rate, "pickRate": r.pick_rate, "banRate": r.ban_rate,
                            "counters": r.counters.iter().take(3).map(|c| champion(*c)).collect::<Vec<_>>(),
                        }))
                        .collect(),
                ),
            );
        }
        if lines.is_empty() {
            return ToolResult::content(status(
                "error",
                "峡谷强度榜查不到（腾讯的接口连不上或改了）",
            ));
        }
        lines.push(format!("数据：{TENCENT}，{date}"));
        let page = json!({
            "mode": "rift",
            "date": date,
            "lanes": tables,
            "laneNames": lanes.iter().map(|l| json!({"id": l, "name": qq101::lane_name(l)})).collect::<Vec<_>>(),
            "sources": [{"name": TENCENT, "date": date}],
        });
        ToolResult::content(json!({"status": "ok", "text": lines.join("\n")}))
            .with_page(PAGE_TIER, page)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_count_back_in_beijing_time() {
        let today = beijing_date(0);
        assert_eq!(today.len(), 8);
        assert!(today.starts_with("20"));
        assert_ne!(beijing_date(1), today);
        assert_eq!(recent_days().len(), 7);
        assert_eq!(pct(Some(0.5)), "50.0%");
        assert_eq!(pct(None), "—");
    }
}
