use serde_json::json;

use super::*;

fn seen(phase: Phase, native: &str) -> Observation {
    Observation {
        phase,
        native_phase: native.into(),
        attrs: Map::new(),
        source: String::new(),
    }
}

fn kinds(events: &[GameEvent]) -> Vec<&str> {
    events.iter().map(|e| e.kind.as_str()).collect()
}

#[test]
fn a_game_without_adapter_is_one_match_from_start_to_exit() {
    let mut hub = Hub::default();
    let started = hub.observe("genshin", seen(Phase::Running, ""), 1_000);
    assert_eq!(
        kinds(&started),
        ["game.started", "game.phase_changed", "game.match_started"]
    );
    assert_eq!(started[1].data["from"], "stopped");
    assert_eq!(started[1].data["to"], "running");
    let state = hub.state("genshin").unwrap().clone();
    assert_eq!(state.match_id, "genshin-1000");
    assert_eq!(started[2].match_id, "genshin-1000");
    // 没变：不发事件
    assert!(hub
        .observe("genshin", seen(Phase::Running, ""), 3_000)
        .is_empty());

    let stopped = hub.observe("genshin", seen(Phase::Stopped, ""), 201_000);
    assert_eq!(
        kinds(&stopped),
        ["game.match_ended", "game.phase_changed", "game.stopped"]
    );
    assert_eq!(stopped[0].data["elapsedSeconds"], 200);
    assert_eq!(stopped[0].match_id, "genshin-1000", "结束的事还带着这一局");
    assert_eq!(stopped[2].data["elapsedSeconds"], 200);
    assert!(hub.state("genshin").is_none());
    assert_eq!(hub.recent("genshin", 100).len(), 6);
}

#[test]
fn lol_phases_make_matches_between_lobby_and_end_of_game() {
    let mut hub = Hub::default();
    let mut all = Vec::new();
    let steps = [
        (Phase::Menu, "Lobby"),
        (Phase::Queue, "Matchmaking"),
        (Phase::PreMatch, "ChampSelect"),
        (Phase::Loading, "InProgress"),
        (Phase::InMatch, "InProgress"),
        (Phase::PostMatch, "EndOfGame"),
        (Phase::Menu, "Lobby"),
        (Phase::Stopped, ""),
    ];
    for (i, (phase, native)) in steps.into_iter().enumerate() {
        all.extend(hub.observe("lol", seen(phase, native), 1_000 * (i as u64 + 1)));
    }
    let names: Vec<&str> = kinds(&all)
        .into_iter()
        .filter(|k| *k != "game.phase_changed")
        .collect();
    assert_eq!(
        names,
        [
            "game.started",
            "game.match_started",
            "game.match_ended",
            "game.stopped"
        ]
    );
    let started = all.iter().find(|e| e.kind == "game.match_started").unwrap();
    let ended = all.iter().find(|e| e.kind == "game.match_ended").unwrap();
    assert_eq!(started.match_id, ended.match_id);
    assert_eq!(ended.data["elapsedSeconds"], 3);
    let to_loading = all
        .iter()
        .find(|e| e.kind == "game.phase_changed" && e.data["to"] == "loading")
        .unwrap();
    assert_eq!(to_loading.data["native"], "InProgress");
    assert_eq!(to_loading.match_id, started.match_id);
}

#[test]
fn focus_and_game_specific_events_only_for_running_games() {
    let mut hub = Hub::default();
    assert!(hub.focus(Some("lol"), 1).is_empty(), "没在跑的游戏不算");
    assert!(hub
        .push("lol", "lol.ace", "lol/live-client", Map::new(), None, 1)
        .is_none());
    hub.observe("lol", seen(Phase::InMatch, "InProgress"), 2);
    hub.observe("cs2", seen(Phase::Running, ""), 2);
    assert_eq!(kinds(&hub.focus(Some("lol"), 3)), ["game.focused"]);
    assert!(hub.focus(Some("lol"), 4).is_empty());
    let switched = hub.focus(Some("cs2"), 5);
    assert_eq!(kinds(&switched), ["game.focused", "game.unfocused"]);
    assert_eq!(switched[0].game, "cs2");
    let ace = hub
        .push(
            "lol",
            "lol.ace",
            "lol/live-client",
            json!({"team": "ally"}).as_object().unwrap().clone(),
            Some(612.5),
            6,
        )
        .unwrap();
    assert_eq!(ace.match_id, hub.state("lol").unwrap().match_id);
    assert_eq!(ace.game_time, Some(612.5));
    let envelope = serde_json::to_value(&ace).unwrap();
    assert_eq!(envelope["type"], "lol.ace");
    assert_eq!(envelope["gameTime"], 612.5);
    assert!(envelope["match"].as_str().unwrap().starts_with("lol-"));
}

