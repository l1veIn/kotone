//! 对局中的提醒和等级、装备事件：每条都当成“前后两份局内数据 → 推什么、报什么”的纯函数测。
//!
//! - 海斗：2026-11-26 录的一整把（`target/lol-probe/20261126-mayhem-midgame`，56 份，每 20 秒一份，1 级到 18 级），
//!   压成 `testdata/mayhem-20261126.json`（名字换成编的，装备的总价从客户端的 `items.json` 抄的）。
//! - 峡谷：拼的（录的那把是海斗，没有分路和峡谷资源）。

use kotone_plugin_api::{PushForm, PushPriority};
use serde_json::{json, Value};

use super::*;
use crate::client::live::parse_all_game_data;

const FIXTURE: &str = include_str!("../testdata/mayhem-20261126.json");

/// 录的那一把的 10 个英雄（英文 id → 编号、名字；Locke 是新英雄，编号是编的）。
const CHAMPIONS: [(&str, i64, &str); 11] = [
    ("Viego", 234, "佛耶戈"),
    ("Lucian", 236, "卢锡安"),
    ("Ashe", 22, "艾希"),
    ("TwistedFate", 4, "崔斯特"),
    ("MasterYi", 11, "易"),
    ("Locke", 950, "洛克"),
    ("Ambessa", 799, "安蓓萨"),
    ("Taliyah", 163, "塔莉垭"),
    ("Leona", 89, "蕾欧娜"),
    ("Viktor", 112, "维克托"),
    ("Soraka", 16, "索拉卡"),
];

fn fixture() -> Value {
    serde_json::from_str(FIXTURE).unwrap()
}

/// 客户端的游戏资料：这几个英雄，录的那把里出现过的装备（名字、总价、还能不能合成别的）。
fn data() -> GameData {
    let f = fixture();
    let champions: Vec<Value> = CHAMPIONS
        .iter()
        .map(|(alias, id, name)| json!({"id": id, "name": name, "description": name, "alias": alias}))
        .collect();
    let items: Vec<Value> = f["prices"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(id, v)| {
            json!({"id": id.parse::<i64>().unwrap(), "name": f["items"][id], "priceTotal": v[0],
                   "to": if v[1] == 1 { json!([1]) } else { json!([]) },
                   "iconPath": format!("/lol-game-data/assets/ASSETS/Items/Icons2D/{id}.png")})
        })
        .collect();
    GameData::from_json(
        &Value::Array(champions),
        &Value::Null,
        &Value::Array(items),
        &Value::Null,
        &Value::Null,
    )
}

/// 压过的一帧 → 局内接口 `allgamedata` 的样子 → [`LiveGame`]。`me_as`：把“我”换成这个英雄（测回血英雄）。
fn frames(me_as: Option<&str>) -> Vec<LiveGame> {
    let f = fixture();
    let people = f["people"].as_array().unwrap();
    let me = f["me"].as_str().unwrap();
    f["frames"]
        .as_array()
        .unwrap()
        .iter()
        .map(|frame| {
            let players: Vec<Value> = frame["p"]
                .as_array()
                .unwrap()
                .iter()
                .map(|r| {
                    let who = &people[r[0].as_u64().unwrap() as usize];
                    let name = who["name"].as_str().unwrap();
                    let mut alias = r[1].as_str().unwrap().to_string();
                    if name == me {
                        if let Some(a) = me_as {
                            alias = a.to_string();
                        }
                    }
                    let items: Vec<Value> = r[9]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|id| json!({"itemID": id, "displayName": f["items"][id.to_string()]}))
                        .collect();
                    json!({
                        "championName": r[2], "rawChampionName": format!("game_character_displayname_{alias}"),
                        "team": who["team"], "level": r[3], "isDead": r[4] == 1, "respawnTimer": r[5],
                        "position": "NONE", "isBot": false,
                        "riotId": if name.is_empty() { "#".to_string() } else { format!("{name}#1") },
                        "riotIdGameName": name,
                        "summonerName": if name.is_empty() { who["summoner"].as_str().unwrap().to_string() } else { format!("{name}#1") },
                        "scores": {"kills": r[6], "deaths": r[7], "assists": r[8], "creepScore": 0},
                        "items": items,
                    })
                })
                .collect();
            parse_all_game_data(&json!({
                "activePlayer": {"riotId": format!("{me}#1")},
                "allPlayers": players,
                "events": {"Events": []},
                "gameData": {"gameMode": f["mode"], "gameTime": frame["t"]},
            }))
            .unwrap()
        })
        .collect()
}

