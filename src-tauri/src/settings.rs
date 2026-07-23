//! 用户配置读写：~/.kotone/config.json（docs/development.md §5.1、§5.4）

use crate::hotkey::HotkeyMode;

/// 用户配置（字段与 docs/development.md §5.4 config.json 对应）
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub hotkey: HotkeyConfig,
    pub audio_device_id: String,
    /// 当前 STT 引擎 ID，设置页可切换
    pub stt_engine: String,
    /// 引擎专有配置项
    pub engine_options: serde_json::Value,
    /// true: 转写完直接发；false: 先预览确认
    pub auto_send: bool,
    pub active_profile_id: Option<String>,
    pub language: String,
    /// 评测录档开关（默认开）
    pub eval_recording: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HotkeyConfig {
    pub key: String,
    pub mode: HotkeyMode,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            hotkey: HotkeyConfig {
                key: "F8".into(),
                mode: HotkeyMode::Toggle,
            },
            audio_device_id: "default".into(),
            stt_engine: "whisper-cpp-sidecar".into(),
            engine_options: serde_json::json!({}),
            auto_send: false,
            active_profile_id: None,
            language: "zh".into(),
            eval_recording: true,
        }
    }
}

/// 读取配置（不存在则返回默认值）（占位实现）
pub fn load() -> Settings {
    todo!("读取 ~/.kotone/config.json，缺失字段用默认值合并")
}

/// 保存配置（占位实现）
pub fn save(_settings: &Settings) -> Result<(), String> {
    todo!("原子写入 ~/.kotone/config.json")
}
