//! 往游戏聊天框打字：占住发送权、交给本体的“发到游戏”入口、超时；重发最近一条。
//!
//! 排队（一把输入锁）、等修饰键松开、回到开始收音时的窗口（拿到锁以后才切，ADR 041 D5）、按前台挑游戏配置和
//! 频道、注入都在本体的入口里（`kotone_core::send_to_game`），这里只决定发什么、回哪个窗口、用哪个频道。

use kotone_core::assistant::SessionPurpose;
use kotone_core::inject::{CancelToken, TargetWindow};
use kotone_core::orchestrator::{
    Orchestrator, OrchestratorState, PendingText, ReadyText, SendCompletion, SendOperation,
    SEND_STILL_RUNNING,
};
use kotone_core::send_to_game::{ChannelPick, ChatFailure, ChatRequest, RestoreFocus, Speaker};
use serde_json::json;

use crate::VoiceInput;

/// 已在状态机内原子占用发送权的任务快照。
///
/// `Sending` 状态与取消令牌必须在任何异步焦点恢复/注入工作开始前一次性写入，
/// 否则按钮双击或热键与按钮并发会启动两次注入，并让后一次覆盖前一次的令牌。
pub(crate) struct SendClaim {
    ready: ReadyText,
    gen: u64,
    operation: SendOperation,
    completion: SendCompletion,
    target: Option<TargetWindow>,
    channel: Option<(String, String)>,
}

impl VoiceInput {
    /// 重发最近一条：把历史里最新一条「发送成功」的文本重新注入（快捷键入口）。
    /// 语义（与用户确认）：
    /// - 仅 Idle 生效：会话进行中（录音/发送/预览等）按下静默忽略，不打断当前输入；
    /// - 用当前 profile + 当前 sticky 频道（与普通发送同一条 run_send 路径，
    ///   ADR-008 频道在发送时刻读取）；历史里的 profileId 不参与；
    /// - 无历史记录或文本为空 → 静默忽略（记日志）。
    pub async fn resend_last(&self, orch: &Orchestrator) {
        if orch.state() != OrchestratorState::Idle {
            return;
        }
        let dir = orch
            .history_dir
            .clone()
            .unwrap_or_else(kotone_core::history::history_dir);
        let text = match kotone_core::history::list_in(&dir) {
            Ok(records) => records
                .iter()
                .find(|r| {
                    r.outcome == kotone_core::history::HistoryOutcome::Sent
                        && !r.final_text.trim().is_empty()
                })
                .map(|r| r.final_text.clone()),
            Err(e) => {
                kotone_core::log::log(&format!("重发最近一条：读取历史失败（忽略）: {e}"));
                return;
            }
        };
        let Some(text) = text else {
            kotone_core::log::log("重发最近一条：历史中无发送成功的文本，忽略");
            return;
        };
        // 重发直发当前前台窗口：不沿用上次会话记录的注入目标（Idle 时
        // target_window 残留，若用户已切窗口，restore 会把焦点抢回旧窗口
        // 导致文本打进旧窗口）；target=None 时 run_send 跳过焦点恢复直发前台。
        // 历史读取期间可能有新会话开始；发送权必须在 op 锁内重新校验并占用。
        // 状态已经变化时保持原有语义：静默忽略，不打断当前输入。
        if let Ok(Some(claim)) = self
            .claim_send(
                orch,
                Some(ReadyText::unchanged(text)),
                None,
                &[OrchestratorState::Idle],
                true,
                None,
            )
            .await
        {
            self.run_send(orch, claim).await;
        }
    }

