//! “听一句”和“听到以后”之间的缝（ADR 038 D3 第 7 条）。
//!
//! 本体（这个目录里的其余文件）负责“听一句”：热键、开始 / 结束 / 取消、录音、语音识别、她的耳朵。
//! 听写（语音输入）听到一句以后的事——文字处理、预览、往游戏里打字、识别记录、聊天频道——在语音输入插件里
//! （`plugins/voice-input`，实现 [`AfterHearing`]），由宿主用 [`Orchestrator::with_after_hearing`] 接上。
//! 没接（插件关着、命令行没接）时听写听到的字没有去处：会话以 Error 结束并说明原因。
//!
//! 本体在“听一句”的几个时刻调插件（括号里是本体哪个文件调）：
//!
//! - 开始收音（capture）：[`AfterHearing::start_record`] 建识别记录草稿（同时是录音旁听者）；发布会话时在
//!   inner 锁里 [`AfterHearing::on_capture_started`] 记下注入目标窗口、收下草稿。
//! - 听到了一句（finalize）：[`AfterHearing::on_transcribed`]，本体落完评测录档后 [`AfterHearing::on_heard`]。
//!   空识别：[`AfterHearing::heard_audio_ms`] 给诊断，op + inner 锁里 [`AfterHearing::discard_record`]。
//! - 确认（lifecycle）：[`AfterHearing::confirm_heard`]。
//! - 会话结束（lifecycle、mod）：锁里停下正在跑的发送（发送权 [`SendOperation`] 留在本体，开始新会话、
//!   改热键都要看它），锁外 [`AfterHearing::on_session_ended`]。
//!
//! 插件改会话状态只经 [`Orchestrator::lock_op`]（begin / end / cancel / confirm 共用的临界区）和
//! [`Orchestrator::session`]（状态那把锁）。**锁的顺序**：op → 状态锁 → 插件自己的锁。本体会在状态锁里调
//! `on_capture_started`、`discard_record`，所以插件拿着自己的锁时不能再调本体的任何方法。

use std::any::Any;
use std::future::Future;
use std::pin::Pin;

use super::*;
use crate::postprocess::{PostProcessingConfig, ProcessingContext};

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// 听写听到以后的事（语音输入插件实现）。
pub trait AfterHearing: Send + Sync + 'static {
    /// 宿主拿回具体类型用（热键、重发这类只有插件自己懂的入口）。
    fn as_any(&self) -> &dyn Any;

    /// 开始收音（还没发布会话，不持锁）：识别记录开着且是听写时给一份草稿，草稿同时旁听录音。
    fn start_record(
        &self,
        purpose: SessionPurpose,
        settings: &Settings,
        engine_id: &str,
        process_case_id: &str,
    ) -> RecordDraft;

    /// 发布会话（调用方持有状态锁）：记下注入目标窗口，收下 `start_record` 给的草稿。
    fn on_capture_started(&self, target: Option<TargetWindow>, record: RecordDraft);

    /// 空识别时给诊断用的录音时长（有识别记录草稿时才有）。
    fn heard_audio_ms(&self) -> Option<u64>;

    /// 空识别（调用方持有 op 与状态锁）：草稿清掉，不写记录。
    fn discard_record(&self);

    /// 识别完成（不持锁）：记识别耗时、存历史音频。
    fn on_transcribed(&self, orch: &Orchestrator, latency_ms: u64);

    /// 听到了一句（不持锁）：文字处理，然后预览或发送。
    fn on_heard<'a>(
        &'a self,
        orch: &'a Orchestrator,
        gen: u64,
        text: String,
    ) -> BoxFuture<'a, Result<(), String>>;

    /// 确认（不持锁；`gen` 是确认那一刻的代际）：预览后发送、出错后重试处理或发送。
    fn confirm_heard<'a>(
        &'a self,
        orch: &'a Orchestrator,
        gen: u64,
        input_ticket: Option<u64>,
    ) -> BoxFuture<'a, Result<(), String>>;

    /// 会话结束（出错 / 取消 / 中止；不持锁）：记一条识别记录。
    fn on_session_ended(
        &self,
        orch: &Orchestrator,
        outcome: crate::history::HistoryOutcome,
        error: Option<String>,
    );
}

