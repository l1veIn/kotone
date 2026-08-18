//! 文本后处理端口与有序 pipeline runner。
//!
//! `kotone-core` 只定义领域契约、注册表和编排语义；具体处理器由外部 crate
//! 通过 `ProcessorFactory` 注册，依赖方向与 STT 的 EngineRegistry 保持一致。

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::Value;
use tokio::sync::Notify;

pub const DEFAULT_STEP_TIMEOUT_MS: u64 = 5_000;
pub const BUILTIN_BLOCKLIST_PIPELINE_ID: &str = "blocklist";
pub const BUILTIN_BLOCKLIST_PROCESSOR_ID: &str = "builtin.blocklist-filter";

fn default_pipeline_id() -> String {
    "default".into()
}

fn default_active_pipeline_id() -> String {
    BUILTIN_BLOCKLIST_PIPELINE_ID.into()
}

fn default_step_timeout_ms() -> u64 {
    DEFAULT_STEP_TIMEOUT_MS
}

fn default_enabled() -> bool {
    true
}

fn default_post_processing_enabled() -> bool {
    true
}

/// 用户配置中的后处理总开关、当前流程，以及可切换的多条流程。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PostProcessingConfig {
    #[serde(default = "default_post_processing_enabled")]
    pub enabled: bool,
    #[serde(default = "default_active_pipeline_id")]
    pub active_pipeline_id: String,
    #[serde(default = "default_pipelines")]
    pub pipelines: Vec<PipelineConfig>,
}

/// 一条线性 pipeline；step 数组顺序就是执行顺序。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineConfig {
    #[serde(default = "default_pipeline_id")]
    pub id: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub steps: Vec<PipelineStepConfig>,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            id: default_pipeline_id(),
            display_name: String::new(),
            steps: Vec::new(),
        }
    }
}

impl PipelineConfig {
    pub fn display_label(&self) -> String {
        let name = self.display_name.trim();
        if !name.is_empty() {
            return name.to_string();
        }
        if self.id == BUILTIN_BLOCKLIST_PIPELINE_ID {
            return "屏蔽词".into();
        }
        if self.id == "default" {
            return "自定义".into();
        }
        self.id.clone()
    }
}

pub fn builtin_blocklist_pipeline() -> PipelineConfig {
    PipelineConfig {
        id: BUILTIN_BLOCKLIST_PIPELINE_ID.into(),
        display_name: "屏蔽词".into(),
        steps: vec![PipelineStepConfig {
            id: "blocklist-1".into(),
            processor_id: BUILTIN_BLOCKLIST_PROCESSOR_ID.into(),
            enabled: true,
            config: serde_json::json!({}),
            timeout_ms: DEFAULT_STEP_TIMEOUT_MS,
            on_error: StepFailurePolicy::Required,
        }],
    }
}

fn default_pipelines() -> Vec<PipelineConfig> {
    vec![builtin_blocklist_pipeline()]
}

impl Default for PostProcessingConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            active_pipeline_id: default_active_pipeline_id(),
            pipelines: default_pipelines(),
        }
    }
}

impl PostProcessingConfig {
    /// 测试与旧调用点：把单条 pipeline 包成当前唯一流程。
    pub fn with_pipeline(enabled: bool, pipeline: PipelineConfig) -> Self {
        let mut pipeline = pipeline;
        if pipeline.display_name.trim().is_empty() {
            pipeline.display_name = pipeline.display_label();
        }
        Self {
            enabled,
            active_pipeline_id: pipeline.id.clone(),
            pipelines: vec![pipeline],
        }
    }

    pub fn active_pipeline(&self) -> Option<&PipelineConfig> {
        self.pipelines
            .iter()
            .find(|pipeline| pipeline.id == self.active_pipeline_id)
            .or(self.pipelines.first())
    }

    pub fn active_pipeline_mut(&mut self) -> Option<&mut PipelineConfig> {
        let id = self.active_pipeline()?.id.clone();
        self.pipelines.iter_mut().find(|pipeline| pipeline.id == id)
    }

