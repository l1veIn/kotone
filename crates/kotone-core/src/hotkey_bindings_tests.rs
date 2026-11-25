//! 热键表的存法（ADR 040 D8）：迁移、镜像、往返、旧写法、读不懂的条目。

use super::*;
use crate::settings::{load_from, merge_json, save_to, SettingsRepository};
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

/// 表里的 (命令, 键)，只看绑命令的条目。
fn table(settings: &Settings) -> Vec<(String, String)> {
    settings
        .hotkeys
        .bindings
        .iter()
        .filter_map(|b| {
            b.command_name()
                .map(|c| (c.to_string(), b.key().to_string()))
        })
        .collect()
}

fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
    list.iter()
        .map(|(c, k)| (c.to_string(), k.to_string()))
        .collect()
}

/// 0.2 的老用户（不是 0.1.x 升级来的）：每个热键字段都写着。
fn full_old_config() -> Value {
    json!({
        "ui": {"legacyUpgrade": "notNeeded"},
        "hotkey": {"key": "F8", "mode": "hold"},
        "automationStopHotkey": "Ctrl+Shift+F11",
        "runtimeToggleHotkey": "Ctrl+Alt+K",
        "assistant": {"hotkey": "Ctrl+F9", "callHotkeyRevision": 1},
        "plugins": {"voice-input": {"settings": {
            "channelCycleHotkey": "Shift+CapsLock",
            "resendLastHotkey": "Alt+R",
            "togglePostProcessingHotkey": "Alt+P",
            "cyclePostProcessingHotkey": "Alt+O"
        }}}
    })
}

#[test]
fn a_new_install_has_exactly_the_three_default_keys() {
    let settings = Settings::default();
    assert_eq!(
        table(&settings),
        pairs(&[
            (CMD_CALL, "Ctrl+Backquote"),
            (CMD_STOP, "Ctrl+Shift+F12"),
            (CMD_TALK, "CapsLock")
        ])
    );
    // 默认设置自己就是对齐的：镜像照表算出来不变
    let mut synced = settings.clone();
    synced.sync_hotkey_mirrors();
    assert_eq!(saved_json(&synced), saved_json(&settings));
    assert_eq!(
        settings.channel_cycle_hotkey, "",
        "新装不再默认带频道切换（D4）"
    );

    // 第一次打开（没有配置文件）：落盘的就是这三条，旧字段写全
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    let loaded = load_from(&path);
    assert_eq!(table(&loaded), table(&settings));
    let written: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(written["hotkey"]["key"], "CapsLock");
    assert_eq!(
        written["plugins"]["voice-input"]["settings"]["channelCycleHotkey"],
        ""
    );
}

#[test]
fn an_old_config_moves_every_key_into_the_table() {
    let (settings, _dir) = load_json(full_old_config());
    assert_eq!(
        table(&settings),
        pairs(&[
            (CMD_CALL, "Ctrl+F9"),
            (CMD_STOP, "Ctrl+Shift+F11"),
            (CMD_SPEECH_TOGGLE, "Ctrl+Alt+K"),
            (CMD_TALK, "F8"),
            (CMD_CHANNEL_CYCLE, "Shift+CapsLock"),
            (CMD_RESEND_LAST, "Alt+R"),
            (CMD_PP_TOGGLE, "Alt+P"),
            (CMD_PP_CYCLE, "Alt+O"),
        ])
    );
    // 旧字段照旧读得到（登记、界面这一步还读它们）
    assert_eq!(settings.hotkey.key, "F8");
    assert_eq!(settings.call_hotkey(), "Ctrl+F9");
    assert_eq!(settings.cycle_post_processing_hotkey, "Alt+O");
}

