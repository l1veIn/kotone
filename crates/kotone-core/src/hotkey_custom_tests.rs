//! 自己加的热键（ADR 040 第 3 期，D5）：快捷指令搬进热键表、旧字段 `quickMessages` 是镜像、往返、旧写法。
//! 配置格式是单向门（原则 11、19）：这里每条都对着“旧版本读得到什么、新版本读回来是什么”。

use super::*;
use crate::assistant::CompanionConfig;
use crate::hotkey_bindings::{CMD_CALL, CMD_STOP, CMD_TALK};
use crate::settings::{load_from, merge_json, save_to, SettingsRepository};
use serde_json::json;
use std::sync::{Arc, RwLock};

fn load_json(value: Value) -> (Settings, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    std::fs::write(&path, serde_json::to_string(&value).unwrap()).unwrap();
    (load_from(&path), dir)
}

fn saved_json(settings: &Settings) -> Value {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    save_to(&path, settings).unwrap();
    serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap()
}

/// 存盘再读回来（新版本自己往返）。
fn round_trip(settings: &Settings) -> Settings {
    load_json(saved_json(settings)).0
}

/// 表里自己加的条目（JSON 原样，按表的顺序）。
fn customs(settings: &Settings) -> Vec<Value> {
    settings
        .hotkeys
        .bindings
        .iter()
        .filter(|b| b.action().is_some())
        .map(|b| b.0.clone())
        .collect()
}

/// 旧字段里快捷指令的 id（按顺序）。
fn mirror_ids(settings: &Settings) -> Vec<String> {
    settings
        .assistant
        .companion
        .quick_messages
        .iter()
        .map(|m| m.id.clone())
        .collect()
}

/// `settings.update` / 界面的补丁（桌面端 `update_settings`、命令行 `config set` 都是“序列化 → 合并补丁 → 反序列化”）。
fn patch(current: &Settings, patch: Value) -> Settings {
    let mut merged = serde_json::to_value(current).unwrap();
    merge_json(&mut merged, &patch);
    let mut next: Settings = serde_json::from_value(merged).unwrap();
    next.reconcile_hotkeys(current);
    next
}

/// 三条快捷指令：一句话带键、运行自动化带入参、一句话没绑键（旧版本允许）。
fn three_quick() -> Value {
    json!([
        {"id": "q1", "label": "看队友", "text": "帮我看看队友说了什么", "hotkey": "F7"},
        {"id": "q2", "label": "连发", "text": "", "hotkey": "F8", "automationId": "auto-1", "args": {"n": 3}},
        {"id": "q3", "label": "", "text": "我们推塔", "hotkey": ""}
    ])
}

/// 它们在表里的样子。
fn three_entries() -> Vec<Value> {
    vec![
        json!({"key": "F7", "say": "帮我看看队友说了什么", "id": "q1", "label": "看队友"}),
        json!({"key": "F8", "tool": "automation.run", "args": {"id": "auto-1", "args": {"n": 3}}, "id": "q2", "label": "连发"}),
        json!({"key": "", "say": "我们推塔", "id": "q3"}),
    ]
}

/// 第 2 期的配置：热键表有了（三个默认键），快捷指令还在旧字段。
fn phase2_config(quick: Value) -> Value {
    json!({
        "ui": {"legacyUpgrade": "notNeeded"},
        "hotkeys": {"bindings": [
            {"key": "Ctrl+Backquote", "command": CMD_CALL},
            {"key": "Ctrl+Shift+F12", "command": CMD_STOP},
            {"key": "CapsLock", "command": CMD_TALK}
        ]},
        "assistant": {"companion": {"quickMessages": quick}}
    })
}

fn say_entry(id: &str, key: &str, text: &str) -> Binding {
    Binding(json!({"key": key, "say": text, "id": id}))
}

/// （1）第 2 期的配置读进来：三条快捷指令进表，id、名字、键、话、自动化、入参一字不差；存盘后旧字段和原来逐字段相同。
#[test]
fn a_phase_two_config_moves_its_quick_commands_into_the_table() {
    let (settings, _dir) = load_json(phase2_config(three_quick()));
    assert_eq!(customs(&settings), three_entries());
    // 绑命令的三条没动，在前面
    assert_eq!(settings.binding_key(CMD_CALL), "Ctrl+Backquote");
    assert_eq!(settings.hotkeys.bindings.len(), 6);
    let saved = saved_json(&settings);
    assert_eq!(
        saved["assistant"]["companion"]["quickMessages"],
        three_quick()
    );
    assert_eq!(
        saved["hotkeys"]["bindings"].as_array().unwrap()[3..],
        three_entries()[..]
    );
    // 再读一遍不会再搬一遍
    let again = round_trip(&settings);
    assert_eq!(customs(&again), three_entries());
}

