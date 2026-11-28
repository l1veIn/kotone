//! 热键表（ADR 040）：Kotone 里所有的热键、固定的键（Esc）和游戏配置里往游戏里按的键放在一张表里，
//! **冲突只在这里查**（D6）。「设置 → 热键」那一页、各页面改键时的冲突提示、`settings.update` 拒绝新冲突、
//! 宿主和命令行登记热键（[`registration_plan`]）、`hotkeys.status` / `hotkeys.conflicts` 工具都读这里。
//!
//! 行从热键表（`hotkeys.bindings`，[`crate::hotkey_bindings`]）和命令目录（本体的 [`core_commands`]、插件清单里的
//! `[[commands]]`）来；玩家自己加的（快捷指令、运行自动化，第 3 期 D5，[`crate::hotkey_custom`]）也在表里，每条一行。
//! 游戏的聊天键、控制台触发键不是热键、不进表，但参与冲突检查（D2）：
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
use crate::hotkey_custom::{
    command_binding, custom_entries, resolve_row, row_id, Action, CustomState,
};
use crate::hotkey_options::{is_voice_command, nth_id, KeyWhen, VoiceKeyMode};
use crate::profile::GameProfile;
use crate::settings::Settings;

/// 语音输入热键（按下说话）的 id。它不是具名热键（宿主当主键登记），沿用设置页的 id。
pub const HK_RECORD: &str = "record";
/// Esc：固定的键，不能改。
pub const KEY_ESCAPE: &str = "escape";
/// 热键在哪改（ADR 040 D3：只在这一页改）。
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
    /// `quick-message-n` 改键、`except` 里还认，指快捷指令镜像的第 n 条）、Esc `escape`、
    /// 找不到的 `binding.<序号>`；游戏的键是 `game.<配置 id>.<哪个键>`（频道再加 `.<频道 id>`）。
    /// （`hotkeys.status` 的 `named[].id` 照旧是第 1 期的 id：`assistant`、`channel-cycle`……，对照表 `hotkey_bindings::LEGACY`；
    /// 自己加的就是这里的 id。）
    pub id: String,
    /// 绑的命令（`kotone.call`、`voice-input.talk`）；自己加的、Esc、游戏的键没有。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// 自己加的要做什么：`say`（跟她说一句）、`automation`（运行自动化）、`tool`、`page`；插件登记的调工具的命令
    /// （英雄联盟的“喊对面闪现”）是 `tool`；别的没有。
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
    /// 自己加的调工具要玩家在「设置 → 热键」确认一次（D9）：这个工具会做什么。确认之前不登记。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confirm: Option<String>,
    /// 自己加的调工具、打开页面按不了的原因和下一步（找不到、权限里设成了禁止……）。不登记。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocked: Option<String>,
    /// 最近一次按（自己加的；宿主填）：什么时候、成没成、说了什么。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_run: Option<LastRun>,
    /// 什么时候管用（ADR 054）：`always` / `game` / `game:<游戏配置 id>`。游戏配置里的键是那个游戏。
    pub when: KeyWhen,
    /// 呼叫键、说话这两个命令的这一条实际的方式（`auto` / `hold` / `one-shot` / `dictation` / `continuous`；
    /// 说话是旧的混合组合时没有）；别的没有。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<&'static str>,
    /// 这一条写着自己的方式（没写 = 跟着全局，第一条的全局在她的「唤醒」页 / 语音输入页）。
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub own_mode: bool,
    /// 这个命令的第几个键（从 1 数；第 2 个起行 id 是 `<命令>~<n>`）。
    #[serde(skip_serializing_if = "is_first")]
    pub nth: usize,
}

fn is_first(nth: &usize) -> bool {
    *nth <= 1
}

