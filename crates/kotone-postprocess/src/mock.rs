//! 用于第一阶段全链路验证的 mock 后处理器。

use std::sync::Arc;

use kotone_core::postprocess::{
    ProcessError, ProcessFuture, ProcessingCancelToken, ProcessingContext, ProcessorDescriptor,
    ProcessorFactory, TextDocument, TextProcessor,
};
use serde_json::Value;

pub struct AppendExclamationFactory;

impl ProcessorFactory for AppendExclamationFactory {
    fn descriptor(&self) -> ProcessorDescriptor {
        ProcessorDescriptor {
            id: "mock.append-exclamation".into(),
            display_name: "Mock · 句尾叹号".into(),
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
