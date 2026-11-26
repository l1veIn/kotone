//! 往游戏聊天框打字：占住发送权、交给本体的“发到游戏”入口、超时；重发最近一条；推送卡片上的重发。
//!
//! 排队（一把输入锁）、等修饰键松开、回到开始收音时的窗口（拿到锁以后才切，ADR 041 D5）、按前台挑游戏配置和
//! 频道、注入都在本体的入口里（`kotone_core::send_to_game`），这里只决定发什么、回哪个窗口、用哪个频道。
//! 每发一次，结果交给推送台（[`crate::pushes`]：发出去的话 / 没发出去的卡片、事件）。

use kotone_core::assistant::SessionPurpose;
use kotone_core::history::HistoryOutcome;
use kotone_core::inject::{CancelToken, TargetWindow};
use kotone_core::orchestrator::{
    Orchestrator, OrchestratorState, PendingText, ReadyText, SendCompletion, SendOperation,
    SEND_STILL_RUNNING,
};
use kotone_core::send_to_game::{
    ChannelPick, ChatFailure, ChatOutcome, ChatRequest, RestoreFocus, Speaker,
};
use serde_json::json;

use crate::pushes::{Attempt, Failed};
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
    /// 这一句的 id（识别记录的 sessionId；推送卡片的 key 用它）
    id: String,
    /// 只发到那个窗口（推送卡片上的重发，`ChatRequest::strict_target`）
    strict: bool,
    history: HistoryWrite,
}

/// 这次发给谁、怎么记（[`VoiceInput::claim_send`]）。缺省 = 现在这一句：开始收音时的窗口、现在的频道、照草稿记。
#[derive(Default)]
pub(crate) struct Route {
    /// 这一句的 id；None = 现在这一句（本体的 process case id，和识别记录的 sessionId 是同一个）
    pub(crate) id: Option<String>,
    /// 发去哪个窗口、哪个频道；None = 开始收音时记下的窗口、现在选的频道
    pub(crate) place: Option<Place>,
    pub(crate) strict: bool,
    pub(crate) history: HistoryWrite,
}

/// 发去哪个窗口（None = 不切窗口）、哪个频道 (游戏, 频道)（None = 游戏配置的默认频道）。
pub(crate) type Place = (Option<TargetWindow>, Option<(String, String)>);

/// 识别记录怎么记。
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) enum HistoryWrite {
    /// 照这一句的草稿记（听写、输入条上的重试、重发最近一条热键）
    #[default]
    Draft,
    /// 推送卡片上重发的不是现在这一句：照那一句原来的记录再记一条（sent / error，“失败 → 重发”）
    Copy(String),
}

/// 推送卡片上的重发的结果（页面照它显示；卡片本身由推送原地更新）。
#[derive(Debug)]
pub enum Resent {
    /// 发出去了
    Sent(ChatOutcome),
    /// 发了，没发出去：原因（卡片上换成这个原因）
    Failed(String),
    /// 没发：原因（找不到这句、已经重发过、正忙）
    Refused(String),
}

