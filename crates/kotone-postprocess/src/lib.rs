//! Kotone 后处理器实现 crate。
//!
//! 内置本地工具与开发态 mock 共用同一注册入口；后续 AI 润色、翻译及在线/本地 API
//! 适配器继续沿用这套发现与配置字段契约。

use std::sync::Arc;

use kotone_core::postprocess::{ProcessorFactory, ProcessorRegistry};

pub mod blocklist;
pub mod connections;
pub mod mock;
pub mod secrets;

pub fn builtin_processors() -> Vec<Arc<dyn ProcessorFactory>> {
    vec![
        Arc::new(blocklist::BlocklistFilterFactory),
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
        test_post_processing, NetworkAccess, PipelineConfig, PipelineStepConfig,
        PipelineStepProgress, PostProcessPipeline, PostProcessingConfig, ProcessError,
        ProcessFuture, ProcessingCancelToken, ProcessingContext, ProcessorCategory,
        ProcessorDescriptor, StepFailurePolicy, TextDocument, TextProcessor,
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
                description: "测试失败策略".into(),
                category: ProcessorCategory::Utility,
                developer_only: true,
                network_access: NetworkAccess::None,
                config_fields: Vec::new(),
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
                description: "测试超时与取消".into(),
                category: ProcessorCategory::Utility,
                developer_only: true,
                network_access: NetworkAccess::None,
                config_fields: Vec::new(),
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
                "builtin.blocklist-filter".to_string(),
                "mock.append-exclamation".to_string(),
                "mock.wrap-brackets".to_string(),
            ]
        );
    }

    #[test]
    fn descriptors_expose_ui_metadata_with_camel_case_fields() {
        let descriptor = mock::AppendExclamationFactory.descriptor();
        let value = serde_json::to_value(descriptor).unwrap();
        assert_eq!(value["displayName"], "Mock · 句尾叹号");
        assert_eq!(value["category"], "utility");
        assert_eq!(value["developerOnly"], true);
        assert_eq!(value["networkAccess"], "none");
        assert!(value.get("display_name").is_none());
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
    async fn progress_observer_reports_each_step_before_execution() {
        let mut registry = ProcessorRegistry::new();
        register_builtin(&mut registry).unwrap();
        let pipeline = pipeline_with(
            vec![
                step("punctuation", "mock.append-exclamation"),
                step("wrapper", "mock.wrap-brackets"),
            ],
            &registry,
        );
        let observed = std::sync::Mutex::new(Vec::<PipelineStepProgress>::new());
        let observer = |progress| observed.lock().unwrap().push(progress);

        pipeline
            .run_with_progress(
                "你好".into(),
                &ProcessingContext::default(),
                ProcessingCancelToken::default(),
                Some(&observer),
            )
            .await
            .unwrap();

        let observed = observed.into_inner().unwrap();
        assert_eq!(observed.len(), 2);
        assert_eq!(observed[0].display_name, "Mock · 句尾叹号");
        assert_eq!((observed[0].index, observed[0].total), (1, 2));
        assert_eq!(observed[1].step_id, "wrapper");
        assert_eq!((observed[1].index, observed[1].total), (2, 2));
    }

    #[tokio::test]
    async fn test_api_runs_pipeline_without_runtime_side_effects() {
        let mut registry = ProcessorRegistry::new();
        register_builtin(&mut registry).unwrap();
        let pipeline = PipelineConfig {
            id: "tryout".into(),
            steps: vec![
                step("punctuation", "mock.append-exclamation"),
                step("wrapper", "mock.wrap-brackets"),
            ],
        };

        let result = test_post_processing("你好".into(), pipeline, &registry)
            .await
            .unwrap();

        assert_eq!(result.source_text, "你好");
        assert_eq!(result.final_text, "【你好！】");
        assert_eq!(result.pipeline_id, "tryout");
        assert_eq!(result.steps.len(), 2);
        let value = serde_json::to_value(result).unwrap();
        assert_eq!(value["steps"][0]["displayName"], "Mock · 句尾叹号");
        assert_eq!(value["steps"][0]["outcome"], "succeeded");
        assert!(value["steps"][0].get("error").is_none());
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
