use super::capture::SessionOrigin;
use super::*;

impl Orchestrator {
    /// 助手使用同一个音频/STT端口，但用途在开始时固定，识别结果绝不进入游戏注入。
    pub async fn begin_assistant(&self) -> Result<(), String> {
        self.begin_inner(false, SessionPurpose::Assistant, None, None)
            .await
    }

    /// 语音唤醒后的收音（ADR 017）：助手会话，说完（VAD 静音）直接提交，不管助手热键的
    /// 交互模式；一直没开口超过 `no_speech_timeout_ms` 就安静取消。会话 ID 以
    /// `assistant-wake-` 开头，宿主据此把它记为唤醒来源。只在空闲时开始。
    pub async fn begin_assistant_wake(&self, no_speech_timeout_ms: u32) -> Result<(), String> {
        let _op = self.op.lock().await;
        // 上一次会话停在“成功 / 出错”的提示上：与热键一样，新会话先清掉它。
        if matches!(
            self.state(),
            OrchestratorState::Success | OrchestratorState::Error
        ) {
            self.cancel_locked();
        }
        self.begin_admitted_with(
            false,
            SessionPurpose::Assistant,
            None,
            SessionOrigin::Wake {
                no_speech_timeout_ms,
            },
        )
    }

    /// 呼叫琴音（热键、首页 / 聊天窗口的按钮）：按键版的语音唤醒，按一次就听。
    /// - 空闲（或停在成功 / 出错提示上）：开始一次唤醒式收音（说完自动提交，没开口就安静结束）；
    /// - 琴音正在听：立刻结束这句、交给她（不用等静音）；
    /// - 正在语音输入（另一用途）或在识别 / 发送：先取消旧的，不开始新的（与助手热键一致）。
    ///
    /// `ticket`：热键的输入票（过期的按键不生效）；按钮传 None。
    pub async fn call_assistant(&self, ticket: Option<u64>, no_speech_timeout_ms: u32) {
        if !self.input_ticket_is_current(ticket) {
            return;
        }
        let (state, purpose, gen, legacy_reminder) = self.hotkey_session_snapshot();
        match state {
            OrchestratorState::Idle | OrchestratorState::Success | OrchestratorState::Error => {
                if let Err(e) = self.begin_assistant_wake(no_speech_timeout_ms).await {
                    crate::log::log(&format!("呼叫琴音没有开始：{e}"));
                }
            }
            OrchestratorState::Listening
                if purpose == SessionPurpose::Assistant && !legacy_reminder =>
            {
                let _ = self.end_inner(Some(gen), ticket).await;
            }
            _ => self.cancel_inner(ticket, Some(gen)).await,
        }
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
