//! Kotone 后处理器实现 crate。
//!
//! 当前先提供零依赖 mock，用于验证注册发现和有序 pipeline；
//! 后续 AI 润色、翻译及在线/本地 API 适配器继续沿用同一注册入口。

use std::sync::Arc;

use kotone_core::postprocess::{ProcessorFactory, ProcessorRegistry};

pub mod mock;

pub fn builtin_processors() -> Vec<Arc<dyn ProcessorFactory>> {
    vec![Arc::new(mock::AppendExclamationFactory)]
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

    #[test]
    fn registry_discovers_all_builtin_processors_in_stable_order() {
        let mut registry = ProcessorRegistry::new();
        register_builtin(&mut registry).unwrap();
        let ids: Vec<String> = registry.list_info().into_iter().map(|i| i.id).collect();
        assert_eq!(ids, vec!["mock.append-exclamation".to_string()]);
    }
}