#[test]
fn recent_events_are_capped() {
    let mut hub = Hub::default();
    hub.observe("lol", seen(Phase::InMatch, ""), 0);
    for i in 0..(RECENT_EVENTS as u64 + 20) {
        hub.push("lol", "lol.turret_killed", "x", Map::new(), None, i);
    }
    assert_eq!(hub.recent("lol", 1000).len(), RECENT_EVENTS);
    assert_eq!(hub.recent("lol", 3).len(), 3);
}

fn trigger(value: serde_json::Value) -> EventTrigger {
    serde_json::from_value(value).unwrap()
}

#[test]
fn triggers_are_checked_before_saving() {
    let ok = validate_trigger(&trigger(json!({
        "event": [" game.phase_changed "], "game": "lol", "match": {"to": ["loading", "in_match"]}, "for": 60
    })))
    .unwrap();
    assert_eq!(ok.event, ["game.phase_changed"]);
    assert_eq!(ok.mode, RunMode::Single);
    assert!(ok.enabled);
    assert!(validate_trigger(&trigger(json!({"event": ["lol.*"]}))).is_ok());
    assert!(validate_trigger(&trigger(json!({"event": ["game.*", "lol.dragon_kill"]}))).is_ok());
    for (bad, want) in [
        (json!({"event": []}), "至少"),
        (json!({"event": ["lol.pentakill"]}), "没有"),
        (json!({"event": ["foo.*"]}), "没有"),
        (json!({"event": ["lo.*"]}), "没有"),
        (
            json!({"event": ["lol.ace"], "for": 10}),
            "game.phase_changed",
        ),
        (
            json!({"event": ["lol.ace"], "match": {"team": {"a": 1}}}),
            "match.team",
        ),
        (
            json!({"event": ["game.phase_changed"], "for": 99999}),
            "for",
        ),
    ] {
        let error = validate_trigger(&trigger(bad.clone())).unwrap_err();
        assert!(error.contains(want), "{bad} → {error}");
    }
    // 不认识的字段直接拒（打错字不会悄悄失效）
    assert!(
        serde_json::from_value::<EventTrigger>(json!({"event": ["lol.ace"], "when": 1})).is_err()
    );
}

fn event(kind: &str, game: &str, data: serde_json::Value) -> GameEvent {
    GameEvent {
        id: "e".into(),
        kind: kind.into(),
        game: game.into(),
        match_id: format!("{game}-1"),
        time: 0,
        game_time: None,
        source: "test".into(),
        data: data.as_object().cloned().unwrap_or_default(),
    }
}

#[test]
fn matching_is_a_subset_with_arrays_as_or() {
    let t = trigger(json!({"event": ["lol.dragon_kill"], "game": "lol",
        "match": {"team": "enemy", "dragon": ["fire", "elder"]}}));
    let fire = event(
        "lol.dragon_kill",
        "lol",
        json!({"team": "enemy", "dragon": "fire", "stolen": false}),
    );
    assert!(trigger_matches(&t, &fire), "多出来的字段不影响");
    assert!(!trigger_matches(
        &t,
        &event(
            "lol.dragon_kill",
            "lol",
            json!({"team": "ally", "dragon": "fire"})
        )
    ));
    assert!(!trigger_matches(
        &t,
        &event("lol.dragon_kill", "lol", json!({"team": "enemy"}))
    ));
    assert!(!trigger_matches(
        &t,
        &event(
            "lol.dragon_kill",
            "cs2",
            json!({"team": "enemy", "dragon": "fire"})
        )
    ));
    let any = trigger(json!({"event": ["lol.*"], "match": {"count": 5}}));
    assert!(trigger_matches(
        &any,
        &event("lol.multikill", "lol", json!({"count": 5.0}))
    ));
    assert!(!trigger_matches(
        &any,
        &event("lolx.multikill", "lol", json!({"count": 5}))
    ));
    let mut off = any.clone();
    off.enabled = false;
    assert!(!trigger_matches(
        &off,
        &event("lol.multikill", "lol", json!({"count": 5}))
    ));
}

