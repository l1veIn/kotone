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

/// 压过的一帧 → 局内接口 `allgamedata` 的样子 → [`LiveGame`]。`me_as`：把“我”换成这个英雄（测重伤不看你玩什么英雄）。
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
        .map(|g| (g.time, alerts.step(9100001, None, g, data, spells)))
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
        let max = if line.cells.len() == 1 {
            PushCell::ONE_CELL_MAX_CHARS
        } else {
            PushCell::MAX_CHARS
        };
        for c in &line.cells {
            assert!(c.text.chars().count() <= max, "{}", c.text);
        }
    }
}

/// 一行的几格连起来（“对面洛克 帽子 / 我方卡牌 兰德里”）。
fn line_text(push: &Push) -> String {
    push.line
        .as_ref()
        .unwrap()
        .cells
        .iter()
        .map(|c| c.text.as_str())
        .collect::<Vec<_>>()
        .join(" / ")
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

/// 那一把海斗里的提醒：双方到 6 级的关键英雄和你自己各提醒一次；出成装了（2026-11-28 改）：对面每件成装说一句
/// （用玩家的叫法：帽子、大米、血手……，没有叫法的用客户端里的名字），维克托的中娅、岩雀（湮灭宝珠）和蕾欧娜（棘刺背心）
/// 第一次有了重伤也说；队友和你自己的成装只显示一行；佛耶戈（我）附身时身上的是别人的装备，不算你出的；没有峡谷资源；对面同时死 3 个的时候很多，但还要 30 秒以上才复活的到 3 个只有
/// 17 分钟那一次（以前的规矩“同时死了 3 个、45 秒一次”在这把里说了 9 次）；我每次阵亡同一张卡片原地更新，第二次起再弹一次。
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
    assert_eq!(
        ults.len(),
        2,
        "岩雀、我和卡牌同一轮到 6 级，寒冰下一轮到 6 级"
    );
    assert!((200.0..=210.0).contains(&ults[0].0));
    assert_eq!(ults[0].1.voice[0], "对面岩雀、你和我方卡牌 6 级了");
    assert_eq!(ults[0].1.line.as_ref().unwrap().cells[0].text, "对面岩雀");
    assert_eq!(ults[1].1.voice[0], "我方寒冰 6 级了");
    let items = pushes_of(&outs, PUSH_ENEMY_ITEM);
    let said: Vec<&str> = items
        .iter()
        .filter(|(_, p)| !p.voice.is_empty())
        .map(|(_, p)| p.voice[0].as_str())
        .collect();
    assert_eq!(
        said,
        [
            "对面洛克出了风暴狂涌",
            "对面蕾欧娜出了心之钢",
            "对面安蓓萨出了无穷饥渴，还有 1 件",
            "对面维克托出了黯炎火炬",
            "对面洛克出了影焰，还有 1 件",
            "对面安蓓萨出了死亡之舞",
            "对面岩雀出了炽天使之拥",
            "对面维克托出了中娅",
            "对面岩雀出了重伤",
            "对面蕾欧娜出了贾修",
            "对面安蓓萨出了狂妄",
            "对面洛克出了虚空",
            "对面蕾欧娜出了重伤",
            "对面安蓓萨出了大米",
            "对面蕾欧娜出了兰顿，还有 1 件",
            "对面岩雀出了影焰",
            "对面洛克出了帽子",
            "对面维克托出了帽子",
            "对面安蓓萨出了血手，还有 2 件",
        ],
        "只说对面的（这把我方没出关键装备）"
    );
    let vik = items
        .iter()
        .find(|(_, p)| p.voice.first().map(String::as_str) == Some("对面维克托出了中娅"))
        .unwrap();
    assert_eq!(
        vik.1.voice,
        ["对面维克托出了中娅", "维克托出了中娅", "对面维克托有中娅了"],
        "几个说法各不一样"
    );
    // 队友和你自己的成装：只有一行，不出声
    let quiet: Vec<String> = items
        .iter()
        .filter(|(_, p)| p.voice.is_empty())
        .map(|(_, p)| line_text(p))
        .collect();
    for line in [
        "我方易出了破败",
        "你出了收集者",
        "你出了大米",
        "我方卡牌出了帽子",
    ] {
        assert!(quiet.iter().any(|l| l == line), "{line}：{quiet:?}");
    }
    assert!(
        quiet.iter().all(|l| !l.contains("对面")),
        "对面的都出声：{quiet:?}"
    );
    // 一行里两边都有：对面的打头；这一轮只有队友是安静的，语音只说对面那个
    let mixed = items
        .iter()
        .find(|(_, p)| line_text(p) == "对面洛克 帽子 / 我方卡牌 兰德里")
        .unwrap();
    assert_eq!(mixed.1.voice[0], "对面洛克出了帽子");
    // 佛耶戈（我）附身成蕾欧娜、安蓓萨时身上是她们的装备：不算你出的（蕾欧娜的心之钢、安蓓萨的死亡之舞）
    let all = items
        .iter()
        .map(|(_, p)| line_text(p))
        .collect::<Vec<_>>()
        .join(
            "
",
        );
    assert!(
        !all.contains("你 心之钢") && !all.contains("你出了心之钢"),
        "{all}"
    );
    assert!(!all.contains("你 死亡之舞 / 你 无穷饥渴"), "{all}");
    assert!(
        pushes_of(&outs, PUSH_OBJECTIVE).is_empty(),
        "海斗没有峡谷资源"
    );
    assert!(pushes_of(&outs, PUSH_SPELL_READY).is_empty());
    let down = pushes_of(&outs, PUSH_ENEMIES_DOWN);
    assert_eq!(down.len(), 1, "{down:?}");
    assert!((1020.0..1040.0).contains(&down[0].0), "{}", down[0].0);
    let first = down[0].1;
    assert_eq!(first.voice[0], "对面死了 3 个，最快 32 秒复活", "{first:?}");
    // 照以前的规矩（死着的都算），这把里同时死 3 个以上的有 25 份
    let crowded = games
        .iter()
        .filter(|g| g.enemies().filter(|p| p.dead).count() >= DOWN_AT)
        .count();
    assert_eq!(crowded, 25);
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
    alerts.step(1, None, &games[dead_at - 1], Some(&data), &[]);
    let out = alerts.step(2, None, &games[dead_at], Some(&data), &[]);
    assert!(out.events.is_empty() && out.pushes.is_empty());
}

