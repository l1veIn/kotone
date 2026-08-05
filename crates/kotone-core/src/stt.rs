//! STT 可插拔多引擎架构（core 端口）：`SttEngine`/`SttSession` trait + 引擎注册表容器
//! 设计契约见 docs/development.md §3.3 —— 流式支持是架构一等公民。
//!
//! 与 §3.3 伪代码的一处扩展：`start_session` 增加 `events` 通道参数，
//! partial/final 经 `tokio::sync::mpsc::UnboundedSender<SttEvent>` 外发，
//! 与文档「partial 结果通过事件通道外发」一致，只是把通道显式化。
//!
//! 依赖方向：orchestrator（core）依赖本注册表容器 → 引擎实例必须由外部注入，
//! 否则 core 会与引擎实现 crate 形成循环依赖。内置引擎（mock/sherpa 系）
//! 在 kotone-stt，经 `kotone_stt::register_builtin` 注入。

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
    /// 热词命中加分（X-ASR 等引擎 recognizer 级配置；默认 3.5，
    /// 真源在 settings::default_hotwords_score）。越高热词越易命中，
    /// 也越容易把背景噪声识别成热词。
    #[serde(default = "crate::settings::default_hotwords_score")]
    pub hotwords_score: f32,
    /// 引擎专有配置项（如推理线程数 threads、provider）
    #[serde(default)]
    pub options: serde_json::Value,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            language: "zh".into(),
            hotwords: Vec::new(),
            hotwords_score: crate::settings::default_hotwords_score(),
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
    /// 如 "sherpa-onnx-x-asr-zh-en"
    fn id(&self) -> &'static str;
    fn display_name(&self) -> &str;
    fn capabilities(&self) -> EngineCapabilities;
    /// 模型是否已下载/可用
    fn is_ready(&self) -> bool;
    /// 预热（运行时「启动」调用）：把模型加载进内存 / 做完备性检查。
    /// 默认空实现——无驻留状态的引擎（如 sidecar 每次识别才拉起子进程）无需实现。
    fn warmup(&self) -> Result<(), String> {
        Ok(())
    }
    /// 卸载（运行时「停止」调用）：释放模型内存。默认空实现。
    fn unload(&self) {}
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

/// 引擎注册表容器：只持有「已注入」的引擎实例。
/// 引擎实现（mock/sherpa 系等）由 kotone-stt 经 `register` 注入，
/// core 不知道任何具体引擎（依赖方向：kotone-stt → core）。
pub struct EngineRegistry {
    engines: Vec<Box<dyn SttEngine>>,
}

impl EngineRegistry {
    /// 空注册表；引擎经 `register`（或 kotone-stt 的 register_builtin）注入
    pub fn new() -> Self {
        Self {
            engines: Vec::new(),
        }
    }

    /// 注入一个引擎实例（按 id 排序，稳定输出）
    pub fn register(&mut self, engine: Box<dyn SttEngine>) {
        self.engines.push(engine);
        self.engines.sort_by(|a, b| a.id().cmp(b.id()));
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
