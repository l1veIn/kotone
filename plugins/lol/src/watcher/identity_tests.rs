//! 隐藏了身份的人、电脑（ADR 043 D9；讨论记录第八节“开局战绩里我方少两个人”）。
//!
//! 那一把海斗（`target/lol-probe/20261126-mayhem-midgame`，只录了局内接口）里我方卡牌大师、无极剑圣隐藏了身份：
//! 局内接口里名字是空的，客户端也不给 puuid。这里照它拼一份 gameflow 的 session（隐藏的两个人 puuid 是空的），
//! 看名单、合并、局内接口补电脑、页面数据和给她的字。

use serde_json::{json, Value};

use super::{mark_bots, members, merge_session, Match};
use crate::client::history::PlayerHistory;
use crate::client::live::{parse_all_game_data, tests::mayhem_hidden_json};
use crate::client::text::{self, Identity};
use crate::client::GameData;
use crate::pages;
use crate::LolPlugin;

/// 录的那一把的 10 个英雄（英文 id → 编号；Locke 是新英雄，编号是编的）。
const CHAMPIONS: [(&str, i64, &str); 10] = [
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
];

fn data() -> GameData {
    let champions: Vec<Value> = CHAMPIONS
        .iter()
        .map(|(alias, id, short)| json!({"id": id, "name": short, "description": short, "alias": alias}))
        .collect();
    GameData::from_json(
        &Value::Array(champions),
        &Value::Null,
        &Value::Null,
        &Value::Null,
        &json!([{"id": 2400, "name": "海克斯大乱斗", "pickMode": "AllRandomPickStrategy"}]),
    )
}

/// 照录的局内数据拼的 gameflow session：ORDER 是 teamOne；名字是空的（隐藏了身份）的人 puuid 也是空的。
fn session(live: &Value) -> Value {
    let mut teams = (Vec::new(), Vec::new());
    for (i, p) in live["allPlayers"].as_array().unwrap().iter().enumerate() {
        let raw = p["rawChampionName"]
            .as_str()
            .unwrap()
            .rsplit('_')
            .next()
            .unwrap();
        let id = CHAMPIONS.iter().find(|(a, _, _)| *a == raw).unwrap().1;
        let hidden = p["riotIdGameName"].as_str() == Some("");
        let row = json!({
            "championId": id,
            "puuid": if hidden { String::new() } else { format!("p{i}") },
            "summonerId": if hidden { 0 } else { 1000 + i },
            "summonerInternalName": "",
            "summonerName": "",
            "selectedPosition": "NONE",
            "teamParticipantId": i + 1,
            "profileIconId": 29,
        });
        if p["team"] == "ORDER" {
            teams.0.push(row);
        } else {
            teams.1.push(row);
        }
    }
    json!({"gameData": {"gameId": 77, "queue": {"id": 2400, "gameMode": "KIWI"},
        "teamOne": teams.0, "teamTwo": teams.1, "playerChampionSelections": []}})
}

#[test]
fn hidden_players_stay_in_the_match_and_are_not_looked_up() {
    let live = mayhem_hidden_json();
    let found = members(&session(&live));
    let ours: Vec<(i64, Identity)> = found
        .iter()
        .filter(|m| m.team == 1)
        .map(|m| (m.champion_id, m.identity))
        .collect();
    assert_eq!(
        ours,
        [
            (234, Identity::Known),
            (236, Identity::Known),
            (22, Identity::Known),
            (4, Identity::Hidden),
            (11, Identity::Hidden),
        ],
        "ORDER 五个人都在，隐藏了身份的两个不当成电脑"
    );
    assert_eq!(found.len(), 10);

    // 合进这局：只查 8 个看得到的人
    let mut m = Match {
        game_id: 77,
        queue_id: 2400,
        ..Match::default()
    };
    let missing = merge_session(&mut m, found.clone());
    assert_eq!(missing.len(), 8, "{missing:?}");
    assert!(missing.iter().all(|p| !p.is_empty()));
    assert_eq!(m.members.len(), 10);
    // 下一轮同样的名单：不重复查，人数不变
    assert!(merge_session(&mut m, found.clone()).is_empty());
    assert_eq!(m.members.len(), 10);

    // 局内接口里都不是电脑：不动
    let game = parse_all_game_data(&live).unwrap();
    mark_bots(&mut m.members, &game, Some(&data()));
    assert_eq!(
        m.members
            .iter()
            .filter(|x| x.identity == Identity::Hidden)
            .count(),
        2
    );
}