/// 开始收音时插件给的识别记录草稿：旁听录音的那一面给本体，具体类型只有插件自己认。
#[derive(Default)]
pub struct RecordDraft {
    observer: Option<Arc<dyn CaptureObserver>>,
    draft: Option<Arc<dyn Any + Send + Sync>>,
}

impl RecordDraft {
    pub fn new<T: CaptureObserver + Any + Send + Sync>(draft: Arc<T>) -> Self {
        Self {
            observer: Some(draft.clone() as Arc<dyn CaptureObserver>),
            draft: Some(draft as Arc<dyn Any + Send + Sync>),
        }
    }

    pub fn none() -> Self {
        Self::default()
    }

    pub(super) fn observer(&self) -> Option<Arc<dyn CaptureObserver>> {
        self.observer.clone()
    }

    /// 插件拿回自己的草稿。
    pub fn into_draft<T: Any + Send + Sync>(self) -> Option<Arc<T>> {
        self.draft.and_then(|draft| draft.downcast::<T>().ok())
    }
}

// ---------- 待处置的文字 ----------

/// 已完成全部文本生产步骤、可安全进入预览或注入的文本。
/// `source_text` 保留识别原文，`text` 承载后处理完成的可交付文本。
#[derive(Clone)]
pub struct ReadyText {
    pub source_text: String,
    pub text: String,
}

impl ReadyText {
    pub fn unchanged(text: String) -> Self {
        Self {
            source_text: text.clone(),
            text,
        }
    }
}

/// STT 已完成、但尚未完成后处理的文本与本轮不可变 pipeline/context 快照。
#[derive(Clone)]
pub struct RecognizedText {
    pub source_text: String,
    pub pipeline: PostProcessingConfig,
    pub context: ProcessingContext,
}

/// 状态机中待用户/系统继续处置的类型化文本，替代语义过载的 `preview_text`。
/// Recognized 只能重试处理，预览与注入只接受 Ready。
#[derive(Clone)]
pub enum PendingText {
    Recognized(RecognizedText),
    Ready(ReadyText),
    /// 助手预览保留识别完成时间，只能提交 AI，不能转换为可注入文本。
    Assistant(AssistantRequest),
}

impl PendingText {
    /// 出错但保留文字：Error 态可重试发送（不再经过文字处理）。
    pub fn retryable(text: String) -> Self {
        Self::Ready(ReadyText::unchanged(text))
    }

    pub fn ready(&self) -> Option<&ReadyText> {
        match self {
            Self::Recognized(_) | Self::Assistant(_) => None,
            Self::Ready(ready) => Some(ready),
        }
    }

    pub fn recognized(&self) -> Option<&RecognizedText> {
        match self {
            Self::Recognized(recognized) => Some(recognized),
            Self::Ready(_) | Self::Assistant(_) => None,
        }
    }
}

// ---------- 发送权 ----------

/// 一次发送从准备到最后一个阻塞调用返回始终占用同一份操作权。
/// 取消/超时只发出停止请求，不能伪装成 Win32 调用已经结束。
/// 发送是插件做的，发送权留在本体：发送没结束时不能开始新会话、不能改热键（[`SEND_STILL_RUNNING`]）。
#[derive(Clone)]
pub struct SendOperation(Arc<SendOperationInner>);

struct SendOperationInner {
    cancel: CancelToken,
    finished: AtomicBool,
}

impl SendOperation {
    pub fn new() -> (Self, SendCompletion) {
        let operation = Self(Arc::new(SendOperationInner {
            cancel: CancelToken::default(),
            finished: AtomicBool::new(false),
        }));
        let completion = SendCompletion {
            operation: operation.clone(),
        };
        (operation, completion)
    }

    pub fn cancel(&self) {
        self.0.cancel.cancel();
    }

    /// 这次发送的取消令牌（交给注入器）。
    pub fn token(&self) -> CancelToken {
        self.0.cancel.clone()
    }

