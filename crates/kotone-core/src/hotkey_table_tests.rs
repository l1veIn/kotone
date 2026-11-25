//! 热键表的边界（ADR 040 D6）：每一类冲突一条，加上不冲突的、只列用得上的、和设置说明一致。

use super::*;
use crate::assistant::QuickMessage;
use crate::hotkey::{
    HK_ASSISTANT, HK_AUTOMATION_STOP, HK_CHANNEL_CYCLE, HK_PP_CYCLE, HK_PP_TOGGLE, HK_RESEND_LAST,
    HK_RUNTIME_TOGGLE,
};
use crate::hotkey_bindings::{Binding, CMD_TALK};
use crate::profile::ConsoleChat;
use crate::settings::LegacyUpgrade;

/// 语音输入插件清单里的 5 个命令（`plugins/voice-input/plugin.toml`；插件那边有测试对着本体的对照表）。
fn voice_commands() -> &'static [CommandSpec] {
    static COMMANDS: std::sync::OnceLock<Vec<CommandSpec>> = std::sync::OnceLock::new();
    COMMANDS.get_or_init(|| {
        let spec = |id: &str, name: &str, kind: CommandKind| CommandSpec {
            id: id.into(),
            name: name.into(),
            description: String::new(),
            kind,
            listens: true,
        };
        vec![
            spec("talk", "语音输入热键", CommandKind::Hold),
            spec("channel-cycle", "切换频道热键", CommandKind::Press),
            spec("resend-last", "重发上一句热键", CommandKind::Press),
            spec(
                "post-process-toggle",
                "开关文字处理热键",
                CommandKind::Press,
            ),
            spec(
                "post-process-cycle",
                "切换文字处理方案热键",
                CommandKind::Press,
            ),
        ]
    })
}

fn voice(enabled: bool) -> PluginInfo<'static> {
    PluginInfo {
        id: "voice-input",
        name: "语音输入",
        enabled,
        commands: voice_commands(),
    }
}

fn plugins_on() -> &'static [PluginInfo<'static>] {
    static ON: std::sync::OnceLock<Vec<PluginInfo<'static>>> = std::sync::OnceLock::new();
    ON.get_or_init(|| vec![voice(true)])
}

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
        plugins: plugins_on(),
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
        plugins: plugins_on(),
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
    // 新装不再默认带频道切换（ADR 040 D4）：这里是老用户迁移来的 Shift+CapsLock
    let settings = Settings {
        channel_cycle_hotkey: "Shift+CapsLock".into(),
        ..Default::default()
    };
    // 语音输入热键默认 CapsLock、切换频道 Shift+CapsLock
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
    let off = [voice(false)];
    let src = Sources {
        settings: &settings,
        profiles: &profiles,
        plugins: &off,
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
        plugins: plugins_on(),
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
        plugins: plugins_on(),
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
        plugins: plugins_on(),
    };
    assert_eq!(check(&src, "F1", Some(HK_ASSISTANT)).warnings.len(), 1);
    assert!(check(&src, "Ctrl+F9", Some(HK_ASSISTANT))
        .warnings
        .is_empty());
}

fn plan_of(settings: &Settings) -> Vec<Planned> {
    registration_plan(&Sources {
        settings,
        profiles: &[],
        plugins: plugins_on(),
    })
}

fn planned<'a>(plan: &'a [Planned], id: &str) -> &'a Planned {
    plan.iter()
        .find(|p| p.id == id)
        .unwrap_or_else(|| panic!("{id} 不在登记计划里"))
}