    /// 在 `op` 锁内校验来源状态并原子进入 Sending，保证每个状态只能产生一个发送任务。
    /// `ready=None` 用于 Preview/Error 重试：文本也在同一临界区内从 pending_text 取出。
    pub(crate) async fn claim_send(
        &self,
        orch: &Orchestrator,
        ready: Option<ReadyText>,
        expected_gen: Option<u64>,
        expected_states: &[OrchestratorState],
        clear_target: bool,
        input_ticket: Option<u64>,
    ) -> Result<Option<SendClaim>, String> {
        let _op = orch.lock_op().await;
        if !orch.input_ticket_is_current(input_ticket) {
            return Ok(None);
        }
        if !orch.runtime_inputs_enabled() {
            return Err("运行时已停止，不能发送".into());
        }
        let claim = {
            let mut session = orch.session();
            if expected_gen.is_some_and(|gen| session.gen() != gen) {
                return Ok(None);
            }
            if !expected_states.contains(&session.state()) {
                return Err(format!("当前状态 {:?} 不能发送", session.state()));
            }
            if clear_target && session.state() == OrchestratorState::Idle {
                // 历史重发是显式的听写操作，不沿用上一次助手会话的用途。
                session.reset_to_dictation();
            } else if session.purpose() == SessionPurpose::Assistant {
                return Err("助手请求不能进入游戏发送管线".into());
            }
            if session.send_is_running() {
                return Err(SEND_STILL_RUNNING.into());
            }
            let ready = match ready {
                Some(ready) => ready,
                None => session
                    .pending_text()
                    .and_then(PendingText::ready)
                    .cloned()
                    .ok_or_else(|| "无待发送文本".to_string())?,
            };
            let (target, channel) = {
                let mut state = self.state();
                if clear_target {
                    state.target_window = None;
                }
                (state.target_window, state.channel.clone())
            };
            let (operation, completion) = SendOperation::new();
            session.set_state(OrchestratorState::Sending);
            session.start_send(operation.clone());
            SendClaim {
                ready,
                gen: session.gen(),
                operation,
                completion,
                target,
                channel,
            }
        };
        orch.emit_state(
            OrchestratorState::Sending,
            Some(json!({ "text": claim.ready.text })),
        );
        Ok(Some(claim))
    }

    /// 交给本体的入口（ADR 041）。入口和完成凭据在同一个阻塞任务里，超时也不能另开并行任务。
    async fn execute_send(
        &self,
        orch: &Orchestrator,
        completion: SendCompletion,
        token: CancelToken,
        target: Option<TargetWindow>,
        text: String,
        channel: Option<(String, String)>,
    ) -> Result<(), SendFailure> {
        let entry = orch.send_to_game().clone();
        let mut request = ChatRequest::new(text, Speaker::Player);
        // 频道记的是 (游戏, 频道)：入口挑中的游戏不是那个游戏时退回默认频道（ADR-008）
        request.channel = channel.map(|(game, id)| ChannelPick {
            id,
            for_game: Some(game),
        });
        // 回到开始收音时的窗口：入口拿到锁以后再切（她正在打多行时切走，剩下的键会打进别的窗口）
        request.restore = target.map(|window| RestoreFocus {
            window,
            settle: orch.focus_restore_delay,
        });
        request.cancel = token.clone();
        let worker_token = token.clone();
        let mut task = tokio::task::spawn_blocking(move || {
            let _completion = completion;
            // 检查放在阻塞线程内，覆盖任务排队期间的取消。已经进入的系统调用
            // 无法强制中断；完成凭据会一直保留到它返回，阻止后续发送与之重叠。
            if worker_token.is_cancelled() {
                return Err(SendFailure::from(ChatFailure::Cancelled));
            }
            let outcome = entry.send(request)?;
            match outcome.stopped {
                None => Ok(()),
                // 发了一部分（多行时）：照实说，不重发
                Some(stopped) => Err(SendFailure {
                    message: format!(
                        "发了 {} 行，后面 {} 行没发：{stopped}",
                        outcome.sent_lines,
                        outcome.lines.len() - outcome.sent_lines
                    ),
                    ..SendFailure::from(stopped)
                }),
            }
        });
        match tokio::time::timeout(orch.inject_timeout, &mut task).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(SendFailure::plain("发送线程异常")),
            Err(_) => {
                token.cancel();
                // 保留并观察 JoinHandle；状态可返回 Error，但发送权由工作线程的
                // SendCompletion 持有，不能靠重启 runtime 绕过仍未结束的系统调用。
                tokio::spawn(async move {
                    if let Err(error) = task.await {
                        kotone_core::log::log(&format!(
                            "timed-out send worker ended abnormally: {error}"
                        ));
                    }
                });
                Err(SendFailure::plain(format!(
                    "发送超时（{}s），已请求取消。目标窗口可能已有部分输入，请先检查；\
                     上一次发送结束前无法重试，若持续无响应，请退出并重新打开应用",
                    orch.inject_timeout.as_secs()
                )))
            }
        }
    }

    /// Sending → Success/Error（§6 发送时序下半段；inject 实现负责按键细节）。
    /// 仅接受 `claim_send` 产生的已占用发送任务，不再改写状态或取消令牌。
    pub(crate) async fn run_send(&self, orch: &Orchestrator, claim: SendClaim) {
        let SendClaim {
            ready,
            gen,
            operation,
            completion,
            target,
            channel,
        } = claim;
        let token = operation.token();
        let ReadyText { source_text, text } = ready;
        let injection_started_at = std::time::Instant::now();
        orch.emit_process(
            "injection_started",
            json!({
                "textChars": text.chars().count() as u64,
                "channelId": channel.as_ref().map(|(_, cid)| cid.clone()),
            }),
        );
        // ADR-008 频道（发送时刻读取）和频道前缀、按前台挑游戏配置、Alt+F6 这类组合热键触发时修饰键还按着
        // （立刻开框会变成 Alt+Enter）……都在入口里。用户原文（预览 / 历史 / 重试）不带频道前缀。
        let result = self
            .execute_send(orch, completion, token, target, text.clone(), channel)
            .await;

        let (history_write, resume_continuous) = {
            let _op = orch.lock_op().await;
            let mut session = orch.session();
            session.finish_send(&operation);
            if session.gen() != gen || session.state() != OrchestratorState::Sending {
                return;
            }
            // history 终态（锁外落账，见下方 write_history 调用；两个分支各自产出
            // (outcome, 错误信息, 是否持续续录)，直接用 match 表达式——新 clippy
            // `unneeded late initialization` 会拒绝「先声明再在两个分支赋值」的写法。
            match result {
                Ok(()) => {
                    session.set_pending_text(None);
                    let resuming = session.continuous_session();
                    session.set_state(if resuming {
                        OrchestratorState::Idle
                    } else {
                        OrchestratorState::Success
                    });
                    drop(session);
                    if !resuming {
                        orch.emit_state(OrchestratorState::Success, Some(json!({ "text": text })));
                    }
                    orch.emit_process(
                        "injection_succeeded",
                        json!({
                            "outcome": "success",
                            "durationMs": injection_started_at.elapsed().as_millis() as u64
                        }),
                    );
                    ((kotone_core::history::HistoryOutcome::Sent, None), resuming)
                }
                Err(e) => {
                    // Error 保留 ReadyText，前端/confirm_send 可重试（§4.1）；
                    // needsElevation 透传 UIPI 提权信号（§10 R-1）；
                    // inputBlocked 透传「系统级拦截」信号（安全软件/反作弊），前端弹窗引导排查；
                    // 注入超时（inject_timeout）也落此分支，message 含超时提示
                    session.set_pending_text(Some(PendingText::Ready(ReadyText {
                        source_text,
                        text: text.clone(),
                    })));
                    session.set_state(OrchestratorState::Error);
                    drop(session);
                    orch.emit_state(
                        OrchestratorState::Error,
                        Some(json!({ "message": e.message, "needsElevation": e.needs_elevation, "inputBlocked": e.input_blocked, "text": text })),
                    );
                    orch.emit_process(
                        "injection_failed",
                        json!({
                            "outcome": "error",
                            "errorCode": e.code,
                            "durationMs": injection_started_at.elapsed().as_millis() as u64
                        }),
                    );
                    (
                        (kotone_core::history::HistoryOutcome::Error, Some(e.message)),
                        false,
                    )
                }
            }
        };
        self.write_history(orch, history_write.0, history_write.1);
        if resume_continuous {
            // solo 不显示 Success toast，也不等待 toast_dwell：发送落账后立即开始下一段。
            // 续录不重复播「录制音」（capture_started resumed=true）。
            let _ = orch.begin_resumed(SessionPurpose::Dictation, gen).await;
        } else {
            orch.schedule_idle(gen);
        }
    }
}

