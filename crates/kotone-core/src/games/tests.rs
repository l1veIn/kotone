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
    assert!(hub.focus(Some("demo"), 1).is_empty(), "没在跑的游戏不算");
    assert!(hub
        .push("demo", "demo.ace", "demo/live", Map::new(), None, 1)
        .is_none());
    hub.observe("demo", seen(Phase::InMatch, "InProgress"), 2);
    hub.observe("cs2", seen(Phase::Running, ""), 2);
    assert_eq!(kinds(&hub.focus(Some("demo"), 3)), ["game.focused"]);
    assert!(hub.focus(Some("demo"), 4).is_empty());
    let switched = hub.focus(Some("cs2"), 5);
    assert_eq!(kinds(&switched), ["game.focused", "game.unfocused"]);
    assert_eq!(switched[0].game, "cs2");
    let ace = hub
        .push(
            "demo",
            "demo.ace",
            "demo/live",
            json!({"team": "ally"}).as_object().unwrap().clone(),
            Some(612.5),
            6,
        )
        .unwrap();
    assert_eq!(ace.match_id, hub.state("demo").unwrap().match_id);
    assert_eq!(ace.game_time, Some(612.5));
    let envelope = serde_json::to_value(&ace).unwrap();
    assert_eq!(envelope["type"], "demo.ace");
    assert_eq!(envelope["gameTime"], 612.5);
    assert!(envelope["match"].as_str().unwrap().starts_with("demo-"));
}

/// ADR 047 D5：第三方插件的事只交给自动化：不进最近几件事（她看的 `game.state`）；游戏没在跑也收、触发照样匹配。
#[test]
fn third_party_events_skip_the_recent_list() {
    let mut hub = Hub::default();
    let mut data = Map::new();
    data.insert("to".into(), json!("success"));
    let alone = hub.outside_event(
        "",
        "zzz-od.run_finished",
        "third-party/zzz-od",
        data.clone(),
        1_000,
    );
    assert_eq!(alone.game, "");
    assert!(alone.match_id.is_empty());
    hub.observe("zzz", seen(Phase::Running, ""), 2_000);
    let during = hub.outside_event(
        "zzz",
        "zzz-od.run_finished",
        "third-party/zzz-od",
        data,
        3_000,
    );
    assert_eq!(during.match_id, "zzz-2000", "在跑的游戏带上这一局");
    assert_ne!(during.id, alone.id);
    assert!(
        hub.recent("zzz", 50)
            .iter()
            .all(|e| !e.kind.starts_with("zzz-od.")),
        "不进最近几件事"
    );
    assert!(hub.recent("", 50).is_empty());
    let on: EventTrigger =
        serde_json::from_value(json!({"event": ["zzz-od.run_finished"], "game": "zzz"})).unwrap();
    assert!(trigger_matches(&on, &during));
    assert!(
        !trigger_matches(&on, &alone),
        "限定了游戏的触发不认没有游戏的事"
    );
}

#[test]
fn recent_events_are_capped() {
    let mut hub = Hub::default();
    hub.observe("demo", seen(Phase::InMatch, ""), 0);
    for i in 0..(RECENT_EVENTS as u64 + 20) {
        hub.push("demo", "demo.tower_down", "x", Map::new(), None, i);
    }
    assert_eq!(hub.recent("demo", 1000).len(), RECENT_EVENTS);
    assert_eq!(hub.recent("demo", 3).len(), 3);
}

/// 一个假的插件“示例”（适配器 `demo`）登记的事：本体不认识任何具体游戏的事，测试也用不着英雄联盟。
fn demo_spec(kind: &str, label: &str, fields: &[&str]) -> EventSpec {
    EventSpec {
        kind: kind.into(),
        label: label.into(),
        data: fields
            .iter()
            .map(|f| (f.to_string(), format!("{f} 的说明")))
            .collect(),
        needs: vec!["live".into()],
        private: Vec::new(),
    }
}

fn demo_entry(spec: EventSpec, off: Option<&str>) -> CatalogEntry {
    CatalogEntry {
        spec,
        adapter: "demo".into(),
        plugin: Some(("demo".into(), "示例".into())),
        games: vec!["demo".into()],
        off: off.map(str::to_string),
    }
}

/// 本体的 `game.*` 加上“示例”插件登记的事；`off` 是插件关着时的原因。
fn demo_catalog(off: Option<&str>) -> EventCatalog {
    let mut catalog = EventCatalog::generic();
    for spec in [
        demo_spec("demo.boss_down", "打掉了首领", &["team", "boss", "stolen"]),
        demo_spec("demo.ace", "团灭", &["team"]),
        demo_spec("demo.multikill", "三杀以上", &["team", "count"]),
    ] {
        catalog.add(demo_entry(spec, off)).unwrap();
    }
    catalog
}

