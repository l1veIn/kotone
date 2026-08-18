//! 用户配置读写：~/.kotone/config.json（docs/development.md §5.1、§5.4）
//!
//! 首次运行生成默认配置；缺失字段用默认值合并，保证向前兼容。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use crate::hotkey::HotkeyMode;

/// 热键后端选择（docs/development.md §3.6）。
/// Windows 上 RegisterHotKey 在部分游戏前台不投递事件且不支持鼠标侧键，
/// LL 键鼠钩子（WH_KEYBOARD_LL + WH_MOUSE_LL）是主路径。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HotkeyBackend {
    /// Windows 优先 LL 钩子，安装失败回退 RegisterHotKey；非 Windows 恒 RegisterHotKey
    #[default]
    Auto,
    /// 强制 LL 键鼠钩子（普通键失败仍回退并记录日志）
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
    /// 当前 STT 引擎 ID（由活动模型推导后回写，兼容旧配置/CLI）
    pub stt_engine: String,
    /// 当前活动模型 ID。空 = 仍按 sttEngine + engineOptions.model 推导。
    #[serde(default)]
    pub active_model_id: String,
    /// 按模型 id 存放的 configSchema 值（语言、连接 id 等）。密钥不进这里。
    #[serde(default)]
    pub model_configs: std::collections::HashMap<String, serde_json::Value>,
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
    /// 默认「对讲机」（0.1.5 起，修复跳过引导后无模式的问题：老配置缺键时
    /// 由 load_from 合并默认值自动迁移）。显式 null = 自定义兼容路径，
    /// 由 hotkey.mode + autoSend 旧字段推导（见 InteractionPolicy::from_settings）
    #[serde(default)]
    pub interaction_mode: Option<crate::interaction::InteractionMode>,
    /// VAD 静音判停阈值（ms，ADR-007；one-shot 模式生效，默认 700，
    /// 使用时 clamp 到 vad::SILENCE_MS_RANGE）
    #[serde(default = "default_vad_silence_ms")]
    pub vad_silence_ms: u32,
    /// 启动时自动以管理员重启自身（默认关；防循环逻辑见 elevation::should_auto_elevate）
    pub run_as_admin_on_start: bool,
    /// 频道切换热键（ADR-008；默认 Shift+CapsLock）。按声明顺序循环切换当前
    /// profile 的聊天频道；与录制热键冲突时后端拒绝注册并在状态里给出错误。
    #[serde(default = "default_channel_cycle_hotkey")]
    pub channel_cycle_hotkey: String,
    /// 重发最近一条热键（默认空 = 关闭）。Idle 时按下把历史最新一条发送成功的
    /// 文本重新注入到当前前台窗口；与录制/频道切换热键冲突时拒绝注册。
    #[serde(default)]
    pub resend_last_hotkey: String,
    /// 「以管理员权限重启」启动提示：用户勾选「不再提示」后置 true，不再弹窗（默认 false）
    #[serde(default)]
    pub admin_prompt_dismissed: bool,
    /// 提权重启成功后的「启动时自动请求管理员权限」询问：用户勾选「不再提醒」
    /// 后置 true，不再弹窗（默认 false；与 admin_prompt_dismissed 独立，两段式提示各记各的）
    #[serde(default)]
    pub auto_admin_prompt_dismissed: bool,
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
    /// 悬浮窗配置（显示模式等，见 OverlayConfig）
    #[serde(default)]
    pub overlay: OverlayConfig,
    /// silero VAD 内部参数（ADR-007；one-shot/solo 生效）。
    /// 与 `vad_silence_ms`（判停阈值，SilenceStopTracker 用）独立——
    /// 这里控制 silero 模型的帧级判定灵敏度。默认对齐 kotone-stt vad.rs 原硬编码值。
    #[serde(default)]
    pub vad: VadConfig,
    /// 热词命中加分（默认 3.5；越高热词越易命中，也越容易把背景噪声
    /// 识别成热词。X-ASR 等引擎 recognizer 级配置，改后需「停止→启动」重建）
    #[serde(default = "default_hotwords_score")]
    pub hotwords_score: f32,
    /// STT 最终文本与预览/发送之间的后处理；默认开启内置「屏蔽词」流程。
    #[serde(default)]
    pub post_processing: crate::postprocess::PostProcessingConfig,
    /// 后处理在线连接目录（不含 API key）。缺省空列表，旧配置零行为变化。
    #[serde(default)]
    pub connections: Vec<crate::connection::Connection>,
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

/// 悬浮窗显示模式（config.json `overlay.visibility`，docs/development.md §3.6）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverlayVisibility {
    /// 常驻：启动即显示，停止才隐藏
    Always,
    /// 用时浮现（默认）：平时隐藏；收音/转写/发送时浮现；一次发送完成（成功或失败）
    /// 延迟 ~600ms 自动隐藏；solo 连续模式保持显示直到会话停止
    #[default]
    OnDemand,
    /// 完全不显示：热键和语音输入照常工作，所有悬浮窗唤起事件均被忽略
    Never,
}