/// 一整局一份一份地看：每一份推了什么、报了什么（游戏时间）。
fn run(games: &[LiveGame], data: Option<&GameData>, spells: &[SpellUse]) -> Vec<(f64, Out)> {
    let mut alerts = Alerts::default();
    games
        .iter()
        .map(|g| (g.time, alerts.step(9100001, g, data, spells)))
        .collect()
}

fn pushes_of<'a>(outs: &'a [(f64, Out)], trigger: &str) -> Vec<(f64, &'a Push)> {
    outs.iter()
        .flat_map(|(t, o)| o.pushes.iter().map(move |p| (*t, p)))
        .filter(|(_, p)| p.trigger == trigger)
        .collect()
}

/// 每条推送都过得了宿主的检查（清单里有、样子声明过、卡片的页面在清单里），截字不会截坏（说法 20 字、一格 12 字以内）。
fn assert_well_formed(push: &Push) {
    let spec = crate::manifest()
        .push_spec(&push.trigger)
        .unwrap_or_else(|| panic!("{} 不在清单里", push.trigger));
    let shaped = push.clone().shaped(spec);
    assert_eq!(&shaped, push, "宿主截过以后应该一样：{push:?}");
    shaped.check(crate::manifest()).unwrap();
    for v in &push.voice {
        assert!(v.chars().count() <= Push::VOICE_MAX, "{v}");
    }
    if let Some(line) = &push.line {
        for c in &line.cells {
            assert!(c.text.chars().count() <= PushCell::MAX_CHARS, "{}", c.text);
        }
    }
}

