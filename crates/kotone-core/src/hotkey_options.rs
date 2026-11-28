//! 热键表一条上的两样选项（ADR 054）：说话键 / 呼叫键怎么用（`mode`，五种方式），什么时候管用（`when`）。
//!
//! - **方式**：自动 / 按住说话 / 说一句就走 / 录音笔 / 一直听，两个键一套。全局的方式照旧存在
//!   `assistant.callMode`（她）和 `interactionMode`（说话）里，旧值不改名；一条上写了 `mode` 就用它。
//!   第一条的方式就是那两页上选的（[`Settings::set_voice_key_mode`]），说话的“自动”只能写在那一条上
//!   （`interactionMode` 没有“不认识的值按缺省”的保护，旧版本读到会把整份配置当坏的）。
//! - **什么时候**：`always`（缺省）/ `game`（认得的游戏窗口在最前面）/ `game:<游戏配置 id>`。门关着时这个键原样给前台
//!   程序。钩子回调里只读一个原子量（[`foreground_game`]），桌面端定时写它。

use std::ops::RangeInclusive;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde::{Serialize, Serializer};
use serde_json::Value;

use crate::assistant::{CallMode, DEFAULT_CALL_IDLE_SECONDS};
use crate::hotkey_bindings::{Binding, CMD_CALL, CMD_TALK};
use crate::interaction::InteractionMode;
use crate::settings::Settings;

/// 自动：按住超过这么久（毫秒）算按住说话（`hotkeys.holdMs` 的缺省）。
pub const DEFAULT_HOLD_MS: u32 = 300;
/// `hotkeys.holdMs` 能填的范围。
pub const HOLD_MS_RANGE: RangeInclusive<u32> = 150..=1000;

/// 说话键 / 呼叫键怎么用（ADR 054 第 1 条）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoiceKeyMode {
    /// 点一下 = 说一句就走；按住超过 `holdMs` = 按住说话。
    Auto,
    /// 按住说，松开交出去。
    Hold,
    /// 按一下，说完自动停。
    OneShot,
    /// 按一下开始，再按一下结束。
    Dictation,
    /// 一直听：她是接通通话，语音输入是说一句发一句。
    Continuous,
}

impl VoiceKeyMode {
    pub const ALL: [Self; 5] = [
        Self::Auto,
        Self::Hold,
        Self::OneShot,
        Self::Dictation,
        Self::Continuous,
    ];

    /// 热键表里 `mode` 的值；也认两个全局字段的写法（`push-to-talk`、`solo`、`call`）。不认识的为 None。
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value.trim() {
            "auto" => Self::Auto,
            "hold" | "push-to-talk" => Self::Hold,
            "one-shot" => Self::OneShot,
            "dictation" => Self::Dictation,
            "continuous" | "solo" | "call" => Self::Continuous,
            _ => return None,
        })
    }

    /// 写进热键表的值。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Hold => "hold",
            Self::OneShot => "one-shot",
            Self::Dictation => "dictation",
            Self::Continuous => "continuous",
        }
    }

    /// 给玩家看的名字（docs/glossary.md）。
    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "自动",
            Self::Hold => "按住说话",
            Self::OneShot => "说一句就走",
            Self::Dictation => "录音笔",
            Self::Continuous => "一直听",
        }
    }

    /// 要松开那一下（登记成按住的键：按下、松开都要）。
    pub fn uses_release(self) -> bool {
        matches!(self, Self::Auto | Self::Hold)
    }

    pub fn from_call(mode: CallMode) -> Self {
        match mode {
            CallMode::Auto => Self::Auto,
            CallMode::Hold => Self::Hold,
            CallMode::OneShot => Self::OneShot,
            CallMode::Dictation => Self::Dictation,
            CallMode::Call => Self::Continuous,
        }
    }

    pub fn to_call(self) -> CallMode {
        match self {
            Self::Auto => CallMode::Auto,
            Self::Hold => CallMode::Hold,
            Self::OneShot => CallMode::OneShot,
            Self::Dictation => CallMode::Dictation,
            Self::Continuous => CallMode::Call,
        }
    }

    pub fn from_interaction(mode: InteractionMode) -> Self {
        match mode {
            InteractionMode::PushToTalk => Self::Hold,
            InteractionMode::Dictation => Self::Dictation,
            InteractionMode::OneShot => Self::OneShot,
            InteractionMode::Solo => Self::Continuous,
        }
    }

    /// 写进 `interactionMode` 的值；“自动”写不进去（见模块说明）。
    pub fn to_interaction(self) -> Option<InteractionMode> {
        Some(match self {
            Self::Auto => return None,
            Self::Hold => InteractionMode::PushToTalk,
            Self::OneShot => InteractionMode::OneShot,
            Self::Dictation => InteractionMode::Dictation,
            Self::Continuous => InteractionMode::Solo,
        })
    }
}