/// 悬浮窗样式（config.json `overlay.style`）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverlayStyle {
    /// 卡片：480×120 圆角面板，屏幕中央，贴纸/气泡等完整装饰
    Card,
    /// 胶囊（默认）：Win11 语音输入条风格——水平居中靠下（底部留 ~48px），
    /// 圆角胶囊宽度随内容伸缩（窗口上限 520px），轻装饰
    #[default]
    Capsule,
}

/// 悬浮窗固定位置（config.json `overlay.position`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverlayPosition {
    /// 跟随样式：卡片居中，胶囊底部居中。
    #[default]
    Auto,
    TopLeft,
    TopCenter,
    TopRight,
    Center,
    BottomLeft,
    BottomCenter,
    BottomRight,
    /// 用户拖动后记录物理屏幕坐标。
    Custom,
}

/// 悬浮窗配置（config.json `overlay` 段）
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayConfig {
    /// 显示模式：on_demand 用时浮现（默认）/ always 常驻 / never 完全隐藏
    #[serde(default)]
    pub visibility: OverlayVisibility,
    /// 样式：capsule 胶囊（默认）/ card 卡片
    #[serde(default)]
    pub style: OverlayStyle,
    /// 固定位置预设；拖动完成后切为 custom。
    #[serde(default)]
    pub position: OverlayPosition,
    /// 是否允许直接拖动悬浮窗。
    #[serde(default = "default_true")]
    pub draggable: bool,
    /// 鼠标点击穿透到游戏；开启时悬浮窗自身按钮和拖动不可用。
    #[serde(default)]
    pub click_through: bool,
    /// custom 位置的物理屏幕坐标（支持多显示器负坐标）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_x: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_y: Option<i32>,
}

impl Default for OverlayConfig {
    fn default() -> Self {
        Self {
            visibility: OverlayVisibility::default(),
            style: OverlayStyle::default(),
            position: OverlayPosition::default(),
            draggable: true,
            click_through: false,
            custom_x: None,
            custom_y: None,
        }
    }
}

impl OverlayConfig {
    /// UI 只保留一个「允许交互并拖动」开关：
    /// - 允许拖动 = 不穿透；
    /// - 不允许拖动 = 鼠标穿透。
    ///
    /// 老配置可能同时写了两个独立字段，迁移时点击穿透优先，避免升级后突然拦截游戏鼠标。
    pub fn normalize_interaction(&mut self) {
        if self.click_through {
            self.draggable = false;
        } else {
            self.click_through = !self.draggable;
        }
    }
}

/// silero VAD 内部参数（config.json `vad` 段；ADR-007）。
/// 与 `vad_silence_ms`（判停阈值，SilenceStopTracker 用）独立。
/// 默认值 = kotone-stt vad.rs 原硬编码值。
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VadConfig {
    /// 语音判定阈值（0.1–0.9，默认 0.5）。调高 → 更严格（背景噪声更难
    /// 被判成语音）；调低 → 更宽松、更容易误判。
    #[serde(default = "default_vad_threshold")]
    pub threshold: f32,
    /// 最短语音时长 ms（20–500，默认 50）。拉长可过滤短促噪声突发。
    #[serde(default = "default_vad_min_speech_ms")]
    pub min_speech_ms: u32,
    /// 最短静音时长 ms（20–500，默认 50）。
    #[serde(default = "default_vad_min_silence_ms")]
    pub min_silence_ms: u32,
}

impl Default for VadConfig {
    fn default() -> Self {
        Self {
            threshold: default_vad_threshold(),
            min_speech_ms: default_vad_min_speech_ms(),
            min_silence_ms: default_vad_min_silence_ms(),
        }
    }
}

fn default_vad_threshold() -> f32 {
    0.5
}

fn default_vad_min_speech_ms() -> u32 {
    50
}

fn default_vad_min_silence_ms() -> u32 {
    50
}

/// 热词命中加分默认值（Sherpa 官方示例常用强度；SessionConfig 与
/// settings.hotwords_score 的共同真源，改动需两侧同步）
pub fn default_hotwords_score() -> f32 {
    3.5
}

/// 下载源选择（config.json `download.source`）。
/// 大模型优先使用 ModelScope 国内镜像，其他资源仍可使用 HF / GitHub 镜像。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DownloadSource {
    /// ModelScope / 其他镜像优先，失败后自动回退官方源（默认）
    #[default]
    Auto,
    /// 只用官方源（huggingface.co / github.com）
    Official,
    /// 只用镜像（ModelScope / hf-mirror.com / ghProxy），不回退
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

fn default_true() -> bool {
    true
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
                key: "CapsLock".into(),
                mode: HotkeyMode::Toggle,
            },
            hotkey_backend: HotkeyBackend::Auto,
            audio_device_id: "default".into(),
            stt_engine: "sherpa-streaming".into(),
            active_model_id: String::new(),
            model_configs: HashMap::new(),
            engine_options: serde_json::json!({
                "sherpa-streaming": { "provider": "cpu" }
            }),
            auto_send: false,
            active_profile_id: Some("lol".into()),
            language: "zh".into(),
            eval_recording: false,
            interaction_mode: Some(crate::interaction::InteractionMode::PushToTalk),
            vad_silence_ms: default_vad_silence_ms(),
            run_as_admin_on_start: false,
            channel_cycle_hotkey: default_channel_cycle_hotkey(),
            resend_last_hotkey: String::new(), // 默认关闭（设置里录入）
            admin_prompt_dismissed: false,
            auto_admin_prompt_dismissed: false,
            history: crate::history::HistoryConfig::default(),
            ui: UiConfig::default(),
            models: ModelsConfig::default(),
            download: DownloadConfig::default(),
            overlay: OverlayConfig::default(),
            vad: VadConfig::default(),
            hotwords_score: default_hotwords_score(),
            post_processing: crate::postprocess::PostProcessingConfig::default(),
            connections: Vec::new(),
        }
    }
}