/// 等级、装备事件（`lol.level`、`lol.item`）：照录的一整把海斗。只报对面和自己；自己每升一级都报（佛耶戈附身换了英雄也
/// 还是“我”），对面只报到 6、11、16 级；装备只报成装，同一个人同一件只报一次；第一份只记下样子。
#[test]
fn level_and_item_events_over_a_whole_mayhem_game() {
    let data = data();
    let games = frames(None);
    assert_eq!(games.len(), 56);
    let outs = run(&games, Some(&data), &[]);
    assert!(
        outs[0].1.events.is_empty() && outs[0].1.pushes.is_empty(),
        "第一份只记下样子"
    );
    let events: Vec<&(&str, Map<String, Value>, f64)> =
        outs.iter().flat_map(|(_, o)| o.events.iter()).collect();
    let levels: Vec<_> = events.iter().filter(|e| e.0 == "lol.level").collect();
    let items: Vec<_> = events.iter().filter(|e| e.0 == "lol.item").collect();
    // 我：1 级 → 18 级，每次升级一条（20 秒一份，一次可能升好几级：每次一条，带现在的等级）
    let mine: Vec<i64> = levels
        .iter()
        .filter(|e| e.1["me"] == true)
        .map(|e| e.1["level"].as_i64().unwrap())
        .collect();
    assert!(mine.windows(2).all(|w| w[0] < w[1]), "{mine:?}");
    assert_eq!(*mine.last().unwrap(), 18);
    assert!(levels
        .iter()
        .filter(|e| e.1["me"] == true)
        .all(|e| e.1["team"] == "ally"));
    // 对面：每个人只在过 6、11、16 时报，5 个人 × 3 次
    let theirs: Vec<_> = levels.iter().filter(|e| e.1["me"] == false).collect();
    assert!(theirs.iter().all(|e| e.1["team"] == "enemy"), "队友不报");
    assert_eq!(theirs.len(), 15, "{theirs:?}");
    let taliyah: Vec<i64> = theirs
        .iter()
        .filter(|e| e.1["champion"] == "塔莉垭")
        .map(|e| e.1["level"].as_i64().unwrap())
        .collect();
    assert_eq!(taliyah.len(), 3);
    assert!(taliyah[0] >= 6 && taliyah[0] < 11, "{taliyah:?}");
    // 装备：都是成装（总价 2000 以上、不再合成别的），没有队友的；维克托的中娅沙漏在里面；同一个人同一件只一次
    assert!(!items.is_empty());
    for e in &items {
        let id = e.1["itemId"].as_i64().unwrap();
        assert!(crate::client::events::completed(id, Some(&data)), "{e:?}");
        assert!(e.1["team"] == "enemy" || e.1["me"] == true, "{e:?}");
        assert!(!e.1["item"].as_str().unwrap().is_empty());
    }
    assert!(items
        .iter()
        .any(|e| e.1["champion"] == "维克托" && e.1["itemId"] == 3157));
    let mut seen = std::collections::HashSet::new();
    for e in &items {
        assert!(
            seen.insert((
                e.1["champion"].to_string(),
                e.1["me"].clone(),
                e.1["itemId"].clone()
            )),
            "重复：{e:?}"
        );
    }
    // 别人的名字不进事件的数据
    let text = format!("{events:?}");
    assert!(!text.contains("玩家") && !text.contains("对手"), "{text}");
    // 数据的字段都在清单里登记过
    for (kind, data, _) in events.iter().map(|e| (e.0, &e.1, e.2)) {
        let spec = crate::manifest()
            .events
            .iter()
            .find(|s| s.kind == kind)
            .unwrap();
        for field in data.keys() {
            assert!(spec.data.contains_key(field), "{kind}.{field}");
        }
    }
    // 没读到客户端的装备资料：只认关键装备（中娅还在，别的成装不报）
    let bare = run(&games, None, &[]);
    let bare_items: Vec<i64> = bare
        .iter()
        .flat_map(|(_, o)| o.events.iter())
        .filter(|e| e.0 == "lol.item")
        .map(|e| e.1["itemId"].as_i64().unwrap())
        .collect();
    assert!(!bare_items.is_empty());
    assert!(bare_items
        .iter()
        .all(|id| crate::client::events::KEY_ITEM_IDS.contains(&base_item(*id))));
}

/// 那一把海斗里的提醒：对面岩雀（塔莉垭，远距离大招）到 6 级说一次；维克托出了中娅说一次；没有峡谷资源；
/// 对面死了 3 个以上说过几次、两次之间至少 45 秒；我每次阵亡同一张卡片原地更新，第二次起再弹一次。
#[test]
fn alerts_over_a_whole_mayhem_game() {
    let data = data();
    let games = frames(None);
    let outs = run(&games, Some(&data), &[]);
    for (_, o) in &outs {
        for p in &o.pushes {
            assert_well_formed(p);
        }
    }
    let ults = pushes_of(&outs, PUSH_ENEMY_ULT);
    assert_eq!(ults.len(), 1, "只有岩雀是远距离大招（大乱斗没有对线对手）");
    assert!((200.0..=210.0).contains(&ults[0].0));
    assert_eq!(ults[0].1.voice[0], "对面岩雀 6 级了");
    assert_eq!(ults[0].1.line.as_ref().unwrap().cells[0].text, "对面岩雀");
    let items = pushes_of(&outs, PUSH_ENEMY_ITEM);
    assert_eq!(items.len(), 1, "{items:?}");
    assert_eq!(
        items[0].1.voice,
        ["对面维克托出了中娅", "维克托出了中娅", "对面维克托有中娅了"],
        "几个说法各不一样"
    );
    assert!(
        pushes_of(&outs, PUSH_OBJECTIVE).is_empty(),
        "海斗没有峡谷资源"
    );
    assert!(pushes_of(&outs, PUSH_SPELL_READY).is_empty());
    let down = pushes_of(&outs, PUSH_ENEMIES_DOWN);
    assert!(down.len() >= 3, "{}", down.len());
    for w in down.windows(2) {
        assert!(w[1].0 - w[0].0 >= DOWN_GAP, "{} → {}", w[0].0, w[1].0);
    }
    let first = down[0].1;
    assert_eq!(first.voice[0], "对面死了 3 个，最快 3 秒复活", "{first:?}");
    // 阵亡：一局一张（同一个 key），第一次不带 alert，之后每次带
    let deaths = pushes_of(&outs, PUSH_MY_DEATH);
    assert!(deaths.len() >= 10, "{}", deaths.len());
    assert!(deaths.iter().all(|(_, p)| p.key == "death:9100001"));
    assert!(!deaths[0].1.alert && deaths[1..].iter().all(|(_, p)| p.alert));
    let card = &deaths[0].1.card.as_ref().unwrap().data;
    assert_eq!(card["deaths"], 1);
    assert_eq!(card["goldKnown"], true);
    let enemies = card["enemies"].as_array().unwrap();
    assert_eq!(enemies.len(), 5);
    let gold: Vec<i64> = enemies
        .iter()
        .map(|e| e["gold"].as_i64().unwrap())
        .collect();
    assert!(
        gold.windows(2).all(|w| w[0] >= w[1]),
        "按装备总价排：{gold:?}"
    );
    assert!(
        enemies.iter().all(|e| e["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|i| i["new"] == false)),
        "第一次阵亡不标新出的"
    );
    assert!(card["me"]["champion"]["short"].is_string());
    // 第二次阵亡：上次以后新出的标出来
    let second = &deaths[1].1.card.as_ref().unwrap().data;
    assert_eq!(second["deaths"], 2);
    assert!(second["enemies"]
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["new"] == true)));
    let text = card.to_string();
    assert!(
        !text.contains("玩家") && !text.contains("对手"),
        "不放名字：{text}"
    );
}

