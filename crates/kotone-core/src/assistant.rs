//! 助手识别边界：只将用户语音转成受限提醒计划，不直接执行输入注入或任意工具。

use std::{future::Future, pin::Pin};

use crate::cancel::CancellationToken;
use crate::interaction::InteractionMode;
pub use crate::reminders::{DeliveryKind, ReminderIntent};

/// 助手每次只接收一条请求；游戏输入独有的 Solo 不进入助手配置。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AssistantInteractionMode {
    /// 兼容旧配置，保存时统一写回 push-to-talk。
    #[serde(alias = "solo")]
    PushToTalk,
    Dictation,
    OneShot,
}

impl From<AssistantInteractionMode> for InteractionMode {
    fn from(mode: AssistantInteractionMode) -> Self {
        match mode {
            AssistantInteractionMode::PushToTalk => Self::PushToTalk,
            AssistantInteractionMode::Dictation => Self::Dictation,
            AssistantInteractionMode::OneShot => Self::OneShot,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantVoiceSession {
    pub session_id: String,
    pub interaction_mode: AssistantInteractionMode,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AssistantConfig {
    pub hotkey: String,
    pub interaction_mode: AssistantInteractionMode,
    pub connection_id: String,
    pub default_delivery: Vec<DeliveryKind>,
    pub voice_id: String,
    pub speech_volume: u16,
    pub speech_rate: i32,
    pub speech_engine: SpeechEngine,
    pub kokoro_voice_id: String,
    /// Retained for configuration compatibility; scrolling uses distance/speed.
    pub danmaku_duration_ms: u32,
    pub danmaku_speed_px_per_sec: u32,
    pub danmaku_font_size: u32,
    /// Distance from the monitor's top edge in logical (CSS) pixels.
    pub danmaku_top_offset_px: u32,
    pub danmaku_color: String,
    pub danmaku_emote: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SpeechEngine {
    Kokoro,
    System,
}

impl Default for AssistantConfig {
    fn default() -> Self {
        Self {
            hotkey: String::new(),
            interaction_mode: AssistantInteractionMode::PushToTalk,
            connection_id: String::new(),
            default_delivery: vec![DeliveryKind::Voice, DeliveryKind::Danmaku],
            voice_id: String::new(),
            speech_volume: 80,
            speech_rate: 0,
            speech_engine: SpeechEngine::System,
            kokoro_voice_id: "kokoro:zf_001".into(),
            danmaku_duration_ms: 8_000,
            danmaku_speed_px_per_sec: 160,
            danmaku_font_size: 32,
            danmaku_top_offset_px: 120,
            danmaku_color: "#FFFFFF".into(),
            danmaku_emote: "none".into(),
        }
    }
}

impl AssistantConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.default_delivery.is_empty() || self.default_delivery.len() > 2 {
            return Err("助手默认提醒方式需选择语音、弹幕或两者".into());
        }
        if self.default_delivery.len() == 2 && self.default_delivery[0] == self.default_delivery[1]
        {
            return Err("助手默认提醒方式不能重复".into());
        }
        if self.speech_volume > 100 {
            return Err("语音音量必须在 0 到 100 之间".into());
        }
        if !(-10..=10).contains(&self.speech_rate) {
            return Err("语音速度必须在 -10 到 10 之间".into());
        }
        if !(1_000..=60_000).contains(&self.danmaku_duration_ms) {
            return Err("弹幕显示时间必须在 1 到 60 秒之间".into());
        }
        if !(40..=600).contains(&self.danmaku_speed_px_per_sec) {
            return Err("弹幕速度必须在 40 到 600 像素/秒之间".into());
        }
        if !(16..=72).contains(&self.danmaku_font_size) {
            return Err("弹幕字号必须在 16 到 72 之间".into());
        }
        if self.danmaku_top_offset_px > 2_160 {
            return Err("弹幕距顶部必须在 0 到 2160 像素之间".into());
        }
        if self.danmaku_color.len() != 7
            || !self.danmaku_color.starts_with('#')
            || !self.danmaku_color[1..]
                .bytes()
                .all(|b| b.is_ascii_hexdigit())
        {
            return Err("弹幕颜色必须为 #RRGGBB 格式".into());
        }
        if !["none", "hello", "cheering", "pointing", "amazed"]
            .contains(&self.danmaku_emote.as_str())
        {
            return Err("请选择内置琴音表情".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionPurpose {
    Dictation,
    Assistant,
}

#[derive(Debug, Clone)]
pub struct AssistantRequest {
    pub session_id: String,
    pub text: String,
    /// 识别完成时刻，提醒相对时间以此计算，不叠加 AI 请求耗时。
    pub received_at_ms: u64,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AssistantPlan {
    /// 无提醒时可携带澄清或暂不支持的说明；不会当作工具指令执行。
    pub message: String,
    pub reminders: Vec<ReminderIntent>,
}

pub type AssistantFuture<'a> =
    Pin<Box<dyn Future<Output = Result<AssistantPlan, String>> + Send + 'a>>;

pub trait AssistantHandler: Send + Sync {
    /// 仅解析计划。持久化提交由会话所有者核验代际与取消状态后完成。
    fn handle<'a>(
        &'a self,
        request: AssistantRequest,
        cancel: CancellationToken,
    ) -> AssistantFuture<'a>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_defaults_and_bounds() {
        let config: AssistantConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(config, AssistantConfig::default());
        assert_eq!(
            config.interaction_mode,
            AssistantInteractionMode::PushToTalk
        );
        assert_eq!(config.speech_engine, SpeechEngine::System);
        config.validate().unwrap();
        assert!(AssistantConfig {
            speech_volume: 101,
            ..config.clone()
        }
        .validate()
        .is_err());
        assert!(AssistantConfig {
            speech_rate: 11,
            ..config.clone()
        }
        .validate()
        .is_err());
        assert!(AssistantConfig {
            default_delivery: vec![],
            ..config
        }
        .validate()
        .is_err());
    }

    #[test]
    fn older_settings_keep_voice_preferences_and_gain_scrolling_defaults() {
        let config: AssistantConfig = serde_json::from_str(
            r##"{"voiceId":"existing-system-voice","speechVolume":65,"danmakuDurationMs":5000}"##,
        )
        .unwrap();
        assert_eq!(config.voice_id, "existing-system-voice");
        assert_eq!(config.speech_volume, 65);
        assert_eq!(config.danmaku_speed_px_per_sec, 160);
        assert_eq!(config.danmaku_top_offset_px, 120);
        assert_eq!(config.danmaku_color, "#FFFFFF");
        assert_eq!(config.speech_engine, SpeechEngine::System);
        config.validate().unwrap();
    }

    #[test]
    fn explicit_assistant_mode_and_speech_engine_roundtrip() {
        for mode in ["push-to-talk", "dictation", "one-shot"] {
            let input = serde_json::json!({"interactionMode":mode,"speechEngine":"kokoro"});
            let config: AssistantConfig = serde_json::from_value(input).unwrap();
            let output = serde_json::to_value(config).unwrap();
            assert_eq!(output["interactionMode"], mode);
            assert_eq!(output["speechEngine"], "kokoro");
        }
        assert!(serde_json::from_value::<AssistantConfig>(
            serde_json::json!({"interactionMode":"invalid"})
        )
        .is_err());
    }

    #[test]
    fn legacy_assistant_solo_becomes_push_to_talk() {
        let config: AssistantConfig =
            serde_json::from_value(serde_json::json!({"interactionMode":"solo"})).unwrap();
        assert_eq!(
            config.interaction_mode,
            AssistantInteractionMode::PushToTalk
        );
        assert_eq!(
            serde_json::to_value(config).unwrap()["interactionMode"],
            "push-to-talk"
        );
        for mode in [
            AssistantInteractionMode::PushToTalk,
            AssistantInteractionMode::Dictation,
            AssistantInteractionMode::OneShot,
        ] {
            assert!(!crate::interaction::InteractionPolicy::from_preset(mode.into()).continuous);
        }
    }

    #[test]
    fn invalid_danmaku_appearance_cannot_be_saved() {
        for patch in [
            serde_json::json!({"danmakuSpeedPxPerSec":0}),
            serde_json::json!({"danmakuSpeedPxPerSec":601}),
            serde_json::json!({"danmakuFontSize":15}),
            serde_json::json!({"danmakuFontSize":73}),
            serde_json::json!({"danmakuTopOffsetPx":2161}),
            serde_json::json!({"danmakuColor":"#FFFFFG"}),
            serde_json::json!({"danmakuColor":"颜色"}),
            serde_json::json!({"danmakuColor":"red; background: black"}),
            serde_json::json!({"danmakuEmote":"../../file"}),
        ] {
            assert!(serde_json::from_value::<AssistantConfig>(patch)
                .unwrap()
                .validate()
                .is_err());
        }
    }

    #[test]
    fn danmaku_top_offset_preserves_exact_values_and_rejects_invalid_pixels() {
        for offset in [0, 1, 120, 2_160] {
            let config: AssistantConfig =
                serde_json::from_value(serde_json::json!({"danmakuTopOffsetPx":offset})).unwrap();
            config.validate().unwrap();
            assert_eq!(config.danmaku_top_offset_px, offset);
            assert_eq!(
                serde_json::to_value(config).unwrap()["danmakuTopOffsetPx"],
                offset
            );
        }
        for offset in [serde_json::json!(-1), serde_json::json!(12.5)] {
            assert!(serde_json::from_value::<AssistantConfig>(
                serde_json::json!({"danmakuTopOffsetPx":offset})
            )
            .is_err());
        }
    }
}
