//! 腾讯 101 / 掌盟的统计接口（`mlol.qt.qq.com`，国服的峡谷、大乱斗、海斗）：没有文档，返回的 JSON 里套着一段
//! JSON 字符串，字符串里是用 `#`、`_`、`,` 拼的记录。解析照 LeagueAkari（MIT，`dev` 分支
//! `src/shared/data-adapter/champion-data/qq101-protocol.ts`，2026-11-24）逐字段移植：读不懂的字段跳过，不报错
//! （接口随时可能改，照 Akari 容错）。
//!
//! 只有解析（纯函数，有单测）；请求和缓存在 [`super`]。

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const BASE: &str = "https://mlol.qt.qq.com";
pub const RIFT: &str = "/go/battle_info/odp_proxy/lol_101strategy";
pub const MAYHEM: &str = "/go/battle_info/odp_proxy";
pub const VERSIONS: &str = "/go/database/versionlist?zone=lol&from=h5";

/// 峡谷的分路（101 接口要大写、要写全：`MIDDLE` / `BOTTOM`，`MID` / `ADC` 返回空，2026-11-24 实测）。
pub const LANES: &[&str] = &["TOP", "JUNGLE", "MIDDLE", "BOTTOM", "SUPPORT"];

/// 全部段位。
pub const ALL_TIERS: u32 = 255;

/// 外层信封：`code` 不是 0 就是出错。
fn envelope_ok(v: &Value) -> bool {
    v.get("code").and_then(Value::as_i64) == Some(0)
}

fn payload_string(v: &Value) -> Option<String> {
    match v {
        Value::Null => None,
        Value::String(s) => Some(s.clone()),
        other => Some(other.to_string()),
    }
}

/// 套在里面的那段 JSON 字符串（照 Akari `extractQq101Field`）。
pub fn field(v: &Value, field_id: &str) -> Option<String> {
    let data = v.get("data")?;
    if let Value::String(s) = data {
        return Some(s.clone());
    }
    if let Value::Object(map) = data {
        for key in ["result", "augmentlist", "championid_data"] {
            if let Some(s) = map.get(key).and_then(payload_string) {
                return Some(s);
            }
        }
        if let Some(Value::Object(fields)) = map.get("_fieldValues") {
            if !field_id.is_empty() {
                if let Some(s) = fields.get(&format!("R{field_id}")).and_then(payload_string) {
                    return Some(s);
                }
            }
            return fields.values().next().and_then(payload_string);
        }
    }
    v.get("result").and_then(payload_string)
}

/// 里面那段 JSON。信封不对、没有、不是 JSON 都是 None。
pub fn inner(v: &Value, field_id: &str) -> Option<Value> {
    if !envelope_ok(v) {
        return None;
    }
    serde_json::from_str(&field(v, field_id)?).ok()
}

fn records<'a>(value: Option<&'a str>, sep: &'a [char]) -> impl Iterator<Item = &'a str> {
    value.unwrap_or("").split(sep).filter(|s| !s.is_empty())
}

fn int(s: Option<&str>) -> Option<i64> {
    let s = s?.trim();
    if s.is_empty() || s == "-1" {
        return None;
    }
    s.parse::<f64>()
        .ok()
        .filter(|f| f.is_finite())
        .map(|f| f.trunc() as i64)
}

fn num(s: Option<&str>) -> Option<f64> {
    let s = s?.trim();
    if s.is_empty() || s == "-1" {
        return None;
    }
    s.parse::<f64>().ok().filter(|f| f.is_finite())
}

/// 百分数（`50.45`）→ 0..1。
fn ratio(s: Option<&str>) -> Option<f64> {
    num(s).map(|p| (p / 100.0 * 1e8).round() / 1e8)
}

fn ids(s: Option<&str>) -> Vec<i64> {
    s.unwrap_or("")
        .split(',')
        .filter_map(|x| int(Some(x)))
        .filter(|x| *x > 0)
        .collect()
}

fn text<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