/// 带方式的两个命令：呼叫她、说话。
pub fn is_voice_command(command: &str) -> bool {
    command == CMD_CALL || command == CMD_TALK
}

/// 自动：松开时这一下算不算按住说话（按了 `hold_ms` 毫秒以上）。短于它 = 点了一下，接着照说一句就走等说完。
pub fn auto_release_ends(held: Duration, hold_ms: u32) -> bool {
    held >= Duration::from_millis(u64::from(hold_ms))
}

// ---------- 什么时候管用 ----------

/// 一个键什么时候管用（ADR 054 第 2 条）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum KeyWhen {
    /// 一直（缺省）。
    #[default]
    Always,
    /// 认得的游戏窗口在最前面时。
    Game,
    /// 这个游戏配置的游戏在最前面时。
    Profile(String),
}

impl KeyWhen {
    /// 热键表里 `when` 的值：没写、空、`always` = 一直；`game`；`game:<id>`；别的字符串当游戏配置 id。
    pub fn parse(value: Option<&str>) -> Self {
        match value.map(str::trim) {
            None | Some("") | Some("always") => Self::Always,
            Some("game") | Some("game:") => Self::Game,
            Some(other) => {
                let id = other.strip_prefix("game:").unwrap_or(other).trim();
                if id.is_empty() {
                    Self::Game
                } else {
                    Self::Profile(id.to_string())
                }
            }
        }
    }

    /// 写进热键表的值（一直 = 不写）。
    pub fn to_value(&self) -> Option<String> {
        match self {
            Self::Always => None,
            Self::Game => Some("game".into()),
            Self::Profile(id) => Some(format!("game:{id}")),
        }
    }

    /// 给界面和她看的写法：`always` / `game` / `game:<id>`。
    pub fn as_string(&self) -> String {
        self.to_value().unwrap_or_else(|| "always".into())
    }

    pub fn is_always(&self) -> bool {
        *self == Self::Always
    }

    /// 两个键会不会同时管用（同一个键两条只在这时算冲突）。
    pub fn overlaps(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Always, _) | (_, Self::Always) => true,
            (Self::Game, _) | (_, Self::Game) => true,
            (Self::Profile(a), Self::Profile(b)) => a == b,
        }
    }

    /// 钩子里用的门。
    pub fn gate(&self) -> KeyGate {
        match self {
            Self::Always => KeyGate::Always,
            Self::Game => KeyGate::AnyGame,
            Self::Profile(id) => KeyGate::Game(game_hash(id)),
        }
    }

    /// 说给玩家听的：“一直”“在游戏里”“在《x》里”（`name` 查游戏名，查不到用 id）。
    pub fn label(&self, name: impl Fn(&str) -> Option<String>) -> String {
        match self {
            Self::Always => "一直".into(),
            Self::Game => "在游戏里".into(),
            Self::Profile(id) => format!("在《{}》里", name(id).unwrap_or_else(|| id.clone())),
        }
    }
}

impl Serialize for KeyWhen {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.as_string())
    }
}

/// 钩子回调里判“现在管不管用”的门：只比一个整数，不做 IO、不拿锁。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum KeyGate {
    #[default]
    Always,
    AnyGame,
    /// 游戏配置 id 的哈希（[`game_hash`]）。
    Game(u64),
}

impl KeyGate {
    /// `foreground` 是 [`foreground_game`] 读到的值（0 = 最前面不是认得的游戏）。
    pub fn open(self, foreground: u64) -> bool {
        match self {
            Self::Always => true,
            Self::AnyGame => foreground != 0,
            Self::Game(hash) => foreground == hash,
        }
    }
}

/// 游戏配置 id → 非零的 64 位哈希（FNV-1a）。
pub fn game_hash(id: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in id.trim().bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash.max(1)
}

static FOREGROUND_GAME: AtomicU64 = AtomicU64::new(0);

/// 最前面是哪个游戏的那个原子量（钩子匹配器读它）。
pub fn foreground_game_cell() -> &'static AtomicU64 {
    &FOREGROUND_GAME
}

/// 桌面端看到前台换了窗口时写：最前面的游戏配置 id（不是游戏为 None）。
pub fn set_foreground_game(id: Option<&str>) {
    FOREGROUND_GAME.store(id.map_or(0, game_hash), Ordering::Relaxed);
}

/// 最前面的游戏（[`game_hash`]，0 = 不是认得的游戏）。
pub fn foreground_game() -> u64 {
    FOREGROUND_GAME.load(Ordering::Relaxed)
}

// ---------- 一条上的选项、同一个命令的几条 ----------

