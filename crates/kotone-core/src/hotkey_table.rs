//! 热键表（ADR 040）：Kotone 里所有的热键、固定的键（Esc）和游戏配置里往游戏里按的键放在一张表里，
//! **冲突只在这里查**（D6）。「设置 → 热键」那一页、各页面改键时的冲突提示、`settings.update` 拒绝新冲突、
//! 宿主和命令行登记热键（[`registration_plan`]）、`hotkeys.status` / `hotkeys.conflicts` 工具都读这里。
//!
//! 行从热键表（`hotkeys.bindings`，[`crate::hotkey_bindings`]）和命令目录（本体的 [`core_commands`]、插件清单里的
//! `[[commands]]`）来；玩家自己加的（快捷指令、运行自动化，第 3 期 D5，[`crate::hotkey_custom`]）也在表里，每条一行；
//! 提醒助手热键（D10）还存在原来的地方，照旧列出。游戏的聊天键、控制台触发键不是热键、不进表，但参与冲突检查（D2）：
//! 热键和它们一样时，在游戏里按这个键会被 Kotone 拿走。
//!
//! 冲突的规矩（和 [`crate::hotkey::combos_conflict`] 一样）：修饰键集合加主键完全相同才算；
//! 只有 Esc 不看修饰键——录音时按 Esc 取消、她操作游戏时按 Esc 停下，都不管有没有按着 Ctrl / Shift / Alt。

use serde::Serialize;

use crate::hotkey::{combos_conflict, parse_hotkey, VK_ESCAPE};
use crate::hotkey_bindings::{
    legacy_by_id, legacy_for, normalized, Binding, CMD_CALL, CMD_STOP, DEFAULT_BINDINGS,
};
use crate::hotkey_commands::{core_commands, CommandKind, CommandSpec, CORE_SOURCE};
use crate::hotkey_custom::{custom_entries, resolve_row, row_id, Action};
use crate::profile::GameProfile;
use crate::settings::{LegacyUpgrade, Settings};

/// 语音输入热键（按下说话）的 id。它不是具名热键（宿主当主键登记），沿用设置页的 id。
pub const HK_RECORD: &str = "record";
/// 0.1.x 升级用户保留的提醒助手热键。
pub const HK_LEGACY_REMINDER: &str = "legacy-reminder";
/// Esc：固定的键，不能改。
pub const KEY_ESCAPE: &str = "escape";
/// 热键在哪改（ADR 040 D3：只在这一页改；提醒助手热键照旧在提醒助手页，D10）。
pub const HOTKEYS_PAGE: &str = "设置 → 热键";
/// 热键和 Esc 撞上时的话（ADR 040 实施记录，主会话定的第 4 条）。
pub const ESC_REASON: &str = "Esc 留给停下她和取消收音，换一个键";

