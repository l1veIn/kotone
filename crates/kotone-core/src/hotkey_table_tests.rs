//! 热键表的边界（ADR 040 D6）：每一类冲突一条，加上不冲突的、只列用得上的、和设置说明一致。

use super::*;
use crate::assistant::QuickMessage;
use crate::profile::ConsoleChat;
use crate::settings::LegacyUpgrade;

const VOICE: PluginInfo<'static> = PluginInfo {
    id: "voice-input",
    name: "语音输入",
    enabled: true,
};

fn profiles() -> Vec<GameProfile> {
    let mut dota = GameProfile::builtin_dota2();
    dota.console_chat = Some(ConsoleChat {
        cfg_dir: "game/dota/cfg".into(),
        trigger_key: "F23".into(),
    });
    vec![
        GameProfile::builtin_generic(),
        GameProfile::builtin_lol(),
        dota,
        GameProfile::builtin_cs2(),
    ]
}

fn check_ids(settings: &Settings, key: &str, except: Option<&str>) -> Vec<String> {
    let profiles = profiles();
    let src = Sources {
        settings,
        profiles: &profiles,
        voice_input: VOICE,
    };
    check(&src, key, except)
        .conflicts
        .into_iter()
        .map(|c| c.id)
        .collect()
}

fn table_of(settings: &Settings) -> HotkeyTable {
    let profiles = profiles();
    table(&Sources {
        settings,
        profiles: &profiles,
        voice_input: VOICE,
    })
}

fn row<'a>(table: &'a HotkeyTable, id: &str) -> &'a KeyUse {
    table
        .bindings
        .iter()
        .find(|b| b.id == id)
        .unwrap_or_else(|| panic!("{id} 不在表里"))
}

fn quick(hotkey: &str, text: &str) -> QuickMessage {
    QuickMessage {
        id: format!("q-{text}"),
        text: text.into(),
        hotkey: hotkey.into(),
        ..Default::default()
    }
}

#[test]
fn defaults_have_no_conflicts() {
    let settings = Settings::default();
    let table = table_of(&settings);
    for b in &table.bindings {
        assert!(
            b.conflicts.is_empty(),
            "{} ({}) 默认就冲突：{:?}",
            b.id,
            b.key,
            b.conflicts
        );
    }
    // 呼叫键改成没人用的键、或者原样保存：不冲突（不和自己比）
    assert!(check_ids(&settings, "Ctrl+F9", Some(HK_ASSISTANT)).is_empty());
    assert!(check_ids(&settings, settings.call_hotkey(), Some(HK_ASSISTANT)).is_empty());
    // 修饰键不同就不算（CapsLock 和 Shift+CapsLock 是两个键；Ctrl+Enter 不是游戏的 Enter）
    assert_eq!(
        check_ids(&settings, "Shift+CapsLock", Some(HK_CHANNEL_CYCLE)),
        Vec::<String>::new()
    );
    assert!(check_ids(&settings, "Ctrl+Enter", Some(HK_ASSISTANT)).is_empty());
    // 认不出的键不算冲突（保存时另外报错）
    assert!(check_ids(&settings, "NotAKey", Some(HK_ASSISTANT)).is_empty());
}

#[test]
fn core_against_core() {
    let settings = Settings {
        automation_stop_hotkey: "Ctrl+Backquote".into(),
        ..Default::default()
    };
    assert_eq!(
        check_ids(&settings, "ctrl + ~", Some(HK_ASSISTANT)),
        vec![HK_AUTOMATION_STOP]
    );
    let table = table_of(&settings);
    assert_eq!(
        row(&table, HK_ASSISTANT).conflicts,
        vec![HK_AUTOMATION_STOP]
    );
    assert_eq!(
        row(&table, HK_AUTOMATION_STOP).conflicts,
        vec![HK_ASSISTANT]
    );
}

#[test]
fn core_against_plugin() {
    let settings = Settings::default();
    // 语音输入热键默认 CapsLock、切换频道默认 Shift+CapsLock
    assert_eq!(
        check_ids(&settings, "CapsLock", Some(HK_ASSISTANT)),
        vec![HK_RECORD]
    );
    assert_eq!(
        check_ids(&settings, "Shift+CapsLock", Some(HK_AUTOMATION_STOP)),
        vec![HK_CHANNEL_CYCLE]
    );
    // 插件关着也算：键还在设置里，打开插件就会撞上
    let profiles = profiles();
    let off = PluginInfo {
        enabled: false,
        ..VOICE
    };
    let src = Sources {
        settings: &settings,
        profiles: &profiles,
        voice_input: off,
    };
    let conflicts = check(&src, "CapsLock", Some(HK_ASSISTANT)).conflicts;
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].source, KeySource::Plugin);
    assert_eq!(conflicts[0].owner.as_deref(), Some("voice-input"));
    assert!(!conflicts[0].enabled);
}

