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

/// 列出全部模型（占位实现）
pub fn list() -> Vec<ModelInfo> {
    todo!("汇总各引擎的模型声明 + 本地就绪状态")
}

/// 下载模型，进度经 "kotone://download" 事件外发（占位实现）
pub fn download(_model_id: &str) -> Result<(), String> {
    todo!("reqwest 流式下载 + sha256 校验 + 断点续传")
}

/// 切换引擎的活动模型（占位实现）
pub fn set_active(_engine_id: &str, _model_id: &str) -> Result<(), String> {
    todo!("写入配置并通知引擎重载")
}
