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

use crate::assistant::{AssistantHandler, AssistantRequest, SessionPurpose};
use crate::audio::{AudioBackend, AudioHandle};
use crate::automation::AutomationScheduler;
use crate::inject::{CancelToken, FocusBackend, Injector, TargetWindow};
use crate::input_lock::InputLock;
use crate::interaction::{EndTrigger, InteractionPolicy, PostFinalize};
use crate::postprocess::ProcessingCancelToken;
use crate::profile::{self, GameProfile};
use crate::send_to_game::{GameChat, SendToGame};
use crate::settings::Settings;
use crate::stt::{EngineRegistry, SessionConfig, SttEvent, SttSession};

mod after_hearing;
mod assistant_input;
mod capture;
mod finalize;
mod hotkeys;
mod lifecycle;

pub use self::after_hearing::{
    AfterHearing, BoxFuture, PendingText, ReadyText, RecognizedText, RecordDraft, SendCompletion,
    SendOperation, SessionGuard, SEND_STILL_RUNNING,
};
use self::capture::ActiveSession;
pub use self::capture::CaptureObserver;

#[cfg(test)]
use self::capture::build_session_config;

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

/// Rust → UI 事件出口（生产实现发 Tauri 事件，测试用 Vec 收集）
pub trait Emitter: Send + Sync {
    fn emit(&self, event: &str, payload: serde_json::Value);
}

struct Inner {
    state: OrchestratorState,
    purpose: SessionPurpose,
    /// 开始时固定采集/收尾规则，设置热更新从下一会话生效。
    session_policy: Option<InteractionPolicy>,
    gen: u64,
    active: Option<ActiveSession>,
    trial: Option<capture::trial::VoiceTrial>,
    pending_text: Option<PendingText>,
    /// 语音输入正在准备或执行的发送。取消/超时后仍保留，直到阻塞调用真正退出；
    /// 开始新会话、改热键前都要看它（`send_is_running`）。
    send_operation: Option<SendOperation>,
    /// Processing 状态的取消令牌；Esc/cancel 可立即打断异步处理器 future。
    processing_cancel: Option<ProcessingCancelToken>,
    /// 本地诊断流程的 case id；无论 history 是否关闭，每次 begin 都生成。
    process_case_id: Option<String>,
    /// 松手宽限期的延迟结束任务（见 DEFAULT_RELEASE_GRACE）；
    /// 再次按下 / cancel / 状态变迁时 abort
    release_grace_task: Option<tokio::task::JoinHandle<()>>,
    /// 本轮会话是否按 solo 连续策略运行。VAD 初始化/推理失败时会降级为 false，
    /// 让第二次热键结束并发送当前句，而不是丢弃在途音频或自动续段。
    continuous_session: bool,
}

