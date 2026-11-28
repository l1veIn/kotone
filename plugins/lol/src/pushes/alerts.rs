//! 对局中的提醒（ADR 043 B 轮第 2 条，讨论记录第二节推送清单里“对局中”那几行）：都从局内接口（`127.0.0.1:2999`，
//! 只读）来，**只说事实**，说法固定（几个说法轮着用），不叫模型（ADR 043 D2、D3）。主循环每 2 秒一轮，比这一份和上一份
//! 局内数据（[`events::Roster`]）：
//!
//! | 推送（清单 id） | 什么时候 | 样子 | 多要紧 |
//! |---|---|---|---|
//! | `enemy-ult` | 你、双方全图 / 远距离大招的英雄（[`LONG_ULTS`]）、峡谷里你的对线对手到 6 级 | 语音 + 一行 | 普通 |
//! | `enemy-item`（出成装了） | 双方谁出了一件成装（[`events::completed`]，和 `lol.item` 同一个判断；说的时候用玩家的叫法 [`SHORT_ITEMS`]）；复活甲、中娅、水银、女妖、巨蛇之牙（[`KEY_ITEMS`]，水银饰带不是成装也算）；谁身上第一次有了带重伤的装备（[`GRIEVOUS`]，小件也算） | 对面的成装、关键装备、重伤：语音 + 一行；队友和你自己别的成装：只有一行 | 低 |
//! | `spell-ready` | 你或她记下的召唤师技能（`lol.spell`）转好了 | 语音 + 一行 | 普通 |
//! | `objective` | 峡谷的小龙、远古龙、虚空巢虫、峡谷先锋、大龙刷新前 30 秒（[`RIFT_RULES`]，只在峡谷，快速模式不报）；说游戏里的时间（“小龙 12 分 30 秒刷新”） | 语音 + 一行 | 普通 |
//! | `enemies-down` | 对面阵亡、还要 30 秒以上才复活的（[`DOWN_RESPAWN`]）这一份刚到 3 个（60 秒内只说一次） | 语音 + 一行 | 要紧 |
//! | `my-death` | 你阵亡时：对面每个人的等级、KDA、装备（按装备总价排），上次阵亡以后新出的标出来 | 卡片（一局一张，每次阵亡原地更新、再弹一次） | 普通 |
//!
//! 谁先说、20 秒冷却、过时不说由宿主管（`kotone_agent::push_voice`）：同一轮里来了几条，宿主只说最要紧的那句，别的只显示
//! 一行、记进聊天。插件这边只管同一件事不重复（同一个人到 6 级只说一次、同一个人同一件装备只说一次）。大乱斗、海斗没有分路
//! 和峡谷资源：只有“关键英雄到 6 级”“出成装了”“技能转好”“对面阵亡”“你阵亡”。
//!
//! 等级、装备、阵亡这几类游戏事件（`lol.level`、`lol.item`、`lol.death` 和“我阵亡了 / 我复活了”，给自动化和她）也在
//! 这里报，规矩在 `client/events.rs`：阵亡和“你阵亡时”的卡片、“对面阵亡”用的是同一份前后数据、同一个判断（ADR 046 D7）。

use std::collections::{HashMap, HashSet};

use kotone_core::log;
use kotone_plugin_api::{Host, Push, PushCard, PushCell, PushLine, PushOutcome, PushTone};
use serde_json::{json, Map, Value};

use super::PAGE_ENEMY_BUILD;
use crate::client::events::{self, base_item, Roster};
use crate::client::live::{LiveGame, LiveItem, LivePlayer};
use crate::client::spells::SpellUse;
use crate::client::GameData;
use crate::LolPlugin;

/// 清单 `[[pushes]]` 里这几条的 id。
pub(crate) const PUSH_ENEMY_ULT: &str = "enemy-ult";
pub(crate) const PUSH_ENEMY_ITEM: &str = "enemy-item";
pub(crate) const PUSH_SPELL_READY: &str = "spell-ready";
pub(crate) const PUSH_OBJECTIVE: &str = "objective";
pub(crate) const PUSH_ENEMIES_DOWN: &str = "enemies-down";
pub(crate) const PUSH_MY_DEATH: &str = "my-death";
/// 阵亡那张卡片的名字。
pub(crate) const DEATH_CARD_TITLE: &str = "对面的装备";

/// 全图或远距离大招的英雄（英文 id，说的时候的叫法）。讨论记录第二节点名的卡牌、梦魇、潘森、慎、死歌、加里奥、寒冰、
/// 金克丝、EZ、德莱文、赛娜，加上同样是全图 / 远距离的几个。叫法是国服玩家常用的外号。
pub(crate) const LONG_ULTS: &[(&str, &str)] = &[
    ("TwistedFate", "卡牌"),
    ("Nocturne", "梦魇"),
    ("Pantheon", "潘森"),
    ("Shen", "慎"),
    ("Karthus", "死歌"),
    ("Galio", "加里奥"),
    ("Ashe", "寒冰"),
    ("Jinx", "金克丝"),
    ("Ezreal", "EZ"),
    ("Draven", "德莱文"),
    ("Senna", "赛娜"),
    ("Soraka", "星妈"),
    ("Gangplank", "船长"),
    ("Ziggs", "炸弹人"),
    ("Xerath", "泽拉斯"),
    ("Jhin", "烬"),
    ("Caitlyn", "女警"),
    ("Taliyah", "岩雀"),
    ("Ryze", "瑞兹"),
    ("RekSai", "挖掘机"),
    ("Quinn", "奎因"),
    ("Sion", "塞恩"),
    ("Smolder", "斯莫德"),
];