#[test]
fn missing_old_fields_use_the_old_version_defaults() {
    // ADR 008 以前的配置没有 channelCycleHotkey，那时它默认 Shift+CapsLock：照旧生效（D4）
    let (settings, _dir) = load_json(json!({
        "ui": {"legacyUpgrade": "notNeeded"},
        "hotkey": {"key": "F8", "mode": "toggle"}
    }));
    assert_eq!(
        table(&settings),
        pairs(&[
            (CMD_CALL, "Ctrl+Backquote"),
            (CMD_STOP, "Ctrl+Shift+F12"),
            (CMD_TALK, "F8"),
            (CMD_CHANNEL_CYCLE, "Shift+CapsLock"),
        ])
    );
    assert_eq!(settings.channel_cycle_hotkey, "Shift+CapsLock");
    // 连 hotkey 都没有：说话照旧是 CapsLock
    let (bare, _dir) = load_json(json!({"ui": {"legacyUpgrade": "notNeeded"}}));
    assert_eq!(bare.binding_key(CMD_TALK), "CapsLock");
}

#[test]
fn empty_old_fields_mean_unbound() {
    let mut old = full_old_config();
    old["plugins"]["voice-input"]["settings"]["channelCycleHotkey"] = json!("");
    old["automationStopHotkey"] = json!("  ");
    old["assistant"]["hotkey"] = json!("");
    let (settings, _dir) = load_json(old);
    let commands: Vec<String> = table(&settings).into_iter().map(|(c, _)| c).collect();
    for unbound in [CMD_CHANNEL_CYCLE, CMD_STOP, CMD_CALL] {
        assert!(
            !commands.contains(&unbound.to_string()),
            "{unbound} 应该没绑"
        );
    }
    assert_eq!(settings.channel_cycle_hotkey, "");
    assert_eq!(settings.automation_stop_hotkey, "");
}

#[test]
fn the_old_call_key_default_is_upgraded_before_it_moves() {
    // 很早的配置：呼叫键还是旧默认的 `（migrate_call_hotkey_json 先改成 Ctrl+`）
    let (settings, _dir) = load_json(json!({
        "ui": {"legacyUpgrade": "notNeeded"},
        "assistant": {"hotkey": "Backquote"}
    }));
    assert_eq!(settings.binding_key(CMD_CALL), "Ctrl+Backquote");
}

#[test]
fn upgraded_users_keep_their_reminder_key_out_of_the_table() {
    // 0.1.x 升级、还没启用她：没有呼叫键，assistant.hotkey 是提醒键（D10，不进表）
    let (pending, _dir) = load_json(json!({
        "hotkey": {"key": "F8", "mode": "hold"},
        "assistant": {"hotkey": "Alt+F9"}
    }));
    assert_eq!(pending.ui.legacy_upgrade, LegacyUpgrade::Pending);
    assert_eq!(pending.binding_key(CMD_CALL), "");
    assert_eq!(pending.binding_key(CMD_TALK), "F8");
    assert_eq!(pending.assistant.hotkey, "Alt+F9", "提醒键不动");
    assert_eq!(saved_json(&pending)["assistant"]["hotkey"], "Alt+F9");

    // 启用了她：呼叫键在 companionBindings.hotkey
    let (opted, _dir) = load_json(json!({
        "ui": {"legacyUpgrade": "optedIn"},
        "assistant": {"hotkey": "Alt+F9", "companionBindings": {"hotkey": "Ctrl+Alt+F10", "connectionId": "c1"}}
    }));
    assert_eq!(opted.binding_key(CMD_CALL), "Ctrl+Alt+F10");
    assert_eq!(opted.assistant.hotkey, "Alt+F9");
    let saved = saved_json(&opted);
    assert_eq!(saved["assistant"]["hotkey"], "Alt+F9");
    assert_eq!(
        saved["assistant"]["companionBindings"]["hotkey"],
        "Ctrl+Alt+F10"
    );
    assert_eq!(
        saved["assistant"]["companionBindings"]["connectionId"],
        "c1"
    );
}

#[test]
fn an_existing_table_wins_over_old_fields() {
    let mut config = full_old_config();
    config["hotkeys"] = json!({"bindings": [{"key": "F9", "command": CMD_TALK}]});
    let (settings, _dir) = load_json(config);
    assert_eq!(table(&settings), pairs(&[(CMD_TALK, "F9")]));
    // 旧字段跟着表：hotkey.key 是 F9，其余没绑的写空
    assert_eq!(settings.hotkey.key, "F9");
    assert_eq!(settings.channel_cycle_hotkey, "");
    assert_eq!(settings.call_hotkey(), "");
    assert_eq!(settings.automation_stop_hotkey, "");

    // 空表也是表：玩家把键全清了，不从旧字段补回来
    let mut cleared = full_old_config();
    cleared["hotkeys"] = json!({"bindings": []});
    let (settings, _dir) = load_json(cleared);
    assert!(settings.hotkeys.bindings.is_empty());
    assert_eq!(settings.hotkey.key, "");
}