#[test]
fn against_quick_messages() {
    let mut settings = Settings::default();
    settings.assistant.companion.quick_messages =
        vec![quick("", "不绑键的"), quick("F8", "集合打龙")];
    assert_eq!(
        check_ids(&settings, "F8", Some(HK_ASSISTANT)),
        vec![HK_QUICK_MESSAGES[1]]
    );
    // 两条快捷指令同一个键：互相冲突
    settings.assistant.companion.quick_messages[0].hotkey = "F8".into();
    let table = table_of(&settings);
    assert_eq!(
        row(&table, HK_QUICK_MESSAGES[0]).conflicts,
        vec![HK_QUICK_MESSAGES[1]]
    );
    assert_eq!(row(&table, HK_QUICK_MESSAGES[1]).source, KeySource::Custom);
    assert_eq!(row(&table, HK_QUICK_MESSAGES[1]).detail, "集合打龙");
    assert_eq!(
        row(&table, HK_QUICK_MESSAGES[1]).field,
        "assistant.companion.quickMessages.1.hotkey"
    );
}

#[test]
fn against_game_chat_keys() {
    let settings = Settings::default();
    // 正在用的游戏配置（英雄联盟）排在最前；英雄联盟的开框和发送都是 Enter，只算一条
    let enter = check_ids(&settings, "Enter", Some(HK_ASSISTANT));
    assert_eq!(
        enter,
        vec![
            "game.lol.open-chat",
            "game.generic.open-chat",
            "game.dota2.open-chat",
            "game.cs2.send"
        ]
    );
    assert_eq!(
        check_ids(&settings, "Shift+Enter", Some(HK_ASSISTANT)),
        vec!["game.lol.channel.all", "game.dota2.channel.all"]
    );
    // CS2 的队伍聊天是 U、所有人是 Y
    assert_eq!(
        check_ids(&settings, "Y", Some(HK_RECORD)),
        vec!["game.cs2.channel.all"]
    );
    let profiles = profiles();
    let src = Sources {
        settings: &settings,
        profiles: &profiles,
        voice_input: VOICE,
    };
    let lol = &check(&src, "Enter", None).conflicts[0];
    assert_eq!(
        (lol.source, lol.game_key),
        (KeySource::Game, Some(GameKey::OpenChat))
    );
    assert_eq!(lol.owner_name.as_deref(), Some("League of Legends"));
    // 游戏的键不进表，但表里的热键撞上它时写着
    let mut bad = Settings::default();
    bad.assistant.hotkey = "Enter".into();
    let table = table_of(&bad);
    assert!(table.bindings.iter().all(|b| b.source != KeySource::Game));
    assert!(row(&table, HK_ASSISTANT)
        .conflicts
        .contains(&"game.lol.open-chat".to_string()));
}

#[test]
fn against_the_console_trigger_key() {
    let settings = Settings::default();
    assert_eq!(
        check_ids(&settings, "F23", Some(HK_ASSISTANT)),
        vec!["game.dota2.console"]
    );
    // 没有控制台通道的游戏配置不占 F23（内置的都不带，ADR 032）
    let builtin = vec![GameProfile::builtin_lol(), GameProfile::builtin_dota2()];
    let src = Sources {
        settings: &settings,
        profiles: &builtin,
        voice_input: VOICE,
    };
    assert!(check(&src, "F23", Some(HK_ASSISTANT)).conflicts.is_empty());
}

#[test]
fn against_escape() {
    let settings = Settings::default();
    assert_eq!(
        check_ids(&settings, "Escape", Some(HK_ASSISTANT)),
        vec![KEY_ESCAPE]
    );
    assert_eq!(
        check_ids(&settings, "Esc", Some(HK_RECORD)),
        vec![KEY_ESCAPE]
    );
    // Esc 不看修饰键：录音时按 Shift+Esc 一样会取消
    assert_eq!(
        check_ids(&settings, "Shift+Esc", Some(HK_AUTOMATION_STOP)),
        vec![KEY_ESCAPE]
    );
    let bad = Settings {
        runtime_toggle_hotkey: "Ctrl+Escape".into(),
        ..Default::default()
    };
    let table = table_of(&bad);
    assert_eq!(row(&table, HK_RUNTIME_TOGGLE).conflicts, vec![KEY_ESCAPE]);
    assert_eq!(row(&table, KEY_ESCAPE).conflicts, vec![HK_RUNTIME_TOGGLE]);
    assert_eq!(row(&table, KEY_ESCAPE).source, KeySource::Fixed);
    assert!(
        row(&table, KEY_ESCAPE).page.is_empty(),
        "Esc 不能改，没有“去改”"
    );
}