/// 关键装备（装备 id 的后四位，说的时候的叫法）：复活甲、中娅、水银（饰带、弯刀、密银黎明都带净化）、女妖、
/// 巨蛇之牙（破护盾）。出了就说，不管你玩什么英雄。
pub(crate) const KEY_ITEMS: &[(i64, &str)] = &[
    (3026, "复活甲"),
    (3157, "中娅"),
    (3140, "水银"),
    (3139, "水银弯刀"),
    (6035, "密银黎明"),
    (3102, "女妖"),
    (6695, "巨蛇之牙"),
];

/// 说装备时玩家的叫法（后四位 → 叫法），没有的用客户端里的名字。只放国服玩家都这么叫、不会听错的；2026-11-28 对过
/// 客户端的装备资料（2026-11-23 录的 `items.json`）：都是成装（总价 2000 以上、不再合成别的）。[`KEY_ITEMS`] 里的
/// 不在这里（用那边的叫法）。
pub(crate) const SHORT_ITEMS: &[(i64, &str)] = &[
    (3031, "无尽"),     // 无尽之刃
    (3089, "帽子"),     // 灭世者的死亡之帽
    (3072, "饮血"),     // 饮血剑
    (3153, "破败"),     // 破败王者之刃
    (3078, "三相"),     // 三相之力
    (6655, "卢登"),     // 卢登的回声
    (3143, "兰顿"),     // 兰顿之兆
    (3068, "日炎"),     // 日炎圣盾
    (3071, "黑切"),     // 黑色切割者
    (3193, "石像鬼"),   // 石像鬼石板甲
    (3065, "振奋"),     // 振奋盔甲
    (6662, "冰拳"),     // 冰脉护手
    (3135, "虚空"),     // 虚空之杖
    (3083, "狂徒"),     // 狂徒铠甲
    (3110, "冰心"),     // 冰霜之心
    (3742, "板甲"),     // 亡者的板甲
    (3075, "反甲"),     // 荆棘之甲
    (3165, "莫雷洛"),   // 莫雷洛秘典
    (6609, "链锯剑"),   // 炼金朋克链锯剑
    (3124, "鬼索"),     // 鬼索的狂暴之刃
    (3115, "纳什"),     // 纳什之牙
    (3156, "饮魔刀"),   // 玛莫提乌斯之噬
    (3053, "血手"),     // 斯特拉克的挑战护手
    (3036, "大米"),     // 多米尼克领主的致意
    (3046, "幻影"),     // 幻影之舞
    (3094, "火炮"),     // 疾射火炮
    (3085, "飓风"),     // 卢安娜的飓风
    (3100, "巫妖"),     // 巫妖之祸
    (3003, "大天使"),   // 大天使之杖
    (3142, "幽梦"),     // 幽梦之灵
    (3087, "电刀"),     // 斯塔缇克电刃
    (3190, "鸟盾"),     // 钢铁烈阳之匣
    (3152, "火箭腰带"), // 海克斯科技火箭腰带
    (3146, "科技枪"),   // 海克斯科技枪刃
    (3116, "冰杖"),     // 瑞莱的冰晶节杖
    (3748, "巨九"),     // 巨型九头蛇
    (3074, "九头蛇"),   // 贪欲九头蛇
    (6672, "海妖"),     // 海妖杀手
    (6673, "盾弓"),     // 不朽盾弓
    (6653, "兰德里"),   // 兰德里的折磨
    (4633, "裂隙"),     // 裂隙制造者
    (6665, "贾修"),     // 千变者贾修
    (6694, "赛瑞尔达"), // 赛瑞尔达的怨恨
];

/// 带重伤的装备（后四位）：死刑宣告、凡性的提醒、炼金朋克链锯剑、湮灭宝珠、莫雷洛秘典、炼金科技纯化器（已经不在商店里，
/// 留着不碍事）、棘刺背心、荆棘之甲。2026-11-27 对过客户端的装备资料（2026-11-23 录的 `items.json`，863 件）：说明里
/// 写着重伤的就这 8 件，海斗、斗魂竞技场的是同一件前面多两位（`base_item`）。
pub(crate) const GRIEVOUS: &[i64] = &[3123, 3033, 6609, 3916, 3165, 3011, 3076, 3075];

/// 峡谷的分路（局内接口的 `position`）→ 说的时候的叫法。
const POSITIONS: [(&str, &str); 5] = [
    ("TOP", "上单"),
    ("JUNGLE", "打野"),
    ("MIDDLE", "中单"),
    ("BOTTOM", "下路"),
    ("UTILITY", "辅助"),
];

/// 峡谷资源的刷新规则（游戏时间，秒）。按赛季写成数据（讨论记录第二节“刷新规则按赛季写成数据”）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RiftRules {
    /// 小龙第一次刷新、被拿以后多久再刷。
    pub dragon_first: f64,
    pub dragon_respawn: f64,
    /// 一边拿到几条小龙就有龙魂，之后只刷远古龙：拿到龙魂后多久刷、远古龙被拿以后多久再刷。
    pub soul_at: usize,
    pub elder_after_soul: f64,
    pub elder_respawn: f64,
    /// 虚空巢虫（一波，不再刷）、峡谷先锋（一次，不再刷）。
    pub grubs: Option<f64>,
    pub herald: Option<f64>,
    /// 大龙第一次刷新、被拿以后多久再刷。
    pub baron_first: f64,
    pub baron_respawn: f64,
}