/// 没发出去：给悬浮窗的话、过程日志的错误码、提权 / 被拦截的信号（前端据此弹窗引导排查）。
pub(crate) struct SendFailure {
    message: String,
    code: &'static str,
    needs_elevation: bool,
    input_blocked: bool,
}

impl SendFailure {
    fn plain(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: "INJECTION_FAILED",
            needs_elevation: false,
            input_blocked: false,
        }
    }
}

impl From<ChatFailure> for SendFailure {
    fn from(failure: ChatFailure) -> Self {
        let (needs_elevation, input_blocked) = match &failure {
            ChatFailure::Inject(e) => (e.needs_elevation, e.input_blocked),
            _ => (false, false),
        };
        let code = match &failure {
            ChatFailure::ModifiersHeld => "MODIFIERS_STILL_DOWN",
            ChatFailure::Busy(_) => "INPUT_BUSY",
            // 选英雄、房间里经游戏插件的客户端接口发（ADR 041 第 2 期），没按键
            ChatFailure::Provider(_) => "CLIENT_CHAT_FAILED",
            _ if input_blocked => "INJECTION_BLOCKED",
            _ if needs_elevation => "INJECTION_ELEVATION_REQUIRED",
            _ => "INJECTION_FAILED",
        };
        Self {
            message: failure.to_string(),
            code,
            needs_elevation,
            input_blocked,
        }
    }
}