#[test]
fn lists_every_hotkey_with_its_source_in_page_order() {
    let mut settings = Settings::default();
    settings.assistant.companion.quick_messages = vec![quick("F8", "集合")];
    let table = table_of(&settings);
    let ids: Vec<&str> = table.bindings.iter().map(|b| b.id.as_str()).collect();
    assert_eq!(
        ids,
        vec![
            HK_ASSISTANT,
            HK_AUTOMATION_STOP,
            HK_RUNTIME_TOGGLE,
            KEY_ESCAPE,
            HK_RECORD,
            HK_CHANNEL_CYCLE,
            HK_RESEND_LAST,
            HK_PP_TOGGLE,
            HK_PP_CYCLE,
            HK_QUICK_MESSAGES[0],
        ]
    );
    let sources: Vec<KeySource> = table.bindings.iter().map(|b| b.source).collect();
    use KeySource::*;
    assert_eq!(
        sources,
        vec![Core, Core, Core, Fixed, Plugin, Plugin, Plugin, Plugin, Plugin, Custom]
    );
    // 没设的键也列出来（玩家看得到还能设什么）；规范写法和录键得到的一样
    assert_eq!(row(&table, HK_RESEND_LAST).key, "");
    assert_eq!(row(&table, HK_RESEND_LAST).combo, "");
    assert_eq!(row(&table, HK_ASSISTANT).combo, "Ctrl+Backquote");
    assert_eq!(
        row(&table, HK_RECORD).owner_name.as_deref(),
        Some("语音输入")
    );
}

#[test]
fn legacy_users_only_see_what_they_use() {
    // 0.1.x 升级、只用语音输入：没有呼叫键、停下热键、快捷指令；提醒助手热键是原来的 assistant.hotkey
    let mut settings = Settings::default();
    settings.ui.legacy_upgrade = LegacyUpgrade::Dismissed;
    settings.assistant.hotkey = "Alt+F9".into();
    settings.assistant.companion.quick_messages = vec![quick("F8", "集合")];
    let table = table_of(&settings);
    let ids: Vec<&str> = table.bindings.iter().map(|b| b.id.as_str()).collect();
    assert!(
        !ids.contains(&HK_ASSISTANT)
            && !ids.contains(&HK_AUTOMATION_STOP)
            && !ids.contains(&HK_QUICK_MESSAGES[0])
    );
    assert_eq!(row(&table, HK_LEGACY_REMINDER).key, "Alt+F9");
    assert!(
        check_ids(&settings, "F8", Some(HK_RECORD)).is_empty(),
        "不用的快捷指令不占键"
    );
    // 升级后启用了她：呼叫键在 companionBindings 里，和提醒助手热键互相查
    settings.ui.legacy_upgrade = LegacyUpgrade::OptedIn;
    settings.assistant.companion_bindings = Some(crate::assistant::CompanionBindings {
        hotkey: "Alt+F9".into(),
        ..Default::default()
    });
    let table = table_of(&settings);
    assert_eq!(
        row(&table, HK_ASSISTANT).field,
        "assistant.companionBindings.hotkey"
    );
    assert_eq!(
        row(&table, HK_ASSISTANT).conflicts,
        vec![HK_LEGACY_REMINDER]
    );
    assert_eq!(
        check_ids(&settings, "Alt+F9", Some(HK_ASSISTANT)),
        vec![HK_LEGACY_REMINDER]
    );
}

/// 表里写的字段和“在哪改”和设置说明（她和外部 Agent 读的那张）一致：改了一边忘了另一边就失败。
#[test]
fn fields_and_pages_match_the_settings_guide() {
    let mut settings = Settings::default();
    settings.assistant.companion.quick_messages = vec![quick("F8", "集合")];
    let mut rows = table_of(&settings).bindings;
    settings.ui.legacy_upgrade = LegacyUpgrade::OptedIn;
    rows.extend(table_of(&settings).bindings);
    for row in rows.iter().filter(|r| r.source != KeySource::Fixed) {
        let field = crate::settings_guide::field_for(&row.field)
            .unwrap_or_else(|| panic!("{} 的字段 {} 不在设置说明里", row.id, row.field));
        let options: Vec<&str> = field.page.split("，或").map(str::trim).collect();
        assert!(
            options.contains(&row.page.as_str()),
            "{}：表里写 {}，设置说明写 {}",
            row.id,
            row.page,
            field.page
        );
    }
}

#[test]
fn warnings_still_flag_common_game_keys() {
    let settings = Settings::default();
    let profiles = profiles();
    let src = Sources {
        settings: &settings,
        profiles: &profiles,
        voice_input: VOICE,
    };
    assert_eq!(check(&src, "F1", Some(HK_ASSISTANT)).warnings.len(), 1);
    assert!(check(&src, "Ctrl+F9", Some(HK_ASSISTANT))
        .warnings
        .is_empty());
}
