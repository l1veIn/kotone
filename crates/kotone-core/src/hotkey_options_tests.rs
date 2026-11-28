//! ADR 054 的边界：五种方式怎么读写、自动的点一下 / 按住、她的按住说话的按下 / 松开、“什么时候管用”的门和冲突、
//! 一个命令几个键、旧配置照旧读。

use std::sync::atomic::AtomicU64;
use std::time::Duration;

use super::*;
use crate::hotkey::{parse_hotkey, HookEvent, HookMatcher, HotkeyMode, KeyAction, HK_ASSISTANT};
use crate::hotkey_commands::{CommandKind, CommandSpec};
use crate::hotkey_table::{self, PluginInfo, Sources};
use crate::profile::GameProfile;

fn talk_plugin() -> &'static [PluginInfo<'static>] {
    static COMMANDS: std::sync::OnceLock<Vec<CommandSpec>> = std::sync::OnceLock::new();
    static PLUGINS: std::sync::OnceLock<Vec<PluginInfo<'static>>> = std::sync::OnceLock::new();
    let commands = COMMANDS.get_or_init(|| {
        vec![CommandSpec {
            id: "talk".into(),
            name: "语音输入热键".into(),
            description: String::new(),
            kind: CommandKind::Hold,
            listens: true,
            tool: None,
            args: None,
        }]
    });
    PLUGINS.get_or_init(|| {
        vec![PluginInfo {
            id: "voice-input",
            name: "语音输入",
            enabled: true,
            commands,
        }]
    })
}

fn sources<'a>(settings: &'a Settings, profiles: &'a [GameProfile]) -> Sources<'a> {
    Sources {
        settings,
        profiles,
        plugins: talk_plugin(),
        custom: None,
    }
}

fn plan(settings: &Settings) -> Vec<crate::hotkey_table::Planned> {
    hotkey_table::registration_plan(&sources(settings, &[]))
}

/// 一个“最前面是哪个游戏”的原子量，每个测试自己一份（不碰全局那一个）。
fn foreground() -> &'static AtomicU64 {
    Box::leak(Box::new(AtomicU64::new(0)))
}

#[test]
fn five_ways_read_and_write_with_the_old_values() {
    for mode in VoiceKeyMode::ALL {
        assert_eq!(VoiceKeyMode::parse(mode.as_str()), Some(mode));
        assert_eq!(VoiceKeyMode::from_call(mode.to_call()), mode);
        if let Some(interaction) = mode.to_interaction() {
            assert_eq!(VoiceKeyMode::from_interaction(interaction), mode);
        }
    }
    // 两个全局字段的写法也认
    assert_eq!(
        VoiceKeyMode::parse("push-to-talk"),
        Some(VoiceKeyMode::Hold)
    );
    assert_eq!(VoiceKeyMode::parse("solo"), Some(VoiceKeyMode::Continuous));
    assert_eq!(VoiceKeyMode::parse("call"), Some(VoiceKeyMode::Continuous));
    assert_eq!(VoiceKeyMode::parse("walkie"), None);
    // 说话的“自动”写不进 interactionMode（旧版本读到不认识的值会把整份配置当坏的）
    assert_eq!(VoiceKeyMode::Auto.to_interaction(), None);
    let labels: Vec<&str> = VoiceKeyMode::ALL.iter().map(|m| m.label()).collect();
    assert_eq!(
        labels,
        vec!["自动", "按住说话", "说一句就走", "录音笔", "一直听"]
    );
    assert!(VoiceKeyMode::Auto.uses_release() && VoiceKeyMode::Hold.uses_release());
    assert!(!VoiceKeyMode::OneShot.uses_release() && !VoiceKeyMode::Continuous.uses_release());
}

#[test]
fn auto_tap_is_one_shot_and_a_long_hold_is_hold_to_talk() {
    let threshold = DEFAULT_HOLD_MS;
    assert!(
        !auto_release_ends(Duration::from_millis(80), threshold),
        "点一下"
    );
    assert!(!auto_release_ends(Duration::from_millis(299), threshold));
    assert!(
        auto_release_ends(Duration::from_millis(300), threshold),
        "按住"
    );
    assert!(auto_release_ends(Duration::from_secs(4), threshold));
    // 门槛是设置，夹在范围里
    let mut settings = Settings::default();
    assert_eq!(settings.hold_ms(), 300);
    settings.hotkeys.hold_ms = 10;
    assert_eq!(settings.hold_ms(), *HOLD_MS_RANGE.start());
    settings.hotkeys.hold_ms = 99_999;
    assert_eq!(settings.hold_ms(), *HOLD_MS_RANGE.end());
}

