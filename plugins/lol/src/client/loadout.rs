//! 一键导入（ADR 043 B 轮；讨论记录第二节“锁定后……一键导入（符文页、出装方案、召唤师技能）”）：选人卡片上点
//! “一键导入”，经客户端自己的本机接口（LCU）写三样东西。只在“操作客户端”开着、玩家点了按钮的时候（讨论记录第七节
//! “替玩家操作客户端：只在玩家点了按钮或打开了开关以后”）；不读游戏内存、不注入。
//!
//! 照 LeagueAkari 的“英雄数据窗”（`src/renderer/src-opgg-window/opgg/utils/loadout.ts`，MIT，2026-11-26 读的，只学做法、
//! 不抄代码）：
//!
//! - **符文页**：`GET /lol-perks/v1/inventory` 的 `canAddCustomPage` 说还能不能加一页。Akari 能加就新建一页，加不了就
//!   改第一页。我们多一步：先找名字以“Kotone”开头的那一页（上次导入的），有就改它，不多占一页；没有、能加就新建；
//!   满了就改当前那一页（不是当前页能改的就改第一页能改的），结果里说清换掉了哪一页。写完 `PUT /lol-perks/v1/currentpage`。
//! - **出装方案**：`/lol-item-sets/v1/item-sets/{summonerId}/sets`（读整份、换掉我们上次写的那一套、整份写回）。
//!   Akari 现在是写游戏目录里的文件；这里照 ADR 043 B 轮的要求只走客户端接口。我们那一套的 `uid` 固定
//!   （`kotone-<英雄>-<模式>`），同一个英雄再导入一次是换掉，不会越堆越多；玩家自己的方案一套不动。
//! - **召唤师技能**：`PATCH /lol-champ-select/v1/session/my-selection`（只在选英雄时）。闪现的位置照 Akari 的“auto”：
//!   新的两个和现在的两个位置交叉了（新的第一个是现在的第二个，或者反过来）就对调，玩家习惯放 D 还是 F 不变。
//!
//! 这里只有拼请求（纯函数，有单测）和发请求；用哪份数据在 `pushes/import.rs`。

use serde::Serialize;
use serde_json::{json, Value};

use crate::client::http::{paths as lcu_paths, Lcu};

/// 我们写的符文页、出装方案的名字都以它开头（下次导入时认得出来，换掉它而不是再加一页）。
pub const PREFIX: &str = "Kotone";

pub mod paths {
    pub const PERK_PAGES: &str = "/lol-perks/v1/pages";
    pub const PERK_INVENTORY: &str = "/lol-perks/v1/inventory";
    pub const CURRENT_PAGE: &str = "/lol-perks/v1/currentpage";
    pub const MY_SELECTION: &str = "/lol-champ-select/v1/session/my-selection";

    pub fn perk_page(id: i64) -> String {
        format!("/lol-perks/v1/pages/{id}")
    }

    pub fn item_sets(summoner_id: i64) -> String {
        format!("/lol-item-sets/v1/item-sets/{summoner_id}/sets")
    }
}

/// 召唤师峡谷、嚎哭深渊（大乱斗、海斗）的地图 id（出装方案的 `associatedMaps`）。
pub const MAP_RIFT: i64 = 11;
pub const MAP_ABYSS: i64 = 12;

/// 一页符文：主系、副系、9 个符文（主系 4 + 副系 2 + 属性 3，客户端要的顺序）。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuneSpec {
    pub primary_style: i64,
    pub sub_style: i64,
    pub perks: Vec<i64>,
}

impl RuneSpec {
    /// 数据齐不齐（不齐的不写：客户端会拒，写一半更糟）。
    pub fn complete(&self) -> bool {
        self.primary_style > 0
            && self.sub_style > 0
            && self.primary_style != self.sub_style
            && self.perks.len() == 9
            && self.perks.iter().all(|p| *p > 0)
    }
}

/// 出装方案里的一组。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ItemBlock {
    pub title: String,
    pub items: Vec<i64>,
}

/// 要导入的一整份：名字、英雄、地图、三样东西（哪样没有数据就是空的）。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Loadout {
    pub champion: i64,
    /// “Kotone 亚索 中路”：符文页和出装方案的名字。
    pub name: String,
    /// 出装方案的固定 `uid`（`kotone-<英雄>-<模式>`）。
    pub uid: String,
    pub map: i64,
    pub runes: Option<RuneSpec>,
    pub blocks: Vec<ItemBlock>,
    pub spells: Option<[i64; 2]>,
}