/// 重伤（2026-11-27 rc.14 试玩后改）：不管你玩什么英雄（我是佛耶戈、换成索拉卡都一样），对面谁身上第一次有了带重伤的
/// 装备就说一次——湮灭宝珠、棘刺背心这样的小件也算；之后升级成荆棘之甲不再说。
#[test]
fn grievous_wounds_whatever_you_play() {
    let data = data();
    for me in [None, Some("Soraka")] {
        let outs = run(&frames(me), Some(&data), &[]);
        let items = pushes_of(&outs, PUSH_ENEMY_ITEM);
        let grievous: Vec<&Push> = items
            .iter()
            .map(|(_, p)| *p)
            .filter(|p| line_text(p).contains("重伤"))
            .collect();
        let said: Vec<&str> = grievous.iter().map(|p| p.voice[0].as_str()).collect();
        assert_eq!(said, ["对面岩雀出了重伤", "对面蕾欧娜出了重伤"], "{me:?}");
        assert_eq!(
            grievous[1].voice,
            ["对面蕾欧娜出了重伤", "蕾欧娜出了重伤", "对面蕾欧娜有重伤了"]
        );
        // 蕾欧娜之后把棘刺背心合成荆棘之甲：不是第一次有重伤，是成装，照成装说（“反甲”）
        assert!(
            items
                .iter()
                .any(|(_, p)| line_text(p).contains("对面蕾欧娜 反甲")),
            "{me:?}"
        );
    }
}

