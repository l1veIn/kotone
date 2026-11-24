//! 名字表（客户端没开时也要能显示）：腾讯 `game.gtimg.cn` 上的英雄（`hero_list.js`，见 [`super::nicknames`]）、
//! 装备（`items.js`）、符文（`rune_list.js`）、召唤师技能（`summonerskill_list.js`）。文件里就是 JSON。
//! 客户端开着时优先用客户端自带的资料（[`crate::client::GameData`]），这里是备用。

use std::collections::HashMap;

use serde_json::Value;

pub const ITEMS: &str = "https://game.gtimg.cn/images/lol/act/img/js/items/items.js";
pub const RUNES: &str = "https://game.gtimg.cn/images/lol/act/img/js/runeList/rune_list.js";
pub const SPELLS: &str =
    "https://game.gtimg.cn/images/lol/act/img/js/summonerskillList/summonerskill_list.js";

fn id_of(v: &Value) -> Option<i64> {
    match v {
        Value::String(s) => s.trim().parse().ok(),
        other => other.as_i64(),
    }
}

/// `items.js` → 装备 id → 名字。
pub fn parse_items(v: &Value) -> HashMap<i64, String> {
    v.get("items")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|it| {
                    Some((
                        id_of(it.get("itemId")?)?,
                        it.get("name")?.as_str()?.to_string(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// `{"rune": {"8010": {"name": …}}}` / `{"summonerskill": {"4": {"name": …}}}` → id → 名字。
pub fn parse_named_map(v: &Value, key: &str) -> HashMap<i64, String> {
    v.get(key)
        .and_then(Value::as_object)
        .map(|map| {
            map.iter()
                .filter_map(|(k, x)| {
                    Some((k.trim().parse().ok()?, x.get("name")?.as_str()?.to_string()))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// 装备、符文、召唤师技能的名字。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Statics {
    pub items: HashMap<i64, String>,
    pub runes: HashMap<i64, String>,
    pub spells: HashMap<i64, String>,
}

impl Statics {
    pub fn item(&self, id: i64) -> String {
        self.items
            .get(&id)
            .cloned()
            .unwrap_or_else(|| format!("装备 {id}"))
    }

    pub fn rune(&self, id: i64) -> String {
        self.runes
            .get(&id)
            .cloned()
            .unwrap_or_else(|| format!("符文 {id}"))
    }

    pub fn spell(&self, id: i64) -> String {
        self.spells
            .get(&id)
            .cloned()
            .unwrap_or_else(|| format!("技能 {id}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn name_tables() {
        let items = parse_items(
            &json!({"items": [{"itemId": "1001", "name": "鞋子"}, {"itemId": 3031, "name": "无尽之刃"}, {"name": "x"}]}),
        );
        assert_eq!(items.get(&1001).map(String::as_str), Some("鞋子"));
        assert_eq!(items.get(&3031).map(String::as_str), Some("无尽之刃"));
        assert_eq!(items.len(), 2);
        let runes = parse_named_map(&json!({"rune": {"8010": {"name": "征服者"}}}), "rune");
        assert_eq!(runes.get(&8010).map(String::as_str), Some("征服者"));
        let spells = parse_named_map(
            &json!({"summonerskill": {"4": {"name": "闪现"}}}),
            "summonerskill",
        );
        let s = Statics {
            items,
            runes,
            spells,
        };
        assert_eq!(s.spell(4), "闪现");
        assert_eq!(s.item(9), "装备 9");
        assert!(parse_items(&json!(null)).is_empty());
    }
}