/// 阵亡事件（ADR 046 D6、D7）照录的一整把海斗：和“你阵亡时”的卡片同一个判断——我每阵亡一次，同一轮里有一张卡片、
/// 一件 `lol.death`（`me = true`）、紧跟着一件 `lol.my_death`，只报一次（下一份还死着不再报）；复活一次一件
/// `lol.my_respawn`；第一份不报；两边的阵亡都报，同一轮里死了几个时人数逐个递减；换一局从头来。
#[test]
fn death_events_over_a_whole_mayhem_game() {
    let data = data();
    let games = frames(None);
    let outs = run(&games, Some(&data), &[]);
    assert!(outs[0].1.events.is_empty(), "第一份只记下样子");
    let me = |g: &LiveGame| g.me().unwrap().dead;
    let mut cards = 0;
    let mut respawns = 0;
    for (i, (_, out)) in outs.iter().enumerate() {
        let kinds: Vec<&str> = out.events.iter().map(|e| e.0).collect();
        let card = out.pushes.iter().any(|p| p.trigger == PUSH_MY_DEATH);
        let my_deaths: Vec<usize> = (0..kinds.len())
            .filter(|j| kinds[*j] == "lol.my_death")
            .collect();
        assert_eq!(
            card,
            !my_deaths.is_empty(),
            "第 {i} 份：卡片和事件同一个判断"
        );
        assert!(my_deaths.len() <= 1);
        if card {
            cards += 1;
            let j = my_deaths[0];
            let death = &out.events[j - 1];
            assert_eq!(death.0, "lol.death");
            assert_eq!(death.1["me"], true);
            assert_eq!(death.1["meAlive"], false);
            assert_eq!(death.1["champion"], "佛耶戈", "附身的时候死也是佛耶戈");
            assert_eq!(out.events[j].1["respawnSeconds"], death.1["respawnSeconds"]);
            assert!(me(&games[i]) && !me(&games[i - 1]));
        }
        if kinds.contains(&"lol.my_respawn") {
            respawns += 1;
            assert!(!me(&games[i]) && me(&games[i - 1]));
        }
        // 同一轮里的阵亡：每边的人数逐个递减，最后一件是这一份的样子
        let deaths: Vec<&Map<String, Value>> = out
            .events
            .iter()
            .filter(|e| e.0 == "lol.death")
            .map(|e| &e.1)
            .collect();
        for side in ["ally", "enemy"] {
            let key = if side == "ally" {
                "alliesAlive"
            } else {
                "enemiesAlive"
            };
            let counts: Vec<i64> = deaths
                .iter()
                .filter(|d| d["team"] == side)
                .map(|d| d[key].as_i64().unwrap())
                .collect();
            assert!(counts.windows(2).all(|w| w[0] == w[1] + 1), "{counts:?}");
        }
        if let Some(last) = deaths.last() {
            let g = &games[i];
            assert_eq!(
                last["alliesAlive"].as_i64().unwrap() as usize,
                g.allies().filter(|p| !p.dead).count()
            );
            assert_eq!(
                last["enemiesAlive"].as_i64().unwrap() as usize,
                g.enemies().filter(|p| !p.dead).count()
            );
        }
    }
    let my_deaths = pushes_of(&outs, PUSH_MY_DEATH).len();
    assert_eq!(cards, my_deaths);
    assert!(my_deaths >= 10, "{my_deaths}");
    assert!(respawns >= my_deaths - 1, "{respawns}");
    let events: Vec<&(&str, Map<String, Value>, f64)> =
        outs.iter().flat_map(|(_, o)| o.events.iter()).collect();
    let all_deaths = events.iter().filter(|e| e.0 == "lol.death").count();
    let enemy_deaths = events
        .iter()
        .filter(|e| e.0 == "lol.death" && e.1["team"] == "enemy")
        .count();
    assert!(all_deaths > my_deaths && enemy_deaths > 0, "两边都报");
    assert!(
        events
            .iter()
            .filter(|e| e.0 == "lol.death")
            .all(|e| e.1["killer"] == ""),
        "这份压过的数据里没有 ChampionKill"
    );
    let text = format!("{events:?}");
    assert!(!text.contains("玩家") && !text.contains("对手"), "{text}");
    // 换一局（对局 id 变了）：第一份只记下样子
    let mut alerts = Alerts::default();
    let dead_at = games.iter().position(|g| g.me().unwrap().dead).unwrap();
    alerts.step(1, &games[dead_at - 1], Some(&data), &[]);
    let out = alerts.step(2, &games[dead_at], Some(&data), &[]);
    assert!(out.events.is_empty() && out.pushes.is_empty());
}

