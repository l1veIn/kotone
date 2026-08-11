//! 会话生命周期交互策略（ADR-006）：BeginTrigger × EndTrigger × PostFinalize
//! 正交组装，替代 orchestrator 中 hold/toggle + autoSend 的硬编码分支。
//!
//! - 策略由配置推导：`interactionMode` 预设优先，缺省由 `hotkey.mode` +
//!   `autoSend` 旧字段推导（兼容已有配置与混合组合，行为零变化）；
//! - orchestrator 每次热键事件现场组装策略（settings 热更新无需失效处理）；
//! - B3（VAD 静音判停）由 ADR-007 接入：one-shot / solo 预设生效，VAD 帧判定与
//!   判停状态机见 `crate::vad`；
//! - solo（独奏模式）= one-shot 触发三元组 + continuous 连续标志：发送完成
//!   不停机，自动开下一段回到 Listening；Listening 态点按热键 = 停止会话。

use crate::hotkey::HotkeyMode;
use crate::settings::Settings;

/// 开始触发（ADR-006 决策点 1）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BeginTrigger {
    /// A1：热键按住开始
    HotkeyHold,
    /// A2：热键点按开始
    HotkeyToggle,
    // A3：VAD 检测语音（Phase 3 全时免按，本期不做）
}

/// 结束触发（ADR-006 决策点 2；Esc 取消恒在，不在枚举内）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndTrigger {
    /// B1：松开热键结束
    HotkeyRelease,
    /// B2：再按一次热键结束
    HotkeyPress,
    /// B3：VAD 静音判停（ADR-007；热键按下强制结束恒在兜底）
    VadSilence,
}

/// 转写完成后的处置（ADR-006 决策点 3）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PostFinalize {
    /// C1：直接发送
    SendDirect,
    /// C2：预览确认（单键语义：Preview 态触发键 = 确认发送）
    PreviewConfirm,
}

/// 交互模式预设（ADR-006 §2）：config.json `interactionMode` 的可选值
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum InteractionMode {
    /// 对讲机：A1 + B1 + C1（按住说话，松手直接发）
    #[serde(rename = "push-to-talk")]
    PushToTalk,
    /// 录音笔：A2 + B2 + C2（点按开始，再按结束，预览确认后发）
    #[serde(rename = "dictation")]
    Dictation,
    /// 说一句就走：A2 + B3 + C1（点按开始，VAD 静音判停，转写完直接发；
    /// 热键强制结束恒在兜底）。需要 VAD 接入（vad_factory），否则 begin 报错
    #[serde(rename = "one-shot")]
    OneShot,
    /// 独奏模式：A2 + B3 + C1 + 连续（点按开始持续收音，VAD 每判停一段 →
    /// 转写 → 直发 → 不停机回到监听等下一句；再点按热键 / 停止按钮停止）。
    /// 与 one-shot 的唯一区别是发送完成不停机（continuous = true）
    #[serde(rename = "solo")]
    Solo,
    // hands-free（A3 全时免按）Phase 3
}

/// 生效的热键模式（单源）：`interaction_mode` 预设优先，缺省回落 `hotkey.mode` 旧字段。
///
/// 修复「对讲机模式松开无反应」的根因：预设只管策略（begin/end/post），
/// 若 LL 钩子仍按旧 `hotkey.mode`（默认 toggle）匹配，push-to-talk 下按 F8 只发
/// `Toggle` 事件、松开无事件，会话卡死在 Listening（on_hotkey_toggle 的
/// `end == HotkeyRelease` 分支也不处理再按）。因此钩子注册侧必须用本函数
/// 推导出的模式，与策略保持同一预设源。
///
/// 映射：PushToTalk → Hold（按住开始/松开结束）；Dictation / OneShot / Solo → Toggle
/// （点按开始，结束分别由再按 / VAD 判停 / 再按停止负责）。
pub fn effective_hotkey_mode(settings: &Settings) -> HotkeyMode {
    match settings.interaction_mode {
        Some(InteractionMode::PushToTalk) => HotkeyMode::Hold,
        Some(InteractionMode::Dictation)
        | Some(InteractionMode::OneShot)
        | Some(InteractionMode::Solo) => HotkeyMode::Toggle,
        None => settings.hotkey.mode,
    }
}

/// 组装后的交互策略：orchestrator 的热键路由只读这个结构
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InteractionPolicy {
    pub begin: BeginTrigger,
    pub end: EndTrigger,
    pub post: PostFinalize,
    /// 连续模式（solo）：发送完成不停机，自动开下一段回到 Listening；
    ///  Listening 态点按热键 = 停止会话（不发送在途段）
    pub continuous: bool,
}

