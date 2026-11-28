//! 热键表的边界（ADR 040 D6）：每一类冲突一条，加上不冲突的、只列用得上的、和设置说明一致。

use super::*;
use crate::assistant::QuickMessage;
use crate::hotkey::{
    HK_ASSISTANT, HK_AUTOMATION_STOP, HK_CHANNEL_CYCLE, HK_PP_CYCLE, HK_PP_TOGGLE, HK_RESEND_LAST,
    HK_RUNTIME_TOGGLE,
};
use crate::hotkey_bindings::{Binding, CMD_TALK};
use crate::hotkey_custom::CustomState;
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
            tool: None,
            args: None,
        };
        vec![
            spec("talk", "语音输入热键", CommandKind::Hold),
            spec("channel-cycle", "频道切换热键", CommandKind::Press),
            spec("resend-last", "重发最近一条热键", CommandKind::Press),
            spec(
                "post-process-toggle",
                "文字处理开关热键",
                CommandKind::Press,
            ),
            spec(
                "post-process-cycle",
                "切换文字处理流程热键",
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
        custom: None,
    };
    check(&src, key, except)
        .conflicts
        .into_iter()
        .map(|c| short_id(&c.id))
        .collect()
}

/// 表里的行 id 是命令的完整名（`kotone.call`）；下面的检查用第 1 期的短 id（`assistant`）写，读起来短。
fn short_id(id: &str) -> String {
    crate::hotkey_bindings::legacy_for(id).map_or_else(|| id.to_string(), |l| l.id.to_string())
}

fn table_of(settings: &Settings) -> HotkeyTable {
    let profiles = profiles();
    let mut table = table(&Sources {
        settings,
        profiles: &profiles,
        plugins: plugins_on(),
        custom: None,
    });
    for row in &mut table.bindings {
        row.id = short_id(&row.id);
        row.conflicts = row.conflicts.iter().map(|c| short_id(c)).collect();
    }
    table
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
        custom: None,
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
    let (first, second) = ("custom.q-不绑键的", "custom.q-集合打龙");
    assert_eq!(check_ids(&settings, "F8", Some(HK_ASSISTANT)), vec![second]);
    // 第 1、2 期的 id 还认：`quick-message-2` 是快捷指令的第 2 条（改它自己不算冲突）
    assert!(check_ids(&settings, "F8", Some("quick-message-2")).is_empty());
    // 两条快捷指令同一个键：互相冲突
    settings.assistant.companion.quick_messages[0].hotkey = "F8".into();
    let table = table_of(&settings);
    assert_eq!(row(&table, first).conflicts, vec![second]);
    let row2 = row(&table, second);
    assert_eq!(row2.source, KeySource::Custom);
    assert_eq!(row2.action, Some("say"));
    assert_eq!(row2.detail, "集合打龙");
    assert_eq!(row2.label, "快捷指令「集合打龙」");
    assert_eq!(row2.status_id, second);
    // 快捷指令搬进了热键表（第 3 期）
    assert_eq!(row2.field, "hotkeys.bindings");
}

/// 自己加的那一行：名字是玩家起的，没起就用要做的事；运行自动化不用听（识别没跑也登记），跟她说一句要听；
/// 工具、页面这一步还按不了，列成“找不到”、不登记。
#[test]
fn custom_rows_say_what_they_do() {
    let mut settings = Settings::default();
    settings.hotkeys.bindings.extend([
        Binding(serde_json::json!({"key": "F6", "say": "一句很长很长很长很长很长很长的话呀", "id": "a"})),
        Binding(serde_json::json!({"key": "F7", "tool": "automation.run", "args": {"id": "auto-1"}, "id": "b", "label": "连发"})),
        Binding(serde_json::json!({"key": "F9", "tool": "game_chat.send", "args": {"text": "gg"}, "id": "c"})),
    ]);
    // 直接改了表就照表写回旧字段（不然算“代码直接改了旧字段”，照旧字段对齐）
    settings.sync_hotkey_mirrors();
    let table = table_of(&settings);
    let say = row(&table, "custom.a");
    assert_eq!(say.label, "快捷指令「一句很长很长很长很长很长很长的话…」");
    let auto = row(&table, "custom.b");
    assert_eq!(
        (auto.label.as_str(), auto.action),
        ("连发", Some("automation"))
    );
    assert_eq!(auto.detail, "运行自动化 auto-1");
    let tool = row(&table, "custom.c");
    assert_eq!(
        (tool.label.as_str(), tool.action, tool.source),
        ("调用 game_chat.send", Some("tool"), KeySource::Custom)
    );
    let plan = plan_of(&settings);
    let say = planned(&plan, "custom.a");
    assert_eq!((say.always, say.hint), (false, Some(HintAt::Bubble)));
    let auto = planned(&plan, "custom.b");
    assert_eq!((auto.always, auto.hint), (true, None));
    // 调工具不用听：随应用登记（宿主没说能不能按时当能按，按下时宿主还会再判）
    let tool = planned(&plan, "custom.c");
    assert_eq!((tool.always, tool.error.as_deref()), (true, None));
}