/// 巨蛇之牙和复活甲一级，出了就说（海斗的 id 前面多两位也认得）；刚连上时身上已经有棘刺背心的，之后合成荆棘之甲
/// 不算“第一次有重伤”。
#[test]
fn serpents_fang_is_a_key_item_and_an_upgrade_is_not_new_grievous() {
    let item = |id: i64, name: &str| crate::client::live::LiveItem {
        id,
        name: name.into(),
    };
    // 对面：慎、亚索、卢锡安（rift 的第 3、4、5 个人）
    let mut g0 = rift(900.0, 11, json!([]));
    g0.players[3].items = vec![item(3076, "棘刺背心")];
    let mut g1 = g0.clone();
    g1.time = 902.0;
    g1.players[2].items = vec![item(226695, "巨蛇之牙")];
    g1.players[3].items = vec![item(3075, "荆棘之甲")];
    let mut alerts = Alerts::default();
    alerts.step(7, None, &g0, None, &[]);
    let out = alerts.step(7, None, &g1, None, &[]);
    let items: Vec<&Push> = out
        .pushes
        .iter()
        .filter(|p| p.trigger == PUSH_ENEMY_ITEM)
        .collect();
    assert_eq!(items.len(), 1, "{items:?}");
    assert_well_formed(items[0]);
    assert_eq!(items[0].voice[0], "对面慎出了巨蛇之牙");
    // 再下一份：亚索又出了一件死刑宣告，也不算第一次
    let mut g2 = g1.clone();
    g2.time = 904.0;
    g2.players[3].items.push(item(3123, "死刑宣告"));
    let out = alerts.step(7, None, &g2, None, &[]);
    assert!(out.pushes.is_empty(), "{:?}", out.pushes);
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

/// 峡谷资源刷新前 30 秒：小龙 5:00（之后被拿 + 5 分钟）、巢虫 8:00、先锋 15:00、大龙 20:00（被拿 + 6 分钟）、
/// 有一边拿到龙魂以后只刷远古龙（+ 6 分钟）。每 2 秒看一份（和主循环一样），每次刷新只说一次；说的是几点刷（宿主晚一点说
/// 也不会错，2026-11-28 玩家说“不太准”以后改）。
#[test]
fn rift_objectives_thirty_seconds_ahead() {
    let kill = |id: i64, t: f64, kind: &str, dragon: &str, killer: &str| json!({"EventID": id, "EventName": kind, "EventTime": t, "DragonType": dragon, "Stolen": "False", "KillerName": killer});
    // 我方拿了 4 条小龙（5:30、11:00、16:30、22:00），22:00 有了龙魂 → 远古龙 28:00；大龙 21:00 被对面拿了 → 27:00
    let all = [
        json!({"EventID": 0, "EventName": "GameStart", "EventTime": 0.0}),
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
        let out = alerts.step(7, Some(420), &rift(t, 1, Value::Array(events)), None, &[]);
        for p in out.pushes.iter().filter(|p| p.trigger == PUSH_OBJECTIVE) {
            assert_well_formed(p);
            said.push((t, p.voice[0].clone()));
        }
        t += 2.0;
    }
    let said: Vec<(f64, &str)> = said.iter().map(|(t, v)| (*t, v.as_str())).collect();
    assert_eq!(
        said,
        [
            (270.0, "小龙 5 分刷新"),
            (450.0, "虚空巢虫 8 分刷新"),
            (600.0, "小龙 10 分 30 秒刷新"),
            (870.0, "峡谷先锋 15 分刷新"),
            (930.0, "小龙 16 分刷新"),
            (1170.0, "大龙 20 分刷新"),
            (1260.0, "小龙 21 分 30 秒刷新"),
            (1590.0, "大龙 27 分刷新"),
            (1650.0, "远古龙 28 分刷新"),
        ]
    );
    // 一行：资源和几点刷；几个说法轮着用
    let push = objectives(
        &rift(600.0, 1, Value::Array(all[..2].to_vec())),
        &RIFT_RULES,
        7,
    )
    .into_iter()
    .find(|p| p.key == "objective:7:dragon:630")
    .unwrap();
    assert_eq!(
        push.line.as_ref().unwrap().cells,
        [cell("小龙", Some(PushTone::Ok)), cell("10:30 刷新", None)]
    );
    assert_eq!(
        push.voice,
        [
            "小龙 10 分 30 秒刷新",
            "10 分 30 秒刷小龙",
            "小龙快刷了，10 分 30 秒"
        ]
    );
    // 规则表就是 2026 赛季的：大龙 20:00（26.1）、先锋 15:00（25.09）、巢虫 8:00
    assert_eq!(RIFT_RULES.baron_first, 1200.0);
    assert_eq!(RIFT_RULES.herald, Some(900.0));
    assert_eq!(RIFT_RULES.grubs, Some(480.0));
}

/// 刚连上（或者一轮慢了）的时候已经在“刷新前 30 秒”里：照说（说的是几点刷，晚说也对），只说一次；刷了以后不说。
#[test]
fn an_objective_already_inside_the_lead_is_still_said_once() {
    let mut alerts = Alerts::default();
    assert!(alerts
        .step(7, None, &rift(281.0, 1, json!([])), None, &[])
        .pushes
        .is_empty());
    let out = alerts.step(7, None, &rift(283.0, 1, json!([])), None, &[]);
    let said: Vec<&str> = out
        .pushes
        .iter()
        .filter(|p| p.trigger == PUSH_OBJECTIVE)
        .map(|p| p.voice[0].as_str())
        .collect();
    assert_eq!(said, ["小龙 5 分刷新"]);
    let again = alerts.step(7, None, &rift(285.0, 1, json!([])), None, &[]);
    assert!(again.pushes.iter().all(|p| p.trigger != PUSH_OBJECTIVE));
    // 5:02 才连上：已经刷了，不说
    let mut late = Alerts::default();
    late.step(8, None, &rift(302.0, 1, json!([])), None, &[]);
    let out = late.step(8, None, &rift(304.0, 1, json!([])), None, &[]);
    assert!(out.pushes.iter().all(|p| p.trigger != PUSH_OBJECTIVE));
}

/// 快速模式也在峡谷打，资源刷得早：不照峡谷的时间报。局内接口写 `SWIFTPLAY` 的、写 `CLASSIC` 但这局的队列是 480 的都不报；
/// 技能转好照旧按分路叫。
#[test]
fn swiftplay_gets_no_rift_timers() {
    let said = |queue: Option<i64>, mode: &str| -> usize {
        let mut alerts = Alerts::default();
        let mut n = 0;
        for t in [268.0, 270.0, 272.0] {
            let mut g = rift(t, 1, json!([]));
            g.mode = mode.into();
            n += alerts
                .step(7, queue, &g, None, &[])
                .pushes
                .iter()
                .filter(|p| p.trigger == PUSH_OBJECTIVE)
                .count();
        }
        n
    };
    assert_eq!(said(Some(420), "CLASSIC"), 1, "排位照报");
    assert_eq!(said(None, "CLASSIC"), 1, "不知道队列：照 gameMode");
    assert_eq!(said(Some(SWIFTPLAY_QUEUE), "CLASSIC"), 0);
    assert_eq!(said(None, "SWIFTPLAY"), 0);
    assert!(!rift_timers(&rift(1.0, 1, json!([])), Some(480)));
}

/// 一条拿龙事件（`KillerName` 照局内接口：名字不带编号；隐藏了身份的写英雄的称号）。
fn dragon(id: i64, t: f64, kind: &str, killer: &str) -> Value {
    json!({"EventID": id, "EventName": "DragonKill", "EventTime": t, "DragonType": kind, "Stolen": "False", "KillerName": killer})
}

/// 从 `GameStart`（0）起编好号的事件列表。
fn numbered(events: Vec<Value>) -> Value {
    let mut all = vec![json!({"EventID": 0, "EventName": "GameStart", "EventTime": 0.0})];
    all.extend(events);
    for (i, e) in all.iter_mut().enumerate() {
        e["EventID"] = json!(i);
    }
    Value::Array(all)
}

fn spawn_of(game: &LiveGame, objective: Objective) -> Option<f64> {
    next_spawns(game, &RIFT_RULES)
        .into_iter()
        .find(|(o, _)| *o == objective)
        .map(|(_, t)| t)
}

/// 龙魂（2026-11-28 改）：认不出拿龙的是谁的那几条不再都算给对面——两边都可能，说不准就不报龙（不把远古龙说成小龙）；
/// 拿过远古龙就一定有龙魂；两边加起来 7 条一定有一边到了；龙魂那条是最后一条小龙。
#[test]
fn dragon_soul_is_not_made_up_from_unknown_killers() {
    let game = |events: Vec<Value>| rift(1500.0, 11, numbered(events));
    // 对面 2 条 + 认不出的 2 条：以前算成对面龙魂、报远古龙；现在说不准，不报
    let g = game(vec![
        dragon(1, 330.0, "Fire", "敌人一"),
        dragon(2, 660.0, "Water", "某个路人"),
        dragon(3, 990.0, "Earth", "敌人二"),
        dragon(4, 1290.0, "Air", "某个路人"),
    ]);
    assert_eq!(spawn_of(&g, Objective::Dragon), None);
    assert_eq!(spawn_of(&g, Objective::Elder), None);
    assert_eq!(spawn_of(&g, Objective::Baron), Some(1200.0), "大龙不受影响");
    // 我方 1 条 + 认不出的 2 条：哪边都到不了 4 条，照旧刷小龙
    let g = game(vec![
        dragon(1, 330.0, "Fire", "玩家"),
        dragon(2, 660.0, "Water", "某个路人"),
        dragon(3, 990.0, "Earth", "某个路人"),
    ]);
    assert_eq!(spawn_of(&g, Objective::Dragon), Some(1290.0));
    // 3 + 3 + 认不出的 1 条 = 7 条：一定有一边到了，龙魂是最后那条（35:30）→ 远古龙 41:30
    let mut events: Vec<Value> = (1..=6)
        .map(|i| {
            dragon(
                i,
                300.0 * i as f64 + 30.0,
                "Fire",
                if i % 2 == 0 { "敌人一" } else { "队友" },
            )
        })
        .collect();
    events.push(dragon(7, 2130.0, "Chemtech", "某个路人"));
    let g = game(events);
    assert_eq!(spawn_of(&g, Objective::Dragon), None);
    assert_eq!(spawn_of(&g, Objective::Elder), Some(2130.0 + 360.0));
    // 看到了远古龙被拿（之前的小龙认不出是谁的）：一定有龙魂，远古龙被拿 + 6 分钟
    let g = game(vec![
        dragon(1, 330.0, "Fire", "某个路人"),
        dragon(2, 660.0, "Water", "某个路人"),
        dragon(3, 990.0, "Earth", "敌人一"),
        dragon(4, 1290.0, "Air", "敌人一"),
        dragon(5, 1700.0, "Elder", "玩家"),
    ]);
    assert_eq!(spawn_of(&g, Objective::Dragon), None);
    assert_eq!(spawn_of(&g, Objective::Elder), Some(2060.0));
}

/// `KillerName` 的几种写法都认得：名字不带编号（实录的样子）、带编号的 Riot ID、隐藏了身份的人写英雄的称号；抢的龙算抢的那边。
#[test]
fn dragon_killers_are_matched_the_way_the_live_api_names_them() {
    let events = numbered(vec![
        dragon(1, 330.0, "Fire", "敌人一"),
        dragon(2, 660.0, "Water", "敌人一#1"),
        dragon(3, 990.0, "Earth", "疾风剑豪"),
        json!({"EventName": "DragonKill", "EventTime": 1290.0, "DragonType": "Air", "Stolen": "True", "KillerName": "敌人三"}),
    ]);
    let mut g = rift(1500.0, 11, events);
    // 对面亚索隐藏了身份：riotId 是 `#`、名字空、summonerName 是称号
    let yasuo = g.players.iter_mut().find(|p| p.alias == "Yasuo").unwrap();
    yasuo.riot_id = "#".into();
    yasuo.name = String::new();
    yasuo.summoner = "疾风剑豪".into();
    assert_eq!(g.is_ally_name("疾风剑豪"), Some(false));
    assert_eq!(g.is_ally_name("敌人一#1"), Some(false));
    assert_eq!(spawn_of(&g, Objective::Dragon), None, "对面 4 条：龙魂");
    assert_eq!(spawn_of(&g, Objective::Elder), Some(1290.0 + 360.0));
}

/// 事件列表缺了几条（编号断了：断线重连、接口漏了）：不知道缺的是不是拿龙、拿大龙，龙和大龙都不报；巢虫、先锋照报。
#[test]
fn a_gap_in_the_event_list_drops_dragon_and_baron_timers() {
    let events = json!([
        {"EventID": 0, "EventName": "GameStart", "EventTime": 0.0},
        dragon(5, 330.0, "Fire", "玩家"),
    ]);
    let g = rift(800.0, 9, events);
    let kinds: Vec<Objective> = next_spawns(&g, &RIFT_RULES)
        .into_iter()
        .map(|(o, _)| o)
        .collect();
    assert_eq!(kinds, [Objective::Grubs, Objective::Herald]);
    // 编号连着：都在
    let g = rift(800.0, 9, numbered(vec![dragon(1, 330.0, "Fire", "玩家")]));
    let kinds: Vec<Objective> = next_spawns(&g, &RIFT_RULES)
        .into_iter()
        .map(|(o, _)| o)
        .collect();
    assert_eq!(
        kinds,
        [
            Objective::Dragon,
            Objective::Grubs,
            Objective::Herald,
            Objective::Baron
        ]
    );
}

/// 峡谷里到 6 级：慎（全图大招）按外号叫，对线的亚索按分路叫（“对面中单”），下路卢锡安两样都不是、不说。
/// 同一轮的并成一条；之后再升级不再说。大乱斗里没有对线对手。
#[test]
fn level_six_names_long_ults_and_the_lane_opponent() {
    let mut alerts = Alerts::default();
    alerts.step(7, None, &rift(400.0, 5, json!([])), None, &[]);
    let out = alerts.step(7, None, &rift(402.0, 6, json!([])), None, &[]);
    let ult: Vec<&Push> = out
        .pushes
        .iter()
        .filter(|p| p.trigger == PUSH_ENEMY_ULT)
        .collect();
    assert_eq!(ult.len(), 1);
    assert_well_formed(ult[0]);
    assert_eq!(ult[0].voice[0], "对面慎、中单 6 级了");
    assert_eq!(ult[0].key, "enemy-ult:7:402");
    let again = alerts.step(7, None, &rift(404.0, 7, json!([])), None, &[]);
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
    alerts.step(8, None, &aram(400.0, 5), None, &[]);
    let out = alerts.step(8, None, &aram(402.0, 6), None, &[]);
    let ult: Vec<&Push> = out
        .pushes
        .iter()
        .filter(|p| p.trigger == PUSH_ENEMY_ULT)
        .collect();
    assert_eq!(ult[0].voice[0], "对面慎 6 级了");
    assert!(out.pushes.iter().all(|p| p.trigger != PUSH_OBJECTIVE));
}

#[test]
fn simultaneous_milestones_show_both_teams_and_the_player() {
    let item = |id| crate::client::live::LiveItem {
        id,
        name: String::new(),
    };
    let mut before = rift(400.0, 5, json!([]));
    before.players[0].level = 5;
    before.players[1].level = 5;
    let mut after = rift(402.0, 6, json!([]));
    after.players[0].items = vec![item(3140)];
    after.players[1].items = vec![item(3102)];
    after.players[2].items = vec![item(3157)];
    let mut alerts = Alerts::default();
    assert!(alerts.step(7, None, &before, None, &[]).pushes.is_empty());
    let out = alerts.step(7, None, &after, None, &[]);
    assert_eq!(out.pushes.len(), 2);
    let levels = &out.pushes[0];
    assert_eq!(levels.trigger, PUSH_ENEMY_ULT);
    assert_well_formed(levels);
    assert_eq!(
        levels.line.as_ref().unwrap().cells,
        [
            cell("对面慎、中单", Some(PushTone::Bad)),
            cell("你和我方金克丝", Some(PushTone::Good)),
            cell("6 级了，有大招", None),
        ]
    );
    assert_eq!(levels.voice[0], "对面慎、中单、你和我方金克丝 6 级了");
    let items = &out.pushes[1];
    assert_eq!(items.trigger, PUSH_ENEMY_ITEM);
    assert_well_formed(items);
    assert_eq!(
        items.line.as_ref().unwrap().cells,
        [
            cell("对面慎 中娅", Some(PushTone::Bad)),
            cell("你 水银", Some(PushTone::Good)),
            cell("我方金克丝 女妖", Some(PushTone::Good)),
        ]
    );
    assert_eq!(items.voice[0], "对面慎出中娅，你出水银");
    after.time = 404.0;
    assert!(alerts.step(7, None, &after, None, &[]).pushes.is_empty());
}

/// 关键等级（2026-11-28 玩家：“卡萨丁天使蒙多到 16 级，船长 13 级”）：我是中单阿狸（一直 5 级），队友上单蒙多；
/// 对面上单船长、中单卡萨丁（对线对手）、下路天使。`levels` 依次是蒙多、船长、卡萨丁、天使的等级。
fn spikes(time: f64, levels: [i64; 4]) -> LiveGame {
    parse_all_game_data(&json!({
        "activePlayer": {"riotId": "玩家#1"},
        "allPlayers": [
            rift_player("玩家", "Ahri", "ORDER", "MIDDLE", 5),
            rift_player("队友", "DrMundo", "ORDER", "TOP", levels[0]),
            rift_player("敌人一", "Gangplank", "CHAOS", "TOP", levels[1]),
            rift_player("敌人二", "Kassadin", "CHAOS", "MIDDLE", levels[2]),
            rift_player("敌人三", "Kayle", "CHAOS", "BOTTOM", levels[3]),
        ],
        "events": {"Events": []},
        "gameData": {"gameMode": "CLASSIC", "gameTime": time},
    }))
    .unwrap()
}

fn level_pushes(out: &Out) -> Vec<&Push> {
    out.pushes
        .iter()
        .filter(|p| p.trigger == PUSH_ENEMY_ULT)
        .collect()
}

/// 6 级照旧（船长是远距离大招、卡萨丁是对线对手；蒙多、天使 6 级不说）；船长 13 级、卡萨丁 天使 蒙多 16 级各说一次，
/// 双方都说，同一轮同一级的并成一句；之后再升级不再说。
#[test]
fn key_levels_cover_power_spikes_for_both_teams() {
    let mut alerts = Alerts::default();
    alerts.step(7, None, &spikes(400.0, [5, 5, 5, 5]), None, &[]);
    let out = alerts.step(7, None, &spikes(402.0, [6, 6, 6, 6]), None, &[]);
    let six = level_pushes(&out);
    assert_eq!(six.len(), 1);
    assert_well_formed(six[0]);
    assert_eq!(six[0].voice[0], "对面船长、中单 6 级了");

    let out = alerts.step(7, None, &spikes(800.0, [12, 13, 12, 12]), None, &[]);
    let gp = level_pushes(&out);
    assert_eq!(gp.len(), 1);
    assert_well_formed(gp[0]);
    assert_eq!(
        gp[0].line.as_ref().unwrap().cells,
        [cell("对面船长", Some(PushTone::Bad)), cell("13 级了", None)]
    );
    assert_eq!(gp[0].voice, ["对面船长 13 级了", "船长到 13 级了"]);

    let out = alerts.step(7, None, &spikes(1500.0, [16, 15, 16, 16]), None, &[]);
    let sixteen = level_pushes(&out);
    assert_eq!(sixteen.len(), 1);
    assert_well_formed(sixteen[0]);
    assert_eq!(
        sixteen[0].line.as_ref().unwrap().cells,
        [
            cell("对面卡萨丁、天使", Some(PushTone::Bad)),
            cell("我方蒙多", Some(PushTone::Good)),
            cell("16 级了", None),
        ]
    );
    assert_eq!(sixteen[0].voice[0], "对面卡萨丁、天使、我方蒙多 16 级了");

    let out = alerts.step(7, None, &spikes(1600.0, [17, 16, 17, 17]), None, &[]);
    assert!(level_pushes(&out).is_empty());
}

/// 几个等级碰在同一轮：一行写成一整句；连起来超过 20 个字时只说“好几个人到关键等级了”。
#[test]
fn different_key_levels_in_one_tick_are_joined() {
    let mut alerts = Alerts::default();
    alerts.step(7, None, &spikes(400.0, [12, 12, 15, 12]), None, &[]);
    let out = alerts.step(7, None, &spikes(402.0, [12, 13, 16, 12]), None, &[]);
    let joined = level_pushes(&out);
    assert_eq!(joined.len(), 1);
    assert_well_formed(joined[0]);
    assert_eq!(
        joined[0].line.as_ref().unwrap().cells,
        [cell("对面船长 13 级了，对面卡萨丁 16 级了", None)]
    );
    assert_eq!(joined[0].voice, ["好几个人到关键等级了"]);
}

/// 峡谷用的一小份装备资料（照 2026-11-23 录的 `items.json`）：无尽之刃、灭世者的死亡之帽、心之钢、海克斯注力刚壁是成装，
/// 暴风之剑（1300，能合成无尽）不是。
fn rift_items() -> GameData {
    let item = |id: i64, name: &str, price: i64, to: Value| json!({"id": id, "name": name, "priceTotal": price, "to": to});
    GameData::from_json(
        &Value::Null,
        &Value::Null,
        &json!([
            item(3031, "无尽之刃", 3500, json!([])),
            item(3089, "灭世者的死亡之帽", 3500, json!([])),
            item(3084, "心之钢", 3000, json!([])),
            item(3073, "海克斯注力刚壁", 3000, json!([])),
            item(1038, "暴风之剑", 1300, json!([3031])),
        ]),
        &Value::Null,
        &Value::Null,
    )
}

/// 出成装了（2026-11-28 玩家：“如果某个英雄合成了成装，比如无尽帽子之类的，也直接提醒”）：谁出了成装都提醒，用玩家的
/// 叫法（无尽、帽子），没有叫法的用局内接口里的名字、再没有用客户端资料里的名字；对面的说一句，队友和你自己的只显示一行；
/// 小件不算；同一个人同一件只一次。
#[test]
fn any_completed_item_is_shown_and_enemy_ones_are_said() {
    let data = rift_items();
    let item = |id: i64, name: &str| crate::client::live::LiveItem {
        id,
        name: name.into(),
    };
    let g0 = rift(900.0, 11, json!([]));
    let mut alerts = Alerts::default();
    alerts.step(7, None, &g0, Some(&data), &[]);
    let items_of = |out: &Out| -> Vec<Push> {
        out.pushes
            .iter()
            .filter(|p| p.trigger == PUSH_ENEMY_ITEM)
            .cloned()
            .collect()
    };
    // 同一轮：对面慎出了无尽、我方金克丝出了帽子 → 一行两格，只说对面那件
    let mut g1 = g0.clone();
    g1.time = 902.0;
    g1.players[2].items = vec![item(3031, "无尽之刃")];
    g1.players[1].items = vec![item(3089, "灭世者的死亡之帽")];
    let out = items_of(&alerts.step(7, None, &g1, Some(&data), &[]));
    assert_eq!(out.len(), 1);
    assert_well_formed(&out[0]);
    assert_eq!(
        out[0].line.as_ref().unwrap().cells,
        [
            cell("对面慎 无尽", Some(PushTone::Bad)),
            cell("我方金克丝 帽子", Some(PushTone::Good)),
        ]
    );
    assert_eq!(
        out[0].voice,
        ["对面慎出了无尽", "慎出了无尽", "对面慎有无尽了"]
    );
    // 你自己出了无尽：只有一行，不出声；暴风之剑（小件）不算
    let mut g2 = g1.clone();
    g2.time = 904.0;
    g2.players[0].items = vec![item(3031, "无尽之刃"), item(1038, "暴风之剑")];
    let out = items_of(&alerts.step(7, None, &g2, Some(&data), &[]));
    assert_eq!(out.len(), 1);
    assert_well_formed(&out[0]);
    assert_eq!(line_text(&out[0]), "你出了无尽");
    assert!(out[0].voice.is_empty(), "队友和你自己的成装不出声");
    // 没有叫法的：局内接口里的名字；局内接口没写名字的：客户端资料里的名字（海斗的 id 前面多两位也认得）
    let mut g3 = g2.clone();
    g3.time = 906.0;
    g3.players[2].items.push(item(3084, "心之钢"));
    let out = items_of(&alerts.step(7, None, &g3, Some(&data), &[]));
    assert_eq!(line_text(&out[0]), "对面慎出了心之钢");
    let mut g3 = g3.clone();
    g3.time = 907.0;
    g3.players[4].champion = "卢锡安".into();
    g3.players[4].items = vec![item(223073, "")];
    let out = items_of(&alerts.step(7, None, &g3, Some(&data), &[]));
    assert_well_formed(&out[0]);
    assert_eq!(line_text(&out[0]), "对面卢锡安出了海克斯注力刚壁");
    assert_eq!(out[0].voice[0], "对面卢锡安出了海克斯注力刚壁");
    // 慎卖了无尽又买回来：不再说
    let mut g4 = g3.clone();
    g4.time = 908.0;
    g4.players[2].items.retain(|i| i.id != 3031);
    assert!(items_of(&alerts.step(7, None, &g4, Some(&data), &[])).is_empty());
    let mut g5 = g4.clone();
    g5.time = 910.0;
    g5.players[2].items.push(item(3031, "无尽之刃"));
    assert!(items_of(&alerts.step(7, None, &g5, Some(&data), &[])).is_empty());
}

/// 刚连上时身上已经有的成装不说；佛耶戈附身时（英雄和第一次看到时不一样）身上是被附身那个人的装备，附身完又变回来，
/// 都不算他出了装备。
#[test]
fn items_seen_at_connect_and_while_possessing_are_not_new() {
    let data = rift_items();
    let item = |id: i64, name: &str| crate::client::live::LiveItem {
        id,
        name: name.into(),
    };
    let mut g0 = rift(900.0, 11, json!([]));
    g0.players[2].alias = "Viego".into();
    g0.players[2].champion = "佛耶戈".into();
    g0.players[2].items = vec![item(3089, "灭世者的死亡之帽")];
    let mut alerts = Alerts::default();
    alerts.step(7, None, &g0, Some(&data), &[]);
    // 附身成我方金克丝：身上是她的无尽
    let mut g1 = g0.clone();
    g1.time = 902.0;
    g1.players[2].alias = "Jinx".into();
    g1.players[2].items = vec![item(3031, "无尽之刃")];
    let out = alerts.step(7, None, &g1, Some(&data), &[]);
    assert!(
        out.pushes.iter().all(|p| p.trigger != PUSH_ENEMY_ITEM),
        "{:?}",
        out.pushes
    );
    // 变回来：帽子是连上时就有的
    let mut g2 = g0.clone();
    g2.time = 904.0;
    let out = alerts.step(7, None, &g2, Some(&data), &[]);
    assert!(
        out.pushes.iter().all(|p| p.trigger != PUSH_ENEMY_ITEM),
        "{:?}",
        out.pushes
    );
    // 自己出了无尽：说
    let mut g3 = g2.clone();
    g3.time = 906.0;
    g3.players[2].items.push(item(3031, "无尽之刃"));
    let out = alerts.step(7, None, &g3, Some(&data), &[]);
    let said: Vec<&str> = out
        .pushes
        .iter()
        .filter(|p| p.trigger == PUSH_ENEMY_ITEM)
        .map(|p| p.voice[0].as_str())
        .collect();
    assert_eq!(said, ["对面佛耶戈出了无尽"]);
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
            None,
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
        p.respawn = 40.0;
        p.items = vec![crate::client::live::LiveItem {
            id: 3157,
            name: "中娅沙漏".into(),
        }];
    }
    let mut alerts = Alerts::default();
    alerts.step(7, None, &g0, None, &[]);
    let out = alerts.step(7, None, &g1, None, &[]);
    let kinds: Vec<&str> = out.pushes.iter().map(|p| p.trigger.as_str()).collect();
    assert_eq!(kinds, [PUSH_ENEMY_ULT, PUSH_ENEMY_ITEM, PUSH_ENEMIES_DOWN]);
    let priority = |id: &str| crate::manifest().push_spec(id).unwrap().priority;
    let loudest = out
        .pushes
        .iter()
        .max_by_key(|p| priority(&p.trigger))
        .unwrap();
    assert_eq!(loudest.trigger, PUSH_ENEMIES_DOWN, "同一时刻宿主只说这一句");
    assert_eq!(loudest.voice[0], "对面死了 3 个，最快 40 秒复活");
    let items = out
        .pushes
        .iter()
        .find(|p| p.trigger == PUSH_ENEMY_ITEM)
        .unwrap();
    assert_eq!(items.voice[0], "对面慎出了中娅，还有 2 件");
    assert_eq!(items.line.as_ref().unwrap().cells.len(), 3);
    assert_eq!(items.line.as_ref().unwrap().cells[0].text, "对面慎 中娅");
    // 下一轮还都死着：不再说
    let mut g2 = g1.clone();
    g2.time = 604.0;
    let out = alerts.step(7, None, &g2, None, &[]);
    assert!(out.pushes.is_empty(), "{:?}", out.pushes);
    // 换了一局（对局 id 变了）：从头来，第一份只记下样子
    let out = alerts.step(8, None, &g1, None, &[]);
    assert!(out.pushes.is_empty() && out.events.is_empty());
}

