//! 语音输入的“听到以后”（ADR 038 D3 第 7 条）：识别出的文字、文字处理、往游戏里打字、
//! 识别记录、聊天频道。以后整块搬进 `plugins/voice-input`。
//!
//! 本体（`mod.rs`、`capture.rs`、`hotkeys.rs`、`assistant_input.rs`、`finalize.rs`、
//! `lifecycle.rs`）只经过这个文件里的口子碰语音输入；下面几个文件里的函数只在本目录可见。
//! 口子按“听一句”的几个时刻分（括号里是谁调）：
//!
//! - 开始收音（capture）：[`Orchestrator::start_record`] 建识别记录草稿，草稿同时是录音旁听者；
//!   发布会话时在锁里 [`VoiceInputState::on_capture_started`] 记下注入目标窗口、收下草稿。
//! - 听到了一句（finalize）：[`Orchestrator::on_transcribed`] 记识别耗时、存历史音频，
//!   本体落完评测录档后 [`Orchestrator::on_heard`] 文字处理，然后预览或发送。
//!   空识别：[`Orchestrator::heard_audio_ms`] 给诊断用，锁里 [`VoiceInputState::discard_record`]。
//! - 确认（lifecycle）：[`Orchestrator::confirm_heard`] 预览后发送、出错后重试处理或发送。
//! - 会话结束（lifecycle、mod）：锁里 [`Inner::stop_send`] 请求停下发送，
//!   锁外 [`Orchestrator::on_session_ended`] 记一条识别记录。
//! - 发送在不在跑（mod、capture）：[`Inner::send_is_running`]。
//! - 重发、频道、文字处理开关、注入处理器：`resend_last`、`on_cycle_channel`、
//!   `on_post_processing_toggle` / `_cycle`、`with_processors`，公开给壳，本体不调。

mod hotkeys;
mod postprocessing;
mod records;
mod send;
mod text;

pub(super) use self::send::SEND_STILL_RUNNING;
pub(super) use self::text::PendingText;

use self::records::HistoryDraft;
use self::send::SendOperation;
use super::capture::CaptureObserver;
use super::Inner;
use crate::orchestrator::*;
use crate::postprocess::ProcessorRegistry;

/// 语音输入在 Orchestrator 上的部件：文字处理器注册表（由外层实现 crate 注册）。
pub(super) struct VoiceInput {
    processors: Arc<ProcessorRegistry>,
}

impl Default for VoiceInput {
    fn default() -> Self {
        Self {
            processors: Arc::new(ProcessorRegistry::new()),
        }
    }
}

/// 语音输入跨状态迁移要记住的东西，放在 `Inner` 里（和状态同一把锁）。本体不看里面。
#[derive(Default)]
pub(super) struct VoiceInputState {
    /// begin 时记录的前台窗口 = 注入目标（发送前把焦点还给它）
    target_window: Option<TargetWindow>,
    /// history 草稿（history.mode != off 时 begin 创建；不放 ActiveSession——
    /// end() 会 take 走 active，而 Sending/Error 态的终态落账仍需要草稿）
    history: Option<Arc<Mutex<HistoryDraft>>>,
    /// 当前聊天频道（ADR-008）：(profile_id, channel_id)，sticky——
    /// 切换后保持到下次切换；profile_id 与当前激活 profile 不符时视为默认频道
    /// （用户切了游戏适配，旧频道选择自动失效）。None = 默认频道。
    channel: Option<(String, String)>,
}

/// 正在准备或执行的发送。取消/超时后仍保留，直到阻塞调用真正退出。
/// 留在 `Inner` 上（单元测试直接写这个字段）；本体只经 `send_is_running` / `stop_send` 问它。
pub(super) type SendSlot = Option<SendOperation>;

/// 开始收音时建的识别记录草稿（识别记录关着、或不是听写时为空），
/// 发布会话时交回 [`VoiceInputState::on_capture_started`]。本体只拿它当录音旁听者。
pub(super) struct RecordDraft(Option<Arc<Mutex<HistoryDraft>>>);