/// 2026 赛季（26.1 起）的峡谷：拳头补丁说明 26.1 “Baron Nashor: 25:00 ⇒ 20:00”、去掉了阿塔坎，“Epic Monster spawn
/// times are unchanged unless noted above”；25.09 起峡谷先锋 15:00（英文维基 Rift_Herald 的版本记录 V25.09：“Spawn
/// time reduced to 15:00 from 16:00”，19:45 消失）、虚空巢虫 8:00 只刷一波；小龙 5:00、被拿以后 5 分钟，龙魂以后
/// 6 分钟刷远古龙、远古龙被拿以后 6 分钟，大龙被拿以后 6 分钟，多年没变。2026-11-26 查、11-28 对过维基（先锋原来写成
/// 16:00 是错的），以后的版本改了就加一份。快速模式的刷新时间不一样，不用这一份（[`rift_timers`]）。
pub const RIFT_RULES: RiftRules = RiftRules {
    dragon_first: 300.0,
    dragon_respawn: 300.0,
    soul_at: 4,
    elder_after_soul: 360.0,
    elder_respawn: 360.0,
    grubs: Some(480.0),
    herald: Some(900.0),
    baron_first: 1200.0,
    baron_respawn: 360.0,
};

/// 资源刷新前多久提醒。
pub const OBJECTIVE_LEAD: f64 = 30.0;
/// “对面阵亡”只数还要这么久以上才复活的（局内接口的剩余复活秒数）：这时候我方才有空打大龙、推一波。复活得这么慢
/// 本身就说明是后期，不另加游戏时间的门槛；峡谷、大乱斗、海斗同一条。
pub const DOWN_RESPAWN: f64 = 30.0;
/// 对面这样阵亡的有几个才提醒。
pub const DOWN_AT: usize = 3;
/// “对面阵亡”两次之间至少隔多久（游戏时间，秒）：一波团战只说一次。
pub const DOWN_GAP: f64 = 60.0;

/// 峡谷（只有它有分路和峡谷资源；大乱斗是 `ARAM`、海斗是 `KIWI`）。
pub fn is_rift(game: &LiveGame) -> bool {
    game.mode.eq_ignore_ascii_case("CLASSIC")
}

/// 快速模式的队列（客户端的 `queues.json`：480 “快速模式”；拳头开发者文档的 `queues.json`：480 “Swiftplay Games”）。
pub const SWIFTPLAY_QUEUE: i64 = 480;

/// 照 [`RIFT_RULES`] 报峡谷资源：峡谷、不是快速模式。快速模式也在峡谷打，资源刷得早，刷新时间这里没有——宁可不报也不报错的。
/// 拳头的模式表里快速模式的 `gameMode` 是 `SWIFTPLAY`（[`is_rift`] 已经不算），没录到局内接口实际写什么，所以再按
/// 这局的队列（`queue`，客户端的对局信息里来，不知道时 None）认一次。
pub fn rift_timers(game: &LiveGame, queue: Option<i64>) -> bool {
    is_rift(game) && queue != Some(SWIFTPLAY_QUEUE)
}

fn position_name(position: &str) -> Option<&'static str> {
    POSITIONS
        .iter()
        .find(|(p, _)| p.eq_ignore_ascii_case(position))
        .map(|(_, name)| *name)
}

/// 说的时候怎么叫这个人：有外号的用外号，否则用英雄的短名（“亚索”）。
fn spoken(p: &LivePlayer, data: Option<&GameData>) -> String {
    LONG_ULTS
        .iter()
        .find(|(alias, _)| alias.eq_ignore_ascii_case(&p.alias))
        .map(|(_, name)| name.to_string())
        .unwrap_or_else(|| crate::client::text::champion_names(p, data).1)
}

/// 峡谷里你的对线对手（同一个分路的对面那个人）。
fn lane_opponent(game: &LiveGame) -> Option<&LivePlayer> {
    let mine = game.me()?.position.clone();
    position_name(&mine)?;
    game.enemies()
        .find(|p| p.position.eq_ignore_ascii_case(&mine))
}

/// 一段话的几个说法里，留下不超过 [`Push::VOICE_MAX`] 个字的（太长的宿主会截断，宁可不用）。
fn fit(voice: Vec<String>) -> Vec<String> {
    voice
        .into_iter()
        .filter(|v| v.chars().count() <= Push::VOICE_MAX)
        .collect()
}

fn cell(text: impl Into<String>, tone: Option<PushTone>) -> PushCell {
    PushCell {
        text: text.into(),
        tone,
    }
}

fn compact_cell(text: &str, tone: PushTone) -> PushCell {
    cell(
        text.chars().take(PushCell::MAX_CHARS).collect::<String>(),
        Some(tone),
    )
}

fn summarize_names(names: &[String]) -> String {
    if names.len() > 2 {
        format!("{}等 {} 个", names[..2].join("、"), names.len())
    } else {
        names.join("、")
    }
}

fn line_push(trigger: &str, key: String, cells: Vec<PushCell>, voice: Vec<String>) -> Push {
    Push {
        trigger: trigger.into(),
        key,
        line: Some(PushLine { cells }),
        voice: fit(voice),
        ..Push::default()
    }
}

/// 出装备的一条：谁（“亚索”“我方金克丝”“你”）、出了什么（叫法或“重伤”）、说不说出来。
type Found = (String, String, bool);

/// 说的时候怎么叫这件装备：关键装备、玩家的叫法（[`SHORT_ITEMS`]，海斗、斗魂竞技场的按后四位），没有的用局内接口里的
/// 名字、客户端的装备资料里的名字。
fn item_spoken(item: &LiveItem, data: Option<&GameData>) -> String {
    let base = base_item(item.id);
    KEY_ITEMS
        .iter()
        .chain(SHORT_ITEMS)
        .find(|(id, _)| *id == base)
        .map(|(_, name)| name.to_string())
        .or_else(|| Some(item.name.trim().to_string()).filter(|n| !n.is_empty()))
        .or_else(|| {
            data.and_then(|d| d.items.get(&item.id).or_else(|| d.items.get(&base)))
                .cloned()
        })
        .unwrap_or_else(|| "成装".into())
}

