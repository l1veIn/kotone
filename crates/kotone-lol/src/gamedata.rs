//! 客户端自带的游戏资料（`/lol-game-data/assets/v1/…`，语言跟客户端，国服是中文）：
//! 英雄、召唤师技能（含基础冷却）、装备、海克斯、队列。不用联网，换版本自动跟着变。

use std::collections::{HashMap, HashSet};

use serde_json::Value;

/// 随机英雄的队列（客户端资料读不到时用）：极地大乱斗 450、大乱斗冠军杯 720、无限乱斗 900、海克斯大乱斗 2400。
pub const RANDOM_PICK_QUEUES: [i64; 4] = [450, 720, 900, 2400];

/// 这个队列是不是随机英雄：客户端资料里 `pickMode` 是 `AllRandomPickStrategy`，或者是上面几个常见的。
/// 随机英雄的模式里“这个英雄最近没玩过 / 常玩”不说明什么（ADR 035）。
pub fn random_pick(data: Option<&GameData>, queue_id: i64) -> bool {
    RANDOM_PICK_QUEUES.contains(&queue_id)
        || data.is_some_and(|d| d.random_pick_queues.contains(&queue_id))
}

#[derive(Debug, Clone, PartialEq)]
pub struct Champion {
    pub id: i64,
    /// 英文 id，例如 `Irelia`（局内接口的 `rawChampionName` 末尾就是它）。
    pub alias: String,
    /// `champion-summary` 的 `name`。国服是称号（“刀锋舞者”），局内接口的 `championName` 也是它。
    pub title: String,
    /// `champion-summary` 的 `description`。国服是名字（“艾瑞莉娅”）。
    pub name: String,
}

impl Champion {
    /// “刀锋舞者 艾瑞莉娅”。
    pub fn label(&self) -> String {
        match (self.title.is_empty(), self.name.is_empty()) {
            (false, false) if self.title != self.name => format!("{} {}", self.title, self.name),
            (false, _) => self.title.clone(),
            (true, false) => self.name.clone(),
            (true, true) => self.alias.clone(),
        }
    }

    /// 发到聊天里用的短名字：国服玩家多半叫名字（“亚索”），没有就用称号。
    pub fn short(&self) -> &str {
        if !self.name.is_empty() {
            &self.name
        } else if !self.title.is_empty() {
            &self.title
        } else {
            &self.alias
        }
    }

    /// 玩家说的（称号、名字、英文 id 的任意一个，或它们的一部分）对得上吗。
    pub fn matches(&self, query: &str) -> bool {
        let q = normalize(query);
        if q.is_empty() {
            return false;
        }
        [&self.title, &self.name, &self.alias]
            .iter()
            .map(|s| normalize(s))
            .any(|s| !s.is_empty() && (s == q || s.contains(&q) || q.contains(&s)))
    }
}