/// 符文写到哪一页。
#[derive(Debug, Clone, PartialEq)]
pub enum RuneTarget {
    /// 上次导入的那一页（名字以 Kotone 开头）：改它。
    Ours { id: i64, old: String },
    /// 还能加一页：新建。
    Create,
    /// 满了：换掉玩家的这一页（当前页能改就是它，不然第一页能改的）。
    Replace { id: i64, old: String },
    /// 一页能改的都没有（只有默认页）。
    NoRoom,
}

fn flag(v: &Value, key: &str) -> bool {
    v.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn name_of(v: &Value) -> String {
    v.get("name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

/// 往哪一页写（`pages`：`GET /lol-perks/v1/pages`；`inventory`：`GET /lol-perks/v1/inventory`，拿不到时 Null）。
pub fn rune_target(pages: &Value, inventory: &Value) -> RuneTarget {
    let pages: Vec<&Value> = pages
        .as_array()
        .map(|p| p.iter().collect())
        .unwrap_or_default();
    // 玩家能改的页（默认页、临时页不算）
    let editable = |p: &&&Value| {
        flag(p, "isEditable")
            && !flag(p, "isTemporary")
            && p.get("id").and_then(Value::as_i64).is_some()
    };
    if let Some(p) = pages
        .iter()
        .filter(editable)
        .find(|p| name_of(p).starts_with(PREFIX))
    {
        return RuneTarget::Ours {
            id: p["id"].as_i64().unwrap_or(0),
            old: name_of(p),
        };
    }
    let can_add = match inventory.get("canAddCustomPage").and_then(Value::as_bool) {
        Some(can) => can,
        // 老的客户端没有这一项：自己数（玩家能删的页比拥有的页数少就还能加）
        None => {
            let owned = inventory
                .get("ownedPageCount")
                .and_then(Value::as_i64)
                .unwrap_or(0);
            let custom = pages.iter().filter(|p| flag(p, "isDeletable")).count() as i64;
            owned > custom
        }
    };
    if can_add {
        return RuneTarget::Create;
    }
    let replaceable = |p: &&&Value| editable(p) && flag(p, "isDeletable");
    let target = pages
        .iter()
        .filter(replaceable)
        .find(|p| flag(p, "current"))
        .or_else(|| pages.iter().find(replaceable));
    match target {
        Some(p) => RuneTarget::Replace {
            id: p["id"].as_i64().unwrap_or(0),
            old: name_of(p),
        },
        None => RuneTarget::NoRoom,
    }
}

/// 新建 / 改一页的请求体（改的时候带 `id`）。
pub fn rune_page_body(name: &str, spec: &RuneSpec, id: Option<i64>) -> Value {
    let mut body = json!({
        "name": name,
        "primaryStyleId": spec.primary_style,
        "subStyleId": spec.sub_style,
        "selectedPerkIds": spec.perks,
        "current": true,
        "isTemporary": false,
        "isRecommendationOverride": false,
    });
    if let Some(id) = id {
        body["id"] = json!(id);
    }
    body
}

/// 一套出装方案（客户端的 `ItemSet`）。同一组里一样的装备并成一格写数量（出门的两瓶药）。
pub fn item_set(loadout: &Loadout) -> Value {
    let blocks: Vec<Value> = loadout
        .blocks
        .iter()
        .filter(|b| !b.items.is_empty())
        .map(|b| {
            let mut items: Vec<(i64, i64)> = Vec::new();
            for id in &b.items {
                match items.iter_mut().find(|(x, _)| x == id) {
                    Some((_, count)) => *count += 1,
                    None => items.push((*id, 1)),
                }
            }
            json!({
                "type": b.title,
                "items": items.iter().map(|(id, count)| json!({"id": id.to_string(), "count": count})).collect::<Vec<_>>(),
                "hideIfSummonerSpell": "",
                "showIfSummonerSpell": "",
            })
        })
        .collect();
    json!({
        "uid": loadout.uid,
        "title": loadout.name,
        "type": "custom",
        "map": "any",
        "mode": "any",
        "sortrank": 0,
        "startedFrom": "blank",
        "associatedChampions": [loadout.champion],
        "associatedMaps": [loadout.map],
        "preferredItemSlots": [],
        "blocks": blocks,
    })
}

/// 整份出装方案（`GET` 回来的，拿不到时 Null）里换上我们这一套：同一个 `uid` 的换掉，玩家自己的一套不动；
/// 我们的放在最前面。`account_id`：原来那份没有时用（当前召唤师的）。
pub fn merge_item_sets(existing: &Value, set: Value, account_id: i64, now_ms: i64) -> Value {
    let uid = set["uid"].as_str().unwrap_or("").to_string();
    let mut sets: Vec<Value> = existing["itemSets"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|s| s["uid"].as_str() != Some(uid.as_str()))
        .collect();
    sets.insert(0, set);
    json!({
        "accountId": existing["accountId"].as_i64().filter(|a| *a > 0).unwrap_or(account_id),
        "itemSets": sets,
        "timestamp": now_ms,
    })
}

/// 召唤师技能的左右（照 Akari 的“auto”）：新的两个和现在的两个交叉了就对调，闪现还在玩家原来放的那一边。
pub fn spell_order(new: [i64; 2], current: [i64; 2]) -> [i64; 2] {
    let [a, b] = new;
    if a == current[1] || b == current[0] {
        [b, a]
    } else {
        [a, b]
    }
}

/// 写一样东西的结果（页面照 `result` 选文案，`error` 是客户端回的原话）。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WriteResult {
    /// 新建了一页符文。
    Created,
    /// 改了上次导入的那一页。
    Updated,
    /// 符文页满了，换掉了玩家的一页（`old` 是它原来的名字）。
    Replaced,
    /// 出装方案、召唤师技能写好了。
    Written,
    /// 这份数据里没有这一样（海斗的数据没有符文）。
    NoData,
    /// 现在写不了（召唤师技能只在选英雄时能换）。
    NotNow,
    /// 一页能改的符文页都没有。
    NoRoom,
    Error,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Written {
    pub result: WriteResult,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// 写进去的召唤师技能（左、右）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spells: Option<[i64; 2]>,
    /// 出装方案有几组。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocks: Option<usize>,
}

impl Written {
    fn of(result: WriteResult) -> Self {
        Self {
            result,
            name: None,
            old: None,
            error: None,
            spells: None,
            blocks: None,
        }
    }

    pub fn no_data() -> Self {
        Self::of(WriteResult::NoData)
    }

    pub fn not_now() -> Self {
        Self::of(WriteResult::NotNow)
    }

    fn error(e: impl std::fmt::Display) -> Self {
        Self {
            error: Some(e.to_string()),
            ..Self::of(WriteResult::Error)
        }
    }

    /// 写成了（新建、改了、换掉、写好了）。
    pub fn ok(&self) -> bool {
        matches!(
            self.result,
            WriteResult::Created
                | WriteResult::Updated
                | WriteResult::Replaced
                | WriteResult::Written
        )
    }
}

/// 写符文页（见文件开头）。
pub async fn write_runes(lcu: &Lcu, name: &str, spec: &RuneSpec) -> Written {
    if !spec.complete() {
        return Written::no_data();
    }
    let pages = match lcu.get(paths::PERK_PAGES).await {
        Ok(p) => p,
        Err(e) => return Written::error(e),
    };
    let inventory = lcu.get(paths::PERK_INVENTORY).await.unwrap_or(Value::Null);
    let target = rune_target(&pages, &inventory);
    let (id, result, old) = match target {
        RuneTarget::NoRoom => return Written::of(WriteResult::NoRoom),
        RuneTarget::Create => match lcu
            .post(paths::PERK_PAGES, &rune_page_body(name, spec, None))
            .await
        {
            Ok(page) => (page["id"].as_i64(), WriteResult::Created, None),
            Err(e) => return Written::error(e),
        },
        RuneTarget::Ours { id, .. } => {
            match lcu
                .put(&paths::perk_page(id), &rune_page_body(name, spec, Some(id)))
                .await
            {
                Ok(_) => (Some(id), WriteResult::Updated, None),
                Err(e) => return Written::error(e),
            }
        }
        RuneTarget::Replace { id, old } => {
            match lcu
                .put(&paths::perk_page(id), &rune_page_body(name, spec, Some(id)))
                .await
            {
                Ok(_) => (Some(id), WriteResult::Replaced, Some(old)),
                Err(e) => return Written::error(e),
            }
        }
    };
    // 设成当前页（新建时请求体里的 current 有的客户端不认，照 Akari 再设一次）；设不上不算失败（页已经写好了）
    if let Some(id) = id {
        let _ = lcu.put(paths::CURRENT_PAGE, &json!(id)).await;
    }
    Written {
        name: Some(name.to_string()),
        old,
        ..Written::of(result)
    }
}

/// 写出装方案（见文件开头）。`now_ms`：写进 `timestamp`。
pub async fn write_items(lcu: &Lcu, loadout: &Loadout, now_ms: i64) -> Written {
    let blocks = loadout
        .blocks
        .iter()
        .filter(|b| !b.items.is_empty())
        .count();
    if blocks == 0 {
        return Written::no_data();
    }
    let me = match lcu.get(lcu_paths::ME).await {
        Ok(me) => me,
        Err(e) => return Written::error(e),
    };
    let Some(summoner) = me["summonerId"].as_i64().filter(|s| *s > 0) else {
        return Written::error("客户端没给召唤师编号");
    };
    let account = me["accountId"].as_i64().unwrap_or(0);
    let path = paths::item_sets(summoner);
    // 读不到（新号还没有方案时客户端可能回 404）就从空的开始
    let existing = lcu.get(&path).await.unwrap_or(Value::Null);
    let body = merge_item_sets(&existing, item_set(loadout), account, now_ms);
    match lcu.put(&path, &body).await {
        Ok(_) => Written {
            name: Some(loadout.name.clone()),
            blocks: Some(blocks),
            ..Written::of(WriteResult::Written)
        },
        Err(e) => Written::error(e),
    }
}

/// 换召唤师技能（只在选英雄时；`current`：选人接口里我现在的两个）。
pub async fn write_spells(lcu: &Lcu, spells: [i64; 2], current: [i64; 2]) -> Written {
    if spells.iter().any(|s| *s <= 0) || spells[0] == spells[1] {
        return Written::no_data();
    }
    let [spell1, spell2] = spell_order(spells, current);
    match lcu
        .patch(
            paths::MY_SELECTION,
            &json!({"spell1Id": spell1, "spell2Id": spell2}),
        )
        .await
    {
        Ok(_) => Written {
            spells: Some([spell1, spell2]),
            ..Written::of(WriteResult::Written)
        },
        Err(e) => Written::error(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> RuneSpec {
        RuneSpec {
            primary_style: 8000,
            sub_style: 8400,
            perks: vec![8010, 9111, 9104, 8299, 8444, 8473, 5005, 5008, 5001],
        }
    }

    fn page(id: i64, name: &str, current: bool, deletable: bool) -> Value {
        json!({"id": id, "name": name, "current": current, "isEditable": deletable, "isDeletable": deletable,
               "isTemporary": false, "isActive": current})
    }

    #[test]
    fn which_rune_page_gets_written() {
        let defaults = json!([
            {"id": 1, "name": "精密", "isEditable": false, "isDeletable": false},
            {"id": 2, "name": "主宰", "isEditable": false, "isDeletable": false}
        ]);
        // 只有默认页、还能加：新建
        assert_eq!(
            rune_target(&defaults, &json!({"canAddCustomPage": true})),
            RuneTarget::Create
        );
        // 只有默认页、不能加：没地方写
        assert_eq!(
            rune_target(&defaults, &json!({"canAddCustomPage": false})),
            RuneTarget::NoRoom
        );
        // 上次导入的那一页在：改它（就算还能加也不多占一页）
        let mine = json!([
            page(10, "我的亚索", false, true),
            page(11, "Kotone 亚索 中路", false, true),
            page(12, "上单", true, true),
        ]);
        assert_eq!(
            rune_target(&mine, &json!({"canAddCustomPage": true})),
            RuneTarget::Ours {
                id: 11,
                old: "Kotone 亚索 中路".into()
            }
        );
        // 满了、没有我们的：换掉当前页
        let full = json!([
            page(10, "我的亚索", false, true),
            page(12, "上单", true, true),
            {"id": 13, "name": "临时", "isEditable": true, "isDeletable": true, "isTemporary": true, "current": false},
        ]);
        assert_eq!(
            rune_target(&full, &json!({"canAddCustomPage": false})),
            RuneTarget::Replace {
                id: 12,
                old: "上单".into()
            }
        );
        // 当前页是默认页：换第一页能改的
        let mut on_default = full.clone();
        on_default[1]["current"] = json!(false);
        on_default
            .as_array_mut()
            .unwrap()
            .push(json!({"id": 1, "name": "精密", "current": true, "isEditable": false, "isDeletable": false}));
        assert_eq!(
            rune_target(&on_default, &json!({"canAddCustomPage": false})),
            RuneTarget::Replace {
                id: 10,
                old: "我的亚索".into()
            }
        );
        // 老客户端没有 canAddCustomPage：拥有 3 页、玩家的页 2 页 → 还能加
        assert_eq!(
            rune_target(&full, &json!({"ownedPageCount": 4})),
            RuneTarget::Create
        );
        assert_eq!(
            rune_target(&full, &json!({"ownedPageCount": 3})),
            RuneTarget::Replace {
                id: 12,
                old: "上单".into()
            }
        );
    }

    #[test]
    fn rune_page_and_item_set_bodies() {
        let body = rune_page_body("Kotone 亚索 中路", &spec(), Some(11));
        assert_eq!(body["selectedPerkIds"].as_array().unwrap().len(), 9);
        assert_eq!(body["primaryStyleId"], 8000);
        assert_eq!(body["subStyleId"], 8400);
        assert_eq!(body["current"], true);
        assert_eq!(body["id"], 11);
        assert!(rune_page_body("x", &spec(), None).get("id").is_none());
        assert!(spec().complete());
        let mut half = spec();
        half.perks.pop();
        assert!(!half.complete(), "少一个符文不写");

        let loadout = Loadout {
            champion: 157,
            name: "Kotone 亚索 中路".into(),
            uid: "kotone-157-rift".into(),
            map: MAP_RIFT,
            runes: Some(spec()),
            blocks: vec![
                ItemBlock {
                    title: "出门".into(),
                    items: vec![1055, 2003, 2003],
                },
                ItemBlock {
                    title: "空的".into(),
                    items: vec![],
                },
                ItemBlock {
                    title: "核心".into(),
                    items: vec![6673, 3031, 6333],
                },
            ],
            spells: Some([4, 14]),
        };
        let set = item_set(&loadout);
        assert_eq!(set["uid"], "kotone-157-rift");
        assert_eq!(set["title"], "Kotone 亚索 中路");
        assert_eq!(set["associatedChampions"], json!([157]));
        assert_eq!(set["associatedMaps"], json!([11]));
        let blocks = set["blocks"].as_array().unwrap();
        assert_eq!(blocks.len(), 2, "空的一组不写");
        assert_eq!(
            blocks[0]["items"],
            json!([{"id": "1055", "count": 1}, {"id": "2003", "count": 2}])
        );
        // 合进原来那份：同一个 uid 的换掉，玩家自己的不动，我们的在最前面
        let existing = json!({"accountId": 99, "timestamp": 1, "itemSets": [
            {"uid": "abc", "title": "我的"},
            {"uid": "kotone-157-rift", "title": "Kotone 旧的"},
        ]});
        let merged = merge_item_sets(&existing, set.clone(), 5, 777);
        let sets = merged["itemSets"].as_array().unwrap();
        assert_eq!(sets.len(), 2);
        assert_eq!(sets[0]["title"], "Kotone 亚索 中路");
        assert_eq!(sets[1]["uid"], "abc");
        assert_eq!(merged["accountId"], 99);
        assert_eq!(merged["timestamp"], 777);
        let fresh = merge_item_sets(&Value::Null, set, 5, 777);
        assert_eq!(fresh["accountId"], 5);
        assert_eq!(fresh["itemSets"].as_array().unwrap().len(), 1);
    }

    /// 本机的假客户端（照 `chat/tests.rs` 的写法，走真 HTTP）：记下每个请求，按路径回。
    mod fake {
        use std::sync::{Arc, Mutex};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        use super::*;

        /// (方法, 路径, 请求体)
        pub type Seen = Arc<Mutex<Vec<(String, String, String)>>>;

        pub async fn serve(routes: Vec<(&'static str, &'static str, u16, Value)>) -> (Lcu, Seen) {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let seen = Seen::default();
            let log = seen.clone();
            let routes = Arc::new(routes);
            tokio::spawn(async move {
                while let Ok((mut socket, _)) = listener.accept().await {
                    let (log, routes) = (log.clone(), routes.clone());
                    tokio::spawn(async move {
                        let mut buf = Vec::new();
                        let mut chunk = [0u8; 4096];
                        let head_end = loop {
                            let Ok(n) = socket.read(&mut chunk).await else {
                                return;
                            };
                            if n == 0 {
                                return;
                            }
                            buf.extend_from_slice(&chunk[..n]);
                            if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                                break i + 4;
                            }
                        };
                        let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
                        let mut words = head.split_whitespace();
                        let method = words.next().unwrap_or("").to_string();
                        let path = words.next().unwrap_or("").to_string();
                        let length: usize = head
                            .lines()
                            .find_map(|l| {
                                l.split_once(':')
                                    .filter(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                                    .map(|(_, v)| v.trim().parse().unwrap_or(0))
                            })
                            .unwrap_or(0);
                        while buf.len() < head_end + length {
                            let Ok(n) = socket.read(&mut chunk).await else {
                                break;
                            };
                            if n == 0 {
                                break;
                            }
                            buf.extend_from_slice(&chunk[..n]);
                        }
                        let body = String::from_utf8_lossy(&buf[head_end..]).to_string();
                        log.lock()
                            .unwrap()
                            .push((method.clone(), path.clone(), body));
                        let (status, value) = routes
                            .iter()
                            .find(|(m, p, _, _)| *m == method && *p == path)
                            .map(|(_, _, s, v)| (*s, v.clone()))
                            .unwrap_or((404, json!({"message": "no such path"})));
                        let data = value.to_string();
                        let head = format!(
                            "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            data.len()
                        );
                        let _ = socket.write_all(head.as_bytes()).await;
                        let _ = socket.write_all(data.as_bytes()).await;
                        let _ = socket.shutdown().await;
                    });
                }
            });
            (
                Lcu::with_base(&format!("http://127.0.0.1:{port}"), "tok"),
                seen,
            )
        }
    }

    fn loadout() -> Loadout {
        Loadout {
            champion: 157,
            name: "Kotone 亚索 中路".into(),
            uid: "kotone-157-rift".into(),
            map: MAP_RIFT,
            runes: Some(spec()),
            blocks: vec![ItemBlock {
                title: "核心三件".into(),
                items: vec![6673, 3031, 6333],
            }],
            spells: Some([4, 14]),
        }
    }

    /// 发给客户端的请求（地址和请求体）：符文页满了换掉当前页再设成当前页；出装方案读整份、换上我们的、整份写回；
    /// 召唤师技能照玩家放闪现的那一边。
    #[tokio::test]
    async fn the_requests_sent_to_the_client() {
        let (lcu, seen) = fake::serve(vec![
            ("GET", paths::PERK_PAGES, 200, json!([page(10, "我的亚索", false, true), page(12, "上单", true, true)])),
            ("GET", paths::PERK_INVENTORY, 200, json!({"canAddCustomPage": false, "ownedPageCount": 2})),
            ("PUT", "/lol-perks/v1/pages/12", 200, json!({})),
            ("PUT", paths::CURRENT_PAGE, 204, Value::Null),
            ("GET", "/lol-summoner/v1/current-summoner", 200, json!({"summonerId": 4242, "accountId": 99, "puuid": "p"})),
            ("GET", "/lol-item-sets/v1/item-sets/4242/sets", 200, json!({"accountId": 99, "timestamp": 1, "itemSets": [{"uid": "mine", "title": "我的"}]})),
            ("PUT", "/lol-item-sets/v1/item-sets/4242/sets", 201, json!({})),
            ("PATCH", paths::MY_SELECTION, 204, Value::Null),
        ])
        .await;
        let l = loadout();
        let runes = write_runes(&lcu, &l.name, l.runes.as_ref().unwrap()).await;
        assert_eq!(runes.result, WriteResult::Replaced);
        assert_eq!(runes.old.as_deref(), Some("上单"));
        let items = write_items(&lcu, &l, 777).await;
        assert_eq!(items.result, WriteResult::Written, "{items:?}");
        assert_eq!(items.blocks, Some(1));
        let spells = write_spells(&lcu, [4, 14], [12, 4]).await;
        assert_eq!(spells.spells, Some([14, 4]));
        let seen = seen.lock().unwrap().clone();
        let sent: Vec<(&str, &str)> = seen
            .iter()
            .map(|(m, p, _)| (m.as_str(), p.as_str()))
            .collect();
        assert_eq!(
            sent,
            [
                ("GET", paths::PERK_PAGES),
                ("GET", paths::PERK_INVENTORY),
                ("PUT", "/lol-perks/v1/pages/12"),
                ("PUT", paths::CURRENT_PAGE),
                ("GET", "/lol-summoner/v1/current-summoner"),
                ("GET", "/lol-item-sets/v1/item-sets/4242/sets"),
                ("PUT", "/lol-item-sets/v1/item-sets/4242/sets"),
                ("PATCH", paths::MY_SELECTION),
            ]
        );
        let body = |i: usize| serde_json::from_str::<Value>(&seen[i].2).unwrap();
        assert_eq!(body(2)["name"], "Kotone 亚索 中路");
        assert_eq!(body(2)["id"], 12);
        assert_eq!(body(2)["selectedPerkIds"].as_array().unwrap().len(), 9);
        assert_eq!(body(3), json!(12));
        let sets = body(6);
        assert_eq!(sets["itemSets"].as_array().unwrap().len(), 2);
        assert_eq!(sets["itemSets"][0]["uid"], "kotone-157-rift");
        assert_eq!(sets["itemSets"][1]["uid"], "mine", "玩家自己的方案不动");
        assert_eq!(body(7), json!({"spell1Id": 14, "spell2Id": 4}));
    }

    /// 还能加一页：新建（POST，带 current），再设成当前页；客户端拒了时说出原话。
    #[tokio::test]
    async fn a_new_page_when_there_is_room_and_plain_errors() {
        let (lcu, seen) = fake::serve(vec![
            ("GET", paths::PERK_PAGES, 200, json!([])),
            (
                "GET",
                paths::PERK_INVENTORY,
                200,
                json!({"canAddCustomPage": true}),
            ),
            (
                "POST",
                paths::PERK_PAGES,
                200,
                json!({"id": 77, "name": "Kotone 亚索 中路"}),
            ),
            ("PUT", paths::CURRENT_PAGE, 204, Value::Null),
            (
                "PATCH",
                paths::MY_SELECTION,
                400,
                json!({"message": "not in champ select"}),
            ),
        ])
        .await;
        let runes = write_runes(&lcu, "Kotone 亚索 中路", &spec()).await;
        assert_eq!(runes.result, WriteResult::Created);
        let spells = write_spells(&lcu, [4, 14], [4, 12]).await;
        assert_eq!(spells.result, WriteResult::Error);
        assert!(spells.error.unwrap().contains("not in champ select"));
        // 拿不到召唤师：出装方案没写成
        let items = write_items(&lcu, &loadout(), 1).await;
        assert_eq!(items.result, WriteResult::Error);
        // 数据不全的不发请求
        let mut half = spec();
        half.perks.truncate(5);
        let before = seen.lock().unwrap().len();
        assert_eq!(
            write_runes(&lcu, "x", &half).await.result,
            WriteResult::NoData
        );
        assert_eq!(
            write_spells(&lcu, [4, 4], [0, 0]).await.result,
            WriteResult::NoData
        );
        assert_eq!(seen.lock().unwrap().len(), before);
        let seen = seen.lock().unwrap().clone();
        assert_eq!(seen[2].0, "POST");
        let created: Value = serde_json::from_str(&seen[2].2).unwrap();
        assert_eq!(created["current"], true);
        assert!(created.get("id").is_none());
        assert_eq!(seen[3].2, "77");
    }

    #[test]
    fn flash_stays_where_the_player_keeps_it() {
        // 玩家闪现放 F（第二个）：新的“闪现 + 点燃”对调成“点燃 + 闪现”
        assert_eq!(spell_order([4, 14], [12, 4]), [14, 4]);
        // 闪现放 D：照旧
        assert_eq!(spell_order([4, 14], [4, 12]), [4, 14]);
        // 没交叉：照数据的顺序
        assert_eq!(spell_order([4, 14], [6, 7]), [4, 14]);
    }
}