/// 你玩回血英雄时对面出了重伤：每个人只说一次（升级成荆棘之甲不再说）；不是回血英雄不说。
#[test]
fn grievous_wounds_only_when_you_play_a_healer() {
    let data = data();
    let outs = run(&frames(Some("Soraka")), Some(&data), &[]);
    let items = pushes_of(&outs, PUSH_ENEMY_ITEM);
    let said: Vec<&str> = items.iter().map(|(_, p)| p.voice[0].as_str()).collect();
    assert!(said.contains(&"对面维克托出了中娅"), "{said:?}");
    assert!(said.contains(&"对面岩雀出了重伤"), "湮灭宝珠：{said:?}");
    assert!(said.contains(&"对面蕾欧娜出了重伤"), "棘刺背心：{said:?}");
    assert_eq!(
        said.iter().filter(|s| s.contains("蕾欧娜")).count(),
        1,
        "升级成荆棘之甲不再说：{said:?}"
    );
}

// ---------- 峡谷（拼的） ----------

fn rift_player(name: &str, alias: &str, team: &str, position: &str, level: i64) -> Value {
    json!({
        "championName": alias, "rawChampionName": format!("game_character_displayname_{alias}"),
        "team": team, "level": level, "isDead": false, "respawnTimer": 0.0, "position": position,
        "isBot": false, "riotId": format!("{name}#1"), "riotIdGameName": name, "summonerName": format!("{name}#1"),
        "scores": {"kills": 0, "deaths": 0, "assists": 0, "creepScore": 0}, "items": [],
        "summonerSpells": {
            "summonerSpellOne": {"displayName": "闪现", "rawDisplayName": "GeneratedTip_SummonerSpell_SummonerFlash_DisplayName"},
            "summonerSpellTwo": {"displayName": "引燃", "rawDisplayName": "GeneratedTip_SummonerSpell_SummonerDot_DisplayName"}
        }
    })
}