fn trigger(value: serde_json::Value) -> EventTrigger {
    serde_json::from_value(value).unwrap()
}

#[test]
fn triggers_are_checked_before_saving() {
    let events = demo_catalog(None);
    let ok = validate_trigger(
        &trigger(json!({
            "event": [" game.phase_changed "], "game": "demo", "match": {"to": ["loading", "in_match"]}, "for": 60
        })),
        &events,
    )
    .unwrap();
    assert_eq!(ok.event, ["game.phase_changed"]);
    assert_eq!(ok.mode, RunMode::Single);
    assert!(ok.enabled);
    assert!(validate_trigger(&trigger(json!({"event": ["demo.*"]})), &events).is_ok());
    assert!(validate_trigger(
        &trigger(json!({"event": ["game.*", "demo.boss_down"]})),
        &events
    )
    .is_ok());
    for (bad, want) in [
        (json!({"event": []}), "至少"),
        (json!({"event": ["demo.pentakill"]}), "没有"),
        (json!({"event": ["foo.*"]}), "没有"),
        (json!({"event": ["dem.*"]}), "没有"),
        (
            json!({"event": ["demo.ace"], "for": 10}),
            "game.phase_changed",
        ),
        (
            json!({"event": ["demo.ace"], "match": {"team": {"a": 1}}}),
            "match.team",
        ),
        (
            json!({"event": ["game.phase_changed"], "for": 99999}),
            "for",
        ),
    ] {
        let error = validate_trigger(&trigger(bad.clone()), &events).unwrap_err();
        assert!(error.contains(want), "{bad} → {error}");
    }
    // 不认识的字段直接拒（打错字不会悄悄失效）
    assert!(
        serde_json::from_value::<EventTrigger>(json!({"event": ["demo.ace"], "when": 1})).is_err()
    );
    // 只有本体的目录（插件没装）：插件的事认不得
    let error = validate_trigger(
        &trigger(json!({"event": ["demo.ace"]})),
        &EventCatalog::generic(),
    )
    .unwrap_err();
    assert!(error.contains("没有“demo.ace”"), "{error}");
}

/// 插件关着：它登记的事还认得（保存成功、旧的不报废），保存时说清楚“这个触发不会发生”（原则 21）。
#[test]
fn a_closed_plugins_events_still_validate_with_a_note() {
    let closed = demo_catalog(Some("示例插件关着"));
    let open = demo_catalog(None);
    let only_demo =
        trigger(json!({"event": ["demo.boss_down", "demo.ace"], "match": {"team": "enemy"}}));
    assert!(validate_trigger(&only_demo, &closed).is_ok());
    assert!(open.notes(&only_demo).is_empty());
    assert_eq!(
        closed.notes(&only_demo),
        ["示例插件关着，这个触发不会发生；到「插件」里打开以后照常触发"]
    );
    // 前缀也一样
    let any = trigger(json!({"event": ["demo.*"]}));
    assert_eq!(closed.notes(&any).len(), 1);
    // 和通用的事混着写：只说不会发生的那几种
    let mixed = trigger(json!({"event": ["game.match_ended", "demo.ace"]}));
    assert_eq!(
        closed.notes(&mixed),
        ["示例插件关着，其中“团灭”不会发生；到「插件」里打开以后照常触发"]
    );
    // 触发关着：不用提醒
    let mut paused = only_demo.clone();
    paused.enabled = false;
    assert!(closed.notes(&paused).is_empty());
    // 本体的事从不提醒
    assert!(closed
        .notes(&trigger(json!({"event": ["game.phase_changed"]})))
        .is_empty());
}

/// `game.state` 的事件目录：按适配器分组；插件关着时不列它的事，只说为什么、怎么办。
#[test]
fn the_summary_lists_what_can_happen_now() {
    let open = demo_catalog(None).summary();
    assert_eq!(open[0]["adapter"], "generic");
    assert_eq!(open[0]["games"], "所有有进程名的游戏配置");
    assert!(open[0].get("plugin").is_none());
    assert_eq!(open[1]["adapter"], "demo");
    assert_eq!(open[1]["plugin"], "示例");
    assert_eq!(open[1]["games"], "demo");
    assert_eq!(open[1]["events"].as_array().unwrap().len(), 3);
    assert_eq!(open[1]["events"][0]["type"], "demo.boss_down");
    assert!(open[1]["events"][0]["data"]["boss"].is_string());
    assert!(open[1].get("off").is_none());

    let closed = demo_catalog(Some("示例插件关着")).summary();
    assert_eq!(closed[1]["events"], json!([]));
    assert_eq!(
        closed[1]["off"],
        "示例插件关着：这些事现在都不会发生；到「插件」里打开以后才有"
    );
    // 本体的照常
    assert_eq!(
        closed[0]["events"].as_array().unwrap().len(),
        generic_events().len()
    );
}