impl Settings {
    /// 引擎专有项 + 当前活动模型的 configSchema 值。
    /// 旧家族 id 与新 I/O 引擎 id 的 engineOptions 会合并；活动模型 id 写入 `model`。
    pub fn session_options(&self, engine_id: &str) -> serde_json::Value {
        let canonical = crate::stt::canonical_stt_engine(engine_id);
        let mut opts = serde_json::json!({});
        for key in crate::stt::stt_engine_option_keys(engine_id)
            .iter()
            .copied()
            .chain(std::iter::once(engine_id))
            .chain(std::iter::once(canonical))
        {
            if let Some(extra) = self.engine_options.get(key) {
                merge_json_objects(&mut opts, extra);
            }
        }
        if !opts.is_object() {
            opts = serde_json::json!({});
        }
        let model_id = if !self.active_model_id.trim().is_empty() {
            self.active_model_id.clone()
        } else {
            opts.get("model")
                .and_then(|m| m.as_str())
                .unwrap_or("")
                .to_string()
        };
        if let Some(cfg) = self.model_configs.get(&model_id) {
            if let (Some(base), Some(extra)) = (opts.as_object_mut(), cfg.as_object()) {
                for (k, v) in extra {
                    base.insert(k.clone(), v.clone());
                }
            }
        }
        if !model_id.is_empty() {
            if let Some(base) = opts.as_object_mut() {
                base.insert("model".into(), serde_json::json!(model_id));
            }
        }
        opts
    }
}

fn merge_json_objects(base: &mut serde_json::Value, extra: &serde_json::Value) {
    match (base.as_object_mut(), extra.as_object()) {
        (Some(dst), Some(src)) => {
            for (k, v) in src {
                dst.insert(k.clone(), v.clone());
            }
        }
        (None, Some(_)) => *base = extra.clone(),
        _ => {}
    }
}

/// 从当前 Settings 快照解析连接公开字段。密钥由后续凭据层补上。
pub struct SettingsConnectionResolver {
    settings: Arc<RwLock<Settings>>,
}

impl SettingsConnectionResolver {
    pub fn new(settings: Arc<RwLock<Settings>>) -> Self {
        Self { settings }
    }
}

impl crate::connection::ConnectionResolver for SettingsConnectionResolver {
    fn resolve(
        &self,
        connection_id: &str,
    ) -> Result<crate::connection::ResolvedConnection, String> {
        let id = connection_id.trim();
        if id.is_empty() {
            return Err("连接 ID 不能为空".into());
        }
        let settings = self.settings.read().unwrap();
        let found = settings
            .connections
            .iter()
            .find(|connection| connection.id == id)
            .ok_or_else(|| format!("未找到连接：{id}"))?;
        found.validate()?;
        Ok(crate::connection::ResolvedConnection::from_public(found))
    }
}

fn default_vad_silence_ms() -> u32 {
    crate::vad::DEFAULT_SILENCE_MS
}