    pub fn is_finished(&self) -> bool {
        self.0.finished.load(Ordering::SeqCst)
    }

    pub fn same_operation(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// 唯一完成凭据：准备阶段由 async 调用持有，进入阻塞阶段后移交给该线程。
/// 即使调用方取消或等待超时，也只有阻塞线程真正退出后才释放发送权。
pub struct SendCompletion {
    operation: SendOperation,
}

impl Drop for SendCompletion {
    fn drop(&mut self) {
        self.operation.cancel();
        self.operation.0.finished.store(true, Ordering::SeqCst);
    }
}

pub const SEND_STILL_RUNNING: &str =
    "上一次发送仍在结束中，暂时不能开始新的发送；请稍后重试。若持续无响应，请退出并重新打开应用";

/// 听写听到了字、却没有接语音输入时的说明。
const NO_AFTER_HEARING: &str = "语音输入插件关着，听到的话没有地方发；请在“插件”里打开语音输入";

impl Inner {
    /// 发送在不在跑；已经跑完的顺手清掉。
    pub(super) fn send_is_running(&mut self) -> bool {
        if self
            .send_operation
            .as_ref()
            .is_some_and(SendOperation::is_finished)
        {
            self.send_operation = None;
        }
        self.send_operation.is_some()
    }

    /// 取消 / 中止（调用方持有 inner 锁）：请求停下正在跑的发送，不等阻塞调用退出。
    pub(super) fn stop_send(&self) {
        if let Some(operation) = &self.send_operation {
            operation.cancel();
        }
    }
}

// ---------- 插件改状态用的句柄 ----------

/// 状态锁（`inner`）的句柄：插件看、改会话状态都经它。**拿着它时不能 await**，
/// 发状态事件（[`Orchestrator::emit_state`]）之前先放掉。
pub struct SessionGuard<'a>(std::sync::MutexGuard<'a, Inner>);

impl SessionGuard<'_> {
    /// 会话代际：每次开始 / 结束 / 取消都加一，异步空隙之后拿它核对是不是还是同一轮。
    pub fn gen(&self) -> u64 {
        self.0.gen
    }

    pub fn state(&self) -> OrchestratorState {
        self.0.state
    }

    pub fn set_state(&mut self, state: OrchestratorState) {
        self.0.state = state;
    }

    pub fn purpose(&self) -> SessionPurpose {
        self.0.purpose
    }

    /// 空闲时的显式听写操作（重发最近一条）：不沿用上一次助手会话的用途和策略。
    pub fn reset_to_dictation(&mut self) {
        self.0.purpose = SessionPurpose::Dictation;
        self.0.session_policy = None;
    }

    /// 本轮会话是否按 solo 连续策略运行（VAD 降级时为 false）。
    pub fn continuous_session(&self) -> bool {
        self.0.continuous_session
    }

    /// 本地诊断流程的 case id（和识别记录、评测录档共用）。
    pub fn process_case_id(&self) -> Option<&str> {
        self.0.process_case_id.as_deref()
    }

    pub fn pending_text(&self) -> Option<&PendingText> {
        self.0.pending_text.as_ref()
    }

    pub fn set_pending_text(&mut self, text: Option<PendingText>) {
        self.0.pending_text = text;
    }

    /// Processing 状态的取消令牌：Esc / 取消时本体会触发它。
    pub fn set_processing_cancel(&mut self, cancel: Option<ProcessingCancelToken>) {
        self.0.processing_cancel = cancel;
    }

    /// 发送在不在跑（已经跑完的顺手清掉）。
    pub fn send_is_running(&mut self) -> bool {
        self.0.send_is_running()
    }

    /// 占用发送权（调用方已经核对过 [`Self::send_is_running`]）。
    pub fn start_send(&mut self, operation: SendOperation) {
        self.0.send_operation = Some(operation);
    }

    /// 发送的阻塞调用结束了：还是这一次、并且真的结束了才放掉发送权。
    pub fn finish_send(&mut self, operation: &SendOperation) {
        if self
            .0
            .send_operation
            .as_ref()
            .is_some_and(|current| current.same_operation(operation) && current.is_finished())
        {
            self.0.send_operation = None;
        }
    }
}

