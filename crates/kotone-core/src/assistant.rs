//! 助手识别边界：把用户的一句话交给“Kotone 陪玩”（一次大模型调用，从提醒 / 帮忙打字 /
//! 回答中选一种，见 docs/product-evolution-game-tools.md §10）或转交外部 Agent。
//! 模型只能返回这三种受限动作，不能执行任意工具或修改配置。

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

/// 助手入口（热键语音 / 文字）收到的请求交给谁处理。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AssistantRoute {
    /// Kotone 陪玩（默认）：截图 + OCR + 聊天跟踪 + profile 说明，一次大模型调用，
    /// 从“创建提醒 / 帮忙打字 / 直接回答”中选一种。
    ///
    /// 配置里仍写作旧值 `reminders`（旧版本能读；它们把它当作提醒规划），`companion` 只作为
    /// 读入别名。配置兼容原则：旧字段只写旧版本认识的值，新能力用新字段。
    #[default]
    #[serde(rename = "reminders", alias = "companion")]
    Companion,
    /// 原文转交外部 Agent：经工具服务发布 `user.request` 事件，由 Agent 处理并用
    /// `agent.reply` 回应；需要开启工具服务。
    Agent,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantVoiceSession {
    pub session_id: String,
    pub interaction_mode: AssistantInteractionMode,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AssistantConfig {
    /// 首次安装为呼叫键。旧用户保留原提醒设置，启用琴音后单独绑定呼叫键。
    pub hotkey: String,
    /// 旧试用版用过的呼叫键迁移标记，读取时用于识别已有 0.2 配置。
    pub call_hotkey_defaulted: bool,
    /// 默认呼叫键改过几次（[`CALL_HOTKEY_REVISION`]）。配置里没有这一项 = 更早的版本：读取时把还是旧默认
    /// `Backquote` 的呼叫键改成新默认，再记上（之后玩家自己设回 `~` 不会再被改）。
    pub call_hotkey_revision: u32,
    /// 旧版提醒助手热键的交互方式。启用琴音后，呼叫热键固定为“说完自动提交”。
    pub interaction_mode: AssistantInteractionMode,
    pub connection_id: String,
    /// 仅旧版升级后使用，保留 0.1.x 可直接读取的提醒热键和连接。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub companion_bindings: Option<CompanionBindings>,
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
    /// 请求去向：陪玩（默认）或转交 Agent。
    pub route: AssistantRoute,
    /// 陪玩：快捷消息、帮忙打字是否自动发送、AI 连接能力自检结果。
    pub companion: CompanionConfig,
    /// 选中的在线语音合成模型（统一模型清单 TTS 类别的在线条目，如 `openai-compat-tts`）。
    /// 空 = 用旧字段 `speech_engine`（系统语音 / Kokoro）。在线引擎不写进 `speech_engine`，
    /// 否则旧版本读不懂这个值会把整份配置当作损坏。
    pub tts_model_id: String,
    /// 语音唤醒（ADR 017）。新字段，缺省 = 关闭。
    pub wake: WakeConfig,
    /// 琴音的对话气泡（屏幕角落）。新字段，缺省 = 开启、右下角。
    pub bubble: BubbleConfig,
    /// 回答完接着听几秒（ADR 029 §五）。ADR 033 起不再使用（通话取代，见 `call_idle_seconds`），
    /// 只为旧版本读得懂而保留；读入时 0 迁移成“不通话”。
    pub follow_up_seconds: u32,
    /// 通话（ADR 033）：呼叫键 / 唤醒 = 接通，一直在听；这么多秒没人说话就挂断
    /// （她在说、在想、在做、在等确认时不计时）。0 = 不通话：一问一答，答完不再听。新字段，缺省 30 秒。
    pub call_idle_seconds: u32,
    /// 通话里做回声消除（ADR 033 第三节）：开着音箱也能直接开口打断她。新字段，缺省开。
    pub echo_cancellation: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CompanionBindings {
    pub hotkey: String,
    pub connection_id: String,
}

/// 对话气泡放在屏幕哪个角。不认识的值按右下角处理。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BubbleCorner {
    BottomLeft,
    TopRight,
    TopLeft,
    #[default]
    #[serde(other)]
    BottomRight,
}

/// 琴音的对话气泡：语音入口（唤醒、陪玩热键、追问）的状态与回答显示在屏幕角落，
/// 不再借用语音输入法的悬浮窗；回答默认不再走弹幕。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct BubbleConfig {
    pub enabled: bool,
    pub corner: BubbleCorner,
    /// 离屏幕上下边缘再挪开多少逻辑像素（避开小地图等）。
    pub offset_y: u32,
    /// 回答同时用弹幕显示。
    pub also_danmaku: bool,
}

