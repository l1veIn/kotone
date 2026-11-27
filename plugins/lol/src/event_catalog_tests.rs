//! 英雄联盟的事件目录由插件登记（插件重构第 7 步，ADR 036 D4、ADR 038 D3 第 10 条）：
//! - 插件报给游戏中心的每种事都在 `plugin.toml` 的 `[[events]]` 里，登记的每种也真的会报（原则 15、16）；
//! - 界面上的名字（messages）和清单一致；
//! - 旧的自动化（配置里存着 `lol.*` 触发）照样读得进来、照样触发；插件关着时保存照常，只是说清楚不会发生（原则 19、21）。

use serde_json::{json, Map, Value};

use kotone_core::automation::library::{AutomationDraft, AutomationLibrary, CreatedBy};
use kotone_core::games::{trigger_matches, Armed, EventCatalog, GameEvent, Triggers};
use kotone_core::settings::Settings;
use kotone_plugin_api::{event_catalog, Plugin};

use crate::client::events::new_events;
use crate::client::live::{parse_all_game_data, tests as l};
use crate::{manifest, LolPlugin};

fn registered() -> Vec<String> {
    manifest().events.iter().map(|e| e.kind.clone()).collect()
}

/// `plugins/lol/src` 下的全部 Rust 源码（路径，内容）。
fn sources() -> Vec<(String, String)> {
    fn walk(dir: &std::path::Path, out: &mut Vec<(String, String)>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push((
                    path.display().to_string(),
                    std::fs::read_to_string(&path).unwrap(),
                ));
            }
        }
    }
    let mut out = Vec::new();
    walk(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut out,
    );
    out
}

/// 一段源码里的字符串字面量 `"lol.<名字>"`（名字是小写、数字、下划线）。
fn lol_literals(text: &str) -> Vec<String> {
    text.split('"')
        .skip(1)
        .step_by(2)
        .filter(|s| {
            s.strip_prefix("lol.").is_some_and(|name| {
                !name.is_empty()
                    && name
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
            })
        })
        .map(str::to_string)
        .collect()
}

/// `host.game_event(<游戏>, <类型>, …)` 的第二个参数（原样，去空白）。
fn game_event_kinds(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (at, _) in text.match_indices(".game_event(") {
        let args = &text[at + ".game_event(".len()..];
        let mut depth = 0;
        let mut commas = Vec::new();
        for (i, c) in args.char_indices() {
            match c {
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' if depth == 0 => break,
                ')' | ']' | '}' => depth -= 1,
                ',' if depth == 0 => commas.push(i),
                _ => {}
            }
            if commas.len() == 2 {
                break;
            }
        }
        if let [first, second] = commas[..] {
            out.push(args[first + 1..second].trim().to_string());
        }
    }
    out
}