/// 一局峡谷在第 `time` 秒：我是中单阿狸；对面上单慎、中单亚索、下路卢锡安；`level` 是对面三个人的等级；
/// `events` 是到这时候为止的局内事件。
fn rift(time: f64, level: i64, events: Value) -> LiveGame {
    parse_all_game_data(&json!({
        "activePlayer": {"riotId": "玩家#1"},
        "allPlayers": [
            rift_player("玩家", "Ahri", "ORDER", "MIDDLE", 6),
            rift_player("队友", "Jinx", "ORDER", "BOTTOM", 6),
            rift_player("敌人一", "Shen", "CHAOS", "TOP", level),
            rift_player("敌人二", "Yasuo", "CHAOS", "MIDDLE", level),
            rift_player("敌人三", "Lucian", "CHAOS", "BOTTOM", level),
        ],
        "events": {"Events": events},
        "gameData": {"gameMode": "CLASSIC", "gameTime": time},
    }))
    .unwrap()
}

/// 峡谷资源刷新前 30 秒：小龙 5:00（之后被拿 + 5 分钟）、巢虫 8:00、先锋 16:00、大龙 20:00（被拿 + 6 分钟）、
/// 有一边拿到龙魂以后只刷远古龙（+ 6 分钟）。每 2 秒看一份（和主循环一样），每次刷新只说一次。
#[test]
fn rift_objectives_thirty_seconds_ahead() {
    let kill = |id: i64, t: f64, kind: &str, dragon: &str, killer: &str| json!({"EventID": id, "EventName": kind, "EventTime": t, "DragonType": dragon, "Stolen": "False", "KillerName": killer});
    // 我方拿了 4 条小龙（5:30、11:00、16:30、22:00），22:00 有了龙魂 → 远古龙 28:00；大龙 21:00 被对面拿了 → 27:00
    let all = [
        kill(1, 330.0, "DragonKill", "Fire", "玩家"),
        kill(2, 660.0, "DragonKill", "Water", "队友"),
        kill(3, 990.0, "DragonKill", "Earth", "玩家"),
        kill(4, 1260.0, "BaronKill", "", "敌人一"),
        kill(5, 1320.0, "DragonKill", "Air", "队友"),
    ];
    let mut alerts = Alerts::default();
    let mut said: Vec<(f64, String)> = Vec::new();
    let mut t = 2.0;
    while t <= 1800.0 {
        let events: Vec<Value> = all
            .iter()
            .filter(|e| e["EventTime"].as_f64().unwrap() <= t)
            .cloned()
            .collect();
        let out = alerts.step(7, &rift(t, 1, Value::Array(events)), None, &[]);
        for p in out.pushes.iter().filter(|p| p.trigger == PUSH_OBJECTIVE) {
            assert_well_formed(p);
            said.push((t, p.voice[0].clone()));
        }
        t += 2.0;
    }
    assert_eq!(
        said,
        [
            (270.0, "30 秒后小龙刷新".to_string()),
            (450.0, "30 秒后虚空巢虫刷新".to_string()),
            (600.0, "30 秒后小龙刷新".to_string()),
            (930.0, "30 秒后小龙刷新".to_string()),
            (930.0, "30 秒后峡谷先锋刷新".to_string()),
            (1170.0, "30 秒后大龙刷新".to_string()),
            (1260.0, "30 秒后小龙刷新".to_string()),
            (1590.0, "30 秒后大龙刷新".to_string()),
            (1650.0, "30 秒后远古龙刷新".to_string()),
        ]
    );
    // 规则表就是 2026 赛季的：大龙 20:00（26.1）、先锋 16:00、巢虫 8:00
    assert_eq!(RIFT_RULES.baron_first, 1200.0);
    assert_eq!(RIFT_RULES.herald, Some(960.0));
    assert_eq!(RIFT_RULES.grubs, Some(480.0));
}