/// 出装备那一行：两边都有时对面第一个、我方第一个打头，最多三格；只有一边时只有一条写成一句（“对面亚索出了无尽”）。
fn item_cells(enemies: &[Found], allies: &[Found]) -> Vec<PushCell> {
    if let (Some(enemy), Some(ally)) = (enemies.first(), allies.first()) {
        let enemy_cell =
            |(who, what, _): &Found| compact_cell(&format!("对面{who} {what}"), PushTone::Bad);
        let ally_cell =
            |(who, what, _): &Found| compact_cell(&format!("{who} {what}"), PushTone::Good);
        return [enemy_cell(enemy), ally_cell(ally)]
            .into_iter()
            .chain(enemies.iter().skip(1).map(enemy_cell))
            .chain(allies.iter().skip(1).map(ally_cell))
            .take(PushLine::MAX_CELLS)
            .collect();
    }
    let (found, side, tone) = if enemies.is_empty() {
        (allies, "", PushTone::Good)
    } else {
        (enemies, "对面", PushTone::Bad)
    };
    found
        .iter()
        .take(PushLine::MAX_CELLS)
        .enumerate()
        .map(|(i, (who, what, _))| {
            if found.len() == 1 {
                let text = format!("{side}{who}出了{what}");
                cell(
                    text.chars()
                        .take(PushCell::ONE_CELL_MAX_CHARS)
                        .collect::<String>(),
                    Some(tone),
                )
            } else {
                let text = format!("{side}{who} {what}");
                cell(
                    text.chars().take(PushCell::MAX_CHARS).collect::<String>(),
                    (i == 0).then_some(tone),
                )
            }
        })
        .collect()
}

/// 出装备那一句的几个说法（只用要说出来的那几条；一条都没有 = 不说，只显示一行）。
fn item_voice(enemies: &[Found], allies: &[Found]) -> Vec<String> {
    if let (Some((enemy, enemy_item, _)), Some((ally, ally_item, _))) =
        (enemies.first(), allies.first())
    {
        return vec![
            format!("对面{enemy}出{enemy_item}，{ally}出{ally_item}"),
            "双方都出了新装备".into(),
        ];
    }
    let (found, side) = if enemies.is_empty() {
        (allies, "")
    } else {
        (enemies, "对面")
    };
    let Some((who, what, _)) = found.first() else {
        return Vec::new();
    };
    let more = if found.len() > 1 {
        format!("，还有 {} 件", found.len() - 1)
    } else {
        String::new()
    };
    vec![
        format!("{side}{who}出了{what}{more}"),
        if side.is_empty() {
            format!("{who}出{what}了{more}")
        } else {
            format!("{who}出了{what}{more}")
        },
        format!("{side}{who}有{what}了"),
    ]
}

/// 说的时候的游戏时间：`750` 秒 → “12 分 30 秒”，整分 → “20 分”。
fn spoken_clock(seconds: f64) -> String {
    let total = seconds.max(0.0).round() as i64;
    match total % 60 {
        0 => format!("{} 分", total / 60),
        s => format!("{} 分 {s} 秒", total / 60),
    }
}

/// 一种峡谷资源。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Objective {
    Dragon,
    Elder,
    Grubs,
    Herald,
    Baron,
}

impl Objective {
    fn id(self) -> &'static str {
        match self {
            Objective::Dragon => "dragon",
            Objective::Elder => "elder",
            Objective::Grubs => "grubs",
            Objective::Herald => "herald",
            Objective::Baron => "baron",
        }
    }

    fn name(self) -> &'static str {
        match self {
            Objective::Dragon => "小龙",
            Objective::Elder => "远古龙",
            Objective::Grubs => "虚空巢虫",
            Objective::Herald => "峡谷先锋",
            Objective::Baron => "大龙",
        }
    }
}

/// 局内接口的事件列表全不全：`EventID` 从 0（`GameStart`）起一个挨一个（录过的几局都是这样）。中间缺了的（断线重连、
/// 接口漏了）不知道缺的是不是拿龙、拿大龙，龙和大龙的时间就不算了。
fn events_complete(game: &LiveGame) -> bool {
    let mut ids: Vec<i64> = game.events.iter().map(|e| e.id).collect();
    ids.sort_unstable();
    ids.iter().enumerate().all(|(i, id)| *id == i as i64)
}