#[test]
fn the_plan_follows_the_table_order_and_says_when_each_key_registers() {
    let mut settings = Settings::default();
    settings.set_binding(crate::hotkey_bindings::CMD_CHANNEL_CYCLE, "Shift+CapsLock");
    settings.assistant.companion.quick_messages = vec![quick("F8", "集合打龙")];
    settings.sync_hotkey_mirrors();
    let plan = plan_of(&settings);
    let ids: Vec<&str> = plan.iter().map(|p| p.id.as_str()).collect();
    // 没绑的（启停识别、重发……）不登记
    assert_eq!(
        ids,
        vec![
            HK_ASSISTANT,
            HK_AUTOMATION_STOP,
            HK_RECORD,
            HK_CHANNEL_CYCLE,
            HK_QUICK_MESSAGES[0]
        ]
    );
    assert!(plan.iter().all(|p| p.error.is_none()));
    // 停下一直登记；呼叫、快捷指令要听（识别没跑时只看不吞、气泡提示）；语音输入的命令在悬浮窗提示
    let stop = planned(&plan, HK_AUTOMATION_STOP);
    assert!(stop.always && stop.hint.is_none());
    let call = planned(&plan, HK_ASSISTANT);
    assert_eq!((call.always, call.hint), (false, Some(HintAt::Bubble)));
    assert_eq!(
        call.command.as_deref(),
        Some(crate::hotkey_bindings::CMD_CALL)
    );
    let talk = planned(&plan, HK_RECORD);
    assert_eq!(
        (talk.always, talk.hold, talk.hint),
        (false, true, Some(HintAt::Overlay))
    );
    assert_eq!(planned(&plan, HK_CHANNEL_CYCLE).hint, Some(HintAt::Overlay));
    assert_eq!(
        planned(&plan, HK_QUICK_MESSAGES[0]).hint,
        Some(HintAt::Bubble)
    );
}

#[test]
fn the_first_one_keeps_a_shared_key_and_esc_is_never_taken() {
    let mut settings = Settings {
        automation_stop_hotkey: "Ctrl+Backquote".into(),
        runtime_toggle_hotkey: "Ctrl+Escape".into(),
        ..Default::default()
    };
    settings.assistant.companion.quick_messages = vec![quick("CapsLock", "集合")];
    let plan = plan_of(&settings);
    assert!(
        planned(&plan, HK_ASSISTANT).error.is_none(),
        "表里靠前的先拿到"
    );
    assert_eq!(
        planned(&plan, HK_AUTOMATION_STOP).error.as_deref(),
        Some("和「呼叫键」用的是同一个键，没生效")
    );
    assert_eq!(
        planned(&plan, HK_RUNTIME_TOGGLE).error.as_deref(),
        Some(ESC_REASON)
    );
    assert!(planned(&plan, HK_RECORD).error.is_none());
    assert_eq!(
        planned(&plan, HK_QUICK_MESSAGES[0]).error.as_deref(),
        Some("和「语音输入热键」用的是同一个键，没生效")
    );
    // 和游戏聊天键一样的照旧登记（表里标红；写入时才拦）
    let enter = Settings {
        hotkey: crate::settings::HotkeyConfig {
            key: "Enter".into(),
            mode: crate::hotkey::HotkeyMode::Toggle,
        },
        ..Default::default()
    };
    assert!(planned(&plan_of(&enter), HK_RECORD).error.is_none());
}

