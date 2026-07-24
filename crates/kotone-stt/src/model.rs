//! 模型管理：各引擎模型的下载 / 校验 / 切换（docs/development.md §3.3、§5.1）
//! 计划依赖 reqwest + sha2；安装包不含任何模型，首启引导下载。

/// 模型信息（跨引擎统一列出：已下载/可下载）
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ModelInfo {
    pub id: String,
    pub engine_id: String,
    pub display_name: String,
    pub size_bytes: u64,
    pub download_url: String,
    pub sha256: String,
    pub downloaded: bool,
}

/// 列出全部模型
/// TODO(model 子代理)：汇总各引擎的模型声明 + 本地就绪状态
pub fn list() -> Result<Vec<ModelInfo>, String> {
    Err("模型列表未实现".into())
}

/// 下载模型，进度经 "kotone://download" 事件外发
/// TODO(model 子代理)：reqwest 流式下载 + sha256 校验 + 断点续传
pub fn download(_model_id: &str) -> Result<(), String> {
    Err("模型下载未实现".into())
}

/// 切换引擎的活动模型
/// TODO(model 子代理)：写入配置并通知引擎重载
pub fn set_active(_engine_id: &str, _model_id: &str) -> Result<(), String> {
    Err("模型切换未实现".into())
}