#[test]
fn when_parses_overlaps_and_opens_only_in_the_right_game() {
    assert_eq!(KeyWhen::parse(None), KeyWhen::Always);
    assert_eq!(KeyWhen::parse(Some("always")), KeyWhen::Always);
    assert_eq!(KeyWhen::parse(Some("game")), KeyWhen::Game);
    assert_eq!(
        KeyWhen::parse(Some("game:lol")),
        KeyWhen::Profile("lol".into())
    );
    // 她直接写游戏配置 id 也认
    assert_eq!(KeyWhen::parse(Some("lol")), KeyWhen::Profile("lol".into()));
    assert_eq!(KeyWhen::Profile("lol".into()).as_string(), "game:lol");
    assert_eq!(KeyWhen::Always.to_value(), None, "一直 = 不写");

    let lol = KeyWhen::Profile("lol".into());
    let dota = KeyWhen::Profile("dota2".into());
    assert!(KeyWhen::Always.overlaps(&lol) && KeyWhen::Game.overlaps(&lol));
    assert!(KeyWhen::Game.overlaps(&KeyWhen::Game));
    assert!(!lol.overlaps(&dota), "两个不同的游戏不同时管用");

    let fg_lol = game_hash("lol");
    assert_ne!(fg_lol, 0);
    assert!(KeyGate::Always.open(0));
    assert!(!KeyGate::AnyGame.open(0), "不在游戏里");
    assert!(KeyGate::AnyGame.open(fg_lol));
    assert!(lol.gate().open(fg_lol));
    assert!(!dota.gate().open(fg_lol));
}

#[test]
fn a_key_only_for_games_passes_through_outside_and_fires_inside() {
    let fg = foreground();
    let side = parse_hotkey("Mouse5").unwrap();
    let mut m = HookMatcher::new(parse_hotkey("F24").unwrap(), HotkeyMode::Toggle)
        .with_foreground_source(fg);
    m.set_enabled(false);
    m.set_named_spec_gated(HK_ASSISTANT, Some(side), HotkeyMode::Hold, KeyGate::AnyGame);
    // 桌面上：侧键原样给前台程序（浏览器的“后退”），不吞、不触发
    let down = m.on_key(side.vk, KeyAction::Down);
    assert!(!down.swallow && down.event.is_none());
    let up = m.on_key(side.vk, KeyAction::Up);
    assert!(!up.swallow && up.event.is_none());
    // 游戏在最前面：吞掉，按下 / 松开都报（她的按住说话）
    fg.store(game_hash("lol"), std::sync::atomic::Ordering::Relaxed);
    let down = m.on_key(side.vk, KeyAction::Down);
    assert!(down.swallow);
    assert_eq!(
        down.event,
        Some(HookEvent::NamedHold {
            id: HK_ASSISTANT.into(),
            pressed: true
        })
    );
    // 按着的时候切出了游戏：这一次的松开照样归它（不会只按下不松开）
    fg.store(0, std::sync::atomic::Ordering::Relaxed);
    let up = m.on_key(side.vk, KeyAction::Up);
    assert!(up.swallow);
    assert_eq!(
        up.event,
        Some(HookEvent::NamedHold {
            id: HK_ASSISTANT.into(),
            pressed: false
        })
    );
}

#[test]
fn the_talk_key_only_for_one_game_passes_through_in_another() {
    let fg = foreground();
    let caps = parse_hotkey("CapsLock").unwrap();
    let mut m = HookMatcher::new(caps, HotkeyMode::Hold).with_foreground_source(fg);
    m.set_main_gate(KeyWhen::Profile("lol".into()).gate());
    fg.store(game_hash("dota2"), std::sync::atomic::Ordering::Relaxed);
    let down = m.on_key(caps.vk, KeyAction::Down);
    assert!(
        !down.swallow && down.event.is_none(),
        "别的游戏里照常是大写锁定"
    );
    assert!(!m.on_key(caps.vk, KeyAction::Up).swallow);
    fg.store(game_hash("lol"), std::sync::atomic::Ordering::Relaxed);
    let down = m.on_key(caps.vk, KeyAction::Down);
    assert!(down.swallow);
    assert_eq!(down.event, Some(HookEvent::HoldPressed));
    assert_eq!(
        m.on_key(caps.vk, KeyAction::Up).event,
        Some(HookEvent::HoldReleased)
    );
}