/// （2）ADR 040 以前的配置（没有 `hotkeys`）：别的热键旧字段照第 2 期搬，快捷指令跟在后面。
#[test]
fn an_old_config_without_a_table_moves_quick_commands_too() {
    let config = json!({
        "ui": {"legacyUpgrade": "notNeeded"},
        "hotkey": {"key": "F8", "mode": "hold"},
        "assistant": {"hotkey": "Ctrl+F9", "callHotkeyRevision": 1, "companion": {"quickMessages": three_quick()}}
    });
    let (settings, _dir) = load_json(config);
    assert_eq!(settings.binding_key(CMD_CALL), "Ctrl+F9");
    assert_eq!(settings.binding_key(CMD_TALK), "F8");
    assert_eq!(customs(&settings), three_entries());
    assert_eq!(
        saved_json(&settings)["assistant"]["companion"]["quickMessages"],
        three_quick()
    );
}

/// （3）镜像是表里前 8 条“说一句 / 运行自动化”，过得了旧版本的校验；工具、页面、第 9 条以后、写不进旧字段的不进镜像。
#[test]
fn the_mirror_is_the_first_eight_that_old_versions_can_read() {
    let mut settings = Settings::default();
    settings.hotkeys.bindings.push(Binding(
        json!({"key": "F1", "tool": "game_chat.send", "args": {"text": "gg"}, "id": "t1"}),
    ));
    settings.hotkeys.bindings.push(Binding(
        json!({"key": "F2", "page": "ui://kotone/lol/match", "id": "p1"}),
    ));
    settings
        .hotkeys
        .bindings
        .push(say_entry("long", "", &"字".repeat(201)));
    for n in 1..=10 {
        settings.hotkeys.bindings.push(say_entry(
            &format!("s{n}"),
            &format!("Ctrl+F{n}"),
            &format!("第 {n} 句"),
        ));
    }
    settings.sync_hotkey_mirrors();
    assert_eq!(
        mirror_ids(&settings),
        (1..=8).map(|n| format!("s{n}")).collect::<Vec<_>>()
    );
    CompanionConfig {
        quick_messages: settings.assistant.companion.quick_messages.clone(),
        ..Default::default()
    }
    .validate()
    .expect("写给旧版本的镜像要过得了它的校验");
    // 存盘读回来：表里 14 条自己加的都在
    let back = round_trip(&settings);
    assert_eq!(customs(&back), customs(&settings));
    assert_eq!(customs(&back).len(), 13);
    // 运行自动化那种不留 text（旧字段里写空）；自动化 id 去掉空格
    let mut odd = Settings::default();
    odd.assistant.companion.quick_messages = vec![QuickMessage {
        id: "q".into(),
        text: "切成运行自动化之前的话".into(),
        hotkey: "F9".into(),
        automation_id: " auto-1 ".into(),
        ..Default::default()
    }];
    let odd = round_trip(&odd);
    assert_eq!(
        customs(&odd),
        vec![json!({"key": "F9", "tool": "automation.run", "args": {"id": "auto-1"}, "id": "q"})]
    );
    assert_eq!(odd.assistant.companion.quick_messages[0].text, "");
}

/// 一张有 10 条“说一句”、一个工具、一个页面的表（第 3 期以后的样子）。
fn big_table() -> Settings {
    let mut settings = Settings::default();
    for n in 1..=10 {
        settings.hotkeys.bindings.push(say_entry(
            &format!("s{n}"),
            &format!("Ctrl+F{n}"),
            &format!("第 {n} 句"),
        ));
    }
    settings.hotkeys.bindings.push(Binding(
        json!({"key": "F1", "tool": "game_chat.send", "args": {"text": "gg"}, "id": "t1"}),
    ));
    settings.hotkeys.bindings.push(Binding(
        json!({"key": "F2", "page": "ui://kotone/lol/match", "id": "p1"}),
    ));
    settings.sync_hotkey_mirrors();
    settings
}

