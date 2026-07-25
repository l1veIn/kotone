//! 会话生命周期交互策略（ADR-006）：BeginTrigger × EndTrigger × PostFinalize
//! 正交组装，替代 orchestrator 中 hold/toggle + autoSend 的硬编码分支。
//!
//! - 策略由配置推导：`interactionMode` 预设优先，缺省由 `hotkey.mode` +
//!   `autoSend` 旧字段推导（兼容已有配置与混合组合，行为零变化）；
//! - orchestrator 每次热键事件现场组装策略（settings 热更新无需失效处理）；
//! - B3（VAD 静音判停）本期只留枚举与路由接口，下一任务接入判停逻辑。

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
    /// B3：VAD 静音判停（下一任务接入；本期无人构造该值，
    /// 路由层按「热键按下强制结束」兜底，保证插入时不改已有分支）
    #[allow(dead_code)]
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
    // one-shot（A2+B3+C1）下一任务；hands-free（A3）Phase 3
}

/// 组装后的交互策略：orchestrator 的热键路由只读这个结构
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InteractionPolicy {
    pub begin: BeginTrigger,
    pub end: EndTrigger,
    pub post: PostFinalize,
}

impl InteractionPolicy {
    /// 预设策略
    pub fn from_preset(mode: InteractionMode) -> Self {
        match mode {
            InteractionMode::PushToTalk => Self {
                begin: BeginTrigger::HotkeyHold,
                end: EndTrigger::HotkeyRelease,
                post: PostFinalize::SendDirect,
            },
            InteractionMode::Dictation => Self {
                begin: BeginTrigger::HotkeyToggle,
                end: EndTrigger::HotkeyPress,
                post: PostFinalize::PreviewConfirm,
            },
        }
    }

    /// 从配置推导：interactionMode 预设优先；缺省由 hotkey.mode + autoSend
    /// 旧字段推导（混合组合保持既有行为）
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
        Self { begin, end, post }
    }

    /// 当前策略命中的预设（混合组合为 None，设置页可显示「自定义」）
    pub fn preset(&self) -> Option<InteractionMode> {
        if *self == Self::from_preset(InteractionMode::PushToTalk) {
            Some(InteractionMode::PushToTalk)
        } else if *self == Self::from_preset(InteractionMode::Dictation) {
            Some(InteractionMode::Dictation)
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
        assert_eq!(p, InteractionPolicy::from_preset(InteractionMode::PushToTalk));
    }

    #[test]
    fn default_config_is_dictation_compatible() {
        // Settings::default()（toggle + autoSend=false）推导 = 录音笔
        let p = InteractionPolicy::from_settings(&Settings::default());
        assert_eq!(p.preset(), Some(InteractionMode::Dictation));
    }

    #[test]
    fn preset_serde_kebab_case() {
        let j = serde_json::to_string(&InteractionMode::PushToTalk).unwrap();
        assert_eq!(j, "\"push-to-talk\"");
        let m: InteractionMode = serde_json::from_str("\"dictation\"").unwrap();
        assert_eq!(m, InteractionMode::Dictation);
    }
}
