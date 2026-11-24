use super::*;

impl Orchestrator {
    /// 结束会话：finalize → autoSend 分流（§6 发送时序上半段）
    pub async fn end(&self) -> Result<(), String> {
        self.end_inner(None, None).await
    }

    /// 自动结束的代际校验与 Listening -> Transcribing 必须在同一 op 临界区。
    pub(super) async fn end_inner(
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
                    let audio_ms = self.heard_audio_ms();
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
                        inner.voice.discard_record();
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
                // 识别记录补齐 finalize 产物、历史音频独立落盘（语音输入）
                self.on_transcribed(t.latency_ms as u64);
                // eval 录档落盘（wav + partial 时间线 + 指标）；失败静默记日志不阻断流程
                if let Some(rec) = active.recorder.take() {
                    match rec.finish(&t.text, t.latency_ms as u64) {
                        Ok(saved) => {
                            crate::log::log(&format!("eval 录档完成: {}", saved.session_id))
                        }
                        Err(e) => crate::log::log(&format!("eval 录档失败（忽略）: {e}")),
                    }
                }
                // 听到了一句：交给语音输入（文字处理，然后预览或发送）
                self.on_heard(gen, t.text).await?;
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
}
