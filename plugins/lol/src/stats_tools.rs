//! 英雄联盟插件“用网上的统计”的工具（ADR 038 第 3 步，数据开关 `stats`）：
//! - `lol.champion`：一个英雄的强度、克制、出装、符文、加点、召唤师技能（峡谷，腾讯 101）；页面还带这个英雄
//!   技能的名字和图标路径（[`crate::client::abilities`]：客户端开着问客户端，不然 CommunityDragon）；
//!   海斗时是英雄榜上的名次、胜率、搭档和最好的海克斯（腾讯 101 + aramhex-data）；
//! - `lol.tier`：强度榜（峡谷各路 / 海斗）。
//!
//! 给她的是几行字（[`ToolResult::content`]），页面拿完整的数据。数据来源写在结果里（页面上署名）。

use std::sync::{Arc, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use kotone_plugin_api::{Host, ToolResult};
use serde_json::{json, Value};

use crate::client::abilities::{self, Abilities};
use crate::client::gamedata::Ability;
use crate::stats::{self, aramhex, qq101, Stats};
use crate::LolPlugin;

pub const PAGE_CHAMPION: &str = "ui://kotone/lol/champion";
pub const PAGE_TIER: &str = "ui://kotone/lol/tier";
/// “用网上的统计”开关（plugin.toml 的 sources）。
pub const SOURCE_STATS: &str = "stats";
/// OP.GG 的数据开关（ADR 043 D9：默认开；用法照 issue #26 答应的，见 [`stats::opgg`]）。
pub const SOURCE_OPGG: &str = "opgg";

/// 来源标签（ADR 043 D8）：同一块内容一次只用一个数据源，不把不同来源的数字混在一张表里。页面上每块一个小标签，
/// 能换的块点标签换，玩家选的记在插件设置 `plugins.lol.settings.sourceFor.<块>` 里（[`source_tool`]）。
/// 能换的只有两块：峡谷的英雄数据（腾讯 101 / OP.GG）、海斗的海克斯（aramhex-data / OP.GG）；
/// 强度榜、海斗英雄榜只有腾讯 101，这局、战绩、单局详情只有客户端（标签只写出处）。
pub const KIND_RIFT: &str = "rift";
pub const KIND_MAYHEM_AUGMENTS: &str = "mayhemAugments";
/// 数据源的 id（页面上的名字在 messages 里：`plugin_lol_source_tag_<id>`）。
pub const FROM_QQ101: &str = "qq101";
pub const FROM_OPGG: &str = "opgg";
pub const FROM_ARAMHEX: &str = "aramhex";
/// 设置里放玩家选的来源的那一项。
const SETTING_SOURCE_FOR: &str = "sourceFor";

/// 这一块能用哪些来源，第一个是默认（国服的峡谷用腾讯 101，海斗的海克斯用 aramhex-data，ADR 043 D8）。
fn source_options(kind: &str) -> Option<&'static [&'static str]> {
    match kind {
        KIND_RIFT => Some(&[FROM_QQ101, FROM_OPGG]),
        KIND_MAYHEM_AUGMENTS => Some(&[FROM_ARAMHEX, FROM_OPGG]),
        _ => None,
    }
}

/// 玩家给这一块选的来源（插件设置里没写、写的不认得时是默认）。
pub fn chosen_source(settings: &Value, kind: &str) -> &'static str {
    let options = source_options(kind).unwrap_or(&[FROM_QQ101]);
    let picked = settings[SETTING_SOURCE_FOR][kind].as_str().unwrap_or("");
    options
        .iter()
        .find(|o| **o == picked)
        .copied()
        .unwrap_or(options[0])
}

/// 这一次用哪个来源：她在参数里写了就用她的，不然用玩家选的；OP.GG 的开关关着时退回默认。
fn source_for(host: &dyn Host, arguments: &Value, kind: &str) -> &'static str {
    let options = source_options(kind).unwrap_or(&[FROM_QQ101]);
    let asked = arguments.get("source").and_then(Value::as_str);
    let wanted = asked
        .and_then(|a| options.iter().find(|o| **o == a).copied())
        .unwrap_or_else(|| chosen_source(&host.settings(), kind));
    if wanted == FROM_OPGG && !host.source_enabled(SOURCE_OPGG) {
        options[0]
    } else {
        wanted
    }
}

/// 页面上这一块的来源标签：现在用的、能换成哪些（OP.GG 的开关关着时标成用不了）。
fn source_tag(host: &dyn Host, kind: &str, current: &str) -> Value {
    let options: Vec<Value> = source_options(kind)
        .unwrap_or(&[])
        .iter()
        .map(|id| json!({"id": id, "available": *id != FROM_OPGG || host.source_enabled(SOURCE_OPGG)}))
        .collect();
    json!({"kind": kind, "id": current, "options": options})
}