/// 这个键是谁的。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum KeySource {
    /// Kotone 自己的（呼叫键、停下热键……）。
    Core,
    /// 插件的（清单里 `[[commands]]` 登记的）。
    Plugin,
    /// 玩家自己加的（快捷指令、运行自动化……）。
    Custom,
    /// 固定的键（Esc），只读列出。
    Fixed,
    /// 游戏配置里往游戏里按的键：不是热键，不进表，只参与冲突检查。
    Game,
    /// 表里有、这个版本找不到的（插件没装或被删了、新版本写的命令、第 3 期的工具 / 页面热键）：原样留着，不登记。
    Unknown,
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
    /// 稳定 id：命令就是命令的完整名（`kotone.call`、`voice-input.talk`）；自己加的 `custom.<条目 id>`（第 1、2 期的
    /// `quick-message-n` 改键、`except` 里还认，指快捷指令镜像的第 n 条）、提醒助手热键 `legacy-reminder`、Esc `escape`、
    /// 找不到的 `binding.<序号>`；游戏的键是 `game.<配置 id>.<哪个键>`（频道再加 `.<频道 id>`）。
    /// （`hotkeys.status` 的 `named[].id` 照旧是第 1 期的 id：`assistant`、`channel-cycle`……，对照表 `hotkey_bindings::LEGACY`；
    /// 自己加的就是这里的 id。）
    pub id: String,
    /// 绑的命令（`kotone.call`、`voice-input.talk`）；提醒助手热键、自己加的、Esc、游戏的键没有。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// 自己加的要做什么：`say`（跟她说一句）、`automation`（运行自动化）、`tool`、`page`；别的没有。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<&'static str>,
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
    /// 补充：快捷指令跟她说的话 / 运行的自动化、频道名、Esc 做什么。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub detail: String,
    /// 在哪改（设置说明里的位置，如“琴音 → 唤醒 → 呼叫键”）；Esc 为空（不能改）。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub page: String,
    /// 存在设置里的哪个字段（点分路径）；Esc 和游戏的键为空。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub field: String,
    /// false：插件关着（或命令找不到），这个键现在没登记（插件关着的照样参与冲突检查：打开插件就会撞上）。
    pub enabled: bool,
    /// 和谁冲突（别的项的 id）；只在 [`table`] 的结果里填。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub conflicts: Vec<String>,
    /// 新装时的键（ADR 040 D4：只有呼叫键、停下热键、说话有）；表里“恢复默认”用。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub default_key: String,
    /// 登记状态里的 id（`hotkeys.status` 的 `named[].id`；说话是 `record`，状态在 `key` 里）；Esc、游戏的键、找不到的没有。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub status_id: String,
    /// 按住（按下、松开都要）；登记用，不给界面。
    #[serde(skip)]
    pub hold: bool,
    /// 要语音识别（识别没在跑时只看不吞、按了提示）；登记用，不给界面。
    #[serde(skip)]
    pub listens: bool,
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

/// 一个插件在表里的样子（宿主给：本体不认识具体的插件）：名字、开没开、清单里登记的命令。
#[derive(Debug, Clone, Copy)]
pub struct PluginInfo<'a> {
    pub id: &'a str,
    pub name: &'a str,
    pub enabled: bool,
    pub commands: &'a [CommandSpec],
}

/// 查表要的东西：设置、全部游戏配置、插件（按左侧栏 / 插件表的顺序）。
#[derive(Debug, Clone, Copy)]
pub struct Sources<'a> {
    pub settings: &'a Settings,
    pub profiles: &'a [GameProfile],
    pub plugins: &'a [PluginInfo<'a>],
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

fn row(id: &str, key: &str, label: &str, source: KeySource) -> KeyUse {
    KeyUse {
        id: id.into(),
        command: None,
        action: None,
        key: key.trim().into(),
        combo: combo_of(key),
        source,
        owner: None,
        owner_name: None,
        game_key: None,
        label: label.into(),
        detail: String::new(),
        page: String::new(),
        field: String::new(),
        enabled: true,
        conflicts: Vec::new(),
        default_key: String::new(),
        status_id: String::new(),
        hold: false,
        listens: false,
    }
}

/// 一个命令那一行：键是表里这个命令的第一条（没绑为空）；id、字段、在哪改照对照表（没有旧字段的命令写热键表）。
fn command_row(
    s: &Settings,
    name: &str,
    spec: &CommandSpec,
    plugin: Option<&PluginInfo<'_>>,
) -> KeyUse {
    let legacy = legacy_for(name);
    let field = match legacy {
        Some(_) if name == CMD_CALL && s.ui.legacy_upgrade == LegacyUpgrade::OptedIn => {
            "assistant.companionBindings.hotkey"
        }
        Some(l) => l.field,
        None => "hotkeys.bindings",
    };
    KeyUse {
        command: Some(name.into()),
        owner: plugin.map(|p| p.id.into()),
        owner_name: plugin.map(|p| p.name.into()),
        // 热键都在「设置 → 热键」改（ADR 040 D3）
        page: HOTKEYS_PAGE.into(),
        field: field.into(),
        enabled: plugin.map_or(true, |p| p.enabled),
        hold: spec.kind == CommandKind::Hold,
        listens: spec.listens,
        default_key: DEFAULT_BINDINGS
            .iter()
            .find(|(command, _)| *command == name)
            .map(|(_, key)| key.to_string())
            .unwrap_or_default(),
        status_id: legacy.map_or(name, |l| l.id).into(),
        ..row(
            name,
            s.binding_key(name),
            &spec.name,
            if plugin.is_some() {
                KeySource::Plugin
            } else {
                KeySource::Core
            },
        )
    }
}

