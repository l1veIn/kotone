//! 用于第一阶段全链路验证的 mock 后处理器。

use std::sync::Arc;

use kotone_core::postprocess::{
    NetworkAccess, ProcessError, ProcessFuture, ProcessingCancelToken, ProcessingContext,
    ProcessorCategory, ProcessorDescriptor, ProcessorFactory, TextDocument, TextProcessor,
};
use serde_json::Value;

pub struct AppendExclamationFactory;

impl ProcessorFactory for AppendExclamationFactory {
    fn descriptor(&self) -> ProcessorDescriptor {
        ProcessorDescriptor {
            id: "mock.append-exclamation".into(),
            display_name: "Mock · 句尾叹号".into(),
            description: "在文本句尾追加一个全角叹号，用于验证后处理流程。".into(),
            category: ProcessorCategory::Utility,
            developer_only: true,
            network_access: NetworkAccess::None,
            config_fields: Vec::new(),
        }
    }

    fn create(&self, _config: &Value) -> Result<Arc<dyn TextProcessor>, String> {
        Ok(Arc::new(AppendExclamation))
    }
}

struct AppendExclamation;

impl TextProcessor for AppendExclamation {
    fn process<'a>(
        &'a self,
        mut input: TextDocument,
        _context: &'a ProcessingContext,
        cancel: ProcessingCancelToken,
    ) -> ProcessFuture<'a> {
        Box::pin(async move {
            if cancel.is_cancelled() {
                return Err(ProcessError::failed("后处理已取消"));
            }
            input.text.push('！');
            Ok(input)
        })
    }
}

pub struct WrapBracketsFactory;

impl ProcessorFactory for WrapBracketsFactory {
    fn descriptor(&self) -> ProcessorDescriptor {
        ProcessorDescriptor {
            id: "mock.wrap-brackets".into(),
            display_name: "Mock · 方括号包裹".into(),
            description: "使用全角方括号包裹文本，用于验证多步骤编排。".into(),
            category: ProcessorCategory::Utility,
            developer_only: true,
            network_access: NetworkAccess::None,
            config_fields: Vec::new(),
        }
    }

    fn create(&self, _config: &Value) -> Result<Arc<dyn TextProcessor>, String> {
        Ok(Arc::new(WrapBrackets))
    }
}

struct WrapBrackets;

impl TextProcessor for WrapBrackets {
    fn process<'a>(
        &'a self,
        mut input: TextDocument,
        _context: &'a ProcessingContext,
        cancel: ProcessingCancelToken,
    ) -> ProcessFuture<'a> {
        Box::pin(async move {
            if cancel.is_cancelled() {
                return Err(ProcessError::failed("后处理已取消"));
            }
            input.text = format!("【{}】", input.text);
            Ok(input)
        })
    }
}