/// 版本列表（新的在前）：`[(id, "16.23")]`。
pub fn parse_versions(v: &Value) -> Vec<(i64, String)> {
    if !envelope_ok(v) {
        return Vec::new();
    }
    v.get("data")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|p| {
                    let id = p.get("id").and_then(|x| match x {
                        Value::String(s) => s.parse().ok(),
                        other => other.as_i64(),
                    })?;
                    Some((id, p.get("name")?.as_str()?.to_string()))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// 强度榜的一行。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TierRow {
    pub rank: Option<i64>,
    pub champion_id: i64,
    /// `T1`…`T5`。
    pub tier: String,
    pub lane: String,
    pub win_rate: Option<f64>,
    pub pick_rate: Option<f64>,
    pub ban_rate: Option<f64>,
    /// 克制它的英雄。
    pub counters: Vec<i64>,
    pub rank_change: Option<i64>,
}

/// 峡谷强度榜（`lol_101strategy`，字段 17960）。
pub fn parse_tier_list(v: &Value) -> (String, Vec<TierRow>) {
    let Some(p) = inner(v, "17960") else {
        return (String::new(), Vec::new());
    };
    let rows = records(text(&p, "datadetails"), &['#'])
        .filter_map(|r| {
            let f: Vec<&str> = r.split('_').collect();
            Some(TierRow {
                rank: int(f.first().copied()),
                champion_id: int(f.get(1).copied())?,
                tier: f.get(2).unwrap_or(&"").to_string(),
                lane: f.get(3).unwrap_or(&"NONE").to_string(),
                win_rate: ratio(f.get(4).copied()),
                pick_rate: ratio(f.get(5).copied()),
                ban_rate: ratio(f.get(6).copied()),
                counters: ids(f.get(7).copied()),
                rank_change: int(f.get(8).copied()),
            })
        })
        .collect();
    (text(&p, "dtstatdate").unwrap_or("").to_string(), rows)
}

/// 对位：这个英雄对某个英雄的胜率。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Matchup {
    pub champion_id: i64,
    pub win_rate: Option<f64>,
}

fn matchups(s: Option<&str>) -> Vec<Matchup> {
    records(s, &['#'])
        .filter_map(|r| {
            let f: Vec<&str> = r.split('_').collect();
            Some(Matchup {
                champion_id: int(f.get(1).copied())?,
                win_rate: ratio(f.get(2).copied()),
            })
        })
        .collect()
}

/// 克制（`_confront`，字段 17968）：（好打的，难打的）。
pub fn parse_matchups(v: &Value) -> (Vec<Matchup>, Vec<Matchup>) {
    let Some(p) = inner(v, "17968") else {
        return (Vec::new(), Vec::new());
    };
    (
        matchups(text(&p, "high_op_details")),
        matchups(text(&p, "low_op_details")),
    )
}

/// 出装的一种选择。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildOption {
    pub items: Vec<i64>,
    pub rank: Option<i64>,
    pub pick_rate: Option<f64>,
    pub win_rate: Option<f64>,
}

fn build_options(s: Option<&str>) -> Vec<BuildOption> {
    if s.map_or(true, |s| s == "-1") {
        return Vec::new();
    }
    records(s, &['#'])
        .map(|r| {
            let f: Vec<&str> = r.split('_').collect();
            BuildOption {
                items: f
                    .first()
                    .unwrap_or(&"")
                    .split(',')
                    .filter_map(|x| int(Some(x)))
                    .collect(),
                rank: int(f.get(1).copied()),
                pick_rate: ratio(f.get(2).copied()),
                win_rate: ratio(f.get(3).copied()),
            }
        })
        .collect()
}

/// 出装（`_build`，字段 18087）：出门、鞋、核心三件、第四到第六件。
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Build {
    pub starting: Vec<BuildOption>,
    pub boots: Vec<BuildOption>,
    pub core: Vec<BuildOption>,
    pub fourth: Vec<BuildOption>,
    pub fifth: Vec<BuildOption>,
    pub sixth: Vec<BuildOption>,
}

pub fn parse_build(v: &Value) -> Build {
    let Some(p) = inner(v, "18087") else {
        return Build::default();
    };
    let slot = |key: &str| build_options(text(&p, key));
    Build {
        starting: slot("starting_details"),
        boots: slot("shoes_details"),
        core: slot("core_details"),
        fourth: slot("forth_details"),
        fifth: slot("fifth_details"),
        sixth: slot("sixth_details"),
    }
}

/// 符文页。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunePage {
    pub rank: i64,
    pub primary_style: Option<i64>,
    pub secondary_style: Option<i64>,
    pub primary: Vec<i64>,
    pub secondary: Vec<i64>,
    pub shards: Vec<i64>,
    pub pick_rate: Option<f64>,
    pub win_rate: Option<f64>,
    pub games: i64,
}

fn style_of_keystone(id: i64) -> Option<i64> {
    [8000, 8100, 8200, 8300, 8400]
        .into_iter()
        .find(|s| (*s..*s + 100).contains(&id))
}

fn style_of_code(code: &str) -> Option<i64> {
    Some(match code {
        "jm" => 8000,
        "zz" => 8100,
        "ws" => 8200,
        "qd" => 8300,
        "jj" => 8400,
        _ => return None,
    })
}

