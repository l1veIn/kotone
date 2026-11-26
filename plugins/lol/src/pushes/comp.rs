//! 选人卡片的“阵容”那块（ADR 043 B 轮；讨论记录第二节“队友都预选后：阵容缺什么（全 AD、没前排、没开团）”）：
//! 按固定规则数客户端英雄资料里的几项，只说数（“物理 3 · 魔法 1”“没有前排”），不说谁该换、不评价人（讨论记录第七节）。
//!
//! 资料是客户端自己的 `/lol-game-data/assets/v1/champions/{id}.json`（Riot 给每个英雄标的）：
//! - `tacticalInfo.damageType`：`kPhysical` 物理 / `kMagic` 魔法 / `kMixed` 混合；`tacticalInfo.attackType`：近战 / 远程；
//! - `roles` 里有 `tank`，或者 `playstyleInfo.durability` 是 3（满分 3）：算**前排**；
//! - `playstyleInfo.crowdControl` 是 3（满分 3）：算**强控**。
//!
//! 同样的英雄永远得到同样的数（ADR 043 D2 的“按固定规则”）。

use serde_json::{json, Value};

/// 一个英雄的几项（[`traits`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Traits {
    pub damage: Damage,
    pub ranged: bool,
    pub frontline: bool,
    pub strong_cc: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Damage {
    Physical,
    Magic,
    Mixed,
}

/// 至少几个人选了英雄才数（少于这个数，“全是物理”“没有前排”说了也没意思）。
pub const MIN_PICKS: usize = 3;

/// 客户端的英雄资料 → 几项；资料不对（没有伤害类型）时 None。
pub fn traits(champion: &Value) -> Option<Traits> {
    let tactical = champion.get("tacticalInfo")?;
    let damage = match tactical.get("damageType")?.as_str()? {
        "kPhysical" => Damage::Physical,
        "kMagic" => Damage::Magic,
        "kMixed" => Damage::Mixed,
        _ => return None,
    };
    let play = |key: &str| {
        champion
            .pointer(&format!("/playstyleInfo/{key}"))
            .and_then(Value::as_i64)
            .unwrap_or(0)
    };
    let tank = champion
        .get("roles")
        .and_then(Value::as_array)
        .is_some_and(|r| r.iter().any(|x| x.as_str() == Some("tank")));
    Some(Traits {
        damage,
        ranged: tactical.get("attackType").and_then(Value::as_str) == Some("ranged"),
        frontline: tank || play("durability") >= 3,
        strong_cc: play("crowdControl") >= 3,
    })
}

/// 一边的阵容：几个人有资料，物理 / 魔法 / 混合、前排、强控、近战 / 远程各几个，加上照固定规则得出的几条
/// （`all_physical` 全是物理、`all_magic` 全是魔法、`no_frontline` 没有前排、`no_cc` 没有强控）。
/// 有资料的不到 [`MIN_PICKS`] 个时 None。
pub fn side(list: &[Traits]) -> Option<Value> {
    if list.len() < MIN_PICKS {
        return None;
    }
    let count = |f: &dyn Fn(&Traits) -> bool| list.iter().filter(|t| f(t)).count();
    let physical = count(&|t| t.damage == Damage::Physical);
    let magic = count(&|t| t.damage == Damage::Magic);
    let mixed = count(&|t| t.damage == Damage::Mixed);
    let frontline = count(&|t| t.frontline);
    let cc = count(&|t| t.strong_cc);
    let ranged = count(&|t| t.ranged);
    let mut flags = Vec::new();
    if physical == list.len() {
        flags.push("all_physical");
    }
    if magic == list.len() {
        flags.push("all_magic");
    }
    if frontline == 0 {
        flags.push("no_frontline");
    }
    if cc == 0 {
        flags.push("no_cc");
    }
    Some(json!({
        "count": list.len(),
        "physical": physical,
        "magic": magic,
        "mixed": mixed,
        "frontline": frontline,
        "cc": cc,
        "melee": list.len() - ranged,
        "ranged": ranged,
        "flags": flags,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 照 CommunityDragon 2026-11-26 的几个英雄资料（只留用到的几项）。
    fn champion(damage: &str, attack: &str, durability: i64, cc: i64, roles: &[&str]) -> Value {
        json!({"tacticalInfo": {"damageType": damage, "attackType": attack},
               "playstyleInfo": {"durability": durability, "crowdControl": cc}, "roles": roles})
    }

    #[test]
    fn counts_and_fixed_rule_flags() {
        let yasuo = traits(&champion(
            "kPhysical",
            "melee",
            1,
            2,
            &["fighter", "assassin"],
        ))
        .unwrap();
        let malphite = traits(&champion("kMagic", "melee", 3, 3, &["tank", "mage"])).unwrap();
        let lux = traits(&champion("kMagic", "ranged", 1, 2, &["mage", "support"])).unwrap();
        let jinx = traits(&champion("kPhysical", "ranged", 1, 1, &["marksman"])).unwrap();
        let zed = traits(&champion("kPhysical", "melee", 1, 1, &["assassin"])).unwrap();
        let kayle = traits(&champion("kMixed", "ranged", 2, 1, &["fighter"])).unwrap();
        assert!(malphite.frontline && malphite.strong_cc);
        assert!(!yasuo.frontline && !yasuo.ranged);
        assert_eq!(kayle.damage, Damage::Mixed);
        assert!(traits(&json!({"tacticalInfo": {}})).is_none());

        // 全物理、没前排、没强控：三条都写
        let ad = side(&[yasuo, jinx, zed]).unwrap();
        assert_eq!(ad["count"], 3);
        assert_eq!(ad["physical"], 3);
        assert_eq!(
            ad["flags"],
            json!(["all_physical", "no_frontline", "no_cc"])
        );
        assert_eq!(ad["melee"], 2);
        assert_eq!(ad["ranged"], 1);
        // 有石头人：前排 1、强控 1，没有那两条
        let mixed = side(&[yasuo, malphite, lux, jinx, kayle]).unwrap();
        assert_eq!(mixed["flags"], json!([]));
        assert_eq!(mixed["magic"], 2);
        assert_eq!(mixed["mixed"], 1);
        assert_eq!(mixed["frontline"], 1);
        assert_eq!(mixed["cc"], 1);
        // 不到 3 个人：不数
        assert!(side(&[yasuo, lux]).is_none());
    }
}