/// 峡谷一份（第 `time` 秒）：我是中单阿狸，对面五个人各还要几秒复活（`None` = 活着）。
fn rift_down(time: f64, respawns: [Option<f64>; 5]) -> LiveGame {
    let enemies = ["Shen", "Yasuo", "Lucian", "LeeSin", "Thresh"];
    let mut players = vec![rift_player("玩家", "Ahri", "ORDER", "MIDDLE", 16)];
    for (i, (alias, respawn)) in enemies.iter().zip(respawns).enumerate() {
        let mut p = rift_player(&format!("敌人{i}"), alias, "CHAOS", "NONE", 16);
        if let Some(r) = respawn {
            p["isDead"] = json!(true);
            p["respawnTimer"] = json!(r);
        }
        players.push(p);
    }
    parse_all_game_data(&json!({
        "activePlayer": {"riotId": "玩家#1"},
        "allPlayers": players,
        "events": {"Events": []},
        "gameData": {"gameMode": "CLASSIC", "gameTime": time},
    }))
    .unwrap()
}

/// 对面阵亡（2026-11-27 rc.14 试玩后改：海斗里一直在报）：只数还要 30 秒以上才复活的；这一份刚到 3 个（上一份不到）才说，
/// 几个、最快几秒都按这几个算；上一份已经 3 个的不再说；两次之间至少 60 秒。
#[test]
fn enemies_down_counts_only_long_respawns() {
    let mut alerts = Alerts::default();
    let mut said = |time: f64, respawns: [Option<f64>; 5]| -> Option<String> {
        let out = alerts.step(7, None, &rift_down(time, respawns), None, &[]);
        let push = out
            .pushes
            .into_iter()
            .find(|p| p.trigger == PUSH_ENEMIES_DOWN)?;
        assert_well_formed(&push);
        Some(push.voice[0].clone())
    };
    assert_eq!(said(1500.0, [None; 5]), None);
    // 死了三个，有一个 25 秒就复活：不算
    assert_eq!(
        said(1502.0, [Some(40.0), Some(38.0), Some(25.0), None, None]),
        None
    );
    // 又死一个（45 秒）：够久的刚到 3 个。那个 23 秒的不算进人数和“最快”
    assert_eq!(
        said(
            1504.0,
            [Some(38.0), Some(36.0), Some(23.0), Some(45.0), None]
        ),
        Some("对面死了 3 个，最快 36 秒复活".to_string())
    );
    // 上一份已经 3 个：再死一个也不说
    assert_eq!(
        said(
            1506.0,
            [Some(36.0), Some(34.0), Some(21.0), Some(43.0), Some(50.0)]
        ),
        None
    );
    // 都复活了，46 秒后又倒了三个：离上次不到 60 秒，不说
    assert_eq!(
        said(1540.0, [None, None, None, Some(9.0), Some(16.0)]),
        None
    );
    assert_eq!(
        said(1550.0, [Some(40.0), Some(40.0), Some(40.0), None, None]),
        None
    );
    // 一分半以后一下倒了四个：说，人数是四个
    assert_eq!(said(1590.0, [None; 5]), None);
    assert_eq!(
        said(
            1602.0,
            [Some(31.0), Some(33.0), Some(35.0), Some(44.0), None]
        ),
        Some("对面死了 4 个，最快 31 秒复活".to_string())
    );
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
    assert_eq!(m.push_spec(PUSH_ENEMY_ITEM).unwrap().title, "出成装了");
    // 玩家的叫法：不重复、不和关键装备重，都是短的（两三个字，说得顺）
    let mut ids: Vec<i64> = SHORT_ITEMS.iter().map(|(id, _)| *id).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), SHORT_ITEMS.len());
    for (id, name) in SHORT_ITEMS {
        assert!(KEY_ITEMS.iter().all(|(k, _)| k != id), "{id}");
        assert!((2..=4).contains(&name.chars().count()), "{name}");
    }
    // 关键装备两张表一样（推送和装备事件）
    let ids: Vec<i64> = KEY_ITEMS.iter().map(|(id, _)| *id).collect();
    assert_eq!(ids, crate::client::events::KEY_ITEM_IDS);
    // 表里的英雄 id 没有重复
    let mut aliases: Vec<&str> = LONG_ULTS.iter().map(|(a, _)| *a).collect();
    aliases.sort_unstable();
    aliases.dedup();
    assert_eq!(aliases.len(), LONG_ULTS.len());
}
