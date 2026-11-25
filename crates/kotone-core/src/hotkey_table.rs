//! 热键表（ADR 040 第 1 期）：Kotone 里所有的热键、固定的键（Esc）和游戏配置里往游戏里按的键放在一张表里，
//! **冲突只在这里查**（D6）。「设置 → 热键」那一页、各页面改键时的冲突提示、`hotkeys.status` /
//! `hotkeys.conflicts` 工具都读这里，前端只显示结果。
//!
//! 这一期只读：键还存在原来的字段里，每一项写着字段和在哪改（和设置说明 `settings_guide` 里的位置一致，
//! 测试保证）；第 2 期才收成 `hotkeys.bindings`（D8）。游戏的聊天键、控制台触发键不是热键、不进表，
//! 但参与冲突检查（D2）：热键和它们一样时，在游戏里按这个键会被 Kotone 拿走。
//!
//! 冲突的规矩（和 [`crate::hotkey::combos_conflict`] 一样）：修饰键集合加主键完全相同才算；
//! 只有 Esc 不看修饰键——录音时按 Esc 取消、她操作游戏时按 Esc 停下，都不管有没有按着 Ctrl / Shift / Alt。

use serde::Serialize;

use crate::hotkey::{
    combos_conflict, parse_hotkey, HK_ASSISTANT, HK_AUTOMATION_STOP, HK_CHANNEL_CYCLE, HK_PP_CYCLE,
    HK_PP_TOGGLE, HK_QUICK_MESSAGES, HK_RESEND_LAST, HK_RUNTIME_TOGGLE, VK_ESCAPE,
};
use crate::profile::GameProfile;
use crate::settings::{LegacyUpgrade, Settings};

/// 语音输入热键（按下说话）的 id。它不是具名热键（宿主当主键登记），沿用设置页的 id。
pub const HK_RECORD: &str = "record";
/// 0.1.x 升级用户保留的提醒助手热键。
pub const HK_LEGACY_REMINDER: &str = "legacy-reminder";
/// Esc：固定的键，不能改。
pub const KEY_ESCAPE: &str = "escape";

/// 这个键是谁的。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum KeySource {
    /// Kotone 自己的（呼叫键、停下热键……）。
    Core,
    /// 插件的（现在只有语音输入的 5 个，还写死在本体的设置里，ADR 040 第 2 期改成命令）。
    Plugin,
    /// 玩家自己加的（快捷指令）。
    Custom,
    /// 固定的键（Esc），只读列出。
    Fixed,
    /// 游戏配置里往游戏里按的键：不是热键，不进表，只参与冲突检查。
    Game,
}

/// 游戏配置里的哪一个键。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum GameKey {
    /// 打开聊天框（默认频道）。
    OpenChat,
    /// 某个频道自己的开聊天框键（例如英雄联盟“所有人”的 Shift+Enter）；频道名在 `detail`。
    Channel,
    /// 发送。
    Send,
    /// 控制台通道的触发键（F13–F23，默认 F23，ADR 032 决定 3）。
    Console,
}

/// 一个占着某个键的东西：一个热键、Esc，或者游戏配置里的一个键。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyUse {
    /// 稳定 id：热键和设置页、`hotkeys.status` 的 `named[].id` 一样（`assistant`、`record`、`quick-message-3`……）；
    /// Esc 是 `escape`；游戏的键是 `game.<配置 id>.<哪个键>`（频道再加 `.<频道 id>`）。
    pub id: String,
    /// 设置里写的键（空 = 没设）。
    pub key: String,
    /// 规范写法（`Ctrl+Alt+Shift+主键`，和录键得到的一样）；没设或认不出时为空。
    pub combo: String,
    pub source: KeySource,
    /// 插件 id（`source = plugin`）或游戏配置 id（`source = game`）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    /// 插件名或游戏名。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_name: Option<String>,
    /// 游戏配置里的哪一个键（只有 `source = game` 有）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub game_key: Option<GameKey>,
    /// 名字（中文，和设置说明一样）。
    pub label: String,
    /// 补充：快捷指令说的话 / 运行的自动化、频道名、Esc 做什么。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub detail: String,
    /// 在哪改（设置说明里的位置，如“琴音 → 唤醒 → 呼叫键”）；Esc 为空（不能改）。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub page: String,
    /// 存在设置里的哪个字段（点分路径）；Esc 和游戏的键为空。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub field: String,
    /// false：插件关着，这个键现在没登记（照样参与冲突检查：打开插件就会撞上）。
    pub enabled: bool,
    /// 和谁冲突（别的项的 id）；只在 [`table`] 的结果里填。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub conflicts: Vec<String>,
}