/// `lol.source`（只给页面）：记下玩家给一块选的来源。整份 `sourceFor` 一起写（别的块选的不丢）。
pub(crate) fn source_tool(host: &dyn Host, arguments: &Value) -> ToolResult {
    let kind = arguments.get("kind").and_then(Value::as_str).unwrap_or("");
    let source = arguments
        .get("source")
        .and_then(Value::as_str)
        .unwrap_or("");
    let Some(options) = source_options(kind) else {
        return ToolResult::content(status("error", "kind 只能是 rift 或 mayhemAugments"));
    };
    let Some(source) = options.iter().find(|o| **o == source) else {
        return ToolResult::content(status(
            "error",
            format!("{kind} 只能用这些来源：{}", options.join("、")),
        ));
    };
    let mut picked = host.settings()[SETTING_SOURCE_FOR]
        .as_object()
        .cloned()
        .unwrap_or_default();
    picked.insert(kind.to_string(), json!(source));
    match host.update_settings(json!({ SETTING_SOURCE_FOR: picked })) {
        Ok(()) => ToolResult::content(json!({"status": "ok", "kind": kind, "source": source})),
        // 别的 MCP 宿主里改不了 Kotone 的设置：这一次照样换，只是不记
        Err(e) => ToolResult::content(status("not_saved", format!("记不下来：{e}"))),
    }
}
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

    /// 英雄技能（英雄页加点那块的图，[`abilities`]）：内存 → 客户端（连着时）→ 磁盘缓存 → CommunityDragon，
    /// 客户端、CommunityDragon 各最多等 2 秒（和腾讯 101 的请求一起等）；拿不到是空的，页面显示字母格子。
    /// 只给页面，给她的字不用它。
    async fn champion_abilities(&self, host: &dyn Host, id: i64) -> Vec<Ability> {
        static ABILITIES: OnceLock<Abilities> = OnceLock::new();
        let cache = ABILITIES.get_or_init(|| {
            Abilities::new(host.data_dir().join("abilities"), abilities::online_base())
        });
        let lcu = self.connected_lcu(host);
        cache
            .get(lcu.as_ref().map(|l| l as &dyn abilities::Source), id)
            .await
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
            let from = source_for(host, arguments, KIND_MAYHEM_AUGMENTS);
            self.mayhem_champion(host, id, from).await
        } else {
            let lane = arguments
                .get("lane")
                .and_then(Value::as_str)
                .and_then(qq101::lane_code);
            match source_for(host, arguments, KIND_RIFT) {
                FROM_OPGG => self.rift_champion_opgg(host, id, lane).await,
                _ => self.rift_champion(host, id, lane).await,
            }
        }
    }

    /// 峡谷的英雄数据换成 OP.GG（全球数据，ADR 043 D8）：一次 `lol_get_champion_analysis`，结果原样给页面和她
    /// （OP.GG 的回复是一种紧凑的文字，不是表）。分路没写时按腾讯 101 的强度榜找这个英雄最常走的路（只用来问 OP.GG，
    /// 页面上不放 101 的数字）。
    async fn rift_champion_opgg(
        &self,
        host: &dyn Host,
        id: i64,
        lane: Option<&'static str>,
    ) -> ToolResult {
        let lane = match lane {
            Some(lane) => lane,
            None => self.main_lane(host, id).await.unwrap_or("TOP"),
        };
        let stats = self.stats_for(host);
        let (names, skills) = tokio::join!(self.names(host), self.champion_abilities(host, id));
        let names = names.as_deref();
        let me = names
            .map(|n| n.short(id))
            .unwrap_or_else(|| format!("英雄 {id}"));
        let lane_name = qq101::lane_name(lane);
        let alias = names
            .and_then(|n| n.aliases.get(&id).cloned())
            .or_else(|| {
                self.loaded_data()
                    .and_then(|d| d.champion(id).map(|c| c.alias.clone()))
            })
            .unwrap_or_default();
        let fetched = if alias.is_empty() {
            Err("不知道这个英雄的英文名，问不了 OP.GG".to_string())
        } else {
            stats
                .opgg(
                    "lol_get_champion_analysis",
                    json!({
                        "champion": stats::opgg::champion_key(&alias),
                        "position": stats::opgg::position(lane),
                        "game_mode": "ranked",
                        "lang": "zh_CN",
                    }),
                    &host.app_version(),
                )
                .await
        };
        let (text, block, ok) = match fetched {
            Ok(f) => {
                let short: String = f.value.chars().take(1500).collect();
                (
                    format!(
                        "{me} {lane_name}（{}）：{short}\n数据：{}",
                        stats::opgg::ATTRIBUTION,
                        stats::opgg::ATTRIBUTION
                    ),
                    json!({"text": f.value, "attribution": stats::opgg::ATTRIBUTION,
                           "url": stats::opgg::HOME, "stale": f.stale}),
                    true,
                )
            }
            Err(e) => (
                format!("OP.GG 查不到：{e}"),
                json!({"error": e, "attribution": stats::opgg::ATTRIBUTION, "url": stats::opgg::HOME}),
                false,
            ),
        };
        let empty: Vec<Value> = Vec::new();
        let page = json!({
            "mode": "rift",
            "champion": Self::champion_json(names, id),
            "lane": lane,
            "laneName": lane_name,
            "patch": "",
            "date": "",
            "tier": Value::Null,
            "goodAgainst": empty,
            "badAgainst": empty,
            "build": {"starting": empty, "boots": empty, "core": empty, "fourth": empty, "fifth": empty, "sixth": empty},
            "runes": empty,
            "skillPriority": empty,
            "skillOrder": empty,
            "abilities": skills,
            "spells": empty,
            "sources": [{"name": stats::opgg::ATTRIBUTION}],
            "stale": block["stale"].as_bool().unwrap_or(false),
            "opgg": block,
            "source": source_tag(host, KIND_RIFT, FROM_OPGG),
        });
        let status_word = if ok { "ok" } else { "error" };
        ToolResult::content(json!({"status": status_word, "text": text}))
            .with_page(PAGE_CHAMPION, page)
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
        let (detail, tier, statics, names, skills) = tokio::join!(
            stats.champion(id, lane),
            stats.tier_list(lane),
            stats.statics(),
            self.names(host),
            self.champion_abilities(host, id)
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
        // OP.GG 不再接在后面（ADR 043 D8：一块只用一个来源）；要全球数据就换来源（[`Self::rift_champion_opgg`]）
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
            // 被动和 Q W E R 的名字、图标路径（拿不到时空列表，页面显示字母格子）
            "abilities": skills,
            "spells": detail.spells.iter().map(|p| json!({
                "spells": p.spells.iter().map(|x| json!({"id": x, "name": statics.spell(*x), "icon": spell_icon(*x)})).collect::<Vec<_>>(),
                "winRate": p.win_rate, "pickRate": p.pick_rate,
            })).collect::<Vec<_>>(),
            "sources": [{"name": TENCENT, "date": date}],
            "stale": detail.stale,
            "opgg": Value::Null,
            "source": source_tag(host, KIND_RIFT, FROM_QQ101),
        });
        ToolResult::content(json!({"status": "ok", "text": lines.join("\n")}))
            .with_page(PAGE_CHAMPION, page)
    }

    /// 海斗的英雄：英雄榜（腾讯 101）+ 最好的海克斯（`from`：aramhex-data，或者换成 OP.GG 的
    /// `lol_list_aram_augments`，ADR 043 D8）。只请求用得着的那一个。
    async fn mayhem_champion(&self, host: &dyn Host, id: i64, from: &'static str) -> ToolResult {
        let stats = self.stats_for(host);
        let days = recent_days();
        let opgg_wanted = from == FROM_OPGG;
        let version = host.app_version();
        let (table, hex, opgg, names) = tokio::join!(
            stats.mayhem_champions(&days),
            async {
                if opgg_wanted {
                    Err("换成了 OP.GG".to_string())
                } else {
                    stats.aramhex().await
                }
            },
            async {
                if !opgg_wanted {
                    return None;
                }
                Some(
                    stats
                        .opgg(
                            "lol_list_aram_augments",
                            json!({
                                "champion_id": id,
                                "lang": "zh_CN",
                                "desired_output_fields": ["data.augments[].{id,name,tier,performance,popular}"],
                            }),
                            &version,
                        )
                        .await,
                )
            },
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
        // 海克斯换成 OP.GG：原样给（文字），出处写 OP.GG
        let mut augments_opgg = Value::Null;
        match opgg {
            Some(Ok(f)) => {
                let short: String = f.value.chars().take(800).collect();
                lines.push(format!(
                    "最好的海克斯（{}）：{short}",
                    stats::opgg::ATTRIBUTION
                ));
                sources.push(json!({"name": stats::opgg::ATTRIBUTION}));
                augments_opgg = json!({"text": f.value, "attribution": stats::opgg::ATTRIBUTION,
                                       "url": stats::opgg::HOME, "stale": f.stale});
            }
            Some(Err(e)) => {
                lines.push(format!("OP.GG 的海克斯查不到：{e}"));
                augments_opgg = json!({"error": e, "attribution": stats::opgg::ATTRIBUTION,
                                       "url": stats::opgg::HOME});
            }
            None => {}
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
            "augmentsOpgg": augments_opgg,
            "source": source_tag(host, KIND_MAYHEM_AUGMENTS, from),
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

    /// 记得住玩家选的来源（插件设置 `sourceFor`），只有设置、没有别的状态。
    struct SettingsHost {
        settings: std::sync::Mutex<Value>,
        opgg: bool,
    }

    impl Host for SettingsHost {
        fn plugin(&self) -> &str {
            "lol"
        }
        fn enabled(&self) -> bool {
            true
        }
        fn source_enabled(&self, source: &str) -> bool {
            source != SOURCE_OPGG || self.opgg
        }
        fn settings(&self) -> Value {
            self.settings.lock().unwrap().clone()
        }
        fn update_settings(&self, patch: Value) -> Result<(), String> {
            // 和宿主一样：对象按字段合并（这里只合一层，测试 source_tool 自己把整份 sourceFor 写全）
            let mut settings = self.settings.lock().unwrap();
            for (k, v) in patch.as_object().unwrap() {
                settings[k] = v.clone();
            }
            Ok(())
        }
        fn data_dir(&self) -> std::path::PathBuf {
            std::path::PathBuf::new()
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

    /// OP.GG 默认开（ADR 043 D9，玩家决定“直接用”）。
    #[test]
    fn opgg_is_on_by_default() {
        let m = crate::manifest();
        assert!(m.source(SOURCE_OPGG).unwrap().enabled_by_default);
    }

    /// ADR 043 D8：默认国服的峡谷用腾讯 101、海斗的海克斯用 aramhex-data；点标签换了会记住，两块各记各的；
    /// 她在参数里写了来源就用她的；OP.GG 的开关关着时退回默认；不认得的来源不收。
    #[test]
    fn source_tags_switch_and_remember() {
        let host = SettingsHost {
            settings: std::sync::Mutex::new(json!({"autoAccept": true})),
            opgg: true,
        };
        assert_eq!(source_for(&host, &json!({}), KIND_RIFT), FROM_QQ101);
        assert_eq!(
            source_for(&host, &json!({}), KIND_MAYHEM_AUGMENTS),
            FROM_ARAMHEX
        );
        let tag = source_tag(&host, KIND_RIFT, FROM_QQ101);
        assert_eq!(
            tag,
            json!({"kind": "rift", "id": "qq101", "options": [
                {"id": "qq101", "available": true}, {"id": "opgg", "available": true}]})
        );

        let ok = source_tool(&host, &json!({"kind": "rift", "source": "opgg"}));
        assert_eq!(ok.content["status"], "ok");
        let ok = source_tool(&host, &json!({"kind": "mayhemAugments", "source": "opgg"}));
        assert_eq!(ok.content["status"], "ok");
        let saved = host.settings();
        assert_eq!(
            saved["sourceFor"],
            json!({"rift": "opgg", "mayhemAugments": "opgg"})
        );
        assert_eq!(saved["autoAccept"], true, "别的设置不动");
        assert_eq!(source_for(&host, &json!({}), KIND_RIFT), FROM_OPGG);
        // 她写了来源：用她的（这一次），不改玩家选的
        assert_eq!(
            source_for(&host, &json!({"source": "qq101"}), KIND_RIFT),
            FROM_QQ101
        );
        assert_eq!(chosen_source(&host.settings(), KIND_RIFT), FROM_OPGG);
        // 不认得的
        let bad = source_tool(&host, &json!({"kind": "rift", "source": "aramhex"}));
        assert_eq!(bad.content["status"], "error");
        let bad = source_tool(&host, &json!({"kind": "tier", "source": "qq101"}));
        assert_eq!(bad.content["status"], "error");
        assert_eq!(
            chosen_source(&json!({"sourceFor": {"rift": "x"}}), KIND_RIFT),
            FROM_QQ101
        );

        // OP.GG 关着：退回默认，标签上标成用不了
        let off = SettingsHost {
            settings: std::sync::Mutex::new(host.settings()),
            opgg: false,
        };
        assert_eq!(source_for(&off, &json!({}), KIND_RIFT), FROM_QQ101);
        assert_eq!(
            source_for(&off, &json!({"source": "opgg"}), KIND_MAYHEM_AUGMENTS),
            FROM_ARAMHEX
        );
        assert_eq!(
            source_tag(&off, KIND_RIFT, FROM_QQ101)["options"][1]["available"],
            false
        );
    }
}
