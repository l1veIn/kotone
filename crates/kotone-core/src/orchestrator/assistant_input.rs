use super::capture::SessionOrigin;
use super::*;

impl Orchestrator {
    /// 语音唤醒后的收音（ADR 017）：助手会话（和语音输入用同一个音频 / 识别端口，但识别结果绝不进入游戏
    /// 注入），说完（VAD 静音）直接提交；一直没开口超过 `no_speech_timeout_ms` 就安静取消。会话 ID 以
    /// `assistant-wake-` 开头，宿主据此把它记为唤醒来源。只在空闲时开始。
    pub async fn begin_assistant_wake(&self, no_speech_timeout_ms: u32) -> Result<(), String> {
        self.begin_assistant_call(SessionOrigin::Wake {
            no_speech_timeout_ms,
        })
        .await
    }

    /// 呼叫来的助手会话（唤醒 / 说一句就走 / 呼叫键的录音笔）。只在空闲时开始。
    async fn begin_assistant_call(&self, origin: SessionOrigin) -> Result<(), String> {
        let _op = self.op.lock().await;
        // 上一次会话停在“成功 / 出错”的提示上：与热键一样，新会话先清掉它。
        if matches!(
            self.state(),
            OrchestratorState::Success | OrchestratorState::Error
        ) {
            self.cancel_locked();
        }
        self.begin_admitted_with(false, SessionPurpose::Assistant, None, origin)
    }

    /// 呼叫琴音（热键、首页 / 聊天窗口的按钮）：按键版的语音唤醒，按一次就听。
    /// - 空闲（或停在成功 / 出错提示上）：开始一次唤醒式收音（说完自动提交，没开口就安静结束）；
    /// - 琴音正在听：立刻结束这句、交给她（不用等静音）；
    /// - 正在语音输入（另一用途）或在识别 / 发送：先取消旧的，不开始新的（与助手热键一致）。
    ///
    /// `ticket`：热键的输入票（过期的按键不生效）；按钮传 None。
    pub async fn call_assistant(&self, ticket: Option<u64>, no_speech_timeout_ms: u32) {
        self.call_assistant_with(
            ticket,
            SessionOrigin::Wake {
                no_speech_timeout_ms,
            },
        )
        .await
    }

    /// 呼叫键的录音笔（ADR 033 2026-11-26 修订）：空闲时开始听，不判停；正在听就结束这句、直接交给她
    /// （没有预览）。其余和 [`Self::call_assistant`] 一样。
    pub async fn call_assistant_dictation(&self, ticket: Option<u64>) {
        self.call_assistant_with(ticket, SessionOrigin::CallDictation)
            .await
    }

    /// 呼叫键的“自动”按下（ADR 054）：照说一句就走（[`Self::call_assistant`]）；这一下开始了会话就记下来，
    /// 按着的时候静音判停不算数，松开时看按了多久（[`Self::call_assistant_release`]）。
    pub async fn call_assistant_auto_press(&self, ticket: Option<u64>, no_speech_timeout_ms: u32) {
        self.call_assistant_pressed(
            ticket,
            SessionOrigin::Wake {
                no_speech_timeout_ms,
            },
        )
        .await
    }

    /// 呼叫键的按住说话按下（ADR 054）：空闲时开始听（不判停），松开交给她。正在听（松开那一下丢了）就结束这句。
    pub async fn call_assistant_hold_press(&self, ticket: Option<u64>) {
        self.call_assistant_pressed(ticket, SessionOrigin::CallHold)
            .await
    }

    /// 呼叫键松开（自动 / 按住说话）。`hold_ms` = Some(门槛) 是自动：按了这么久以上才当场结束、交给她，短于它
    /// 照说一句就走等说完；None 是按住说话：松开就结束。这一下按下没开始会话（挂断、结束上一句……）时什么都不做。
    pub async fn call_assistant_release(&self, ticket: Option<u64>, hold_ms: Option<u32>) {
        let ticket = ticket.unwrap_or_else(|| self.hotkey_input_ticket());
        let _input = self.hold_input.lock().await;
        if let Some(gen) = self.take_long_press(ticket, hold_ms, SessionPurpose::Assistant) {
            let _ = self.end_inner(Some(gen), Some(ticket)).await;
        }
    }

    async fn call_assistant_pressed(&self, ticket: Option<u64>, origin: SessionOrigin) {
        let at = std::time::Instant::now();
        // 按下、松开按顺序进来（松开等按下开好麦）
        let _input = self.hold_input.lock().await;
        *self.key_press.lock().unwrap() = None;
        let (_, _, before) = self.hotkey_session_snapshot();
        self.call_assistant_with(ticket, origin).await;
        self.remember_press(before, SessionPurpose::Assistant, at);
    }

    async fn call_assistant_with(&self, ticket: Option<u64>, origin: SessionOrigin) {
        if !self.input_ticket_is_current(ticket) {
            return;
        }
        let (state, purpose, gen) = self.hotkey_session_snapshot();
        match state {
            OrchestratorState::Idle | OrchestratorState::Success | OrchestratorState::Error => {
                if let Err(e) = self.begin_assistant_call(origin).await {
                    crate::log::log(&format!("呼叫琴音没有开始：{e}"));
                }
            }
            OrchestratorState::Listening if purpose == SessionPurpose::Assistant => {
                let _ = self.end_inner(Some(gen), ticket).await;
            }
            _ => self.cancel_inner(ticket, Some(gen)).await,
        }
    }

    pub(super) async fn run_assistant(
        &self,
        gen: u64,
        request: AssistantRequest,
    ) -> Result<(), String> {
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
}