    pub fn normalize_active_id(&mut self) {
        if self
            .pipelines
            .iter()
            .any(|pipeline| pipeline.id == self.active_pipeline_id)
        {
            return;
        }
        self.active_pipeline_id = self
            .pipelines
            .first()
            .map(|pipeline| pipeline.id.clone())
            .unwrap_or_else(default_active_pipeline_id);
        if self.pipelines.is_empty() {
            *self = Self::default();
        }
    }
}

/// 单个处理步骤。`processor_id` 指向注册表中的 factory，`config` 由 factory
/// 自行反序列化并校验，core 不需要认识不同模块的专有字段。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineStepConfig {
    pub id: String,
    pub processor_id: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub config: Value,
    #[serde(default = "default_step_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default)]
    pub on_error: StepFailurePolicy,
}

/// Required 保证语义不被静默绕过；BestEffort 失败时保留上一步文本继续。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StepFailurePolicy {
    #[default]
    Required,
    BestEffort,
}

/// pipeline 内部同时保留不可变识别原文与逐步演进的当前文本。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextDocument {
    pub source_text: String,
    pub text: String,
}

impl TextDocument {
    pub fn recognized(text: impl Into<String>) -> Self {
        let text = text.into();
        Self {
            source_text: text.clone(),
            text,
        }
    }
}

/// 每次 pipeline 的稳定上下文快照；处理器不能反向读取可变全局设置。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProcessingContext {
    pub session_id: String,
    pub profile_id: Option<String>,
    pub channel_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessorDescriptor {
    pub id: String,
    pub display_name: String,
    pub description: String,
    pub category: ProcessorCategory,
    pub developer_only: bool,
    pub network_access: NetworkAccess,
    pub config_fields: Vec<ProcessorConfigField>,
}

/// 处理器自行声明的简单配置字段。设置页据此生成通用表单，不需要认识具体模块 ID。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessorConfigField {
    pub key: String,
    pub display_name: String,
    pub description: String,
    pub kind: ProcessorConfigFieldKind,
    pub required: bool,
    pub file_extensions: Vec<String>,
    /// 输入框占位；留空时设置页用通用文案。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub placeholder: String,
    /// 一键填入的预设值；空 = 不展示模板选择。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub presets: Vec<ProcessorFieldPreset>,
    /// 连接字段可选用的 provider id；空 = 不限。翻译只应列出通义。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub compatible_providers: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessorFieldPreset {
    pub id: String,
    pub display_name: String,
    pub value: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProcessorConfigFieldKind {
    Text,
    File,
    /// 引用 Settings.connections 里的一条连接，step JSON 只存 connectionId。
    Connection,
}

/// 设置页用于分组展示处理器的稳定领域分类。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProcessorCategory {
    Writing,
    Translation,
    Utility,
}

/// 模块处理文本时可能触达的最远网络边界，供 UI 做隐私提示。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum NetworkAccess {
    None,
    Local,
    Internet,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessErrorKind {
    Cancelled,
    Failed,
    InvalidOutput,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessError {
    pub kind: ProcessErrorKind,
    pub message: String,
}

impl ProcessError {
    pub fn failed(message: impl Into<String>) -> Self {
        Self {
            kind: ProcessErrorKind::Failed,
            message: message.into(),
        }
    }
}

pub type ProcessFuture<'a> =
    Pin<Box<dyn Future<Output = Result<TextDocument, ProcessError>> + Send + 'a>>;

/// 一个已按配置构造完成、可执行的文本处理器实例。
pub trait TextProcessor: Send + Sync {
    fn process<'a>(
        &'a self,
        input: TextDocument,
        context: &'a ProcessingContext,
        cancel: ProcessingCancelToken,
    ) -> ProcessFuture<'a>;
}

/// 注册与发现的最小单元。一个 factory 可由不同 step config 构造多个实例。
pub trait ProcessorFactory: Send + Sync {
    fn descriptor(&self) -> ProcessorDescriptor;
    fn create(&self, config: &Value) -> Result<Arc<dyn TextProcessor>, String>;
}