/// （4）旧写法（`settings.update` 整组替换快捷指令、还没改的界面）：加一条、删第 3 条、改第 1 条的键和话 → 表跟着改；
/// 第 9、10 条、工具、页面不动；新加的那条在镜像里。
#[test]
fn old_style_writes_change_the_table_by_id() {
    let current = big_table();
    let mut quick = current.assistant.companion.quick_messages.clone();
    quick[0].hotkey = "Alt+1".into();
    quick[0].text = "改过的第一句".into();
    quick.remove(2); // s3
    quick.push(QuickMessage {
        id: "new".into(),
        text: "新加的".into(),
        hotkey: "Alt+9".into(),
        ..Default::default()
    });
    let next = patch(
        &current,
        json!({"assistant": {"companion": {"quickMessages": quick}}}),
    );
    let ids: Vec<&str> = next
        .hotkeys
        .bindings
        .iter()
        .filter_map(Binding::id)
        .collect();
    assert_eq!(
        ids,
        vec!["s1", "s2", "s4", "s5", "s6", "s7", "s8", "new", "s9", "s10", "t1", "p1"]
    );
    let first = &next.hotkeys.bindings[3];
    assert_eq!((first.key(), first.say()), ("Alt+1", Some("改过的第一句")));
    assert_eq!(
        mirror_ids(&next),
        vec!["s1", "s2", "s4", "s5", "s6", "s7", "s8", "new"]
    );
    // 顺序照旧字段：把第 2 条挪到最前
    let mut reordered = next.assistant.companion.quick_messages.clone();
    let second = reordered.remove(1);
    reordered.insert(0, second);
    let moved = patch(
        &next,
        json!({"assistant": {"companion": {"quickMessages": reordered}}}),
    );
    assert_eq!(mirror_ids(&moved)[..2], ["s2", "s1"]);
    assert_eq!(customs(&moved).len(), 12, "别的条目一条没少");
    // 什么都没改的补丁（别的设置）：表不变
    let same = patch(&moved, json!({"language": "en"}));
    assert_eq!(customs(&same), customs(&moved));
}

/// （5）表改了（删掉第 2 条）：镜像跟着挪，原来的第 9 条进镜像。
#[test]
fn table_edits_move_the_mirror() {
    let current = big_table();
    let mut bindings = current.hotkeys.bindings.clone();
    bindings.retain(|b| b.id() != Some("s2"));
    let next = patch(&current, json!({"hotkeys": {"bindings": bindings}}));
    assert_eq!(
        mirror_ids(&next),
        vec!["s1", "s3", "s4", "s5", "s6", "s7", "s8", "s9"]
    );
}

/// （6）同一个补丁里表和旧字段都改了：以表为准（和第 2 期别的热键一样）。
#[test]
fn a_patch_that_sets_the_table_wins_over_quick_messages() {
    let current = big_table();
    let next = patch(
        &current,
        json!({
            "hotkeys": {"bindings": [say_entry("only", "F3", "只剩这一句").0]},
            "assistant": {"companion": {"quickMessages": [
                {"id": "ignored", "label": "", "text": "旧写法这句不算", "hotkey": "F4"}
            ]}}
        }),
    );
    assert_eq!(
        customs(&next),
        vec![say_entry("only", "F3", "只剩这一句").0]
    );
    assert_eq!(mirror_ids(&next), vec!["only"]);
}

/// （7）代码直接改了内存里的快捷指令就 `save_to`（没经过对齐）：保存时照旧写法算进表；仓库的修改也一样。
#[test]
fn direct_edits_and_repository_writes_reach_the_table() {
    let mut settings = Settings::default();
    settings.assistant.companion.quick_messages = vec![QuickMessage {
        id: "q1".into(),
        text: "直接改的".into(),
        hotkey: "F6".into(),
        ..Default::default()
    }];
    let back = round_trip(&settings);
    assert_eq!(customs(&back), vec![say_entry("q1", "F6", "直接改的").0]);

    let dir = tempfile::tempdir().unwrap();
    let repo = SettingsRepository::new_at(
        Arc::new(RwLock::new(Settings::default())),
        dir.path().join("config.json"),
    );
    let next = repo
        .update(|s| {
            s.assistant.companion.quick_messages = vec![QuickMessage {
                id: "q2".into(),
                text: "仓库里改的".into(),
                hotkey: "F5".into(),
                ..Default::default()
            }];
            Ok(())
        })
        .unwrap();
    assert_eq!(customs(&next), vec![say_entry("q2", "F5", "仓库里改的").0]);
    let removed = repo
        .update(|s| {
            s.assistant.companion.quick_messages.clear();
            Ok(())
        })
        .unwrap();
    assert!(customs(&removed).is_empty(), "旧写法删了就从表里删");
}

