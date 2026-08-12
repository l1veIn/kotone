//! Kotone 后处理器实现 crate。
//!
//! 当前先提供零依赖 mock，用于验证注册发现和有序 pipeline；
//! 后续 AI 润色、翻译及在线/本地 API 适配器继续沿用同一注册入口。

use std::sync::Arc;

use kotone_core::postprocess::{ProcessorFactory, ProcessorRegistry};

pub mod mock;

pub fn builtin_processors() -> Vec<Arc<dyn ProcessorFactory>> {
    vec![
        Arc::new(mock::AppendExclamationFactory),
        Arc::new(mock::WrapBracketsFactory),
    ]
}

pub fn register_builtin(registry: &mut ProcessorRegistry) -> Result<(), String> {
    for processor in builtin_processors() {
        registry.register(processor)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    use kotone_core::postprocess::{
        PipelineConfig, PipelineStepConfig, PostProcessPipeline, PostProcessingConfig,
        ProcessError, ProcessFuture, ProcessingCancelToken, ProcessingContext, ProcessorDescriptor,
        StepFailurePolicy, TextDocument, TextProcessor,
    };

    fn step(id: &str, processor_id: &str) -> PipelineStepConfig {
        PipelineStepConfig {
            id: id.into(),
            processor_id: processor_id.into(),
            enabled: true,
            config: serde_json::Value::Null,
            timeout_ms: 1_000,
            on_error: StepFailurePolicy::Required,
        }
    }

    struct AlwaysFailFactory;

    impl ProcessorFactory for AlwaysFailFactory {
        fn descriptor(&self) -> ProcessorDescriptor {
            ProcessorDescriptor {
                id: "test.always-fail".into(),
                display_name: "Test · 恒失败".into(),
            }
        }

        fn create(&self, _config: &serde_json::Value) -> Result<Arc<dyn TextProcessor>, String> {
            Ok(Arc::new(AlwaysFail))
        }
    }

    struct AlwaysFail;

    impl TextProcessor for AlwaysFail {
        fn process<'a>(
            &'a self,
            _input: TextDocument,
            _context: &'a ProcessingContext,
            _cancel: ProcessingCancelToken,
        ) -> ProcessFuture<'a> {
            Box::pin(async { Err(ProcessError::failed("预期失败")) })
        }
    }

    struct SlowFactory;

    impl ProcessorFactory for SlowFactory {
        fn descriptor(&self) -> ProcessorDescriptor {
            ProcessorDescriptor {
                id: "test.slow".into(),
                display_name: "Test · 慢处理".into(),
            }
        }

        fn create(&self, _config: &serde_json::Value) -> Result<Arc<dyn TextProcessor>, String> {
            Ok(Arc::new(Slow))
        }
    }

    struct Slow;

    impl TextProcessor for Slow {
        fn process<'a>(
            &'a self,
            input: TextDocument,
            _context: &'a ProcessingContext,
            _cancel: ProcessingCancelToken,
        ) -> ProcessFuture<'a> {
            Box::pin(async move {
                tokio::time::sleep(Duration::from_secs(30)).await;
                Ok(input)
            })
        }
    }

    fn pipeline_with(
        steps: Vec<PipelineStepConfig>,
        registry: &ProcessorRegistry,
    ) -> PostProcessPipeline {
        PostProcessPipeline::compile(
            &PostProcessingConfig {
                enabled: true,
                pipeline: PipelineConfig {
                    id: "test".into(),
                    steps,
                },
            },
            registry,
        )
        .unwrap()
    }

    #[test]
    fn registry_discovers_all_builtin_processors_in_stable_order() {
        let mut registry = ProcessorRegistry::new();
        register_builtin(&mut registry).unwrap();
        let ids: Vec<String> = registry.list_info().into_iter().map(|i| i.id).collect();
        assert_eq!(
            ids,
            vec![
                "mock.append-exclamation".to_string(),
                "mock.wrap-brackets".to_string(),
            ]
        );
    }

    #[test]
    fn duplicate_registration_is_rejected_instead_of_overwriting() {
        let mut registry = ProcessorRegistry::new();
        registry
            .register(Arc::new(mock::AppendExclamationFactory))
            .unwrap();
        let error = registry
            .register(Arc::new(mock::AppendExclamationFactory))
            .unwrap_err();
        assert!(error.contains("重复注册"));
    }

    #[tokio::test]
    async fn pipeline_resolves_registered_processors_and_preserves_step_order() {
        let mut registry = ProcessorRegistry::new();
        register_builtin(&mut registry).unwrap();
        let config = PostProcessingConfig {
            enabled: true,
            pipeline: PipelineConfig {
                id: "ordered".into(),
                steps: vec![
                    step("punctuation", "mock.append-exclamation"),
                    step("wrapper", "mock.wrap-brackets"),
                ],
            },
        };
        let pipeline = PostProcessPipeline::compile(&config, &registry).unwrap();

        let result = pipeline
            .run(
                "你好".into(),
                &ProcessingContext::default(),
                ProcessingCancelToken::default(),
            )
            .await
            .unwrap();

        assert_eq!(result.source_text, "你好");
        assert_eq!(result.final_text, "【你好！】");
        assert_eq!(
            result
                .trace
                .iter()
                .map(|step| step.processor_id.as_str())
                .collect::<Vec<_>>(),
            vec!["mock.append-exclamation", "mock.wrap-brackets"]
        );
    }

    #[tokio::test]
    async fn best_effort_failure_keeps_last_good_text_and_continues() {
        let mut registry = ProcessorRegistry::new();
        register_builtin(&mut registry).unwrap();
        registry.register(Arc::new(AlwaysFailFactory)).unwrap();
        let mut failing = step("optional", "test.always-fail");
        failing.on_error = StepFailurePolicy::BestEffort;
        let pipeline = pipeline_with(
            vec![failing, step("punctuation", "mock.append-exclamation")],
            &registry,
        );

        let result = pipeline
            .run(
                "你好".into(),
                &ProcessingContext::default(),
                ProcessingCancelToken::default(),
            )
            .await
            .unwrap();

        assert_eq!(result.final_text, "你好！");
        assert_eq!(result.trace.len(), 2);
        assert_eq!(result.trace[0].error.as_deref(), Some("预期失败"));
    }

    #[tokio::test]
    async fn required_failure_stops_before_following_steps() {
        let mut registry = ProcessorRegistry::new();
        register_builtin(&mut registry).unwrap();
        registry.register(Arc::new(AlwaysFailFactory)).unwrap();
        let pipeline = pipeline_with(
            vec![
                step("required", "test.always-fail"),
                step("must-not-run", "mock.append-exclamation"),
            ],
            &registry,
        );

        let error = pipeline
            .run(
                "你好".into(),
                &ProcessingContext::default(),
                ProcessingCancelToken::default(),
            )
            .await
            .unwrap_err();

        assert_eq!(error.step_id, "required");
        assert_eq!(error.trace.len(), 1);
    }

    #[tokio::test]
    async fn timeout_and_cancellation_drop_slow_processor_future() {
        let mut registry = ProcessorRegistry::new();
        registry.register(Arc::new(SlowFactory)).unwrap();

        let mut timeout_step = step("slow", "test.slow");
        timeout_step.timeout_ms = 5;
        let pipeline = pipeline_with(vec![timeout_step], &registry);
        let error = pipeline
            .run(
                "你好".into(),
                &ProcessingContext::default(),
                ProcessingCancelToken::default(),
            )
            .await
            .unwrap_err();
        assert!(error.message.contains("超时"));

        let pipeline = pipeline_with(vec![step("slow", "test.slow")], &registry);
        let cancel = ProcessingCancelToken::default();
        let trigger = cancel.clone();
        tokio::spawn(async move {
            tokio::task::yield_now().await;
            trigger.cancel();
        });
        let error = pipeline
            .run("你好".into(), &ProcessingContext::default(), cancel)
            .await
            .unwrap_err();
        assert_eq!(
            error.kind,
            kotone_core::postprocess::ProcessErrorKind::Cancelled
        );
    }
}