/// 峡谷里到 6 级：慎（全图大招）按外号叫，对线的亚索按分路叫（“对面中单”），下路卢锡安两样都不是、不说。
/// 同一轮的并成一条；之后再升级不再说。大乱斗里没有对线对手。
#[test]
fn level_six_names_long_ults_and_the_lane_opponent() {
    let mut alerts = Alerts::default();
    alerts.step(7, &rift(400.0, 5, json!([])), None, &[]);
    let out = alerts.step(7, &rift(402.0, 6, json!([])), None, &[]);
    let ult: Vec<&Push> = out
        .pushes
        .iter()
        .filter(|p| p.trigger == PUSH_ENEMY_ULT)
        .collect();
    assert_eq!(ult.len(), 1);
    assert_well_formed(ult[0]);
    assert_eq!(ult[0].voice[0], "对面慎、中单 6 级了");
    assert_eq!(ult[0].key, "enemy-ult:7:402");
    let again = alerts.step(7, &rift(404.0, 7, json!([])), None, &[]);
    assert!(again.pushes.iter().all(|p| p.trigger != PUSH_ENEMY_ULT));
    // 大乱斗：同样的人，没有分路 → 只有慎
    let aram = |t: f64, level: i64| {
        let mut g = rift(t, level, json!([]));
        g.mode = "ARAM".into();
        for p in &mut g.players {
            p.position = "NONE".into();
        }
        g
    };
    let mut alerts = Alerts::default();
    alerts.step(8, &aram(400.0, 5), None, &[]);
    let out = alerts.step(8, &aram(402.0, 6), None, &[]);
    let ult: Vec<&Push> = out
        .pushes
        .iter()
        .filter(|p| p.trigger == PUSH_ENEMY_ULT)
        .collect();
    assert_eq!(ult[0].voice[0], "对面慎 6 级了");
    assert!(out.pushes.iter().all(|p| p.trigger != PUSH_OBJECTIVE));
}

/// 记下的闪现转好了：峡谷里按分路说（“中单闪现好了”），大乱斗按英雄；只在跨过好的那一刻说一次。
#[test]
fn a_marked_spell_coming_back_is_said_once() {
    let flash = SpellUse {
        alias: "Yasuo".into(),
        champion: "亚索".into(),
        spell: "闪现".into(),
        used_at: 200.0,
        ready_at: 500.0,
        notes: Vec::new(),
    };
    let mut alerts = Alerts::default();
    let mut said = Vec::new();
    for t in [496.0, 498.0, 500.0, 502.0, 504.0] {
        let out = alerts.step(
            7,
            &rift(t, 6, json!([])),
            None,
            std::slice::from_ref(&flash),
        );
        for p in out.pushes.iter().filter(|p| p.trigger == PUSH_SPELL_READY) {
            assert_well_formed(p);
            said.push((t, p.voice[0].clone(), p.key.clone()));
        }
    }
    assert_eq!(
        said,
        [(
            500.0,
            "中单闪现好了".to_string(),
            "spell:7:Yasuo:闪现:500".to_string()
        )]
    );
    let mut g = rift(500.0, 6, json!([]));
    g.mode = "ARAM".into();
    let pushes = spells_ready(498.0, &g, std::slice::from_ref(&flash), 7);
    assert_eq!(pushes[0].voice[0], "亚索闪现好了");
}