pub struct Orchestrator {
    /// Arc：schedule_idle 的延迟任务需要共享访问
    inner: Arc<Mutex<Inner>>,
    op: tokio::sync::Mutex<()>,
    /// 按住输入的 down/up 串行化；down 等待开麦时不能丢掉已到达的 up。
    hold_input: tokio::sync::Mutex<()>,
    /// 自动（ADR 054）：说话键 / 呼叫键还按着。按着的时候静音判停不算数（收音循环看它），松开才看按了多久。
    key_held: Arc<AtomicBool>,
    /// 自动 / 按住说话：这一下按下开始了哪一次会话（代际）、什么时候按下的。松开时对上了才算这一次的松开。
    key_press: Mutex<Option<(u64, std::time::Instant)>>,
    /// 这一轮语音输入是照键上的方式开始的（ADR 054）：一直听逐句续录时照它，不回到 `interactionMode`。
    resume_preset: Mutex<Option<crate::interaction::InteractionMode>>,
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
    /// 语音输入（“听到以后”，插件实现；没接 = 听写听到的字没有去处）
    after_hearing: Option<Arc<dyn AfterHearing>>,
    assistant: Option<(Arc<dyn AssistantHandler>, Arc<AutomationScheduler>)>,
    audio: Arc<dyn AudioBackend>,
    before_capture: Option<Arc<dyn Fn() -> Result<(), String> + Send + Sync>>,
    after_capture: Option<Arc<dyn Fn() + Send + Sync>>,
    /// 发到游戏的入口（ADR 041）：语音输入打字只经它。缺省用构造时的注入器和焦点后端自己建一份
    /// （带自己的一把输入锁，命令行 / 测试用）；桌面端用 [`Self::with_send_to_game`] 换成和
    /// `game_chat.send`、`game.act`、按键程序共用一把锁的那一份。
    send_to_game: Arc<dyn SendToGame>,
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
        let send_to_game: Arc<dyn SendToGame> = Arc::new(GameChat::new(
            injector,
            focus.clone(),
            InputLock::new(),
            settings.clone(),
        ));
        Self {
            inner: Arc::new(Mutex::new(Inner {
                state: OrchestratorState::Idle,
                purpose: SessionPurpose::Dictation,
                session_policy: None,
                gen: 0,
                active: None,
                trial: None,
                pending_text: None,
                send_operation: None,
                processing_cancel: None,
                process_case_id: None,
                release_grace_task: None,
                continuous_session: false,
            })),
            op: tokio::sync::Mutex::new(()),
            hold_input: tokio::sync::Mutex::new(()),
            key_held: Arc::new(AtomicBool::new(false)),
            key_press: Mutex::new(None),
            resume_preset: Mutex::new(None),
            hotkey_epoch: AtomicU64::new(0),
            settings,
            settings_repository: None,
            runtime_stt_settings: RwLock::new(None),
            runtime_inputs_enabled: AtomicBool::new(true),
            engines,
            after_hearing: None,
            assistant: None,
            audio,
            before_capture: None,
            after_capture: None,
            send_to_game,
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

    /// 换成宿主那一份发到游戏的入口（和别的发话人、`game.act`、按键程序共用一把输入锁）。
    pub fn with_send_to_game(mut self, send_to_game: Arc<dyn SendToGame>) -> Self {
        self.send_to_game = send_to_game;
        self
    }

    pub fn with_secrets(mut self, store: Arc<dyn crate::connection::SecretStore>) -> Self {
        self.secrets = Some(store);
        self
    }

    pub fn with_assistant(
        mut self,
        handler: Arc<dyn AssistantHandler>,
        reminders: Arc<AutomationScheduler>,
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

    /// 包装为 Arc 并接线 Weak 自引用（VAD 判停 → end() 的回调通道），再告诉接上的语音输入（[`AfterHearing::attached`]）。
    pub fn into_arc(self) -> Arc<Self> {
        let orch = Arc::new(self);
        *orch.me.write().unwrap() = Some(Arc::downgrade(&orch));
        if let Some(after) = orch.after_hearing.clone() {
            after.attached(&orch);
        }
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

    /// 热键排队的票据还有效吗（Esc / 重注册会让排着队的旧票据作废）；`None` = 不是热键来的。
    pub fn input_ticket_is_current(&self, ticket: Option<u64>) -> bool {
        ticket.map_or(true, |ticket| ticket == self.hotkey_input_ticket())
    }

    /// 桌面壳停止运行时后解除快照并保持入口关闭；CLI/测试默认一直开放。
    pub fn deactivate_runtime_settings(&self) {
        self.suspend_runtime_inputs();
        *self.runtime_stt_settings.write().unwrap() = None;
    }

    // ---------- 内部 ----------

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
        inner.pending_text = text.clone().map(PendingText::retryable);
        inner.processing_cancel = None;
        inner.state = OrchestratorState::Error;
        drop(inner);
        self.emit_state(
            OrchestratorState::Error,
            Some(json!({ "message": message, "text": text })),
        );
        self.on_session_ended(
            crate::history::HistoryOutcome::Error,
            Some(message.to_string()),
        );
        self.schedule_idle(gen);
    }

    /// Success 在 toast_dwell 后自动回 Idle（期间有新会话/取消则不动作）。
    /// Error 一律保持到用户关闭/重试，确保核心链路失败不会在用户读到前消失。
    pub fn schedule_idle(&self, gen: u64) {
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

    pub fn emit_state(&self, state: OrchestratorState, mut payload: Option<serde_json::Value>) {
        let (case_id, purpose) = {
            let inner = self.inner.lock().unwrap();
            (inner.process_case_id.clone(), inner.purpose)
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
            }
        }
        self.emitter.emit(
            "kotone://state",
            json!({ "state": state, "payload": payload, "caseId": case_id }),
        );
    }

    /// 本地诊断的流程事件（`kotone://process`，带本轮的 case id）。
    pub fn emit_process(&self, activity: &str, data: serde_json::Value) {
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

#[cfg(test)]
mod tests;