#[test]
fn two_plugins_cannot_register_the_same_event() {
    let mut catalog = demo_catalog(None);
    let error = catalog
        .add(demo_entry(demo_spec("demo.ace", "又一个团灭", &[]), None))
        .unwrap_err();
    assert!(error.contains("示例插件"), "{error}");
    let error = catalog
        .add(demo_entry(demo_spec("game.started", "x", &[]), None))
        .unwrap_err();
    assert!(error.contains("本体"), "{error}");
    assert_eq!(catalog.entries().len(), generic_events().len() + 3);
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
    let t = trigger(json!({"event": ["demo.boss_down"], "game": "demo",
        "match": {"team": "enemy", "boss": ["golem", "dragon"]}}));
    let fire = event(
        "demo.boss_down",
        "demo",
        json!({"team": "enemy", "boss": "golem", "stolen": false}),
    );
    assert!(trigger_matches(&t, &fire), "多出来的字段不影响");
    assert!(!trigger_matches(
        &t,
        &event(
            "demo.boss_down",
            "demo",
            json!({"team": "ally", "boss": "golem"})
        )
    ));
    assert!(!trigger_matches(
        &t,
        &event("demo.boss_down", "demo", json!({"team": "enemy"}))
    ));
    assert!(!trigger_matches(
        &t,
        &event(
            "demo.boss_down",
            "cs2",
            json!({"team": "enemy", "boss": "golem"})
        )
    ));
    let any = trigger(json!({"event": ["demo.*"], "match": {"count": 5}}));
    assert!(trigger_matches(
        &any,
        &event("demo.multikill", "demo", json!({"count": 5.0}))
    ));
    assert!(!trigger_matches(
        &any,
        &event("demox.multikill", "demo", json!({"count": 5}))
    ));
    let mut off = any.clone();
    off.enabled = false;
    assert!(!trigger_matches(
        &off,
        &event("demo.multikill", "demo", json!({"count": 5}))
    ));
}

#[test]
fn once_per_match_and_cooldown() {
    let once = trigger(json!({"event": ["demo.boss_down"], "oncePerMatch": true}));
    let every = trigger(json!({"event": ["demo.boss_down"]}));
    let cooled = trigger(json!({"event": ["demo.boss_down"], "cooldownSeconds": 60}));
    let armed = [
        Armed {
            id: "once",
            trigger: &once,
        },
        Armed {
            id: "every",
            trigger: &every,
        },
        Armed {
            id: "cooled",
            trigger: &cooled,
        },
    ];
    let boss = event("demo.boss_down", "demo", json!({}));
    let mut t = Triggers::default();
    let mut fired: HashMap<String, u32> = HashMap::new();
    for i in 0..5u64 {
        for f in t.on_event(&armed, &boss, i * 10_000) {
            *fired.entry(f.automation).or_default() += 1;
        }
    }
    assert_eq!(fired["once"], 1);
    assert_eq!(fired["every"], 5, "触发本身不限次数（每局叫她几次看 ask）");
    // 0 秒、40 秒都在 60 秒冷却里只算第一次；（0、10、20、30、40 秒）
    assert_eq!(fired["cooled"], 1);

    // 一局结束：次数清零
    t.on_event(
        &armed,
        &event("game.match_ended", "demo", json!({})),
        100_000,
    );
    let again: Vec<String> = t
        .on_event(&armed, &boss, 200_000)
        .into_iter()
        .map(|f| f.automation)
        .collect();
    assert_eq!(again, ["once", "every", "cooled"]);
}