/// 自己加的热键最近一次按下的结果（`hotkeys.status` 的 `bindings[].lastRun`）。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LastRun {
    pub at_ms: u64,
    pub ok: bool,
    /// 给玩家的那句话（成了是工具回的第一句，没成是原因）。
    pub message: String,
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

    /// 在 `when` 的时候按 `key` 会不会撞上这一项：键一样、而且两边会同时管用（ADR 054；Esc 一直管用）。
    pub fn collides_at(&self, key: &str, when: &KeyWhen) -> bool {
        self.when.overlaps(when) && self.collides(key)
    }

    fn collides_with(&self, other: &KeyUse) -> bool {
        self.id != other.id
            && (self.collides_at(&other.key, &other.when)
                || other.collides_at(&self.key, &self.when))
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

/// 查表要的东西：设置、全部游戏配置、插件（按左侧栏 / 插件表的顺序）、自己加的调工具 / 打开页面现在能不能按
/// （宿主照工具目录和权限算，[`CustomState`]；None = 不知道，当能按：按下时宿主还会再判一次）。
#[derive(Clone, Copy)]
pub struct Sources<'a> {
    pub settings: &'a Settings,
    pub profiles: &'a [GameProfile],
    pub plugins: &'a [PluginInfo<'a>],
    pub custom: Option<&'a dyn Fn(&Binding) -> CustomState>,
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
        confirm: None,
        blocked: None,
        last_run: None,
        when: KeyWhen::Always,
        mode: None,
        own_mode: false,
        nth: 1,
    }
}

/// 一个命令的第 `nth` 个键那一行（第 1 个没绑时键为空）：id、字段、在哪改照对照表（没有旧字段的命令写热键表）；
/// 第 2 个起行 id、登记状态 id 加 `~<n>`，存在热键表里，没有默认键。
fn command_row(
    s: &Settings,
    name: &str,
    spec: &CommandSpec,
    plugin: Option<&PluginInfo<'_>>,
    nth: usize,
) -> KeyUse {
    let legacy = legacy_for(name);
    let binding = s.command_binding(name, nth);
    let field = match legacy {
        Some(l) if nth <= 1 => l.field,
        _ => "hotkeys.bindings",
    };
    let mode = s.voice_key_mode(name, nth);
    // 说话是按住的命令（插件照方式决定按住还是按一下）；呼叫键、说话选了自动 / 按住说话的要松开那一下（ADR 054）
    let hold = match mode {
        Some(mode) => mode.uses_release(),
        None => spec.kind == CommandKind::Hold,
    };
    KeyUse {
        command: Some(name.into()),
        owner: plugin.map(|p| p.id.into()),
        owner_name: plugin.map(|p| p.name.into()),
        // 热键都在「设置 → 热键」改（ADR 040 D3）
        page: HOTKEYS_PAGE.into(),
        field: field.into(),
        enabled: plugin.map_or(true, |p| p.enabled),
        hold,
        listens: spec.listens,
        default_key: if nth <= 1 {
            DEFAULT_BINDINGS
                .iter()
                .find(|(command, _)| *command == name)
                .map(|(_, key)| key.to_string())
                .unwrap_or_default()
        } else {
            String::new()
        },
        status_id: nth_id(legacy.map_or(name, |l| l.id), nth),
        when: binding.map(Binding::when).unwrap_or_default(),
        mode: mode.map(VoiceKeyMode::as_str),
        own_mode: is_voice_command(name) && binding.and_then(Binding::mode).is_some(),
        nth,
        ..row(
            &nth_id(name, nth),
            binding.map_or("", Binding::key),
            &spec.name,
            if plugin.is_some() {
                KeySource::Plugin
            } else {
                KeySource::Core
            },
        )
    }
}

/// 一个命令的全部键（没绑也有一行，玩家看得到还能设什么）。
fn command_rows(
    s: &Settings,
    name: &str,
    spec: &CommandSpec,
    plugin: Option<&PluginInfo<'_>>,
) -> Vec<KeyUse> {
    let count = s.command_binding_indexes(name).len().max(1);
    (1..=count)
        .map(|nth| command_row(s, name, spec, plugin, nth))
        .collect()
}