/// 符文（`_runeinfo`，字段 18119）。
pub fn parse_runes(v: &Value) -> Vec<RunePage> {
    let Some(p) = inner(v, "18119") else {
        return Vec::new();
    };
    let primary = text(&p, "rune_top_details")
        .or_else(|| text(&p, "top_details"))
        .or_else(|| text(&p, "rune_details"));
    if primary.map_or(true, |s| s == "-1") {
        return Vec::new();
    }
    records(primary, &['#'])
        .filter_map(|r| {
            let f: Vec<&str> = r.split('_').collect();
            let keystone = int(f.get(1).copied())?;
            if f.len() < 7 {
                return None;
            }
            let runes: Vec<Option<i64>> = f[3].split(',').map(|x| int(Some(x))).collect();
            let take = |from: usize, to: usize| -> Vec<i64> {
                runes
                    .iter()
                    .skip(from)
                    .take(to - from)
                    .filter_map(|x| *x)
                    .collect()
            };
            Some(RunePage {
                rank: int(f.first().copied()).unwrap_or(1),
                primary_style: style_of_keystone(keystone),
                secondary_style: style_of_code(f[2]),
                primary: take(0, 4),
                secondary: take(4, 6),
                shards: take(6, 9),
                pick_rate: ratio(f.get(4).copied()),
                win_rate: ratio(f.get(5).copied()),
                games: int(f.get(6).copied()).unwrap_or(0),
            })
        })
        .collect()
}

/// 加点（`_skill_point`，字段 18070）：（主升顺序 Q/W/E，前 18 级的顺序）。
pub fn parse_skill_order(v: &Value) -> (Vec<String>, Vec<String>) {
    let Some(p) = inner(v, "18070") else {
        return (Vec::new(), Vec::new());
    };
    let key = |s: &str| -> String {
        match s {
            "1" => "Q".into(),
            "2" => "W".into(),
            "3" => "E".into(),
            "4" => "R".into(),
            other => other.to_string(),
        }
    };
    let mut priority = Vec::new();
    let mut order = Vec::new();
    for segment in records(text(&p, "detaildetails"), &['$']) {
        let parts: Vec<&str> = segment.split('@').filter(|s| !s.is_empty()).collect();
        let header: Vec<&str> = parts.first().unwrap_or(&"").split(':').collect();
        if header.len() < 3 {
            continue;
        }
        if priority.is_empty() {
            priority = header[0]
                .split(',')
                .filter(|s| !s.is_empty())
                .map(key)
                .collect();
        }
        for detail in parts.iter().skip(1) {
            let levels: Vec<String> = detail
                .split('_')
                .next()
                .unwrap_or("")
                .split(',')
                .filter(|s| !s.is_empty())
                .map(key)
                .collect();
            if order.is_empty() && !levels.is_empty() {
                order = levels;
            }
        }
    }
    (priority, order)
}

/// 召唤师技能（`_skill`，字段 18029）：按选用率排，最多 5 组。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpellPair {
    pub spells: [i64; 2],
    pub win_rate: Option<f64>,
    pub pick_rate: Option<f64>,
}

pub fn parse_summoner_spells(v: &Value) -> Vec<SpellPair> {
    let Some(p) = inner(v, "18029") else {
        return Vec::new();
    };
    let mut out: Vec<SpellPair> = records(text(&p, "data_details"), &['#'])
        .filter_map(|r| {
            let f: Vec<&str> = r.split('_').collect();
            let mut a = int(f.first().copied())?;
            let mut b = int(f.get(1).copied())?;
            // 闪现（4）放前面（照 Akari）
            if b == 4 && a != 4 {
                std::mem::swap(&mut a, &mut b);
            }
            Some(SpellPair {
                spells: [a, b],
                win_rate: ratio(f.get(2).copied()),
                pick_rate: ratio(f.get(3).copied()),
            })
        })
        .collect();
    out.sort_by(|x, y| {
        y.pick_rate
            .unwrap_or(0.0)
            .total_cmp(&x.pick_rate.unwrap_or(0.0))
    });
    out.truncate(5);
    out
}

/// 海斗英雄榜的一行（`fuwen_aram_hero_rank_v2`，字段 15380）。胜率、选用率是 0..1（照接口，和峡谷的百分数不同）。
/// 按天出：当天的常常还是空的，往前找最近有数据的一天。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MayhemChampion {
    pub champion_id: i64,
    pub rank: Option<i64>,
    pub rank_change: Option<i64>,
    pub win_rate: Option<f64>,
    pub pick_rate: Option<f64>,
    /// 最搭的队友（英雄 id）。
    pub partners: Vec<i64>,
    /// 最不推荐的海克斯。
    pub worst_augments: Vec<i64>,
}