impl InteractionPolicy {
    /// 预设策略
    pub fn from_preset(mode: InteractionMode) -> Self {
        match mode {
            InteractionMode::PushToTalk => Self {
                begin: BeginTrigger::HotkeyHold,
                end: EndTrigger::HotkeyRelease,
                post: PostFinalize::SendDirect,
                continuous: false,
            },
            InteractionMode::Dictation => Self {
                begin: BeginTrigger::HotkeyToggle,
                end: EndTrigger::HotkeyPress,
                post: PostFinalize::PreviewConfirm,
                continuous: false,
            },
            InteractionMode::OneShot => Self {
                begin: BeginTrigger::HotkeyToggle,
                end: EndTrigger::VadSilence,
                post: PostFinalize::SendDirect,
                continuous: false,
            },
            InteractionMode::Solo => Self {
                begin: BeginTrigger::HotkeyToggle,
                end: EndTrigger::VadSilence,
                post: PostFinalize::SendDirect,
                continuous: true,
            },
        }
    }

    /// 从配置推导：interactionMode 预设优先；显式 null（自定义兼容路径）
    /// 由 hotkey.mode + autoSend 旧字段推导（混合组合保持既有行为）
    pub fn from_settings(settings: &Settings) -> Self {
        if let Some(mode) = settings.interaction_mode {
            return Self::from_preset(mode);
        }
        let (begin, end) = match settings.hotkey.mode {
            HotkeyMode::Hold => (BeginTrigger::HotkeyHold, EndTrigger::HotkeyRelease),
            HotkeyMode::Toggle => (BeginTrigger::HotkeyToggle, EndTrigger::HotkeyPress),
        };
        let post = if settings.auto_send {
            PostFinalize::SendDirect
        } else {
            PostFinalize::PreviewConfirm
        };
        Self {
            begin,
            end,
            post,
            continuous: false,
        }
    }