pub const BUBBLE_OFFSET_RANGE: std::ops::RangeInclusive<u32> = 0..=800;

impl Default for BubbleConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            corner: BubbleCorner::default(),
            offset_y: 0,
            also_danmaku: false,
        }
    }
}

/// 唤醒词（预置；第二版再考虑自定义）。不认识的值按默认处理。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WakePhrase {
    /// “你好琴音”
    NihaoQinyin,
    /// “琴音琴音”
    #[default]
    #[serde(other)]
    QinyinQinyin,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WakeSensitivity {
    /// 更不容易误唤醒，可能要喊得清楚些。
    Low,
    /// 更容易唤醒，也更容易被别的声音误触发。
    High,
    #[default]
    #[serde(other)]
    Normal,
}

/// 唤醒后怎么应答。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WakeAck {
    /// 只放开始录音的提示音。
    Tone,
    /// 不提示。
    None,
    /// 说一声“嗯？”/“我在”（连着往下说时不说）。
    #[default]
    #[serde(other)]
    Voice,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct WakeConfig {
    /// 开启后常开麦克风，在本机检测唤醒词（默认关）。
    pub enabled: bool,
    pub phrase: WakePhrase,
    pub sensitivity: WakeSensitivity,
    pub ack: WakeAck,
    /// 唤醒后一直不说话多久就结束（毫秒）。
    pub no_speech_timeout_ms: u32,
}

pub const WAKE_NO_SPEECH_RANGE_MS: std::ops::RangeInclusive<u32> = 2_000..=15_000;

/// 回答完接着听的缺省秒数（照 Pipecat 唤醒后 10 s 内不用再喊、xiaozhi 念完自动再开麦，取 8 秒）。
pub const DEFAULT_FOLLOW_UP_SECONDS: u32 = 8;
/// 回答完接着听的秒数范围（0 = 不接着听）。
pub const FOLLOW_UP_RANGE_SECONDS: std::ops::RangeInclusive<u32> = 0..=30;
/// 通话多久没人说话就挂断（照 orb `idle.ts` 的缺省 30 秒，ADR 033）。
pub const DEFAULT_CALL_IDLE_SECONDS: u32 = 30;
/// 通话空闲挂断的秒数范围（0 = 不通话；照 orb 5–600）。
pub const CALL_IDLE_RANGE_SECONDS: std::ops::RangeInclusive<u32> = 0..=600;

impl Default for WakeConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            phrase: WakePhrase::default(),
            sensitivity: WakeSensitivity::default(),
            ack: WakeAck::default(),
            no_speech_timeout_ms: 5_000,
        }
    }
}

/// 实际使用的语音合成引擎（由 `speech_engine` 与 `tts_model_id` 共同决定，不直接落盘）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TtsEngine {
    System,
    Kokoro,
    /// OpenAI 兼容的在线语音合成（`/audio/speech`），绑定 AI 连接。
    Openai,
    /// 火山语音合成（在线）。
    Volcano,
    /// DeepSeek 官方语音合成（在线，`/v1/tools/tts`），复用 DeepSeek 连接的 API key。
    Deepseek,
}

/// 在线语音合成的清单 ID（与 `kotone_tts::catalog` 一致）。
pub const OPENAI_TTS_MODEL_ID: &str = "openai-compat-tts";
pub const VOLCANO_TTS_MODEL_ID: &str = "volcano-tts";
pub const DEEPSEEK_TTS_MODEL_ID: &str = "deepseek-tts";
/// `ttsModelId` 写这个值 = 用户明确选了本地语音（`speechEngine`），不参与“自动”。
pub const LOCAL_TTS_MODEL_ID: &str = "local";

