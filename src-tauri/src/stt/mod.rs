//! STT 可插拔多引擎架构：`SttEngine` trait + 引擎注册表
//! 设计契约见 docs/development.md §3.3 —— 流式支持是架构一等公民。

pub mod sherpa;
pub mod whisper_sidecar;

/// 引擎静态能力声明，UI 据此展示可用功能与提示
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EngineCapabilities {
    /// 是否支持 partial 流式结果
    pub streaming: bool,
    /// 是否支持热词表
    pub hotwords: bool,
    /// 是否可用 GPU 加速
    pub gpu: bool,
    /// 是否完全离线
    pub offline: bool,
    pub languages: Vec<String>,
}

/// 会话配置（采样率、热词、引擎专有选项等）
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct SessionConfig {
    pub language: String,
    pub hotwords: Vec<String>,
    /// 引擎专有配置项（如 whisper 线程数）
    pub options: serde_json::Value,
}

/// 识别结果
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Transcript {
    pub text: String,
    pub latency_ms: u32,
}

/// partial 结果通过事件通道外发，非流式引擎只发 Final
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum SttEvent {
    Partial { text: String },
    Final { text: String, latency_ms: u32 },
}

/// 一个引擎 = 一种 STT 策略（含其模型管理）
pub trait SttEngine: Send + Sync {
    /// 如 "whisper-cpp-sidecar"
    fn id(&self) -> &'static str;
    fn display_name(&self) -> &str;
    fn capabilities(&self) -> EngineCapabilities;
    /// 模型是否已下载/可用
    fn is_ready(&self) -> bool;
    fn start_session(&self, cfg: &SessionConfig) -> Result<Box<dyn SttSession>, String>;
}

/// 一次「按下到松手」的识别会话；流式与非流式引擎共用同一接口
pub trait SttSession: Send {
    /// 实时喂入 PCM（16kHz mono f32），流式引擎边收边识别
    fn push_audio(&mut self, pcm: &[f32]) -> Result<(), String>;
    /// 松手收尾，返回最终文本；流式引擎此时输出最终修正结果
    fn finalize(self: Box<Self>) -> Result<Transcript, String>;
    /// 取消（用户 Esc / 再按热键）
    fn cancel(&mut self);
}

/// 引擎注册表：按编译 feature 收集已启用引擎（占位实现）
pub fn registry() -> Vec<Box<dyn SttEngine>> {
    // TODO: feature-gated 注册 whisper-cpp-sidecar / sherpa-onnx 等引擎
    Vec::new()
}

/// 按 ID 取当前引擎（占位实现）
pub fn get_engine(_id: &str) -> Option<Box<dyn SttEngine>> {
    todo!("从注册表按 ID 查找引擎实例")
}