/// 一波团战同一轮里几条一起来（对面死了 3 个、两个人到 6 级、出了中娅）：每种最多一条，清单里“对面阵亡”最要紧、
/// “关键装备”最不要紧——宿主同一时刻只说最要紧的那句（`push_voice`），别的只显示一行、记进聊天。
#[test]
fn a_teamfight_tick_brings_one_push_per_kind_ranked_by_priority() {
    let before = |g: &mut LiveGame| {
        for p in g.players.iter_mut().filter(|p| p.team == "CHAOS") {
            p.level = 5;
        }
    };
    let mut g0 = rift(600.0, 5, json!([]));
    before(&mut g0);
    let mut g1 = rift(602.0, 6, json!([]));
    for p in g1.players.iter_mut().filter(|p| p.team == "CHAOS") {
        p.dead = true;
        p.respawn = 12.0;
        p.items = vec![crate::client::live::LiveItem {
            id: 3157,
            name: "中娅沙漏".into(),
        }];
    }
    let mut alerts = Alerts::default();
    alerts.step(7, &g0, None, &[]);
    let out = alerts.step(7, &g1, None, &[]);
    let kinds: Vec<&str> = out.pushes.iter().map(|p| p.trigger.as_str()).collect();
    assert_eq!(kinds, [PUSH_ENEMY_ULT, PUSH_ENEMY_ITEM, PUSH_ENEMIES_DOWN]);
    let priority = |id: &str| crate::manifest().push_spec(id).unwrap().priority;
    let loudest = out
        .pushes
        .iter()
        .max_by_key(|p| priority(&p.trigger))
        .unwrap();
    assert_eq!(loudest.trigger, PUSH_ENEMIES_DOWN, "同一时刻宿主只说这一句");
    assert_eq!(loudest.voice[0], "对面死了 3 个，最快 12 秒复活");
    let items = out
        .pushes
        .iter()
        .find(|p| p.trigger == PUSH_ENEMY_ITEM)
        .unwrap();
    assert_eq!(items.voice[0], "对面慎出了中娅，还有 2 件");
    assert_eq!(items.line.as_ref().unwrap().cells.len(), 3);
    assert_eq!(items.line.as_ref().unwrap().cells[0].text, "慎 中娅");
    // 下一轮还都死着：不再说
    let mut g2 = g1.clone();
    g2.time = 604.0;
    let out = alerts.step(7, &g2, None, &[]);
    assert!(out.pushes.is_empty(), "{:?}", out.pushes);
    // 换了一局（对局 id 变了）：从头来，第一份只记下样子
    let out = alerts.step(8, &g1, None, &[]);
    assert!(out.pushes.is_empty() && out.events.is_empty());
}

/// 清单：六条提醒都声明了（名字、时机、做什么、样子、多要紧），都要“读客户端数据”、默认开；阵亡的卡片页面在清单里。
#[test]
fn the_manifest_declares_the_in_game_alerts() {
    let m = crate::manifest();
    let expect: [(&str, &[PushForm], PushPriority); 6] = [
        (
            PUSH_ENEMY_ULT,
            &[PushForm::Voice, PushForm::Line],
            PushPriority::Normal,
        ),
        (
            PUSH_ENEMY_ITEM,
            &[PushForm::Voice, PushForm::Line],
            PushPriority::Low,
        ),
        (
            PUSH_SPELL_READY,
            &[PushForm::Voice, PushForm::Line],
            PushPriority::Normal,
        ),
        (
            PUSH_OBJECTIVE,
            &[PushForm::Voice, PushForm::Line],
            PushPriority::Normal,
        ),
        (
            PUSH_ENEMIES_DOWN,
            &[PushForm::Voice, PushForm::Line],
            PushPriority::High,
        ),
        (PUSH_MY_DEATH, &[PushForm::Card], PushPriority::Normal),
    ];
    for (id, forms, priority) in expect {
        let spec = m.push_spec(id).unwrap_or_else(|| panic!("{id}"));
        assert_eq!(spec.forms, forms, "{id}");
        assert_eq!(spec.priority, priority, "{id}");
        assert_eq!(spec.needs, ["client"], "{id}");
        assert!(spec.enabled_by_default, "{id}");
        assert!(!spec.until_done, "{id}");
    }
    assert!(m.page(PAGE_ENEMY_BUILD).is_some());
    // 关键装备两张表一样（推送和装备事件）
    let ids: Vec<i64> = KEY_ITEMS.iter().map(|(id, _)| *id).collect();
    assert_eq!(ids, crate::client::events::KEY_ITEM_IDS);
    // 表里的英雄 id 没有重复
    let mut aliases: Vec<&str> = LONG_ULTS.iter().map(|(a, _)| *a).collect();
    aliases.sort_unstable();
    aliases.dedup();
    assert_eq!(aliases.len(), LONG_ULTS.len());
}