#[test]
fn a_broken_table_is_rebuilt_from_old_fields() {
    let mut config = full_old_config();
    config["hotkeys"] = json!({"bindings": "oops", "later": {"x": 1}});
    let (settings, _dir) = load_json(config);
    assert_eq!(settings.binding_key(CMD_TALK), "F8");
    assert_eq!(settings.binding_key(CMD_CALL), "Ctrl+F9");
    assert_eq!(
        settings.hotkeys.extra["later"],
        json!({"x": 1}),
        "hotkeys 下面别的东西留着"
    );
}

#[test]
fn saving_writes_every_old_field_so_old_versions_keep_the_keys() {
    let mut settings = Settings::default();
    settings.hotkeys.bindings = vec![
        Binding::command("F8", CMD_TALK),
        Binding::command("F9", CMD_CALL),
    ];
    settings.sync_hotkey_mirrors();
    let saved = saved_json(&settings);
    assert_eq!(
        saved["hotkeys"]["bindings"],
        json!([
            {"key": "F8", "command": CMD_TALK},
            {"key": "F9", "command": CMD_CALL}
        ])
    );
    assert_eq!(saved["hotkey"]["key"], "F8");
    assert_eq!(saved["assistant"]["hotkey"], "F9");
    // 没绑的写空串：旧版本读到缺字段会用它自己的默认补上（例如 rc.8 会把频道切换补成 Shift+CapsLock）
    assert_eq!(saved["automationStopHotkey"], "");
    assert_eq!(saved["runtimeToggleHotkey"], "");
    let voice = &saved["plugins"]["voice-input"]["settings"];
    for field in [
        "channelCycleHotkey",
        "resendLastHotkey",
        "togglePostProcessingHotkey",
        "cyclePostProcessingHotkey",
    ] {
        assert_eq!(voice[field], "", "{field}");
    }
}

#[test]
fn an_old_version_in_between_keeps_the_keys_it_changed() {
    // 新版存 → 装回旧版本：旧版本不认识 hotkeys，保存时丢掉；在旧版本里改了说话键和频道切换
    let (settings, _dir) = load_json(full_old_config());
    let mut on_disk = saved_json(&settings);
    on_disk.as_object_mut().unwrap().remove("hotkeys");
    on_disk["hotkey"]["key"] = json!("F7");
    on_disk["plugins"]["voice-input"]["settings"]["channelCycleHotkey"] = json!("Alt+C");
    // → 再装回新版本：从旧字段重新造，拿到的是在旧版本里改的键，别的照旧
    let (back, _dir) = load_json(on_disk);
    assert_eq!(back.binding_key(CMD_TALK), "F7");
    assert_eq!(back.binding_key(CMD_CHANNEL_CYCLE), "Alt+C");
    assert_eq!(back.binding_key(CMD_CALL), "Ctrl+F9");
    assert_eq!(back.binding_key(CMD_PP_CYCLE), "Alt+O");
    // 新版存、新版读：一模一样
    let (again, _dir) = load_json(saved_json(&back));
    assert_eq!(table(&again), table(&back));
}