/// 照局内接口的事件（拿龙、拿大龙）和规则算：接下来每种资源什么时候刷（游戏时间）。只给峡谷用。
///
/// 龙魂：有一边拿到 [`RiftRules::soul_at`] 条小龙（远古龙不算）就有了，之后只刷远古龙，所以有龙魂时**最后一条小龙就是
/// 龙魂那条**。拿龙的人是哪边看 `KillerName`（[`LiveGame::is_ally_name`]：名字不带编号、隐藏了身份的是英雄的称号，
/// 都认得）；认不出的那条两边都可能是：
/// - 拿过远古龙、有一边认得出的已经到了、或者总共拿了 `2 × soul_at − 1` 条（两边加起来，总有一边到了）→ 有龙魂；
/// - 认不出的都算给一边也到不了 → 没有，照旧刷小龙；
/// - 别的说不准 → 这一项不给（宁可不报，也不把远古龙说成小龙）。
///
/// 事件列表不全时（[`events_complete`]）龙和大龙都不给，虚空巢虫、峡谷先锋只刷一次、时间固定，照给。
pub fn next_spawns(game: &LiveGame, rules: &RiftRules) -> Vec<(Objective, f64)> {
    let mut out = Vec::new();
    let complete = events_complete(game);
    // 每边拿了几条小龙（我方，对面），认不出是哪边的几条
    let mut sides = [0usize; 2];
    let mut unknown = 0usize;
    let mut last_dragon: Option<f64> = None;
    let mut last_elder: Option<f64> = None;
    let mut last_baron: Option<f64> = None;
    let mut events: Vec<_> = game.events.iter().collect();
    events.sort_by(|a, b| a.time.total_cmp(&b.time));
    for e in events {
        match e.name.as_str() {
            "DragonKill" if e.detail.starts_with("Elder") => last_elder = Some(e.time),
            "DragonKill" => {
                last_dragon = Some(e.time);
                match game.is_ally_name(&e.killer) {
                    Some(true) => sides[0] += 1,
                    Some(false) => sides[1] += 1,
                    None => unknown += 1,
                }
            }
            "BaronKill" => last_baron = Some(e.time),
            _ => {}
        }
    }
    let total = sides[0] + sides[1] + unknown;
    let soul = if last_elder.is_some()
        || sides.iter().any(|n| *n >= rules.soul_at)
        || total + 1 >= 2 * rules.soul_at
    {
        Some(true)
    } else if sides.iter().all(|n| n + unknown < rules.soul_at) {
        Some(false)
    } else {
        None
    };
    let dragon = match soul {
        // 龙魂那条 = 最后一条小龙（之后不再刷小龙）；只看到远古龙、没看到小龙的，按远古龙算
        Some(true) => match (last_dragon, last_elder) {
            (Some(soul), Some(t)) if t < soul => Some(soul + rules.elder_after_soul),
            (_, Some(t)) => Some(t + rules.elder_respawn),
            (Some(soul), None) => Some(soul + rules.elder_after_soul),
            (None, None) => None,
        }
        .map(|t| (Objective::Elder, t)),
        Some(false) => Some((
            Objective::Dragon,
            last_dragon.map_or(rules.dragon_first, |t| t + rules.dragon_respawn),
        )),
        None => None,
    };
    if let Some(d) = dragon.filter(|_| complete) {
        out.push(d);
    }
    if let Some(t) = rules.grubs {
        out.push((Objective::Grubs, t));
    }
    if let Some(t) = rules.herald {
        out.push((Objective::Herald, t));
    }
    if complete {
        out.push((
            Objective::Baron,
            last_baron.map_or(rules.baron_first, |t| t + rules.baron_respawn),
        ));
    }
    out
}

/// 一轮看出来的：要推的、要报的游戏事件（类型，数据，游戏时间）。
#[derive(Debug, Default)]
pub struct Out {
    pub pushes: Vec<Push>,
    pub events: Vec<(&'static str, Map<String, Value>, f64)>,
}

/// 这局提醒过什么（一局一份；换了一局从头来）。
#[derive(Debug, Default)]
pub struct Alerts {
    game: Option<i64>,
    roster: Option<Roster>,
    /// 每个人这局第一次看到时的样子（[`LivePlayer::key`]）：阵亡事件里的英雄名按它（佛耶戈附身时还是佛耶戈）。
    first_seen: HashMap<String, LivePlayer>,
    /// 到 6 级已经说过的人（[`LivePlayer::key`]）。
    ult_said: HashSet<String>,
    /// 说过的、或者早就在身上的装备（`<人>:<装备后四位>`，重伤是 `<人>:grievous`）：同一个人同一件只说一次，刚连上时
    /// 身上已经有的不说（[`Alerts::remember_items`]）。
    item_said: HashSet<String>,
    /// 说过的峡谷资源（推送的 key：哪种、几点刷）。
    objective_said: HashSet<String>,
    /// 报过装备事件的（`<人>:<装备 id>`）。
    item_reported: HashSet<String>,
    /// 上一次说“对面死了几个”的游戏时间。
    down_at: Option<f64>,
    /// 这局阵亡了几次。
    deaths: u32,
    /// 上一次阵亡时对面每个人身上的装备（下一次阵亡时标出新出的）。
    at_last_death: Option<HashMap<String, Vec<i64>>>,
}

impl Alerts {
    /// 看一份局内数据（`game_id` 是这局的对局 id，没有时 0；`queue` 是这局的队列，不知道时 None）：和上一份比，出了
    /// 什么事。第一次看到这局只记下样子。
    pub fn step(
        &mut self,
        game_id: i64,
        queue: Option<i64>,
        game: &LiveGame,
        data: Option<&GameData>,
        spells: &[SpellUse],
    ) -> Out {
        let restarted = self
            .roster
            .as_ref()
            .is_some_and(|r| game.time + 5.0 < r.time);
        if self.game != Some(game_id) || restarted {
            *self = Self {
                game: Some(game_id),
                ..Self::default()
            };
        }
        for p in &game.players {
            self.first_seen.entry(p.key()).or_insert_with(|| p.clone());
        }
        let now = Roster::of(game);
        let Some(before) = self.roster.replace(now) else {
            self.remember_items(game);
            return Out::default();
        };
        let changes = events::changes(&before, game);
        let mut out = Out {
            events: events::death_events(&before, game, &self.first_seen, data),
            pushes: Vec::new(),
        };
        out.events.extend(events::roster_events(
            &changes,
            game,
            data,
            &mut self.item_reported,
        ));
        let key = |kind: &str| format!("{kind}:{game_id}:{}", game.time.round() as i64);
        if let Some(push) = self.level_six(&changes, game, data, key(PUSH_ENEMY_ULT)) {
            out.pushes.push(push);
        }
        if let Some(push) = self.key_items(&before, &changes, game, data, key(PUSH_ENEMY_ITEM)) {
            out.pushes.push(push);
        }
        self.remember_items(game);
        out.pushes
            .extend(spells_ready(before.time, game, spells, game_id));
        if rift_timers(game, queue) {
            let due = objectives(game, &RIFT_RULES, game_id);
            out.pushes.extend(
                due.into_iter()
                    .filter(|p| self.objective_said.insert(p.key.clone())),
            );
        }
        if let Some(push) = self.enemies_down(&before, game, key(PUSH_ENEMIES_DOWN)) {
            out.pushes.push(push);
        }
        if let Some(push) = self.my_death(&before, game, data, game_id) {
            out.pushes.push(push);
        }
        out
    }