#[test]
fn two_games_can_share_a_key_and_each_fires_in_its_own_game() {
    let fg = foreground();
    let f5 = parse_hotkey("F5").unwrap();
    let mut m = HookMatcher::new(parse_hotkey("F24").unwrap(), HotkeyMode::Toggle)
        .with_foreground_source(fg);
    m.set_enabled(false);
    m.set_named_spec_gated(
        "a",
        Some(f5),
        HotkeyMode::Toggle,
        KeyWhen::Profile("lol".into()).gate(),
    );
    m.set_named_spec_gated(
        "b",
        Some(f5),
        HotkeyMode::Toggle,
        KeyWhen::Profile("dota2".into()).gate(),
    );
    fg.store(game_hash("dota2"), std::sync::atomic::Ordering::Relaxed);
    assert_eq!(
        m.on_key(f5.vk, KeyAction::Down).event,
        Some(HookEvent::NamedHotkey { id: "b".into() })
    );
    m.on_key(f5.vk, KeyAction::Up);
    fg.store(game_hash("lol"), std::sync::atomic::Ordering::Relaxed);
    assert_eq!(
        m.on_key(f5.vk, KeyAction::Down).event,
        Some(HookEvent::NamedHotkey { id: "a".into() })
    );
}

#[test]
fn her_call_key_registers_as_hold_for_auto_and_hold_to_talk() {
    let mut settings = Settings::default();
    // 新装：自动 → 按下、松开都要
    assert_eq!(
        settings.assistant.call_mode,
        crate::assistant::CallMode::Auto
    );
    let call = |s: &Settings| plan(s).into_iter().find(|p| p.id == HK_ASSISTANT).unwrap();
    assert!(call(&settings).hold);
    settings.assistant.call_mode = crate::assistant::CallMode::Hold;
    assert!(call(&settings).hold);
    // 按一下的三种：只要按下
    for mode in [
        crate::assistant::CallMode::OneShot,
        crate::assistant::CallMode::Dictation,
        crate::assistant::CallMode::Call,
    ] {
        settings.assistant.call_mode = mode;
        assert!(!call(&settings).hold, "{mode:?}");
    }
    // 那一条写了自己的方式：照它
    settings
        .set_voice_key_mode(CMD_CALL, 1, VoiceKeyMode::Hold)
        .unwrap();
    assert!(call(&settings).hold);
}

#[test]
fn a_command_can_have_several_keys_each_with_its_own_way_and_when() {
    let mut settings = Settings::default();
    settings.assistant.call_mode = crate::assistant::CallMode::OneShot;
    settings.add_command_binding(CMD_CALL, "Mouse5", Some(VoiceKeyMode::Hold), &KeyWhen::Game);
    settings.sync_hotkey_mirrors();
    // 第一个键还是镜像
    assert_eq!(
        settings.assistant.hotkey,
        crate::assistant::DEFAULT_CALL_HOTKEY
    );
    assert_eq!(
        settings.voice_key_mode(CMD_CALL, 1),
        Some(VoiceKeyMode::OneShot)
    );
    assert_eq!(
        settings.voice_key_mode(CMD_CALL, 2),
        Some(VoiceKeyMode::Hold)
    );
    let planned = plan(&settings);
    let second = planned
        .iter()
        .find(|p| p.id == "assistant~2")
        .expect("第二个呼叫键登记了");
    assert!(second.error.is_none(), "{:?}", second.error);
    assert!(second.hold);
    assert_eq!(second.when, KeyWhen::Game);
    assert_eq!(split_nth(&second.id), (HK_ASSISTANT, 2));
    // 表里每个键一行
    let table = hotkey_table::table(&sources(&settings, &[]));
    let row = table
        .bindings
        .iter()
        .find(|r| r.id == "kotone.call~2")
        .unwrap();
    assert_eq!(
        (row.key.as_str(), row.mode, row.nth),
        ("Mouse5", Some("hold"), 2)
    );
    assert!(row.own_mode && row.default_key.is_empty());
    assert_eq!(row.status_id, "assistant~2");
    let first = table
        .bindings
        .iter()
        .find(|r| r.id == "kotone.call")
        .unwrap();
    assert_eq!(first.mode, Some("one-shot"));
    assert!(!first.own_mode);
    // 删掉第二个
    settings.remove_command_binding(CMD_CALL, 2).unwrap();
    assert!(plan(&settings).iter().all(|p| p.id != "assistant~2"));
}