/// 重发最近一条热键、没有游戏的地方用的新 id。
fn fresh_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    format!(
        "resend-{}-{}",
        kotone_core::eval::new_session_id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
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
        // 它是新的一次发送（卡片的 key 不能用上一句的）。
        if let Ok(Some(claim)) = self
            .claim_send(
                orch,
                Some(ReadyText::unchanged(text)),
                None,
                &[OrchestratorState::Idle],
                true,
                None,
                Route {
                    id: Some(fresh_id()),
                    ..Route::default()
                },
            )
            .await
        {
            let _ = self.run_send(orch, claim).await;
        }
    }

    /// 推送卡片上的“重发”（ADR 043 D10，玩家点的）：把没发出去的那一句经本体的入口再发一次，回到当时要发去的那个窗口、
    /// 用当时选的频道；切不回去（窗口关了、Kotone 重启过又不在游戏里）就不发，绝不打进 Kotone 自己的窗口（入口的
    /// `strict_target`）。不是替玩家补按键（ADR 032）：发的是这一句识别出来的话，玩家看着卡片点的。
    ///
    /// - 输入条上还停着这一句（没发出去、等着重试）：和点输入条上的“重试发送”一样接着用它；
    /// - 否则（空着、刚发完一句、停着别的一句）：照这一句的文字另发一次，识别记录照原来那条再记一条。
    ///
    /// `channel_hint`：卡片上记的当时的频道（Kotone 重启过、内存里没有这一句时用）。结果由推送原地更新卡片。
    pub async fn resend_card(
        &self,
        orch: &Orchestrator,
        id: &str,
        channel_hint: Option<(String, String)>,
    ) -> Resent {
        let failed = match self.pushes.failed(id) {
            Some(failed) => failed,
            None => match self.failed_from_history(orch, id, channel_hint) {
                Some(failed) => failed,
                None => {
                    return Resent::Refused(
                        "这句话找不到了（Kotone 重启过，识别记录里也没有），没法重发".into(),
                    )
                }
            },
        };
        if failed.resent.is_some() {
            return Resent::Refused("这句已经重发过了".into());
        }
        let retry_here = {
            let session = orch.session();
            session.state() == OrchestratorState::Error && session.process_case_id() == Some(id)
        };
        let claim = if retry_here {
            self.claim_send(
                orch,
                None,
                None,
                &[OrchestratorState::Error],
                false,
                None,
                Route {
                    id: Some(id.to_string()),
                    strict: true,
                    ..Route::default()
                },
            )
            .await
        } else {
            let source = failed.source.clone().unwrap_or_else(|| failed.text.clone());
            self.claim_send(
                orch,
                Some(ReadyText {
                    source_text: source,
                    text: failed.text.clone(),
                }),
                None,
                &[
                    OrchestratorState::Idle,
                    OrchestratorState::Success,
                    OrchestratorState::Error,
                ],
                true,
                None,
                Route {
                    id: Some(id.to_string()),
                    place: Some((failed.target, failed.channel.clone())),
                    strict: true,
                    history: HistoryWrite::Copy(id.to_string()),
                },
            )
            .await
        };
        let claim = match claim {
            Ok(Some(claim)) => claim,
            Ok(None) => return Resent::Refused("这一句刚刚变了，再点一次重发".into()),
            Err(e) if e.contains("运行时已停止") => {
                return Resent::Refused(
                    "语音识别没在运行（在「设置 → 模型」里启动），这次没重发".into(),
                )
            }
            Err(e) => {
                kotone_core::log::log(&format!("voice-input 重发 {id}：{e}"));
                return Resent::Refused("正在说话或者正在发上一句，等它结束再点重发".into());
            }
        };
        match self.run_send(orch, claim).await {
            Ok(outcome) => Resent::Sent(outcome),
            Err(reason) => Resent::Failed(reason),
        }
    }

    /// 内存里没有这一句（Kotone 重启过、太早）：从识别记录里找那条没发出去的。
    fn failed_from_history(
        &self,
        orch: &Orchestrator,
        id: &str,
        channel_hint: Option<(String, String)>,
    ) -> Option<Failed> {
        let dir = orch
            .history_dir
            .clone()
            .unwrap_or_else(kotone_core::history::history_dir);
        let records = kotone_core::history::list_in(&dir).ok()?;
        let record = records.into_iter().find(|r| r.session_id == id)?;
        if record.outcome == HistoryOutcome::Sent {
            // 后来发出去了（输入条上点了重试）
            let mut failed = Failed::recovered(id, record.final_text, None, None);
            failed.resent = Some((
                record.game.unwrap_or_else(|| place("", "")),
                record.channel.unwrap_or_else(|| place("", "")),
                0,
            ));
            return Some(failed);
        }
        if record.outcome != HistoryOutcome::Error || record.final_text.trim().is_empty() {
            return None;
        }
        Some(Failed::recovered(
            id,
            record.final_text,
            record.source_text,
            channel_hint,
        ))
    }

    /// 在 `op` 锁内校验来源状态并原子进入 Sending，保证每个状态只能产生一个发送任务。
    /// `ready=None` 用于 Preview/Error 重试：文本也在同一临界区内从 pending_text 取出。
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn claim_send(
        &self,
        orch: &Orchestrator,
        ready: Option<ReadyText>,
        expected_gen: Option<u64>,
        expected_states: &[OrchestratorState],
        clear_target: bool,
        input_ticket: Option<u64>,
        route: Route,
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
            if clear_target
                && matches!(
                    session.state(),
                    OrchestratorState::Idle | OrchestratorState::Success
                )
            {
                // 历史重发、卡片上的重发是显式的听写操作，不沿用上一次助手会话的用途。
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
            let id = route
                .id
                .clone()
                .or_else(|| session.process_case_id().map(str::to_string))
                .unwrap_or_else(fresh_id);
            let (target, channel) = {
                let mut state = self.state();
                match route.place {
                    Some(place) => place,
                    None => {
                        if clear_target {
                            state.target_window = None;
                        }
                        (state.target_window, state.channel.clone())
                    }
                }
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
                id,
                strict: route.strict,
                history: route.history,
            }
        };
        orch.emit_state(
            OrchestratorState::Sending,
            Some(json!({ "text": claim.ready.text })),
        );
        Ok(Some(claim))
    }

    /// 交给本体的入口（ADR 041）。入口和完成凭据在同一个阻塞任务里，超时也不能另开并行任务。
    #[allow(clippy::too_many_arguments)]
    async fn execute_send(
        &self,
        orch: &Orchestrator,
        completion: SendCompletion,
        token: CancelToken,
        target: Option<TargetWindow>,
        text: String,
        channel: Option<(String, String)>,
        strict: bool,
    ) -> Result<ChatOutcome, SendFailure> {
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
        request.strict_target = strict;
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
            match outcome.stopped.clone() {
                None => Ok(outcome),
                // 发了一部分（多行时）：照实说，不重发
                Some(stopped) => Err(SendFailure {
                    message: format!(
                        "发了 {} 行，后面 {} 行没发：{stopped}",
                        outcome.sent_lines,
                        outcome.lines.len() - outcome.sent_lines
                    ),
                    outcome: Some(Box::new(outcome)),
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
    /// 返回发到了哪里，或者没发出去的原因（推送卡片上的重发用；别的地方不看）。
    pub(crate) async fn run_send(
        &self,
        orch: &Orchestrator,
        claim: SendClaim,
    ) -> Result<ChatOutcome, String> {
        let SendClaim {
            ready,
            gen,
            operation,
            completion,
            target,
            channel,
            id,
            strict,
            history,
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
            .execute_send(
                orch,
                completion,
                token,
                target,
                text.clone(),
                channel.clone(),
                strict,
            )
            .await;

        let (history_write, resume_continuous) = {
            let _op = orch.lock_op().await;
            let mut session = orch.session();
            session.finish_send(&operation);
            if session.gen() != gen || session.state() != OrchestratorState::Sending {
                return Err("这一句被取消了".into());
            }
            // history 终态（锁外落账，见下方 write_history 调用；两个分支各自产出
            // (outcome, 错误信息, 是否持续续录)，直接用 match 表达式——新 clippy
            // `unneeded late initialization` 会拒绝「先声明再在两个分支赋值」的写法。
            match &result {
                Ok(_) => {
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
                    ((HistoryOutcome::Sent, None), resuming)
                }
                Err(e) => {
                    // Error 保留 ReadyText，前端/confirm_send 可重试（§4.1）；
                    // needsElevation 透传 UIPI 提权信号（§10 R-1）；
                    // inputBlocked 透传「系统级拦截」信号（安全软件/反作弊），前端弹窗引导排查；
                    // 注入超时（inject_timeout）也落此分支，message 含超时提示
                    session.set_pending_text(Some(PendingText::Ready(ReadyText {
                        source_text: source_text.clone(),
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
                    ((HistoryOutcome::Error, Some(e.message.clone())), false)
                }
            }
        };
        // 发到了哪里（全发了，或者多行时发了一部分）：记进识别记录、交给推送台
        let sent_to = match &result {
            Ok(outcome) => Some(outcome),
            Err(e) => e.outcome.as_deref(),
        };
        match &history {
            HistoryWrite::Draft => {
                self.set_draft_place(sent_to);
                self.write_history(orch, history_write.0, history_write.1);
            }
            HistoryWrite::Copy(of) => {
                self.copy_history(orch, of, &text, history_write.0, history_write.1, sent_to)
            }
        }
        self.pushes.report(Attempt {
            id,
            text,
            source: source_text,
            target,
            channel,
            outcome: sent_to.cloned(),
            failure: result
                .as_ref()
                .err()
                .map(|e| (e.message.clone(), e.code.to_string())),
            game: orch.settings().read().unwrap().active_profile_id.clone(),
            now_ms: now_ms(),
        });
        if resume_continuous {
            // solo 不显示 Success toast，也不等待 toast_dwell：发送落账后立即开始下一段。
            // 续录不重复播「录制音」（capture_started resumed=true）。
            let _ = orch.begin_resumed(SessionPurpose::Dictation, gen).await;
        } else {
            orch.schedule_idle(gen);
        }
        result.map_err(|e| e.message)
    }
}

fn place(id: &str, name: &str) -> kotone_core::history::HistoryPlace {
    kotone_core::history::HistoryPlace {
        id: id.to_string(),
        name: name.to_string(),
    }
}

/// 没发出去：给输入条的话、过程日志的错误码、提权 / 被拦截的信号（前端据此弹窗引导排查）；多行时发了一部分的结果。
pub(crate) struct SendFailure {
    pub(crate) message: String,
    pub(crate) code: &'static str,
    needs_elevation: bool,
    input_blocked: bool,
    /// 多行时发了前几行：发到了哪里、发了几行
    outcome: Option<Box<ChatOutcome>>,
}

impl SendFailure {
    fn plain(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: "INJECTION_FAILED",
            needs_elevation: false,
            input_blocked: false,
            outcome: None,
        }
    }
}

/// 推送卡片上的重发切不回要发去的游戏窗口（入口的 `strict_target`）时给玩家的话。
pub(crate) const NO_GAME_WINDOW: &str =
    "没找到要发去的游戏窗口（关了，或者前台是 Kotone 自己）：回到游戏里再点重发";

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
            // 只有推送卡片上的重发会遇到（入口的 strict_target）：一个键都没按
            ChatFailure::WindowGone
            | ChatFailure::OwnWindow
            | ChatFailure::Desktop
            | ChatFailure::NoForeground => "TARGET_WINDOW_GONE",
            _ if input_blocked => "INJECTION_BLOCKED",
            _ if needs_elevation => "INJECTION_ELEVATION_REQUIRED",
            _ => "INJECTION_FAILED",
        };
        let message = match code {
            "TARGET_WINDOW_GONE" => NO_GAME_WINDOW.to_string(),
            _ => failure.to_string(),
        };
        Self {
            message,
            code,
            needs_elevation,
            input_blocked,
            outcome: None,
        }
    }
}