/// 插件报的每种事都登记过：局内接口的事件列表全都换得出来、都在清单里；直接 `game_event` 报的字面量也都在；
/// 不是字面量的只能是 `new_events` 换出来的 `kind`。反过来，登记的每种源码里都报得出（没有空登记）。
#[test]
fn every_emitted_event_is_registered() {
    let registered = registered();
    assert!(!registered.is_empty());

    // 局内接口的事件列表：每种认得的都放进来一次
    let mut v = l::sample_json();
    v["events"]["Events"] = json!([
        {"EventID": 1, "EventName": "FirstBlood", "EventTime": 90.0, "Recipient": "玩家"},
        {"EventID": 2, "EventName": "DragonKill", "EventTime": 300.0, "DragonType": "Elder", "Stolen": "False", "KillerName": "敌人一"},
        {"EventID": 3, "EventName": "HeraldKill", "EventTime": 400.0, "Stolen": "True", "KillerName": "玩家"},
        {"EventID": 4, "EventName": "BaronKill", "EventTime": 1200.0, "Stolen": "False", "KillerName": "玩家"},
        {"EventID": 5, "EventName": "TurretKilled", "EventTime": 500.0, "TurretKilled": "Turret_T2_L_03_A", "KillerName": "玩家"},
        {"EventID": 6, "EventName": "InhibKilled", "EventTime": 800.0, "InhibKilled": "Barracks_T1_C1", "KillerName": "敌人二"},
        {"EventID": 7, "EventName": "Multikill", "EventTime": 812.0, "KillerName": "敌人二", "KillStreak": 5},
        {"EventID": 8, "EventName": "Ace", "EventTime": 815.0, "Acer": "敌人二", "AcingTeam": "CHAOS"},
        {"EventID": 9, "EventName": "ChampionKill", "EventTime": 901.0, "KillerName": "玩家", "VictimName": "敌人一"}
    ]);
    let game = parse_all_game_data(&v).unwrap();
    let live = new_events(&v, &game, 0);
    assert_eq!(live.len(), 8, "每种认得的事都换出来了");
    for (_, kind, data, _) in &live {
        let spec = manifest()
            .events
            .iter()
            .find(|e| e.kind == *kind)
            .unwrap_or_else(|| panic!("{kind} 没在 plugin.toml 的 [[events]] 里登记"));
        for field in data.keys() {
            assert!(
                spec.data.contains_key(field),
                "{kind} 的 data.{field} 没登记"
            );
        }
    }

    // 源码：client/events.rs 里的 lol.* 都是事件；game_event 的类型要么是登记过的字面量，要么是上面那个 kind
    let mut seen = Vec::new();
    for (path, text) in sources() {
        let path = path.replace('\\', "/");
        if path.ends_with("/event_catalog_tests.rs") {
            continue; // 这个文件自己（注释里写着 game_event 的样子）
        }
        if path.ends_with("/client/events.rs") {
            for kind in lol_literals(&text) {
                assert!(registered.contains(&kind), "{path}：{kind} 没登记");
                seen.push(kind);
            }
        }
        for arg in game_event_kinds(&text) {
            match arg.strip_prefix('"').and_then(|a| a.strip_suffix('"')) {
                Some(kind) => {
                    assert!(
                        registered.iter().any(|r| r == kind),
                        "{path}：{kind} 没登记"
                    );
                    seen.push(kind.to_string());
                }
                None => assert_eq!(
                    arg, "kind",
                    "{path}：game_event 的类型要是字面量或 new_events 的 kind"
                ),
            }
        }
    }
    for kind in &registered {
        assert!(seen.contains(kind), "{kind} 登记了，但插件从不报它");
    }
}

/// 界面上的名字：`plugin_lol_event_<名字>` 中文和清单的 label 一样，英文也有。
#[test]
fn messages_match_the_registered_events() {
    let zh: Map<String, Value> =
        serde_json::from_str(include_str!("../messages/zh-CN.json")).unwrap();
    let en: Map<String, Value> = serde_json::from_str(include_str!("../messages/en.json")).unwrap();
    for spec in &manifest().events {
        let key = format!(
            "plugin_lol_event_{}",
            spec.kind.strip_prefix("lol.").unwrap()
        );
        assert_eq!(
            zh.get(&key).and_then(Value::as_str),
            Some(spec.label.as_str()),
            "{key}"
        );
        assert!(
            en.get(&key)
                .and_then(Value::as_str)
                .is_some_and(|t| !t.is_empty()),
            "en {key}"
        );
    }
}

/// 宿主把开着的插件登记的事并进目录；插件关着、“读客户端数据”关着时还认得，只是标着不会发生。
#[test]
fn the_host_catalog_follows_the_plugin_switches() {
    let plugin = LolPlugin::new();
    let catalog = |s: &Settings| event_catalog([plugin.manifest()], s);
    let on = catalog(&Settings::default());
    for kind in registered() {
        let entry = on.get(&kind).unwrap();
        assert_eq!(entry.adapter, "lol");
        assert_eq!(entry.games, ["lol"]);
        assert!(entry.off.is_none(), "{kind}");
    }
    let mut closed = Settings::default();
    closed.plugins.entry("lol".into()).or_default().enabled = Some(false);
    let closed = catalog(&closed);
    assert!(closed.knows("lol.*"));
    assert_eq!(
        closed.get("lol.dragon_kill").unwrap().off.as_deref(),
        Some("英雄联盟插件关着")
    );
    let mut no_client = Settings::default();
    no_client
        .plugins
        .entry("lol".into())
        .or_default()
        .sources
        .insert("client".into(), false);
    assert_eq!(
        catalog(&no_client).get("lol.ace").unwrap().off.as_deref(),
        Some("英雄联盟插件的“读客户端数据”关着")
    );
    // 只有本体的目录（没有英雄联盟插件）：一种 lol.* 都不认得
    assert!(!EventCatalog::generic().knows("lol.*"));
}

