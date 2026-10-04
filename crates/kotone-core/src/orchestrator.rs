//! orchestrator：唯一状态所有者。串联 hotkey → audio → stt → inject，
//! partial 转发、取消与超时（docs/development.md §4、§4.1、§5.1）
//!
//! 状态迁移全部在 Rust 侧完成，UI 只渲染 emit 的状态事件。
//!
//! 并发模型：
//! - `inner`（std Mutex）只存状态与句柄，**绝不跨 await 持有**；
//! - `op`（tokio Mutex）串行化 begin/end/cancel/confirm 的状态切换临界区；
//! - `gen` 代际计数：每次 begin/cancel 自增，async 空隙后（finalize、发送）校验 gen，
//!   期间被取消则丢弃过期结果，保证「任意状态 Esc 取消 → Idle」不错乱。

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use serde_json::json;
use tokio::sync::{mpsc, oneshot};

use crate::assistant::{AssistantHandler, AssistantRequest, AssistantVoiceSession, SessionPurpose};
use crate::audio::{AudioBackend, AudioHandle};
use crate::inject::{CancelToken, FocusBackend, InjectError, Injector, TargetWindow};
use crate::interaction::{EndTrigger, InteractionPolicy, PostFinalize};
use crate::postprocess::{
    PostProcessPipeline, PostProcessingConfig, ProcessErrorKind, ProcessingCancelToken,
    ProcessingContext, ProcessorRegistry,
};
use crate::profile::{self, GameProfile};
use crate::reminders::ReminderScheduler;
use crate::settings::Settings;
use crate::stt::{EngineRegistry, SessionConfig, SttEvent, SttSession};

/// finalize 超时（docs/development.md §6：finalize 设置 10s 超时）
const DEFAULT_FINALIZE_TIMEOUT: Duration = Duration::from_secs(10);
/// 时间戳仅用于可读性；同一毫秒内取消/重开也必须获得不同的操作凭证。
static NEXT_PROCESS_SESSION_ID: AtomicU64 = AtomicU64::new(1);
/// 单次持续收音硬上限。除了避免误触后长时间占用麦克风，也给
/// 离线引擎、评测录档与可选历史音频缓冲设置明确的内存上界。
const DEFAULT_MAX_SESSION_DURATION: Duration = Duration::from_secs(10 * 60);
/// 注入发送超时：SendInput 被安全软件/反作弊钩住时可能不返回（挂起而非立即 0/N
/// 失败），没有超时会让状态机永久卡在 Sending、取消也无法打断阻塞线程（用户反馈
/// 「SendInput 被 360 拦截后软件卡死」）。超时后阻塞线程无法回收，但状态机得以
/// 回到 Error 态提示用户，避免整机/会话冻结。
const DEFAULT_INJECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Success toast 停留时长，之后自动回 Idle；Error 必须由用户关闭或重试
const DEFAULT_TOAST_DWELL: Duration = Duration::from_millis(1500);
/// 发送前焦点恢复后的等待：给系统完成前台切换与目标窗口激活的时间
const DEFAULT_FOCUS_RESTORE_DELAY: Duration = Duration::from_millis(30);
/// 对讲机松手宽限期：松手不立刻停采集，继续收音这一小段再结束。
/// 0.1.6 真机实验实锤：松手即停会丢掉仍在传输链路上（采集驱动/回调队列）
/// 与「还在空气中」（人边说边松，句尾音素晚于松手）的句尾音频，稳定丢
/// 最后 1-2 字；录音笔/一句话模式天然带 0.5-2s 人类反应尾音，无此问题。
/// 宽限期内再次按下 = 收回结束、继续说话（对讲机直觉）。多补的尾音是
/// 环境声/静音，finalize 本就再补静音尾帧，对识别无副作用。
const DEFAULT_RELEASE_GRACE: Duration = Duration::from_millis(500);

/// 核心状态机（§4.1）
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OrchestratorState {
    Idle,
    Listening,
    Transcribing,
    Processing,
    Preview,
    Sending,
    Success,
    Error,
}

const VAD_FALLBACK_WARNING: &str = "自动静音检测暂时不可用；本次录音请再按一次热键结束并发送";

struct SessionBeginError {
    code: &'static str,
    message: String,
}

impl SessionBeginError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// Rust → UI 事件出口（生产实现发 Tauri 事件，测试用 Vec 收集）
pub trait Emitter: Send + Sync {
    fn emit(&self, event: &str, payload: serde_json::Value);
}

/// 进行中的会话句柄
struct ActiveSession {
    purpose: SessionPurpose,
    stop_tx: Option<oneshot::Sender<()>>,
    session_rx: Option<oneshot::Receiver<Box<dyn SttSession>>>,
    /// pump 线程据此决定收尾时 session.cancel() 还是交还 session
    cancelled_flag: Arc<AtomicBool>,
    /// 录音采集 guard：drop 即停止采集线程
    guard: Option<CaptureGuard>,
    pump: tokio::task::JoinHandle<()>,
    level_task: tokio::task::JoinHandle<()>,
    /// eval 录档句柄（仅 evalRecording 开启时存在；历史音频使用独立缓冲）
    recorder: Option<crate::eval::SessionRecorder>,
}

/// The release callback belongs to the actual microphone handle, not a UI state.
/// AudioHandle::drop joins the capture thread before admission is released.
struct CaptureGuard {
    audio: Option<AudioHandle>,
    _completion: CaptureCompletion,
}

impl Drop for CaptureGuard {
    fn drop(&mut self) {
        drop(self.audio.take());
    }
}

struct CaptureCompletion(Option<Arc<dyn Fn() + Send + Sync>>);

impl Drop for CaptureCompletion {
    fn drop(&mut self) {
        if let Some(finished) = &self.0 {
            finished();
        }
    }
}

/// 已在状态机内原子占用发送权的任务快照。
///
/// `Sending` 状态与取消令牌必须在任何异步焦点恢复/注入工作开始前一次性写入，
/// 否则按钮双击或热键与按钮并发会启动两次注入，并让后一次覆盖前一次的令牌。
struct SendClaim {
    ready: ReadyText,
    gen: u64,
    operation: SendOperation,
    completion: SendCompletion,
    target: Option<TargetWindow>,
    channel: Option<(String, String)>,
}

/// 一次发送从准备到最后一个阻塞调用返回始终占用同一份操作权。
/// 取消/超时只发出停止请求，不能伪装成 Win32 调用已经结束。
#[derive(Clone)]
struct SendOperation(Arc<SendOperationInner>);

struct SendOperationInner {
    cancel: CancelToken,
    finished: AtomicBool,
}

impl SendOperation {
    fn new() -> (Self, SendCompletion) {
        let operation = Self(Arc::new(SendOperationInner {
            cancel: CancelToken::default(),
            finished: AtomicBool::new(false),
        }));
        let completion = SendCompletion {
            operation: operation.clone(),
        };
        (operation, completion)
    }

    fn cancel(&self) {
        self.0.cancel.cancel();
    }

    fn is_finished(&self) -> bool {
        self.0.finished.load(Ordering::SeqCst)
    }