    /// 你、双方远距离大招的英雄和你的对线对手到 6 级。同一轮的并成一条。
    fn level_six(
        &mut self,
        changes: &events::Changes,
        game: &LiveGame,
        data: Option<&GameData>,
        key: String,
    ) -> Option<Push> {
        let my_team = game.my_team();
        if my_team.is_empty() {
            return None;
        }
        let opponent = lane_opponent(game).map(LivePlayer::key);
        let mut enemies = Vec::new();
        let mut allies = Vec::new();
        let mut self_leveled = false;
        for up in &changes.levels {
            let p = up.player;
            if p.team.is_empty() || !(up.from < 6 && p.level >= 6) {
                continue;
            }
            let is_me = p.riot_id == game.me;
            let long = LONG_ULTS
                .iter()
                .any(|(alias, _)| alias.eq_ignore_ascii_case(&p.alias));
            let player_key = p.key();
            let lane = opponent.as_deref() == Some(player_key.as_str());
            if !(is_me || long || lane) || !self.ult_said.insert(player_key) {
                continue;
            }
            if is_me {
                self_leveled = true;
            } else if p.team == my_team {
                allies.push(spoken(p, data));
            } else {
                // 对线对手没有外号时按分路叫（“对面中单”）。
                enemies.push(if lane && !long {
                    position_name(&p.position)
                        .map(str::to_string)
                        .unwrap_or_else(|| spoken(p, data))
                } else {
                    spoken(p, data)
                });
            }
        }
        let enemy_names = summarize_names(&enemies);
        let allied_names = match (self_leveled, allies.is_empty()) {
            (true, true) => "你".to_string(),
            (true, false) => format!("你和我方{}", summarize_names(&allies)),
            (false, false) => format!("我方{}", summarize_names(&allies)),
            (false, true) => String::new(),
        };
        let mut cells = Vec::new();
        let voice = match (enemies.is_empty(), allied_names.is_empty()) {
            (true, true) => return None,
            (true, false) => {
                cells.push(compact_cell(&allied_names, PushTone::Good));
                vec![
                    format!("{allied_names} 6 级了"),
                    format!("{allied_names}有大招了"),
                    format!("{allied_names}到 6 级了"),
                ]
            }
            (false, true) => {
                cells.push(compact_cell(&format!("对面{enemy_names}"), PushTone::Bad));
                vec![
                    format!("对面{enemy_names} 6 级了"),
                    format!("对面{enemy_names}有大招了"),
                    format!("{enemy_names}到 6 级了"),
                ]
            }
            (false, false) => {
                cells.push(compact_cell(&format!("对面{enemy_names}"), PushTone::Bad));
                cells.push(compact_cell(&allied_names, PushTone::Good));
                vec![
                    format!("对面{enemy_names}、{allied_names} 6 级了"),
                    "双方都有人到 6 级了".into(),
                ]
            }
        };
        cells.push(cell("6 级了，有大招", None));
        Some(line_push(PUSH_ENEMY_ULT, key, cells, voice))
    }

    /// 这个人现在是不是佛耶戈附身成了别人（英雄和这局第一次看到时不一样）：附身时身上是被附身那个人的装备，附身完又变回来，
    /// 都不算他出了装备。
    fn possessing(&self, p: &LivePlayer) -> bool {
        self.first_seen
            .get(&p.key())
            .is_some_and(|first| !first.alias.eq_ignore_ascii_case(&p.alias))
    }

    /// 每个人现在身上的装备都记成“说过了”（附身时的不记）：刚连上时身上已经有的、卖了又买回来的，都不再说。
    fn remember_items(&mut self, game: &LiveGame) {
        for p in &game.players {
            if self.possessing(p) {
                continue;
            }
            for item in p.items.iter().filter(|i| i.id > 0) {
                self.item_said
                    .insert(format!("{}:{}", p.key(), base_item(item.id)));
            }
        }
    }