/// 原则 19：旧版本存下的自动化（`on` 里写着 `lol.*`）照样读得进来、照样触发；插件关着时改一改照样保存，
/// 只是说清楚“英雄联盟插件关着，这个触发不会发生”。
#[test]
fn old_automations_with_lol_triggers_still_load_and_fire() {
    let dir =
        std::env::temp_dir().join(format!("kotone-lol-old-automations-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("automations.json");
    // 0.2.0-rc.7 写下的样子（事件目录还在本体里的时候）
    let old = json!({
        "version": 1, "nextId": 3, "builtin": [],
        "automations": [
            {"id": "auto-1", "name": "对面拿龙提醒", "description": "", "params": [],
             "action": {"kind": "speak", "text": "对面拿了小龙", "delivery": ["voice"]},
             "createdBy": "user", "createdAtMs": 1, "updatedAtMs": 1,
             "on": {"event": ["lol.dragon_kill"], "game": "lol", "match": {"team": "enemy"},
                    "for": 0, "mode": "single", "oncePerMatch": false, "cooldownSeconds": 0, "enabled": true}},
            {"id": "auto-2", "name": "英雄联盟的任何事", "params": [],
             "action": {"kind": "prompt", "text": "说一句"},
             "createdBy": "agent", "createdAtMs": 1, "updatedAtMs": 1,
             "on": {"event": ["lol.*"], "enabled": true}}
        ]
    });
    std::fs::write(&path, serde_json::to_vec(&old).unwrap()).unwrap();
    let library = AutomationLibrary::open(path.clone()).unwrap();
    let list = library.list();
    assert_eq!(list.len(), 2);

    // 照样触发：插件报的龙（new_events 换出来的）对得上
    let mut v = l::sample_json();
    v["events"]["Events"] = json!([
        {"EventID": 7, "EventName": "DragonKill", "EventTime": 600.5, "DragonType": "Fire", "Stolen": "True", "KillerName": "敌人一"}
    ]);
    let game = parse_all_game_data(&v).unwrap();
    let (_, kind, data, time) = new_events(&v, &game, 0).remove(0);
    let event = GameEvent {
        id: "lol-live-7".into(),
        kind: kind.into(),
        game: "lol".into(),
        match_id: "lol-1".into(),
        time: 1,
        game_time: Some(time),
        source: "lol/live-client".into(),
        data,
    };
    let armed: Vec<Armed> = list
        .iter()
        .map(|a| Armed {
            id: &a.id,
            trigger: a.on.as_ref().unwrap(),
        })
        .collect();
    assert!(armed.iter().all(|a| trigger_matches(a.trigger, &event)));
    let fired: Vec<String> = Triggers::default()
        .on_event(&armed, &event, 1)
        .into_iter()
        .map(|f| f.automation)
        .collect();
    assert_eq!(fired, ["auto-1", "auto-2"]);

    // 插件关着时改个名字再保存：照常保存（不报废），提醒里说清楚
    let mut closed = Settings::default();
    closed.plugins.entry("lol".into()).or_default().enabled = Some(false);
    let events = event_catalog([manifest()], &closed);
    let first = list.iter().find(|a| a.id == "auto-1").unwrap();
    let draft = AutomationDraft {
        id: Some(first.id.clone()),
        name: "对面拿龙提醒（改）".into(),
        description: String::new(),
        params: vec![],
        action: first.action.clone(),
        schedule: None,
        on: first.on.clone(),
    };
    let saved = library
        .save(draft.clone(), CreatedBy::User, &events)
        .unwrap();
    assert_eq!(saved.on, first.on);
    assert_eq!(
        events.notes(saved.on.as_ref().unwrap()),
        ["英雄联盟插件关着，这个触发不会发生；到「插件」里打开以后照常触发"]
    );
    // 重新打开也还在
    let reopened = AutomationLibrary::open(path).unwrap();
    assert_eq!(
        reopened.get("auto-1").unwrap().on.unwrap().event,
        ["lol.dragon_kill"]
    );
    // 没装英雄联盟插件（只有本体的目录）：认不得，说“没有这种游戏事件”
    let error = library
        .save(draft, CreatedBy::User, &EventCatalog::generic())
        .unwrap_err();
    assert!(error.contains("没有“lol.dragon_kill”"), "{error}");
    let _ = std::fs::remove_dir_all(&dir);
}