impl KeyUse {
    /// 按 `key` 会不会撞上这一项（两边都没设不算；认不出的键不算，由保存 / 登记时报错）。
    pub fn collides(&self, key: &str) -> bool {
        if self.key.trim().is_empty() || key.trim().is_empty() {
            return false;
        }
        if self.id == KEY_ESCAPE {
            return parse_hotkey(key).is_some_and(|spec| spec.vk == VK_ESCAPE);
        }
        combos_conflict(&self.key, key)
    }

    fn collides_with(&self, other: &KeyUse) -> bool {
        self.id != other.id && (self.collides(&other.key) || other.collides(&self.key))
    }
}

/// 插件在表里的样子（宿主给：本体不认识具体的插件）。
#[derive(Debug, Clone, Copy)]
pub struct PluginInfo<'a> {
    pub id: &'a str,
    pub name: &'a str,
    pub enabled: bool,
}

/// 查表要的东西：设置、全部游戏配置、语音输入插件（它的 5 个键还写死在本体的设置里）。
#[derive(Debug, Clone, Copy)]
pub struct Sources<'a> {
    pub settings: &'a Settings,
    pub profiles: &'a [GameProfile],
    pub voice_input: PluginInfo<'a>,
}

/// 「设置 → 热键」那一页：全部热键（本体的、插件的、自己加的、固定的），每一项带冲突；
/// 游戏配置里的键单独放（不进表，冲突说明里引用它们的 id）。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HotkeyTable {
    pub bindings: Vec<KeyUse>,
    pub game_keys: Vec<KeyUse>,
}

/// 改一个键之前查一次（设置页、`hotkeys.conflicts`）：和谁冲突；这个键游戏里常不常用。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HotkeyCheck {
    pub key: String,
    /// 会撞上的（热键、Esc、游戏配置里的键）：有就不该用这个键。
    pub conflicts: Vec<KeyUse>,
    /// 提示（不拦）：这个键是常见的游戏 / 系统键。
    pub warnings: Vec<String>,
}

fn combo_of(key: &str) -> String {
    parse_hotkey(key)
        .map(|spec| spec.combo_name())
        .unwrap_or_default()
}

struct Row<'a> {
    id: &'a str,
    key: &'a str,
    label: &'a str,
    page: &'a str,
    field: &'a str,
}

fn binding(row: Row<'_>, source: KeySource, plugin: Option<PluginInfo<'_>>) -> KeyUse {
    KeyUse {
        id: row.id.into(),
        key: row.key.trim().into(),
        combo: combo_of(row.key),
        source,
        owner: plugin.map(|p| p.id.into()),
        owner_name: plugin.map(|p| p.name.into()),
        game_key: None,
        label: row.label.into(),
        detail: String::new(),
        page: row.page.into(),
        field: row.field.into(),
        enabled: plugin.map_or(true, |p| p.enabled),
        conflicts: Vec::new(),
    }
}