/// 比较用：小写、去掉空白和标点。
pub fn normalize(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

#[derive(Debug, Clone, PartialEq)]
pub struct SpellInfo {
    pub id: i64,
    pub name: String,
    /// 基础冷却（秒）。
    pub cooldown: f64,
    /// 能用这个技能的模式（`CLASSIC`、`ARAM`、`KIWI`……）。
    pub modes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Augment {
    pub name: String,
    /// `kSilver` / `kGold` / `kPrismatic`。
    pub rarity: String,
}

#[derive(Debug, Clone, Default)]
pub struct GameData {
    pub champions: Vec<Champion>,
    pub spells: Vec<SpellInfo>,
    pub items: HashMap<i64, String>,
    pub augments: HashMap<i64, Augment>,
    pub queues: HashMap<i64, String>,
    /// `pickMode` 是 `AllRandomPickStrategy` 的队列。
    pub random_pick_queues: HashSet<i64>,
}

fn s(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string()
}

fn i(v: &Value, key: &str) -> i64 {
    v.get(key).and_then(Value::as_i64).unwrap_or(0)
}

fn list(v: &Value) -> &[Value] {
    v.as_array().map(Vec::as_slice).unwrap_or(&[])
}

impl GameData {
    /// 五个文件的 JSON（缺哪个就传 `Value::Null`）。
    pub fn from_json(
        champions: &Value,
        spells: &Value,
        items: &Value,
        augments: &Value,
        queues: &Value,
    ) -> Self {
        Self {
            champions: list(champions)
                .iter()
                .filter(|c| i(c, "id") > 0)
                .map(|c| Champion {
                    id: i(c, "id"),
                    alias: s(c, "alias"),
                    title: s(c, "name"),
                    name: s(c, "description"),
                })
                .collect(),
            spells: list(spells)
                .iter()
                .filter(|x| !s(x, "name").is_empty())
                .map(|x| SpellInfo {
                    id: i(x, "id"),
                    name: s(x, "name"),
                    cooldown: x.get("cooldown").and_then(Value::as_f64).unwrap_or(0.0),
                    modes: list(x.get("gameModes").unwrap_or(&Value::Null))
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect(),
                })
                .collect(),
            items: list(items)
                .iter()
                .map(|x| (i(x, "id"), s(x, "name")))
                .filter(|(_, name)| !name.is_empty())
                .collect(),
            augments: list(augments)
                .iter()
                .map(|x| {
                    (
                        i(x, "id"),
                        Augment {
                            name: s(x, "nameTRA"),
                            rarity: s(x, "rarity"),
                        },
                    )
                })
                .filter(|(_, a)| !a.name.is_empty())
                .collect(),
            queues: list(queues)
                .iter()
                .map(|x| (i(x, "id"), s(x, "name")))
                .filter(|(_, name)| !name.is_empty())
                .collect(),
            random_pick_queues: list(queues)
                .iter()
                .filter(|x| s(x, "pickMode") == "AllRandomPickStrategy")
                .map(|x| i(x, "id"))
                .collect(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.champions.is_empty()
    }

    pub fn champion(&self, id: i64) -> Option<&Champion> {
        self.champions.iter().find(|c| c.id == id)
    }

    pub fn champion_by_alias(&self, alias: &str) -> Option<&Champion> {
        self.champions
            .iter()
            .find(|c| c.alias.eq_ignore_ascii_case(alias))
    }

    /// 召唤师技能（按名字，优先这个模式能用的那一条：同名的有好几条，例如斗魂竞技场的闪现）。
    pub fn spell(&self, name: &str, mode: &str) -> Option<&SpellInfo> {
        let same: Vec<&SpellInfo> = self
            .spells
            .iter()
            .filter(|x| x.name == name.trim() && x.cooldown > 0.0)
            .collect();
        same.iter()
            .find(|x| x.modes.iter().any(|m| m.eq_ignore_ascii_case(mode)))
            .or_else(|| same.iter().find(|x| x.modes.iter().any(|m| m == "CLASSIC")))
            .or_else(|| same.first())
            .copied()
    }

    pub fn queue_name(&self, id: i64) -> Option<&str> {
        self.queues.get(&id).map(String::as_str)
    }

    pub fn augment_name(&self, id: i64) -> Option<&str> {
        self.augments.get(&id).map(|a| a.name.as_str())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::json;

    pub(crate) fn sample() -> GameData {
        GameData::from_json(
            &json!([
                {"id": -1, "name": "无", "description": "", "alias": "None"},
                {"id": 39, "name": "刀锋舞者", "description": "艾瑞莉娅", "alias": "Irelia"},
                {"id": 157, "name": "疾风剑豪", "description": "亚索", "alias": "Yasuo"},
                {"id": 103, "name": "九尾妖狐", "description": "阿狸", "alias": "Ahri"}
            ]),
            &json!([
                {"id": 4, "name": "闪现", "cooldown": 300, "gameModes": ["ARAM", "KIWI", "CLASSIC"]},
                {"id": 2202, "name": "闪现", "cooldown": 0, "gameModes": ["CHERRY"]},
                {"id": 74, "name": "闪现", "cooldown": 280, "gameModes": ["JADE"]},
                {"id": 32, "name": "标记", "cooldown": 80, "gameModes": ["KIWI", "ARAM"]},
                {"id": 14, "name": "引燃", "cooldown": 180, "gameModes": ["ARAM", "CLASSIC"]}
            ]),
            &json!([{"id": 3158, "name": "明朗之靴"}]),
            &json!([{"id": 1205, "nameTRA": "物理转魔法", "rarity": "kSilver"}]),
            &json!([
                {"id": 2400, "name": "海克斯大乱斗 ", "pickMode": "AllRandomPickStrategy"},
                {"id": 3120, "name": "召唤师峡谷 全随机", "pickMode": "AllRandomPickStrategy"},
                {"id": 420, "name": "单双排", "pickMode": "DraftModeSinglePickStrategy"}
            ]),
        )
    }

    #[test]
    fn champions_skip_the_placeholder_and_match_any_name() {
        let data = sample();
        assert_eq!(data.champions.len(), 3);
        let irelia = data.champion(39).unwrap();
        assert_eq!(irelia.label(), "刀锋舞者 艾瑞莉娅");
        assert_eq!(irelia.short(), "艾瑞莉娅");
        for q in ["刀锋舞者", "艾瑞莉娅", "irelia", "艾瑞莉娅 "] {
            assert!(irelia.matches(q), "{q}");
        }
        assert!(!irelia.matches("亚索"));
        assert!(!irelia.matches(""));
        assert_eq!(data.champion_by_alias("yasuo").unwrap().id, 157);
    }

    #[test]
    fn spell_prefers_the_current_mode() {
        let data = sample();
        assert_eq!(data.spell("闪现", "KIWI").unwrap().id, 4);
        assert_eq!(data.spell("闪现", "JADE").unwrap().id, 74);
        // 没有这个模式的：用峡谷那条；冷却 0 的（斗魂竞技场）不算
        assert_eq!(data.spell("闪现", "TUTORIAL").unwrap().id, 4);
        assert!(data.spell("不存在", "KIWI").is_none());
    }

    #[test]
    fn queues_and_augments_are_trimmed() {
        let data = sample();
        assert_eq!(data.queue_name(2400), Some("海克斯大乱斗"));
        assert_eq!(data.augment_name(1205), Some("物理转魔法"));
    }

    #[test]
    fn random_pick_queues_come_from_pick_mode_or_the_fallback_list() {
        let data = sample();
        assert!(random_pick(Some(&data), 2400));
        // 资料里标了全随机的（列表里没有）
        assert!(random_pick(Some(&data), 3120));
        assert!(!random_pick(Some(&data), 420));
        // 没有资料：大乱斗、海斗照样算
        assert!(random_pick(None, 450));
        assert!(random_pick(None, 2400));
        assert!(!random_pick(None, 420));
        assert!(!random_pick(None, 3120));
    }
}
