//! 引擎的判定：按时间线喂合成的 POST（照 Valve GSI 文档的字段），看阶段、事件、推送。

use super::*;
use crate::gsi::parse;
use serde_json::json;

const ME: &str = "76561190000000001";
const MATE: &str = "76561190000000002";

/// 一份合成的 POST。
#[derive(Clone)]
struct P {
    map: Option<&'static str>,
    mode: &'static str,
    phase: &'static str,
    round: &'static str,
    bomb: &'static str,
    win: &'static str,
    ct: i64,
    t: i64,
    team: &'static str,
    player: &'static str,
    health: i64,
    stats: (i64, i64, i64, i64),
    wins: Vec<(u32, &'static str)>,
    countdown: Option<(&'static str, f64)>,
}

impl Default for P {
    fn default() -> Self {
        Self {
            map: Some("de_mirage"),
            mode: "competitive",
            phase: "live",
            round: "freezetime",
            bomb: "",
            win: "",
            ct: 0,
            t: 0,
            team: "T",
            player: ME,
            health: 100,
            stats: (0, 0, 0, 0),
            wins: Vec::new(),
            countdown: None,
        }
    }
}

impl P {
    fn json(&self) -> Value {
        let mut v = json!({
            "provider": {"name": "Counter-Strike 2", "appid": 730, "steamid": ME, "timestamp": 1},
            "player": {
                "steamid": self.player, "team": self.team, "activity": "playing",
                "state": {"health": self.health, "armor": 100, "helmet": true, "money": 4000, "round_kills": 0, "round_killhs": 0, "equip_value": 4000},
                "match_stats": {"kills": self.stats.0, "deaths": self.stats.1, "assists": self.stats.2, "mvps": self.stats.3, "score": 10},
            },
            "auth": {"token": "x"},
        });
        if let Some(name) = self.map {
            let wins: Map<String, Value> = self
                .wins
                .iter()
                .map(|(n, r)| (n.to_string(), json!(r)))
                .collect();
            v["map"] = json!({"name": name, "mode": self.mode, "phase": self.phase,
                "team_ct": {"score": self.ct}, "team_t": {"score": self.t}, "round_wins": wins});
            let mut round = json!({"phase": self.round});
            if !self.bomb.is_empty() {
                round["bomb"] = json!(self.bomb);
            }
            if !self.win.is_empty() {
                round["win_team"] = json!(self.win);
            }
            v["round"] = round;
        }
        if let Some((phase, left)) = self.countdown {
            v["phase_countdowns"] = json!({"phase": phase, "phase_ends_in": format!("{left}")});
        }
        v
    }
}

fn feed(e: &mut Engine, p: &P, at: u64) -> Vec<Action> {
    e.feed(parse(&p.json()), at)
}

fn events(actions: &[Action]) -> Vec<(&'static str, Map<String, Value>)> {
    actions
        .iter()
        .filter_map(|a| match a {
            Action::Event { kind, data } => Some((*kind, data.clone())),
            _ => None,
        })
        .collect()
}

fn kinds(actions: &[Action]) -> Vec<&'static str> {
    events(actions).into_iter().map(|(k, _)| k).collect()
}

fn pushes(actions: &[Action]) -> Vec<Push> {
    actions
        .iter()
        .filter_map(|a| match a {
            Action::Push(p) => Some(p.clone()),
            _ => None,
        })
        .collect()
}

fn phase(actions: &[Action]) -> Option<Phase> {
    actions.iter().rev().find_map(|a| match a {
        Action::Observe(o) => Some(o.phase),
        _ => None,
    })
}

#[test]
fn phases_follow_the_map_phase_and_the_first_post_claims_cs2() {
    let mut e = Engine::default();
    let menu = P {
        map: None,
        ..P::default()
    };
    let a = feed(&mut e, &menu, 1_000);
    assert_eq!(a[0], Action::Claim(true));
    assert_eq!(phase(&a), Some(Phase::Menu));
    assert!(e.live());
    let a = feed(
        &mut e,
        &P {
            phase: "warmup",
            round: "warmup",
            ..P::default()
        },
        2_000,
    );
    assert!(!a.contains(&Action::Claim(true)), "只认领一次");
    assert_eq!(phase(&a), Some(Phase::PreMatch));
    let a = feed(&mut e, &P::default(), 3_000);
    assert_eq!(phase(&a), Some(Phase::InMatch));
    let a = feed(
        &mut e,
        &P {
            phase: "intermission",
            ..P::default()
        },
        4_000,
    );
    assert_eq!(phase(&a), Some(Phase::InMatch), "半场换边还在对局中");
    let a = feed(
        &mut e,
        &P {
            phase: "gameover",
            ..P::default()
        },
        5_000,
    );
    assert_eq!(phase(&a), Some(Phase::PostMatch));
    // attrs 以适配器 id 开头
    let observe = a
        .iter()
        .find_map(|x| match x {
            Action::Observe(o) => Some(o.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(observe.attrs["cs2.map"], "de_mirage");
    assert!(observe.attrs.keys().all(|k| k.starts_with("cs2.")));
    assert_eq!(observe.source, SOURCE);
}

#[test]
fn the_first_post_only_records_what_is_there() {
    // 连上时包已经下了、回合已经结束：不补发
    let mut e = Engine::default();
    let a = feed(
        &mut e,
        &P {
            round: "over",
            bomb: "planted",
            win: "T",
            ct: 3,
            t: 4,
            ..P::default()
        },
        1_000,
    );
    assert!(kinds(&a).is_empty(), "{:?}", kinds(&a));
    assert!(pushes(&a).is_empty());
}

#[test]
fn plant_countdown_ten_seconds_then_explode() {
    let mut e = Engine::default();
    feed(&mut e, &P::default(), 0);
    let a = feed(
        &mut e,
        &P {
            round: "live",
            ..P::default()
        },
        15_000,
    );
    assert_eq!(kinds(&a), [EV_ROUND_STARTED]);
    assert_eq!(events(&a)[0].1["round"], 1);
    let planted = P {
        round: "live",
        bomb: "planted",
        ..P::default()
    };
    let a = feed(&mut e, &planted, 60_000);
    assert_eq!(kinds(&a), [EV_BOMB_PLANTED]);
    assert_eq!(events(&a)[0].1["seconds"], 40);
    let p = pushes(&a);
    assert_eq!(p.len(), 1);
    assert_eq!(p[0].trigger, PUSH_BOMB);
    let card = p[0].card.as_ref().unwrap();
    assert_eq!(card.uri, PAGE_BOMB);
    assert_eq!(card.data["state"], "planted");
    assert_eq!(card.data["explodesAtMs"], 100_000);
    assert!(p[0].note.is_empty(), "下包不给她留话");
    let key = p[0].key.clone();

    // 同一份再来：什么都没有
    assert!(kinds(&feed(&mut e, &planted, 61_000)).is_empty());
    // 还剩 10 秒之前不说；到了说一次
    assert!(pushes(&e.tick(89_000, true)).is_empty());
    let t = pushes(&e.tick(90_000, true));
    assert_eq!(t.len(), 1);
    assert_eq!(t[0].trigger, PUSH_BOMB_TEN);
    assert_eq!(t[0].line.as_ref().unwrap().cells[0].text, "炸弹还剩 10 秒");
    assert!(!t[0].voice.is_empty());
    assert!(pushes(&e.tick(91_000, true)).is_empty(), "只说一次");

    // 炸了：事件 + 同一张卡片原地更新
    let a = feed(
        &mut e,
        &P {
            round: "live",
            bomb: "exploded",
            ..P::default()
        },
        100_100,
    );
    assert_eq!(kinds(&a), [EV_BOMB_EXPLODED]);
    let p = pushes(&a);
    assert_eq!(p[0].key, key);
    assert_eq!(p[0].card.as_ref().unwrap().data["state"], "exploded");
    // 回合结束：T 赢、怎么赢的是炸了
    let a = feed(
        &mut e,
        &P {
            round: "over",
            bomb: "exploded",
            win: "T",
            t: 1,
            ..P::default()
        },
        101_000,
    );
    let ev = events(&a);
    assert_eq!(ev[0].0, EV_ROUND_ENDED);
    assert_eq!(ev[0].1["winner"], "T");
    assert_eq!(ev[0].1["won"], true);
    assert_eq!(ev[0].1["reason"], "bomb");
    assert_eq!(
        (ev[0].1["myScore"].clone(), ev[0].1["enemyScore"].clone()),
        (json!(1), json!(0))
    );
    assert!(pushes(&a).is_empty(), "卡片已经写了炸了，不再改");
}

#[test]
fn defused_updates_the_card_and_no_ten_seconds_after() {
    let mut e = Engine::default();
    feed(&mut e, &P::default(), 0);
    feed(
        &mut e,
        &P {
            round: "live",
            ..P::default()
        },
        1_000,
    );
    let planted = feed(
        &mut e,
        &P {
            round: "live",
            bomb: "planted",
            ..P::default()
        },
        10_000,
    );
    let key = pushes(&planted)[0].key.clone();
    let a = feed(
        &mut e,
        &P {
            round: "over",
            bomb: "defused",
            win: "CT",
            ct: 1,
            wins: vec![(1, "ct_win_defuse")],
            ..P::default()
        },
        30_000,
    );
    assert_eq!(kinds(&a), [EV_BOMB_DEFUSED, EV_ROUND_ENDED]);
    let p = pushes(&a);
    assert_eq!(p.len(), 1);
    assert_eq!(p[0].key, key);
    assert_eq!(p[0].card.as_ref().unwrap().data["state"], "defused");
    let ended = &events(&a)[1].1;
    assert_eq!(ended["won"], false);
    assert_eq!(ended["reason"], "defuse");
    assert!(
        pushes(&e.tick(41_000, true)).is_empty(),
        "拆了就不说还剩十秒"
    );
}

#[test]
fn round_over_with_the_bomb_still_ticking_closes_the_card() {
    let mut e = Engine::default();
    feed(&mut e, &P::default(), 0);
    feed(
        &mut e,
        &P {
            round: "live",
            ..P::default()
        },
        1_000,
    );
    feed(
        &mut e,
        &P {
            round: "live",
            bomb: "planted",
            ..P::default()
        },
        10_000,
    );
    let a = feed(
        &mut e,
        &P {
            round: "over",
            bomb: "planted",
            win: "T",
            t: 1,
            ..P::default()
        },
        20_000,
    );
    let p = pushes(&a);
    assert_eq!(p[0].card.as_ref().unwrap().data["state"], "over");
    assert!(pushes(&e.tick(41_000, true)).is_empty());
}

#[test]
fn spectating_uses_the_games_countdown() {
    let mut e = Engine::default();
    feed(
        &mut e,
        &P {
            player: MATE,
            ..P::default()
        },
        0,
    );
    feed(
        &mut e,
        &P {
            player: MATE,
            round: "live",
            ..P::default()
        },
        1_000,
    );
    let a = feed(
        &mut e,
        &P {
            player: MATE,
            round: "live",
            bomb: "planted",
            countdown: Some(("bomb", 38.0)),
            ..P::default()
        },
        10_000,
    );
    assert_eq!(events(&a)[0].1["seconds"], 38);
    assert_eq!(
        pushes(&a)[0].card.as_ref().unwrap().data["explodesAtMs"],
        48_000
    );
}

/// 一张竞技图：上半场 7 : 5，半场换边，下半场 12 : 5 我们赛点，对面追到 11 : 12，打完 13 : 11。
#[test]
fn halftime_match_point_and_match_end() {
    let mut e = Engine::default();
    let base = P {
        team: "CT",
        ..P::default()
    };
    feed(
        &mut e,
        &P {
            phase: "warmup",
            round: "warmup",
            ..base.clone()
        },
        0,
    );
    let mut at = 1_000;
    let mut ct = 0;
    let mut t = 0;
    let mut all = Vec::new();
    // 打一回合：冻结 → 开打 → 结束（winner 赢）
    let mut play = |e: &mut Engine,
                    winner: &'static str,
                    ct: &mut i64,
                    t: &mut i64,
                    team: &'static str,
                    phase: &'static str| {
        let mut out = Vec::new();
        let p = P {
            ct: *ct,
            t: *t,
            team,
            phase: "live",
            ..base.clone()
        };
        out.extend(feed(e, &p, at));
        at += 15_000;
        out.extend(feed(
            e,
            &P {
                round: "live",
                ..p.clone()
            },
            at,
        ));
        at += 60_000;
        if winner == "CT" {
            *ct += 1;
        } else {
            *t += 1;
        }
        out.extend(feed(
            e,
            &P {
                round: "over",
                win: winner,
                ct: *ct,
                t: *t,
                phase,
                ..p
            },
            at,
        ));
        at += 7_000;
        out
    };
    // 上半场：CT 7 : 5（我在 CT）
    for i in 0..12 {
        let w = if i < 7 { "CT" } else { "T" };
        all.extend(play(&mut e, w, &mut ct, &mut t, "CT", "live"));
    }
    assert_eq!((ct, t), (7, 5));
    // 半场：换边，我到 T，比分跟着队走（我这边是 T 的 7）
    let a = feed(
        &mut e,
        &P {
            phase: "intermission",
            round: "over",
            win: "T",
            ct: 7,
            t: 5,
            team: "CT",
            ..base.clone()
        },
        900_000,
    );
    let ev = events(&a);
    assert_eq!(ev.len(), 1);
    assert_eq!(ev[0].0, EV_HALFTIME);
    assert_eq!(ev[0].1["myScore"], 7);
    let p = pushes(&a);
    assert_eq!(p[0].trigger, PUSH_HALFTIME);
    assert_eq!(
        p[0].line.as_ref().unwrap().cells[0].text,
        "半场 · 我们 7 : 5 对面"
    );
    assert_eq!(p[0].voice[0], "半场了，7 比 5");
    // 换边以后：我在 T，T 是 7
    let (mut ct2, mut t2) = (5, 7);
    for i in 0..10 {
        let w = if i < 5 { "T" } else { "CT" };
        all.extend(play(&mut e, w, &mut ct2, &mut t2, "T", "live"));
    }
    assert_eq!((ct2, t2), (10, 12));
    let mp: Vec<_> = events(&all)
        .into_iter()
        .filter(|(k, _)| *k == EV_MATCH_POINT)
        .collect();
    assert_eq!(
        mp.len(),
        1,
        "12 : 5 我们到了赛点，之后连着几回合都是我们的赛点，只说一次"
    );
    assert_eq!(mp[0].1["team"], "T");
    assert_eq!(mp[0].1["ours"], true);
    let mpp: Vec<_> = pushes(&all)
        .into_iter()
        .filter(|p| p.trigger == PUSH_MATCH_POINT)
        .collect();
    assert_eq!(mpp[0].voice[0], "我们赛点了");
    assert_eq!(
        mpp[0].line.as_ref().unwrap().cells[0].tone,
        Some(PushTone::Good)
    );
    assert_eq!(mp[0].1["tScore"], 12);
    assert_eq!(mp[0].1["ctScore"], 5);
    // 对面追到 11 : 12：还是我们的赛点，不再说
    let a = play(&mut e, "CT", &mut ct2, &mut t2, "T", "live");
    assert!(!kinds(&a).contains(&EV_MATCH_POINT));
    // 最后一回合我们赢：13 : 11，结束
    let a = play(&mut e, "T", &mut ct2, &mut t2, "T", "gameover");
    let ev = events(&a);
    let ended = ev.iter().find(|(k, _)| *k == EV_MATCH_ENDED).unwrap();
    assert_eq!(ended.1["result"], "win");
    assert_eq!(ended.1["myScore"], 13);
    assert_eq!(ended.1["enemyScore"], 11);
    assert_eq!(ended.1["map"], "de_mirage");
    assert!(!kinds(&a).contains(&EV_MATCH_POINT), "结束那回合不说赛点");
    let card = pushes(&a)
        .into_iter()
        .find(|p| p.trigger == PUSH_MATCH_END)
        .unwrap();
    assert_eq!(card.card.as_ref().unwrap().uri, PAGE_MATCH);
    let data = &card.card.as_ref().unwrap().data;
    assert_eq!(data["over"], true);
    assert_eq!(data["rounds"].as_array().unwrap().len(), 24);
    assert!(
        card.note
            .starts_with("CS2 这张图打完了：荒漠迷城（竞技）我们 13 : 11 对面，赢了"),
        "{}",
        card.note
    );
    assert!(card.note.chars().count() <= Push::NOTE_MAX);
    // 结束以后再来一份 gameover：不重复
    let again = feed(
        &mut e,
        &P {
            phase: "gameover",
            round: "over",
            win: "T",
            ct: ct2,
            t: t2,
            team: "T",
            ..base.clone()
        },
        at + 1_000,
    );
    assert!(kinds(&again).is_empty());
    // 页面：没在打以后还能看到上一张
    let view = e.match_view(at + 2_000);
    assert_eq!(view["over"], true);
}

#[test]
fn the_same_input_gives_the_same_output() {
    let run = || {
        let mut e = Engine::default();
        let mut out = Vec::new();
        out.extend(feed(&mut e, &P::default(), 0));
        out.extend(feed(
            &mut e,
            &P {
                round: "live",
                ..P::default()
            },
            1_000,
        ));
        out.extend(feed(
            &mut e,
            &P {
                round: "live",
                bomb: "planted",
                ..P::default()
            },
            2_000,
        ));
        out.extend(e.tick(32_000, true));
        out
    };
    assert_eq!(run(), run());
}

#[test]
fn my_death_is_reported_once_per_round() {
    let mut e = Engine::default();
    feed(&mut e, &P::default(), 0);
    feed(
        &mut e,
        &P {
            round: "live",
            ..P::default()
        },
        1_000,
    );
    // 死了马上换成在看队友（队友的数据不一样）
    let a = feed(
        &mut e,
        &P {
            round: "live",
            player: MATE,
            stats: (9, 1, 0, 2),
            ..P::default()
        },
        5_000,
    );
    assert_eq!(kinds(&a), [EV_MY_DEATH]);
    assert_eq!(events(&a)[0].1["round"], 1);
    // 队友的数据不算你的
    let view = e.match_view(6_000);
    assert_eq!(view["stats"]["kills"], 0);
    assert!(kinds(&feed(
        &mut e,
        &P {
            round: "live",
            player: MATE,
            ..P::default()
        },
        6_000
    ))
    .is_empty());
}

#[test]
fn no_match_point_in_modes_without_known_rules() {
    let mut e = Engine::default();
    let casual = P {
        mode: "casual",
        ct: 12,
        t: 10,
        ..P::default()
    };
    feed(&mut e, &casual, 0);
    feed(
        &mut e,
        &P {
            round: "live",
            ..casual.clone()
        },
        1_000,
    );
    let a = feed(
        &mut e,
        &P {
            round: "over",
            win: "CT",
            ct: 13,
            ..casual
        },
        2_000,
    );
    assert!(!kinds(&a).contains(&EV_MATCH_POINT));
}

#[test]
fn stale_data_hands_the_game_back() {
    let mut e = Engine::default();
    feed(&mut e, &P::default(), 0);
    assert!(e.tick(STALE_MS, false).is_empty());
    let a = e.tick(STALE_MS + 1, false);
    assert_eq!(
        a,
        vec![
            Action::Observe(Observation {
                phase: Phase::Stopped,
                source: SOURCE.into(),
                ..Observation::default()
            }),
            Action::Claim(false)
        ]
    );
    assert!(!e.live());
    // 游戏还在跑：交回成“在跑”
    feed(&mut e, &P::default(), 100_000);
    assert_eq!(phase(&e.release(true)), Some(Phase::Running));
    assert!(e.release(true).is_empty(), "交回过就不再交");
}

#[test]
fn a_new_map_starts_a_new_match() {
    let mut e = Engine::default();
    feed(&mut e, &P::default(), 0);
    feed(
        &mut e,
        &P {
            round: "live",
            ..P::default()
        },
        1_000,
    );
    let first = pushes(&feed(
        &mut e,
        &P {
            round: "live",
            bomb: "planted",
            ..P::default()
        },
        2_000,
    ))[0]
        .key
        .clone();
    feed(
        &mut e,
        &P {
            map: Some("de_inferno"),
            ..P::default()
        },
        3_000,
    );
    feed(
        &mut e,
        &P {
            map: Some("de_inferno"),
            round: "live",
            ..P::default()
        },
        4_000,
    );
    let second = pushes(&feed(
        &mut e,
        &P {
            map: Some("de_inferno"),
            round: "live",
            bomb: "planted",
            ..P::default()
        },
        5_000,
    ))[0]
        .key
        .clone();
    assert_ne!(first, second, "换了一张图，同样是第 1 回合，卡片也不一样");
}

/// 插件报的每种事都在清单里登记了，登记的每种都会报（照英雄联盟的 event_catalog_tests）。
#[test]
fn every_event_is_registered_and_every_registered_event_happens() {
    let registered: HashSet<String> = crate::manifest()
        .events
        .iter()
        .map(|e| e.kind.clone())
        .collect();
    let all = [
        EV_ROUND_STARTED,
        EV_ROUND_ENDED,
        EV_BOMB_PLANTED,
        EV_BOMB_DEFUSED,
        EV_BOMB_EXPLODED,
        EV_HALFTIME,
        EV_MATCH_POINT,
        EV_MATCH_ENDED,
        EV_MY_DEATH,
    ];
    let ours: HashSet<String> = all.iter().map(|s| s.to_string()).collect();
    assert_eq!(registered, ours);
    // 每种事的字段都登记过
    for spec in &crate::manifest().events {
        assert!(spec.kind.starts_with("cs2."));
        assert!(!spec.data.is_empty() || spec.kind.is_empty());
    }
}

/// 每条推送都在清单里，样子是清单声明过的。
#[test]
fn pushes_use_only_declared_forms() {
    let m = crate::manifest();
    let mut e = Engine::default();
    feed(&mut e, &P::default(), 0);
    feed(
        &mut e,
        &P {
            round: "live",
            ..P::default()
        },
        1_000,
    );
    let mut all = feed(
        &mut e,
        &P {
            round: "live",
            bomb: "planted",
            ..P::default()
        },
        2_000,
    );
    all.extend(e.tick(32_000, true));
    all.extend(feed(
        &mut e,
        &P {
            phase: "intermission",
            ..P::default()
        },
        50_000,
    ));
    for push in pushes(&all) {
        let spec = m.push_spec(&push.trigger).expect("清单里有");
        let shaped = push.clone().shaped(spec);
        assert_eq!(shaped, push, "{} 用了没声明的样子或者超长", push.trigger);
        shaped.check(m).unwrap();
    }
}