    fn same_operation(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// 唯一完成凭据：准备阶段由 async 调用持有，进入阻塞阶段后移交给该线程。
/// 即使调用方取消或等待超时，也只有阻塞线程真正退出后才释放发送权。
struct SendCompletion {
    operation: SendOperation,
}

impl Drop for SendCompletion {
    fn drop(&mut self) {
        self.operation.cancel();
        self.operation.0.finished.store(true, Ordering::SeqCst);
    }
}

const SEND_STILL_RUNNING: &str =
    "上一次发送仍在结束中，暂时不能开始新的发送；请稍后重试。若持续无响应，请退出并重新打开应用";

/// 已完成全部文本生产步骤、可安全进入预览或注入的文本。
/// `source_text` 保留识别原文，`text` 承载后处理完成的可交付文本。
#[derive(Clone)]
struct ReadyText {
    source_text: String,
    text: String,
}

impl ReadyText {
    fn unchanged(text: String) -> Self {
        Self {
            source_text: text.clone(),
            text,
        }
    }
}

/// STT 已完成、但尚未完成后处理的文本与本轮不可变 pipeline/context 快照。
#[derive(Clone)]
struct RecognizedText {
    source_text: String,
    pipeline: PostProcessingConfig,
    context: ProcessingContext,
}

/// 状态机中待用户/系统继续处置的类型化文本，替代语义过载的 `preview_text`。
/// Recognized 只能重试处理，预览与注入只接受 Ready。
#[derive(Clone)]
enum PendingText {
    Recognized(RecognizedText),
    Ready(ReadyText),
    /// 助手预览保留识别完成时间，只能提交 AI，不能转换为可注入文本。
    Assistant(AssistantRequest),
}

impl PendingText {
    fn ready(&self) -> Option<&ReadyText> {
        match self {
            Self::Recognized(_) | Self::Assistant(_) => None,
            Self::Ready(ready) => Some(ready),
        }
    }

    fn recognized(&self) -> Option<&RecognizedText> {
        match self {
            Self::Recognized(recognized) => Some(recognized),
            Self::Ready(_) | Self::Assistant(_) => None,
        }
    }
}

struct Inner {
    state: OrchestratorState,
    purpose: SessionPurpose,
    /// 开始时固定采集/收尾规则，设置热更新从下一会话生效。
    session_policy: Option<InteractionPolicy>,
    gen: u64,
    active: Option<ActiveSession>,
    pending_text: Option<PendingText>,
    /// 正在准备或执行的发送。取消/超时后仍保留，直到阻塞调用真正退出。
    send_operation: Option<SendOperation>,
    /// Processing 状态的取消令牌；Esc/cancel 可立即打断异步处理器 future。
    processing_cancel: Option<ProcessingCancelToken>,
    /// begin 时记录的前台窗口 = 注入目标（发送前把焦点还给它）
    target_window: Option<TargetWindow>,
    /// 本地诊断流程的 case id；无论 history 是否关闭，每次 begin 都生成。
    process_case_id: Option<String>,
    /// 仅面板按钮创建的会话接受带 ID 的面板结束/确认/取消命令。
    assistant_voice_owner: bool,
    /// history 草稿（history.mode != off 时 begin 创建；不放 ActiveSession——
    /// end() 会 take 走 active，而 Sending/Error 态的终态落账仍需要草稿）
    history: Option<Arc<Mutex<HistoryDraft>>>,
    /// 当前聊天频道（ADR-008）：(profile_id, channel_id)，sticky——
    /// 切换后保持到下次切换；profile_id 与当前激活 profile 不符时视为默认频道
    /// （用户切了游戏适配，旧频道选择自动失效）。None = 默认频道。
    channel: Option<(String, String)>,
    /// 松手宽限期的延迟结束任务（见 DEFAULT_RELEASE_GRACE）；
    /// 再次按下 / cancel / 状态变迁时 abort
    release_grace_task: Option<tokio::task::JoinHandle<()>>,
    /// 本轮会话是否按 solo 连续策略运行。VAD 初始化/推理失败时会降级为 false，
    /// 让第二次热键结束并发送当前句，而不是丢弃在途音频或自动续段。
    continuous_session: bool,
}

impl Inner {
    fn send_is_running(&mut self) -> bool {
        if self
            .send_operation
            .as_ref()
            .is_some_and(SendOperation::is_finished)
        {
            self.send_operation = None;
        }
        self.send_operation.is_some()
    }
}

/// history 草稿：会话期间累计指标，终态（sent/cancelled/error）时落账一条记录
struct HistoryDraft {
    /// 与 eval 录档共用的 sessionId（begin 生成一次同时喂两边，可互查）
    session_id: String,
    started: std::time::Instant,
    engine_id: String,
    profile_id: Option<String>,
    /// 已喂入的采样数（落账时按 eval::SAMPLE_RATE 换算 ms；persist 裁剪后
    /// 重算为实际语音时长，与保存的 wav 一致）
    audio_samples: u64,
    /// 历史音频独立缓冲；仅 history.includeAudio 开启时分配，
    /// 不依赖 evalRecording，也不会写入 ~/.kotone/eval。
    audio_pcm: Option<Vec<f32>>,
    /// finalize 成功后写入 history/audio 的相对文件名。
    audio_file: Option<String>,
    first_partial_ms: Option<u64>,
    final_text: String,
    source_text: Option<String>,
    process_duration_ms: Option<u64>,
    finalize_latency_ms: Option<u64>,
    /// error 终态已落账（Error 态保留文本供重试：重试成功仍写 sent；
    /// error 后的 cancel 是清理动作，不再双记 cancelled）
    reported: bool,
    /// VAD 判定过语音（one-shot/solo 模式；取消时据此判断「是否真的开过口」，
    /// 比 STT partial 可靠——噪音也会让流式引擎出 partial，但 silero 判定 speech
    /// 才是「疑似说话」。非 VAD 模式恒 false，判据退化到 first_partial_ms）
    vad_speech_seen: bool,
    /// VAD 判定为语音的采样区间（半开 [start, end)，采样号相对 audio_pcm 起点；
    /// 相邻段间隙 < SPEECH_GAP_SAMPLES 时合并保留句间短停顿）。
    /// 仅 VAD 模式填充；persist 时按区间拼接裁剪掉段首/段尾静音。
    speech_segments: Vec<(usize, usize)>,
}

/// 语音区间合并间隙：相邻语音段间 ≤ 此采样数视为句间停顿（保留），
/// 超过则断开（300ms @16kHz）。句内换气/短语停顿一般 < 300ms。
const SPEECH_GAP_SAMPLES: usize = 480 * 10; // 300ms

impl HistoryDraft {
    /// 追加一段语音区间（按 VAD 帧判定逐步喂入）：与上一段间隙小则合并。
    fn push_speech_range(&mut self, start: usize, end: usize) {
        if start >= end {
            return;
        }
        if let Some(last) = self.speech_segments.last_mut() {
            if start <= last.1 + SPEECH_GAP_SAMPLES {
                last.1 = last.1.max(end);
                return;
            }
        }
        self.speech_segments.push((start, end));
    }

    #[cfg(test)]
    fn speech_harness() -> Self {
        HistoryDraft {
            session_id: "test".to_string(),
            started: std::time::Instant::now(),
            engine_id: "test".to_string(),
            profile_id: None,
            audio_samples: 0,
            audio_pcm: None,
            audio_file: None,
            first_partial_ms: None,
            final_text: String::new(),
            source_text: None,
            process_duration_ms: None,
            finalize_latency_ms: None,
            reported: false,
            vad_speech_seen: false,
            speech_segments: Vec::new(),
        }
    }
}

pub struct Orchestrator {
    /// Arc：schedule_idle 的延迟任务需要共享访问
    inner: Arc<Mutex<Inner>>,
    op: tokio::sync::Mutex<()>,
    /// 按住输入的 down/up 串行化；down 等待开麦时不能丢掉已到达的 up。
    hold_input: tokio::sync::Mutex<()>,
    /// 热键队列代际独立于逐句会话代际；Esc / 重注册同步废弃旧票据。
    hotkey_epoch: AtomicU64,
    settings: Arc<RwLock<Settings>>,
    /// 配置仓库（桌面壳注入）：热键类动作（切换文字处理开关/流程）经它原子写盘并
    /// 更新共享内存快照。None = 未注入（CLI/测试），此类动作直接忽略并记日志。
    settings_repository: Option<Arc<crate::settings::SettingsRepository>>,
    /// 桌面运行时成功预热时的 STT 配置快照。设置页在 Running 期间
    /// 仍可编辑下次启动的引擎/模型/参数，但本轮会话必须继续使用
    /// 已预热配置。None 用于 CLI/测试，保持原有「现场读设置」语义。
    runtime_stt_settings: RwLock<Option<Settings>>,
    /// 桌面停止前同步关闭入口；CLI 默认开放。必须在 op 临界区再次核验。
    runtime_inputs_enabled: AtomicBool,
    engines: Arc<EngineRegistry>,
    processors: Arc<ProcessorRegistry>,
    assistant: Option<(Arc<dyn AssistantHandler>, Arc<ReminderScheduler>)>,
    audio: Arc<dyn AudioBackend>,
    before_capture: Option<Arc<dyn Fn() -> Result<(), String> + Send + Sync>>,
    after_capture: Option<Arc<dyn Fn() + Send + Sync>>,
    injector: Arc<dyn Injector>,
    focus: Arc<dyn FocusBackend>,
    emitter: Arc<dyn Emitter>,
    /// 自引用（Weak）：VAD 判停时 pump 需要触发 end()（与热键结束同路径）
    me: RwLock<Option<std::sync::Weak<Orchestrator>>>,
    /// finalize 超时（测试可调小）
    pub finalize_timeout: Duration,
    /// 单次收音最长时间（测试可调小）。
    pub max_session_duration: Duration,
    /// 注入发送超时（测试可调小）：SendInput 挂起时兜底回到 Error 态
    pub inject_timeout: Duration,
    /// Success 停留时长（测试可设为 0）；Error 不自动消失
    pub toast_dwell: Duration,
    /// 发送前焦点恢复后的等待（测试可设为 0）
    pub focus_restore_delay: Duration,
    /// eval 录档目录覆盖（None = ~/.kotone/eval/；测试指向临时目录）
    pub eval_dir: Option<std::path::PathBuf>,
    /// history 目录覆盖（None = ~/.kotone/history/；测试指向临时目录）
    pub history_dir: Option<std::path::PathBuf>,
    /// 松手宽限期（测试可调小；0 = 旧行为：松手即停）
    pub release_grace: Duration,
    secrets: Option<std::sync::Arc<dyn crate::connection::SecretStore>>,
    /// VAD 工厂（ADR-007，B3 静音判停；None = 未接入——VAD 策略下 begin 报清晰错误）。
    /// 构造后注入（壳/CLI 按 feature 与模型就绪情况接线）
    pub vad_factory: Option<crate::vad::VadFactory>,
}

impl Orchestrator {
    /// 构造（返回 Self 便于调用方先调整 pub 字段：超时/录档目录/vad_factory）。
    /// **必须接 `.into_arc()` 收尾**：VAD 判停靠 Weak 自引用触发 end()，
    /// 不 into_arc 的实例在 one-shot 模式下不会自动判停。
    pub fn new(
        settings: Arc<RwLock<Settings>>,
        engines: Arc<EngineRegistry>,
        audio: Arc<dyn AudioBackend>,
        injector: Arc<dyn Injector>,
        focus: Arc<dyn FocusBackend>,
        emitter: Arc<dyn Emitter>,
    ) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                state: OrchestratorState::Idle,
                purpose: SessionPurpose::Dictation,
                session_policy: None,
                gen: 0,
                active: None,
                pending_text: None,
                send_operation: None,
                processing_cancel: None,
                target_window: None,
                process_case_id: None,
                assistant_voice_owner: false,
                history: None,
                channel: None,
                release_grace_task: None,
                continuous_session: false,
            })),
            op: tokio::sync::Mutex::new(()),
            hold_input: tokio::sync::Mutex::new(()),
            hotkey_epoch: AtomicU64::new(0),
            settings,
            settings_repository: None,
            runtime_stt_settings: RwLock::new(None),
            runtime_inputs_enabled: AtomicBool::new(true),
            engines,
            processors: Arc::new(ProcessorRegistry::new()),
            assistant: None,
            audio,
            before_capture: None,
            after_capture: None,
            injector,
            focus,
            emitter,
            me: RwLock::new(None),
            finalize_timeout: DEFAULT_FINALIZE_TIMEOUT,
            max_session_duration: DEFAULT_MAX_SESSION_DURATION,
            inject_timeout: DEFAULT_INJECT_TIMEOUT,
            toast_dwell: DEFAULT_TOAST_DWELL,
            focus_restore_delay: DEFAULT_FOCUS_RESTORE_DELAY,
            eval_dir: None,
            history_dir: None,
            release_grace: DEFAULT_RELEASE_GRACE,
            secrets: None,
            vad_factory: None,
        }
    }

    pub fn with_secrets(mut self, store: Arc<dyn crate::connection::SecretStore>) -> Self {
        self.secrets = Some(store);
        self
    }

    pub fn with_assistant(
        mut self,
        handler: Arc<dyn AssistantHandler>,
        reminders: Arc<ReminderScheduler>,
    ) -> Self {
        self.assistant = Some((handler, reminders));
        self
    }

    /// 同步准备开麦：壳先停止并确认本机播报，失败时保持麦克风关闭。
    pub fn with_before_capture(
        mut self,
        callback: Arc<dyn Fn() -> Result<(), String> + Send + Sync>,
    ) -> Self {
        self.before_capture = Some(callback);
        self
    }

    /// 与 before_capture 配对：准备/开麦失败或采集线程完全停止后恰好调用一次。
    /// 不依赖状态事件，因此 Transcribing、迟到的 UI 事件不会提前放行播报。
    pub fn with_after_capture(mut self, callback: Arc<dyn Fn() + Send + Sync>) -> Self {
        self.after_capture = Some(callback);
        self
    }

    /// 注入配置仓库（桌面壳）：热键类动作（切换文字处理开关/流程）经它原子落盘，
    /// 并同步更新共享内存快照（设置页与运行时读同一个 Arc）。
    pub fn with_settings_repository(
        mut self,
        repo: Arc<crate::settings::SettingsRepository>,
    ) -> Self {
        self.settings_repository = Some(repo);
        self
    }

    /// 包装为 Arc 并接线 Weak 自引用（VAD 判停 → end() 的回调通道）
    pub fn into_arc(self) -> Arc<Self> {
        let orch = Arc::new(self);
        *orch.me.write().unwrap() = Some(Arc::downgrade(&orch));
        orch
    }

    pub fn state(&self) -> OrchestratorState {
        self.inner.lock().unwrap().state
    }

    /// 当前会话是否实际采用连续模式；与设置值不同，VAD 降级时会返回 false。
    pub fn continuous_session(&self) -> bool {
        self.inner.lock().unwrap().continuous_session
    }

    /// settings 句柄（集成测试与壳需要改写引擎选择等运行时配置）
    pub fn settings(&self) -> &Arc<RwLock<Settings>> {
        &self.settings
    }

    /// 壳在同步重注册热键与发布配置期间持有，避免配置检查后又有会话入场。
    /// 活跃会话必须先完成或取消；此方法不等待，不会阻塞 Esc。
    pub fn try_input_configuration_guard(&self) -> Result<tokio::sync::MutexGuard<'_, ()>, String> {
        let guard = self
            .op
            .try_lock()
            .map_err(|_| "会话正在切换，请稍后修改热键或交互模式".to_string())?;
        let mut inner = self.inner.lock().unwrap();
        if !matches!(
            inner.state,
            OrchestratorState::Idle | OrchestratorState::Success | OrchestratorState::Error
        ) || inner.send_is_running()
        {
            return Err("请先完成或取消当前会话，再修改热键或交互模式".into());
        }
        Ok(guard)
    }

    /// 桌面壳在 warmup 成功后锁定本轮运行时的 STT 配置。
    pub fn activate_runtime_settings(&self, settings: Settings) {
        *self.runtime_stt_settings.write().unwrap() = Some(settings);
        self.runtime_inputs_enabled.store(true, Ordering::SeqCst);
    }

    /// 桌面停止的第一个同步动作；阻止已经排队的 begin/发送在 cancel 后重新入场。
    pub fn suspend_runtime_inputs(&self) {
        self.runtime_inputs_enabled.store(false, Ordering::SeqCst);
        self.invalidate_hotkey_inputs();
    }

    pub fn hotkey_input_ticket(&self) -> u64 {
        self.hotkey_epoch.load(Ordering::SeqCst)
    }

    pub fn invalidate_hotkey_inputs(&self) {
        self.hotkey_epoch.fetch_add(1, Ordering::SeqCst);
    }

    fn input_ticket_is_current(&self, ticket: Option<u64>) -> bool {
        ticket.map_or(true, |ticket| ticket == self.hotkey_input_ticket())
    }

    /// 桌面壳停止运行时后解除快照并保持入口关闭；CLI/测试默认一直开放。
    pub fn deactivate_runtime_settings(&self) {
        self.suspend_runtime_inputs();
        *self.runtime_stt_settings.write().unwrap() = None;
    }

    // ---------- 热键入口（hotkey 模块调用；ADR-006 单键语义表 + 策略路由） ----------

    /// 进行中的会话使用开始时的策略；热更新不偷换一次按住/松开配对的语义。
    fn policy(&self) -> InteractionPolicy {
        let inner = self.inner.lock().unwrap();
        if let Some(policy) = inner
            .session_policy
            .filter(|_| inner.state != OrchestratorState::Idle)
        {
            return policy;
        }
        let purpose = inner.purpose;
        drop(inner);
        let settings = self.settings.read().unwrap();
        Self::configured_policy(&settings, purpose)
    }

    fn configured_policy(settings: &Settings, purpose: SessionPurpose) -> InteractionPolicy {
        match purpose {
            SessionPurpose::Dictation => InteractionPolicy::from_settings(settings),
            SessionPurpose::Assistant => {
                InteractionPolicy::from_preset(settings.assistant.interaction_mode.into())
            }
        }
    }

    fn hotkey_session_snapshot(&self) -> (OrchestratorState, SessionPurpose, u64) {
        let inner = self.inner.lock().unwrap();
        (inner.state, inner.purpose, inner.gen)
    }

    /// toggle 触发键（A2 点按事件）按状态路由：
    /// Idle 开始 / Listening 结束（B2；B3 接入后兜底强制结束；solo 连续模式 = 停止会话，
    /// 不发送在途段）/ Preview 确认发送（C2 的主交互，全程不碰鼠标不抢焦点）/ 其余状态中止。
    pub async fn on_hotkey_toggle(&self) {
        self.on_hotkey_toggle_with_ticket(self.hotkey_input_ticket())
            .await;
    }

    pub async fn on_hotkey_toggle_with_ticket(&self, ticket: u64) {
        self.on_purpose_hotkey_toggle(SessionPurpose::Dictation, ticket)
            .await;
    }

    async fn on_purpose_hotkey_toggle(&self, purpose: SessionPurpose, ticket: u64) {
        if !self.input_ticket_is_current(Some(ticket)) {
            return;
        }
        // 另一用途的热键只能取消旧会话，不结束它、确认它或把结果交给另一条管线。
        let (state, active_purpose, gen) = self.hotkey_session_snapshot();
        if self.inner.lock().unwrap().assistant_voice_owner
            && !matches!(
                state,
                OrchestratorState::Idle | OrchestratorState::Success | OrchestratorState::Error
            )
        {
            return;
        }
        if state != OrchestratorState::Idle && active_purpose != purpose {
            self.cancel_inner(Some(ticket), Some(gen)).await;
            return;
        }
        match state {
            OrchestratorState::Idle => {
                let _ = self
                    .begin_inner(false, purpose, Some(gen), Some(ticket))
                    .await;
            }
            OrchestratorState::Listening => {
                // B1（松手结束）策略下不会产生 toggle 事件；B2/B3 均为按下结束
                let policy = self.policy();
                if self.continuous_session() {
                    // solo：再点按 = 停止会话（发送完成不停机由 schedule_idle 负责；
                    // 这里丢弃在途段——用户的意图是「停」，不是「把这句发出去」）
                    self.cancel_inner(Some(ticket), Some(gen)).await;
                } else if policy.end != EndTrigger::HotkeyRelease {
                    let _ = self.end_inner(Some(gen), Some(ticket)).await;
                }
            }
            OrchestratorState::Preview => {
                let _ = self.confirm_send_inner(Some(ticket), Some(gen)).await;
            }
            // Success / 无文本 Error 的 toast 驻留期：点按 = 立即开始下一句
            // （先 cancel 清理 toast 再 begin，「第一次按没反应」修复）。
            // 带文本的 Error（发送失败待重试）保持只取消，不吞掉重试入口。
            OrchestratorState::Success => {
                self.cancel_inner(Some(ticket), Some(gen)).await;
                let _ = self
                    .begin_inner(false, purpose, Some(gen + 1), Some(ticket))
                    .await;
            }
            OrchestratorState::Error if self.inner.lock().unwrap().pending_text.is_none() => {
                self.cancel_inner(Some(ticket), Some(gen)).await;
                let _ = self
                    .begin_inner(false, purpose, Some(gen + 1), Some(ticket))
                    .await;
            }
            _ => self.cancel_inner(Some(ticket), Some(gen)).await,
        }
    }

    /// hold 触发键（A1）：按下开始。Idle 直接 begin；Success / 无文本 Error 的
    /// toast 驻留期内按下 = 立即开始下一句（先 cancel 再 begin，「第一次按没反应」
    /// 修复）；带文本的 Error（发送失败待重试）按下 = 重试发送——hold 模式没有
    /// 独立的确认键，悬浮窗重试按钮要动鼠标，不修复的话用户只能重启 runtime
    /// （0.1.5 用户反馈：注入失败后按热键全部无响应）；Preview 保留确认入口。
    /// 松开在 Listening 态结束（B1）。
    pub async fn on_hotkey_hold(&self, pressed: bool) {
        self.on_hotkey_hold_with_ticket(pressed, self.hotkey_input_ticket())
            .await;
    }

    pub async fn on_hotkey_hold_with_ticket(&self, pressed: bool, ticket: u64) {
        self.on_purpose_hotkey_hold(SessionPurpose::Dictation, pressed, ticket)
            .await;
    }

    async fn on_purpose_hotkey_hold(&self, purpose: SessionPurpose, pressed: bool, ticket: u64) {
        let _input = self.hold_input.lock().await;
        if !self.input_ticket_is_current(Some(ticket)) {
            return;
        }
        let (state, active_purpose, gen) = self.hotkey_session_snapshot();
        if self.inner.lock().unwrap().assistant_voice_owner
            && !matches!(
                state,
                OrchestratorState::Idle | OrchestratorState::Success | OrchestratorState::Error
            )
        {
            return;
        }
        if state != OrchestratorState::Idle && active_purpose != purpose {
            if pressed {
                self.cancel_inner(Some(ticket), Some(gen)).await;
            }
            return;
        }
        if pressed {
            // 宽限期内再次按下 = 收回「结束」，当前这句继续说（对讲机直觉）
            {
                let mut inner = self.inner.lock().unwrap();
                if inner.gen != gen || !self.input_ticket_is_current(Some(ticket)) {
                    return;
                }
                if let Some(task) = inner.release_grace_task.take() {
                    task.abort();
                }
            }
            match state {
                OrchestratorState::Idle => {
                    let _ = self
                        .begin_inner(false, purpose, Some(gen), Some(ticket))
                        .await;
                }
                OrchestratorState::Success => {
                    self.cancel_inner(Some(ticket), Some(gen)).await;
                    let _ = self
                        .begin_inner(false, purpose, Some(gen + 1), Some(ticket))
                        .await;
                }
                OrchestratorState::Error => {
                    if self.inner.lock().unwrap().pending_text.is_some() {
                        // 带文本 Error = 发送失败待重试：按下即重发（confirm_send
                        // 从 Error 态重新进入 Sending）
                        let _ = self.confirm_send_inner(Some(ticket), Some(gen)).await;
                    } else {
                        self.cancel_inner(Some(ticket), Some(gen)).await;
                        let _ = self
                            .begin_inner(false, purpose, Some(gen + 1), Some(ticket))
                            .await;
                    }
                }
                _ => {}
            }
        } else if self.policy().end == EndTrigger::HotkeyRelease
            && self.state() == OrchestratorState::Listening
        {
            // 松手 ≠ 立刻结束：进入宽限期继续收音，倒计时到点才 finalize
            // （句尾音素与传输链路上的在途音频不再被切掉，见 DEFAULT_RELEASE_GRACE）
            self.schedule_release_end(gen, ticket);
        }
    }

    /// 松手宽限期编排：延迟 release_grace 后结束会话；期间 gen/状态变化则作废。
    /// 宽限期内 pump 照常把 PCM 灌进 session——多录的是环境尾音，
    /// 与录音笔模式人类反应延迟天然产生的尾音等效。
    fn schedule_release_end(&self, gen: u64, ticket: u64) {
        // 防御：理论上一句只松手一次，重复松手（双通道事件）以前一个倒计时为准
        let mut inner = self.inner.lock().unwrap();
        if inner.gen != gen
            || inner.state != OrchestratorState::Listening
            || !self.input_ticket_is_current(Some(ticket))
            || inner.release_grace_task.is_some()
        {
            return;
        }
        let grace = self.release_grace;
        let me_weak = self.me.read().unwrap().clone();
        let handle = tokio::spawn(async move {
            tokio::time::sleep(grace).await;
            let Some(me) = me_weak.as_ref().and_then(|w| w.upgrade()) else {
                return;
            };
            // 保留任务句柄直到原子进入 Transcribing，等待 op 期间重按仍可 abort。
            let _ = me.end_inner(Some(gen), Some(ticket)).await;
        });
        inner.release_grace_task = Some(handle);
    }

    /// 收回宽限期（再次按下/取消/停止 runtime 时调用；无待结束任务则无事发生）
    fn abort_release_grace(&self) {
        if let Some(t) = self.inner.lock().unwrap().release_grace_task.take() {
            t.abort();
        }
    }

    // ---------- 频道切换（ADR-008：sticky 频道态，句间动作） ----------

    /// 频道切换键：按声明顺序循环当前激活 profile 的频道，悬浮窗提示新频道。
    /// sticky：切换后保持到下次切换；频道在发送时刻读取，因此 toggle/one-shot/
    /// solo 模式会话进行中按下自然作用于当前这句（hold 模式该组合物理上按不出）。
    /// 单频道 profile 无操作。任意状态都接受（频道与会话状态正交）。
    pub async fn on_cycle_channel(&self) {
        let settings = self.settings.read().unwrap().clone();
        let profile = settings
            .active_profile_id
            .as_deref()
            .and_then(profile::get)
            .unwrap_or_else(GameProfile::builtin_generic);
        let current = { self.inner.lock().unwrap().channel.clone() };
        // profile 不符（切了游戏适配）的残留视为「位于默认频道」
        let current_id = current
            .filter(|(pid, _)| *pid == profile.id)
            .map(|(_, cid)| cid);
        let Some(next_id) = profile.next_channel_id(current_id.as_deref()) else {
            return; // 单频道：切换键无操作
        };
        let strategy = profile.channel_strategy(Some(&next_id));
        {
            self.inner.lock().unwrap().channel = Some((profile.id.clone(), next_id.clone()));
        }
        self.emitter.emit(
            "kotone://channel",
            json!({
                "channelId": strategy.id,
                "displayName": strategy.display_name,
                "isDefault": strategy.is_default,
            }),
        );
        self.emit_process(
            "channel_switched",
            json!({ "channelId": strategy.id, "profileId": profile.id }),
        );
    }

    // ---------- 文字处理热键（开关 / 切换流程） ----------

    /// 文字处理启用开关热键：切换 `postProcessing.enabled` 并持久化，悬浮窗显示状态。
    /// 与设置页改配置一致，下一句识别按快照生效；任意状态都可切换（与会话正交）。
    pub async fn on_post_processing_toggle(&self) {
        let Some(repo) = self.settings_repository.clone() else {
            crate::log::log("post-processing toggle: 未注入配置仓库，忽略");
            return;
        };
        let next = match repo.update(|settings| {
            settings.post_processing.enabled = !settings.post_processing.enabled;
            Ok(())
        }) {
            Ok(next) => next,
            Err(e) => {
                crate::log::log(&format!("post-processing toggle 保存失败: {e}"));
                return;
            }
        };
        self.emit_pp_status(
            "toggle",
            next.post_processing.enabled,
            next.post_processing.active_pipeline_id,
        );
    }

    /// 切换文字处理流程热键：按声明顺序把 `active_pipeline_id` 前进到下一个流程并
    /// 持久化；不足两条流程时无操作。悬浮窗显示切换到的新流程名。
    pub async fn on_post_processing_cycle(&self) {
        let Some(repo) = self.settings_repository.clone() else {
            crate::log::log("post-processing cycle: 未注入配置仓库，忽略");
            return;
        };
        if repo.snapshot().post_processing.pipelines.len() < 2 {
            // 单流程甚至无流程：切换键无操作（记日志便于排障）
            crate::log::log("post-processing cycle: 流程不足两条，忽略");
            return;
        }
        let next = match repo.update(|settings| {
            let pipelines = &settings.post_processing.pipelines;
            let active = settings.post_processing.active_pipeline_id.clone();
            let cur = pipelines.iter().position(|p| p.id == active).unwrap_or(0);
            let next_id = pipelines[(cur + 1) % pipelines.len()].id.clone();
            settings.post_processing.active_pipeline_id = next_id;
            Ok(())
        }) {
            Ok(next) => next,
            Err(e) => {
                crate::log::log(&format!("post-processing cycle 保存失败: {e}"));
                return;
            }
        };
        self.emit_pp_status(
            "cycle",
            next.post_processing.enabled,
            next.post_processing.active_pipeline_id,
        );
    }

    /// 向悬浮窗广播一次文字处理状态变化（开灯 / 切换流程；`action` 供前端区分文案）。
    fn emit_pp_status(&self, action: &str, enabled: bool, active_pipeline_id: String) {
        let settings = self.settings.read().unwrap().clone();
        let display_name = settings
            .post_processing
            .active_pipeline()
            .map(|p| p.display_name.clone())
            .unwrap_or_default();
        self.emitter.emit(
            "kotone://pp-status",
            json!({
                "action": action,
                "enabled": enabled,
                "pipelineId": active_pipeline_id,
                "displayName": display_name,
            }),
        );
    }

    // ---------- 状态机操作 ----------

    /// 开始一次「按下到松手」的会话：建 STT session → 开录音 → Listening
    pub async fn begin(&self) -> Result<(), String> {
        self.begin_inner(false, SessionPurpose::Dictation, None, None)
            .await
    }

    /// 助手使用同一个音频/STT端口，但用途在开始时固定，识别结果绝不进入游戏注入。
    pub async fn begin_assistant(&self) -> Result<(), String> {
        self.begin_inner(false, SessionPurpose::Assistant, None, None)
            .await
    }

    /// 独立助手键的点按入口，按助手自己的预设路由。
    pub async fn on_assistant_hotkey(&self) {
        self.on_assistant_hotkey_with_ticket(self.hotkey_input_ticket())
            .await;
    }

    pub async fn on_assistant_hotkey_with_ticket(&self, ticket: u64) {
        self.on_purpose_hotkey_toggle(SessionPurpose::Assistant, ticket)
            .await;
    }

    /// 对讲机必须由壳传入真实 down/up；松开沿用尾音宽限与重按撤回机制。
    pub async fn on_assistant_hotkey_hold(&self, pressed: bool) {
        self.on_assistant_hotkey_hold_with_ticket(pressed, self.hotkey_input_ticket())
            .await;
    }

    pub async fn on_assistant_hotkey_hold_with_ticket(&self, pressed: bool, ticket: u64) {
        self.on_purpose_hotkey_hold(SessionPurpose::Assistant, pressed, ticket)
            .await;
    }

    /// 面板语音按钮专用入口。会话 ID 在同一入场临界区内生成和返回，
    /// 迟到的按钮命令不能接管热键会话，也不能操作下一次录音。
    pub async fn start_assistant_voice_input(
        &self,
        input_ticket: u64,
    ) -> Result<AssistantVoiceSession, String> {
        let _op = self.op.lock().await;
        if !self.input_ticket_is_current(Some(input_ticket)) {
            return Err("语音输入操作已失效，请重试".into());
        }
        let clear_terminal = {
            let inner = self.inner.lock().unwrap();
            match inner.state {
                OrchestratorState::Idle => false,
                OrchestratorState::Success | OrchestratorState::Error
                    if inner.purpose == SessionPurpose::Assistant =>
                {
                    true
                }
                _ => return Err("已有语音会话正在进行，请先结束或取消".into()),
            }
        };
        if clear_terminal {
            self.cancel_locked();
        }
        self.begin_admitted(false, SessionPurpose::Assistant, None, true)?;
        let session_id = self
            .inner
            .lock()
            .unwrap()
            .process_case_id
            .clone()
            .expect("begin assigned session ID");
        // begin_admitted 持有 op；配置发布也使用 op，故这里读取与开麦相同的模式。
        let interaction_mode = self.settings.read().unwrap().assistant.interaction_mode;
        Ok(AssistantVoiceSession {
            session_id,
            interaction_mode,
        })
    }

    fn assistant_voice_generation(&self, session_id: &str) -> Option<u64> {
        let inner = self.inner.lock().unwrap();
        (inner.assistant_voice_owner
            && inner.purpose == SessionPurpose::Assistant
            && inner.process_case_id.as_deref() == Some(session_id))
        .then_some(inner.gen)
    }

    pub async fn finish_assistant_voice_input(&self, session_id: &str) -> Result<(), String> {
        let op = self.op.lock().await;
        let Some(gen) = self.assistant_voice_generation(session_id) else {
            return Ok(());
        };
        if self.state() != OrchestratorState::Listening {
            return Ok(());
        }
        if self.policy().end == EndTrigger::HotkeyRelease {
            self.schedule_release_end(gen, self.hotkey_input_ticket());
            return Ok(());
        }
        drop(op);
        self.end_inner(Some(gen), None).await
    }

    pub async fn confirm_assistant_voice_input(&self, session_id: &str) -> Result<(), String> {
        let Some(gen) = self.assistant_voice_generation(session_id) else {
            return Ok(());
        };
        self.confirm_send_inner(None, Some(gen)).await
    }

    pub async fn cancel_assistant_voice_input(&self, session_id: &str) -> Result<(), String> {
        let _op = self.op.lock().await;
        if self.assistant_voice_generation(session_id).is_some() {
            self.cancel_locked();
        }
        Ok(())
    }

    /// solo 连续模式逐句续录：与 begin 同路径，但「录制音」提示由壳做区分
    /// （capture_started 事件带 resumed=true，不播录制音效，只保留逐句发送音）。
    async fn begin_resumed(&self, purpose: SessionPurpose, gen: u64) -> Result<(), String> {
        self.begin_inner(true, purpose, Some(gen), None).await
    }

    async fn begin_inner(
        &self,
        resumed: bool,
        purpose: SessionPurpose,
        expected_gen: Option<u64>,
        input_ticket: Option<u64>,
    ) -> Result<(), String> {
        let _op = self.op.lock().await;
        if !self.input_ticket_is_current(input_ticket) {
            return Ok(());
        }
        self.begin_admitted(resumed, purpose, expected_gen, false)
    }

    /// 调用方持有 op；所有来源共享录音入场和失败收尾，owner 只限制操作权限。
    fn begin_admitted(
        &self,
        resumed: bool,
        purpose: SessionPurpose,
        expected_gen: Option<u64>,
        assistant_voice_owner: bool,
    ) -> Result<(), String> {
        if !self.runtime_inputs_enabled.load(Ordering::SeqCst) {
            return Err("运行时已停止，不能开始新会话".into());
        }
        let settings = self.settings.read().unwrap().clone();
        let policy = Self::configured_policy(&settings, purpose);
        {
            let mut inner = self.inner.lock().unwrap();
            if expected_gen.is_some_and(|gen| inner.gen != gen) {
                return Ok(());
            }
            if inner.state != OrchestratorState::Idle {
                return Err(format!("当前状态 {:?} 不能开始新会话", inner.state));
            }
            if inner.send_is_running() {
                return Err(SEND_STILL_RUNNING.into());
            }
            if resumed && !policy.continuous {
                // 本段沿用旧策略完成，下一段若已改成按键触发则等待真实按键。
                // 不能因旧段的 continuous=true 自动开启新的对讲机会话。
                inner.continuous_session = false;
                drop(inner);
                self.emit_state(OrchestratorState::Idle, None);
                return Ok(());
            }
            inner.purpose = purpose;
            inner.process_case_id = Some(format!(
                "session-{}-{}",
                crate::eval::new_session_id(),
                NEXT_PROCESS_SESSION_ID.fetch_add(1, Ordering::Relaxed),
            ));
            inner.assistant_voice_owner = assistant_voice_owner;
        }
        match self.begin_locked(resumed, purpose, settings, policy) {
            Ok(()) => Ok(()),
            Err(e) => {
                // 开始失败（如引擎/麦克风未就绪）：Error 保持到用户关闭或重试
                self.emit_process(
                    "session_failed",
                    json!({ "outcome": "error", "errorCode": e.code }),
                );
                self.toast_error(&e.message);
                Err(e.message)
            }
        }
    }

    fn begin_locked(
        &self,
        resumed: bool,
        purpose: SessionPurpose,
        settings: Settings,
        policy: InteractionPolicy,
    ) -> Result<(), SessionBeginError> {
        if purpose == SessionPurpose::Assistant && self.assistant.is_none() {
            return Err(SessionBeginError::new(
                "ASSISTANT_NOT_CONFIGURED",
                "助手尚未配置，请先设置助手连接",
            ));
        }
        let stt_settings = self
            .runtime_stt_settings
            .read()
            .unwrap()
            .clone()
            .unwrap_or_else(|| settings.clone());
        let engine_id = stt_settings.stt_engine.clone();
        let engine = self.engines.get(&engine_id).ok_or_else(|| {
            SessionBeginError::new(
                "STT_ENGINE_NOT_REGISTERED",
                format!("未注册的 STT 引擎: {engine_id}"),
            )
        })?;
        if !engine.is_ready() {
            return Err(SessionBeginError::new(
                "STT_ENGINE_NOT_READY",
                format!(
                    "引擎「{}」未就绪（模型未下载），请到「设置 → 高级」下载模型",
                    engine.display_name()
                ),
            ));
        }

        let active_profile = settings
            .active_profile_id
            .as_deref()
            .and_then(profile::get)
            .unwrap_or_else(GameProfile::builtin_generic);
        let mut cfg = build_session_config(&stt_settings, &engine_id, &active_profile);
        enrich_remote_connection(&stt_settings, &mut cfg, self.secrets.as_deref());

        let (stt_tx, stt_rx) = mpsc::unbounded_channel::<SttEvent>();
        let session = engine
            .start_session(&cfg, stt_tx)
            .map_err(|error| SessionBeginError::new("STT_SESSION_START_FAILED", error))?;

        let capture_completion = CaptureCompletion(self.after_capture.clone());
        if let Some(before_capture) = &self.before_capture {
            if let Err(error) = before_capture() {
                let mut session = session;
                session.cancel();
                return Err(SessionBeginError::new(
                    "AUDIO_CAPTURE_PREPARATION_FAILED",
                    error,
                ));
            }
        }

        // 开录音失败要有清晰错误；已建的 session 取消掉
        let mut handle = match self.audio.start(&settings.audio_device_id) {
            Ok(h) => h,
            Err(e) => {
                let mut session = session;
                session.cancel();
                return Err(SessionBeginError::new("AUDIO_CAPTURE_START_FAILED", e));
            }
        };
        let pcm_rx = handle.pcm_rx.take().expect("audio handle pcm channel");
        let level_rx = handle.level_rx.take().expect("audio handle level channel");
        let audio_err_rx = handle.error_rx.take();

        let (stop_tx, stop_rx) = oneshot::channel::<()>();
        let (session_tx, session_rx) = oneshot::channel::<Box<dyn SttSession>>();
        let cancelled_flag = Arc::new(AtomicBool::new(false));

        // eval 录档：evalRecording 开启时创建录档句柄，pump 边录边喂；
        // finalize 成功后落盘，取消则随会话句柄整体丢弃（docs/adr/005）。
        // history 音频使用自己的 PCM 缓冲，与评测录档完全解耦。
        // process case id 在 begin 入口生成；history/eval 开启时复用同一 id，
        // 便于诊断包里的 events.csv 与 history/eval 互查。
        let history_on = purpose == SessionPurpose::Dictation
            && settings.history.mode != crate::history::HistoryMode::Off;
        let process_case_id = self
            .inner
            .lock()
            .unwrap()
            .process_case_id
            .clone()
            .unwrap_or_else(|| format!("session-{}", crate::eval::new_session_id()));
        let session_id = history_on.then(|| process_case_id.clone());
        let recorder =
            (purpose == SessionPurpose::Dictation && settings.eval_recording).then(|| {
                let dir = self.eval_dir.clone().unwrap_or_else(crate::eval::eval_dir);
                crate::eval::SessionRecorder::new_with_id(dir, &engine_id, process_case_id.clone())
            });
        let history_draft = session_id.map(|sid| {
            Arc::new(Mutex::new(HistoryDraft {
                session_id: sid,
                started: std::time::Instant::now(),
                engine_id: engine_id.clone(),
                profile_id: settings.active_profile_id.clone(),
                audio_samples: 0,
                audio_pcm: settings.history.include_audio.then(Vec::new),
                audio_file: None,
                first_partial_ms: None,
                final_text: String::new(),
                source_text: None,
                process_duration_ms: None,
                finalize_latency_ms: None,
                reported: false,
                vad_speech_seen: false,
                speech_segments: Vec::new(),
            }))
        });

        // B3（VAD 静音判停，ADR-007）：初始化失败不能让整个输入模式失效。
        // 降级后本轮按普通 toggle 会话运行：再次按热键即可结束并发送；solo
        // 暂停自动续段，避免第二次热键按原语义把在途音频直接丢弃。
        let use_vad = policy.end == EndTrigger::VadSilence;
        let (vad_state, vad_warning_code) = if use_vad {
            match self.vad_factory.as_ref() {
                Some(factory) => match factory(&settings.vad) {
                    Ok(vad) => {
                        let silence_ms = settings.vad_silence_ms.clamp(
                            *crate::vad::SILENCE_MS_RANGE.start(),
                            *crate::vad::SILENCE_MS_RANGE.end(),
                        );
                        (
                            Some((
                                vad,
                                crate::vad::FrameSplitter::new(),
                                crate::vad::SilenceStopTracker::new(
                                    silence_ms,
                                    crate::vad::MIN_SESSION_MS,
                                ),
                            )),
                            None,
                        )
                    }
                    Err(error) => {
                        crate::log::log(&format!(
                            "VAD 初始化失败，本次会话降级为热键结束: {error}"
                        ));
                        (None, Some("VAD_INIT_FAILED"))
                    }
                },
                None => {
                    crate::log::log("VAD 工厂未接入，本次会话降级为热键结束");
                    (None, Some("VAD_NOT_CONFIGURED"))
                }
            }
        } else {
            (None, None)
        };
        let continuous_session = policy.continuous && vad_state.is_some();
        // 0 = 正常，1 = 推理失败但尚未上报，2 = 已上报。PCM pump 可能在
        // Listening 首个状态事件之前就拿到音频，这两个原子量保证降级不会漏报、
        // 不会重复上报，也不会被随后发出的普通 Listening 事件覆盖。
        let vad_inference_status = Arc::new(AtomicU8::new(0));
        let listening_state_emitted = Arc::new(AtomicBool::new(false));

        // PCM pump：录音 → session.push_audio（VAD 策略下并行喂 VAD）；
        // STT partial → kotone://partial；VAD 判停 → 触发 end()（与热键结束同路径）
        let emitter = self.emitter.clone();
        let pump_flag = cancelled_flag.clone();
        let pump_recorder = recorder.clone();
        let pump_history = history_draft.clone();
        let pump_case_id = process_case_id.clone();
        let pump_vad_inference_status = vad_inference_status.clone();
        let pump_listening_state_emitted = listening_state_emitted.clone();
        let max_session_duration = self.max_session_duration;
        let mut pump_first_partial_seen = false;
        // VAD 帧的累计采样偏移（相对 audio_pcm 起点；帧在流中连续，
        // 与 chunk 边界无关，每切出一帧 +VAD_FRAME_SAMPLES）
        let mut vad_frame_pos = 0usize;
        let me_weak = self.me.read().unwrap().clone();
        // begin 持有 op；所有异步结束回调必须携带本次即将发布的会话代际。
        let capture_gen = self.inner.lock().unwrap().gen + 1;
        let pump = tokio::spawn(async move {
            let mut session = session;
            let mut stop_rx = stop_rx;
            let mut pcm_rx = pcm_rx;
            let mut stt_rx = stt_rx;
            let mut audio_err_rx = audio_err_rx;
            let mut vad_state = vad_state;
            let mut stt_push_error = None;
            let session_deadline = tokio::time::sleep(max_session_duration);
            tokio::pin!(session_deadline);
            // 松手收尾：end() 先停采集再发停止信号，pump 收到信号后把通道里
            // 已缓冲未消费的 PCM 全部灌进 session（松手丢字 P0：采音侧排空）；
            // 取消（pump_flag）则立即停，音频丢弃
            loop {
                tokio::select! {
                    biased;
                    _ = &mut session_deadline => {
                        if let Some(me) = me_weak.as_ref().and_then(|weak| weak.upgrade()) {
                            tokio::spawn(async move {
                                me.abort_with_code(
                                    "单次录音已达 10 分钟上限，本次会话已中止，请重新开始",
                                    "SESSION_DURATION_LIMIT",
                                )
                                .await;
                            });
                        }
                        break;
                    }
                    r = &mut stop_rx => {
                        let _ = r;
                        if !pump_flag.load(Ordering::SeqCst) {
                            // 非阻塞排空：此时采集线程已被 join（end 先 drop
                            // guard），通道里剩下的就是全部缓冲
                            while let Ok(c) = pcm_rx.try_recv() {
                                if let Some(rec) = &pump_recorder {
                                    rec.push_pcm(&c);
                                }
                                if let Some(h) = &pump_history {
                                    let mut h = h.lock().unwrap();
                                    h.audio_samples += c.len() as u64;
                                    if let Some(pcm) = &mut h.audio_pcm {
                                        pcm.extend_from_slice(&c);
                                    }
                                }
                                if let Err(e) = session.push_audio(&c) {
                                    stt_push_error = Some(e);
                                    break;
                                }
                            }
                        }
                        break;
                    }
                    chunk = pcm_rx.recv() => {
                        match chunk {
                            Some(c) => {
                                if let Some(rec) = &pump_recorder {
                                    rec.push_pcm(&c);
                                }
                                if let Some(h) = &pump_history {
                                    let mut h = h.lock().unwrap();
                                    h.audio_samples += c.len() as u64;
                                    if let Some(pcm) = &mut h.audio_pcm {
                                        pcm.extend_from_slice(&c);
                                    }
                                }
                                if let Err(e) = session.push_audio(&c) {
                                    stt_push_error = Some(e);
                                    break;
                                }
                                // VAD 帧判定与 STT 并行（同一 PCM 流）；
                                // 推理失败只禁用判停（热键强制结束兜底），不断会话
                                if let Some((vad, splitter, tracker)) = &mut vad_state {
                                    let mut decision = crate::vad::StopDecision::Continue;
                                    let mut failed = None;
                                    for frame in splitter.push(&c) {
                                        match vad.push_frame(&frame) {
                                            Ok(speech) => {
                                                if speech {
                                                    // 语音判定：标记「开过口」（取消判据）
                                                    // 并记录语音区间（历史音频裁剪用）
                                                    if let Some(h) = &pump_history {
                                                        let mut g = h.lock().unwrap();
                                                        g.vad_speech_seen = true;
                                                        g.push_speech_range(
                                                            vad_frame_pos,
                                                            vad_frame_pos
                                                                + crate::vad::VAD_FRAME_SAMPLES,
                                                        );
                                                    }
                                                }
                                                decision = tracker.push(speech);
                                                if decision == crate::vad::StopDecision::Stop {
                                                    break;
                                                }
                                            }
                                            Err(e) => {
                                                failed = Some(e);
                                                break;
                                            }
                                        }
                                        vad_frame_pos += crate::vad::VAD_FRAME_SAMPLES;
                                    }
                                    if let Some(e) = failed {
                                        crate::log::log(&format!(
                                            "VAD 推理失败，本次会话禁用静音判停（热键仍可结束）: {e}"
                                        ));
                                        pump_vad_inference_status.store(1, Ordering::SeqCst);
                                        if pump_listening_state_emitted.load(Ordering::SeqCst)
                                            && pump_vad_inference_status
                                                .compare_exchange(
                                                    1,
                                                    2,
                                                    Ordering::SeqCst,
                                                    Ordering::SeqCst,
                                                )
                                                .is_ok()
                                        {
                                            let me = me_weak
                                                .as_ref()
                                                .and_then(|weak| weak.upgrade());
                                            let listening = {
                                                if let Some(me) = &me {
                                                    let mut inner = me.inner.lock().unwrap();
                                                    if inner.state
                                                        == OrchestratorState::Listening
                                                    {
                                                        inner.continuous_session = false;
                                                        true
                                                    } else {
                                                        false
                                                    }
                                                } else {
                                                    false
                                                }
                                            };
                                            if let Some(me) = me {
                                                if listening {
                                                    me.emit_state(
                                                        OrchestratorState::Listening,
                                                        Some(json!({
                                                            "warning": me.vad_fallback_warning(),
                                                            "manualStop": true,
                                                            "preserveText": true
                                                        })),
                                                    );
                                                }
                                                // 即使用户恰好已结束录音，也保留稳定错误码，
                                                // 但不把状态倒退回 Listening。
                                                me.emit_process(
                                                    "vad_degraded",
                                                    json!({
                                                        "outcome": "warning",
                                                        "errorCode": "VAD_INFERENCE_FAILED",
                                                        "fallback": "hotkey_stop"
                                                    }),
                                                );
                                            }
                                        }
                                        vad_state = None;
                                    } else if decision == crate::vad::StopDecision::Stop {
                                        emitter.emit(
                                            "kotone://vad-stop",
                                            json!({ "elapsedMs": tracker.elapsed_ms() }),
                                        );
                                        if let Some(me) =
                                            me_weak.as_ref().and_then(|w| w.upgrade())
                                        {
                                            tokio::spawn(async move {
                                                let _ = me.end_inner(Some(capture_gen), None).await;
                                            });
                                        }
                                        // 立刻交出 session：end() 正在等 session_rx
                                        break;
                                    }
                                }
                            }
                            None => break, // 采集结束
                        }
                    }
                    err = async {
                        match &mut audio_err_rx {
                            Some(rx) => rx.recv().await,
                            // 无错误通道（mock/测试）：分支永不完成，不影响原有 select
                            None => std::future::pending::<Option<String>>().await,
                        }
                    } => {
                        if let Some(msg) = err {
                            // 录音设备中途故障（拔麦克风/驱动错误）：中止会话并 toast，
                            // 避免状态机永久停在 Listening 无感知（P1-⑤）
                            if let Some(me) = me_weak.as_ref().and_then(|w| w.upgrade()) {
                                let msg2 = msg.clone();
                                tokio::spawn(async move {
                                    me.abort_with_message(&format!(
                                        "{msg2}（本次会话已中止，请检查麦克风后重试）"
                                    ))
                                    .await;
                                });
                            }
                            break;
                        }
                        // 通道关闭 = 采集线程正常退出（pcm 分支会收尾）；停用该分支防 busy 循环
                        audio_err_rx = None;
                    }
                    ev = stt_rx.recv() => {
                        match ev {
                            Some(SttEvent::Partial { text }) => {
                                if let Some(rec) = &pump_recorder {
                                    rec.push_partial(&text);
                                }
                                if let Some(h) = &pump_history {
                                    let mut g = h.lock().unwrap();
                                    if g.first_partial_ms.is_none() {
                                        g.first_partial_ms =
                                            Some(g.started.elapsed().as_millis() as u64);
                                    }
                                }
                                if !pump_first_partial_seen {
                                    pump_first_partial_seen = true;
                                    emitter.emit(
                                        "kotone://process",
                                        json!({
                                            "caseId": pump_case_id.clone(),
                                            "activity": "first_partial",
                                            "data": { "textChars": text.chars().count() as u64 }
                                        }),
                                    );
                                }
                                emitter.emit("kotone://partial", json!({ "text": text }));
                            }
                            // Final 由 finalize 返回值承载，这里不重复上屏
                            Some(SttEvent::Final { .. }) | None => {}
                        }
                    }
                }
            }
            if let Some(error) = stt_push_error {
                session.cancel();
                drop(session);
                if let Some(me) = me_weak.as_ref().and_then(|w| w.upgrade()) {
                    me.abort_with_code(
                        &format!("语音识别处理音频失败：{error}（本次会话已中止，请重试）"),
                        "STT_AUDIO_PUSH_FAILED",
                    )
                    .await;
                }
                return;
            }
            if pump_flag.load(Ordering::SeqCst) {
                session.cancel();
            }
            // 取消路径下接收端已 drop，send 失败则 session 随之释放
            let _ = session_tx.send(session);
        });

        // RMS 电平 → kotone://level（驱动波形，~50ms 一条）
        let emitter = self.emitter.clone();
        let level_task = tokio::spawn(async move {
            let mut level_rx = level_rx;
            while let Some(rms) = level_rx.recv().await {
                emitter.emit("kotone://level", json!({ "rms": rms }));
            }
        });

        let mut inner = self.inner.lock().unwrap();
        inner.gen += 1;
        inner.state = OrchestratorState::Listening;
        inner.session_policy = Some(policy);
        inner.pending_text = None;
        inner.processing_cancel = None;
        // 目标窗口记忆：用户按下热键说话前所在的前台窗口 = 注入目标，
        // 发送前（do_send）会把焦点还给它，避免 preview 交互抢焦点导致注入打错窗口
        inner.target_window = self.focus.foreground_window();
        inner.history = history_draft;
        inner.continuous_session =
            continuous_session && vad_inference_status.load(Ordering::SeqCst) == 0;
        inner.active = Some(ActiveSession {
            purpose,
            stop_tx: Some(stop_tx),
            session_rx: Some(session_rx),
            cancelled_flag,
            guard: Some(CaptureGuard {
                audio: Some(handle),
                _completion: capture_completion,
            }),
            pump,
            level_task,
            recorder,
        });
        drop(inner);
        let listening_payload = vad_warning_code.map(|_| {
            json!({
                "warning": self.vad_fallback_warning(),
                "manualStop": true
            })
        });
        self.emit_state(OrchestratorState::Listening, listening_payload);
        listening_state_emitted.store(true, Ordering::SeqCst);
        if let Some(error_code) = vad_warning_code {
            self.emit_process(
                "vad_degraded",
                json!({
                    "outcome": "warning",
                    "errorCode": error_code,
                    "fallback": "hotkey_stop"
                }),
            );
        }
        if vad_inference_status
            .compare_exchange(1, 2, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            let listening = {
                let mut inner = self.inner.lock().unwrap();
                if inner.state == OrchestratorState::Listening {
                    inner.continuous_session = false;
                    true
                } else {
                    false
                }
            };
            if listening {
                self.emit_state(
                    OrchestratorState::Listening,
                    Some(json!({
                        "warning": self.vad_fallback_warning(),
                        "manualStop": true,
                        "preserveText": true
                    })),
                );
            }
            self.emit_process(
                "vad_degraded",
                json!({
                    "outcome": "warning",
                    "errorCode": "VAD_INFERENCE_FAILED",
                    "fallback": "hotkey_stop"
                }),
            );
        }
        self.emit_process("capture_started", json!({ "resumed": resumed }));
        Ok(())
    }

    /// 重发最近一条：把历史里最新一条「发送成功」的文本重新注入（快捷键入口）。
    /// 语义（与用户确认）：
    /// - 仅 Idle 生效：会话进行中（录音/发送/预览等）按下静默忽略，不打断当前输入；
    /// - 用当前 profile + 当前 sticky 频道（与普通发送同一条 run_send 路径，
    ///   ADR-008 频道在发送时刻读取）；历史里的 profileId 不参与；
    /// - 无历史记录或文本为空 → 静默忽略（记日志）。
    pub async fn resend_last(&self) {
        if self.state() != OrchestratorState::Idle {
            return;
        }
        let dir = self
            .history_dir
            .clone()
            .unwrap_or_else(crate::history::history_dir);
        let text = match crate::history::list_in(&dir) {
            Ok(records) => records
                .iter()
                .find(|r| {
                    r.outcome == crate::history::HistoryOutcome::Sent
                        && !r.final_text.trim().is_empty()
                })
                .map(|r| r.final_text.clone()),
            Err(e) => {
                crate::log::log(&format!("重发最近一条：读取历史失败（忽略）: {e}"));
                return;
            }
        };
        let Some(text) = text else {
            crate::log::log("重发最近一条：历史中无发送成功的文本，忽略");
            return;
        };
        // 重发直发当前前台窗口：不沿用上次会话记录的注入目标（Idle 时
        // target_window 残留，若用户已切窗口，restore 会把焦点抢回旧窗口
        // 导致文本打进旧窗口）；target=None 时 run_send 跳过焦点恢复直发前台。
        // 历史读取期间可能有新会话开始；发送权必须在 op 锁内重新校验并占用。
        // 状态已经变化时保持原有语义：静默忽略，不打断当前输入。
        if let Ok(Some(claim)) = self
            .claim_send(
                Some(ReadyText::unchanged(text)),
                None,
                &[OrchestratorState::Idle],
                true,
                None,
            )
            .await
        {
            self.run_send(claim).await;
        }
    }

    /// 结束会话：finalize → autoSend 分流（§6 发送时序上半段）
    pub async fn end(&self) -> Result<(), String> {
        self.end_inner(None, None).await
    }

    /// 自动结束的代际校验与 Listening -> Transcribing 必须在同一 op 临界区。
    async fn end_inner(
        &self,
        expected_gen: Option<u64>,
        input_ticket: Option<u64>,
    ) -> Result<(), String> {
        let (gen, mut active) = {
            let _op = self.op.lock().await;
            if !self.input_ticket_is_current(input_ticket) {
                return Ok(());
            }
            let mut inner = self.inner.lock().unwrap();
            if expected_gen.is_some_and(|gen| inner.gen != gen) {
                return Ok(());
            }
            if inner.state != OrchestratorState::Listening {
                return Err(format!("当前状态 {:?} 不能结束会话", inner.state));
            }
            inner.gen += 1;
            let active = inner
                .active
                .take()
                .ok_or_else(|| "会话句柄缺失（内部错误）".to_string())?;
            inner.state = OrchestratorState::Transcribing;
            // 当前任务可能就是宽限任务；取走句柄不 abort，避免中途丢掉 finalize。
            inner.release_grace_task = None;
            let gen = inner.gen;
            drop(inner);
            self.emit_state(OrchestratorState::Transcribing, None);
            (gen, active)
        };
        self.emit_process("capture_stopped", json!({}));

        // 停 pump 并取回 session（不持锁，允许 cancel 插队，gen 校验兜底）。
        // 顺序关键（松手丢字 P0）：先 drop guard 停采集——采集线程 join 后 pcm
        // 通道不再有新数据，pump 收到停止信号才能一次性排空通道里已缓冲的 PCM
        drop(active.guard.take());
        if let Some(tx) = active.stop_tx.take() {
            let _ = tx.send(());
        }
        let session = match active.session_rx.take() {
            Some(rx) => match rx.await {
                Ok(s) => s,
                Err(_) => {
                    self.fail(gen, "录音线程异常结束", None);
                    return Err("录音线程异常结束".into());
                }
            },
            None => {
                self.fail(gen, "会话句柄缺失（内部错误）", None);
                return Err("会话句柄缺失".into());
            }
        };
        // 停止辅助任务
        active.pump.abort();
        active.level_task.abort();

        // finalize（同步引擎可能耗时，spawn_blocking + 10s 超时）
        self.emit_process("finalize_started", json!({}));
        let finalize = tokio::task::spawn_blocking(move || session.finalize());
        let result = match tokio::time::timeout(self.finalize_timeout, finalize).await {
            Ok(Ok(Ok(t))) => Ok(t),
            Ok(Ok(Err(e))) => Err(e),
            Ok(Err(_)) => Err("转写线程异常".to_string()),
            Err(_) => Err(format!("转写超时（{}s）", self.finalize_timeout.as_secs())),
        };

        // 期间被取消 → 丢弃过期结果
        if self.inner.lock().unwrap().gen != gen {
            return Ok(());
        }

        match result {
            Ok(t) => {
                // 空转录绝不能继续注入（空文本会导致只敲聊天键），并保留足够的
                // 脱敏指标供诊断。普通模式进入 Error 告知用户；solo 连续模式仍
                // 直接续录，避免把一句静音打断变成必须手动恢复的错误状态。
                if t.text.trim().is_empty() {
                    let audio_ms = self.inner.lock().unwrap().history.as_ref().map(|draft| {
                        let draft = draft.lock().unwrap();
                        draft.audio_samples * 1000 / crate::eval::SAMPLE_RATE as u64
                    });
                    self.emit_process(
                        "transcript_empty",
                        json!({
                            "outcome": "empty",
                            "durationMs": t.latency_ms as u64,
                            "audioMs": audio_ms,
                            "textChars": 0
                        }),
                    );
                    // 录档句柄直接 drop（不落盘）；history 草稿清掉不写记录
                    drop(active.recorder.take());
                    let continuous = self.continuous_session();
                    // 块作用域收窄 MutexGuard（绝不跨 await 持锁）。非连续模式保留
                    // Error，让用户明确知道这次没有识别到可发送文字。
                    {
                        let _op = self.op.lock().await;
                        let mut inner = self.inner.lock().unwrap();
                        if inner.gen != gen {
                            return Ok(());
                        }
                        inner.history = None;
                        inner.state = if continuous {
                            OrchestratorState::Idle
                        } else {
                            OrchestratorState::Error
                        };
                    }
                    if continuous {
                        self.emit_state(OrchestratorState::Idle, None);
                        // solo 连续模式：立即开下一段（同 schedule_idle 的 continuous 处理；
                        // begin 失败走自身 toast_error 收尾；续录不重复播「录制音」
                        let _ = self.begin_resumed(active.purpose, gen).await;
                    } else {
                        self.emit_state(
                            OrchestratorState::Error,
                            Some(json!({
                                "message": "未识别到语音，请按住热键后再说一遍",
                                "audioMs": audio_ms
                            })),
                        );
                    }
                    return Ok(());
                }
                if active.purpose == SessionPurpose::Assistant {
                    let received_at_ms = crate::reminders::now_ms();
                    let request = {
                        let _op = self.op.lock().await;
                        let mut inner = self.inner.lock().unwrap();
                        if inner.gen != gen || inner.state != OrchestratorState::Transcribing {
                            return Ok(());
                        }
                        let request = AssistantRequest {
                            session_id: inner.process_case_id.clone().unwrap_or_default(),
                            text: t.text,
                            received_at_ms,
                        };
                        if inner
                            .session_policy
                            .is_some_and(|policy| policy.post == PostFinalize::PreviewConfirm)
                        {
                            let text = request.text.clone();
                            inner.pending_text = Some(PendingText::Assistant(request));
                            inner.state = OrchestratorState::Preview;
                            drop(inner);
                            self.emit_state(
                                OrchestratorState::Preview,
                                Some(json!({ "text": text })),
                            );
                            return Ok(());
                        }
                        request
                    };
                    return self.run_assistant(gen, request).await;
                }
                self.emit_process(
                    "transcript_ready",
                    json!({
                        "outcome": "success",
                        "durationMs": t.latency_ms as u64,
                        "textChars": t.text.chars().count() as u64
                    }),
                );
                // history 草稿补齐 finalize 产物（终态落账时写入记录）
                if let Some(h) = &self.inner.lock().unwrap().history {
                    let mut g = h.lock().unwrap();
                    g.finalize_latency_ms = Some(t.latency_ms as u64);
                }
                // 历史音频独立落盘：即使 evalRecording 关闭也能保存与播放。
                self.persist_history_audio();
                // eval 录档落盘（wav + partial 时间线 + 指标）；失败静默记日志不阻断流程
                if let Some(rec) = active.recorder.take() {
                    match rec.finish(&t.text, t.latency_ms as u64) {
                        Ok(saved) => {
                            crate::log::log(&format!("eval 录档完成: {}", saved.session_id))
                        }
                        Err(e) => crate::log::log(&format!("eval 录档失败（忽略）: {e}")),
                    }
                }
                // STT 产物与可交付文本从此分离。pipeline 关闭/为空时 runner 走零步骤
                // 透传，不产生 Processing 状态，保证旧配置行为与事件序列不变。
                let recognized = self.snapshot_recognized_text(t.text);
                if let Some(h) = &self.inner.lock().unwrap().history {
                    h.lock().unwrap().source_text = Some(recognized.source_text.clone());
                }
                self.run_postprocessing(gen, recognized, &[OrchestratorState::Transcribing])
                    .await?;
            }
            Err(e) => {
                self.emit_process(
                    "recognize_failed",
                    json!({ "outcome": "error", "errorCode": "STT_FINALIZE_FAILED" }),
                );
                self.fail(gen, &e, None);
                return Err(e);
            }
        }
        Ok(())
    }

    /// 注入可发现的后处理器注册表；与 STT 引擎一样由外层实现 crate 负责注册。
    pub fn with_processors(mut self, processors: Arc<ProcessorRegistry>) -> Self {
        self.processors = processors;
        self
    }

    async fn run_assistant(&self, gen: u64, request: AssistantRequest) -> Result<(), String> {
        let Some((handler, reminders)) = &self.assistant else {
            self.fail(gen, "助手尚未配置", None);
            return Err("助手尚未配置".into());
        };
        let received_at_ms = request.received_at_ms;
        let text = request.text.clone();
        let session_id = request.session_id.clone();
        let cancel = crate::cancel::CancellationToken::default();
        {
            let _op = self.op.lock().await;
            let mut inner = self.inner.lock().unwrap();
            if inner.gen != gen || inner.state != OrchestratorState::Transcribing {
                return Ok(());
            }
            inner.state = OrchestratorState::Processing;
            inner.processing_cancel = Some(cancel.clone());
        }
        self.emit_state(
            OrchestratorState::Processing,
            Some(json!({
                "purpose": "assistant", "text": text, "displayName": "正在理解提醒请求"
            })),
        );
        let result = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Ok(()),
            result = tokio::time::timeout(Duration::from_secs(45), handler.handle(request, cancel.clone())) => {
                match result {
                    Ok(result) => result,
                    Err(_) => { cancel.cancel(); Err("助手请求超时，请稍后重试".into()) }
                }
            },
        };
        // 提交与取消共用 op 临界区；即使处理器忽略取消并迟到返回，也不能写入提醒。
        let _op = self.op.lock().await;
        {
            let mut inner = self.inner.lock().unwrap();
            if inner.gen != gen || inner.state != OrchestratorState::Processing {
                return Ok(());
            }
            inner.processing_cancel = None;
        }
        let plan = match result {
            Ok(plan) => plan,
            Err(error) => {
                self.fail(gen, &error, None);
                return Err(error);
            }
        };
        let Some(scheduled) = cancel.run_if_active(|| {
            reminders.schedule_batch(&session_id, received_at_ms, &plan.reminders)
        }) else {
            return Ok(());
        };
        let scheduled = match scheduled {
            Ok(scheduled) => scheduled,
            Err(error) => {
                self.fail(gen, &error, None);
                return Err(error);
            }
        };
        let message = if !plan.message.trim().is_empty() {
            plan.message.trim().chars().take(300).collect::<String>()
        } else if scheduled.is_empty() {
            "没有创建提醒，请说明提醒时间和内容".into()
        } else {
            format!("已设置 {} 条提醒", scheduled.len())
        };
        self.inner.lock().unwrap().state = OrchestratorState::Success;
        self.emitter.emit(
            "kotone://assistant",
            json!({
                "sessionId": session_id, "text": text, "message": message, "reminders": scheduled,
            }),
        );
        self.emit_state(
            OrchestratorState::Success,
            Some(json!({ "purpose": "assistant", "text": message })),
        );
        self.schedule_idle(gen);
        Ok(())
    }

    fn snapshot_recognized_text(&self, source_text: String) -> RecognizedText {
        let settings = self.settings.read().unwrap().clone();
        let inner = self.inner.lock().unwrap();
        RecognizedText {
            source_text,
            pipeline: settings.post_processing,
            context: ProcessingContext {
                session_id: inner.process_case_id.clone().unwrap_or_default(),
                profile_id: settings.active_profile_id,
                channel_id: inner.channel.as_ref().map(|(_, id)| id.clone()),
            },
        }
    }

    /// 编译并运行一份会话级 pipeline 快照。只有非空 pipeline 才进入 Processing；
    /// Required 错误保留 RecognizedText，confirm_send 会重新运行处理而不会绕过。
    async fn run_postprocessing(
        &self,
        gen: u64,
        recognized: RecognizedText,
        expected_states: &[OrchestratorState],
    ) -> Result<(), String> {
        let pipeline = match PostProcessPipeline::compile(&recognized.pipeline, &self.processors) {
            Ok(pipeline) => pipeline,
            Err(error) => {
                self.finish_processing_error(gen, recognized, &error, expected_states)
                    .await;
                return Err(error);
            }
        };

        if pipeline.is_empty() {
            return self
                .deliver_ready_text(gen, ReadyText::unchanged(recognized.source_text))
                .await;
        }

        let cancel = ProcessingCancelToken::default();
        {
            let _op = self.op.lock().await;
            let mut inner = self.inner.lock().unwrap();
            if inner.gen != gen || !expected_states.contains(&inner.state) {
                return Ok(());
            }
            inner.state = OrchestratorState::Processing;
            inner.pending_text = Some(PendingText::Recognized(recognized.clone()));
            inner.processing_cancel = Some(cancel.clone());
        }
        self.emit_state(
            OrchestratorState::Processing,
            Some(json!({
                "text": recognized.source_text,
                "pipelineId": recognized.pipeline.active_pipeline().map(|p| p.id.clone()).unwrap_or_default(),
                "stepCount": pipeline.len(),
            })),
        );
        self.emit_process(
            "postprocess_started",
            json!({
                "pipelineId": recognized.pipeline.active_pipeline().map(|p| p.id.clone()).unwrap_or_default(),
                "stepCount": pipeline.len() as u64,
            }),
        );

        let started = std::time::Instant::now();
        let progress_observer = |progress: crate::postprocess::PipelineStepProgress| {
            self.emit_state(
                OrchestratorState::Processing,
                Some(json!({
                    "text": recognized.source_text,
                    "pipelineId": recognized.pipeline.active_pipeline().map(|p| p.id.clone()).unwrap_or_default(),
                    "stepId": progress.step_id,
                    "processorId": progress.processor_id,
                    "displayName": progress.display_name,
                    "index": progress.index,
                    "total": progress.total,
                })),
            );
        };
        let result = pipeline
            .run_with_progress(
                recognized.source_text.clone(),
                &recognized.context,
                cancel,
                Some(&progress_observer),
            )
            .await;

        {
            let _op = self.op.lock().await;
            let mut inner = self.inner.lock().unwrap();
            if inner.gen != gen || inner.state != OrchestratorState::Processing {
                return Ok(());
            }
            inner.processing_cancel = None;
        }

        match result {
            Ok(result) => {
                let duration_ms = started.elapsed().as_millis() as u64;
                if let Some(h) = &self.inner.lock().unwrap().history {
                    h.lock().unwrap().process_duration_ms = Some(duration_ms);
                }
                self.emit_process(
                    "postprocess_completed",
                    json!({
                        "pipelineId": result.pipeline_id,
                        "stepCount": result.trace.len() as u64,
                        "durationMs": duration_ms,
                        "sourceChars": result.source_text.chars().count() as u64,
                        "finalChars": result.final_text.chars().count() as u64,
                    }),
                );
                self.deliver_ready_text(
                    gen,
                    ReadyText {
                        source_text: result.source_text,
                        text: result.final_text,
                    },
                )
                .await
            }
            Err(error) if error.kind == ProcessErrorKind::Cancelled => Ok(()),
            Err(error) => {
                let message = format!("后处理步骤「{}」失败：{}", error.step_id, error.message);
                self.finish_processing_error(
                    gen,
                    recognized,
                    &message,
                    &[OrchestratorState::Processing],
                )
                .await;
                Err(message)
            }
        }
    }

    async fn finish_processing_error(
        &self,
        gen: u64,
        recognized: RecognizedText,
        message: &str,
        expected_states: &[OrchestratorState],
    ) {
        {
            let _op = self.op.lock().await;
            let mut inner = self.inner.lock().unwrap();
            if inner.gen != gen || !expected_states.contains(&inner.state) {
                return;
            }
            inner.processing_cancel = None;
            inner.pending_text = Some(PendingText::Recognized(recognized.clone()));
            inner.state = OrchestratorState::Error;
        }
        self.emit_state(
            OrchestratorState::Error,
            Some(json!({
                "message": message,
                "text": recognized.source_text,
                "retryStage": "processing",
            })),
        );
        self.emit_process(
            "postprocess_failed",
            json!({ "outcome": "error", "errorCode": "POSTPROCESS_FAILED" }),
        );
        self.write_history(
            crate::history::HistoryOutcome::Error,
            Some(message.to_string()),
        );
    }

    /// 所有可交付文本的唯一出口：更新 UI/history 后，按交互策略进入预览或发送。
    /// STT、未来后处理和处理失败重试都必须先产出 ReadyText，再调用本方法；
    /// injector 与 Preview 因而永远不会接触「尚未处理完成」的识别原文。
    async fn deliver_ready_text(&self, gen: u64, ready: ReadyText) -> Result<(), String> {
        if self.inner.lock().unwrap().gen != gen {
            return Ok(());
        }

        self.emitter
            .emit("kotone://partial", json!({ "text": ready.text }));
        if let Some(history) = &self.inner.lock().unwrap().history {
            history.lock().unwrap().final_text = ready.text.clone();
        }

        match self.policy().post {
            PostFinalize::SendDirect => {
                if let Some(claim) = self
                    .claim_send(
                        Some(ready),
                        Some(gen),
                        &[
                            OrchestratorState::Transcribing,
                            OrchestratorState::Processing,
                        ],
                        false,
                        None,
                    )
                    .await?
                {
                    self.run_send(claim).await;
                }
            }
            PostFinalize::PreviewConfirm => {
                let _op = self.op.lock().await;
                let mut inner = self.inner.lock().unwrap();
                if inner.gen != gen
                    || ![
                        OrchestratorState::Transcribing,
                        OrchestratorState::Processing,
                    ]
                    .contains(&inner.state)
                {
                    return Ok(());
                }
                let text = ready.text.clone();
                inner.pending_text = Some(PendingText::Ready(ready));
                inner.state = OrchestratorState::Preview;
                drop(inner);
                self.emit_state(OrchestratorState::Preview, Some(json!({ "text": text })));
            }
        }
        Ok(())
    }

    /// Preview 状态下用户确认发送；Error 状态（payload 带文本）也接受，
    /// 重新进入 Sending 实现重试（docs/development.md §4.1「Error 保留文本可重试」）。
    /// ADR-006 预览只读化：不再接受编辑文本，一律发送已定的 ReadyText。
    pub async fn confirm_send(&self) -> Result<(), String> {
        self.confirm_send_inner(None, None).await
    }

    async fn confirm_send_inner(
        &self,
        input_ticket: Option<u64>,
        expected_gen: Option<u64>,
    ) -> Result<(), String> {
        let (assistant_preview, confirmation_gen) = {
            let _op = self.op.lock().await;
            if !self.input_ticket_is_current(input_ticket) {
                return Ok(());
            }
            let mut inner = self.inner.lock().unwrap();
            if expected_gen.is_some_and(|gen| inner.gen != gen) {
                return Ok(());
            }
            let gen = inner.gen;
            let preview = if inner.purpose == SessionPurpose::Assistant {
                if !self.runtime_inputs_enabled.load(Ordering::SeqCst) {
                    return Err("运行时已停止，不能提交助手请求".into());
                }
                if inner.state != OrchestratorState::Preview {
                    return Err("当前没有待确认的助手请求".into());
                }
                let Some(PendingText::Assistant(request)) = inner.pending_text.take() else {
                    return Err("待确认的助手请求已失效".into());
                };
                inner.state = OrchestratorState::Transcribing;
                Some((inner.gen, request))
            } else {
                None
            };
            (preview, gen)
        };
        if let Some((gen, request)) = assistant_preview {
            return self.run_assistant(gen, request).await;
        }
        let retry_processing = {
            let inner = self.inner.lock().unwrap();
            if inner.gen == confirmation_gen && inner.state == OrchestratorState::Error {
                inner
                    .pending_text
                    .as_ref()
                    .and_then(PendingText::recognized)
                    .cloned()
                    .map(|recognized| (inner.gen, recognized))
            } else {
                None
            }
        };
        if let Some((gen, recognized)) = retry_processing {
            return self
                .run_postprocessing(gen, recognized, &[OrchestratorState::Error])
                .await;
        }

        let claim = self
            .claim_send(
                None,
                Some(confirmation_gen),
                &[OrchestratorState::Preview, OrchestratorState::Error],
                false,
                input_ticket,
            )
            .await?
            .ok_or_else(|| "待发送内容已失效".to_string())?;
        self.run_send(claim).await;
        Ok(())
    }

    /// 录音设备故障等会话中止：与 cancel 相同清理，但以 Error toast 告知原因
    /// （P1-⑤：拔麦克风/驱动错误不再永久卡 Listening）。
    /// Error 保持到用户关闭/重试，确保按需悬浮窗也能让用户看到核心链路故障。
    pub async fn abort_with_message(&self, message: &str) {
        self.abort_with_code(message, "AUDIO_DEVICE_ERROR").await;
    }

    /// 会话泵的致命错误统一中止入口：清理录音/STT/发送句柄并转入可见 Error。
    async fn abort_with_code(&self, message: &str, error_code: &str) {
        // 宽限期倒计时一并收回
        self.abort_release_grace();
        let _op = self.op.lock().await;
        let gen;
        {
            let mut inner = self.inner.lock().unwrap();
            if inner.state == OrchestratorState::Idle {
                return;
            }
            inner.gen += 1;
            gen = inner.gen;
            let mut active = inner.active.take();
            inner.pending_text = None;
            if let Some(operation) = &inner.send_operation {
                operation.cancel();
            }
            if let Some(token) = inner.processing_cancel.take() {
                token.cancel();
            }
            inner.state = OrchestratorState::Error;
            drop(inner);
            if let Some(a) = active.as_mut() {
                a.cancelled_flag.store(true, Ordering::SeqCst);
                if let Some(tx) = a.stop_tx.take() {
                    let _ = tx.send(());
                }
                drop(a.guard.take());
                a.pump.abort();
                a.level_task.abort();
            }
        }
        self.emit_state(
            OrchestratorState::Error,
            Some(json!({ "message": message })),
        );
        self.emit_process(
            "session_aborted",
            json!({ "outcome": "error", "errorCode": error_code }),
        );
        self.write_history(crate::history::HistoryOutcome::Cancelled, None);
        self.schedule_idle(gen);
    }

    /// 任意状态取消：回到 Idle（session cancel；发送中置取消令牌）
    pub async fn cancel(&self) {
        self.cancel_inner(None, None).await;
    }

    async fn cancel_inner(&self, input_ticket: Option<u64>, expected_gen: Option<u64>) {
        let _op = self.op.lock().await;
        if !self.input_ticket_is_current(input_ticket) {
            return;
        }
        if expected_gen.is_some_and(|gen| self.inner.lock().unwrap().gen != gen) {
            return;
        }
        self.cancel_locked();
    }

    /// 调用方持有 op；按钮取消在核验归属后同一临界区内立即取消 AI。
    fn cancel_locked(&self) {
        // 宽限期倒计时一并收回（Esc / 停止 runtime / 状态变迁兜底）
        self.abort_release_grace();
        let mut inner = self.inner.lock().unwrap();
        inner.assistant_voice_owner = false;
        if inner.state == OrchestratorState::Idle {
            // 逐句模式两段之间的 Idle 窗口也必须撤销尚未入场的续录。
            inner.gen += 1;
            return;
        }
        inner.gen += 1;
        let mut active = inner.active.take();
        inner.pending_text = None;
        if let Some(operation) = &inner.send_operation {
            operation.cancel();
        }
        if let Some(token) = inner.processing_cancel.take() {
            token.cancel();
        }
        inner.state = OrchestratorState::Idle;
        drop(inner);

        if let Some(a) = active.as_mut() {
            a.cancelled_flag.store(true, Ordering::SeqCst);
            if let Some(tx) = a.stop_tx.take() {
                let _ = tx.send(());
            }
            drop(a.guard.take());
            a.pump.abort();
            a.level_task.abort();
            // session_rx 随 ActiveSession drop：pump 收尾时 session.cancel() 后发送失败即释放
        }
        self.emit_state(OrchestratorState::Idle, None);
        self.emit_process("session_cancelled", json!({ "outcome": "cancelled" }));
        // error 已落账过的草稿跳过（Error 后的 Esc 是清理动作，不双记 cancelled）
        self.write_history(crate::history::HistoryOutcome::Cancelled, None);
    }

    // ---------- 内部 ----------

    /// 在 `op` 锁内校验来源状态并原子进入 Sending，保证每个状态只能产生一个发送任务。
    /// `ready=None` 用于 Preview/Error 重试：文本也在同一临界区内从 pending_text 取出。
    async fn claim_send(
        &self,
        ready: Option<ReadyText>,
        expected_gen: Option<u64>,
        expected_states: &[OrchestratorState],
        clear_target: bool,
        input_ticket: Option<u64>,
    ) -> Result<Option<SendClaim>, String> {
        let _op = self.op.lock().await;
        if !self.input_ticket_is_current(input_ticket) {
            return Ok(None);
        }
        if !self.runtime_inputs_enabled.load(Ordering::SeqCst) {
            return Err("运行时已停止，不能发送".into());
        }
        let claim = {
            let mut inner = self.inner.lock().unwrap();
            if expected_gen.is_some_and(|gen| inner.gen != gen) {
                return Ok(None);
            }
            if !expected_states.contains(&inner.state) {
                return Err(format!("当前状态 {:?} 不能发送", inner.state));
            }
            if clear_target && inner.state == OrchestratorState::Idle {
                // 历史重发是显式的听写操作，不沿用上一次助手会话的用途。
                inner.purpose = SessionPurpose::Dictation;
                inner.session_policy = None;
            } else if inner.purpose == SessionPurpose::Assistant {
                return Err("助手请求不能进入游戏发送管线".into());
            }
            if inner.send_is_running() {
                return Err(SEND_STILL_RUNNING.into());
            }
            let ready = match ready {
                Some(ready) => ready,
                None => inner
                    .pending_text
                    .as_ref()
                    .and_then(PendingText::ready)
                    .cloned()
                    .ok_or_else(|| "无待发送文本".to_string())?,
            };
            if clear_target {
                inner.target_window = None;
            }
            let (operation, completion) = SendOperation::new();
            inner.state = OrchestratorState::Sending;
            inner.send_operation = Some(operation.clone());
            SendClaim {
                ready,
                gen: inner.gen,
                operation,
                completion,
                target: inner.target_window,
                channel: inner.channel.clone(),
            }
        };
        self.emit_state(
            OrchestratorState::Sending,
            Some(json!({ "text": claim.ready.text })),
        );
        Ok(Some(claim))
    }

    async fn wait_for_modifiers_up(&self, token: &CancelToken) -> Result<(), InjectError> {
        const MAX_WAIT: Duration = Duration::from_millis(1000);
        const POLL: Duration = Duration::from_millis(10);
        let started = std::time::Instant::now();
        if token.is_cancelled() {
            return Err(InjectError::new("发送已取消"));
        }
        if !self.focus.modifier_keys_down() {
            return Ok(());
        }
        crate::log::log("inject: waiting for Ctrl/Alt/Shift/Win to release");
        loop {
            if token.is_cancelled() {
                return Err(InjectError::new("发送已取消"));
            }
            if !self.focus.modifier_keys_down() {
                crate::process_log::record_activity(
                    "inject_wait_modifiers",
                    crate::process_log::EventData {
                        detail: Some(format!("waitedMs={}", started.elapsed().as_millis())),
                        outcome: Some("ok".into()),
                        ..Default::default()
                    },
                );
                return Ok(());
            }
            if started.elapsed() >= MAX_WAIT {
                crate::log::log(
                    "inject: modifiers still down after 1000ms; preserving text instead of sending",
                );
                crate::process_log::record_activity(
                    "inject_wait_modifiers",
                    crate::process_log::EventData {
                        detail: Some("timeoutMs=1000".into()),
                        outcome: Some("timeout".into()),
                        ..Default::default()
                    },
                );
                return Err(InjectError::new(
                    "发送前 Ctrl、Alt、Shift 或 Win 仍未松开；已保留文字，请松开按键后重试",
                ));
            }
            tokio::time::sleep(POLL).await;
        }
    }

    /// 焦点恢复和输入注入共享同一个阻塞任务/完成凭据，超时也不能另开并行任务。
    async fn execute_send(
        &self,
        completion: SendCompletion,
        token: CancelToken,
        target: Option<TargetWindow>,
        text: String,
        profile: GameProfile,
    ) -> Result<(), InjectError> {
        let injector = self.injector.clone();
        let focus = self.focus.clone();
        let restore_delay = self.focus_restore_delay;
        let worker_token = token.clone();
        let mut task = tokio::task::spawn_blocking(move || {
            let _completion = completion;
            if worker_token.is_cancelled() {
                return Err(InjectError::new("发送已取消"));
            }
            // 检查放在阻塞线程内，覆盖任务排队期间的取消。已经进入的系统调用
            // 无法强制中断；完成凭据会一直保留到它返回，阻止后续发送与之重叠。
            if let Some(target) = target {
                if focus.restore(target) && restore_delay > Duration::ZERO {
                    std::thread::sleep(restore_delay);
                }
            }
            if worker_token.is_cancelled() {
                return Err(InjectError::new("发送已取消"));
            }
            let fg_before = focus.foreground_process_name();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(500)).await;
                let fg_after = focus.foreground_process_name();
                crate::log::log(&format!(
                    "inject focus probe: before={} after500ms={}",
                    fg_before.as_deref().unwrap_or("(none)"),
                    fg_after.as_deref().unwrap_or("(none)")
                ));
            });
            injector.send(&text, &profile, worker_token)
        });
        match tokio::time::timeout(self.inject_timeout, &mut task).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(InjectError::new("发送线程异常")),
            Err(_) => {
                token.cancel();
                // 保留并观察 JoinHandle；状态可返回 Error，但发送权由工作线程的
                // SendCompletion 持有，不能靠重启 runtime 绕过仍未结束的系统调用。
                tokio::spawn(async move {
                    if let Err(error) = task.await {
                        crate::log::log(&format!(
                            "timed-out send worker ended abnormally: {error}"
                        ));
                    }
                });
                Err(InjectError::new(format!(
                    "发送超时（{}s），已请求取消。目标窗口可能已有部分输入，请先检查；\
                     上一次发送结束前无法重试，若持续无响应，请退出并重新打开应用",
                    self.inject_timeout.as_secs()
                )))
            }
        }
    }

    /// Sending → Success/Error（§6 发送时序下半段；inject 实现负责按键细节）。
    /// 仅接受 `claim_send` 产生的已占用发送任务，不再改写状态或取消令牌。
    async fn run_send(&self, claim: SendClaim) {
        let SendClaim {
            ready,
            gen,
            operation,
            completion,
            target,
            channel,
        } = claim;
        let token = operation.0.cancel.clone();
        let ReadyText { source_text, text } = ready;
        let injection_started_at = std::time::Instant::now();
        self.emit_process(
            "injection_started",
            json!({
                "textChars": text.chars().count() as u64,
                "channelId": channel.as_ref().map(|(_, cid)| cid.clone()),
            }),
        );
        let settings = self.settings.read().unwrap().clone();
        let profile = settings
            .active_profile_id
            .as_deref()
            .and_then(profile::get)
            .unwrap_or_else(GameProfile::builtin_generic);

        // ADR-008 频道策略：克隆 profile 换开框键、文本加前缀——
        // 用户原文（预览/历史/重试）不受频道前缀污染。
        let strategy = resolve_send_strategy(&profile, &channel);
        let mut wire_profile = profile.clone();
        wire_profile.open_chat_key = strategy.open_chat_key.clone();
        let wire_text = if strategy.text_prefix.is_empty() {
            text.clone()
        } else {
            format!("{}{text}", strategy.text_prefix)
        };

        // Alt+F6 等组合热键在主键 down 时就触发；修饰键从不吞键，此时 Alt
        // 仍按着。立刻开框/打字会变成 Alt+Enter，游戏里看不到消息，注入却成功。
        let modifier_result = self.wait_for_modifiers_up(&token).await;
        let modifiers_failed = modifier_result.is_err() && !token.is_cancelled();
        let result = match modifier_result {
            Err(error) => {
                // 准备阶段也走下面唯一的发送收尾出口，清理操作权并保留 ReadyText。
                drop(completion);
                Err(error)
            }
            Ok(()) => {
                self.execute_send(completion, token, target, wire_text, wire_profile)
                    .await
            }
        };

        let (history_write, resume_continuous) = {
            let _op = self.op.lock().await;
            let mut inner = self.inner.lock().unwrap();
            if inner
                .send_operation
                .as_ref()
                .is_some_and(|current| current.same_operation(&operation) && current.is_finished())
            {
                inner.send_operation = None;
            }
            if inner.gen != gen || inner.state != OrchestratorState::Sending {
                return;
            }
            // history 终态（锁外落账，见下方 write_history 调用；两个分支各自产出
            // (outcome, 错误信息, 是否持续续录)，直接用 match 表达式——新 clippy
            // `unneeded late initialization` 会拒绝「先声明再在两个分支赋值」的写法。
            match result {
                Ok(()) => {
                    inner.pending_text = None;
                    let resuming = inner.continuous_session;
                    inner.state = if resuming {
                        OrchestratorState::Idle
                    } else {
                        OrchestratorState::Success
                    };
                    drop(inner);
                    if !resuming {
                        self.emit_state(OrchestratorState::Success, Some(json!({ "text": text })));
                    }
                    self.emit_process(
                        "injection_succeeded",
                        json!({
                            "outcome": "success",
                            "durationMs": injection_started_at.elapsed().as_millis() as u64
                        }),
                    );
                    ((crate::history::HistoryOutcome::Sent, None), resuming)
                }
                Err(e) => {
                    // Error 保留 ReadyText，前端/confirm_send 可重试（§4.1）；
                    // needsElevation 透传 UIPI 提权信号（§10 R-1）；
                    // inputBlocked 透传「系统级拦截」信号（安全软件/反作弊），前端弹窗引导排查；
                    // 注入超时（inject_timeout）也落此分支，message 含超时提示
                    inner.pending_text = Some(PendingText::Ready(ReadyText {
                        source_text,
                        text: text.clone(),
                    }));
                    inner.state = OrchestratorState::Error;
                    drop(inner);
                    self.emit_state(
                        OrchestratorState::Error,
                        Some(json!({ "message": e.message, "needsElevation": e.needs_elevation, "inputBlocked": e.input_blocked, "text": text })),
                    );
                    self.emit_process(
                        "injection_failed",
                        json!({
                            "outcome": "error",
                            "errorCode": if modifiers_failed {
                                "MODIFIERS_STILL_DOWN"
                            } else if e.input_blocked {
                                "INJECTION_BLOCKED"
                            } else if e.needs_elevation {
                                "INJECTION_ELEVATION_REQUIRED"
                            } else {
                                "INJECTION_FAILED"
                            },
                            "durationMs": injection_started_at.elapsed().as_millis() as u64
                        }),
                    );
                    (
                        (crate::history::HistoryOutcome::Error, Some(e.message)),
                        false,
                    )
                }
            }
        };
        self.write_history(history_write.0, history_write.1);
        if resume_continuous {
            // solo 不显示 Success toast，也不等待 toast_dwell：发送落账后立即开始下一段。
            // 续录不重复播「录制音」（capture_started resumed=true）。
            let _ = self.begin_resumed(SessionPurpose::Dictation, gen).await;
        } else {
            self.schedule_idle(gen);
        }
    }

    /// 开始失败等场景：Error 保持到用户关闭/重试（无文本，不可发送重试）
    fn toast_error(&self, message: &str) {
        let gen = {
            let mut inner = self.inner.lock().unwrap();
            inner.pending_text = None;
            inner.processing_cancel = None;
            inner.state = OrchestratorState::Error;
            inner.gen
        };
        self.emit_state(
            OrchestratorState::Error,
            Some(json!({ "message": message })),
        );
        self.schedule_idle(gen);
    }

    /// 转写失败：Error toast（保留文本可重试时经 fail 的 text 参数携带）
    fn fail(&self, gen: u64, message: &str, text: Option<String>) {
        let mut inner = self.inner.lock().unwrap();
        if inner.gen != gen {
            return;
        }
        inner.pending_text = text
            .clone()
            .map(ReadyText::unchanged)
            .map(PendingText::Ready);
        inner.processing_cancel = None;
        inner.state = OrchestratorState::Error;
        drop(inner);
        self.emit_state(
            OrchestratorState::Error,
            Some(json!({ "message": message, "text": text })),
        );
        self.write_history(
            crate::history::HistoryOutcome::Error,
            Some(message.to_string()),
        );
        self.schedule_idle(gen);
    }

    /// history 终态落账：取草稿快照 → 组记录 → 追加到 history.jsonl。
    /// - mode=off / 无草稿 → 不落盘；已结束的会话仍终结草稿，避免重新启用后补写旧记录；
    /// - error：落账但保留草稿（Error 态可重试，重试成功同 sessionId 再写一条 sent，
    ///   两条记录是刻意的「失败→重试成功」叙事）；
    /// - cancelled：草稿已被 error 落账过则跳过（Error 后的 Esc 是清理动作，不双记）；
    /// - sent / cancelled：落账并终结草稿；
    /// - IO 失败静默记日志，绝不炸主流程。
    fn write_history(&self, outcome: crate::history::HistoryOutcome, error: Option<String>) {
        let draft = match self.inner.lock().unwrap().history.clone() {
            Some(d) => d,
            None => return,
        };
        let cfg = self.settings.read().unwrap().history.clone();
        if cfg.mode == crate::history::HistoryMode::Off {
            if outcome != crate::history::HistoryOutcome::Error {
                // Off 只禁止持久化，不延长已取消/发送会话的草稿生命周期。
                // Error 仍保留草稿，允许同一次听写失败后重试。
                let mut inner = self.inner.lock().unwrap();
                if inner
                    .history
                    .as_ref()
                    .is_some_and(|current| Arc::ptr_eq(current, &draft))
                {
                    inner.history = None;
                }
            }
            return;
        }
        let record = {
            let mut g = draft.lock().unwrap();
            if outcome == crate::history::HistoryOutcome::Cancelled {
                if g.reported {
                    // error 已落账过的草稿跳过（Error 后的 Esc 是清理动作，不双记 cancelled）
                    return;
                }
                if g.first_partial_ms.is_none()
                    && !g.vad_speech_seen
                    && g.final_text.trim().is_empty()
                {
                    // 取消时没有任何可验证的语音产出 → 不落账：
                    // solo 结束独奏时刚开出的空段、或没开口就取消，都不是
                    // 「放弃了这句话」，与空转录「无事发生」语义一致（P2 用户反馈
                    // 「结束独奏总有一条未说完成取消」）。草稿一并终结防残留。
                    // 判据说明：VAD 模式以 vad_speech_seen（silero 判定过语音）为主，
                    // STT partial 只作非 VAD 模式兜底——流式引擎对持续环境音/背景
                    // 噪音也会出 partial，单靠 partial 会把沉默段误判为「开过口」；
                    // 非流式引擎（FunASR-Nano/SenseVoice 等）不发 partial 且无 VAD，
                    // 取消时无法确认是否开口，同样按无产出处理不落账。
                    drop(g);
                    self.inner.lock().unwrap().history = None;
                    return;
                }
            } else {
                g.reported = true;
            }
            crate::history::HistoryRecord {
                session_id: g.session_id.clone(),
                ts: crate::eval::utc_now_iso(),
                engine_id: g.engine_id.clone(),
                profile_id: g.profile_id.clone(),
                final_text: g.final_text.clone(),
                source_text: g.source_text.clone(),
                process_duration_ms: g.process_duration_ms,
                audio_ms: g.audio_samples * 1000 / crate::eval::SAMPLE_RATE as u64,
                first_partial_ms: g.first_partial_ms,
                finalize_latency_ms: g.finalize_latency_ms,
                outcome,
                error,
                audio_file: g.audio_file.clone(),
            }
        };
        let dir = self
            .history_dir
            .clone()
            .unwrap_or_else(crate::history::history_dir);
        if let Err(e) = crate::history::append_in(&dir, &record, &cfg) {
            crate::log::log(&format!("history 落账失败（忽略）: {e}"));
        } else {
            // 历史已更新：通知前端（历史记录页挂载期间据此即时刷新，无需切页）
            // 未落账的路径（off / 无草稿 / 无产出取消）不会走到这里。
            self.emitter.emit(
                "kotone://history-updated",
                serde_json::json!({ "outcome": outcome }),
            );
        }
        if outcome != crate::history::HistoryOutcome::Error {
            self.inner.lock().unwrap().history = None;
        }
    }

    /// finalize 成功后把历史音频缓冲写入 history/audio。
    /// 关闭评测录档不影响该路径；取消/空转录不会调用，因此不留下无效音频。
    /// VAD 模式（one-shot/solo）下按语音区间裁剪：段首思考静音与段尾判停
    /// 尾音不再进保存的 wav（用户反馈「独奏历史音频很长、含大量沉默」）；
    /// 非 VAD 模式无语音区间标记，原样保存。
    fn persist_history_audio(&self) {
        let cfg = self.settings.read().unwrap().history.clone();
        if cfg.mode == crate::history::HistoryMode::Off || !cfg.include_audio {
            return;
        }
        let draft = match self.inner.lock().unwrap().history.clone() {
            Some(d) => d,
            None => return,
        };
        let (session_id, pcm, segments) = {
            let mut g = draft.lock().unwrap();
            (
                g.session_id.clone(),
                g.audio_pcm.take(),
                g.speech_segments.clone(),
            )
        };
        let Some(pcm) = pcm else {
            return;
        };
        // 按语音区间裁剪（VAD 模式有区间；非 VAD 模式 segments 空 → 原样）
        let trimmed = trim_speech_pcm(&pcm, &segments);
        if trimmed.is_empty() {
            crate::log::log("history 音频裁剪后为空（忽略保存）");
            return;
        }
        // 实际语音时长写回草稿：记录 audio_ms 与保存的 wav 一致
        draft.lock().unwrap().audio_samples = trimmed.len() as u64;
        let dir = self
            .history_dir
            .clone()
            .unwrap_or_else(crate::history::history_dir);
        let audio_file = crate::history::write_audio_in(&dir, &session_id, &trimmed);
        if audio_file.is_none() {
            crate::log::log("history 音频落盘失败（忽略）");
        }
        draft.lock().unwrap().audio_file = audio_file;
    }

    /// Success 在 toast_dwell 后自动回 Idle（期间有新会话/取消则不动作）。
    /// Error 一律保持到用户关闭/重试，确保核心链路失败不会在用户读到前消失。
    fn schedule_idle(&self, gen: u64) {
        let dwell = self.toast_dwell;
        let inner = self.inner.clone();
        let emitter = self.emitter.clone();
        tokio::spawn(async move {
            tokio::time::sleep(dwell).await;
            let should_idle = {
                let mut g = inner.lock().unwrap();
                let should_idle = g.gen == gen && g.state == OrchestratorState::Success;
                if should_idle {
                    g.state = OrchestratorState::Idle;
                }
                should_idle
            };
            if !should_idle {
                return;
            }
            emitter.emit(
                "kotone://state",
                json!({ "state": OrchestratorState::Idle, "payload": null }),
            );
        });
    }

    fn vad_fallback_warning(&self) -> &'static str {
        if self.inner.lock().unwrap().assistant_voice_owner {
            "自动静音检测暂时不可用；请点击语音按钮结束并提交本次请求"
        } else {
            VAD_FALLBACK_WARNING
        }
    }

    fn emit_state(&self, state: OrchestratorState, mut payload: Option<serde_json::Value>) {
        let (case_id, purpose, button_owned) = {
            let inner = self.inner.lock().unwrap();
            (
                inner.process_case_id.clone(),
                inner.purpose,
                inner.assistant_voice_owner,
            )
        };
        if state != OrchestratorState::Idle {
            let data = payload.get_or_insert_with(|| json!({}));
            if let Some(data) = data.as_object_mut() {
                data.insert(
                    "purpose".into(),
                    json!(match purpose {
                        SessionPurpose::Assistant => "assistant",
                        SessionPurpose::Dictation => "dictation",
                    }),
                );
                if purpose == SessionPurpose::Assistant {
                    data.insert(
                        "inputSource".into(),
                        json!(if button_owned { "button" } else { "hotkey" }),
                    );
                }
            }
        }
        self.emitter.emit(
            "kotone://state",
            json!({ "state": state, "payload": payload, "caseId": case_id }),
        );
    }

    fn emit_process(&self, activity: &str, data: serde_json::Value) {
        let case_id = self.inner.lock().unwrap().process_case_id.clone();
        let Some(case_id) = case_id else {
            return;
        };
        self.emitter.emit(
            "kotone://process",
            json!({ "caseId": case_id, "activity": activity, "data": data }),
        );
    }
}

