//! STT 可插拔多引擎架构：`SttEngine` trait + 引擎注册表
//! 设计契约见 docs/development.md §3.3 —— 流式支持是架构一等公民。
//!
//! 与 §3.3 伪代码的一处扩展：`start_session` 增加 `events` 通道参数，
//! partial/final 经 `tokio::sync::mpsc::UnboundedSender<SttEvent>` 外发，
//! 与文档「partial 结果通过事件通道外发」一致，只是把通道显式化。

pub mod mock;
pub mod sherpa;
pub mod whisper_sidecar;

use tokio::sync::mpsc;

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

/// IPC 用引擎信息（docs/development.md §5.3 list_stt_engines）
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineInfo {
    pub id: String,
    pub display_name: String,
    pub capabilities: EngineCapabilities,
    pub is_ready: bool,
}

/// 会话配置（采样率固定 16kHz mono f32，另有热词、引擎专有选项等）
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SessionConfig {
    pub language: String,
    pub hotwords: Vec<String>,
    /// 引擎专有配置项（如 whisper 线程数）
    #[serde(default)]
    pub options: serde_json::Value,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            language: "zh".into(),
            hotwords: Vec::new(),
            options: serde_json::Value::Null,
        }
    }
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
    /// 开始一次识别会话；partial/final 事件经 `events` 外发
    fn start_session(
        &self,
        cfg: &SessionConfig,
        events: mpsc::UnboundedSender<SttEvent>,
    ) -> Result<Box<dyn SttSession>, String>;
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

/// 引擎注册表：收集全部已编译引擎。
/// mock-stream 恒在（全链路联调用）；whisper/sherpa 占位注册（is_ready=false，未实现）。
/// 后续真实引擎按 cargo feature 控制是否进二进制（docs/development.md §3.3）。
pub struct EngineRegistry {
    engines: Vec<Box<dyn SttEngine>>,
}

impl EngineRegistry {
    pub fn new() -> Self {
        let mut engines: Vec<Box<dyn SttEngine>> = vec![
            Box::new(mock::MockStreamEngine),
            Box::new(whisper_sidecar::WhisperSidecarEngine),
            Box::new(sherpa::SherpaEngine),
        ];
        // 候选池引擎（funasr / cloud-asr 等）后续按 feature 追加
        engines.sort_by(|a, b| a.id().cmp(b.id()));
        Self { engines }
    }

    /// 按 ID 取引擎实例引用
    pub fn get(&self, id: &str) -> Option<&dyn SttEngine> {
        self.engines.iter().find(|e| e.id() == id).map(|e| &**e)
    }

    /// IPC：列出全部引擎信息
    pub fn list_info(&self) -> Vec<EngineInfo> {
        self.engines
            .iter()
            .map(|e| EngineInfo {
                id: e.id().to_string(),
                display_name: e.display_name().to_string(),
                capabilities: e.capabilities(),
                is_ready: e.is_ready(),
            })
            .collect()
    }
}

impl Default for EngineRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_contains_mock_and_placeholders() {
        let reg = EngineRegistry::new();
        let ids: Vec<String> = reg.list_info().iter().map(|i| i.id.clone()).collect();
        assert!(ids.contains(&"mock-stream".to_string()));
        assert!(ids.contains(&"whisper-cpp-sidecar".to_string()));
        assert!(ids.contains(&"sherpa-onnx-zipformer-zh".to_string()));
    }

    #[test]
    fn mock_is_ready_placeholders_are_not() {
        let reg = EngineRegistry::new();
        assert!(reg.get("mock-stream").unwrap().is_ready());
        assert!(!reg.get("whisper-cpp-sidecar").unwrap().is_ready());
        assert!(!reg.get("sherpa-onnx-zipformer-zh").unwrap().is_ready());
        assert!(reg.get("no-such-engine").is_none());
    }
}
