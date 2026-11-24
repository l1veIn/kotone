//! 英雄的外号（“刀妹”“女刀”“火女”）：腾讯 `hero_list.js` 的 `keywords`（文件里就是 JSON）。
//! 玩家和她说外号时，用它对上英雄 id（客户端资料里只有称号、名字和英文 id）。

use std::collections::HashMap;

use serde_json::Value;

pub const URL: &str = "https://game.gtimg.cn/images/lol/act/img/js/heroList/hero_list.js";

/// 腾讯的表里没有、玩家常说的几个外号（2026-11-24 对过一遍常用外号，只缺这几个）。
const EXTRA: &[(i64, &str)] = &[(39, "刀妹"), (86, "德玛"), (81, "EZ"), (67, "VN")];

/// 英雄 id → 外号（含名字、称号、英文名、拼音）；还有显示用的名字。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Nicknames {
    pub by_hero: HashMap<i64, Vec<String>>,
    /// 英雄 id → (名字“安妮”, 称号“黑暗之女”)。注意 `hero_list.js` 里 `name` 是称号、`title` 是名字（和 Data Dragon 一样反着）。
    pub names: HashMap<i64, (String, String)>,
    /// 英雄 id → 英文 id（`Irelia`），OP.GG 要用。
    pub aliases: HashMap<i64, String>,
}

fn norm(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace() && *c != '·' && *c != '\'')
        .flat_map(char::to_lowercase)
        .collect()
}

impl Nicknames {
    pub fn parse(text: &str) -> Self {
        // 有时外面包着 `var heroList = …;`：取第一个 { 到最后一个 }
        let body = match (text.find('{'), text.rfind('}')) {
            (Some(a), Some(b)) if b > a => &text[a..=b],
            _ => return Self::default(),
        };
        let Ok(v) = serde_json::from_str::<Value>(body) else {
            return Self::default();
        };
        let heroes = v.get("hero").and_then(Value::as_array);
        let names = heroes
            .map(|heroes| {
                heroes
                    .iter()
                    .filter_map(|h| {
                        let id: i64 = h.get("heroId")?.as_str()?.parse().ok()?;
                        let text =
                            |k: &str| h.get(k).and_then(Value::as_str).unwrap_or("").to_string();
                        Some((id, (text("title"), text("name"))))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let aliases = heroes
            .map(|heroes| {
                heroes
                    .iter()
                    .filter_map(|h| {
                        let id: i64 = h.get("heroId")?.as_str()?.parse().ok()?;
                        Some((id, h.get("alias")?.as_str()?.to_string()))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let by_hero = heroes
            .map(|heroes| {
                heroes
                    .iter()
                    .filter_map(|h| {
                        let id: i64 = h.get("heroId")?.as_str()?.parse().ok()?;
                        let mut words: Vec<String> = ["name", "title", "alias"]
                            .iter()
                            .filter_map(|k| h.get(*k).and_then(Value::as_str))
                            .map(str::to_string)
                            .collect();
                        words.extend(
                            h.get("keywords")
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .split([',', '，'])
                                .map(str::trim)
                                .filter(|w| !w.is_empty())
                                .map(str::to_string),
                        );
                        words.extend(
                            EXTRA
                                .iter()
                                .filter(|(hero, _)| *hero == id)
                                .map(|(_, w)| w.to_string()),
                        );
                        words.dedup();
                        Some((id, words))
                    })
                    .collect()
            })
            .unwrap_or_default();
        Self {
            by_hero,
            names,
            aliases,
        }
    }

    /// 显示用的短名字（“安妮”）；没有时用 `英雄 <id>`。
    pub fn short(&self, id: i64) -> String {
        self.names
            .get(&id)
            .map(|(name, title)| {
                if name.is_empty() {
                    title.clone()
                } else {
                    name.clone()
                }
            })
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| format!("英雄 {id}"))
    }

    /// “黑暗之女 安妮”。
    pub fn label(&self, id: i64) -> String {
        match self.names.get(&id) {
            Some((name, title)) if !name.is_empty() && !title.is_empty() => {
                format!("{title} {name}")
            }
            _ => self.short(id),
        }
    }

    /// 玩家说的（外号、名字、称号、英文名、拼音）→ 英雄 id。整个词对上的优先；只有一个英雄的外号包含它时也算。
    pub fn find(&self, query: &str) -> Option<i64> {
        let q = norm(query);
        if q.is_empty() {
            return None;
        }
        let mut exact: Vec<i64> = self
            .by_hero
            .iter()
            .filter(|(_, words)| words.iter().any(|w| norm(w) == q))
            .map(|(id, _)| *id)
            .collect();
        exact.sort_unstable();
        exact.dedup();
        if exact.len() == 1 {
            return exact.first().copied();
        }
        if !exact.is_empty() {
            return None; // 好几个英雄都叫这个，问清楚
        }
        // 至少两个字才按包含找（单个字太容易撞）
        if q.chars().count() < 2 {
            return None;
        }
        let contains: Vec<i64> = self
            .by_hero
            .iter()
            .filter(|(_, words)| words.iter().any(|w| norm(w).contains(&q)))
            .map(|(id, _)| *id)
            .collect();
        (contains.len() == 1).then(|| contains[0])
    }

    pub fn is_empty(&self) -> bool {
        self.by_hero.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{"hero":[
      {"heroId":"1","name":"黑暗之女","alias":"Annie","title":"安妮","keywords":"安妮,黑暗之女,火女,Annie,anni,heianzhinv,huonv"},
      {"heroId":"39","name":"刀锋舞者","alias":"Irelia","title":"艾瑞莉娅","keywords":"女刀,艾瑞莉娅,刀锋舞者,airuiliya"},
      {"heroId":"157","name":"疾风剑豪","alias":"Yasuo","title":"亚索","keywords":"亚索,快乐风男,yasuo"}
    ]}"#;

    #[test]
    fn nicknames_find_heroes() {
        let n = Nicknames::parse(SAMPLE);
        assert_eq!(n.find("刀妹"), Some(39), "腾讯的表里没有，补上的");
        assert_eq!(n.find("女刀"), Some(39));
        assert_eq!(n.find("火女"), Some(1));
        assert_eq!(n.find("yasuo"), Some(157));
        assert_eq!(n.find("快乐风"), Some(157), "包含也算（只有一个）");
        assert_eq!(n.find("女"), None, "一个字不按包含找");
        assert_eq!(n.find("盲僧"), None);
        assert!(Nicknames::parse(&format!("var x = {SAMPLE};"))
            .find("刀妹")
            .is_some());
        assert!(Nicknames::parse("garbage").is_empty());
        assert_eq!(n.short(39), "艾瑞莉娅");
        assert_eq!(n.label(39), "刀锋舞者 艾瑞莉娅");
        assert_eq!(n.short(9999), "英雄 9999");
    }
}
