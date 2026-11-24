//! 召唤师技能计时（ADR 035 D4）：局内接口没有冷却（国服实测），玩家说“阿狸闪了”才记。
//!
//! 照 LeagueAkari 的计时器：冷却 = 基础冷却 × 100 / (100 + 召唤师技能急速)；模式急速
//! 大乱斗 / 海克斯大乱斗 70、无限火力 300。Akari 不算装备，这里另外算明朗之靴（10）、
//! 猩红明朗（20），装备取局内接口里那个人当时的。星界洞悉（符文）和海斗的“闪现 2”这类海克斯看不到。

use crate::client::live::LiveItem;

/// 模式给的召唤师技能急速。
pub fn mode_haste(mode: &str) -> f64 {
    match mode.to_ascii_uppercase().as_str() {
        "ARAM" | "KIWI" => 70.0,
        "URF" | "ARURF" => 300.0,
        _ => 0.0,
    }
}

/// 装备给的召唤师技能急速，和用来说明的名字。
pub fn item_haste(items: &[LiveItem]) -> (f64, Vec<String>) {
    let mut haste = 0.0;
    let mut notes = Vec::new();
    for item in items {
        let (value, name) = match item.id {
            3158 | 223158 | 773158 => (10.0, "明朗之靴"),
            3171 => (20.0, "猩红明朗"),
            _ if item.name.contains("明朗之靴") => (10.0, "明朗之靴"),
            _ if item.name.contains("猩红明朗") || item.name.contains("深红明朗") => {
                (20.0, "猩红明朗")
            }
            _ => continue,
        };
        haste += value;
        notes.push(format!("{name} +{value:.0}"));
    }
    (haste, notes)
}

pub fn cooldown(base: f64, haste: f64) -> f64 {
    base * 100.0 / (100.0 + haste.max(0.0))
}

/// 记下的一次交技能。
#[derive(Debug, Clone, PartialEq)]
pub struct SpellUse {
    /// 英雄的英文 id（认人用）。
    pub alias: String,
    /// 说给玩家、发到聊天里用的英雄名。
    pub champion: String,
    pub spell: String,
    /// 交的时间、好的时间（游戏时间，秒）。
    pub used_at: f64,
    pub ready_at: f64,
    /// 算进去了什么（“大乱斗 +70”“明朗之靴 +10”），以及没算进去的。
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct SpellTracker {
    uses: Vec<SpellUse>,
}

impl SpellTracker {
    /// 同一个人的同一个技能只留最新的一次。
    pub fn record(&mut self, entry: SpellUse) {
        self.uses
            .retain(|u| !(u.alias == entry.alias && u.spell == entry.spell));
        self.uses.push(entry);
    }

    /// 清掉记错的：只给英雄清这个人的全部，再给技能只清那一个；都不给清全部。返回清了几条。
    pub fn clear(&mut self, alias: Option<&str>, spell: Option<&str>) -> usize {
        let before = self.uses.len();
        self.uses.retain(|u| {
            let hit = alias.map_or(true, |a| u.alias.eq_ignore_ascii_case(a))
                && spell.map_or(true, |s| u.spell == s);
            !hit
        });
        before - self.uses.len()
    }

    /// 这个人的这个技能还没好的话，什么时候好。
    pub fn ready_at(&self, alias: &str, spell: &str, now: f64) -> Option<f64> {
        self.uses
            .iter()
            .find(|u| u.alias.eq_ignore_ascii_case(alias) && u.spell == spell && u.ready_at > now)
            .map(|u| u.ready_at)
    }

    /// 现在还没好的，先好的在前。
    pub fn down(&self, now: f64) -> Vec<&SpellUse> {
        let mut out: Vec<&SpellUse> = self.uses.iter().filter(|u| u.ready_at > now).collect();
        out.sort_by(|a, b| a.ready_at.total_cmp(&b.ready_at));
        out
    }

