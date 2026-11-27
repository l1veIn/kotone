//! 对局中的提醒（ADR 043 B 轮第 2 条，讨论记录第二节推送清单里“对局中”那几行）：都从局内接口（`127.0.0.1:2999`，
//! 只读）来，**只说事实**，说法固定（几个说法轮着用），不叫模型（ADR 043 D2、D3）。主循环每 2 秒一轮，比这一份和上一份
//! 局内数据（[`events::Roster`]）：
//!
//! | 推送（清单 id） | 什么时候 | 样子 | 多要紧 |
//! |---|---|---|---|
//! | `enemy-ult` | 对面全图 / 远距离大招的英雄（[`LONG_ULTS`]）、峡谷里你的对线对手到 6 级 | 语音 + 一行 | 普通 |
//! | `enemy-item` | 对面出了复活甲、中娅、水银、女妖（[`KEY_ITEMS`]）；你玩回血英雄（[`HEALERS`]）时对面出了重伤（[`GRIEVOUS`]） | 语音 + 一行 | 低 |
//! | `spell-ready` | 你或她记下的召唤师技能（`lol.spell`）转好了 | 语音 + 一行 | 普通 |
//! | `objective` | 峡谷的小龙、远古龙、虚空巢虫、峡谷先锋、大龙刷新前 30 秒（[`RIFT_RULES`]，只在峡谷） | 语音 + 一行 | 普通 |
//! | `enemies-down` | 对面同时死了 3 个以上（这一刻刚到 3 个，45 秒内只说一次） | 语音 + 一行 | 要紧 |
//! | `my-death` | 你阵亡时：对面每个人的等级、KDA、装备（按装备总价排），上次阵亡以后新出的标出来 | 卡片（一局一张，每次阵亡原地更新、再弹一次） | 普通 |
//!
//! 谁先说、20 秒冷却、过时不说由宿主管（`kotone_agent::push_voice`）：同一轮里来了几条，宿主只说最要紧的那句，别的只显示
//! 一行、记进聊天。插件这边只管同一件事不重复（同一个人到 6 级只说一次、同一个人同一件装备只说一次）。大乱斗、海斗没有分路
//! 和峡谷资源：只有“关键英雄到 6 级”“关键装备”“技能转好”“对面阵亡”“你阵亡”。
//!
//! 等级、装备、阵亡这几类游戏事件（`lol.level`、`lol.item`、`lol.death` 和“我阵亡了 / 我复活了”，给自动化和她）也在
//! 这里报，规矩在 `client/events.rs`：阵亡和“你阵亡时”的卡片、“对面阵亡”用的是同一份前后数据、同一个判断（ADR 046 D7）。

use std::collections::{HashMap, HashSet};

use kotone_core::log;
use kotone_plugin_api::{Host, Push, PushCard, PushCell, PushLine, PushOutcome, PushTone};
use serde_json::{json, Map, Value};

use super::PAGE_ENEMY_BUILD;
use crate::client::events::{self, base_item, Roster};
use crate::client::live::{LiveGame, LivePlayer};
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

/// 关键装备（装备 id 的后四位，说的时候的叫法）：复活甲、中娅、水银（饰带、弯刀、密银黎明都带净化）、女妖。
pub(crate) const KEY_ITEMS: &[(i64, &str)] = &[
    (3026, "复活甲"),
    (3157, "中娅"),
    (3140, "水银"),
    (3139, "水银弯刀"),
    (6035, "密银黎明"),
    (3102, "女妖"),
];

/// 带重伤的装备（后四位）：死刑宣告、凡性的提醒、炼金朋克链锯剑、湮灭宝珠、莫雷洛秘典、炼金科技纯化器、棘刺背心、荆棘之甲。
pub(crate) const GRIEVOUS: &[i64] = &[3123, 3033, 6609, 3916, 3165, 3011, 3076, 3075];

/// 回血的英雄（英文 id）：你玩这些时，对面出了重伤也提醒。
pub(crate) const HEALERS: &[&str] = &[
    "Aatrox",
    "Alistar",
    "Briar",
    "DrMundo",
    "Fiddlesticks",
    "Fiora",
    "Gwen",
    "Illaoi",
    "Maokai",
    "Milio",
    "Nami",
    "Nasus",
    "Olaf",
    "Seraphine",
    "Sona",
    "Soraka",
    "Swain",
    "Sylas",
    "Taric",
    "Trundle",
    "Vladimir",
    "Volibear",
    "Warwick",
    "Yuumi",
    "Zac",
];

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
/// times are unchanged unless noted above”；峡谷先锋 16:00 是 25.S1.1 改的，虚空巢虫 8:00、只刷一波是 25.09 改的；
/// 小龙 5:00、被拿以后 5 分钟，远古龙 6 分钟，大龙被拿以后 6 分钟，多年没变。2026-11-26 查的，以后的版本改了就加一份。
pub const RIFT_RULES: RiftRules = RiftRules {
    dragon_first: 300.0,
    dragon_respawn: 300.0,
    soul_at: 4,
    elder_after_soul: 360.0,
    elder_respawn: 360.0,
    grubs: Some(480.0),
    herald: Some(960.0),
    baron_first: 1200.0,
    baron_respawn: 360.0,
};