/// 表里的全部热键，顺序 = 页面上的顺序 = 登记时谁先拿到键：本体的（Esc 跟在后面）→ 插件的 → 自己加的
/// → 这个版本找不到的。没绑键的命令也列出来（玩家看得到还能设什么）。只列这个玩家用得上的：0.1.x 升级后只用
/// 语音输入的人没有呼叫键、停下热键和快捷指令；提醒助手热键只有升级用户有。
pub fn bindings(src: &Sources<'_>) -> Vec<KeyUse> {
    let settings = normalized(src.settings);
    let s: &Settings = &settings;
    let legacy_only = s.ui.legacy_upgrade.uses_legacy_assistant();
    let mut out = Vec::new();
    for spec in core_commands() {
        let name = format!("{CORE_SOURCE}.{}", spec.id);
        if legacy_only && (name == CMD_CALL || name == CMD_STOP) {
            continue;
        }
        out.push(command_row(s, &name, &spec, None));
    }
    if s.ui.legacy_upgrade.has_legacy_reminders() {
        out.push(KeyUse {
            page: "提醒助手 → 提醒设置".into(),
            field: "assistant.hotkey".into(),
            hold: s.assistant.interaction_mode
                == crate::assistant::AssistantInteractionMode::PushToTalk,
            status_id: HK_LEGACY_REMINDER.into(),
            ..row(
                HK_LEGACY_REMINDER,
                s.legacy_reminder_hotkey(),
                "提醒助手热键",
                KeySource::Core,
            )
        });
    }
    out.push(KeyUse {
        detail: "取消这次录音；她操作游戏时让她停下".into(),
        ..row(KEY_ESCAPE, "Escape", "Esc", KeySource::Fixed)
    });
    for plugin in src.plugins {
        for spec in plugin.commands {
            let name = format!("{}.{}", plugin.id, spec.id);
            out.push(command_row(s, &name, spec, Some(plugin)));
        }
    }
    if !legacy_only {
        for (index, binding, action) in custom_entries(&s.hotkeys) {
            if !action.runnable() {
                continue; // 工具、页面（第 3 期第 2 步）：这个版本还按不了，下面列成“找不到”
            }
            out.push(custom_row(index, binding, action));
        }
    }
    // 这个版本认识的命令（本体的、插件清单里的；0.1.x 用户看不到的呼叫、停下也算认识）
    let known: Vec<String> = core_commands()
        .iter()
        .map(|c| format!("{CORE_SOURCE}.{}", c.id))
        .chain(
            src.plugins
                .iter()
                .flat_map(|p| p.commands.iter().map(move |c| format!("{}.{}", p.id, c.id))),
        )
        .collect();
    for (index, binding) in s.hotkeys.bindings.iter().enumerate() {
        if binding
            .command_name()
            .is_some_and(|c| known.iter().any(|k| k == c))
        {
            continue;
        }
        // 自己加的、这个版本按得了的上面列过了（0.1.x 只用语音输入的人不列：他没有快捷指令）
        if binding.action().is_some_and(|a| a.runnable()) {
            continue;
        }
        out.push(unknown_row(index, binding));
    }
    out
}

/// 名字太长时截一段（行名里放跟她说的话）。
fn short(text: &str) -> String {
    let text = text.trim();
    let mut out: String = text.chars().take(16).collect();
    if text.chars().count() > 16 {
        out.push('…');
    }
    out
}