    /// 谁出了成装、关键装备，身上第一次有了重伤。同一轮的并成一条，最多写三个；对面的、关键装备、重伤说一句，队友和你
    /// 自己别的成装只显示一行。
    fn key_items(
        &mut self,
        before: &Roster,
        changes: &events::Changes,
        game: &LiveGame,
        data: Option<&GameData>,
        key: String,
    ) -> Option<Push> {
        let my_team = game.my_team();
        if my_team.is_empty() {
            return None;
        }
        let grievous = |items: &[i64]| items.iter().any(|i| GRIEVOUS.contains(&base_item(*i)));
        let mut enemies: Vec<Found> = Vec::new();
        let mut allies: Vec<Found> = Vec::new();
        for bought in &changes.items {
            let p = bought.player;
            if p.team.is_empty() || self.possessing(p) {
                continue;
            }
            let enemy = p.team != my_team;
            let base = base_item(bought.item.id);
            let said = format!("{}:{base}", p.key());
            let what = if let Some((_, name)) = KEY_ITEMS.iter().find(|(id, _)| *id == base) {
                self.item_said
                    .insert(said)
                    .then(|| (name.to_string(), true))
            } else {
                // 第一次有重伤才说“重伤”：上一份身上已经有一件（小件合成大件、刚连上时就有）的不算第一次，是成装的照成装说
                let first_grievous = GRIEVOUS.contains(&base) && {
                    let had = before
                        .players
                        .get(&p.key())
                        .is_some_and(|s| grievous(&s.items));
                    self.item_said.insert(format!("{}:grievous", p.key())) && !had
                };
                let new = self.item_said.insert(said);
                if first_grievous {
                    Some(("重伤".to_string(), true))
                } else if new && events::completed(bought.item.id, data) {
                    Some((item_spoken(bought.item, data), enemy))
                } else {
                    None
                }
            };
            let Some((what, voiced)) = what else {
                continue;
            };
            if p.riot_id == game.me {
                allies.insert(0, ("你".into(), what, voiced));
            } else if enemy {
                enemies.push((spoken(p, data), what, voiced));
            } else {
                allies.push((format!("我方{}", spoken(p, data)), what, voiced));
            }
        }
        if enemies.is_empty() && allies.is_empty() {
            return None;
        }
        let cells = item_cells(&enemies, &allies);
        let heard =
            |found: &[Found]| -> Vec<Found> { found.iter().filter(|f| f.2).cloned().collect() };
        let voice = item_voice(&heard(&enemies), &heard(&allies));
        Some(line_push(PUSH_ENEMY_ITEM, key, cells, voice))
    }

    /// 对面阵亡、还要 [`DOWN_RESPAWN`] 秒以上才复活的（“够久的”），这一份刚到 [`DOWN_AT`] 个（上一份不到）：几个、
    /// 最快的几秒复活，都按够久的那几个算。[`DOWN_GAP`] 之内只说一次。
    fn enemies_down(&mut self, before: &Roster, game: &LiveGame, key: String) -> Option<Push> {
        if game.my_team().is_empty() {
            return None;
        }
        let long = |dead: bool, respawn: f64| dead && respawn >= DOWN_RESPAWN;
        let down: Vec<&LivePlayer> = game.enemies().filter(|p| long(p.dead, p.respawn)).collect();
        let was = game
            .enemies()
            .filter(|p| {
                before
                    .players
                    .get(&p.key())
                    .is_some_and(|s| long(s.dead, s.respawn))
            })
            .count();
        if down.len() < DOWN_AT || was >= DOWN_AT {
            return None;
        }
        if self.down_at.is_some_and(|t| game.time - t < DOWN_GAP) {
            return None;
        }
        self.down_at = Some(game.time);
        let n = down.len();
        let soonest = down
            .iter()
            .map(|p| p.respawn)
            .fold(f64::INFINITY, f64::min)
            .round() as i64;
        Some(line_push(
            PUSH_ENEMIES_DOWN,
            key,
            vec![
                cell(format!("对面死了 {n} 个"), Some(PushTone::Good)),
                cell(format!("最快 {soonest} 秒复活"), None),
            ],
            vec![
                format!("对面死了 {n} 个，最快 {soonest} 秒复活"),
                format!("对面倒了 {n} 个，最快 {soonest} 秒回来"),
            ],
        ))
    }