impl RecordDraft {
    pub(super) fn observer(&self) -> Option<Arc<dyn CaptureObserver>> {
        self.0
            .clone()
            .map(|draft| draft as Arc<dyn CaptureObserver>)
    }
}

impl VoiceInputState {
    /// 发布会话（调用方持有 inner 锁）：记下注入目标窗口、收下这一句的识别记录草稿。
    pub(super) fn on_capture_started(&mut self, target: Option<TargetWindow>, record: RecordDraft) {
        self.target_window = target;
        self.history = record.0;
    }

    /// 空识别（调用方持有 op 与 inner 锁）：草稿清掉，不写记录。
    pub(super) fn discard_record(&mut self) {
        self.history = None;
    }
}

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

impl Orchestrator {
    /// 开始收音：history 开着且是听写时建识别记录草稿（与评测录档共用 process case id）。
    pub(super) fn start_record(
        &self,
        purpose: SessionPurpose,
        settings: &Settings,
        engine_id: &str,
        process_case_id: &str,
    ) -> RecordDraft {
        let history_on = purpose == SessionPurpose::Dictation
            && settings.history.mode != crate::history::HistoryMode::Off;
        RecordDraft(history_on.then(|| {
            Arc::new(Mutex::new(HistoryDraft::new(
                process_case_id.to_string(),
                engine_id,
                settings,
            )))
        }))
    }

    /// 空识别时给诊断用的录音时长（只有识别记录草稿在时才有）。
    pub(super) fn heard_audio_ms(&self) -> Option<u64> {
        self.inner
            .lock()
            .unwrap()
            .voice
            .history
            .as_ref()
            .map(|draft| {
                let draft = draft.lock().unwrap();
                draft.audio_samples * 1000 / crate::eval::SAMPLE_RATE as u64
            })
    }

    /// 识别完成：草稿补齐 finalize 产物（终态落账时写入记录），历史音频独立落盘
    /// （即使 evalRecording 关闭也能保存与播放）。
    pub(super) fn on_transcribed(&self, latency_ms: u64) {
        if let Some(h) = &self.inner.lock().unwrap().voice.history {
            let mut g = h.lock().unwrap();
            g.finalize_latency_ms = Some(latency_ms);
        }
        self.persist_history_audio();
    }

    /// 听到了一句：STT 产物与可交付文本从此分离。pipeline 关闭/为空时 runner 走零步骤
    /// 透传，不产生 Processing 状态，保证旧配置行为与事件序列不变。
    pub(super) async fn on_heard(&self, gen: u64, text: String) -> Result<(), String> {
        let recognized = self.snapshot_recognized_text(text);
        if let Some(h) = &self.inner.lock().unwrap().voice.history {
            h.lock().unwrap().source_text = Some(recognized.source_text.clone());
        }
        self.run_postprocessing(gen, recognized, &[OrchestratorState::Transcribing])
            .await
    }

    /// 确认（听写）：Error 带识别原文 → 重新跑文字处理；否则 Preview / Error 的
    /// ReadyText 在 op 锁内原子进入 Sending 再发送。
    pub(super) async fn confirm_heard(
        &self,
        confirmation_gen: u64,
        input_ticket: Option<u64>,
    ) -> Result<(), String> {
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

    /// 会话结束（出错 / 取消 / 中止）：按识别记录的规则落账一条（见 `write_history`）。
    pub(super) fn on_session_ended(
        &self,
        outcome: crate::history::HistoryOutcome,
        error: Option<String>,
    ) {
        self.write_history(outcome, error);
    }
}

/// 单元测试（`orchestrator/tests.rs`）直接用到的语音输入类型和纯函数。
#[cfg(test)]
pub(super) mod testing {
    pub(in crate::orchestrator) use super::records::{trim_speech_pcm, HistoryDraft};
    pub(in crate::orchestrator) use super::send::{resolve_send_strategy, SendOperation};
}