#[derive(Default)]
pub struct ProcessorRegistry {
    factories: BTreeMap<String, Arc<dyn ProcessorFactory>>,
}

impl ProcessorRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// 重复 ID 直接拒绝，防止注册顺序悄悄改变实际实现。
    pub fn register(&mut self, factory: Arc<dyn ProcessorFactory>) -> Result<(), String> {
        let descriptor = factory.descriptor();
        if descriptor.id.trim().is_empty() {
            return Err("后处理器 ID 不能为空".into());
        }
        if self.factories.contains_key(&descriptor.id) {
            return Err(format!("重复注册后处理器: {}", descriptor.id));
        }
        self.factories.insert(descriptor.id, factory);
        Ok(())
    }

    pub fn get(&self, id: &str) -> Option<&Arc<dyn ProcessorFactory>> {
        self.factories.get(id)
    }

    /// 稳定按 ID 排序，供设置页发现可用模块。
    pub fn list_info(&self) -> Vec<ProcessorDescriptor> {
        self.factories.values().map(|f| f.descriptor()).collect()
    }
}

#[derive(Clone, Default)]
pub struct ProcessingCancelToken {
    inner: Arc<ProcessingCancelInner>,
}

#[derive(Default)]
struct ProcessingCancelInner {
    cancelled: AtomicBool,
    notify: Notify,
}

impl ProcessingCancelToken {
    pub fn cancel(&self) {
        if !self.inner.cancelled.swap(true, Ordering::SeqCst) {
            self.inner.notify.notify_waiters();
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::SeqCst)
    }

    pub async fn cancelled(&self) {
        if self.is_cancelled() {
            return;
        }
        self.inner.notify.notified().await;
    }
}

struct CompiledStep {
    config: PipelineStepConfig,
    descriptor: ProcessorDescriptor,
    processor: Arc<dyn TextProcessor>,
}

/// 配置在执行前一次性编译为处理器实例快照；运行中修改设置不会偷换本轮实现。
pub struct PostProcessPipeline {
    id: String,
    steps: Vec<CompiledStep>,
}

impl PostProcessPipeline {
    pub fn compile(
        config: &PostProcessingConfig,
        registry: &ProcessorRegistry,
    ) -> Result<Self, String> {
        let pipeline = config.active_pipeline().cloned().unwrap_or_default();
        let mut steps = Vec::new();
        if config.enabled {
            for step in pipeline.steps.iter().filter(|step| step.enabled) {
                if step.id.trim().is_empty() {
                    return Err("后处理 step ID 不能为空".into());
                }
                let factory = registry
                    .get(&step.processor_id)
                    .ok_or_else(|| format!("未注册的后处理器: {}", step.processor_id))?;
                let descriptor = factory.descriptor();
                let processor = factory
                    .create(&step.config)
                    .map_err(|error| format!("后处理步骤「{}」配置无效: {error}", step.id))?;
                steps.push(CompiledStep {
                    config: step.clone(),
                    descriptor,
                    processor,
                });
            }
        }
        Ok(Self {
            id: pipeline.id,
            steps,
        })
    }

    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    pub fn len(&self) -> usize {
        self.steps.len()
    }

    pub async fn run(
        &self,
        source_text: String,
        context: &ProcessingContext,
        cancel: ProcessingCancelToken,
    ) -> Result<PipelineResult, PipelineError> {
        self.run_with_progress(source_text, context, cancel, None)
            .await
    }