/// 频道切换热键默认值（ADR-008）
fn default_channel_cycle_hotkey() -> String {
    "Shift+CapsLock".into()
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

/// 配置加载结果。损坏配置会被移动到 `.corrupt` 备份，并通过 `warning`
/// 交给桌面端显示；调用方不能再无声地用默认值覆盖原文件。
#[derive(Debug)]
pub struct SettingsLoadResult {
    pub settings: Settings,
    pub warning: Option<String>,
}

/// 串行化单进程内的配置修改，并保证磁盘写入、外部资源切换与内存发布按事务执行。
///
/// 外部资源（例如全局热键）无法和文件系统组成真正的原子事务，因此失败时会同时
/// 尝试恢复旧资源和旧配置；内存快照只会在全部步骤成功后发布。
pub struct SettingsRepository {
    current: Arc<RwLock<Settings>>,
    path: PathBuf,
    transaction: Mutex<()>,
}

impl SettingsRepository {
    pub fn new(current: Arc<RwLock<Settings>>) -> Self {
        Self::new_at(current, config_path())
    }

    pub fn new_at(current: Arc<RwLock<Settings>>, path: PathBuf) -> Self {
        Self {
            current,
            path,
            transaction: Mutex::new(()),
        }
    }

    pub fn snapshot(&self) -> Settings {
        self.current.read().unwrap().clone()
    }

    /// 仅修改配置的常用事务。
    pub fn update<M>(&self, mutate: M) -> Result<Settings, String>
    where
        M: FnOnce(&mut Settings) -> Result<(), String>,
    {
        self.update_with(mutate, |_, _| Ok(()), |_, _| Ok(()))
            .map(|(_, next, ())| next)
    }

    /// 带外部资源切换的配置事务。
    ///
    /// 顺序为：从最新内存快照构造 next → 写盘 next → apply → 发布 next。
    /// apply 失败时调用 rollback，并把磁盘恢复为 old；返回值同时携带 old/next，
    /// 便于调用方在提交后更新窗口等非关键 UI 副作用。
    pub fn update_with<M, A, R, T>(
        &self,
        mutate: M,
        apply: A,
        rollback: R,
    ) -> Result<(Settings, Settings, T), String>
    where
        M: FnOnce(&mut Settings) -> Result<(), String>,
        A: FnOnce(&Settings, &Settings) -> Result<T, String>,
        R: FnOnce(&Settings, &Settings) -> Result<(), String>,
    {
        let _transaction = self.transaction.lock().unwrap();
        let old = self.current.read().unwrap().clone();
        let mut next = old.clone();
        mutate(&mut next)?;
        save_to(&self.path, &next)?;

        let applied = match apply(&old, &next) {
            Ok(value) => value,
            Err(apply_error) => {
                let rollback_resource_error = rollback(&old, &next).err();
                let rollback_file_error = save_to(&self.path, &old).err();
                let mut message = format!("应用配置失败，已回滚: {apply_error}");
                if let Some(error) = rollback_resource_error {
                    message.push_str(&format!("；恢复外部资源失败: {error}"));
                }
                if let Some(error) = rollback_file_error {
                    message.push_str(&format!("；恢复配置文件失败: {error}"));
                }
                return Err(message);
            }
        };

        *self.current.write().unwrap() = next.clone();
        Ok((old, next, applied))
    }
}

/// 读取配置：文件不存在时生成默认配置并落盘（首次运行）；
/// 已存在时按默认值合并缺失字段（老版本配置向前兼容）。
pub fn load() -> Settings {
    load_with_diagnostic().settings
}

/// 桌面端启动使用：除配置外返回可直接展示给用户的恢复诊断。
pub fn load_with_diagnostic() -> SettingsLoadResult {
    load_from_with_diagnostic(&config_path())
}

/// 保存配置（原子写入：先写临时文件再重命名）
pub fn save(settings: &Settings) -> Result<(), String> {
    save_to(&config_path(), settings)
}

/// 从指定路径读取配置（测试可用临时目录）
pub fn load_from(path: &Path) -> Settings {
    load_from_with_diagnostic(path).settings
}

pub fn load_from_with_diagnostic(path: &Path) -> SettingsLoadResult {
    if !path.exists() {
        let defaults = Settings::default();
        // 首次运行落盘默认配置；失败不致命，下次启动再试
        let _ = save_to(path, &defaults);
        return SettingsLoadResult {
            settings: defaults,
            warning: None,
        };
    }
    let raw = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(error) => return recover_invalid_config(path, format!("读取失败: {error}")),
    };
    // 以默认值为底、用户配置覆盖，实现缺失字段合并
    let mut merged = serde_json::to_value(Settings::default()).unwrap_or_default();
    let user = match serde_json::from_str::<serde_json::Value>(&raw) {
        Ok(user) => user,
        Err(error) => return recover_invalid_config(path, format!("JSON 解析失败: {error}")),
    };
    let mut user = user;
    migrate_legacy_post_processing_json(&mut user);
    merge_json(&mut merged, &user);
    let mut settings: Settings = match serde_json::from_value(merged) {
        Ok(settings) => settings,
        Err(error) => return recover_invalid_config(path, format!("配置项不合法: {error}")),
    };
    settings.overlay.normalize_interaction();
    migrate_legacy_engine_ids(&mut settings);
    settings.post_processing.normalize_active_id();
    SettingsLoadResult {
        settings,
        warning: None,
    }
}

/// 旧配置只有单条 `pipeline`：空流程改种默认屏蔽词；有步骤则收成一条可切换流程。
fn migrate_legacy_post_processing_json(user: &mut serde_json::Value) {
    let Some(post) = user.get("postProcessing") else {
        return;
    };
    if post.get("pipelines").is_some() {
        return;
    }
    let enabled = post.get("enabled").and_then(|value| value.as_bool());
    let legacy = post
        .get("pipeline")
        .cloned()
        .and_then(|value| serde_json::from_value::<crate::postprocess::PipelineConfig>(value).ok());
    match legacy {
        Some(pipeline) if !pipeline.steps.is_empty() => {
            if let Some(post) = user.get_mut("postProcessing") {
                *post =
                    serde_json::to_value(crate::postprocess::PostProcessingConfig::with_pipeline(
                        enabled.unwrap_or(true),
                        pipeline,
                    ))
                    .unwrap_or_else(|_| serde_json::json!({}));
            }
        }
        _ => {
            if let Some(object) = user.as_object_mut() {
                object.remove("postProcessing");
            }
        }
    }
}