/// 表里的全部热键，顺序 = 页面上的顺序：本体的（Esc 跟在后面）→ 插件的 → 自己加的。
/// 只列这个玩家用得上的：0.1.x 升级后只用语音输入的人没有呼叫键、停下热键和快捷指令；
/// 提醒助手热键只有升级用户有（和 `hotkey.rs` 登记的一致）。
pub fn bindings(src: &Sources<'_>) -> Vec<KeyUse> {
    let s = src.settings;
    let legacy_only = s.ui.legacy_upgrade.uses_legacy_assistant();
    let mut out = Vec::new();
    if !legacy_only {
        let field = if s.ui.legacy_upgrade == LegacyUpgrade::OptedIn {
            "assistant.companionBindings.hotkey"
        } else {
            "assistant.hotkey"
        };
        out.push(binding(
            Row {
                id: HK_ASSISTANT,
                key: s.call_hotkey(),
                label: "呼叫键",
                page: "琴音 → 唤醒 → 呼叫键",
                field,
            },
            KeySource::Core,
            None,
        ));
        out.push(binding(
            Row {
                id: HK_AUTOMATION_STOP,
                key: &s.automation_stop_hotkey,
                label: "停下热键",
                page: "设置 → 系统 → 停下",
                field: "automationStopHotkey",
            },
            KeySource::Core,
            None,
        ));
    }
    out.push(binding(
        Row {
            id: HK_RUNTIME_TOGGLE,
            key: &s.runtime_toggle_hotkey,
            label: "启动 / 停止本地语音识别热键",
            page: "设置 → 模型 → 语音识别",
            field: "runtimeToggleHotkey",
        },
        KeySource::Core,
        None,
    ));
    if s.ui.legacy_upgrade.has_legacy_reminders() {
        out.push(binding(
            Row {
                id: HK_LEGACY_REMINDER,
                key: s.legacy_reminder_hotkey(),
                label: "提醒助手热键",
                page: "提醒助手 → 提醒设置",
                field: "assistant.hotkey",
            },
            KeySource::Core,
            None,
        ));
    }
    out.push(KeyUse {
        detail: "取消这次录音；她操作游戏时让她停下".into(),
        ..binding(
            Row {
                id: KEY_ESCAPE,
                key: "Escape",
                label: "Esc",
                page: "",
                field: "",
            },
            KeySource::Fixed,
            None,
        )
    });

    let voice = Some(src.voice_input);
    let others = "插件 → 语音输入 → 输入 → 其他热键";
    for row in [
        Row {
            id: HK_RECORD,
            key: &s.hotkey.key,
            label: "语音输入热键",
            page: "插件 → 语音输入 → 输入",
            field: "hotkey.key",
        },
        Row {
            id: HK_CHANNEL_CYCLE,
            key: &s.channel_cycle_hotkey,
            label: "切换频道热键",
            page: others,
            field: "channelCycleHotkey",
        },
        Row {
            id: HK_RESEND_LAST,
            key: &s.resend_last_hotkey,
            label: "重发上一句热键",
            page: others,
            field: "resendLastHotkey",
        },
        Row {
            id: HK_PP_TOGGLE,
            key: &s.toggle_post_processing_hotkey,
            label: "开关文字处理热键",
            page: others,
            field: "togglePostProcessingHotkey",
        },
        Row {
            id: HK_PP_CYCLE,
            key: &s.cycle_post_processing_hotkey,
            label: "切换文字处理方案热键",
            page: others,
            field: "cyclePostProcessingHotkey",
        },
    ] {
        out.push(binding(row, KeySource::Plugin, voice));
    }

    if !legacy_only {
        for (index, message) in s.assistant.companion.quick_messages.iter().enumerate() {
            let Some(id) = HK_QUICK_MESSAGES.get(index) else {
                break;
            };
            let label = format!("快捷指令 {}", index + 1);
            let field = format!("assistant.companion.quickMessages.{index}.hotkey");
            let detail = if message.runs_automation() {
                format!("运行自动化 {}", message.automation_id.trim())
            } else if message.label.trim().is_empty() {
                message.text.trim().to_string()
            } else {
                message.label.trim().to_string()
            };
            out.push(KeyUse {
                detail,
                ..binding(
                    Row {
                        id,
                        key: &message.hotkey,
                        label: &label,
                        page: "琴音 → 唤醒 → 快捷指令",
                        field: &field,
                    },
                    KeySource::Custom,
                    None,
                )
            });
        }
    }
    out
}

