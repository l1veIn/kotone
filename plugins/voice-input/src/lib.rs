//! 语音输入插件（ADR 038 D4）：听写“听到以后”的事——识别出的文字、文字处理、预览、往游戏里打字、
//! 识别记录、聊天频道。
//!
//! 语音识别只有一份，在本体（ADR 038 D3 第 7 条）：本体的“听一句”（`kotone_core::orchestrator`，热键、录音、
//! 识别、状态机）在几个时刻调这里的 [`VoiceInput`]（实现 [`AfterHearing`]，口子的说明见那个 trait）。
//! 宿主用 [`VoiceInputExt::with_processors`] 接上；关掉插件时宿主不登记语音输入热键，听写就不会开始。
//!
//! - [`postprocessing`]：文字处理（屏蔽词、润色、翻译的流程；处理器由宿主注册）和“可交付文字”的唯一出口；
//! - [`send`]：往游戏聊天框打字（占住发送权、交给本体的“发到游戏”入口、超时）、重发最近一条、推送卡片上的重发；
//! - [`pushes`]：推送和事件（ADR 043 D10）：发出去的话 / 没发出去的卡片，发出去一句 / 没发出去两件事；
//! - [`records`]：识别记录草稿（旁听录音）、落账（发到了哪个游戏、哪个频道）、历史音频裁剪；
//! - [`hotkeys`]：聊天频道切换、文字处理开关 / 切换流程。
//!
//! 能绑热键的 5 个命令登记在 `plugin.toml` 的 `[[commands]]`（ADR 040 D1）：宿主按热键时经本体的“听一句”
//! 调 [`AfterHearing::command`]（这里的实现），插件不碰键盘钩子（D7）。
//!
//! - [`plugin`]：插件清单那一面（ADR 038 D5）。没有给她的工具：她替玩家发言是本体的 `game_chat.send`（ADR 039 D2）。
//!
//! 锁的顺序照本体：op → 状态锁 → 这里的 `state`。拿着 `state` 时不调本体的任何方法。

use std::any::Any;
use std::sync::{Arc, Mutex, MutexGuard};

use kotone_core::assistant::SessionPurpose;
use kotone_core::hotkey::HotkeyMode;
use kotone_core::hotkey_commands::Edge;
use kotone_core::inject::TargetWindow;
use kotone_core::orchestrator::{
    AfterHearing, BoxFuture, Orchestrator, OrchestratorState, PendingText, RecordDraft,
};
use kotone_core::postprocess::ProcessorRegistry;
use kotone_core::settings::Settings;

pub mod hotkeys;
pub mod plugin;
pub mod postprocessing;
pub mod pushes;
pub mod records;
pub mod send;

pub use self::plugin::VoiceInputPlugin;

use self::records::{DraftCell, HistoryDraft};

/// 语音输入：文字处理器注册表（宿主注册）、跨状态迁移要记住的东西、推送台。
pub struct VoiceInput {
    processors: Arc<ProcessorRegistry>,
    state: Mutex<VoiceInputState>,
    /// 发出去的话 / 没发出去的卡片（自己的锁，不在 `state` 里：推送在后台线程里交给宿主）
    pushes: Arc<pushes::PushDesk>,
}

/// 跨状态迁移要记住的东西（本体不看里面）。
#[derive(Default)]
struct VoiceInputState {
    /// begin 时记录的前台窗口 = 注入目标（发送前把焦点还给它）
    target_window: Option<TargetWindow>,
    /// history 草稿（history.mode != off 时 begin 创建；Sending/Error 态的终态落账仍需要它）
    history: Option<Arc<DraftCell>>,
    /// 当前聊天频道（ADR-008）：(profile_id, channel_id)，sticky——
    /// 切换后保持到下次切换；profile_id 与当前激活 profile 不符时视为默认频道
    /// （用户切了游戏适配，旧频道选择自动失效）。None = 默认频道。
    channel: Option<(String, String)>,
}

impl VoiceInput {
    pub fn new(processors: Arc<ProcessorRegistry>) -> Self {
        Self {
            processors,
            state: Mutex::new(VoiceInputState::default()),
            pushes: Arc::new(pushes::PushDesk::default()),
        }
    }

