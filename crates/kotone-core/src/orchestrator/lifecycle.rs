use super::*;

impl Orchestrator {
    /// Preview 状态下用户确认发送；Error 状态（payload 带文本）也接受，
    /// 重新进入 Sending 实现重试（docs/development.md §4.1「Error 保留文本可重试」）。
    /// ADR-006 预览只读化：不再接受编辑文本，一律发送已定的可交付文本。
    pub async fn confirm_send(&self) -> Result<(), String> {
        self.confirm_send_inner(None, None).await
    }

    pub(super) async fn confirm_send_inner(
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
        // 听写：交给语音输入（预览后发送、出错后重试处理或发送）
        self.confirm_heard(confirmation_gen, input_ticket).await
    }

    /// 录音设备故障等会话中止：与 cancel 相同清理，但以 Error toast 告知原因
    /// （P1-⑤：拔麦克风/驱动错误不再永久卡 Listening）。
    /// Error 保持到用户关闭/重试，确保按需悬浮窗也能让用户看到核心链路故障。
    pub async fn abort_with_message(&self, message: &str) {
        self.abort_with_code(message, "AUDIO_DEVICE_ERROR", None)
            .await;
    }

    /// 会话泵的致命错误统一中止入口：清理录音/STT/发送句柄并转入可见 Error。
    pub(super) async fn abort_with_code(
        &self,
        message: &str,
        error_code: &str,
        expected_gen: Option<u64>,
    ) {
        if expected_gen.is_none() {
            self.abort_release_grace();
        }
        let _op = self.op.lock().await;
        let gen;
        {
            let mut inner = self.inner.lock().unwrap();
            if inner.state == OrchestratorState::Idle
                || expected_gen.is_some_and(|expected| inner.gen != expected)
            {
                return;
            }
            // 过期采集错误不能收回新一轮的松键宽限计时。
            if let Some(task) = inner.release_grace_task.take() {
                task.abort();
            }
            inner.gen += 1;
            gen = inner.gen;
            let mut active = inner.active.take();
            inner.pending_text = None;
            inner.stop_send();
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
        self.on_session_ended(crate::history::HistoryOutcome::Cancelled, None);
        self.schedule_idle(gen);
    }

    /// 任意状态取消：回到 Idle（session cancel；发送中置取消令牌）
    pub async fn cancel(&self) {
        self.cancel_inner(None, None).await;
    }

    pub(super) async fn cancel_inner(&self, input_ticket: Option<u64>, expected_gen: Option<u64>) {
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
    pub(super) fn cancel_locked(&self) {
        // 宽限期倒计时一并收回（Esc / 停止 runtime / 状态变迁兜底）
        self.abort_release_grace();
        let mut inner = self.inner.lock().unwrap();
        if let Some(trial) = &inner.trial {
            trial.cancel();
        }
        if inner.state == OrchestratorState::Idle {
            // 逐句模式两段之间的 Idle 窗口也必须撤销尚未入场的续录。
            inner.gen += 1;
            return;
        }
        inner.gen += 1;
        let mut active = inner.active.take();
        inner.pending_text = None;
        inner.stop_send();
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
        self.on_session_ended(crate::history::HistoryOutcome::Cancelled, None);
    }
}