#[test]
fn entries_this_version_does_not_understand_are_written_back_as_they_were() {
    let entries = json!([
        {"key": "F8", "command": CMD_TALK, "when": "later"},
        {"key": "F6", "page": "ui://kotone/lol/tier"},
        {"key": "F7", "tool": "game_chat.send", "args": {"text": "集合打龙", "channel": "team"}},
        {"key": "F5", "command": "someone.new-thing"},
        "not an object",
        42
    ]);
    let mut config = full_old_config();
    config["hotkeys"] = json!({"bindings": entries.clone(), "future": true});
    let (settings, _dir) = load_json(config);
    let tool = &settings.hotkeys.bindings[2];
    assert_eq!(
        (tool.key(), tool.tool(), tool.command_name()),
        ("F7", Some("game_chat.send"), None)
    );
    assert_eq!(tool.args().unwrap()["channel"], "team");
    assert_eq!(
        settings.hotkeys.bindings[1].page(),
        Some("ui://kotone/lol/tier")
    );
    assert_eq!(settings.hotkey.key, "F8");
    let saved = saved_json(&settings);
    assert_eq!(saved["hotkeys"]["bindings"], entries);
    assert_eq!(saved["hotkeys"]["future"], true);
}

#[test]
fn set_binding_changes_one_entry_and_leaves_the_rest() {
    let mut settings = Settings::default();
    settings.hotkeys.bindings = vec![
        Binding(json!({"key": "F8", "command": CMD_TALK, "when": "later"})),
        Binding(json!({"key": "F7", "tool": "game_chat.send"})),
        Binding::command("F9", CMD_TALK),
    ];
    settings.set_binding(CMD_TALK, " F3 ");
    assert_eq!(
        settings.hotkeys.bindings[0].0,
        json!({"key": "F3", "command": CMD_TALK, "when": "later"})
    );
    assert_eq!(
        settings.hotkeys.bindings[2].key(),
        "F9",
        "同一个命令的第二条不动"
    );
    settings.set_binding(CMD_CHANNEL_CYCLE, "Alt+C");
    assert_eq!(
        settings.hotkeys.bindings.last().unwrap().0,
        json!({"key": "Alt+C", "command": CMD_CHANNEL_CYCLE})
    );
    settings.set_binding(CMD_TALK, "");
    assert_eq!(
        settings.binding_key(CMD_TALK),
        "",
        "空 = 这个命令的条目都删掉"
    );
    assert_eq!(settings.hotkeys.bindings.len(), 2, "工具那条和频道切换还在");
}

fn repository(settings: Settings) -> (SettingsRepository, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let repo = SettingsRepository::new_at(
        Arc::new(RwLock::new(settings)),
        dir.path().join("config.json"),
    );
    (repo, dir)
}

#[test]
fn old_style_writes_change_the_table() {
    let (repo, _dir) = repository(Settings::default());
    // 旧写法：改说话键、清掉停下热键、设呼叫键、设一个新的频道切换
    let next = repo
        .update(|s| {
            s.hotkey.key = "F8".into();
            s.automation_stop_hotkey = String::new();
            s.assistant.hotkey = "Ctrl+F9".into();
            s.channel_cycle_hotkey = "Alt+C".into();
            Ok(())
        })
        .unwrap();
    assert_eq!(
        table(&next),
        pairs(&[
            (CMD_CALL, "Ctrl+F9"),
            (CMD_TALK, "F8"),
            (CMD_CHANNEL_CYCLE, "Alt+C")
        ])
    );
    assert_eq!(table(&repo.snapshot()), table(&next));
    // 别的设置改了、热键没动：表不变
    let same = repo
        .update(|s| {
            s.language = "en".into();
            Ok(())
        })
        .unwrap();
    assert_eq!(table(&same), table(&next));
}

#[test]
fn a_patch_that_sets_the_table_wins_over_old_fields_in_the_same_patch() {
    let (repo, _dir) = repository(Settings::default());
    let next = repo
        .update(|s| {
            s.hotkeys.bindings = vec![Binding::command("F1", CMD_TALK)];
            s.hotkey.key = "F2".into();
            Ok(())
        })
        .unwrap();
    assert_eq!(table(&next), pairs(&[(CMD_TALK, "F1")]));
    assert_eq!(next.hotkey.key, "F1");
    assert_eq!(next.call_hotkey(), "", "表里没有呼叫键了，镜像跟着清");
}

/// `settings.update` / 界面的补丁（桌面端 `update_settings`、命令行 `config set` 都是“序列化 → 合并补丁 → 反序列化”）。
fn patch(current: &Settings, patch: Value) -> Settings {
    let mut merged = serde_json::to_value(current).unwrap();
    merge_json(&mut merged, &patch);
    let mut next: Settings = serde_json::from_value(merged).unwrap();
    next.reconcile_hotkeys(current);
    next
}