    /// 运行 pipeline，并在每个步骤开始前同步通知观察者。
    ///
    /// observer 只观察会话级执行快照，不拥有 runner，也不能改变步骤结果；调用方
    /// 可据此发出 UI 进度事件。同步回调保证事件顺序与实际步骤顺序严格一致。
    pub async fn run_with_progress(
        &self,
        source_text: String,
        context: &ProcessingContext,
        cancel: ProcessingCancelToken,
        observer: Option<&PipelineProgressObserver<'_>>,
    ) -> Result<PipelineResult, PipelineError> {
        let mut document = TextDocument::recognized(source_text);
        let mut trace = Vec::with_capacity(self.steps.len());

        for (step_index, step) in self.steps.iter().enumerate() {
            if cancel.is_cancelled() {
                return Err(PipelineError::cancelled(&step.config, trace));
            }
            if let Some(observer) = observer {
                observer(PipelineStepProgress {
                    step_id: step.config.id.clone(),
                    processor_id: step.config.processor_id.clone(),
                    display_name: step.descriptor.display_name.clone(),
                    index: step_index + 1,
                    total: self.steps.len(),
                });
            }
            let started = Instant::now();
            let step_cancel = cancel.clone();
            let run = step
                .processor
                .process(document.clone(), context, step_cancel);
            let timeout = Duration::from_millis(step.config.timeout_ms.max(1));
            let outcome = tokio::select! {
                _ = cancel.cancelled() => Err(ProcessError {
                    kind: ProcessErrorKind::Cancelled,
                    message: "后处理已取消".into(),
                }),
                result = tokio::time::timeout(timeout, run) => match result {
                    Ok(result) => result,
                    Err(_) => Err(ProcessError::failed(format!(
                        "处理超时（{}ms）",
                        step.config.timeout_ms.max(1)
                    ))),
                },
            };

            match outcome {
                Ok(next) if !next.text.trim().is_empty() => {
                    document = next;
                    trace.push(PipelineStepTrace {
                        step_id: step.config.id.clone(),
                        processor_id: step.config.processor_id.clone(),
                        display_name: step.descriptor.display_name.clone(),
                        duration_ms: started.elapsed().as_millis() as u64,
                        outcome: PipelineStepOutcome::Succeeded,
                        error: None,
                    });
                }
                Ok(_) => {
                    let error = ProcessError {
                        kind: ProcessErrorKind::InvalidOutput,
                        message: "处理器返回了空文本".into(),
                    };
                    if let Some(error) = handle_step_error(step, error, started, &mut trace) {
                        return Err(error);
                    }
                }
                Err(error) => {
                    if error.kind == ProcessErrorKind::Cancelled {
                        return Err(PipelineError::cancelled(&step.config, trace));
                    }
                    if let Some(error) = handle_step_error(step, error, started, &mut trace) {
                        return Err(error);
                    }
                }
            }
        }

        Ok(PipelineResult {
            pipeline_id: self.id.clone(),
            source_text: document.source_text,
            final_text: document.text,
            trace,
        })
    }
}

fn handle_step_error(
    step: &CompiledStep,
    error: ProcessError,
    started: Instant,
    trace: &mut Vec<PipelineStepTrace>,
) -> Option<PipelineError> {
    trace.push(PipelineStepTrace {
        step_id: step.config.id.clone(),
        processor_id: step.config.processor_id.clone(),
        display_name: step.descriptor.display_name.clone(),
        duration_ms: started.elapsed().as_millis() as u64,
        outcome: PipelineStepOutcome::Failed,
        error: Some(error.message.clone()),
    });
    (step.config.on_error == StepFailurePolicy::Required).then(|| PipelineError {
        step_id: step.config.id.clone(),
        processor_id: step.config.processor_id.clone(),
        kind: error.kind,
        message: error.message,
        trace: trace.clone(),
    })
}

/// pipeline 开始执行某一步时发出的只读进度快照。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineStepProgress {
    pub step_id: String,
    pub processor_id: String,
    pub display_name: String,
    /// 从 1 开始，适合直接展示为“第 n 步”。
    pub index: usize,
    pub total: usize,
}