/// 表里的全部热键，顺序 = 页面上的顺序 = 登记时谁先拿到键：本体的（Esc 跟在后面）→ 插件的 → 自己加的
/// → 这个版本找不到的。没绑键的命令也列出来（玩家看得到还能设什么）。只列这个玩家用得上的：0.1.x 升级后只用
/// 语音输入的人没有呼叫键、停下热键和快捷指令。
pub fn bindings(src: &Sources<'_>) -> Vec<KeyUse> {
    let settings = normalized(src.settings);
    let s: &Settings = &settings;
    let legacy_only = s.ui.legacy_upgrade.voice_input_only();
    let mut out = Vec::new();
    for spec in core_commands() {
        let name = format!("{CORE_SOURCE}.{}", spec.id);
        if legacy_only && (name == CMD_CALL || name == CMD_STOP) {
            continue;
        }
        out.extend(command_rows(s, &name, &spec, None));
    }
    out.push(KeyUse {
        detail: "取消这次录音；她操作游戏时让她停下".into(),
        ..row(KEY_ESCAPE, "Escape", "Esc", KeySource::Fixed)
    });
    for plugin in src.plugins {
        for spec in plugin.commands {
            let name = format!("{}.{}", plugin.id, spec.id);
            for mut row in command_rows(s, &name, spec, Some(plugin)) {
                if let Some(binding) = command_binding(spec) {
                    tool_command(&mut row, spec, &binding, src.custom);
                }
                out.push(row);
            }
        }
    }
    if !legacy_only {
        for (index, binding, action) in custom_entries(&s.hotkeys) {
            // 说一句不用问；运行自动化、调工具、打开页面照宿主算的（工具目录、插件开关、权限）
            let state = match (action, src.custom) {
                (Action::Say(_), _) | (_, None) => CustomState::Ready,
                (_, Some(custom)) => custom(binding),
            };
            out.push(custom_row(index, binding, action, state));
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
        // 自己加的上面列过了（0.1.x 只用语音输入的人不列：他没有快捷指令）
        if binding.action().is_some() {
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

/// 自己加的那一行（快捷指令、运行自动化、调工具、打开页面）：名字是玩家起的，没起就用要做的事；在「设置 → 热键」改，
/// 存在热键表里。`state` 是现在能不能按（插件关着的不登记也不占键；要确认、按不了的不登记，写原因）。
fn custom_row(index: usize, binding: &Binding, action: Action<'_>, state: CustomState) -> KeyUse {
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
        // 跟她说一句：按了也会接通通话，要语音识别（ADR 040 实施记录，主会话定的第 2 条）；别的不要（第 3 期 P4）
        listens: matches!(action, Action::Say(_)),
        status_id: id.clone(),
        enabled: state != CustomState::Off,
        confirm: match &state {
            CustomState::NeedsConfirm(reason) => Some(reason.clone()),
            _ => None,
        },
        blocked: match state {
            CustomState::Blocked(reason) => Some(reason),
            _ => None,
        },
        when: binding.when(),
        ..row(&id, binding.key(), &label, KeySource::Custom)
    }
}

/// 插件登记的调工具的命令（英雄联盟的“喊对面闪现”，[`CommandSpec::tool`]）：和自己加的“调一个工具”一样照宿主算的
/// 判（工具在不在、数据开关、权限）。插件登记的预设没有确认这一步：权限里设成每次问的，写原因、不登记。
fn tool_command(
    row: &mut KeyUse,
    spec: &CommandSpec,
    binding: &Binding,
    custom: Option<&dyn Fn(&Binding) -> CustomState>,
) {
    row.action = Some("tool");
    row.detail = spec.description.clone();
    match custom.map_or(CustomState::Ready, |state| state(binding)) {
        CustomState::Ready => {}
        CustomState::Off => row.enabled = false,
        CustomState::NeedsConfirm(reason) => {
            row.blocked = Some(format!(
                "{reason}：热键上没法每次问，去她的「权限」里改成允许"
            ))
        }
        CustomState::Blocked(reason) => row.blocked = Some(reason),
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
        when: binding.when(),
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
                confirm: None,
                blocked: None,
                last_run: None,
                // 游戏配置里的键只在那个游戏里按（ADR 054：只在别的游戏里管用的热键不和它冲突）
                when: KeyWhen::Profile(profile.id.clone()),
                mode: None,
                own_mode: false,
                nth: 1,
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
    check_at(src, key, &KeyWhen::Always, except)
}

/// 同 [`check`]，这个键只在 `when` 的时候管用（ADR 054：只在游戏里的键不和只在别的游戏里的键冲突）。
pub fn check_at(src: &Sources<'_>, key: &str, when: &KeyWhen, except: Option<&str>) -> HotkeyCheck {
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
            // （命令名、第 1 期的 id 指第一个键；第 2 个起用行 id `<命令>~<n>` 或登记 id `<旧 id>~<n>`）
            let is_self = except.is_some_and(|e| {
                item.id == e
                    || (!item.status_id.is_empty() && item.status_id == e)
                    || (item.nth <= 1
                        && (item.command.as_deref() == Some(e)
                            || legacy_by_id(e)
                                .is_some_and(|l| item.command.as_deref() == Some(l.command))))
                    || except_row.as_deref() == Some(item.id.as_str())
            });
            !is_self && item.collides_at(key, when)
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
    /// 登记用的 id（第 1 期的 id、命令的完整名、自己加的 `custom.<id>`）。
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
    /// 什么时候管用（ADR 054）：不是一直的只有低层钩子能登记（门关着时键原样放行）。
    pub when: KeyWhen,
}

/// 登记计划：表里每个绑了键、插件开着的热键一条，顺序 = 表的顺序 = 谁先拿到键（本体 → 插件 → 自己加的）。
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
        } else if let Some(reason) = &row.blocked {
            Some(reason.clone())
        } else if let Some(reason) = &row.confirm {
            // 绑的时候问权限（D9）：玩家确认之前不登记，键照常给游戏
            Some(format!("要先在「{HOTKEYS_PAGE}」确认一次：{reason}"))
        } else if escape.as_ref().is_some_and(|esc| esc.collides(&row.key)) {
            Some(ESC_REASON.to_string())
        } else {
            taken
                .iter()
                .find(|prior| prior.collides_at(&row.key, &row.when))
                .map(|prior| format!("和「{}」用的是同一个键，没生效", prior.label))
        };
        if error.is_none() {
            taken.push(row);
        }
        // 不用听的命令和自己加的（停下、启停识别、运行自动化、插件登记的调工具的命令）识别没在跑也登记
        let always = !row.listens
            && match row.source {
                KeySource::Core => row.command.is_some(),
                KeySource::Custom => true,
                KeySource::Plugin => row.action.is_some(),
                _ => false,
            };
        let hint = row.listens.then_some(match row.source {
            KeySource::Plugin => HintAt::Overlay,
            _ => HintAt::Bubble,
        });
        plan.push(Planned {
            // 登记 id 照旧用第 1 期的（钩子、`hotkeys.status` 的 `named[]` 都按它）；没有旧 id 的命令用完整名；
            // 同一个命令的第 2 个键起加 `~<n>`（ADR 054）
            id: row
                .command
                .as_deref()
                .and_then(legacy_for)
                .map_or_else(|| row.id.clone(), |l| nth_id(l.id, row.nth)),
            command: row.command.clone(),
            combo: row.key.clone(),
            hold: row.hold,
            always,
            hint,
            error,
            when: row.when.clone(),
        });
    }
    plan
}

/// 一次修改（`before` → `after`）新造成的冲突：改了键、新加的热键撞上了别的热键、Esc 或游戏配置里的键，就说人话、
/// 说下一步（不让存，ADR 040 实施记录主会话定的第 1 条）。已经冲突着、这次没动的不拦。没有返回 None。
pub fn new_conflict_error(before: &Sources<'_>, after: &Sources<'_>) -> Option<String> {
    // 写错的方式（ADR 054）先说
    if let Some(error) = crate::hotkey_options::binding_option_error(after.settings) {
        return Some(error);
    }
    let old = bindings(before);
    let rows = bindings(after);
    let games = game_keys(after.profiles, after.settings.active_profile_id.as_deref());
    // 自己加的每条有自己的 id（第 3 期），删一条后面的不挪：按 id 认“这次没动”；“什么时候管用”改了也算动了
    let unchanged = |row: &KeyUse| {
        old.iter()
            .any(|o| o.key == row.key && o.id == row.id && o.when == row.when)
    };
    for row in &rows {
        if row.key.is_empty()
            || matches!(row.source, KeySource::Fixed | KeySource::Unknown)
            || unchanged(row)
        {
            continue;
        }
        let key = &row.key;
        for other in &rows {
            if other.id == row.id
                || other.source == KeySource::Unknown
                || !other.collides_at(key, &row.when)
            {
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
        if let Some(game) = games.iter().find(|g| g.collides_at(key, &row.when)) {
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
