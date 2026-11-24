//! 文字处理（屏蔽词、润色、翻译的流程）和可交付文字的唯一出口（预览或发送）。

use kotone_core::interaction::PostFinalize;
use kotone_core::orchestrator::{
    Orchestrator, OrchestratorState, PendingText, ReadyText, RecognizedText,
};
use kotone_core::postprocess::{
    PostProcessPipeline, ProcessErrorKind, ProcessingCancelToken, ProcessingContext,
};
use serde_json::json;

use crate::VoiceInput;

impl VoiceInput {
    pub(crate) fn snapshot_recognized_text(
        &self,
        orch: &Orchestrator,
        source_text: String,
    ) -> RecognizedText {
        let settings = orch.settings().read().unwrap().clone();
        let session_id = orch
            .session()
            .process_case_id()
            .unwrap_or_default()
            .to_string();
        let channel_id = self.state().channel.as_ref().map(|(_, id)| id.clone());
        RecognizedText {
            source_text,
            pipeline: settings.post_processing,
            context: ProcessingContext {
                session_id,
                profile_id: settings.active_profile_id,
                channel_id,
            },
        }
    }

    /// 编译并运行一份会话级 pipeline 快照。只有非空 pipeline 才进入 Processing；
    /// Required 错误保留 RecognizedText，confirm_send 会重新运行处理而不会绕过。
    pub(crate) async fn run_postprocessing(
        &self,
        orch: &Orchestrator,
        gen: u64,
        recognized: RecognizedText,
        expected_states: &[OrchestratorState],
    ) -> Result<(), String> {
        let pipeline = match PostProcessPipeline::compile(&recognized.pipeline, &self.processors) {
            Ok(pipeline) => pipeline,
            Err(error) => {
                self.finish_processing_error(orch, gen, recognized, &error, expected_states)
                    .await;
                return Err(error);
            }
        };

        if pipeline.is_empty() {
            return self
                .deliver_ready_text(orch, gen, ReadyText::unchanged(recognized.source_text))
                .await;
        }

        let cancel = ProcessingCancelToken::default();
        {
            let _op = orch.lock_op().await;
            let mut session = orch.session();
            if session.gen() != gen || !expected_states.contains(&session.state()) {
                return Ok(());
            }
            session.set_state(OrchestratorState::Processing);
            session.set_pending_text(Some(PendingText::Recognized(recognized.clone())));
            session.set_processing_cancel(Some(cancel.clone()));
        }
        orch.emit_state(
            OrchestratorState::Processing,
            Some(json!({
                "text": recognized.source_text,
                "pipelineId": recognized.pipeline.active_pipeline().map(|p| p.id.clone()).unwrap_or_default(),
                "stepCount": pipeline.len(),
            })),
        );
        orch.emit_process(
            "postprocess_started",
            json!({
                "pipelineId": recognized.pipeline.active_pipeline().map(|p| p.id.clone()).unwrap_or_default(),
                "stepCount": pipeline.len() as u64,
            }),
        );

        let started = std::time::Instant::now();
        let progress_observer = |progress: kotone_core::postprocess::PipelineStepProgress| {
            orch.emit_state(
                OrchestratorState::Processing,
                Some(json!({
                    "text": recognized.source_text,
                    "pipelineId": recognized.pipeline.active_pipeline().map(|p| p.id.clone()).unwrap_or_default(),
                    "stepId": progress.step_id,
                    "processorId": progress.processor_id,
                    "displayName": progress.display_name,
                    "index": progress.index,
                    "total": progress.total,
                })),
            );
        };
        let result = pipeline
            .run_with_progress(
                recognized.source_text.clone(),
                &recognized.context,
                cancel,
                Some(&progress_observer),
            )
            .await;

        {
            let _op = orch.lock_op().await;
            let mut session = orch.session();
            if session.gen() != gen || session.state() != OrchestratorState::Processing {
                return Ok(());
            }
            session.set_processing_cancel(None);
        }

        match result {
            Ok(result) => {
                let duration_ms = started.elapsed().as_millis() as u64;
                if let Some(h) = &self.state().history {
                    h.lock().unwrap().process_duration_ms = Some(duration_ms);
                }
                orch.emit_process(
                    "postprocess_completed",
                    json!({
                        "pipelineId": result.pipeline_id,
                        "stepCount": result.trace.len() as u64,
                        "durationMs": duration_ms,
                        "sourceChars": result.source_text.chars().count() as u64,
                        "finalChars": result.final_text.chars().count() as u64,
                    }),
                );
                self.deliver_ready_text(
                    orch,
                    gen,
                    ReadyText {
                        source_text: result.source_text,
                        text: result.final_text,
                    },
                )
                .await
            }
            Err(error) if error.kind == ProcessErrorKind::Cancelled => Ok(()),
            Err(error) => {
                let message = format!("后处理步骤「{}」失败：{}", error.step_id, error.message);
                self.finish_processing_error(
                    orch,
                    gen,
                    recognized,
                    &message,
                    &[OrchestratorState::Processing],
                )
                .await;
                Err(message)
            }
        }
    }