/// 登记 id / 行 id 加上“第几条”：第 1 条就是原来的 id，第 2 条起 `<id>~<n>`。
pub fn nth_id(base: &str, nth: usize) -> String {
    if nth <= 1 {
        base.to_string()
    } else {
        format!("{base}~{nth}")
    }
}

/// [`nth_id`] 反过来：`assistant~2` → (`assistant`, 2)；没有后缀的是第 1 条。
pub fn split_nth(id: &str) -> (&str, usize) {
    match id.rsplit_once('~') {
        Some((base, n)) => match n.parse::<usize>() {
            Ok(n) if n >= 2 => (base, n),
            _ => (id, 1),
        },
        None => (id, 1),
    }
}

impl Binding {
    /// 这一条写的方式原样（没写为 None）。
    pub fn mode_value(&self) -> Option<&str> {
        self.0.get("mode").and_then(Value::as_str)
    }

    /// 这一条写的方式（没写或不认识为 None）。
    pub fn mode(&self) -> Option<VoiceKeyMode> {
        self.mode_value().and_then(VoiceKeyMode::parse)
    }

    /// 这一条什么时候管用（没写 = 一直）。
    pub fn when(&self) -> KeyWhen {
        KeyWhen::parse(self.0.get("when").and_then(Value::as_str))
    }

    fn set_field(&mut self, name: &str, value: Option<String>) {
        if let Some(object) = self.0.as_object_mut() {
            match value {
                Some(value) => {
                    object.insert(name.into(), value.into());
                }
                None => {
                    object.remove(name);
                }
            }
        }
    }

    pub fn set_mode(&mut self, mode: Option<VoiceKeyMode>) {
        self.set_field("mode", mode.map(|m| m.as_str().to_string()));
    }

    pub fn set_when(&mut self, when: &KeyWhen) {
        self.set_field("when", when.to_value());
    }
}

impl Settings {
    /// 这个命令的几条在热键表里的下标（表的顺序 = 第 1 条、第 2 条……）。
    pub fn command_binding_indexes(&self, command: &str) -> Vec<usize> {
        self.hotkeys
            .bindings
            .iter()
            .enumerate()
            .filter(|(_, b)| b.command_name() == Some(command))
            .map(|(index, _)| index)
            .collect()
    }

    /// 这个命令的第 `nth` 条（从 1 数）。
    pub fn command_binding(&self, command: &str, nth: usize) -> Option<&Binding> {
        let index = *self
            .command_binding_indexes(command)
            .get(nth.checked_sub(1)?)?;
        self.hotkeys.bindings.get(index)
    }

    fn command_binding_mut(&mut self, command: &str, nth: usize) -> Option<&mut Binding> {
        let index = *self
            .command_binding_indexes(command)
            .get(nth.checked_sub(1)?)?;
        self.hotkeys.bindings.get_mut(index)
    }

    /// 自动里“按住”的门槛（毫秒，夹在 [`HOLD_MS_RANGE`]）。
    pub fn hold_ms(&self) -> u32 {
        self.hotkeys
            .hold_ms
            .clamp(*HOLD_MS_RANGE.start(), *HOLD_MS_RANGE.end())
    }

    /// 这个命令的全局方式（那两页上选的）。说话是旧的混合组合（`interactionMode` 为 null）时为 None：照旧的路走。
    pub fn global_voice_mode(&self, command: &str) -> Option<VoiceKeyMode> {
        match command {
            CMD_CALL if self.ui.legacy_upgrade.voice_input_only() => Some(VoiceKeyMode::OneShot),
            CMD_CALL => Some(VoiceKeyMode::from_call(
                self.assistant.effective_call_mode(),
            )),
            CMD_TALK => self.interaction_mode.map(VoiceKeyMode::from_interaction),
            _ => None,
        }
    }

    /// 这个命令第 `nth` 条写着的方式（没写、不认识为 None）。
    pub fn binding_mode(&self, command: &str, nth: usize) -> Option<VoiceKeyMode> {
        self.command_binding(command, nth).and_then(Binding::mode)
    }

    /// 这个命令第 `nth` 条实际的方式：那一条写了用它，没写用全局。
    pub fn voice_key_mode(&self, command: &str, nth: usize) -> Option<VoiceKeyMode> {
        if !is_voice_command(command) {
            return None;
        }
        self.binding_mode(command, nth)
            .or_else(|| self.global_voice_mode(command))
    }

    /// 有没有哪一个呼叫键是一直听（全局或某一条）：通话的空闲计时据此决定要不要因为“设置里不再是通话”挂断。
    pub fn call_continuous_anywhere(&self) -> bool {
        let count = self.command_binding_indexes(CMD_CALL).len().max(1);
        (1..=count).any(|nth| self.voice_key_mode(CMD_CALL, nth) == Some(VoiceKeyMode::Continuous))
    }