#[test]
fn plugin_off_unknown_commands_and_legacy_reminders_in_the_plan() {
    let mut settings = Settings::default();
    settings
        .hotkeys
        .bindings
        .push(Binding::command("F7", "someone.new-thing"));
    settings.hotkeys.bindings.push(Binding(
        serde_json::json!({"key": "F6", "page": "ui://kotone/lol/tier"}),
    ));
    let off = [voice(false)];
    let plan = registration_plan(&Sources {
        settings: &settings,
        profiles: &[],
        plugins: &off,
    });
    assert!(
        plan.iter().all(|p| p.id != HK_RECORD),
        "插件关着：语音输入的键不登记"
    );
    let unknown: Vec<&Planned> = plan
        .iter()
        .filter(|p| p.id.starts_with("binding."))
        .collect();
    assert_eq!(unknown.len(), 2);
    assert!(unknown.iter().all(|p| p.error.is_some()));
    // 表里也列出来（标“找不到”），条目原样留着
    let table = table_of(&settings);
    assert!(table
        .bindings
        .iter()
        .any(|b| b.source == KeySource::Unknown && b.label == "someone.new-thing"));

    // 0.1.x 升级用户的提醒助手热键：按交互模式按住 / 按一下，识别没跑时不登记也不提示（D10 照旧）
    let legacy = Settings {
        ui: crate::settings::UiConfig {
            legacy_upgrade: LegacyUpgrade::Dismissed,
            ..Default::default()
        },
        assistant: crate::assistant::AssistantConfig {
            hotkey: "Alt+F9".into(),
            ..Default::default()
        },
        ..Default::default()
    };
    let plan = plan_of(&legacy);
    let reminder = planned(&plan, HK_LEGACY_REMINDER);
    assert_eq!(
        (reminder.hold, reminder.always, reminder.hint),
        (true, false, None)
    );
    assert!(plan
        .iter()
        .all(|p| p.id != HK_ASSISTANT && p.id != HK_AUTOMATION_STOP));
    // 表里还存着呼叫、停下（只是这种用户看不到）：认识的命令，不算“找不到”
    assert!(plan.iter().all(|p| !p.id.starts_with("binding.")));
    assert!(table_of(&legacy)
        .bindings
        .iter()
        .all(|b| b.source != KeySource::Unknown));
}

fn sources<'a>(settings: &'a Settings, profiles: &'a [GameProfile]) -> Sources<'a> {
    Sources {
        settings,
        profiles,
        plugins: plugins_on(),
    }
}

#[test]
fn saving_a_new_conflict_is_refused_with_what_to_do_next() {
    let profiles = profiles();
    let before = Settings::default();
    let changed = |change: &dyn Fn(&mut Settings)| {
        let mut after = before.clone();
        change(&mut after);
        after.reconcile_hotkeys(&before);
        new_conflict_error(&sources(&before, &profiles), &sources(&after, &profiles))
    };
    // 和别的热键撞
    assert_eq!(
        changed(&|s| s.channel_cycle_hotkey = "CapsLock".into()).as_deref(),
        Some(
            "「CapsLock」已经是「语音输入热键」的热键：先在「设置 → 热键」把它换掉，或者换一个键。"
        )
    );
    // 和 Esc 撞
    assert_eq!(
        changed(&|s| s.runtime_toggle_hotkey = "Shift+Escape".into()).as_deref(),
        Some("「Shift+Escape」：Esc 留给停下她和取消收音，换一个键。")
    );
    // 和游戏的聊天键撞
    assert_eq!(
        changed(&|s| s.assistant.hotkey = "Enter".into()).as_deref(),
        Some("「Enter」是《League of Legends》的打开聊天框键，设成热键以后在游戏里按它会被 Kotone 拿走，换一个键。")
    );
    // 不冲突、只改别的设置：放行
    assert_eq!(changed(&|s| s.channel_cycle_hotkey = "Alt+C".into()), None);
    assert_eq!(changed(&|s| s.language = "en".into()), None);
}

#[test]
fn an_existing_conflict_does_not_block_other_edits() {
    let profiles = profiles();
    // 老配置里本来就冲突着（停下热键 = 呼叫键）
    let mut before = Settings::default();
    before.set_binding(crate::hotkey_bindings::CMD_STOP, "Ctrl+Backquote");
    before.assistant.companion.quick_messages = vec![quick("F8", "集合")];
    before.sync_hotkey_mirrors();
    let mut after = before.clone();
    after.language = "en".into();
    // 前面插一条快捷指令：槽位挪了，键没变，不算新冲突
    after
        .assistant
        .companion
        .quick_messages
        .insert(0, quick("", "新的一条"));
    assert_eq!(
        new_conflict_error(&sources(&before, &profiles), &sources(&after, &profiles)),
        None
    );
    // 改说话键：不冲突，放行（老冲突不挡）
    after.set_binding(CMD_TALK, "F9");
    after.sync_hotkey_mirrors();
    assert_eq!(
        new_conflict_error(&sources(&before, &profiles), &sources(&after, &profiles)),
        None
    );
}