/// 资源刷新前多久提醒。
pub const OBJECTIVE_LEAD: f64 = 30.0;
/// 对面同时死了几个才提醒。
pub const DOWN_AT: usize = 3;
/// “对面死了几个”两次之间至少隔多久（游戏时间，秒）：一波团战只说一次。
pub const DOWN_GAP: f64 = 45.0;

/// 峡谷（只有它有分路和峡谷资源；大乱斗是 `ARAM`、海斗是 `KIWI`）。
pub fn is_rift(game: &LiveGame) -> bool {
    game.mode.eq_ignore_ascii_case("CLASSIC")
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

fn line_push(trigger: &str, key: String, cells: Vec<PushCell>, voice: Vec<String>) -> Push {
    Push {
        trigger: trigger.into(),
        key,
        line: Some(PushLine { cells }),
        voice: fit(voice),
        ..Push::default()
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

/// 照局内接口的事件（拿龙、拿大龙）和规则算：接下来每种资源什么时候刷（游戏时间）。只给峡谷用。
pub fn next_spawns(game: &LiveGame, rules: &RiftRules) -> Vec<(Objective, f64)> {
    let mut out = Vec::new();
    // 小龙：每边拿了几条（远古龙不算）；有一边到了龙魂，之后只刷远古龙
    let mut counts: HashMap<bool, usize> = HashMap::new();
    let mut soul_at: Option<f64> = None;
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
                let ally = game.is_ally_name(&e.killer).unwrap_or(false);
                let n = counts.entry(ally).or_default();
                *n += 1;
                if *n >= rules.soul_at && soul_at.is_none() {
                    soul_at = Some(e.time);
                }
            }
            "BaronKill" => last_baron = Some(e.time),
            _ => {}
        }
    }
    match soul_at {
        Some(soul) => out.push((
            Objective::Elder,
            match last_elder {
                Some(t) if t >= soul => t + rules.elder_respawn,
                _ => soul + rules.elder_after_soul,
            },
        )),
        None => out.push((
            Objective::Dragon,
            last_dragon.map_or(rules.dragon_first, |t| t + rules.dragon_respawn),
        )),
    }
    if let Some(t) = rules.grubs {
        out.push((Objective::Grubs, t));
    }
    if let Some(t) = rules.herald {
        out.push((Objective::Herald, t));
    }
    out.push((
        Objective::Baron,
        last_baron.map_or(rules.baron_first, |t| t + rules.baron_respawn),
    ));
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
    /// 出过的关键装备（`<人>:<装备后四位>`，重伤是 `<人>:grievous`）。
    item_said: HashSet<String>,
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
    /// 看一份局内数据（`game_id` 是这局的对局 id，没有时 0）：和上一份比，出了什么事。第一次看到这局只记下样子。
    pub fn step(
        &mut self,
        game_id: i64,
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
        if let Some(push) = self.enemy_ults(&changes, game, data, key(PUSH_ENEMY_ULT)) {
            out.pushes.push(push);
        }
        if let Some(push) = self.enemy_items(&changes, game, data, key(PUSH_ENEMY_ITEM)) {
            out.pushes.push(push);
        }
        out.pushes
            .extend(spells_ready(before.time, game, spells, game_id));
        if is_rift(game) {
            out.pushes
                .extend(objectives(before.time, game, &RIFT_RULES, game_id));
        }
        if let Some(push) = self.enemies_down(&before, game, key(PUSH_ENEMIES_DOWN)) {
            out.pushes.push(push);
        }
        if let Some(push) = self.my_death(&before, game, data, game_id) {
            out.pushes.push(push);
        }
        out
    }

    /// 对面到 6 级的：全图 / 远距离大招的英雄，峡谷里加上你的对线对手。同一轮的并成一条。
    fn enemy_ults(
        &mut self,
        changes: &events::Changes,
        game: &LiveGame,
        data: Option<&GameData>,
        key: String,
    ) -> Option<Push> {
        let opponent = lane_opponent(game).map(LivePlayer::key);
        let mut names = Vec::new();
        for up in &changes.levels {
            let p = up.player;
            if game.my_team().is_empty()
                || p.team == game.my_team()
                || !(up.from < 6 && p.level >= 6)
            {
                continue;
            }
            let long = LONG_ULTS
                .iter()
                .any(|(alias, _)| alias.eq_ignore_ascii_case(&p.alias));
            let lane = opponent.as_deref() == Some(p.key().as_str());
            if !(long || lane) || !self.ult_said.insert(p.key()) {
                continue;
            }
            // 对线对手没有外号时按分路叫（“对面中单”），有外号的照外号
            names.push(if lane && !long {
                position_name(&p.position)
                    .map(str::to_string)
                    .unwrap_or_else(|| spoken(p, data))
            } else {
                spoken(p, data)
            });
        }
        if names.is_empty() {
            return None;
        }
        let who = if names.len() > 2 {
            format!("{}等 {} 个", names[..2].join("、"), names.len())
        } else {
            names.join("、")
        };
        // 一格最多 12 个字：放不下“对面”就不写
        let label = match format!("对面{who}") {
            l if l.chars().count() <= PushCell::MAX_CHARS => l,
            _ => who.clone(),
        };
        Some(line_push(
            PUSH_ENEMY_ULT,
            key,
            vec![
                cell(label, Some(PushTone::Bad)),
                cell("6 级了，有大招", None),
            ],
            vec![
                format!("对面{who} 6 级了"),
                format!("对面{who}有大招了"),
                format!("{who}到 6 级了"),
            ],
        ))
    }

    /// 对面出了关键装备（你玩回血英雄时还有重伤）。同一轮的并成一条，最多写三个。
    fn enemy_items(
        &mut self,
        changes: &events::Changes,
        game: &LiveGame,
        data: Option<&GameData>,
        key: String,
    ) -> Option<Push> {
        let healer = game
            .me()
            .is_some_and(|me| HEALERS.iter().any(|h| h.eq_ignore_ascii_case(&me.alias)));
        let mut found: Vec<(String, String)> = Vec::new();
        for bought in &changes.items {
            let p = bought.player;
            if game.my_team().is_empty() || p.team == game.my_team() {
                continue;
            }
            let base = base_item(bought.item.id);
            let what = if let Some((_, name)) = KEY_ITEMS.iter().find(|(id, _)| *id == base) {
                self.item_said
                    .insert(format!("{}:{base}", p.key()))
                    .then(|| name.to_string())
            } else if healer && GRIEVOUS.contains(&base) {
                self.item_said
                    .insert(format!("{}:grievous", p.key()))
                    .then(|| "重伤".to_string())
            } else {
                None
            };
            if let Some(what) = what {
                found.push((spoken(p, data), what));
            }
        }
        let (who, what) = found.first().cloned()?;
        let cells = found
            .iter()
            .take(3)
            .enumerate()
            .map(|(i, (who, what))| {
                cell(
                    if found.len() == 1 {
                        format!("对面{who}出了{what}")
                    } else {
                        format!("{who} {what}")
                    },
                    (i == 0).then_some(PushTone::Bad),
                )
            })
            .collect();
        let more = if found.len() > 1 {
            format!("，还有 {} 件", found.len() - 1)
        } else {
            String::new()
        };
        Some(line_push(
            PUSH_ENEMY_ITEM,
            key,
            cells,
            vec![
                format!("对面{who}出了{what}{more}"),
                format!("{who}出了{what}{more}"),
                format!("对面{who}有{what}了"),
            ],
        ))
    }

    /// 对面这一刻刚死到 [`DOWN_AT`] 个：几个、最快的几秒复活。[`DOWN_GAP`] 之内只说一次。
    fn enemies_down(&mut self, before: &Roster, game: &LiveGame, key: String) -> Option<Push> {
        if game.my_team().is_empty() {
            return None;
        }
        let dead: Vec<&LivePlayer> = game.enemies().filter(|p| p.dead).collect();
        let was = game
            .enemies()
            .filter(|p| before.players.get(&p.key()).is_some_and(|s| s.dead))
            .count();
        if dead.len() < DOWN_AT || was >= DOWN_AT {
            return None;
        }
        if self.down_at.is_some_and(|t| game.time - t < DOWN_GAP) {
            return None;
        }
        self.down_at = Some(game.time);
        let n = dead.len();
        let soonest = dead
            .iter()
            .map(|p| p.respawn.max(0.0))
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

/// 峡谷资源：上一份和这一份之间到了“刷新前 30 秒”的。
fn objectives(prev: f64, game: &LiveGame, rules: &RiftRules, game_id: i64) -> Vec<Push> {
    next_spawns(game, rules)
        .into_iter()
        .filter(|(_, at)| {
            let warn = at - OBJECTIVE_LEAD;
            prev < warn && warn <= game.time
        })
        .map(|(objective, at)| {
            let name = objective.name();
            line_push(
                PUSH_OBJECTIVE,
                format!(
                    "objective:{game_id}:{}:{}",
                    objective.id(),
                    at.round() as i64
                ),
                vec![cell(name, Some(PushTone::Ok)), cell("30 秒后刷新", None)],
                vec![
                    format!("30 秒后{name}刷新"),
                    format!("{name}还有 30 秒"),
                    format!("{name}快刷了，还有 30 秒"),
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
        let out = self
            .pushes
            .lock()
            .unwrap()
            .alerts
            .step(game_id, game, data.as_deref(), &spells);
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
