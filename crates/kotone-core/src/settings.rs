//! 用户配置读写：~/.kotone/config.json（docs/development.md §5.1、§5.4）
//!
//! 首次运行生成默认配置；缺失字段用默认值合并，保证向前兼容。

use std::path::PathBuf;

use crate::hotkey::HotkeyMode;

/// 热键后端选择（docs/development.md §3.6）。
/// Windows 上 RegisterHotKey 在部分游戏前台不投递事件，LL 钩子（WH_KEYBOARD_LL）是主路径。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HotkeyBackend {
    /// Windows 优先 LL 钩子，安装失败回退 RegisterHotKey；非 Windows 恒 RegisterHotKey
    #[default]
    Auto,
    /// 强制 LL 钩子（失败仍回退并记录日志）
    Llhook,
    /// 强制 RegisterHotKey（tauri-plugin-global-shortcut）
    Register,
}

/// 用户配置（字段与 docs/development.md §5.4 config.json 对应）
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub hotkey: HotkeyConfig,
    /// 热键后端：auto（默认）/ llhook / register
    #[serde(default)]
    pub hotkey_backend: HotkeyBackend,
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
    /// 交互模式预设（ADR-006）：push-to-talk / dictation / one-shot；
    /// 缺省 None = 由 hotkey.mode + autoSend 旧字段推导（兼容混合组合）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interaction_mode: Option<crate::interaction::InteractionMode>,
    /// VAD 静音判停阈值（ms，ADR-007；one-shot 模式生效，默认 700，
    /// 使用时 clamp 到 vad::SILENCE_MS_RANGE）
    #[serde(default = "default_vad_silence_ms")]
    pub vad_silence_ms: u32,
    /// 启动时自动以管理员重启自身（默认关；防循环逻辑见 elevation::should_auto_elevate）
    pub run_as_admin_on_start: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HotkeyConfig {
    pub key: String,
    pub mode: HotkeyMode,
}

impl Default for Settings {
    /// 默认值对齐 docs/development.md §5.4 示例
    fn default() -> Self {
        Self {
            hotkey: HotkeyConfig {
                key: "F8".into(),
                mode: HotkeyMode::Toggle,
            },
            hotkey_backend: HotkeyBackend::Auto,
            audio_device_id: "default".into(),
            stt_engine: "whisper-cpp-sidecar".into(),
            engine_options: serde_json::json!({
                "whisper-cpp-sidecar": { "model": "ggml-small", "threads": 4 },
                "sherpa-onnx-zipformer-zh": { "model": "zipformer-zh-small", "provider": "cpu" }
            }),
            auto_send: false,
            active_profile_id: Some("lol".into()),
            language: "zh".into(),
            eval_recording: true,
            interaction_mode: None,
            vad_silence_ms: default_vad_silence_ms(),
            run_as_admin_on_start: false,
        }
    }
}

fn default_vad_silence_ms() -> u32 {
    crate::vad::DEFAULT_SILENCE_MS
}

/// Kotone 用户数据目录：~/.kotone/
pub fn kotone_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".kotone")
}

fn config_path() -> PathBuf {
    kotone_dir().join("config.json")
}

/// 读取配置：文件不存在时生成默认配置并落盘（首次运行）；
/// 已存在时按默认值合并缺失字段（老版本配置向前兼容）。
pub fn load() -> Settings {
    load_from(&config_path())
}

/// 保存配置（原子写入：先写临时文件再重命名）
pub fn save(settings: &Settings) -> Result<(), String> {
    save_to(&config_path(), settings)
}

/// 从指定路径读取配置（测试可用临时目录）
pub fn load_from(path: &PathBuf) -> Settings {
    if !path.exists() {
        let defaults = Settings::default();
        // 首次运行落盘默认配置；失败不致命，下次启动再试
        let _ = save_to(path, &defaults);
        return defaults;
    }
    let raw = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(_) => return Settings::default(),
    };
    // 以默认值为底、用户配置覆盖，实现缺失字段合并
    let mut merged = serde_json::to_value(Settings::default()).unwrap_or_default();
    if let Ok(user) = serde_json::from_str::<serde_json::Value>(&raw) {
        merge_json(&mut merged, &user);
    }
    serde_json::from_value(merged).unwrap_or_default()
}

/// 写入指定路径（原子写入）
pub fn save_to(path: &PathBuf, settings: &Settings) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建配置目录失败: {e}"))?;
    }
    let json = serde_json::to_string_pretty(settings)
        .map_err(|e| format!("序列化配置失败: {e}"))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json).map_err(|e| format!("写入配置失败: {e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("落盘配置失败: {e}"))?;
    Ok(())
}

/// 浅层递归合并：patch 中的对象键覆盖 base，其余类型整体替换
pub fn merge_json(base: &mut serde_json::Value, patch: &serde_json::Value) {
    match (base, patch) {
        (serde_json::Value::Object(b), serde_json::Value::Object(p)) => {
            for (k, v) in p {
                match b.get_mut(k) {
                    Some(slot) => merge_json(slot, v),
                    None => {
                        b.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        (b, p) => *b = p.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_values_match_doc() {
        let s = Settings::default();
        assert_eq!(s.hotkey.key, "F8");
        assert_eq!(s.hotkey.mode, HotkeyMode::Toggle);
        assert_eq!(s.hotkey_backend, HotkeyBackend::Auto);
        assert_eq!(s.audio_device_id, "default");
        assert_eq!(s.stt_engine, "whisper-cpp-sidecar");
        assert!(!s.auto_send);
        assert_eq!(s.active_profile_id.as_deref(), Some("lol"));
        assert_eq!(s.language, "zh");
        assert!(s.eval_recording);
        assert_eq!(s.vad_silence_ms, 700);
        assert!(s.engine_options["whisper-cpp-sidecar"]["threads"] == 4);
        assert!(!s.run_as_admin_on_start);
    }

    #[test]
    fn roundtrip_serialize() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let mut s = Settings::default();
        s.hotkey.key = "Alt+V".into();
        s.hotkey.mode = HotkeyMode::Hold;
        s.auto_send = true;
        s.stt_engine = "mock-stream".into();
        save_to(&path, &s).unwrap();

        let loaded = load_from(&path);
        assert_eq!(loaded.hotkey.key, "Alt+V");
        assert_eq!(loaded.hotkey.mode, HotkeyMode::Hold);
        assert!(loaded.auto_send);
        assert_eq!(loaded.stt_engine, "mock-stream");
    }

    #[test]
    fn first_run_creates_default_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        assert!(!path.exists());
        let s = load_from(&path);
        assert_eq!(s.hotkey.key, "F8");
        assert!(path.exists(), "首次运行应落盘默认配置");
    }

    #[test]
    fn missing_fields_merged_with_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        // 老版本配置只写了部分字段
        std::fs::write(&path, r#"{ "hotkey": { "key": "F9" }, "language": "en" }"#).unwrap();
        let s = load_from(&path);
        assert_eq!(s.hotkey.key, "F9");
        assert_eq!(s.hotkey.mode, HotkeyMode::Toggle, "缺失字段用默认值合并");
        assert_eq!(s.language, "en");
        assert_eq!(s.stt_engine, "whisper-cpp-sidecar");
        assert!(s.eval_recording);
    }

    #[test]
    fn corrupt_file_falls_back_to_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, "not json {{{").unwrap();
        let s = load_from(&path);
        assert_eq!(s.hotkey.key, "F8");
    }
}