#[test]
fn bots_are_what_the_apis_flag() {
    // 局内接口说卡牌大师是电脑：改成电脑；下一轮 gameflow 的名单照旧（没标）也留着电脑
    let mut live = mayhem_hidden_json();
    live["allPlayers"][3]["isBot"] = json!(true);
    let found = members(&session(&live));
    let mut m = Match::default();
    merge_session(&mut m, found.clone());
    mark_bots(
        &mut m.members,
        &parse_all_game_data(&live).unwrap(),
        Some(&data()),
    );
    let identity = |m: &Match, champion: i64| {
        m.members
            .iter()
            .find(|x| x.champion_id == champion)
            .unwrap()
            .identity
    };
    assert_eq!(identity(&m, 4), Identity::Bot);
    assert_eq!(identity(&m, 11), Identity::Hidden);
    merge_session(&mut m, found);
    assert_eq!(identity(&m, 4), Identity::Bot, "下一轮不变回隐藏了身份");

    // 客户端自己标了电脑（botDifficulty）：不查，puuid 也不用
    let mut flow = session(&mayhem_hidden_json());
    flow["gameData"]["teamTwo"][0]["botDifficulty"] = json!("MEDIUM");
    flow["gameData"]["teamTwo"][1]["botDifficulty"] = json!("NONE");
    let found = members(&flow);
    let theirs: Vec<(Identity, String)> = found
        .iter()
        .filter(|m| m.team == 2)
        .take(2)
        .map(|m| (m.identity, m.puuid.clone()))
        .collect();
    assert_eq!(
        theirs,
        [
            (Identity::Bot, String::new()),
            (Identity::Known, "p6".to_string())
        ]
    );
}

/// 页面数据和给她的字：我方 5 行（两行“隐藏了身份”、不算“还在查”），给她的字里写明，不写突破口。
#[test]
fn the_page_and_her_text_keep_hidden_rows() {
    let live = mayhem_hidden_json();
    let plugin = LolPlugin::new();
    {
        let mut s = plugin.state();
        let mut m = Match {
            game_id: 77,
            queue_id: 2400,
            mode: "KIWI".into(),
            started: true,
            ..Match::default()
        };
        merge_session(&mut m, members(&session(&live)));
        // 查完了 p0..p2（我方看得到的三个），对面还在查
        for i in 0..3 {
            m.histories.insert(
                format!("p{i}"),
                PlayerHistory {
                    riot_id: format!("编的名字{i}#1"),
                    privacy: "PUBLIC".into(),
                    ..PlayerHistory::default()
                },
            );
            m.pending.remove(&format!("p{i}"));
        }
        s.current = Some(m);
    }
    let data = data();
    let view = plugin.match_view(Some(&data)).unwrap();
    let page = pages::match_page_in(
        &view.players,
        &view.puuids,
        view.game_id,
        view.queue_id,
        &view.mode,
        view.random,
        Some(&data),
        &view.stage,
    );
    let allies = page["allies"].as_array().unwrap();
    assert_eq!(allies.len(), 5, "我方 5 行");
    let hidden: Vec<&Value> = allies
        .iter()
        .filter(|p| p["identity"] == "hidden")
        .collect();
    assert_eq!(hidden.len(), 2);
    for p in &hidden {
        assert_eq!(p["pending"], false, "隐藏了身份的不算还在查");
        assert_eq!(p["puuid"], "");
        assert!(p["champion"]["short"].as_str().unwrap() != "");
    }
    assert_eq!(page["waiting"], 5, "只数对面五个还在查的");
    assert!(page["allies"][0].get("weak").is_none(), "没有突破口了");

    let words = text::render_scout(&view.players, view.queue_id, &view.mode, view.random, None);
    assert!(
        words.contains("崔斯特 · 隐藏了身份（查不到战绩）"),
        "{words}"
    );
    assert!(words.contains("我方 2 人"), "{words}");
    assert!(!words.contains("突破口"), "{words}");
}