/// 快捷消息最多几条（每条一个固定的具名热键槽位 `quick-message-<n>`）。
pub const MAX_QUICK_MESSAGES: usize = 8;

/// 一条快捷指令（字段名沿用“快捷消息”）：按下热键，要么等于替用户说了 `text`（交给陪玩的 AI，
/// 不经过语音识别），要么直接运行一个已保存的自动化（`automation_id` + `args`，不调用 AI；
/// ADR 013 §8）。
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct QuickMessage {
    /// 稳定 ID（工具 `companion.ask` 的 `quickMessageId`）。
    pub id: String,
    /// 设置页显示的名字；空 = 用 text。
    pub label: String,
    /// 交给 AI 的一句话（`automation_id` 为空时必填）。
    pub text: String,
    /// 绑定的热键；空 = 未绑定（仍可由工具触发）。
    pub hotkey: String,
    /// 要直接运行的已保存自动化；空 = 这条是交给 AI 的一句话。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub automation_id: String,
    /// 运行自动化时的入参（按自动化的声明校验）。
    #[serde(skip_serializing_if = "serde_json::Map::is_empty")]
    pub args: serde_json::Map<String, serde_json::Value>,
}

impl QuickMessage {
    /// 这条是不是“直接运行自动化”。
    pub fn runs_automation(&self) -> bool {
        !self.automation_id.trim().is_empty()
    }
}

/// 能力自检结果。只在连接 ID、地址与模型都未变时有效。
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CompanionCheck {
    pub connection_id: String,
    pub base_url: String,
    pub model: String,
    pub passed: bool,
    pub checked_at_ms: u64,
    /// 结果说明（失败原因或通过时的摘要），不含密钥或模型原始输出。
    pub message: String,
}