/// 调工具、打开页面能不能按由宿主算（工具目录、插件开关、权限，ADR 040 D9）：插件关着的不登记也不占键；要确认的、
/// 按不了的不登记，写原因（玩家看得到要去哪确认）；说一句不问宿主。
#[test]
fn custom_rows_follow_what_the_host_says() {
    let mut settings = Settings::default();
    settings.hotkeys.bindings.extend([
        Binding(serde_json::json!({"key": "F5", "say": "一句", "id": "say"})),
        Binding(serde_json::json!({"key": "F6", "tool": "x.ready", "id": "ready"})),
        Binding(serde_json::json!({"key": "F7", "tool": "x.ask", "id": "ask"})),
        Binding(serde_json::json!({"key": "F8", "tool": "x.denied", "id": "denied"})),
        Binding(serde_json::json!({"key": "F9", "page": "ui://kotone/off/page", "id": "off"})),
    ]);
    settings.sync_hotkey_mirrors();
    let host = |b: &Binding| match b.id() {
        Some("ask") => CustomState::NeedsConfirm("会修改设置（x.ask）".into()),
        Some("denied") => CustomState::Blocked("用户设置为禁止".into()),
        Some("off") => CustomState::Off,
        Some("say") => panic!("说一句不问宿主"),
        _ => CustomState::Ready,
    };
    let src = Sources {
        settings: &settings,
        profiles: &[],
        plugins: plugins_on(),
        custom: Some(&host),
    };
    let rows = bindings(&src);
    let find = |id: &str| rows.iter().find(|r| r.id == id).unwrap();
    assert_eq!(
        find("custom.ask").confirm.as_deref(),
        Some("会修改设置（x.ask）")
    );
    assert_eq!(
        find("custom.denied").blocked.as_deref(),
        Some("用户设置为禁止")
    );
    assert!(!find("custom.off").enabled);
    let plan = registration_plan(&src);
    let ids: Vec<&str> = plan.iter().map(|p| p.id.as_str()).collect();
    assert!(!ids.contains(&"custom.off"), "插件关着：不登记也不占键");
    assert!(planned(&plan, "custom.ready").error.is_none());
    assert_eq!(
        planned(&plan, "custom.ask").error.as_deref(),
        Some("要先在「设置 → 热键」确认一次：会修改设置（x.ask）")
    );
    assert_eq!(
        planned(&plan, "custom.denied").error.as_deref(),
        Some("用户设置为禁止")
    );
    // 没登记的不占键：后面同一个键的照样拿得到
    settings.hotkeys.bindings.push(Binding(
        serde_json::json!({"key": "F7", "say": "同一个键", "id": "later"}),
    ));
    settings.sync_hotkey_mirrors();
    let src = Sources {
        settings: &settings,
        profiles: &[],
        plugins: plugins_on(),
        custom: Some(&host),
    };
    assert!(planned(&registration_plan(&src), "custom.later")
        .error
        .is_none());
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
        custom: None,
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
        custom: None,
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
            "custom.q-集合",
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
    // 0.1.x 升级、只用语音输入：没有呼叫键、停下热键、快捷指令；提醒助手和它的热键退掉了（ADR 052）
    let mut settings = Settings::default();
    settings.ui.legacy_upgrade = LegacyUpgrade::Dismissed;
    settings.assistant.hotkey = "Alt+F9".into();
    settings.assistant.companion.quick_messages = vec![quick("F8", "集合")];
    let table = table_of(&settings);
    let ids: Vec<&str> = table.bindings.iter().map(|b| b.id.as_str()).collect();
    assert!(
        !ids.contains(&HK_ASSISTANT)
            && !ids.contains(&HK_AUTOMATION_STOP)
            && !ids.contains(&"custom.q-集合")
            && !ids.contains(&"legacy-reminder")
    );
    assert!(
        table.bindings.iter().all(|b| b.key != "Alt+F9"),
        "只用语音输入的人没有呼叫键"
    );
    assert!(
        check_ids(&settings, "F8", Some(HK_RECORD)).is_empty(),
        "不用的快捷指令不占键"
    );
    // 启用了她：呼叫键和新用户一样在 assistant.hotkey
    settings.ui.legacy_upgrade = LegacyUpgrade::NotNeeded;
    let table = table_of(&settings);
    assert_eq!(row(&table, HK_ASSISTANT).field, "assistant.hotkey");
    assert_eq!(row(&table, HK_ASSISTANT).key, "Alt+F9");
}