/// 旧家族引擎 id / `online-asr` 伞：加载时改写成现在的引擎 id，并把旧键选项抄到新键。
fn migrate_legacy_engine_ids(s: &mut Settings) {
    let from = s.stt_engine.clone();
    let to = match from.as_str() {
        "sherpa-onnx-x-asr-zh-en" => "sherpa-streaming",
        "sherpa-onnx-sensevoice" | "sherpa-onnx-funasr-nano" => "sherpa-offline",
        "online-asr" => {
            let hinted = s
                .engine_options
                .get("online-asr")
                .and_then(|v| v.get("model"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if s.active_model_id == "volcano-asr" || hinted == "volcano-asr" {
                "volcano-asr"
            } else if s.active_model_id == "iflytek-asr" || hinted == "iflytek-asr" {
                "iflytek-asr"
            } else {
                "volcano-asr"
            }
        }
        _ => return,
    };
    copy_engine_options_key(&mut s.engine_options, &from, to);
    if s.active_model_id.trim().is_empty() {
        if let Some(model) = s
            .engine_options
            .get(&from)
            .and_then(|v| v.get("model"))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|id| !id.is_empty())
        {
            s.active_model_id = model.to_string();
        }
    }
    s.stt_engine = to.to_string();
}

fn copy_engine_options_key(opts: &mut serde_json::Value, from: &str, to: &str) {
    let Some(src) = opts.get(from).cloned() else {
        return;
    };
    if from == to {
        return;
    }
    if !opts.is_object() {
        *opts = serde_json::json!({});
    }
    let Some(base) = opts.as_object_mut() else {
        return;
    };
    let dest = base
        .entry(to.to_string())
        .or_insert_with(|| serde_json::json!({}));
    merge_json_objects(dest, &src);
}

fn recover_invalid_config(path: &Path, reason: String) -> SettingsLoadResult {
    let backup = next_corrupt_path(path);
    let defaults = Settings::default();
    let warning = match std::fs::rename(path, &backup) {
        Ok(()) => {
            let save_note = save_to(path, &defaults)
                .err()
                .map(|error| format!("；默认配置写入失败: {error}"))
                .unwrap_or_default();
            format!(
                "配置文件损坏，已恢复默认配置。原文件已保留为 {}（{reason}）{save_note}",
                backup.display()
            )
        }
        Err(backup_error) => format!(
            "配置文件无法读取，已在本次运行中使用默认配置，但未覆盖原文件（{reason}；备份失败: {backup_error}）"
        ),
    };
    crate::log::log(&format!("settings recovery: {warning}"));
    SettingsLoadResult {
        settings: defaults,
        warning: Some(warning),
    }
}

fn next_corrupt_path(path: &Path) -> PathBuf {
    let first = path.with_extension("json.corrupt");
    if !first.exists() {
        return first;
    }
    for index in 1..=9999 {
        let candidate = path.with_extension(format!("json.corrupt.{index}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    path.with_extension(format!("json.corrupt.{}", std::process::id()))
}

/// 写入指定路径（原子写入）
pub fn save_to(path: &Path, settings: &Settings) -> Result<(), String> {
    let save_lock = settings_file_lock(path);
    let _save = save_lock.lock().unwrap();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建配置目录失败: {e}"))?;
    }
    let json =
        serde_json::to_string_pretty(settings).map_err(|e| format!("序列化配置失败: {e}"))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json).map_err(|e| format!("写入配置失败: {e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("落盘配置失败: {e}"))?;
    Ok(())
}

fn settings_file_lock(path: &Path) -> Arc<Mutex<()>> {
    static LOCKS: OnceLock<Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>> = OnceLock::new();
    let locks = LOCKS.get_or_init(|| Mutex::new(HashMap::new()));
    locks
        .lock()
        .unwrap()
        .entry(path.to_path_buf())
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
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
    fn legacy_config_without_interaction_mode_defaults_to_push_to_talk() {
        // 0.1.5 迁移：老配置缺 interactionMode 键 → 合并默认值得到「对讲机」；
        // 显式 null（自定义兼容路径）则原样保留
        let dir = std::env::temp_dir().join(format!("kotone-cfg-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        std::fs::write(&path, "{}").unwrap();
        let s = load_from(&path);
        assert_eq!(
            s.interaction_mode,
            Some(crate::interaction::InteractionMode::PushToTalk)
        );
        std::fs::write(&path, r#"{"interactionMode": null}"#).unwrap();
        let s = load_from(&path);
        assert_eq!(s.interaction_mode, None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn default_values_match_doc() {
        let s = Settings::default();
        assert_eq!(s.hotkey.key, "CapsLock");
        assert_eq!(s.hotkey.mode, HotkeyMode::Toggle);
        assert_eq!(s.hotkey_backend, HotkeyBackend::Auto);
        assert_eq!(s.audio_device_id, "default");
        assert_eq!(s.stt_engine, "sherpa-streaming");
        assert!(s.active_model_id.is_empty());
        assert!(s.model_configs.is_empty());
        assert!(!s.auto_send);
        assert_eq!(s.active_profile_id.as_deref(), Some("lol"));
        assert_eq!(s.language, "zh");
        assert!(!s.eval_recording);
        assert_eq!(s.vad_silence_ms, 700);
        assert_eq!(s.vad.threshold, 0.5);
        assert_eq!(s.vad.min_speech_ms, 50);
        assert_eq!(s.vad.min_silence_ms, 50);
        assert_eq!(s.hotwords_score, 3.5);
        assert!(s.engine_options["sherpa-streaming"]["provider"] == "cpu");
        assert_eq!(s.session_options("sherpa-streaming")["provider"], "cpu");
        assert_eq!(
            s.session_options("sherpa-onnx-x-asr-zh-en")["provider"],
            "cpu"
        );
        assert!(!s.run_as_admin_on_start);
        assert_eq!(s.channel_cycle_hotkey, "Shift+CapsLock");
        assert!(!s.admin_prompt_dismissed);
        assert!(!s.auto_admin_prompt_dismissed);
        assert_eq!(s.history.mode, crate::history::HistoryMode::Capped);
        assert_eq!(s.history.max_records, 1000);
        assert!(!s.history.include_audio);
        assert!(!s.ui.first_run_completed);
        assert!(!s.ui.auto_start);
        assert!(
            s.models.dir.is_empty(),
            "默认模型目录为空 = ~/.kotone/models"
        );
        assert_eq!(s.download.source, DownloadSource::Auto);
        assert_eq!(s.download.gh_proxy, "https://ghfast.top/");
        assert_eq!(s.overlay.visibility, OverlayVisibility::OnDemand);
        assert_eq!(s.overlay.style, OverlayStyle::Capsule);
        assert_eq!(s.overlay.position, OverlayPosition::Auto);
        assert!(s.overlay.draggable);
        assert!(!s.overlay.click_through);
        assert_eq!(s.overlay.custom_x, None);
        assert_eq!(s.overlay.custom_y, None);
        assert!(s.post_processing.enabled);
        assert_eq!(s.post_processing.active_pipeline_id, "blocklist");
        assert_eq!(s.post_processing.pipelines.len(), 1);
        assert_eq!(s.post_processing.pipelines[0].display_name, "屏蔽词");
        assert_eq!(
            s.post_processing.pipelines[0].steps[0].processor_id,
            "builtin.blocklist-filter"
        );
    }

    #[test]
    fn load_seeds_default_blocklist_pipeline_from_legacy_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(
            &path,
            r#"{"postProcessing":{"enabled":false,"pipeline":{"id":"default","steps":[]}}}"#,
        )
        .unwrap();
        let settings = load_from(&path);
        assert!(settings.post_processing.enabled);
        assert_eq!(settings.post_processing.active_pipeline_id, "blocklist");
        assert_eq!(
            settings.post_processing.pipelines[0].steps[0].processor_id,
            "builtin.blocklist-filter"
        );
    }

    #[test]
    fn load_wraps_legacy_custom_pipeline() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(
            &path,
            r#"{
              "postProcessing":{
                "enabled":true,
                "pipeline":{
                  "id":"mine",
                  "steps":[{"id":"s1","processorId":"mock.append-exclamation","enabled":true}]
                }
              }
            }"#,
        )
        .unwrap();
        let settings = load_from(&path);
        assert!(settings.post_processing.enabled);
        assert_eq!(settings.post_processing.active_pipeline_id, "mine");
        assert_eq!(settings.post_processing.pipelines.len(), 1);
        assert_eq!(settings.post_processing.pipelines[0].display_name, "mine");
        assert_eq!(
            settings.post_processing.pipelines[0].steps[0].processor_id,
            "mock.append-exclamation"
        );
    }

    #[test]
    fn overlay_visibility_never_roundtrips() {
        let raw = serde_json::json!({ "visibility": "never" });
        let overlay: OverlayConfig = serde_json::from_value(raw).unwrap();
        assert_eq!(overlay.visibility, OverlayVisibility::Never);
        assert_eq!(
            serde_json::to_value(&overlay).unwrap()["visibility"],
            "never"
        );
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
    fn session_options_merge_family_and_io_keys() {
        let mut s = Settings {
            engine_options: serde_json::json!({
                "sherpa-onnx-sensevoice": { "threads": 2 },
                "sherpa-offline": { "model": "sense-voice-zh-en-ja-ko-yue-2024-07-17" }
            }),
            active_model_id: "sense-voice-zh-en-ja-ko-yue-2024-07-17".into(),
            ..Settings::default()
        };
        s.model_configs.insert(
            "sense-voice-zh-en-ja-ko-yue-2024-07-17".into(),
            serde_json::json!({ "language": "yue" }),
        );
        let opts = s.session_options("sherpa-onnx-sensevoice");
        assert_eq!(opts["threads"], 2);
        assert_eq!(opts["language"], "yue");
        assert_eq!(opts["model"], "sense-voice-zh-en-ja-ko-yue-2024-07-17");
        let via_io = s.session_options("sherpa-offline");
        assert_eq!(via_io["language"], "yue");
        assert_eq!(via_io["model"], "sense-voice-zh-en-ja-ko-yue-2024-07-17");
    }

    #[test]
    fn session_options_do_not_mix_legacy_offline_families() {
        let s = Settings {
            active_model_id: String::new(),
            engine_options: serde_json::json!({
                "sherpa-onnx-sensevoice": {
                    "model": "sense-voice-zh-en-ja-ko-yue-2024-07-17",
                    "language": "yue"
                }
            }),
            ..Settings::default()
        };

        let sensevoice = s.session_options("sherpa-onnx-sensevoice");
        assert_eq!(
            sensevoice["model"],
            "sense-voice-zh-en-ja-ko-yue-2024-07-17"
        );
        assert_eq!(sensevoice["language"], "yue");
    }

    #[test]
    fn load_rewrites_legacy_online_asr_engine() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(
            &path,
            r#"{ "sttEngine": "online-asr", "activeModelId": "iflytek-asr" }"#,
        )
        .unwrap();
        let s = load_from(&path);
        assert_eq!(s.stt_engine, "iflytek-asr");

        std::fs::write(
            &path,
            r#"{ "sttEngine": "online-asr", "engineOptions": { "online-asr": { "model": "volcano-asr" } } }"#,
        )
        .unwrap();
        let s = load_from(&path);
        assert_eq!(s.stt_engine, "volcano-asr");
    }

    #[test]
    fn load_rewrites_legacy_sherpa_family_engine() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(
            &path,
            r#"{
                "sttEngine": "sherpa-onnx-x-asr-zh-en",
                "engineOptions": {
                    "sherpa-onnx-x-asr-zh-en": { "provider": "cpu", "model": "x-asr-480ms-streaming-zh-en-punct-int8-2026-06-05" }
                }
            }"#,
        )
        .unwrap();
        let s = load_from(&path);
        assert_eq!(s.stt_engine, "sherpa-streaming");
        assert_eq!(
            s.active_model_id,
            "x-asr-480ms-streaming-zh-en-punct-int8-2026-06-05"
        );
        assert_eq!(s.engine_options["sherpa-streaming"]["provider"], "cpu");

        std::fs::write(&path, r#"{ "sttEngine": "sherpa-onnx-sensevoice" }"#).unwrap();
        let s = load_from(&path);
        assert_eq!(s.stt_engine, "sherpa-offline");
    }

    #[test]
    fn first_run_creates_default_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        assert!(!path.exists());
        let s = load_from(&path);
        assert_eq!(s.hotkey.key, "CapsLock");
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
        assert_eq!(s.stt_engine, "sherpa-streaming");
        assert!(!s.eval_recording);
        assert_eq!(
            s.download.source,
            DownloadSource::Auto,
            "老配置缺 download 段合并默认"
        );
        assert_eq!(s.download.gh_proxy, "https://ghfast.top/");
        assert!(!s.ui.first_run_completed, "老配置缺 ui 段合并默认 = 未完成");
        assert_eq!(s.overlay.position, OverlayPosition::Auto);
        assert!(s.overlay.draggable, "老配置升级后默认允许拖动");
        assert!(!s.overlay.click_through);
        assert_eq!(s.vad.threshold, 0.5, "老配置缺 vad 段合并默认");
        assert_eq!(s.vad.min_speech_ms, 50);
        assert_eq!(s.vad.min_silence_ms, 50);
        assert_eq!(s.hotwords_score, 3.5, "老配置缺 hotwordsScore 合并默认");
    }

    #[test]
    fn vad_partial_patch_preserves_siblings() {
        // 高级页滑块只 patch vad 段里的单个键：merge_json 嵌套合并应保留兄弟键
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        // 手写一段含 vad 段的 JSON，模拟用户已改过阈值
        std::fs::write(
            &path,
            r#"{ "vad": { "threshold": 0.7 }, "hotwordsScore": 6 }"#,
        )
        .unwrap();
        let s = load_from(&path);
        assert_eq!(s.vad.threshold, 0.7);
        assert_eq!(s.vad.min_speech_ms, 50, "兄弟键保留默认");
        assert_eq!(s.vad.min_silence_ms, 50);
        assert_eq!(s.hotwords_score, 6.0);

        // 模拟前端 patch 只带一个子键：vad.threshold 覆盖，其余不动
        let mut merged = serde_json::to_value(&s).unwrap();
        merge_json(
            &mut merged,
            &serde_json::json!({ "vad": { "minSpeechMs": 120 } }),
        );
        let patched: Settings = serde_json::from_value(merged).unwrap();
        assert_eq!(patched.vad.threshold, 0.7, "未 patch 的键保留");
        assert_eq!(patched.vad.min_speech_ms, 120);
        assert_eq!(patched.vad.min_silence_ms, 50);
    }

    #[test]
    fn legacy_overlay_interaction_fields_are_normalized() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");

        std::fs::write(
            &path,
            r#"{ "overlay": { "draggable": false, "clickThrough": false } }"#,
        )
        .unwrap();
        let locked = load_from(&path);
        assert!(!locked.overlay.draggable);
        assert!(locked.overlay.click_through);

        std::fs::write(
            &path,
            r#"{ "overlay": { "draggable": true, "clickThrough": true } }"#,
        )
        .unwrap();
        let click_through = load_from(&path);
        assert!(!click_through.overlay.draggable);
        assert!(click_through.overlay.click_through);
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
        let loaded = load_from_with_diagnostic(&path);
        assert_eq!(loaded.settings.hotkey.key, "CapsLock");
        assert!(loaded.warning.is_some());
        let backup = path.with_extension("json.corrupt");
        assert_eq!(std::fs::read_to_string(backup).unwrap(), "not json {{{");
        assert!(path.exists(), "恢复后应写入一份新的默认配置");
    }

    #[test]
    fn corrupt_backup_never_overwrites_an_existing_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(path.with_extension("json.corrupt"), "older broken config").unwrap();
        std::fs::write(&path, "new broken config").unwrap();

        let loaded = load_from_with_diagnostic(&path);
        assert!(loaded.warning.is_some());
        assert_eq!(
            std::fs::read_to_string(path.with_extension("json.corrupt")).unwrap(),
            "older broken config"
        );
        assert_eq!(
            std::fs::read_to_string(path.with_extension("json.corrupt.1")).unwrap(),
            "new broken config"
        );
    }

    #[test]
    fn unreadable_config_is_preserved_before_default_is_written() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("original"), "keep me").unwrap();

        let loaded = load_from_with_diagnostic(&path);

        assert!(loaded.warning.is_some());
        assert!(path.is_file(), "恢复后的 config.json 应为默认配置文件");
        assert_eq!(
            std::fs::read_to_string(path.with_extension("json.corrupt").join("original")).unwrap(),
            "keep me"
        );
    }

    #[test]
    fn repository_does_not_publish_or_persist_failed_external_change() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let old = Settings::default();
        save_to(&path, &old).unwrap();
        let shared = Arc::new(RwLock::new(old.clone()));
        let repository = SettingsRepository::new_at(shared.clone(), path.clone());

        let error = repository
            .update_with(
                |next| {
                    next.hotkey.key = "Alt+V".into();
                    Ok(())
                },
                |_, _| Err::<(), _>("模拟热键注册冲突".into()),
                |_, _| Ok(()),
            )
            .unwrap_err();

        assert!(error.contains("模拟热键注册冲突"));
        assert_eq!(shared.read().unwrap().hotkey.key, "CapsLock");
        assert_eq!(load_from(&path).hotkey.key, "CapsLock");
    }

    #[test]
    fn repository_merges_each_serial_update_from_latest_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let old = Settings::default();
        save_to(&path, &old).unwrap();
        let shared = Arc::new(RwLock::new(old));
        let repository = SettingsRepository::new_at(shared.clone(), path.clone());

        repository
            .update(|next| {
                next.language = "en".into();
                Ok(())
            })
            .unwrap();
        repository
            .update(|next| {
                next.audio_device_id = "mic-2".into();
                Ok(())
            })
            .unwrap();

        let memory = shared.read().unwrap().clone();
        let disk = load_from(&path);
        assert_eq!(memory.language, "en");
        assert_eq!(memory.audio_device_id, "mic-2");
        assert_eq!(disk.language, memory.language);
        assert_eq!(disk.audio_device_id, memory.audio_device_id);
    }

    #[test]
    fn repository_does_not_publish_when_config_save_fails() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::create_dir(&path).unwrap(); // rename(tmp, directory) 必然失败
        let shared = Arc::new(RwLock::new(Settings::default()));
        let repository = SettingsRepository::new_at(shared.clone(), path);

        let result = repository.update(|next| {
            next.language = "en".into();
            Ok(())
        });

        assert!(result.is_err());
        assert_eq!(shared.read().unwrap().language, "zh");
    }

    #[test]
    fn repository_serializes_concurrent_mutations() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let initial = Settings::default();
        save_to(&path, &initial).unwrap();
        let shared = Arc::new(RwLock::new(initial));
        let repository = Arc::new(SettingsRepository::new_at(shared.clone(), path.clone()));
        let mut workers = Vec::new();
        for _ in 0..16 {
            let repository = repository.clone();
            workers.push(std::thread::spawn(move || {
                repository.update(|next| {
                    next.history.max_records += 1;
                    Ok(())
                })
            }));
        }
        for worker in workers {
            worker.join().unwrap().unwrap();
        }

        assert_eq!(shared.read().unwrap().history.max_records, 1016);
        assert_eq!(load_from(&path).history.max_records, 1016);
    }

    #[test]
    fn save_to_serializes_the_shared_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = Arc::new(dir.path().join("config.json"));
        let barrier = Arc::new(std::sync::Barrier::new(17));
        let mut workers = Vec::new();
        for index in 0..16 {
            let path = path.clone();
            let barrier = barrier.clone();
            workers.push(std::thread::spawn(move || {
                let settings = Settings {
                    language: format!("lang-{index}"),
                    ..Settings::default()
                };
                barrier.wait();
                save_to(&path, &settings)
            }));
        }
        barrier.wait();
        for worker in workers {
            worker.join().unwrap().unwrap();
        }

        let loaded = load_from(&path);
        assert!(loaded.language.starts_with("lang-"));
        assert!(!path.with_extension("json.tmp").exists());
    }

    #[test]
    fn legacy_config_without_connections_defaults_to_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, "{}").unwrap();
        let settings = load_from(&path);
        assert!(settings.connections.is_empty());
    }

    #[test]
    fn settings_connection_resolver_reads_public_fields_only() {
        use crate::connection::{Connection, ConnectionKind, ConnectionResolver};

        let mut settings = Settings::default();
        settings.connections.push(Connection {
            id: "dashscope-main".into(),
            display_name: "通义".into(),
            kind: ConnectionKind::Remote,
            provider: "dashscope".into(),
            base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1".into(),
            model: "qwen-turbo".into(),
        });
        let resolver = SettingsConnectionResolver::new(Arc::new(RwLock::new(settings)));
        let resolved = resolver.resolve("dashscope-main").unwrap();
        assert_eq!(resolved.model, "qwen-turbo");
        assert!(resolved.api_key.is_none());
        assert!(resolver
            .resolve("missing")
            .unwrap_err()
            .contains("未找到连接"));
    }
}