#[test]
fn same_key_conflicts_only_when_both_work_at_the_same_time() {
    let lol = GameProfile::builtin_lol();
    let dota = GameProfile::builtin_dota2();
    let profiles = vec![lol.clone(), dota.clone()];
    let only = |id: &str| KeyWhen::Profile(id.to_string());
    let mut before = Settings::default();
    before.set_binding(CMD_CALL, "F7");
    before.sync_hotkey_mirrors();

    // 一直管用的呼叫键 F7 + 只在游戏里的 F7：会同时管用，冲突
    let mut after = before.clone();
    after.add_command_binding(crate::hotkey_bindings::CMD_STOP, "F7", None, &KeyWhen::Game);
    after.sync_hotkey_mirrors();
    let error =
        hotkey_table::new_conflict_error(&sources(&before, &profiles), &sources(&after, &profiles));
    assert!(error.is_some_and(|e| e.contains("F7")));

    // 只在英雄联盟里 + 只在 Dota 2 里：不冲突，两个都登记
    let mut before = Settings::default();
    before.set_binding(CMD_CALL, "");
    before.add_command_binding(CMD_CALL, "F7", None, &only(&lol.id));
    before.sync_hotkey_mirrors();
    let mut after = before.clone();
    after.add_command_binding(CMD_TALK, "F7", None, &only(&dota.id));
    after.sync_hotkey_mirrors();
    assert_eq!(
        hotkey_table::new_conflict_error(&sources(&before, &profiles), &sources(&after, &profiles)),
        None
    );
    let planned = plan(&after);
    assert!(planned
        .iter()
        .filter(|p| p.combo == "F7")
        .all(|p| p.error.is_none()));
    assert_eq!(planned.iter().filter(|p| p.combo == "F7").count(), 2);

    // 游戏的聊天键只在那个游戏里按：只在 Dota 2 里的 Enter 不和英雄联盟的 Enter 冲突
    let src = sources(&after, &profiles);
    let in_dota = hotkey_table::check_at(&src, "Enter", &only(&dota.id), None);
    assert!(
        in_dota
            .conflicts
            .iter()
            .all(|c| c.owner.as_deref() != Some(lol.id.as_str())),
        "{:?}",
        in_dota.conflicts
    );
    let always = hotkey_table::check_at(&src, "Enter", &KeyWhen::Always, None);
    assert!(always
        .conflicts
        .iter()
        .any(|c| c.owner.as_deref() == Some(lol.id.as_str())));
    // Esc 不管什么时候都不让用
    let esc = hotkey_table::check_at(&src, "Escape", &only(&lol.id), None);
    assert!(esc
        .conflicts
        .iter()
        .any(|c| c.id == hotkey_table::KEY_ESCAPE));
}

#[test]
fn first_key_follows_the_pages_and_later_keys_keep_their_own_way() {
    let mut settings = Settings::default();
    // 「设置 → 热键」上改第一个呼叫键的方式：写进 assistant.callMode（她的「唤醒」页显示的就是它）
    settings
        .set_voice_key_mode(CMD_CALL, 1, VoiceKeyMode::Hold)
        .unwrap();
    assert_eq!(
        settings.assistant.call_mode,
        crate::assistant::CallMode::Hold
    );
    assert_eq!(settings.binding_mode(CMD_CALL, 1), None);
    // 说话的“自动”写不进 interactionMode：写在那一条上
    settings
        .set_voice_key_mode(CMD_TALK, 1, VoiceKeyMode::Auto)
        .unwrap();
    assert_eq!(
        settings.interaction_mode,
        Some(crate::interaction::InteractionMode::PushToTalk),
        "全局不动"
    );
    assert_eq!(settings.binding_mode(CMD_TALK, 1), Some(VoiceKeyMode::Auto));
    assert_eq!(
        crate::interaction::effective_hotkey_mode(&settings),
        HotkeyMode::Hold,
        "自动要松开那一下"
    );
    // 一直听：旧配置的“不通话”（0 秒）换成缺省的 30 秒
    settings.assistant.call_idle_seconds = 0;
    settings
        .set_voice_key_mode(CMD_CALL, 1, VoiceKeyMode::Continuous)
        .unwrap();
    assert_eq!(settings.assistant.call_idle_seconds, 30);

    // 语音输入页又选了录音笔（这次修改没动热键表）：第一个键上的“自动”去掉，那边选的就是它
    let previous = settings.clone();
    settings.interaction_mode = Some(crate::interaction::InteractionMode::Dictation);
    settings.reconcile_hotkeys(&previous);
    assert_eq!(settings.binding_mode(CMD_TALK, 1), None);
    assert_eq!(
        settings.voice_key_mode(CMD_TALK, 1),
        Some(VoiceKeyMode::Dictation)
    );

    // 第二个键带着自己的方式，页面上改全局不动它
    settings.add_command_binding(CMD_CALL, "Mouse4", Some(VoiceKeyMode::Hold), &KeyWhen::Game);
    let previous = settings.clone();
    settings.assistant.call_mode = crate::assistant::CallMode::OneShot;
    settings.reconcile_hotkeys(&previous);
    assert_eq!(settings.binding_mode(CMD_CALL, 2), Some(VoiceKeyMode::Hold));
    assert_eq!(
        settings.voice_key_mode(CMD_CALL, 1),
        Some(VoiceKeyMode::OneShot)
    );
    assert!(!settings.call_continuous_anywhere());
    settings
        .set_voice_key_mode(CMD_CALL, 2, VoiceKeyMode::Continuous)
        .unwrap();
    assert!(settings.call_continuous_anywhere());
    // 别的命令没有方式
    assert!(settings
        .set_voice_key_mode(crate::hotkey_bindings::CMD_STOP, 1, VoiceKeyMode::Hold)
        .is_err());
}