/// 表里写的字段和“在哪改”和设置说明（她和外部 Agent 读的那张）一致：改了一边忘了另一边就失败。
#[test]
fn fields_and_pages_match_the_settings_guide() {
    let mut settings = Settings::default();
    settings.assistant.companion.quick_messages = vec![quick("F8", "集合")];
    let mut rows = table_of(&settings).bindings;
    settings.ui.legacy_upgrade = LegacyUpgrade::Dismissed;
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
        custom: None,
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
        custom: None,
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
    settings
        .hotkeys
        .bindings
        .push(crate::hotkey_custom::binding_of(&quick("F8", "集合打龙")));
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
            "custom.q-集合打龙"
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
        planned(&plan, "custom.q-集合打龙").hint,
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
        planned(&plan, "custom.q-集合").error.as_deref(),
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
fn plugin_off_unknown_commands_and_voice_input_only_users_in_the_plan() {
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
        custom: None,
    });
    assert!(
        plan.iter().all(|p| p.id != HK_RECORD),
        "插件关着：语音输入的键不登记"
    );
    let unknown: Vec<&Planned> = plan
        .iter()
        .filter(|p| p.id.starts_with("binding."))
        .collect();
    assert_eq!(unknown.len(), 1);
    assert!(unknown.iter().all(|p| p.error.is_some()));
    // 打开页面是自己加的（第 3 期）：还没补 id 的行 id 是 `custom.#<下标>`
    assert!(plan
        .iter()
        .any(|p| p.id.starts_with("custom.#") && p.combo == "F6"));
    // 表里也列出来（标“找不到”），条目原样留着
    let table = table_of(&settings);
    assert!(table
        .bindings
        .iter()
        .any(|b| b.source == KeySource::Unknown && b.label == "someone.new-thing"));

    // 0.1.x 升级、只用语音输入：不登记呼叫、停下，旧提醒键也不登记（提醒助手退掉了，ADR 052）
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
    assert!(plan.iter().all(|p| p.combo != "Alt+F9"));
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
        custom: None,
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
    before
        .hotkeys
        .bindings
        .push(crate::hotkey_custom::binding_of(&quick("F8", "集合")));
    before.sync_hotkey_mirrors();
    let mut after = before.clone();
    after.language = "en".into();
    // 前面插一条快捷指令：别的条目挪了位置，键没变，不算新冲突（每条有自己的 id）
    let at = after.hotkeys.bindings.len() - 1;
    after
        .hotkeys
        .bindings
        .insert(at, crate::hotkey_custom::binding_of(&quick("", "新的一条")));
    after.sync_hotkey_mirrors();
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

/// 表里的行 id 是命令的完整名（第 2 期第 3 步）；登记计划和 `hotkeys.status` 照旧用第 1 期的 id。
/// 改键前查冲突时，`except` 写命令名、表里的 id、第 1 期的 id 都认（外部 Agent 照旧文档写 `assistant`）。
#[test]
fn rows_are_named_by_command_and_old_ids_still_work() {
    use crate::hotkey_bindings::{CMD_CALL, CMD_CHANNEL_CYCLE, CMD_SPEECH_TOGGLE, CMD_STOP};
    let settings = Settings::default();
    let profiles = profiles();
    let src = Sources {
        settings: &settings,
        profiles: &profiles,
        plugins: plugins_on(),
        custom: None,
    };
    let rows = bindings(&src);
    let ids: Vec<&str> = rows.iter().take(3).map(|r| r.id.as_str()).collect();
    assert_eq!(ids, vec![CMD_CALL, CMD_STOP, CMD_SPEECH_TOGGLE]);
    let talk = rows.iter().find(|r| r.id == CMD_TALK).unwrap();
    assert_eq!(
        (talk.page.as_str(), talk.default_key.as_str()),
        (HOTKEYS_PAGE, "CapsLock")
    );
    let channel = rows.iter().find(|r| r.id == CMD_CHANNEL_CYCLE).unwrap();
    assert_eq!(channel.default_key, "", "新装不带频道切换（D4）");
    for except in [CMD_TALK, HK_RECORD] {
        assert!(
            check(&src, "CapsLock", Some(except)).conflicts.is_empty(),
            "{except}"
        );
    }
    assert_eq!(
        planned(&registration_plan(&src), HK_RECORD)
            .command
            .as_deref(),
        Some(CMD_TALK)
    );
}