fn rank_change(s: Option<&str>) -> Option<i64> {
    let s = s.unwrap_or("");
    if s.is_empty() || s == "未变化" {
        return Some(0);
    }
    let digits: String = s.chars().filter(char::is_ascii_digit).collect();
    let n: i64 = digits.parse().ok()?;
    if s.starts_with("上升") {
        Some(n)
    } else if s.starts_with("下降") {
        Some(-n)
    } else {
        None
    }
}

pub fn parse_mayhem_champions(v: &Value) -> Vec<MayhemChampion> {
    let Some(p) = inner(v, "15380") else {
        return Vec::new();
    };
    records(text(&p, "listcollect"), &['#', '|'])
        .filter_map(|r| {
            let f: Vec<&str> = r.split('_').collect();
            Some(MayhemChampion {
                champion_id: int(f.first().copied())?,
                rank: int(f.get(1).copied()),
                rank_change: rank_change(f.get(2).copied()),
                win_rate: num(f.get(3).copied()),
                pick_rate: num(f.get(4).copied()),
                partners: f
                    .get(5)
                    .unwrap_or(&"")
                    .split('&')
                    .filter_map(|x| int(x.split(',').next()))
                    .collect(),
                worst_augments: f
                    .get(10)
                    .unwrap_or(&"")
                    .split(',')
                    .filter_map(|x| int(Some(x)))
                    .collect(),
            })
        })
        .collect()
}

/// 海斗海克斯榜（`fuwen_aram_rune_rank_v2`，2026-11-24 实测在字段 R15381 里，Akari 写的是 15332）。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MayhemAugment {
    pub augment_id: i64,
    /// 接口的第二个字段（请求 `augmentid_level=255` 时就是 255）。
    pub tier: Option<i64>,
    pub pick_rate: Option<f64>,
    pub win_rate: Option<f64>,
    pub win_rank: Option<i64>,
    pub best_champions: Vec<i64>,
}

pub fn parse_mayhem_augments(v: &Value) -> Vec<MayhemAugment> {
    let Some(p) = inner(v, "15381") else {
        return Vec::new();
    };
    // 有时整段就是一串记录（不是对象）
    let list = text(&p, "augmentlist")
        .map(str::to_string)
        .or_else(|| p.as_str().map(str::to_string));
    records(list.as_deref(), &['#', '|'])
        .filter_map(|r| {
            let f: Vec<&str> = r.split('_').collect();
            Some(MayhemAugment {
                augment_id: int(f.first().copied())?,
                tier: int(f.get(1).copied()),
                pick_rate: num(f.get(2).copied()),
                win_rate: num(f.get(5).copied()),
                win_rank: int(f.get(6).copied()),
                best_champions: ids(f.get(8).copied()),
            })
        })
        .collect()
}

/// 101 的分路名 → 界面上的。
pub fn lane_name(lane: &str) -> &'static str {
    match lane {
        "TOP" => "上路",
        "JUNGLE" => "打野",
        "MIDDLE" => "中路",
        "BOTTOM" => "下路",
        "SUPPORT" => "辅助",
        _ => "",
    }
}