    /// 你刚阵亡：一张对面装备的卡片（一局一张，同一个 key；第二次起带 `alert` 让悬浮窗再弹一次）。
    fn my_death(
        &mut self,
        before: &Roster,
        game: &LiveGame,
        data: Option<&GameData>,
        game_id: i64,
    ) -> Option<Push> {
        let me = game.me()?;
        let was_dead = before.players.get(&me.key()).is_some_and(|s| s.dead);
        if !me.dead || was_dead {
            return None;
        }
        self.deaths += 1;
        let card = death_card(game, data, self.deaths, self.at_last_death.as_ref());
        self.at_last_death = Some(
            game.enemies()
                .map(|p| (p.key(), p.items.iter().map(|i| i.id).collect()))
                .collect(),
        );
        Some(Push {
            trigger: PUSH_MY_DEATH.into(),
            key: format!("death:{game_id}"),
            card: Some(PushCard {
                title: DEATH_CARD_TITLE.into(),
                uri: PAGE_ENEMY_BUILD.into(),
                params: Value::Null,
                data: card,
            }),
            alert: self.deaths > 1,
            ..Push::default()
        })
    }
}

/// 记下的召唤师技能里，上一份和这一份之间转好的（`prev < 好的时间 <= 现在`）。峡谷里按分路叫（“中单闪现好了”），
/// 别的模式按英雄。
fn spells_ready(prev: f64, game: &LiveGame, spells: &[SpellUse], game_id: i64) -> Vec<Push> {
    spells
        .iter()
        .filter(|u| prev < u.ready_at && u.ready_at <= game.time)
        .map(|u| {
            let player = game
                .players
                .iter()
                .find(|p| p.alias.eq_ignore_ascii_case(&u.alias));
            let who = player
                .filter(|_| is_rift(game))
                .and_then(|p| position_name(&p.position))
                .map(str::to_string)
                .unwrap_or_else(|| u.champion.clone());
            let spell = &u.spell;
            line_push(
                PUSH_SPELL_READY,
                format!(
                    "spell:{game_id}:{}:{spell}:{}",
                    u.alias,
                    u.ready_at.round() as i64
                ),
                vec![
                    cell(format!("{who}{spell}"), Some(PushTone::Ok)),
                    cell("好了", None),
                ],
                vec![
                    format!("{who}{spell}好了"),
                    format!("{who}的{spell}好了"),
                    format!("{who}{spell}转好了"),
                ],
            )
        })
        .collect()
}

/// 峡谷资源：现在在“刷新前 30 秒”到刷新之间的（同一次刷新只说一次由 [`Alerts`] 按 key 管）。不只看上一份和这一份
/// 之间跨没跨过：刚连上、一轮慢了也照说——说的是几点刷（“小龙 12 分 30 秒刷新”），不是“30 秒后”，宿主晚一点说、
/// 气泡和聊天里留着也不会错。
fn objectives(game: &LiveGame, rules: &RiftRules, game_id: i64) -> Vec<Push> {
    next_spawns(game, rules)
        .into_iter()
        .filter(|(_, at)| at - OBJECTIVE_LEAD <= game.time && game.time < *at)
        .map(|(objective, at)| {
            let name = objective.name();
            let when = spoken_clock(at);
            line_push(
                PUSH_OBJECTIVE,
                format!(
                    "objective:{game_id}:{}:{}",
                    objective.id(),
                    at.round() as i64
                ),
                vec![
                    cell(name, Some(PushTone::Ok)),
                    cell(format!("{} 刷新", crate::client::clock(at)), None),
                ],
                vec![
                    format!("{name} {when}刷新"),
                    format!("{when}刷{name}"),
                    format!("{name}快刷了，{when}"),
                ],
            )
        })
        .collect()
}

/// 身上装备的总价（客户端的装备资料里的 `priceTotal`；没读到资料时 0）。
fn item_gold(p: &LivePlayer, data: Option<&GameData>) -> i64 {
    let Some(d) = data else {
        return 0;
    };
    p.items
        .iter()
        .map(|i| {
            d.item_prices
                .get(&i.id)
                .or_else(|| d.item_prices.get(&base_item(i.id)))
                .copied()
                .unwrap_or(0)
        })
        .sum()
}

/// 阵亡那张卡片的数据（页面 `ui://kotone/lol/enemy-build`）：第几次阵亡、几点、几秒复活；我自己；对面每个人
/// `{champion, level, kills, deaths, assists, gold, items: [{id, name, icon, new}], hidden}`，按装备总价从高到低
/// （一样多按击杀）。`new`：上次阵亡以后新出的（第一次阵亡不标）。只放数字和装备，不评价谁。
pub fn death_card(
    game: &LiveGame,
    data: Option<&GameData>,
    deaths: u32,
    before: Option<&HashMap<String, Vec<i64>>>,
) -> Value {
    let champion = |p: &LivePlayer| {
        let found = data.and_then(|d| d.champion_by_alias(&p.alias));
        json!({
            "id": found.map(|c| c.id).unwrap_or(0),
            "short": crate::client::text::champion_names(p, data).1,
        })
    };
    let mut enemies: Vec<&LivePlayer> = game.enemies().collect();
    enemies.sort_by(|a, b| {
        item_gold(b, data)
            .cmp(&item_gold(a, data))
            .then(b.kills.cmp(&a.kills))
            .then(a.deaths.cmp(&b.deaths))
    });
    let rows: Vec<Value> = enemies
        .iter()
        .map(|p| {
            let had = before.and_then(|b| b.get(&p.key()));
            let items: Vec<Value> = p
                .items
                .iter()
                .filter(|i| i.id > 0)
                .map(|i| {
                    let name = if i.name.is_empty() {
                        data.and_then(|d| d.items.get(&i.id)).cloned().unwrap_or_default()
                    } else {
                        i.name.clone()
                    };
                    json!({
                        "id": i.id,
                        "name": name,
                        "icon": data.and_then(|d| d.item_icons.get(&i.id)).cloned().unwrap_or_default(),
                        "new": had.is_some_and(|h| !h.contains(&i.id)),
                    })
                })
                .collect();
            json!({
                "champion": champion(p),
                "level": p.level,
                "kills": p.kills,
                "deaths": p.deaths,
                "assists": p.assists,
                "gold": item_gold(p, data),
                "items": items,
                "dead": p.dead,
                "hidden": p.hidden_identity(),
            })
        })
        .collect();
    let me = game.me();
    json!({
        "deaths": deaths,
        "at": crate::client::clock(game.time),
        "respawn": me.map(|p| p.respawn.max(0.0).round() as i64).unwrap_or(0),
        "me": me.map(|p| json!({
            "champion": champion(p),
            "level": p.level,
            "kills": p.kills,
            "deaths": p.deaths,
            "assists": p.assists,
            "gold": item_gold(p, data),
        })),
        "enemies": rows,
        "goldKnown": data.is_some_and(|d| !d.item_prices.is_empty()),
    })
}

fn pushed(push: &Push, result: Result<PushOutcome, String>) {
    if let Err(e) = result {
        log::log(&format!("lol: 推送「{}」发不出去：{e}", push.trigger));
    }
}

impl LolPlugin {
    /// 后台每一轮（客户端开着时）：对局里（局内接口有游戏时间）看一眼有什么要提醒的、要报的阵亡、等级和装备事件；
    /// 不在对局里时清掉这局的记录。
    pub(crate) fn live_alerts(&self, host: &dyn Host, phase: &str, live: Option<&LiveGame>) {
        let Some(game) = live.filter(|g| g.time > 0.0 && crate::client::in_game_phase(phase))
        else {
            self.pushes.lock().unwrap().alerts = Alerts::default();
            return;
        };
        let data = self.loaded_data();
        let spells = self.spell_uses();
        let game_id = self.current_game_id().unwrap_or(0);
        let queue = self.current_queue();
        let out =
            self.pushes
                .lock()
                .unwrap()
                .alerts
                .step(game_id, queue, game, data.as_deref(), &spells);
        for (kind, data, time) in out.events {
            host.game_event("lol", kind, "lol/live-client", data, Some(time));
        }
        for push in out.pushes {
            let result = host.push(push.clone());
            pushed(&push, result);
        }
    }
}

#[cfg(test)]
mod tests;