/// 插件登记的调工具的命令（第 3 期，英雄联盟的一键喊话）：表里列出来、不带键；照宿主算的判（权限、开关）；
/// 绑了键随应用登记（不用语音识别，也不只看不吞）；权限里设成每次问的写原因、不登记。
#[test]
fn plugin_tool_commands_are_listed_and_run_without_listening() {
    let shout = |id: &str, what: &str| CommandSpec {
        id: id.into(),
        name: format!("喊{what}"),
        description: format!("把{what}发到队伍聊天"),
        kind: CommandKind::Press,
        listens: false,
        tool: Some("lol.shout".into()),
        args: Some(serde_json::json!({"what": what})),
    };
    let commands = vec![
        shout("shout-flash", "flash"),
        shout("shout-records", "records"),
        shout("shout-premade", "premade"),
    ];
    let lol = PluginInfo {
        id: "lol",
        name: "英雄联盟",
        enabled: true,
        commands: &commands,
    };
    let plugins = [voice(true), lol];
    let mut settings = Settings::default();
    settings.hotkeys.bindings.extend([
        Binding::command("F7", "lol.shout-flash"),
        Binding::command("F8", "lol.shout-records"),
        Binding::command("F9", "lol.shout-premade"),
    ]);
    settings.sync_hotkey_mirrors();
    let host = |b: &Binding| match b.args().and_then(|a| a["what"].as_str()) {
        Some("records") => CustomState::NeedsConfirm("用户设置为每次询问（lol.shout）".into()),
        Some("premade") => {
            CustomState::Blocked("当前是只读模式：去她的「权限」里改（lol.shout）".into())
        }
        _ => {
            assert_eq!(b.tool(), Some("lol.shout"));
            CustomState::Ready
        }
    };
    let src = Sources {
        settings: &settings,
        profiles: &[],
        plugins: &plugins,
        custom: Some(&host),
    };
    let rows = bindings(&src);
    let find = |id: &str| rows.iter().find(|r| r.id == id).unwrap();
    let flash = find("lol.shout-flash");
    assert_eq!(flash.source, KeySource::Plugin);
    assert_eq!(flash.action, Some("tool"));
    assert_eq!(flash.label, "喊flash");
    assert_eq!(flash.detail, "把flash发到队伍聊天");
    assert_eq!(flash.owner_name.as_deref(), Some("英雄联盟"));
    assert!(flash.enabled && flash.blocked.is_none() && flash.confirm.is_none());
    assert!(find("lol.shout-records")
        .blocked
        .as_deref()
        .is_some_and(|b| b.contains("热键上没法每次问")));
    assert!(
        find("lol.shout-records").confirm.is_none(),
        "预设没有确认这一步"
    );
    let plan = registration_plan(&src);
    let flash = planned(&plan, "lol.shout-flash");
    assert!(flash.error.is_none(), "{:?}", flash.error);
    assert!(flash.always && flash.hint.is_none(), "不用听：随应用登记");
    assert!(planned(&plan, "lol.shout-records").error.is_some());
    assert!(planned(&plan, "lol.shout-premade")
        .error
        .as_deref()
        .is_some_and(|e| e.contains("只读模式")));
    // 语音输入的命令照旧要听
    if let Some(talk) = plan.iter().find(|p| p.id == HK_RECORD) {
        assert!(!talk.always && talk.hint.is_some());
    }

    // 没设键的照样列出来（D4：插件不带默认键）；插件关着的不登记
    let off = PluginInfo {
        enabled: false,
        ..plugins[1]
    };
    let plugins = [voice(true), off];
    let src = Sources {
        settings: &settings,
        profiles: &[],
        plugins: &plugins,
        custom: Some(&host),
    };
    assert!(registration_plan(&src)
        .iter()
        .all(|p| !p.id.starts_with("lol.")));
    let empty = Settings::default();
    let plugins = [voice(true), lol];
    let src = Sources {
        settings: &empty,
        profiles: &[],
        plugins: &plugins,
        custom: None,
    };
    let rows = bindings(&src);
    let lol_rows: Vec<&KeyUse> = rows
        .iter()
        .filter(|r| r.owner.as_deref() == Some("lol"))
        .collect();
    assert_eq!(lol_rows.len(), 3);
    assert!(lol_rows
        .iter()
        .all(|r| r.key.is_empty() && r.default_key.is_empty()));
}