    fn state(&self) -> MutexGuard<'_, VoiceInputState> {
        self.state.lock().unwrap()
    }
}

impl AfterHearing for VoiceInput {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn owner(&self) -> &str {
        &plugin::manifest().id
    }

    /// 接上了：记下本体（推送卡片上的重发是玩家在页面上点的，不在“听一句”里，ADR 043 D10）。
    fn attached(&self, orch: &Arc<Orchestrator>) {
        pushes::attach(orch);
    }

    /// 清单里登记的 5 个命令（`plugin.toml` 的 `[[commands]]`，ADR 040 D1），宿主经“听一句”转进来。
    /// “说话”是按住的命令：按住还是按一下由交互模式决定（对讲机按住说、松开发；其余按一下开始、再按一下），
    /// 本体的“听一句”照旧管收音和入场票。其余 4 个按下时做一次（任意状态都接受，和会话正交）。
    fn command<'a>(
        &'a self,
        orch: &'a Orchestrator,
        command: &'a str,
        edge: Edge,
        input_ticket: Option<u64>,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            let pressed = edge == Edge::Press;
            // 说话可以有几个键（ADR 054）：第 2 个起宿主转来 `talk~<n>`
            let (command, nth) = kotone_core::hotkey_options::split_nth(command);
            match command {
                plugin::CMD_TALK => {
                    // 这个键写了自己的方式（那一条的 `mode`），或者是第 2 个起的键（跟着 `interactionMode`）：照它；
                    // 第一个键没写方式的照旧的路走（`interactionMode`，含旧的混合组合）
                    let (mode, keyed, hold_ms) = {
                        let settings = orch.settings().read().unwrap();
                        let talk = kotone_core::hotkey_bindings::CMD_TALK;
                        (
                            kotone_core::interaction::effective_hotkey_mode(&settings),
                            settings.binding_mode(talk, nth).or_else(|| {
                                (nth >= 2)
                                    .then(|| settings.global_voice_mode(talk))
                                    .flatten()
                            }),
                            settings.hold_ms(),
                        )
                    };
                    if let Some(keyed) = keyed {
                        orch.on_talk_key(keyed, pressed, input_ticket, hold_ms)
                            .await;
                        return;
                    }
                    match (mode, input_ticket) {
                        (HotkeyMode::Hold, Some(ticket)) => {
                            orch.on_hotkey_hold_with_ticket(pressed, ticket).await
                        }
                        (HotkeyMode::Hold, None) => orch.on_hotkey_hold(pressed).await,
                        (HotkeyMode::Toggle, _) if !pressed => {}
                        (HotkeyMode::Toggle, Some(ticket)) => {
                            orch.on_hotkey_toggle_with_ticket(ticket).await
                        }
                        (HotkeyMode::Toggle, None) => orch.on_hotkey_toggle().await,
                    }
                }
                _ if !pressed => {}
                plugin::CMD_CHANNEL_CYCLE => self.on_cycle_channel(orch).await,
                plugin::CMD_RESEND_LAST => self.resend_last(orch).await,
                plugin::CMD_PP_TOGGLE => hotkeys::toggle_post_processing(orch).await,
                plugin::CMD_PP_CYCLE => hotkeys::cycle_post_processing(orch).await,
                other => kotone_core::log::log(&format!("语音输入没有命令 {other}")),
            }
        })
    }

    /// 开始收音：history 开着且是听写时建识别记录草稿（与评测录档共用 process case id）。
    fn start_record(
        &self,
        purpose: SessionPurpose,
        settings: &Settings,
        engine_id: &str,
        process_case_id: &str,
    ) -> RecordDraft {
        let history_on = purpose == SessionPurpose::Dictation
            && settings.history.mode != kotone_core::history::HistoryMode::Off;
        if !history_on {
            return RecordDraft::none();
        }
        RecordDraft::new(Arc::new(DraftCell::new(HistoryDraft::new(
            process_case_id.to_string(),
            engine_id,
            settings,
        ))))
    }

    fn on_capture_started(&self, target: Option<TargetWindow>, record: RecordDraft) {
        let mut state = self.state();
        state.target_window = target;
        state.history = record.into_draft::<DraftCell>();
    }

    fn heard_audio_ms(&self) -> Option<u64> {
        self.state().history.as_ref().map(|draft| {
            let draft = draft.lock().unwrap();
            draft.audio_samples * 1000 / kotone_core::eval::SAMPLE_RATE as u64
        })
    }

    fn discard_record(&self) {
        self.state().history = None;
    }

    /// 识别完成：草稿补齐 finalize 产物（终态落账时写入记录），历史音频独立落盘
    /// （即使 evalRecording 关闭也能保存与播放）。
    fn on_transcribed(&self, orch: &Orchestrator, latency_ms: u64) {
        if let Some(h) = &self.state().history {
            h.lock().unwrap().finalize_latency_ms = Some(latency_ms);
        }
        self.persist_history_audio(orch);
    }

    /// 听到了一句：STT 产物与可交付文本从此分离。pipeline 关闭/为空时 runner 走零步骤
    /// 透传，不产生 Processing 状态，保证旧配置行为与事件序列不变。
    fn on_heard<'a>(
        &'a self,
        orch: &'a Orchestrator,
        gen: u64,
        text: String,
    ) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            let recognized = self.snapshot_recognized_text(orch, text);
            if let Some(h) = &self.state().history {
                h.lock().unwrap().source_text = Some(recognized.source_text.clone());
            }
            self.run_postprocessing(orch, gen, recognized, &[OrchestratorState::Transcribing])
                .await
        })
    }

    /// 确认（听写）：Error 带识别原文 → 重新跑文字处理；否则 Preview / Error 的
    /// ReadyText 在 op 锁内原子进入 Sending 再发送。
    fn confirm_heard<'a>(
        &'a self,
        orch: &'a Orchestrator,
        confirmation_gen: u64,
        input_ticket: Option<u64>,
    ) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            let retry_processing = {
                let session = orch.session();
                if session.gen() == confirmation_gen && session.state() == OrchestratorState::Error
                {
                    session
                        .pending_text()
                        .and_then(PendingText::recognized)
                        .cloned()
                        .map(|recognized| (session.gen(), recognized))
                } else {
                    None
                }
            };
            if let Some((gen, recognized)) = retry_processing {
                return self
                    .run_postprocessing(orch, gen, recognized, &[OrchestratorState::Error])
                    .await;
            }

            let claim = self
                .claim_send(
                    orch,
                    None,
                    Some(confirmation_gen),
                    &[OrchestratorState::Preview, OrchestratorState::Error],
                    false,
                    input_ticket,
                    send::Route::default(),
                )
                .await?
                .ok_or_else(|| "待发送内容已失效".to_string())?;
            let _ = self.run_send(orch, claim).await;
            Ok(())
        })
    }

    /// 会话结束（出错 / 取消 / 中止）：按识别记录的规则落账一条（见 `write_history`）。
    fn on_session_ended(
        &self,
        orch: &Orchestrator,
        outcome: kotone_core::history::HistoryOutcome,
        error: Option<String>,
    ) {
        self.write_history(orch, outcome, error);
    }
}

/// 宿主（桌面端、命令行）和测试接语音输入、调语音输入自己的热键用。
pub trait VoiceInputExt: Sized {
    /// 接上语音输入，文字处理器用宿主注册好的这一份。
    fn with_processors(self, processors: Arc<ProcessorRegistry>) -> Self;

    /// 接上语音输入，不注册文字处理器（流程里有步骤时会报“处理器不存在”）。
    fn with_voice_input(self) -> Self {
        self.with_processors(Arc::new(ProcessorRegistry::new()))
    }

    /// 接上的语音输入（没接时 None）。
    fn voice_input(&self) -> Option<&VoiceInput>;
}

impl VoiceInputExt for Orchestrator {
    fn with_processors(self, processors: Arc<ProcessorRegistry>) -> Self {
        self.with_after_hearing(Arc::new(VoiceInput::new(processors)))
    }

    fn voice_input(&self) -> Option<&VoiceInput> {
        self.after_hearing()
            .and_then(|after| after.as_any().downcast_ref::<VoiceInput>())
    }
}

#[cfg(test)]
mod tests;