    /// 当前策略命中的预设（混合组合为 None，设置页可显示「自定义」）
    pub fn preset(&self) -> Option<InteractionMode> {
        if *self == Self::from_preset(InteractionMode::PushToTalk) {
            Some(InteractionMode::PushToTalk)
        } else if *self == Self::from_preset(InteractionMode::Dictation) {
            Some(InteractionMode::Dictation)
        } else if *self == Self::from_preset(InteractionMode::Solo) {
            // solo 与 one-shot 触发三元组相同，靠 continuous 区分（先判 solo）
            Some(InteractionMode::Solo)
        } else if *self == Self::from_preset(InteractionMode::OneShot) {
            Some(InteractionMode::OneShot)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings_with(mode: HotkeyMode, auto_send: bool) -> Settings {
        let mut s = Settings::default();
        s.hotkey.mode = mode;
        s.auto_send = auto_send;
        s.interaction_mode = None;
        s
    }

    #[test]
    fn legacy_hold_autosend_is_push_to_talk() {
        let p = InteractionPolicy::from_settings(&settings_with(HotkeyMode::Hold, true));
        assert_eq!(p.begin, BeginTrigger::HotkeyHold);
        assert_eq!(p.end, EndTrigger::HotkeyRelease);
        assert_eq!(p.post, PostFinalize::SendDirect);
        assert_eq!(p.preset(), Some(InteractionMode::PushToTalk));
    }

    #[test]
    fn legacy_toggle_preview_is_dictation() {
        let p = InteractionPolicy::from_settings(&settings_with(HotkeyMode::Toggle, false));
        assert_eq!(p.begin, BeginTrigger::HotkeyToggle);
        assert_eq!(p.end, EndTrigger::HotkeyPress);
        assert_eq!(p.post, PostFinalize::PreviewConfirm);
        assert_eq!(p.preset(), Some(InteractionMode::Dictation));
    }

    #[test]
    fn legacy_mixed_combo_has_no_preset() {
        // toggle + autoSend=true（A2+B2+C1）：合法组合但非预设
        let p = InteractionPolicy::from_settings(&settings_with(HotkeyMode::Toggle, true));
        assert_eq!(p.begin, BeginTrigger::HotkeyToggle);
        assert_eq!(p.post, PostFinalize::SendDirect);
        assert_eq!(p.preset(), None);
        // hold + 预览确认（A1+B1+C2）同理
        let p = InteractionPolicy::from_settings(&settings_with(HotkeyMode::Hold, false));
        assert_eq!(p.preset(), None);
    }

    #[test]
    fn explicit_preset_overrides_legacy_fields() {
        let mut s = settings_with(HotkeyMode::Toggle, false);
        s.interaction_mode = Some(InteractionMode::PushToTalk);
        let p = InteractionPolicy::from_settings(&s);
        assert_eq!(
            p,
            InteractionPolicy::from_preset(InteractionMode::PushToTalk)
        );
    }

    #[test]
    fn one_shot_preset_is_a2_b3_c1() {
        let p = InteractionPolicy::from_preset(InteractionMode::OneShot);
        assert_eq!(p.begin, BeginTrigger::HotkeyToggle);
        assert_eq!(p.end, EndTrigger::VadSilence);
        assert_eq!(p.post, PostFinalize::SendDirect);
        assert!(!p.continuous);
        assert_eq!(p.preset(), Some(InteractionMode::OneShot));
        // 显式配置优先生效
        let mut s = settings_with(HotkeyMode::Hold, false);
        s.interaction_mode = Some(InteractionMode::OneShot);
        assert_eq!(InteractionPolicy::from_settings(&s), p);
    }

    #[test]
    fn solo_preset_is_one_shot_plus_continuous() {
        let p = InteractionPolicy::from_preset(InteractionMode::Solo);
        // 触发三元组与 one-shot 相同（A2+B3+C1），唯一区别 continuous=true
        assert_eq!(p.begin, BeginTrigger::HotkeyToggle);
        assert_eq!(p.end, EndTrigger::VadSilence);
        assert_eq!(p.post, PostFinalize::SendDirect);
        assert!(p.continuous);
        // preset() 靠 continuous 区分 solo / one-shot，不串档
        assert_eq!(p.preset(), Some(InteractionMode::Solo));
        assert_eq!(
            InteractionPolicy::from_preset(InteractionMode::OneShot).preset(),
            Some(InteractionMode::OneShot)
        );
        // 显式配置优先生效
        let mut s = settings_with(HotkeyMode::Hold, true);
        s.interaction_mode = Some(InteractionMode::Solo);
        assert_eq!(InteractionPolicy::from_settings(&s), p);
    }

    #[test]
    fn default_config_is_push_to_talk() {
        // Settings::default()（0.1.5 起默认对讲机）推导 = push-to-talk；
        // 老配置缺 interactionMode 键时由 load_from 合并默认值自动迁移
        let p = InteractionPolicy::from_settings(&Settings::default());
        assert_eq!(p.preset(), Some(InteractionMode::PushToTalk));
    }

    #[test]
    fn explicit_null_interaction_mode_uses_legacy_fields() {
        // 显式 null（自定义兼容路径）：仍由 hotkey.mode + autoSend 旧字段推导
        let s = Settings {
            interaction_mode: None,
            ..Settings::default()
        };
        // toggle + autoSend=false → 录音笔
        assert_eq!(
            InteractionPolicy::from_settings(&s).preset(),
            Some(InteractionMode::Dictation)
        );
    }

    #[test]
    fn preset_serde_kebab_case() {
        let j = serde_json::to_string(&InteractionMode::PushToTalk).unwrap();
        assert_eq!(j, "\"push-to-talk\"");
        let m: InteractionMode = serde_json::from_str("\"dictation\"").unwrap();
        assert_eq!(m, InteractionMode::Dictation);
        let m: InteractionMode = serde_json::from_str("\"solo\"").unwrap();
        assert_eq!(m, InteractionMode::Solo);
        assert_eq!(serde_json::to_string(&m).unwrap(), "\"solo\"");
    }

    // ---- effective_hotkey_mode：钩子注册与策略同一预设源（对讲机松开修复） ----

    #[test]
    fn effective_mode_falls_back_to_legacy_hotkey_mode() {
        // interactionMode 缺省：回落 hotkey.mode 旧字段，行为不变
        let s = settings_with(HotkeyMode::Hold, false);
        assert_eq!(effective_hotkey_mode(&s), HotkeyMode::Hold);
        let s = settings_with(HotkeyMode::Toggle, false);
        assert_eq!(effective_hotkey_mode(&s), HotkeyMode::Toggle);
    }

    #[test]
    fn effective_mode_push_to_talk_is_hold() {
        // 预设优先生效：即使 hotkey.mode 旧字段是 toggle（默认值），
        // 对讲机也必须按 hold 匹配，否则松开不发 HoldReleased → 卡死 Listening
        let mut s = settings_with(HotkeyMode::Toggle, false);
        s.interaction_mode = Some(InteractionMode::PushToTalk);
        assert_eq!(effective_hotkey_mode(&s), HotkeyMode::Hold);
    }

    #[test]
    fn effective_mode_dictation_one_shot_solo_are_toggle() {
        // 即使 hotkey.mode 旧字段是 hold，录音笔 / 说一句就走 / 独奏必须按 toggle 匹配
        for mode in [
            InteractionMode::Dictation,
            InteractionMode::OneShot,
            InteractionMode::Solo,
        ] {
            let mut s = settings_with(HotkeyMode::Hold, false);
            s.interaction_mode = Some(mode);
            assert_eq!(effective_hotkey_mode(&s), HotkeyMode::Toggle);
        }
    }
}
