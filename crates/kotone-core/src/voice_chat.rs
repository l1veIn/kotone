//! 游戏语音设置（ADR 020，config.json `voiceChat` 段）：琴音用虚拟麦克风（VB-CABLE）替玩家在游戏
//! 语音里说话。新字段，缺省 = 关闭；旧版本不认识这一段，原样忽略。

/// 真麦克风音量上限（百分比）。
pub const MAX_MIC_GAIN: u16 = 400;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct VoiceChatConfig {
    /// 开启：把真麦克风（和琴音说给队友的话）转进虚拟麦克风 CABLE Input。
    pub enabled: bool,
    /// 混入玩家自己的麦克风（关掉 = 虚拟麦克风里只有琴音的声音）。
    pub include_mic: bool,
    /// 真麦克风：设备名，`default` = 系统默认录音设备（但不能是 CABLE Output）。
    pub mic_device_id: String,
    /// 真麦克风音量，百分比 0–400。
    pub mic_gain: u16,
    /// 把 Windows 默认麦克风改成 CABLE Output 之前，原来的默认录音设备 ID（“改回”用）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_default_mic: Option<String>,
    /// 原来的默认通信录音设备 ID。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_communications_mic: Option<String>,
}

impl Default for VoiceChatConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            include_mic: true,
            mic_device_id: "default".into(),
            mic_gain: 100,
            previous_default_mic: None,
            previous_communications_mic: None,
        }
    }
}

impl VoiceChatConfig {
    /// 真麦克风音量（倍数）。
    pub fn mic_gain_factor(&self) -> f32 {
        f32::from(self.mic_gain.min(MAX_MIC_GAIN)) / 100.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_configs_get_the_off_defaults_and_unknown_gain_is_capped() {
        let c: VoiceChatConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(c, VoiceChatConfig::default());
        assert!(!c.enabled && c.include_mic);
        let c: VoiceChatConfig = serde_json::from_str(r#"{"micGain": 9000}"#).unwrap();
        assert_eq!(c.mic_gain_factor(), 4.0);
        let v = serde_json::to_value(VoiceChatConfig::default()).unwrap();
        assert!(v.get("previousDefaultMic").is_none());
    }
}