impl Orchestrator {
    /// 接上语音输入（“听到以后”）。宿主在 `into_arc` 之前调。
    pub fn with_after_hearing(mut self, after_hearing: Arc<dyn AfterHearing>) -> Self {
        self.after_hearing = Some(after_hearing);
        self
    }

    /// 接上的语音输入（宿主拿回具体类型用）。
    pub fn after_hearing(&self) -> Option<&Arc<dyn AfterHearing>> {
        self.after_hearing.as_ref()
    }

    /// begin / end / cancel / confirm 串行化的临界区（op 锁）。拿着它再 [`Self::session`]，
    /// 等于这些入口自己的“op + 状态锁”。
    pub async fn lock_op(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.op.lock().await
    }

    /// 状态锁的句柄。
    pub fn session(&self) -> SessionGuard<'_> {
        SessionGuard(self.inner.lock().unwrap())
    }

    /// 运行时开着（桌面停止的第一步就把它关掉，排着队的开始 / 发送不能再入场）。
    pub fn runtime_inputs_enabled(&self) -> bool {
        self.runtime_inputs_enabled.load(Ordering::SeqCst)
    }

    pub fn emitter(&self) -> &Arc<dyn Emitter> {
        &self.emitter
    }

    pub fn injector(&self) -> &Arc<dyn Injector> {
        &self.injector
    }

    pub fn focus(&self) -> &Arc<dyn FocusBackend> {
        &self.focus
    }

    /// 配置仓库（桌面壳注入；命令行 / 测试没有）。
    pub fn settings_repository(&self) -> Option<&Arc<crate::settings::SettingsRepository>> {
        self.settings_repository.as_ref()
    }

    // ---------- 本体在“听一句”的几个时刻调语音输入（没接时的做法也在这里） ----------

    pub(super) fn start_record(
        &self,
        purpose: SessionPurpose,
        settings: &Settings,
        engine_id: &str,
        process_case_id: &str,
    ) -> RecordDraft {
        match &self.after_hearing {
            Some(after) => after.start_record(purpose, settings, engine_id, process_case_id),
            None => RecordDraft::none(),
        }
    }

    /// 调用方持有状态锁。
    pub(super) fn on_capture_started(&self, target: Option<TargetWindow>, record: RecordDraft) {
        if let Some(after) = &self.after_hearing {
            after.on_capture_started(target, record);
        }
    }

    pub(super) fn heard_audio_ms(&self) -> Option<u64> {
        self.after_hearing
            .as_ref()
            .and_then(|after| after.heard_audio_ms())
    }

    /// 调用方持有 op 与状态锁。
    pub(super) fn discard_record(&self) {
        if let Some(after) = &self.after_hearing {
            after.discard_record();
        }
    }

    pub(super) fn on_transcribed(&self, latency_ms: u64) {
        if let Some(after) = &self.after_hearing {
            after.on_transcribed(self, latency_ms);
        }
    }

    pub(super) async fn on_heard(&self, gen: u64, text: String) -> Result<(), String> {
        match &self.after_hearing {
            Some(after) => after.on_heard(self, gen, text).await,
            None => {
                self.fail(gen, NO_AFTER_HEARING, None);
                Err(NO_AFTER_HEARING.into())
            }
        }
    }

    pub(super) async fn confirm_heard(
        &self,
        confirmation_gen: u64,
        input_ticket: Option<u64>,
    ) -> Result<(), String> {
        match &self.after_hearing {
            Some(after) => {
                after
                    .confirm_heard(self, confirmation_gen, input_ticket)
                    .await
            }
            None => Err(NO_AFTER_HEARING.into()),
        }
    }

    pub(super) fn on_session_ended(
        &self,
        outcome: crate::history::HistoryOutcome,
        error: Option<String>,
    ) {
        if let Some(after) = &self.after_hearing {
            after.on_session_ended(self, outcome, error);
        }
    }
}