#[test]
fn once_per_match_cap_and_cooldown() {
    let once = trigger(json!({"event": ["lol.dragon_kill"], "oncePerMatch": true}));
    let capped = trigger(json!({"event": ["lol.dragon_kill"]}));
    let cooled = trigger(json!({"event": ["lol.dragon_kill"], "cooldownSeconds": 60}));
    let armed = [
        Armed {
            id: "once",
            trigger: &once,
            per_match: None,
        },
        Armed {
            id: "capped",
            trigger: &capped,
            per_match: Some(3),
        },
        Armed {
            id: "cooled",
            trigger: &cooled,
            per_match: None,
        },
    ];
    let dragon = event("lol.dragon_kill", "lol", json!({}));
    let mut t = Triggers::default();
    let mut fired: HashMap<String, u32> = HashMap::new();
    for i in 0..5u64 {
        for f in t.on_event(&armed, &dragon, i * 10_000) {
            *fired.entry(f.automation).or_default() += 1;
        }
    }
    assert_eq!(fired["once"], 1);
    assert_eq!(fired["capped"], 3);
    // 0 秒、40 秒都在 60 秒冷却里只算第一次；（0、10、20、30、40 秒）
    assert_eq!(fired["cooled"], 1);

    // 一局结束：次数清零
    t.on_event(
        &armed,
        &event("game.match_ended", "lol", json!({})),
        100_000,
    );
    let again: Vec<String> = t
        .on_event(&armed, &dragon, 200_000)
        .into_iter()
        .map(|f| f.automation)
        .collect();
    assert_eq!(again, ["once", "capped", "cooled"]);
}

#[test]
fn for_waits_until_the_phase_has_held() {
    let t60 = trigger(json!({"event": ["game.phase_changed"], "game": "lol",
        "match": {"to": "in_match"}, "for": 60}));
    let armed = [Armed {
        id: "ward",
        trigger: &t60,
        per_match: None,
    }];
    let to_match = event(
        "game.phase_changed",
        "lol",
        json!({"from": "loading", "to": "in_match"}),
    );
    let mut t = Triggers::default();
    assert!(t.on_event(&armed, &to_match, 0).is_empty());
    assert!(t.tick(&armed, 59_000).is_empty());
    let fired = t.tick(&armed, 60_000);
    assert_eq!(fired.len(), 1);
    assert_eq!(fired[0].event.data["to"], "in_match");
    assert!(t.tick(&armed, 120_000).is_empty(), "只跑一次");

    // 等的时候阶段变了：作废
    assert!(t.on_event(&armed, &to_match, 200_000).is_empty());
    let left = event(
        "game.phase_changed",
        "lol",
        json!({"from": "in_match", "to": "post_match"}),
    );
    t.on_event(&armed, &left, 230_000);
    assert!(t.tick(&armed, 300_000).is_empty());

    // 等的时候自动化被关了：不跑
    assert!(t.on_event(&armed, &to_match, 400_000).is_empty());
    let mut off = t60.clone();
    off.enabled = false;
    let disabled = [Armed {
        id: "ward",
        trigger: &off,
        per_match: None,
    }];
    assert!(t.tick(&disabled, 460_000).is_empty());
}

#[test]
fn catalog_names_follow_the_naming_rule() {
    for (adapter, specs) in catalog() {
        for s in specs {
            let (scope, name) = s.kind.split_once('.').unwrap();
            assert!(
                scope == "game" || scope == adapter,
                "{} 的前缀要是 game 或 {adapter}",
                s.kind
            );
            assert!(
                name.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
                "{}",
                s.kind
            );
            for (field, _) in s.fields {
                assert!(
                    field.chars().next().unwrap().is_ascii_lowercase(),
                    "{field}"
                );
                assert!(!field.contains('_'), "{field} 用 camelCase");
            }
        }
    }
    for p in Phase::ALL {
        let text = serde_json::to_value(p).unwrap();
        assert_eq!(text, p.as_str());
    }
}