#[test]
fn settings_update_accepts_the_old_field_names() {
    let current = Settings::default();
    let next = patch(&current, json!({"hotkey": {"key": "F8"}}));
    assert_eq!(next.binding_key(CMD_TALK), "F8");
    let next = patch(
        &next,
        json!({"channelCycleHotkey": "Alt+C", "assistant": {"hotkey": ""}}),
    );
    assert_eq!(next.binding_key(CMD_CHANNEL_CYCLE), "Alt+C");
    assert_eq!(next.binding_key(CMD_CALL), "");
    // 新写法：整组替换
    let next = patch(
        &next,
        json!({"hotkeys": {"bindings": [{"key": "F4", "command": CMD_STOP}]}}),
    );
    assert_eq!(table(&next), pairs(&[(CMD_STOP, "F4")]));
    assert_eq!(next.automation_stop_hotkey, "F4");
    assert_eq!(next.hotkey.key, "");
    // 调两次和调一次一样
    let mut twice = next.clone();
    twice.reconcile_hotkeys(&next);
    assert_eq!(table(&twice), table(&next));
}

#[test]
fn opting_in_starts_without_a_call_key_until_one_is_set() {
    // 0.1.x 用户在引导里启用她（deepseekSetup.ts：companionBindings + legacyUpgrade 一起写）
    let (pending, _dir) = load_json(json!({"assistant": {"hotkey": "Alt+F9"}}));
    let opted = patch(
        &pending,
        json!({
            "assistant": {"companionBindings": {"connectionId": "c1", "hotkey": ""}},
            "ui": {"legacyUpgrade": "optedIn"}
        }),
    );
    assert_eq!(opted.binding_key(CMD_CALL), "");
    assert_eq!(opted.assistant.hotkey, "Alt+F9");
    // 然后在引导里设呼叫键
    let with_key = patch(
        &opted,
        json!({"assistant": {"companionBindings": {"hotkey": "Ctrl+Alt+F10"}}}),
    );
    assert_eq!(with_key.binding_key(CMD_CALL), "Ctrl+Alt+F10");
    // 改提醒键（assistant.hotkey）不碰呼叫键
    let reminder = patch(&with_key, json!({"assistant": {"hotkey": "Alt+F8"}}));
    assert_eq!(reminder.binding_key(CMD_CALL), "Ctrl+Alt+F10");
    assert_eq!(reminder.assistant.hotkey, "Alt+F8");
}

#[test]
fn every_old_field_has_one_entry_in_the_legacy_table() {
    let commands: Vec<&str> = LEGACY.iter().map(|l| l.command).collect();
    let mut unique = commands.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), commands.len(), "命令不重名");
    // 每个命令都有镜像（新用户），改镜像会改表
    let base = Settings::default();
    for legacy in LEGACY {
        assert!(
            mirror(&base, legacy.command).is_some(),
            "{}",
            legacy.command
        );
        let mut next = base.clone();
        set_mirror(&mut next, legacy.command, "F13".into());
        next.reconcile_hotkeys(&base);
        assert_eq!(
            next.binding_key(legacy.command),
            "F13",
            "{}",
            legacy.command
        );
    }
    // 默认表里的命令都在对照表里
    for (command, _) in DEFAULT_BINDINGS {
        assert!(commands.contains(command));
    }
}

#[test]
fn code_that_writes_an_old_field_and_saves_directly_keeps_the_change() {
    // 没经过 SettingsRepository（例如改完直接 save_to）：保存时照旧写法算进表，读回来还在
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    let mut settings = Settings::default();
    settings.hotkey.key = "Alt+V".into();
    settings.channel_cycle_hotkey = "Alt+C".into();
    save_to(&path, &settings).unwrap();
    let loaded = load_from(&path);
    assert_eq!(loaded.binding_key(CMD_TALK), "Alt+V");
    assert_eq!(loaded.binding_key(CMD_CHANNEL_CYCLE), "Alt+C");
    // 对得上时原样存
    assert!(with_direct_edits(&loaded).is_none());
}