    /// 选一条的方式（「设置 → 热键」、她）：第一条能写进全局字段就写全局字段、去掉那一条上的 `mode`
    /// （那两页显示的就是它）；说话的“自动”和第二条起写在那一条上。
    pub fn set_voice_key_mode(
        &mut self,
        command: &str,
        nth: usize,
        mode: VoiceKeyMode,
    ) -> Result<(), String> {
        if !is_voice_command(command) {
            return Err(format!("「{command}」没有方式可选，只有呼叫键和说话有"));
        }
        if nth >= 2 {
            let binding = self
                .command_binding_mut(command, nth)
                .ok_or_else(|| format!("{command} 没有第 {nth} 个键"))?;
            binding.set_mode(Some(mode));
            return Ok(());
        }
        let global = match command {
            CMD_CALL => {
                self.assistant.call_mode = mode.to_call();
                // 旧配置的“不通话”（callIdleSeconds 0）换成一直听时用缺省的 30 秒（同她的「唤醒」页）
                if mode == VoiceKeyMode::Continuous && self.assistant.call_idle_seconds == 0 {
                    self.assistant.call_idle_seconds = DEFAULT_CALL_IDLE_SECONDS;
                }
                true
            }
            _ => match mode.to_interaction() {
                Some(interaction) => {
                    self.interaction_mode = Some(interaction);
                    true
                }
                None => false,
            },
        };
        match self.command_binding_mut(command, 1) {
            Some(binding) => binding.set_mode((!global).then_some(mode)),
            None if !global => return Err("先给说话设一个键，再选“自动”".into()),
            None => {}
        }
        Ok(())
    }

    /// 改一条什么时候管用。
    pub fn set_binding_when(
        &mut self,
        command: &str,
        nth: usize,
        when: &KeyWhen,
    ) -> Result<(), String> {
        let binding = self
            .command_binding_mut(command, nth)
            .ok_or_else(|| format!("{command} 没有第 {nth} 个键"))?;
        binding.set_when(when);
        Ok(())
    }

    /// 给命令再加一个键（接在表的最后；同一个命令的顺序 = 表里的顺序）。
    pub fn add_command_binding(
        &mut self,
        command: &str,
        key: &str,
        mode: Option<VoiceKeyMode>,
        when: &KeyWhen,
    ) {
        let mut binding = Binding::command(key.trim(), command);
        if is_voice_command(command) {
            binding.set_mode(mode);
        }
        binding.set_when(when);
        self.hotkeys.bindings.push(binding);
    }

    /// 改命令第 `nth` 个键的键（这一条别的字段不动）。
    pub fn set_command_key(&mut self, command: &str, nth: usize, key: &str) -> Result<(), String> {
        let binding = self
            .command_binding_mut(command, nth)
            .ok_or_else(|| format!("{command} 没有第 {nth} 个键"))?;
        if let Some(object) = binding.0.as_object_mut() {
            object.insert("key".into(), key.trim().into());
        }
        Ok(())
    }

    /// 删掉命令的第 `nth` 个键（别的条目不动）。
    pub fn remove_command_binding(&mut self, command: &str, nth: usize) -> Result<(), String> {
        let index = *self
            .command_binding_indexes(command)
            .get(nth.checked_sub(1).ok_or("第 0 个键不存在")?)
            .ok_or_else(|| format!("{command} 没有第 {nth} 个键"))?;
        self.hotkeys.bindings.remove(index);
        Ok(())
    }

    /// 那两页改了全局方式（这次修改没动热键表）：第一条上写着的 `mode` 去掉，那边选的就是第一个键的方式。
    pub(crate) fn follow_global_voice_modes(&mut self, previous: &Settings) {
        if self.assistant.call_mode != previous.assistant.call_mode {
            if let Some(binding) = self.command_binding_mut(CMD_CALL, 1) {
                binding.set_mode(None);
            }
        }
        if self.interaction_mode != previous.interaction_mode {
            if let Some(binding) = self.command_binding_mut(CMD_TALK, 1) {
                binding.set_mode(None);
            }
        }
    }
}

/// 热键表里写错的选项（保存时说人话拒掉）：`mode` 不认识；`when` 是 `game:` 后面什么都没写的照 `game` 读，不拦。
pub fn binding_option_error(settings: &Settings) -> Option<String> {
    for binding in &settings.hotkeys.bindings {
        let Some(value) = binding.mode_value() else {
            continue;
        };
        if VoiceKeyMode::parse(value).is_none() {
            return Some(format!(
                "热键「{}」的方式「{value}」不认识：可以写 auto（自动）、hold（按住说话）、one-shot（说一句就走）、dictation（录音笔）、continuous（一直听）",
                binding.key()
            ));
        }
    }
    None
}

#[cfg(test)]
#[path = "hotkey_options_tests.rs"]
mod tests;