/// ADR 045 D2：按游戏事件触发时每个自动化每局最多叫她 3 次，按（自动化，游戏）记；一局结束、游戏退出清零。
#[test]
fn asks_are_capped_per_automation_game_and_match() {
    let mut t = Triggers::default();
    let take = |t: &mut Triggers, a: &str, g: &str| t.take_ask(a, g, ASKS_PER_MATCH);
    for _ in 0..ASKS_PER_MATCH {
        assert!(take(&mut t, "scout", "lol"));
    }
    assert!(!take(&mut t, "scout", "lol"), "第 4 次不交给她");
    assert!(take(&mut t, "scout", "cs2"), "别的游戏另算");
    assert!(take(&mut t, "other", "lol"), "别的自动化另算");
    // 一局结束：这个游戏的清零，别的游戏不动
    t.on_event(&[], &event("game.match_ended", "lol", json!({})), 1);
    assert!(take(&mut t, "scout", "lol"));
    assert!(take(&mut t, "scout", "cs2"));
    assert!(take(&mut t, "scout", "cs2"));
    assert!(!take(&mut t, "scout", "cs2"));
    // 游戏退出也清零
    t.on_event(&[], &event("game.stopped", "cs2", json!({})), 2);
    assert!(take(&mut t, "scout", "cs2"));
    // 别的事不清零
    t.on_event(&[], &event("game.phase_changed", "lol", json!({})), 3);
    assert!(take(&mut t, "scout", "lol"));
    assert!(take(&mut t, "scout", "lol"));
    assert!(!take(&mut t, "scout", "lol"));
}

#[test]
fn for_waits_until_the_phase_has_held() {
    let t60 = trigger(json!({"event": ["game.phase_changed"], "game": "lol",
        "match": {"to": "in_match"}, "for": 60}));
    let armed = [Armed {
        id: "ward",
        trigger: &t60,
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
    }];
    assert!(t.tick(&disabled, 460_000).is_empty());
}

#[test]
fn catalog_names_follow_the_naming_rule() {
    // 本体只登记通用的 game.*（游戏专用的事由插件在 plugin.toml 里登记，原则 15）
    for s in generic_events() {
        s.check(GENERIC_SCOPE).unwrap();
        assert!(s.needs.is_empty());
    }
    assert!(EventCatalog::generic()
        .entries()
        .iter()
        .all(|e| e.adapter == GENERIC_ADAPTER && e.plugin.is_none() && e.off.is_none()));
    // 写法的检查真能抓到违反（插件的清单也用它）
    for (bad, want) in [
        (demo_spec("demo.BossDown", "x", &[]), "demo.<名字>"),
        (demo_spec("game.boss_down", "x", &[]), "demo.<名字>"),
        (demo_spec("demo.", "x", &[]), "demo.<名字>"),
        (demo_spec("demo.boss_down", " ", &[]), "说明"),
        (
            demo_spec("demo.boss_down", "x", &["boss_name"]),
            "camelCase",
        ),
    ] {
        let error = bad.check("demo").unwrap_err();
        assert!(error.contains(want), "{} → {error}", bad.kind);
    }
    demo_spec("demo.boss_down", "打掉了首领", &["bossName"])
        .check("demo")
        .unwrap();
    for p in Phase::ALL {
        let text = serde_json::to_value(p).unwrap();
        assert_eq!(text, p.as_str());
    }
}

/// ADR 043 D10：登记为不给她看的字段（`private`，语音输入发出去的那句话）在 `game.state` 的最近几件事里去掉，
/// 目录里写明“只交给自动化”；自动化触发时拿到的事件还是整份。`private` 要是 `data` 里登记过的字段。
#[test]
fn private_fields_reach_automations_but_not_her() {
    let mut spec = demo_spec("demo.said", "说了一句", &["text", "channel"]);
    spec.needs.clear();
    spec.private = vec!["text".into()];
    assert!(spec.check("demo").is_ok());
    let mut bad = spec.clone();
    bad.private = vec!["words".into()];
    assert!(bad.check("demo").unwrap_err().contains("words"));
    let mut catalog = EventCatalog::generic();
    catalog.add(demo_entry(spec, None)).unwrap();

    let mut hub = Hub::default();
    hub.observe("demo", seen(Phase::InMatch, ""), 0);
    let event = hub
        .push(
            "demo",
            "demo.said",
            "x",
            data(&[("text", json!("打野来中路")), ("channel", json!("team"))]),
            None,
            1,
        )
        .unwrap();
    // 自动化：整份
    let on = trigger(json!({"event": ["demo.said"]}));
    let fires = Triggers::default().on_event(
        &[Armed {
            id: "a",
            trigger: &on,
        }],
        &event,
        2,
    );
    assert_eq!(fires.len(), 1);
    assert_eq!(fires[0].event.data["text"], "打野来中路");
    // 给她看的：没有那句话
    let shown = catalog.data_for_her(&event.kind, &event.data);
    assert!(!shown.contains_key("text"));
    assert_eq!(shown["channel"], "team");
    // 别的事照旧整份
    assert_eq!(catalog.data_for_her("demo.other", &event.data), event.data);
    // 目录里写明这个字段只交给自动化
    let summary = catalog.summary().to_string();
    assert!(summary.contains("只交给自动化"), "{summary}");
}