/// 语音区间前导缓冲：silero 判定到 speech 时语音已实际开始（判定延迟约
/// 1-数帧），段首保留判定点之前的一段 PCM，避免字头/辅音被裁掉
/// （用户反馈「录制语音听不到第一个字」）。150ms @16kHz = 2400 采样。
const PRE_ROLL_SAMPLES: usize = 2400;
/// 语音区间尾随缓冲：保留尾音/气息，避免句尾被切。
const POST_ROLL_SAMPLES: usize = 2400;

/// 按语音区间裁剪历史音频 PCM（纯函数，可单测）。
/// 区间为半开 [start, end)，采样号相对 pcm 起点；每个区间向前/向后各扩展
/// PRE/POST_ROLL_SAMPLES 缓冲后拼接，扩展后重叠的段合并（段间停顿
/// < 300ms 自然连成一段），整体 clamp 到 pcm 长度。
/// segments 空 = 非 VAD 模式（无裁剪标记）→ 返回原样。
fn trim_speech_pcm(pcm: &[f32], segments: &[(usize, usize)]) -> Vec<f32> {
    if segments.is_empty() {
        return pcm.to_vec();
    }
    let n = pcm.len();
    let mut expanded: Vec<(usize, usize)> = Vec::with_capacity(segments.len());
    // 合并阈值 = PRE_ROLL + POST_ROLL = 4800 采样（300ms），与
    // SPEECH_GAP_SAMPLES（push 阶段的句间合并）数值恰好相等，但语义独立：
    // 这里是「缓冲扩展后重叠即合并」的防御性冗余，改其一无需同步另一处。
    for &(s, e) in segments {
        let ns = s.saturating_sub(PRE_ROLL_SAMPLES);
        let ne = (e + POST_ROLL_SAMPLES).min(n);
        if let Some(last) = expanded.last_mut() {
            if ns <= last.1 {
                // 与前段重叠/相接（缓冲覆盖了段间停顿）：合并
                last.1 = last.1.max(ne);
                continue;
            }
        }
        expanded.push((ns, ne));
    }
    let cap = expanded.iter().map(|&(s, e)| e.saturating_sub(s)).sum();
    let mut out = Vec::with_capacity(cap);
    for &(s, e) in &expanded {
        if s < e {
            out.extend_from_slice(&pcm[s..e]);
        }
    }
    out
}