/// （8）装回 ADR 040 以前的版本：它只认 `quickMessages`（前 8 条都在、能改），保存时丢掉整个 `hotkeys`；
/// 再装回新版本，从旧字段重新搬——它改过的照它的，镜像以外的（第 9 条以后、工具、页面）没了（写进 ADR）。
#[test]
fn going_back_to_a_version_before_the_table_and_forward_again() {
    let saved = saved_json(&big_table());
    let old_quick = saved["assistant"]["companion"]["quickMessages"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(old_quick.len(), 8);
    // 旧版本读到的：每条都是它认识的形状
    for message in &old_quick {
        let message: QuickMessage = serde_json::from_value(message.clone()).unwrap();
        assert!(!message.id.is_empty() && !message.text.is_empty());
    }
    // 旧版本改了第 1 条的话、删了第 8 条，存盘时丢掉 hotkeys
    let mut old_version = saved.clone();
    old_version.as_object_mut().unwrap().remove("hotkeys");
    let quick = old_version["assistant"]["companion"]["quickMessages"]
        .as_array_mut()
        .unwrap();
    quick[0]["text"] = json!("旧版本改的");
    quick.pop();
    let (back, _dir) = load_json(old_version);
    assert_eq!(
        mirror_ids(&back),
        vec!["s1", "s2", "s3", "s4", "s5", "s6", "s7"]
    );
    assert_eq!(customs(&back).len(), 7, "第 9、10 条、工具、页面没了");
    assert_eq!(customs(&back)[0]["say"], "旧版本改的");
    assert_eq!(
        back.binding_key(CMD_CALL),
        "Ctrl+Backquote",
        "别的热键照第 2 期搬"
    );
}

/// （9）读配置时表里已有的以表为准（和第 2 期“有表以表为准”一样）：手改了旧字段里一条已经在表里的，不搬；
/// 表里没有的 id 才搬进来。（第 2、3 期一起发版，没有“只认表、不认自己加的条目”的版本会改旧字段。）
#[test]
fn on_load_the_table_wins_for_ids_it_already_has() {
    let mut config = saved_json(&big_table());
    config["assistant"]["companion"]["quickMessages"][0]["text"] = json!("手改的");
    config["assistant"]["companion"]["quickMessages"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id": "hand", "label": "", "text": "手加的", "hotkey": ""}));
    let (settings, _dir) = load_json(config);
    assert_eq!(settings.hotkeys.bindings[3].say(), Some("第 1 句"));
    assert_eq!(customs(&settings).last().unwrap()["id"], "hand");
    assert_eq!(
        settings.assistant.companion.quick_messages[0].text,
        "第 1 句"
    );
}

/// （10）对齐两次和一次一样；她或外部 Agent 手写的条目没有 id（或 id 重复）的，下一次修改时补上，补上的不会被当成“删掉”。
#[test]
fn reconciling_is_idempotent_and_numbers_entries_without_ids() {
    let current = big_table();
    let mut bindings = current.hotkeys.bindings.clone();
    bindings.push(Binding(json!({"key": "F11", "say": "她加的，没写 id"})));
    bindings.push(Binding(json!({"key": "F12", "say": "id 重复", "id": "s1"})));
    let next = patch(&current, json!({"hotkeys": {"bindings": bindings}}));
    let ids: Vec<&str> = next
        .hotkeys
        .bindings
        .iter()
        .filter_map(Binding::id)
        .collect();
    let unique: HashSet<&str> = ids.iter().copied().collect();
    assert_eq!(ids.len(), unique.len(), "{ids:?}");
    assert_eq!(customs(&next).len(), 14);
    let mut twice = next.clone();
    twice.reconcile_hotkeys(&next);
    assert_eq!(twice.hotkeys, next.hotkeys);
    assert_eq!(
        twice.assistant.companion.quick_messages,
        next.assistant.companion.quick_messages
    );
    // 别的修改（表没动）不会把刚补上 id 的条目删掉
    let other = patch(&next, json!({"language": "en"}));
    assert_eq!(customs(&other), customs(&next));
}

/// 旧写法照样受旧版本的限制挡着（最多 8 条、话 1 到 200 字），新写法的条目自己的校验。
#[test]
fn custom_entries_are_validated() {
    let mut ok = big_table();
    assert!(ok.hotkeys.validate_custom().is_ok());
    ok.hotkeys.bindings.push(say_entry("empty", "F9", "  "));
    assert!(ok
        .hotkeys
        .validate_custom()
        .unwrap_err()
        .contains("1 到 200 字"));
    let mut many = Settings::default();
    for n in 0..=MAX_CUSTOM {
        many.hotkeys
            .bindings
            .push(say_entry(&format!("x{n}"), "", "一句"));
    }
    assert!(many
        .hotkeys
        .validate_custom()
        .unwrap_err()
        .contains("最多 100 条"));
    let mut long = Settings::default();
    long.hotkeys.bindings.push(Binding(
        json!({"key": "", "say": "一句", "id": "a", "label": "名".repeat(41)}),
    ));
    assert!(long.hotkeys.validate_custom().is_err());
}

/// 第 1、2 期的 `quick-message-n` 还认：是镜像的第 n 条；改键只改那一条的键，别的不动。
#[test]
fn quick_message_ids_still_point_at_the_mirror() {
    let mut settings = big_table();
    assert_eq!(
        resolve_row(&settings, "quick-message-2").as_deref(),
        Some("custom.s2")
    );
    assert_eq!(resolve_row(&settings, "quick-message-9"), None);
    assert_eq!(
        resolve_row(&settings, "custom.t1").as_deref(),
        Some("custom.t1")
    );
    assert_eq!(resolve_row(&settings, "kotone.call"), None);
    settings.set_custom_key("quick-message-2", "Alt+2").unwrap();
    settings.set_custom_key("custom.t1", " ").unwrap();
    let index = find_row(&settings.hotkeys, "custom.s2").unwrap();
    assert_eq!(settings.hotkeys.bindings[index].key(), "Alt+2");
    assert_eq!(settings.hotkeys.bindings[index].say(), Some("第 2 句"));
    let tool = find_row(&settings.hotkeys, "custom.t1").unwrap();
    assert_eq!(
        settings.hotkeys.bindings[tool].key(),
        "",
        "空 = 不绑键，条目留着"
    );
    assert!(settings.set_custom_key("custom.gone", "F1").is_err());
}

/// 条目要做什么：种类只能有一个，绑命令的不算；`automation.run` 只有 id / args 时是“运行已保存的自动化”。
#[test]
fn what_an_entry_does() {
    let action = |v: Value| Binding(v).action().map(|a| a.kind());
    assert_eq!(action(json!({"key": "F1", "say": "一句"})), Some("say"));
    assert_eq!(
        action(json!({"key": "F1", "tool": "automation.run", "args": {"id": "a"}})),
        Some("automation")
    );
    assert_eq!(
        action(json!({"key": "F1", "tool": "automation.run", "args": {"source": "key('Q')"}})),
        Some("tool"),
        "带程序源码的是调工具（第 2 步按权限问）"
    );
    assert_eq!(
        action(json!({"key": "F1", "page": "ui://kotone/lol/match"})),
        Some("page")
    );
    assert_eq!(
        action(json!({"key": "F1", "say": "一句", "page": "ui://x"})),
        None
    );
    assert_eq!(
        action(json!({"key": "F1", "command": "kotone.call", "say": "x"})),
        None
    );
    assert_eq!(action(json!({"key": "F1", "say": 3})), None);
    assert_eq!(action(json!("not an object")), None);
}

/// 调工具的一条（还没确认）。
fn tool_entry(id: &str, tool: &str, args: Value) -> Binding {
    Binding(json!({"key": "F6", "tool": tool, "args": args, "id": id}))
}

/// 绑的时候问权限（D9）：确认只有确认命令能写。补丁（界面的通用保存、她和外部 Agent 的 `settings.update`、命令行）
/// 写进来的确认抹掉——新加的条目自带确认、给已有的条目加确认、改了参数还留着原来的确认，都不算；只改键、名字不动确认。
#[test]
fn only_the_confirm_command_can_confirm() {
    let mut current = Settings::default();
    current
        .hotkeys
        .bindings
        .push(tool_entry("t", "settings.update", json!({"patch": {}})));
    current.sync_hotkey_mirrors();
    // 确认命令：记下这时的副作用
    let mut confirmed = current.clone();
    confirmed
        .confirm_custom("custom.t", &["writes-settings".to_string()])
        .unwrap();
    let index = find_row(&confirmed.hotkeys, "custom.t").unwrap();
    assert_eq!(
        confirmed.hotkeys.bindings[index].confirmed(),
        Some(vec!["writes-settings"])
    );
    // 补丁给已有的条目加确认（外部 Agent 伪造）：抹掉
    let mut forged = current.clone();
    forged.hotkeys.bindings[index].0["confirmed"] = json!(["writes-settings"]);
    let mut after = patch(
        &current,
        json!({"hotkeys": {"bindings": forged.hotkeys.bindings}}),
    );
    after.drop_unconfirmed(&current);
    assert_eq!(after.hotkeys.bindings[index].confirmed(), None);
    // 补丁加一条自带确认的新条目：抹掉
    let mut bindings = current.hotkeys.bindings.clone();
    let mut new = tool_entry("n", "everything.echo", json!({"message": "hi"}));
    new.0["confirmed"] = json!(["third-party-action"]);
    bindings.push(new);
    let mut after = patch(&current, json!({"hotkeys": {"bindings": bindings}}));
    after.drop_unconfirmed(&current);
    assert!(after
        .hotkeys
        .bindings
        .iter()
        .all(|b| b.confirmed().is_none()));
    // 确认过的：只改键、名字，确认留着；改了参数，确认抹掉（要重新问）
    let mut bindings = confirmed.hotkeys.bindings.clone();
    bindings[index].0["key"] = json!("F9");
    bindings[index].0["label"] = json!("改设置");
    let mut keyed = patch(
        &confirmed,
        json!({"hotkeys": {"bindings": bindings.clone()}}),
    );
    keyed.drop_unconfirmed(&confirmed);
    assert!(keyed.hotkeys.bindings[index].confirmed().is_some());
    bindings[index].0["args"] = json!({"patch": {"language": "en"}});
    let mut changed = patch(&confirmed, json!({"hotkeys": {"bindings": bindings}}));
    changed.drop_unconfirmed(&confirmed);
    assert_eq!(changed.hotkeys.bindings[index].confirmed(), None);
    // 别的设置改了：确认不动
    let mut other = patch(&confirmed, json!({"language": "en"}));
    other.drop_unconfirmed(&confirmed);
    assert!(other.hotkeys.bindings[index].confirmed().is_some());
    // 存盘读回来还在（配置文件是玩家自己的，读的时候不抹）
    assert!(round_trip(&confirmed).hotkeys.bindings[index]
        .confirmed()
        .is_some());
    // 确认了、作废了都要重新登记（从不登记变成登记，或者反过来）
    assert_ne!(
        registration_view(&confirmed.hotkeys),
        registration_view(&current.hotkeys)
    );
}

/// 只有调工具、运行自动化的能确认；第三方插件的配置变了，它的工具的确认清掉，别的不动。
#[test]
fn confirmations_belong_to_tool_calls_and_follow_the_plugin() {
    let mut settings = Settings::default();
    settings.hotkeys.bindings.extend([
        tool_entry("a", "everything.echo", json!({"message": "hi"})),
        tool_entry("b", "everything-http.echo", json!({})),
        tool_entry("c", "settings.update", json!({"patch": {}})),
        say_entry("s", "F1", "一句"),
    ]);
    settings.sync_hotkey_mirrors();
    for row in ["custom.a", "custom.b", "custom.c"] {
        settings.confirm_custom(row, &["x".to_string()]).unwrap();
    }
    assert!(
        settings.confirm_custom("custom.s", &[]).is_err(),
        "说一句不用确认"
    );
    assert!(settings.confirm_custom("custom.gone", &[]).is_err());
    assert!(settings.forget_confirmations("everything."));
    let confirmed: Vec<bool> = settings
        .hotkeys
        .bindings
        .iter()
        .filter(|b| b.action().is_some())
        .map(|b| b.confirmed().is_some())
        .collect();
    assert_eq!(confirmed, vec![false, true, true, false]);
    assert!(!settings.forget_confirmations("everything."), "没有可清的");
    // 调的是哪个工具：运行自动化就是 automation.run {id, args}
    let auto = Binding(
        json!({"key": "", "tool": "automation.run", "args": {"id": "auto-1", "args": {"n": 3}}, "id": "x"}),
    );
    assert_eq!(
        auto.action().unwrap().tool_call(),
        Some(("automation.run", json!({"id": "auto-1", "args": {"n": 3}})))
    );
    assert_eq!(
        tool_entry("y", "game_chat.send", json!({"text": "gg"}))
            .action()
            .unwrap()
            .tool_call(),
        Some(("game_chat.send", json!({"text": "gg"})))
    );
    assert_eq!(
        say_entry("z", "", "一句").action().unwrap().tool_call(),
        None
    );
}