pub type PipelineProgressObserver<'a> = dyn Fn(PipelineStepProgress) + Send + Sync + 'a;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipelineResult {
    pub pipeline_id: String,
    pub source_text: String,
    pub final_text: String,
    pub trace: Vec<PipelineStepTrace>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipelineError {
    pub step_id: String,
    pub processor_id: String,
    pub kind: ProcessErrorKind,
    pub message: String,
    pub trace: Vec<PipelineStepTrace>,
}

impl PipelineError {
    fn cancelled(step: &PipelineStepConfig, trace: Vec<PipelineStepTrace>) -> Self {
        Self {
            step_id: step.id.clone(),
            processor_id: step.processor_id.clone(),
            kind: ProcessErrorKind::Cancelled,
            message: "后处理已取消".into(),
            trace,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineStepTrace {
    pub step_id: String,
    pub processor_id: String,
    pub display_name: String,
    pub duration_ms: u64,
    pub outcome: PipelineStepOutcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PipelineStepOutcome {
    Succeeded,
    Failed,
}

/// 设置页“试跑”返回值。该路径只编译并执行给定 pipeline，不接触 orchestrator、
/// history 或 injector。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PostProcessingTestResult {
    pub source_text: String,
    pub final_text: String,
    pub pipeline_id: String,
    pub duration_ms: u64,
    pub steps: Vec<PipelineStepTrace>,
}

pub async fn test_post_processing(
    text: String,
    pipeline: PipelineConfig,
    registry: &ProcessorRegistry,
) -> Result<PostProcessingTestResult, String> {
    let compiled = PostProcessPipeline::compile(
        &PostProcessingConfig::with_pipeline(true, pipeline),
        registry,
    )?;
    let started = Instant::now();
    let result = compiled
        .run(
            text,
            &ProcessingContext::default(),
            ProcessingCancelToken::default(),
        )
        .await
        .map_err(|error| format!("后处理步骤「{}」失败：{}", error.step_id, error.message))?;
    Ok(PostProcessingTestResult {
        source_text: result.source_text,
        final_text: result.final_text,
        pipeline_id: result.pipeline_id,
        duration_ms: started.elapsed().as_millis() as u64,
        steps: result.trace,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_enabled_blocklist_pipeline() {
        let config = PostProcessingConfig::default();
        assert!(config.enabled);
        assert_eq!(config.active_pipeline_id, BUILTIN_BLOCKLIST_PIPELINE_ID);
        let pipeline = config.active_pipeline().unwrap();
        assert_eq!(pipeline.display_name, "屏蔽词");
        assert_eq!(
            pipeline.steps[0].processor_id,
            BUILTIN_BLOCKLIST_PROCESSOR_ID
        );
    }

    #[test]
    fn compile_uses_the_active_pipeline_not_the_first() {
        let mut registry = ProcessorRegistry::new();
        registry.register(Arc::new(NoopFactory)).unwrap();
        let config = PostProcessingConfig {
            enabled: true,
            active_pipeline_id: "second".into(),
            pipelines: vec![
                PipelineConfig {
                    id: "first".into(),
                    display_name: "一".into(),
                    steps: vec![],
                },
                PipelineConfig {
                    id: "second".into(),
                    display_name: "二".into(),
                    steps: vec![PipelineStepConfig {
                        id: "n1".into(),
                        processor_id: "test.noop".into(),
                        enabled: true,
                        config: Value::Null,
                        timeout_ms: 1_000,
                        on_error: StepFailurePolicy::Required,
                    }],
                },
            ],
        };
        let compiled = PostProcessPipeline::compile(&config, &registry).unwrap();
        assert_eq!(compiled.len(), 1);
    }

    struct NoopFactory;

    impl ProcessorFactory for NoopFactory {
        fn descriptor(&self) -> ProcessorDescriptor {
            ProcessorDescriptor {
                id: "test.noop".into(),
                display_name: "Noop".into(),
                description: String::new(),
                category: ProcessorCategory::Utility,
                developer_only: true,
                network_access: NetworkAccess::None,
                config_fields: Vec::new(),
            }
        }

        fn create(&self, _config: &Value) -> Result<Arc<dyn TextProcessor>, String> {
            Ok(Arc::new(Noop))
        }
    }

    struct Noop;

    impl TextProcessor for Noop {
        fn process<'a>(
            &'a self,
            input: TextDocument,
            _context: &'a ProcessingContext,
            _cancel: ProcessingCancelToken,
        ) -> ProcessFuture<'a> {
            Box::pin(async move { Ok(input) })
        }
    }
}