    async fn finish_processing_error(
        &self,
        orch: &Orchestrator,
        gen: u64,
        recognized: RecognizedText,
        message: &str,
        expected_states: &[OrchestratorState],
    ) {
        {
            let _op = orch.lock_op().await;
            let mut session = orch.session();
            if session.gen() != gen || !expected_states.contains(&session.state()) {
                return;
            }
            session.set_processing_cancel(None);
            session.set_pending_text(Some(PendingText::Recognized(recognized.clone())));
            session.set_state(OrchestratorState::Error);
        }
        orch.emit_state(
            OrchestratorState::Error,
            Some(json!({
                "message": message,
                "text": recognized.source_text,
                "retryStage": "processing",
            })),
        );
        orch.emit_process(
            "postprocess_failed",
            json!({ "outcome": "error", "errorCode": "POSTPROCESS_FAILED" }),
        );
        self.write_history(
            orch,
            kotone_core::history::HistoryOutcome::Error,
            Some(message.to_string()),
        );
    }

    /// 所有可交付文本的唯一出口：更新 UI/history 后，按交互策略进入预览或发送。
    /// STT、未来后处理和处理失败重试都必须先产出 ReadyText，再调用本方法；
    /// injector 与 Preview 因而永远不会接触「尚未处理完成」的识别原文。
    async fn deliver_ready_text(
        &self,
        orch: &Orchestrator,
        gen: u64,
        ready: ReadyText,
    ) -> Result<(), String> {
        if orch.session().gen() != gen {
            return Ok(());
        }

        orch.emitter()
            .emit("kotone://partial", json!({ "text": ready.text }));
        if let Some(history) = &self.state().history {
            history.lock().unwrap().final_text = ready.text.clone();
        }

        match orch.policy().post {
            PostFinalize::SendDirect => {
                if let Some(claim) = self
                    .claim_send(
                        orch,
                        Some(ready),
                        Some(gen),
                        &[
                            OrchestratorState::Transcribing,
                            OrchestratorState::Processing,
                        ],
                        false,
                        None,
                    )
                    .await?
                {
                    self.run_send(orch, claim).await;
                }
            }
            PostFinalize::PreviewConfirm => {
                let _op = orch.lock_op().await;
                let mut session = orch.session();
                if session.gen() != gen
                    || ![
                        OrchestratorState::Transcribing,
                        OrchestratorState::Processing,
                    ]
                    .contains(&session.state())
                {
                    return Ok(());
                }
                let text = ready.text.clone();
                session.set_pending_text(Some(PendingText::Ready(ready)));
                session.set_state(OrchestratorState::Preview);
                drop(session);
                orch.emit_state(OrchestratorState::Preview, Some(json!({ "text": text })));
            }
        }
        Ok(())
    }
}