#[test]
fn a_wrong_way_is_refused_in_plain_words() {
    let mut settings = Settings::default();
    assert_eq!(binding_option_error(&settings), None);
    settings.hotkeys.bindings.push(Binding(
        serde_json::json!({"key": "F6", "command": CMD_CALL, "mode": "walkie"}),
    ));
    let error = binding_option_error(&settings).unwrap();
    assert!(
        error.contains("walkie") && error.contains("hold"),
        "{error}"
    );
}

#[test]
fn old_tables_and_old_configs_load_unchanged() {
    // 旧的一条（只有 key + command）：没有自己的方式、一直管用；读写不多出字段
    let json = serde_json::json!({"key": "CapsLock", "command": CMD_TALK});
    let binding: Binding = serde_json::from_value(json.clone()).unwrap();
    assert_eq!(binding.mode(), None);
    assert_eq!(binding.when(), KeyWhen::Always);
    assert_eq!(serde_json::to_value(&binding).unwrap(), json);
    let mut copy = binding.clone();
    copy.set_when(&KeyWhen::Always);
    copy.set_mode(None);
    assert_eq!(copy, binding, "一直 / 没方式 = 不写");
    // 旧的 hotkeys 没有 holdMs：缺省 300
    let hotkeys: crate::hotkey_bindings::HotkeysConfig =
        serde_json::from_value(serde_json::json!({"bindings": [json]})).unwrap();
    assert_eq!(hotkeys.hold_ms, DEFAULT_HOLD_MS);

    // 旧配置文件没有 callMode：照旧说一句就走（只有新装是自动）；写过的不动
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    std::fs::write(
        &path,
        r#"{"assistant":{"hotkey":"F9","callHotkeyDefaulted":true}}"#,
    )
    .unwrap();
    let old = crate::settings::load_from(&path);
    assert_eq!(old.assistant.call_mode, crate::assistant::CallMode::OneShot);
    assert_eq!(old.voice_key_mode(CMD_CALL, 1), Some(VoiceKeyMode::OneShot));
    std::fs::write(
        &path,
        r#"{"assistant":{"callMode":"dictation","callHotkeyDefaulted":true}}"#,
    )
    .unwrap();
    assert_eq!(
        crate::settings::load_from(&path).assistant.call_mode,
        crate::assistant::CallMode::Dictation
    );
    // 新装（没有文件）是自动
    let fresh = crate::settings::load_from(&dir.path().join("none.json"));
    assert_eq!(fresh.assistant.call_mode, crate::assistant::CallMode::Auto);
    assert!(!fresh.ui.call_key_chosen);
    // 新版本写的 auto / hold，旧版本按说一句就走读（serde other），不当坏配置
    let future: crate::assistant::AssistantConfig =
        serde_json::from_str(r#"{"callMode":"something-newer"}"#).unwrap();
    assert_eq!(future.call_mode, crate::assistant::CallMode::OneShot);
}

#[test]
fn nth_ids_roundtrip() {
    assert_eq!(nth_id("assistant", 1), "assistant");
    assert_eq!(nth_id("assistant", 2), "assistant~2");
    assert_eq!(split_nth("assistant~2"), ("assistant", 2));
    assert_eq!(split_nth("record"), ("record", 1));
    assert_eq!(split_nth("talk~3"), ("talk", 3));
    // 不是数字、是 1 的后缀不拆
    assert_eq!(split_nth("custom.a~b"), ("custom.a~b", 1));
    assert_eq!(split_nth("x~1"), ("x~1", 1));
}