/// 全部游戏配置里往游戏里按的键：打开聊天框、各频道自己的开框键、发送、控制台触发键。
/// 正在用的游戏配置排在最前（冲突提示先说它）；同一个游戏里键相同的只留一条（英雄联盟的开框和发送都是 Enter）。
pub fn game_keys(profiles: &[GameProfile], active_profile_id: Option<&str>) -> Vec<KeyUse> {
    let mut ordered: Vec<&GameProfile> = profiles.iter().collect();
    ordered.sort_by_key(|p| Some(p.id.as_str()) != active_profile_id);
    let mut out = Vec::new();
    for profile in ordered {
        let mut keys: Vec<(String, GameKey, String, String)> = vec![(
            format!("game.{}.open-chat", profile.id),
            GameKey::OpenChat,
            profile.open_chat_key.clone(),
            String::new(),
        )];
        for channel in &profile.channels {
            if let Some(key) = &channel.open_chat_key {
                keys.push((
                    format!("game.{}.channel.{}", profile.id, channel.id),
                    GameKey::Channel,
                    key.clone(),
                    channel.display_name.clone(),
                ));
            }
        }
        keys.push((
            format!("game.{}.send", profile.id),
            GameKey::Send,
            profile.send_key.clone(),
            String::new(),
        ));
        if let Some(console) = &profile.console_chat {
            keys.push((
                format!("game.{}.console", profile.id),
                GameKey::Console,
                console.trigger_key.clone(),
                String::new(),
            ));
        }
        let mut seen: Vec<String> = Vec::new();
        for (id, game_key, key, detail) in keys {
            let key = key.trim().to_string();
            if key.is_empty()
                || seen
                    .iter()
                    .any(|earlier| combos_conflict(earlier, &key) || *earlier == key)
            {
                continue;
            }
            seen.push(key.clone());
            let label = match game_key {
                GameKey::OpenChat => "打开聊天框键".to_string(),
                GameKey::Channel => format!("「{detail}」频道的开聊天框键"),
                GameKey::Send => "发送键".to_string(),
                GameKey::Console => "控制台触发键".to_string(),
            };
            out.push(KeyUse {
                id,
                combo: combo_of(&key),
                key,
                source: KeySource::Game,
                owner: Some(profile.id.clone()),
                owner_name: Some(profile.display_name.clone()),
                game_key: Some(game_key),
                label,
                detail,
                page: "游戏列表".into(),
                field: String::new(),
                enabled: true,
                conflicts: Vec::new(),
            });
        }
    }
    out
}

/// 整张表：每个热键和谁冲突（别的热键、Esc、游戏配置里的键）。
pub fn table(src: &Sources<'_>) -> HotkeyTable {
    let game_keys = game_keys(src.profiles, src.settings.active_profile_id.as_deref());
    let mut rows = bindings(src);
    let conflicts: Vec<Vec<String>> = rows
        .iter()
        .map(|row| {
            rows.iter()
                .chain(&game_keys)
                .filter(|other| row.collides_with(other))
                .map(|other| other.id.clone())
                .collect()
        })
        .collect();
    for (row, found) in rows.iter_mut().zip(conflicts) {
        row.conflicts = found;
    }
    HotkeyTable {
        bindings: rows,
        game_keys,
    }
}

/// 把 `except` 这一项改成 `key` 之前查一次：会撞上谁（热键、Esc、游戏配置里的键），这个键游戏里常不常用。
/// `except` 是正在改的那一项的 id（不和自己比）；不传 = 新加的键。
pub fn check(src: &Sources<'_>, key: &str, except: Option<&str>) -> HotkeyCheck {
    let key = key.trim();
    let conflicts = bindings(src)
        .into_iter()
        .chain(game_keys(
            src.profiles,
            src.settings.active_profile_id.as_deref(),
        ))
        .filter(|item| Some(item.id.as_str()) != except && item.collides(key))
        .collect();
    HotkeyCheck {
        key: key.into(),
        conflicts,
        warnings: common_game_key_warnings(key),
    }
}

/// 这个键是不是常见的游戏 / 系统键（只提示、不拦；首次引导和 `hotkeys.conflicts` 用）。
/// 以后可以照常见游戏的默认键位表比对，现在只是一张静态表。
pub fn common_game_key_warnings(key: &str) -> Vec<String> {
    let common_game_keys = [
        "F1", "F2", "F3", "F4", "F5", "F6", "F7", "F9", "F10", "F11", "F12", "Tab", "Space",
        "Enter", "Shift", "Control", "Alt",
    ];
    if common_game_keys.iter().any(|k| k.eq_ignore_ascii_case(key)) {
        vec![format!(
            "「{key}」是常见游戏/系统键位，可能与游戏内操作冲突"
        )]
    } else {
        Vec::new()
    }
}

#[cfg(test)]
#[path = "hotkey_table_tests.rs"]
mod tests;