/// 分路的写法（玩家说的、英文的）→ 101 接口的。
pub fn lane_code(lane: &str) -> Option<&'static str> {
    Some(match lane.trim().to_lowercase().as_str() {
        "top" | "上" | "上路" | "上单" => "TOP",
        "jungle" | "jg" | "打野" | "野" => "JUNGLE",
        "mid" | "middle" | "中" | "中路" | "中单" => "MIDDLE",
        "adc" | "bottom" | "bot" | "下路" | "ad" | "射手" => "BOTTOM",
        "support" | "sup" | "utility" | "辅助" => "SUPPORT",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn wrap(field: &str, inner: Value) -> Value {
        json!({"code": 0, "data": {"_fieldValues": {format!("R{field}"): inner.to_string()}}})
    }

    #[test]
    fn tier_list_from_the_real_shape() {
        // 2026-11-24 实际返回的前两条（上路，16.23）
        let v = wrap(
            "17960",
            json!({"dtstatdate": "20261122",
            "datadetails": "1_266_T1_TOP_50.45_12.11_21.31_27,74,114_0_0_0#2_875_T1_TOP_51.28_4.8_2.35_67,240,85_0_0_0"}),
        );
        let (date, rows) = parse_tier_list(&v);
        assert_eq!(date, "20261122");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].champion_id, 266);
        assert_eq!(rows[0].tier, "T1");
        assert_eq!(rows[0].win_rate, Some(0.5045));
        assert_eq!(rows[0].counters, vec![27, 74, 114]);
        assert_eq!(rows[1].rank, Some(2));
        // 出错的信封、坏的数据：空，不 panic
        assert!(parse_tier_list(&json!({"code": 1, "data": null}))
            .1
            .is_empty());
        assert!(parse_tier_list(&json!({"code": 0, "data": "not json"}))
            .1
            .is_empty());
        assert!(
            parse_tier_list(&wrap("17960", json!({"datadetails": "x_y#_#"})))
                .1
                .is_empty()
        );
    }

    #[test]
    fn matchups_builds_runes_spells_skills() {
        let (good, bad) = parse_matchups(&wrap(
            "17968",
            json!({
            "high_op_details": "1_157_55.2#2_103_53.1", "low_op_details": "1_39_44.0"}),
        ));
        assert_eq!(good.len(), 2);
        assert_eq!(good[0].champion_id, 157);
        assert_eq!(bad[0].win_rate, Some(0.44));

        let build = parse_build(&wrap(
            "18087",
            json!({
            "core_details": "3031,6672,3036_1_12.5_55.1#3078,3053_2_8_50", "shoes_details": "-1"}),
        ));
        assert_eq!(build.core[0].items, vec![3031, 6672, 3036]);
        assert_eq!(build.core[1].win_rate, Some(0.5));
        assert!(build.boots.is_empty());

        let runes = parse_runes(&wrap(
            "18119",
            json!({
            "rune_top_details": "1_8010_zz_8010,9111,9104,8299,8139,8135,5005,5008,5011_30.5_52.1_1200"}),
        ));
        assert_eq!(runes[0].primary_style, Some(8000));
        assert_eq!(runes[0].secondary_style, Some(8100));
        assert_eq!(runes[0].primary, vec![8010, 9111, 9104, 8299]);
        assert_eq!(runes[0].shards, vec![5005, 5008, 5011]);
        assert_eq!(runes[0].games, 1200);

        let spells = parse_summoner_spells(&wrap(
            "18029",
            json!({
            "data_details": "12_4_51.0_10.0#4_14_52.0_80.0"}),
        ));
        assert_eq!(spells[0].spells, [4, 14], "按选用率排");
        assert_eq!(spells[1].spells, [4, 12]);

        let (priority, order) = parse_skill_order(&wrap(
            "18070",
            json!({
            "detaildetails": "1,3,2:60.1:52.3@1,3,2,1,1,4_40.0_53.0$3,1,2:20:50"}),
        ));
        assert_eq!(priority, vec!["Q", "E", "W"]);
        assert_eq!(order, vec!["Q", "E", "W", "Q", "Q", "R"]);
    }

    #[test]
    fn mayhem_tables() {
        let champs = parse_mayhem_champions(&wrap(
            "15380",
            json!({
            "listcollect": "39_1_上升2_55.1_6.2_157,1,2,3&103,1,2,3_20_60_25_20_1,2,3#157_2_下降1_54_5_x_0_0_0_0_"}),
        ));
        assert_eq!(champs[0].champion_id, 39);
        assert_eq!(champs[0].rank_change, Some(2));
        assert_eq!(champs[0].partners, vec![157, 103]);
        assert_eq!(champs[0].worst_augments, vec![1, 2, 3]);
        assert_eq!(champs[1].rank_change, Some(-1));
        let augs = parse_mayhem_augments(&wrap(
            "15381",
            json!({"dtstatdate": "20261123", "augmentlist": "1001_3_5.5_1_0_56.2_3_0_39,157|1002_1_2_9_0_48_80_0_"}),
        ));
        assert_eq!(augs.len(), 2);
        assert_eq!(augs[0].tier, Some(3));
        assert_eq!(augs[0].win_rate, Some(56.2));
        assert_eq!(augs[0].best_champions, vec![39, 157]);
    }

    #[test]
    fn versions_and_lanes() {
        let v = json!({"code": 0, "data": [{"id": "232", "name": "16.23"}, {"id": 231, "name": "16.22"}, {"x": 1}]});
        assert_eq!(
            parse_versions(&v),
            vec![(232, "16.23".into()), (231, "16.22".into())]
        );
        assert_eq!(lane_code("中单"), Some("MIDDLE"));
        assert_eq!(lane_code("bot"), Some("BOTTOM"));
        assert_eq!(lane_name("SUPPORT"), "辅助");
    }
}
