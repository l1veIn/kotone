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
    /// 【deprecated（UI 已撤）】true: 转写完直接发；false: 先预览确认。
    /// 仅 `interaction_mode = None` 的兼容路径（InteractionPolicy::from_settings
    /// 旧字段推导）还读它；三个交互模式预设下被 PostFinalize 完全覆盖。
    /// CLI wav 模式仍会强制写 false（兼容路径使用），键保留不删。
    pub auto_send: bool,
    pub active_profile_id: Option<String>,
    pub language: String,
    /// 评测录档开关（默认关；需要留语料复现时手动开）
    pub eval_recording: bool,
    /// 交互模式预设（ADR-006）：push-to-talk / dictation / one-shot / solo；
    /// 缺省 None = 由 hotkey.mode + autoSend 旧字段推导（兼容混合组合）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interaction_mode: Option<crate::interaction::InteractionMode>,
    /// VAD 静音判停阈值（ms，ADR-007；one-shot 模式生效，默认 700，
    /// 使用时 clamp 到 vad::SILENCE_MS_RANGE）
    #[serde(default = "default_vad_silence_ms")]
    pub vad_silence_ms: u32,
    /// 启动时自动以管理员重启自身（默认关；防循环逻辑见 elevation::should_auto_elevate）
    pub run_as_admin_on_start: bool,
    /// 识别历史记录（默认 capped/1000 条/不含音频；off = 零开销不记录）
    #[serde(default)]
    pub history: crate::history::HistoryConfig,
    /// 桌面壳 UI 状态（首启向导等；缺省合并默认 = 未完成的向导会弹一次）
    #[serde(default)]
    pub ui: UiConfig,
    /// 模型存储配置（自定义目录；默认空 = ~/.kotone/models）
    #[serde(default)]
    pub models: ModelsConfig,
    /// 模型下载源配置（镜像 / 代理，见 DownloadConfig）
    #[serde(default)]
    pub download: DownloadConfig,
}

/// 桌面壳 UI 状态（config.json `ui` 段）
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiConfig {
    /// 首启向导已完成（或已跳过）；默认 false——老配置升级合并后也会弹一次向导
    #[serde(default)]
    pub first_run_completed: bool,
    /// app 启动后自动进入 Running（warmup 引擎 + 注册热键 + 显示悬浮窗）；默认 false
    #[serde(default)]
    pub auto_start: bool,
}

/// 模型存储配置（config.json `models` 段）
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelsConfig {
    /// 自定义模型目录；空 = 默认 ~/.kotone/models
    #[serde(default)]
    pub dir: String,
}

/// 下载源选择（config.json `download.source`）。
/// 模型文件托管在 HuggingFace / GitHub，国内直连常超时，镜像可显著提速。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DownloadSource {
    /// 镜像优先，失败后自动回退官方源重试一次（默认）
    #[default]
    Auto,
    /// 只用官方源（huggingface.co / github.com）
    Official,
    /// 只用镜像（hf-mirror.com / ghProxy 代理），不回退
    Mirror,
}

/// 模型下载配置（config.json `download` 段）
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadConfig {
    /// 下载源：auto（默认）/ official / mirror
    #[serde(default)]
    pub source: DownloadSource,
    /// GitHub 加速代理前缀（默认 https://ghfast.top/）。
    /// 此类公益代理稳定性无保障、可能随时失效，故做成可配置项：
    /// 失效时换成其他可用前缀即可，无需升级版本。
    #[serde(default = "default_gh_proxy")]
    pub gh_proxy: String,
}

impl Default for DownloadConfig {
    fn default() -> Self {
        Self {
            source: DownloadSource::Auto,
            gh_proxy: default_gh_proxy(),
        }
    }
}

fn default_gh_proxy() -> String {
    "https://ghfast.top/".into()
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
            stt_engine: "sherpa-onnx-x-asr-zh-en".into(),
            engine_options: serde_json::json!({
                "sherpa-onnx-x-asr-zh-en": { "provider": "cpu" }
            }),
            auto_send: false,
            active_profile_id: Some("lol".into()),
            language: "zh".into(),
            eval_recording: false,
            interaction_mode: None,
            vad_silence_ms: default_vad_silence_ms(),
            run_as_admin_on_start: false,
            history: crate::history::HistoryConfig::default(),
            ui: UiConfig::default(),
            models: ModelsConfig::default(),
            download: DownloadConfig::default(),
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
        assert_eq!(s.stt_engine, "sherpa-onnx-x-asr-zh-en");
        assert!(!s.auto_send);
        assert_eq!(s.active_profile_id.as_deref(), Some("lol"));
        assert_eq!(s.language, "zh");
        assert!(!s.eval_recording);
        assert_eq!(s.vad_silence_ms, 700);
        assert!(s.engine_options["sherpa-onnx-x-asr-zh-en"]["provider"] == "cpu");
        assert!(!s.run_as_admin_on_start);
        assert_eq!(s.history.mode, crate::history::HistoryMode::Capped);
        assert_eq!(s.history.max_records, 1000);
        assert!(!s.history.include_audio);
        assert!(!s.ui.first_run_completed);
        assert!(!s.ui.auto_start);
        assert!(s.models.dir.is_empty(), "默认模型目录为空 = ~/.kotone/models");
        assert_eq!(s.download.source, DownloadSource::Auto);
        assert_eq!(s.download.gh_proxy, "https://ghfast.top/");
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
        assert_eq!(s.stt_engine, "sherpa-onnx-x-asr-zh-en");
        assert!(!s.eval_recording);
        assert_eq!(s.download.source, DownloadSource::Auto, "老配置缺 download 段合并默认");
        assert_eq!(s.download.gh_proxy, "https://ghfast.top/");
        assert!(!s.ui.first_run_completed, "老配置缺 ui 段合并默认 = 未完成");
    }

    #[test]
    fn download_config_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let mut s = Settings::default();
        s.download.source = DownloadSource::Mirror;
        s.download.gh_proxy = "https://gh-proxy.example.com/".into();
        save_to(&path, &s).unwrap();
        let loaded = load_from(&path);
        assert_eq!(loaded.download.source, DownloadSource::Mirror);
        assert_eq!(loaded.download.gh_proxy, "https://gh-proxy.example.com/");
        // 序列化键名 camelCase：ghProxy / source 小写枚举
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains("\"ghProxy\""));
        assert!(raw.contains("\"mirror\""));
    }

    #[test]
    fn download_source_rejects_unknown_value() {
        // 非法枚举值整体回退默认（serde 反序列化失败 → unwrap_or_default）
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(
            &path,
            r#"{ "download": { "source": "fastest", "ghProxy": "https://x/" } }"#,
        )
        .unwrap();
        let s = load_from(&path);
        assert_eq!(s.download.source, DownloadSource::Auto);
        assert_eq!(s.download.gh_proxy, "https://ghfast.top/");
    }

    #[test]
    fn ui_first_run_completed_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let mut s = Settings::default();
        s.ui.first_run_completed = true;
        save_to(&path, &s).unwrap();
        let loaded = load_from(&path);
        assert!(loaded.ui.first_run_completed);
    }

    #[test]
    fn auto_start_and_models_dir_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let mut s = Settings::default();
        s.ui.auto_start = true;
        s.models.dir = "D:\\kotone-models".into();
        save_to(&path, &s).unwrap();
        let loaded = load_from(&path);
        assert!(loaded.ui.auto_start);
        assert_eq!(loaded.models.dir, "D:\\kotone-models");
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