impl CompanionCheck {
    pub fn matches(&self, connection_id: &str, base_url: &str, model: &str) -> bool {
        self.connection_id == connection_id && self.base_url == base_url && self.model == model
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CompanionConfig {
    pub quick_messages: Vec<QuickMessage>,
    /// 帮忙打字后自动按发送键；默认 true（2026-11-16 用户决定：说“帮我跟队友说…”就是要发出去）。
    /// 关掉后只打进聊天框，由用户确认后自己发。
    pub auto_send: bool,
    /// 最近一次能力自检；None = 尚未自检。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub check: Option<CompanionCheck>,
}

impl Default for CompanionConfig {
    fn default() -> Self {
        Self {
            quick_messages: Vec::new(),
            auto_send: true,
            check: None,
        }
    }
}

impl CompanionConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.quick_messages.len() > MAX_QUICK_MESSAGES {
            return Err(format!("快捷消息最多 {MAX_QUICK_MESSAGES} 条"));
        }
        let mut ids = std::collections::HashSet::new();
        for message in &self.quick_messages {
            let id = message.id.trim();
            if id.is_empty() || id.len() > 64 || !ids.insert(id) {
                return Err("快捷消息的 ID 不能为空、过长或重复".into());
            }
            let count = message.text.trim().chars().count();
            if message.runs_automation() {
                if message.automation_id.len() > 64 || count > 200 {
                    return Err("快捷指令的自动化编号或说明过长".into());
                }
                if serde_json::to_vec(&message.args).map_or(true, |b| b.len() > 4_096) {
                    return Err("快捷指令的入参过大".into());
                }
            } else if count == 0 || count > 200 {
                return Err("快捷消息的内容需要 1 到 200 字".into());
            }
            if message.label.chars().count() > 40 {
                return Err("快捷消息的名字不能超过 40 字".into());
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SpeechEngine {
    Kokoro,
    System,
}

/// 呼叫琴音的默认键：Ctrl + 主键盘左上角的 ` / ~。以前是单独的 `（[`OLD_DEFAULT_CALL_HOTKEY`]），
/// 但它是英雄联盟默认的“只攻击英雄”键，Kotone 按下时吞掉它，游戏就收不到（2026-11-21 玩家反馈）；
/// 加上 Ctrl 后单按 ` 照样给游戏。
pub const DEFAULT_CALL_HOTKEY: &str = "Ctrl+Backquote";
/// 0.2.0-rc.1 及以前的默认呼叫键。
pub const OLD_DEFAULT_CALL_HOTKEY: &str = "Backquote";
/// 默认呼叫键的版本：1 = `Ctrl+Backquote`。
pub const CALL_HOTKEY_REVISION: u32 = 1;

impl Default for AssistantConfig {
    fn default() -> Self {
        Self {
            hotkey: DEFAULT_CALL_HOTKEY.into(),
            call_hotkey_defaulted: true,
            call_hotkey_revision: CALL_HOTKEY_REVISION,
            interaction_mode: AssistantInteractionMode::PushToTalk,
            connection_id: String::new(),
            companion_bindings: None,
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
            route: AssistantRoute::Companion,
            companion: CompanionConfig::default(),
            tts_model_id: String::new(),
            wake: WakeConfig::default(),
            bubble: BubbleConfig::default(),
            follow_up_seconds: DEFAULT_FOLLOW_UP_SECONDS,
            call_idle_seconds: DEFAULT_CALL_IDLE_SECONDS,
            echo_cancellation: true,
        }
    }
}

impl AssistantConfig {
    /// 设置里选定的语音合成引擎（不含“自动”的判断，见 [`Self::tts_engine_auto`]）。
    pub fn tts_engine(&self) -> TtsEngine {
        match self.tts_model_id.trim() {
            OPENAI_TTS_MODEL_ID => TtsEngine::Openai,
            VOLCANO_TTS_MODEL_ID => TtsEngine::Volcano,
            DEEPSEEK_TTS_MODEL_ID => TtsEngine::Deepseek,
            _ => self.legacy_tts_engine(),
        }
    }

    pub fn legacy_tts_engine(&self) -> TtsEngine {
        match self.speech_engine {
            SpeechEngine::System => TtsEngine::System,
            SpeechEngine::Kokoro => TtsEngine::Kokoro,
        }
    }

    /// 是不是“自动”（默认）：没有明确选过语音（`ttsModelId` 为空、本地引擎是系统语音）。
    pub fn tts_is_auto(&self) -> bool {
        self.tts_model_id.trim().is_empty() && self.speech_engine == SpeechEngine::System
    }

    /// 实际使用的引擎：“自动”时，能用 DeepSeek 语音（琴音的连接是 DeepSeek 且有密钥）
    /// 就用它，否则用系统语音。`deepseek_ready` 只在“自动”时才调用（要查凭据库）。
    pub fn tts_engine_auto(&self, deepseek_ready: impl FnOnce() -> bool) -> TtsEngine {
        if self.tts_is_auto() && deepseek_ready() {
            TtsEngine::Deepseek
        } else {
            self.tts_engine()
        }
    }

    /// 声音编号在界面与接口上写成 `提供方/声音`，落盘时还原成旧写法
    /// （`kokoro:zf_001`、不带前缀的 Windows 语音 token），旧版本才能继续使用。
    pub fn normalize_voices(&mut self) {
        if let Some(rest) = self.kokoro_voice_id.trim().strip_prefix("kokoro/") {
            self.kokoro_voice_id = format!("kokoro:{rest}");
        }
        if let Some(rest) = self.voice_id.trim().strip_prefix("system/") {
            self.voice_id = rest.to_string();
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.companion_bindings.as_ref().is_some_and(|bindings| {
            !bindings.hotkey.trim().is_empty()
                && crate::hotkey::parse_hotkey(&bindings.hotkey).is_none()
        }) {
            return Err("呼叫琴音的热键无法识别".into());
        }
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
        if !matches!(
            self.tts_model_id.trim(),
            "" | LOCAL_TTS_MODEL_ID
                | OPENAI_TTS_MODEL_ID
                | VOLCANO_TTS_MODEL_ID
                | DEEPSEEK_TTS_MODEL_ID
        ) {
            return Err("在线语音合成模型无效".into());
        }
        if !BUBBLE_OFFSET_RANGE.contains(&self.bubble.offset_y) {
            return Err("对话气泡的偏移必须在 0 到 800 之间".into());
        }
        if !WAKE_NO_SPEECH_RANGE_MS.contains(&self.wake.no_speech_timeout_ms) {
            return Err("唤醒后等待说话的时间必须在 2 到 15 秒之间".into());
        }
        if !FOLLOW_UP_RANGE_SECONDS.contains(&self.follow_up_seconds) {
            return Err("回答完接着听的时间必须在 0 到 30 秒之间".into());
        }
        if !CALL_IDLE_RANGE_SECONDS.contains(&self.call_idle_seconds) {
            return Err("通话多久没人说话挂断，必须在 0 到 600 秒之间".into());
        }
        self.companion.validate()
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

pub const LEGACY_REMINDER_SESSION_PREFIX: &str = "legacy-reminder-";

pub fn is_legacy_reminder_session(id: &str) -> bool {
    id.starts_with(LEGACY_REMINDER_SESSION_PREFIX)
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
    fn follow_up_listening_is_a_new_field_defaulting_to_eight_seconds() {
        // 旧配置没有这个字段：缺省 8 秒（ADR 029 §五）
        let old: AssistantConfig =
            serde_json::from_str(r##"{"voiceId":"v","speechVolume":65}"##).unwrap();
        assert_eq!(old.follow_up_seconds, DEFAULT_FOLLOW_UP_SECONDS);
        old.validate().unwrap();
        // 新字段单独落盘，不改任何旧字段的写法；旧版本忽略它
        let off = AssistantConfig {
            follow_up_seconds: 0,
            ..AssistantConfig::default()
        };
        let json = serde_json::to_value(&off).unwrap();
        assert_eq!(json["followUpSeconds"], 0);
        assert_eq!(json["route"], "reminders");
        off.validate().unwrap();
        let back: AssistantConfig = serde_json::from_value(json).unwrap();
        assert_eq!(back.follow_up_seconds, 0);
        assert!(AssistantConfig {
            follow_up_seconds: 31,
            ..AssistantConfig::default()
        }
        .validate()
        .is_err());
    }

    #[test]
    fn calls_are_new_fields_defaulting_to_thirty_seconds_with_echo_cancellation() {
        // 旧配置没有：缺省通话 30 秒、开回声消除（ADR 033）
        let old: AssistantConfig =
            serde_json::from_str(r##"{"voiceId":"v","followUpSeconds":8}"##).unwrap();
        assert_eq!(old.call_idle_seconds, DEFAULT_CALL_IDLE_SECONDS);
        assert!(old.echo_cancellation);
        let json = serde_json::to_value(AssistantConfig::default()).unwrap();
        assert_eq!(json["callIdleSeconds"], 30);
        assert_eq!(json["echoCancellation"], true);
        for (seconds, ok) in [(0, true), (600, true), (601, false)] {
            let config = AssistantConfig {
                call_idle_seconds: seconds,
                ..AssistantConfig::default()
            };
            assert_eq!(config.validate().is_ok(), ok, "{seconds}");
        }
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
    fn legacy_reminders_route_becomes_companion_and_quick_messages_are_bounded() {
        let config: AssistantConfig =
            serde_json::from_value(serde_json::json!({"route":"reminders"})).unwrap();
        assert_eq!(config.route, AssistantRoute::Companion);
        // 落盘仍写旧值，旧版本能读
        assert_eq!(serde_json::to_value(&config).unwrap()["route"], "reminders");
        let alias: AssistantConfig =
            serde_json::from_value(serde_json::json!({"route":"companion"})).unwrap();
        assert_eq!(alias.route, AssistantRoute::Companion);
        assert!(config.companion.auto_send, "帮忙打字默认自动发送");
        let quick = |id: &str, text: &str| QuickMessage {
            id: id.into(),
            text: text.into(),
            ..Default::default()
        };
        let mut ok = config.clone();
        ok.companion.quick_messages = vec![quick("q1", "帮我看看队友说了什么")];
        ok.validate().unwrap();
        // 快捷指令可以直接运行自动化：此时不需要 text；没设置新字段时落盘不出现它们
        let mut automation = quick("q2", "");
        automation.automation_id = "auto-1".into();
        automation.args.insert("key".into(), "Q".into());
        ok.companion.quick_messages.push(automation);
        ok.validate().unwrap();
        let json = serde_json::to_value(&ok.companion.quick_messages).unwrap();
        assert!(json[0].get("automationId").is_none() && json[0].get("args").is_none());
        assert_eq!(json[1]["automationId"], "auto-1");
        for bad in [
            vec![quick("q1", "")],
            vec![quick("q1", "a"), quick("q1", "b")],
            (0..9).map(|i| quick(&format!("q{i}"), "x")).collect(),
        ] {
            let mut config = config.clone();
            config.companion.quick_messages = bad;
            assert!(config.validate().is_err());
        }
        let check = CompanionCheck {
            connection_id: "c".into(),
            base_url: "u".into(),
            model: "m".into(),
            passed: true,
            ..Default::default()
        };
        assert!(check.matches("c", "u", "m") && !check.matches("c", "u", "m2"));
    }

    #[test]
    fn new_capabilities_never_write_values_old_versions_cannot_read() {
        // 0.1.13 的 speechEngine 只认识 kokoro / system，route 只认识 reminders / agent；
        // 读到不认识的值会把整份 config.json 当作损坏并重置。
        let mut config = AssistantConfig {
            tts_model_id: OPENAI_TTS_MODEL_ID.into(),
            kokoro_voice_id: "kokoro/zf_017".into(),
            voice_id: "system/HKEY_LOCAL_MACHINE\\TTS_MS_ZH-CN_HUIHUI_11.0".into(),
            ..AssistantConfig::default()
        };
        config.normalize_voices();
        assert_eq!(config.tts_engine(), TtsEngine::Openai);
        let json = serde_json::to_value(&config).unwrap();
        assert!(["kokoro", "system"].contains(&json["speechEngine"].as_str().unwrap()));
        assert!(["reminders", "agent"].contains(&json["route"].as_str().unwrap()));
        assert_eq!(json["kokoroVoiceId"], "kokoro:zf_017");
        assert_eq!(
            json["voiceId"],
            "HKEY_LOCAL_MACHINE\\TTS_MS_ZH-CN_HUIHUI_11.0"
        );
        config.tts_model_id.clear();
        config.speech_engine = SpeechEngine::Kokoro;
        assert_eq!(config.tts_engine(), TtsEngine::Kokoro);
        assert!(AssistantConfig {
            tts_model_id: "edge-tts".into(),
            ..AssistantConfig::default()
        }
        .validate()
        .is_err());
    }

    #[test]
    fn deepseek_speech_is_the_automatic_default() {
        let auto = AssistantConfig::default();
        assert!(auto.tts_is_auto());
        assert_eq!(auto.tts_engine_auto(|| true), TtsEngine::Deepseek);
        assert_eq!(auto.tts_engine_auto(|| false), TtsEngine::System);
        // 明确选了系统语音 / Kokoro / 别的在线引擎：不被“自动”换掉，也不去查凭据库
        let explicit = |tts: &str, engine: SpeechEngine| AssistantConfig {
            tts_model_id: tts.into(),
            speech_engine: engine,
            ..AssistantConfig::default()
        };
        let unused = || -> bool { panic!("不是自动时不该查 DeepSeek") };
        assert_eq!(
            explicit(LOCAL_TTS_MODEL_ID, SpeechEngine::System).tts_engine_auto(unused),
            TtsEngine::System
        );
        assert_eq!(
            explicit("", SpeechEngine::Kokoro).tts_engine_auto(unused),
            TtsEngine::Kokoro
        );
        assert_eq!(
            explicit(VOLCANO_TTS_MODEL_ID, SpeechEngine::System).tts_engine_auto(unused),
            TtsEngine::Volcano
        );
        assert_eq!(
            explicit(DEEPSEEK_TTS_MODEL_ID, SpeechEngine::System).tts_engine_auto(unused),
            TtsEngine::Deepseek
        );
        for value in [LOCAL_TTS_MODEL_ID, DEEPSEEK_TTS_MODEL_ID] {
            assert!(explicit(value, SpeechEngine::System).validate().is_ok());
        }
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