/// 自己加的那一行（快捷指令、运行自动化）：名字是玩家起的，没起就用要做的事；在「设置 → 热键」改，存在热键表里。
fn custom_row(index: usize, binding: &Binding, action: Action<'_>) -> KeyUse {
    let id = row_id(index, binding);
    let (name, detail) = match action {
        Action::Say(text) => (
            format!("快捷指令「{}」", short(text)),
            text.trim().to_string(),
        ),
        Action::Automation { id, .. } => {
            (format!("运行自动化「{id}」"), format!("运行自动化 {id}"))
        }
        Action::Tool { tool, .. } => (format!("调用 {tool}"), tool.to_string()),
        Action::Page(page) => (format!("打开 {page}"), page.to_string()),
    };
    let label = match binding.label().trim() {
        "" => name,
        label => label.to_string(),
    };
    KeyUse {
        action: Some(action.kind()),
        detail,
        page: HOTKEYS_PAGE.into(),
        field: "hotkeys.bindings".into(),
        // 跟她说一句：按了也会接通通话，要语音识别（ADR 040 实施记录，主会话定的第 2 条）；运行自动化不要（第 3 期 P4）
        listens: matches!(action, Action::Say(_)),
        status_id: id.clone(),
        ..row(&id, binding.key(), &label, KeySource::Custom)
    }
}

/// 这个版本找不到的那一条（插件没装、新版本的命令、第 3 期的工具 / 页面热键）。
fn unknown_row(index: usize, binding: &Binding) -> KeyUse {
    let what = binding
        .command_name()
        .or(binding.tool())
        .or(binding.page())
        .map(str::to_string)
        .unwrap_or_else(|| format!("第 {} 条", index + 1));
    KeyUse {
        command: binding.command_name().map(str::to_string),
        page: HOTKEYS_PAGE.into(),
        field: "hotkeys.bindings".into(),
        enabled: false,
        ..row(
            &format!("binding.{index}"),
            binding.key(),
            &what,
            KeySource::Unknown,
        )
    }
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
                command: None,
                action: None,
                default_key: String::new(),
                status_id: String::new(),
                hold: false,
                listens: false,
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
/// `except` 是正在改的那一项的 id 或命令名（不和自己比）；不传 = 新加的键。
pub fn check(src: &Sources<'_>, key: &str, except: Option<&str>) -> HotkeyCheck {
    let key = key.trim();
    // `quick-message-n`（第 1、2 期的写法）是快捷指令镜像的第 n 条
    let except_row = except.and_then(|e| resolve_row(&normalized(src.settings), e));
    let conflicts = bindings(src)
        .into_iter()
        .chain(game_keys(
            src.profiles,
            src.settings.active_profile_id.as_deref(),
        ))
        .filter(|item| {
            // 正在改的那一项：表里的 id、命令名，或者第 1 期的 id（外部 Agent 照旧文档写 `assistant`、`record`、
            // `quick-message-2`）
            let is_self = except.is_some_and(|e| {
                item.id == e
                    || item.command.as_deref() == Some(e)
                    || legacy_by_id(e).is_some_and(|l| item.command.as_deref() == Some(l.command))
                    || except_row.as_deref() == Some(item.id.as_str())
            });
            !is_self && item.collides(key)
        })
        .collect();
    HotkeyCheck {
        key: key.into(),
        conflicts,
        warnings: common_game_key_warnings(key),
    }
}

/// 识别没在跑时，按了要“听一句”的热键在哪说“还没准备好”。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HintAt {
    /// 她的气泡（呼叫键、快捷指令）。
    Bubble,
    /// 语音输入的悬浮窗（插件的命令）。
    Overlay,
}

/// 登记计划里的一条（宿主和命令行照它登记，谁先拿到键只在这里算一次）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Planned {
    /// 登记用的 id（第 1 期的 id、命令的完整名、`legacy-reminder`、自己加的 `custom.<id>`）。
    pub id: String,
    pub command: Option<String>,
    /// 设置里写的键。
    pub combo: String,
    /// 按下、松开都要。
    pub hold: bool,
    /// 识别没在跑时也登记（停下热键、启停本地语音识别、自己加的不用听的：运行自动化……）。
    pub always: bool,
    /// 要语音识别：识别没在跑时只看不吞，按了在这里提示。
    pub hint: Option<HintAt>,
    /// 没登记的原因（撞了 Esc、被前面的热键占了、命令找不到）。
    pub error: Option<String>,
}