    pub fn is_empty(&self) -> bool {
        self.uses.is_empty()
    }
}

/// 聊天里的叫法：闪现说“闪”，其余照原名。
pub fn short_spell(spell: &str) -> &str {
    match spell {
        "闪现" => "闪",
        "引燃" => "点燃",
        other => other,
    }
}

/// 发到聊天框的一句：“亚索 闪 14:32好”。
pub fn chat_line(entry: &SpellUse) -> String {
    format!(
        "{} {} {}好",
        entry.champion,
        short_spell(&entry.spell),
        crate::client::clock(entry.ready_at)
    )
}

/// 算一次：基础冷却、模式、装备 → (好的时间, 说明)。
pub fn ready_time(base: f64, used_at: f64, mode: &str, items: &[LiveItem]) -> (f64, Vec<String>) {
    let mut notes = Vec::new();
    let from_mode = mode_haste(mode);
    if from_mode > 0.0 {
        notes.push(format!("模式 +{from_mode:.0}"));
    }
    let (from_items, item_notes) = item_haste(items);
    notes.extend(item_notes);
    match mode.to_ascii_uppercase().as_str() {
        "CLASSIC" | "SWIFTPLAY" => notes.push("没算星界洞悉（看不到符文）".into()),
        "KIWI" => notes.push("没算海克斯（看不到）".into()),
        _ => {}
    }
    let cd = cooldown(base, from_mode + from_items);
    (used_at + cd, notes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: i64, name: &str) -> LiveItem {
        LiveItem {
            id,
            name: name.into(),
        }
    }

    #[test]
    fn flash_cooldowns_per_mode_and_boots() {
        // 峡谷 300 秒；大乱斗 176.5 秒；带明朗之靴 166.7 秒（wiki 的数）
        assert!((cooldown(300.0, 0.0) - 300.0).abs() < 1e-9);
        let (ready, notes) = ready_time(300.0, 100.0, "KIWI", &[]);
        assert!((ready - 100.0 - 176.47).abs() < 0.01, "{ready}");
        assert_eq!(notes, ["模式 +70", "没算海克斯（看不到）"]);
        let (ready, notes) = ready_time(
            300.0,
            0.0,
            "ARAM",
            &[item(3158, "明朗之靴"), item(1001, "鞋子")],
        );
        assert!((ready - 166.67).abs() < 0.01, "{ready}");
        assert_eq!(notes, ["模式 +70", "明朗之靴 +10"]);
        let (ready, notes) = ready_time(300.0, 0.0, "CLASSIC", &[item(3171, "猩红明朗")]);
        assert!((ready - 250.0).abs() < 0.01);
        assert_eq!(notes.last().unwrap(), "没算星界洞悉（看不到符文）");
        assert_eq!(mode_haste("URF"), 300.0);
    }

    fn used(alias: &str, spell: &str, used_at: f64, ready_at: f64) -> SpellUse {
        SpellUse {
            alias: alias.into(),
            champion: if alias == "Yasuo" {
                "亚索".into()
            } else {
                "阿狸".into()
            },
            spell: spell.into(),
            used_at,
            ready_at,
            notes: vec![],
        }
    }

    #[test]
    fn tracker_keeps_the_latest_and_reports_what_is_down() {
        let mut t = SpellTracker::default();
        t.record(used("Yasuo", "闪现", 100.0, 276.0));
        t.record(used("Ahri", "闪现", 200.0, 376.0));
        t.record(used("Yasuo", "闪现", 150.0, 326.0));
        t.record(used("Yasuo", "引燃", 150.0, 250.0));
        let down: Vec<(&str, &str)> = t
            .down(260.0)
            .iter()
            .map(|u| (u.alias.as_str(), u.spell.as_str()))
            .collect();
        assert_eq!(down, [("Yasuo", "闪现"), ("Ahri", "闪现")], "引燃已经好了");
        assert_eq!(t.ready_at("yasuo", "闪现", 260.0), Some(326.0));
        assert_eq!(t.ready_at("Yasuo", "闪现", 400.0), None);
        assert_eq!(chat_line(t.down(0.0)[0]), "亚索 点燃 4:10好");
        assert_eq!(t.clear(Some("Yasuo"), Some("闪现")), 1);
        assert_eq!(t.clear(None, None), 2);
        assert!(t.is_empty());
    }
}