/// ADR-008 发送频道解析（纯函数，可单测）：频道选择与当前 profile 不符
/// （用户切了游戏适配，旧频道残留）时回落默认频道；否则按选择解析策略。
fn resolve_send_strategy(
    profile: &GameProfile,
    channel: &Option<(String, String)>,
) -> crate::profile::ChannelStrategy {
    match channel {
        Some((pid, cid)) if *pid == profile.id => profile.channel_strategy(Some(cid)),
        _ => profile.channel_strategy(None),
    }
}

/// 会话配置组装（纯函数，可单测）：profile 热词 → SessionConfig.hotwords，
/// 引擎专有配置取 engineOptions[engine_id]（缺省 Null）
fn build_session_config(
    settings: &Settings,
    engine_id: &str,
    active_profile: &GameProfile,
) -> SessionConfig {
    SessionConfig {
        language: settings.language.clone(),
        hotwords: active_profile.hotwords.clone(),
        hotwords_score: settings.hotwords_score,
        options: settings.session_options(engine_id),
    }
}

fn enrich_remote_connection(
    settings: &Settings,
    cfg: &mut SessionConfig,
    secrets: Option<&dyn crate::connection::SecretStore>,
) {
    let Some(connection_id) = cfg
        .options
        .get("connectionId")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
    else {
        return;
    };
    let Some(connection) = settings
        .connections
        .iter()
        .find(|item| item.id == connection_id)
    else {
        return;
    };
    if let Some(opts) = cfg.options.as_object_mut() {
        opts.insert("baseUrl".into(), serde_json::json!(connection.base_url));
        opts.insert("remoteModel".into(), serde_json::json!(connection.model));
        if let Some(store) = secrets {
            if let Ok(Some(key)) = store.get(&connection_id) {
                opts.insert("apiKey".into(), serde_json::json!(key));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NoopPorts;

    impl AudioBackend for NoopPorts {
        fn start(&self, _: &str) -> Result<crate::audio::AudioHandle, String> {
            Err("audio must not open in an admission test".into())
        }
    }

    impl FocusBackend for NoopPorts {
        fn foreground_window(&self) -> Option<TargetWindow> {
            None
        }
        fn restore(&self, _: TargetWindow) -> bool {
            false
        }
    }

    impl Emitter for NoopPorts {
        fn emit(&self, _: &str, _: serde_json::Value) {}
    }

    fn admission_test_orchestrator() -> Arc<Orchestrator> {
        let mut orch = Orchestrator::new(
            Arc::new(RwLock::new(Settings::default())),
            Arc::new(EngineRegistry::new()),
            Arc::new(NoopPorts),
            Arc::new(crate::inject::StubInjector),
            Arc::new(NoopPorts),
            Arc::new(NoopPorts),
        );
        orch.release_grace = Duration::from_millis(1);
        orch.into_arc()
    }

    #[tokio::test]
    async fn automatic_end_rechecks_generation_after_waiting_for_state_lock() {
        let orch = admission_test_orchestrator();
        {
            let mut inner = orch.inner.lock().unwrap();
            inner.gen = 4;
            inner.state = OrchestratorState::Listening;
        }
        let guard = orch.op.lock().await;
        let old = orch.clone();
        let ending = tokio::spawn(async move { old.end_inner(Some(4), None).await });
        tokio::task::yield_now().await;
        assert!(!ending.is_finished());
        // Model cancel + a new capture winning the lock before an old VAD/release callback.
        {
            let mut inner = orch.inner.lock().unwrap();
            inner.gen = 6;
            inner.purpose = SessionPurpose::Assistant;
        }
        drop(guard);
        ending.await.unwrap().unwrap();
        assert_eq!(orch.state(), OrchestratorState::Listening);
        assert_eq!(orch.inner.lock().unwrap().gen, 6);
    }

    #[tokio::test]
    async fn expired_release_timer_remains_abortable_while_waiting_for_state_lock() {
        let orch = admission_test_orchestrator();
        orch.inner.lock().unwrap().state = OrchestratorState::Listening;
        let guard = orch.op.lock().await;
        orch.schedule_release_end(0, orch.hotkey_input_ticket());
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(orch.inner.lock().unwrap().release_grace_task.is_some());
        orch.abort_release_grace();
        drop(guard);
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert_eq!(orch.state(), OrchestratorState::Listening);
    }

    #[tokio::test]
    async fn queued_old_hotkey_actions_cannot_end_cancel_or_confirm_a_new_session() {
        for action in 0..3 {
            let orch = admission_test_orchestrator();
            {
                let mut inner = orch.inner.lock().unwrap();
                inner.gen = 4;
                inner.state = OrchestratorState::Preview;
            }
            let ticket = orch.hotkey_input_ticket();
            let guard = orch.op.lock().await;
            let old = orch.clone();
            let task = tokio::spawn(async move {
                match action {
                    0 => old.cancel_inner(Some(ticket), Some(4)).await,
                    1 => {
                        old.end_inner(Some(4), Some(ticket)).await.unwrap();
                    }
                    _ => {
                        old.confirm_send_inner(Some(ticket), Some(4)).await.unwrap();
                    }
                }
            });
            tokio::task::yield_now().await;
            assert!(!task.is_finished());
            orch.invalidate_hotkey_inputs();
            {
                let mut inner = orch.inner.lock().unwrap();
                inner.gen = 6;
                inner.state = OrchestratorState::Listening;
                inner.purpose = SessionPurpose::Assistant;
            }
            drop(guard);
            task.await.unwrap();
            assert_eq!(orch.state(), OrchestratorState::Listening);
            assert_eq!(orch.inner.lock().unwrap().gen, 6);
        }
    }

    #[test]
    fn input_configuration_guard_rejects_all_active_phases_and_unfinished_send() {
        let orch = admission_test_orchestrator();
        for state in [
            OrchestratorState::Listening,
            OrchestratorState::Transcribing,
            OrchestratorState::Processing,
            OrchestratorState::Preview,
            OrchestratorState::Sending,
        ] {
            orch.inner.lock().unwrap().state = state;
            assert!(orch.try_input_configuration_guard().is_err());
        }
        for state in [
            OrchestratorState::Idle,
            OrchestratorState::Success,
            OrchestratorState::Error,
        ] {
            orch.inner.lock().unwrap().state = state;
            let guard = orch.try_input_configuration_guard().unwrap();
            assert!(orch.op.try_lock().is_err());
            drop(guard);
        }
        let (operation, _completion) = SendOperation::new();
        orch.inner.lock().unwrap().send_operation = Some(operation);
        assert!(orch.try_input_configuration_guard().is_err());
    }

    /// 热词端到端链路的 core 段：profile.hotwords 原样进 SessionConfig
    /// （stt 段见 online_transducer 的 format_hotwords 测试）
    #[test]
    fn session_config_carries_profile_hotwords() {
        let settings = Settings::default();
        let profile = GameProfile::builtin_lol();
        let cfg = build_session_config(&settings, "sherpa-onnx-x-asr-zh-en", &profile);
        assert_eq!(cfg.hotwords, profile.hotwords);
        assert!(cfg.hotwords.contains(&"打野".to_string()));
        assert_eq!(cfg.language, "zh");
        assert_eq!(
            cfg.hotwords_score, settings.hotwords_score,
            "热词权重原样携带"
        );
    }

    #[test]
    fn session_config_carries_custom_hotwords_score() {
        let settings = Settings {
            hotwords_score: 1.5,
            ..Settings::default()
        };
        let generic = GameProfile::builtin_generic();
        let cfg = build_session_config(&settings, "sherpa-onnx-x-asr-zh-en", &generic);
        assert_eq!(cfg.hotwords_score, 1.5);
    }

    #[test]
    fn session_config_merges_model_schema_values() {
        let mut settings = Settings {
            active_model_id: "sense-voice-zh-en-ja-ko-yue-2024-07-17".into(),
            ..Settings::default()
        };
        settings.model_configs.insert(
            "sense-voice-zh-en-ja-ko-yue-2024-07-17".into(),
            serde_json::json!({ "language": "yue" }),
        );
        let cfg = build_session_config(
            &settings,
            "sherpa-onnx-sensevoice",
            &GameProfile::builtin_generic(),
        );
        assert_eq!(cfg.options["language"], "yue");
    }

    #[test]
    fn session_config_options_fallback_null() {
        let mut settings = Settings::default();
        settings.engine_options["sherpa-onnx-x-asr-zh-en"]["provider"] = serde_json::json!("cpu");
        let generic = GameProfile::builtin_generic();
        let cfg = build_session_config(&settings, "sherpa-onnx-x-asr-zh-en", &generic);
        assert_eq!(cfg.options["provider"], "cpu");
        assert!(cfg.hotwords.is_empty(), "generic 无热词");
        // 未配置的引擎 → 空对象（可再合并模型 configSchema）
        let cfg2 = build_session_config(&settings, "mock-stream", &generic);
        assert!(cfg2.options.as_object().is_some_and(|o| o.is_empty()));
    }

    // ---------- 发送频道解析（ADR-008） ----------

    #[test]
    fn resolve_send_strategy_uses_selected_channel() {
        let lol = GameProfile::builtin_lol();
        let sel = Some(("lol".to_string(), "all".to_string()));
        let s = resolve_send_strategy(&lol, &sel);
        assert_eq!(s.id, "all");
        assert_eq!(s.open_chat_key, "Shift+Enter");
    }

    #[test]
    fn resolve_send_strategy_falls_back_when_profile_switched() {
        // 频道残留属于 lol，但当前 profile 已是 generic → 默认频道
        let generic = GameProfile::builtin_generic();
        let sel = Some(("lol".to_string(), "all".to_string()));
        let s = resolve_send_strategy(&generic, &sel);
        assert!(s.is_default);
        assert_eq!(s.open_chat_key, generic.open_chat_key);
        // 未选择（None）同样回落默认
        let s2 = resolve_send_strategy(&lol_clone(), &None);
        assert_eq!(s2.id, "team");
    }

    fn lol_clone() -> GameProfile {
        GameProfile::builtin_lol()
    }

    // ---------- 历史音频 VAD 裁剪（trim_speech_pcm / push_speech_range） ----------

    #[test]
    fn trim_speech_pcm_no_segments_returns_original() {
        let pcm = vec![0.5f32; 1000];
        let out = trim_speech_pcm(&pcm, &[]);
        assert_eq!(out, pcm, "非 VAD 模式（无区间）应原样保存");
    }

    #[test]
    fn trim_speech_pcm_single_segment_with_prepost_roll() {
        let pcm: Vec<f32> = (0..10000).map(|i| i as f32).collect();
        // 段 [1000,2000)：前导缓冲取到 0（PRE 2400 超出开头），尾随缓冲到 4400
        let out = trim_speech_pcm(&pcm, &[(1000, 2000)]);
        assert_eq!(out, pcm[0..4400]);
        // 段 [9000,9500)：尾随缓冲 clamp 到 pcm 末尾；前导取到 6600
        let out2 = trim_speech_pcm(&pcm, &[(9000, 9500)]);
        assert_eq!(out2, pcm[6600..10000]);
        // 段 [500,1500)：前后都 clamp 到界内
        let out3 = trim_speech_pcm(&pcm, &[(500, 1500)]);
        assert_eq!(out3, pcm[0..3900]);
    }

    #[test]
    fn trim_speech_pcm_merges_close_segments_after_roll() {
        let pcm: Vec<f32> = (0..10000).map(|i| i as f32).collect();
        // 两段间隙 2000 采样（125ms）< 缓冲覆盖（PRE+POST=4800）→ 合并成一段
        let out = trim_speech_pcm(&pcm, &[(1000, 2000), (4000, 5000)]);
        assert_eq!(out, pcm[0..7400]);
        // 长停顿（间隙 6000 采样 = 375ms > 4800）→ 断开，中间静音裁掉
        let out2 = trim_speech_pcm(&pcm, &[(1000, 2000), (8000, 9000)]);
        assert_eq!(out2.len(), 4400 + 4400);
        assert_eq!(out2[..4400], pcm[0..4400]);
        assert_eq!(out2[4400..], pcm[5600..10000]);
    }

    #[test]
    fn trim_speech_pcm_clamps_out_of_bounds() {
        let pcm: Vec<f32> = (0..50).map(|i| i as f32).collect();
        // 短 PCM + 越界区间：全部扩展后重叠合并、clamp 到 pcm 边界，不 panic
        let out = trim_speech_pcm(&pcm, &[(40, 100), (200, 300)]);
        assert_eq!(out, pcm, "区间越界时应整体 clamp 到 pcm 长度");
    }

    #[test]
    fn push_speech_range_merges_close_segments() {
        let mut g = HistoryDraft::speech_harness();
        // 帧 0-10 语音，停顿 100ms（1600 采样 < 300ms 合并阈值）后又语音
        g.push_speech_range(0, 4800);
        g.push_speech_range(6400, 9600); // 间隙 1600 采样 = 100ms → 合并
        assert_eq!(g.speech_segments, vec![(0, 9600)]);
        // 长停顿（> 300ms）后断开
        g.push_speech_range(19200, 24000); // 距 9600 差 9600 采样 = 600ms → 新段
        assert_eq!(g.speech_segments, vec![(0, 9600), (19200, 24000)]);
        // 无效区间忽略
        g.push_speech_range(50000, 50000);
        assert_eq!(g.speech_segments.len(), 2);
    }

    #[test]
    fn vad_speech_seen_defaults_false() {
        let g = HistoryDraft::speech_harness();
        assert!(!g.vad_speech_seen);
        assert!(g.speech_segments.is_empty());
    }
}
