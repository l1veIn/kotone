use super::text::{PendingText, ReadyText, RecognizedText};
use crate::orchestrator::*;
use crate::postprocess::{
    PostProcessPipeline, ProcessErrorKind, ProcessingContext, ProcessorRegistry,
};

impl Orchestrator {
    /// 注入可发现的后处理器注册表；与 STT 引擎一样由外层实现 crate 负责注册。
    pub fn with_processors(mut self, processors: Arc<ProcessorRegistry>) -> Self {
        self.voice_input.processors = processors;
        self
    }

    pub(super) fn snapshot_recognized_text(&self, source_text: String) -> RecognizedText {
        let settings = self.settings.read().unwrap().clone();
        let inner = self.inner.lock().unwrap();
        RecognizedText {
            source_text,
            pipeline: settings.post_processing,
            context: ProcessingContext {
                session_id: inner.process_case_id.clone().unwrap_or_default(),
                profile_id: settings.active_profile_id,
                channel_id: inner.voice.channel.as_ref().map(|(_, id)| id.clone()),
            },
        }
    }

    /// 编译并运行一份会话级 pipeline 快照。只有非空 pipeline 才进入 Processing；
    /// Required 错误保留 RecognizedText，confirm_send 会重新运行处理而不会绕过。
    pub(super) async fn run_postprocessing(
        &self,
        gen: u64,
        recognized: RecognizedText,
        expected_states: &[OrchestratorState],
    ) -> Result<(), String> {
        let pipeline = match PostProcessPipeline::compile(
            &recognized.pipeline,
            &self.voice_input.processors,
        ) {
            Ok(pipeline) => pipeline,
            Err(error) => {
                self.finish_processing_error(gen, recognized, &error, expected_states)
                    .await;
                return Err(error);
            }
        };

        if pipeline.is_empty() {
            return self
                .deliver_ready_text(gen, ReadyText::unchanged(recognized.source_text))
                .await;
        }

        let cancel = ProcessingCancelToken::default();
        {
            let _op = self.op.lock().await;
            let mut inner = self.inner.lock().unwrap();
            if inner.gen != gen || !expected_states.contains(&inner.state) {
                return Ok(());
            }
            inner.state = OrchestratorState::Processing;
            inner.pending_text = Some(PendingText::Recognized(recognized.clone()));
            inner.processing_cancel = Some(cancel.clone());
        }
        self.emit_state(
            OrchestratorState::Processing,
            Some(json!({
                "text": recognized.source_text,
                "pipelineId": recognized.pipeline.active_pipeline().map(|p| p.id.clone()).unwrap_or_default(),
                "stepCount": pipeline.len(),
            })),
        );
        self.emit_process(
            "postprocess_started",
            json!({
                "pipelineId": recognized.pipeline.active_pipeline().map(|p| p.id.clone()).unwrap_or_default(),
                "stepCount": pipeline.len() as u64,
            }),
        );

        let started = std::time::Instant::now();
        let progress_observer = |progress: crate::postprocess::PipelineStepProgress| {
            self.emit_state(
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
            let _op = self.op.lock().await;
            let mut inner = self.inner.lock().unwrap();
            if inner.gen != gen || inner.state != OrchestratorState::Processing {
                return Ok(());
            }
            inner.processing_cancel = None;
        }

        match result {
            Ok(result) => {
                let duration_ms = started.elapsed().as_millis() as u64;
                if let Some(h) = &self.inner.lock().unwrap().voice.history {
                    h.lock().unwrap().process_duration_ms = Some(duration_ms);
                }
                self.emit_process(
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
        gen: u64,
        recognized: RecognizedText,
        message: &str,
        expected_states: &[OrchestratorState],
    ) {
        {
            let _op = self.op.lock().await;
            let mut inner = self.inner.lock().unwrap();
            if inner.gen != gen || !expected_states.contains(&inner.state) {
                return;
            }
            inner.processing_cancel = None;
            inner.pending_text = Some(PendingText::Recognized(recognized.clone()));
            inner.state = OrchestratorState::Error;
        }
        self.emit_state(
            OrchestratorState::Error,
            Some(json!({
                "message": message,
                "text": recognized.source_text,
                "retryStage": "processing",
            })),
        );
        self.emit_process(
            "postprocess_failed",
            json!({ "outcome": "error", "errorCode": "POSTPROCESS_FAILED" }),
        );
        self.write_history(
            crate::history::HistoryOutcome::Error,
            Some(message.to_string()),
        );
    }

    /// 所有可交付文本的唯一出口：更新 UI/history 后，按交互策略进入预览或发送。
    /// STT、未来后处理和处理失败重试都必须先产出 ReadyText，再调用本方法；
    /// injector 与 Preview 因而永远不会接触「尚未处理完成」的识别原文。
    async fn deliver_ready_text(&self, gen: u64, ready: ReadyText) -> Result<(), String> {
        if self.inner.lock().unwrap().gen != gen {
            return Ok(());
        }

        self.emitter
            .emit("kotone://partial", json!({ "text": ready.text }));
        if let Some(history) = &self.inner.lock().unwrap().voice.history {
            history.lock().unwrap().final_text = ready.text.clone();
        }

        match self.policy().post {
            PostFinalize::SendDirect => {
                if let Some(claim) = self
                    .claim_send(
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
                    self.run_send(claim).await;
                }
            }
            PostFinalize::PreviewConfirm => {
                let _op = self.op.lock().await;
                let mut inner = self.inner.lock().unwrap();
                if inner.gen != gen
                    || ![
                        OrchestratorState::Transcribing,
                        OrchestratorState::Processing,
                    ]
                    .contains(&inner.state)
                {
                    return Ok(());
                }
                let text = ready.text.clone();
                inner.pending_text = Some(PendingText::Ready(ready));
                inner.state = OrchestratorState::Preview;
                drop(inner);
                self.emit_state(OrchestratorState::Preview, Some(json!({ "text": text })));
            }
        }
        Ok(())
    }
}