/// 登记计划：表里每个绑了键、插件开着的热键一条，顺序 = 表的顺序 = 谁先拿到键（本体 → 提醒助手 → 插件 → 自己加的）。
/// 撞上 Esc 的不登记（Esc 留给停下她和取消收音）；撞上前面已经拿到的、找不到命令的也不登记，都写原因。
/// 和游戏聊天键一样的照旧登记（表里标红；写入时 [`new_conflict_error`] 会拦新的）。
pub fn registration_plan(src: &Sources<'_>) -> Vec<Planned> {
    let rows = bindings(src);
    let escape = rows.iter().find(|r| r.id == KEY_ESCAPE).cloned();
    let mut taken: Vec<&KeyUse> = Vec::new();
    let mut plan = Vec::new();
    for row in &rows {
        if row.key.is_empty() || row.source == KeySource::Fixed {
            continue;
        }
        if !row.enabled && row.source != KeySource::Unknown {
            continue; // 插件关着：不登记，也不占键
        }
        let error = if row.source == KeySource::Unknown {
            Some("这个版本找不到这个命令（插件没装或被删了，或者是更新的版本加的）".to_string())
        } else if escape.as_ref().is_some_and(|esc| esc.collides(&row.key)) {
            Some(ESC_REASON.to_string())
        } else {
            taken
                .iter()
                .find(|prior| prior.collides(&row.key))
                .map(|prior| format!("和「{}」用的是同一个键，没生效", prior.label))
        };
        if error.is_none() {
            taken.push(row);
        }
        // 不用听的命令和自己加的（停下、启停识别、运行自动化）识别没在跑也登记
        let always = !row.listens
            && match row.source {
                KeySource::Core => row.command.is_some(),
                KeySource::Custom => true,
                _ => false,
            };
        let hint = row.listens.then_some(match row.source {
            KeySource::Plugin => HintAt::Overlay,
            _ => HintAt::Bubble,
        });
        plan.push(Planned {
            // 登记 id 照旧用第 1 期的（钩子、`hotkeys.status` 的 `named[]` 都按它）；没有旧 id 的命令用完整名
            id: row
                .command
                .as_deref()
                .and_then(legacy_for)
                .map_or_else(|| row.id.clone(), |l| l.id.to_string()),
            command: row.command.clone(),
            combo: row.key.clone(),
            hold: row.hold,
            always,
            hint,
            error,
        });
    }
    plan
}

/// 一次修改（`before` → `after`）新造成的冲突：改了键、新加的热键撞上了别的热键、Esc 或游戏配置里的键，就说人话、
/// 说下一步（不让存，ADR 040 实施记录主会话定的第 1 条）。已经冲突着、这次没动的不拦。没有返回 None。
pub fn new_conflict_error(before: &Sources<'_>, after: &Sources<'_>) -> Option<String> {
    let old = bindings(before);
    let rows = bindings(after);
    let games = game_keys(after.profiles, after.settings.active_profile_id.as_deref());
    // 自己加的每条有自己的 id（第 3 期），删一条后面的不挪：按 id 认“这次没动”
    let unchanged = |row: &KeyUse| old.iter().any(|o| o.key == row.key && o.id == row.id);
    for row in &rows {
        if row.key.is_empty()
            || matches!(row.source, KeySource::Fixed | KeySource::Unknown)
            || unchanged(row)
        {
            continue;
        }
        let key = &row.key;
        for other in &rows {
            if other.id == row.id || other.source == KeySource::Unknown || !other.collides(key) {
                continue;
            }
            if other.source == KeySource::Fixed {
                return Some(format!("「{key}」：{ESC_REASON}。"));
            }
            return Some(format!(
                "「{key}」已经是「{}」的热键：先在「设置 → 热键」把它换掉，或者换一个键。",
                other.label
            ));
        }
        if let Some(game) = games.iter().find(|g| g.collides(key)) {
            return Some(format!(
                "「{key}」是《{}》的{}，设成热键以后在游戏里按它会被 Kotone 拿走，换一个键。",
                game.owner_name.as_deref().unwrap_or_default(),
                game.label
            ));
        }
    }
    None
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
