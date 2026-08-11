//! 音频采集端口（core）：AudioBackend trait + AudioHandle/AudioDevice。
//! 生产实现（CpalBackend）在 kotone-platform-windows；orchestrator 面向 trait 编程，测试注入 mock。
//!
//! 契约：16kHz mono f32 PCM chunk 推入 `pcm_rx`，RMS 电平推入 `level_rx`；
//! Drop AudioHandle 即停止采集。

use tokio::sync::mpsc;

/// 音频接收端既支持测试/文件后端的无界通道，也支持真实麦克风的有界通道。
/// 生产采集必须使用有界通道，避免推理速度低于采集速度时 PCM 无限堆积。
pub enum AudioReceiver<T> {
    Bounded(mpsc::Receiver<T>),
    Unbounded(mpsc::UnboundedReceiver<T>),
}

impl<T> AudioReceiver<T> {
    pub async fn recv(&mut self) -> Option<T> {
        match self {
            Self::Bounded(rx) => rx.recv().await,
            Self::Unbounded(rx) => rx.recv().await,
        }
    }

    pub fn try_recv(&mut self) -> Result<T, mpsc::error::TryRecvError> {
        match self {
            Self::Bounded(rx) => rx.try_recv(),
            Self::Unbounded(rx) => rx.try_recv(),
        }
    }

    pub fn blocking_recv(&mut self) -> Option<T> {
        match self {
            Self::Bounded(rx) => rx.blocking_recv(),
            Self::Unbounded(rx) => rx.blocking_recv(),
        }
    }
}

/// 目标格式：16kHz mono f32（SttSession.push_audio 契约）
pub const TARGET_SAMPLE_RATE: u32 = 16000;
/// 推送粒度：50ms 一个 chunk
pub const CHUNK_SAMPLES: usize = (TARGET_SAMPLE_RATE as usize) / 20;

/// 音频输入设备
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AudioDevice {
    pub id: String,
    pub name: String,
}

/// 采集句柄：接收重采样后的 PCM chunk 与 RMS 电平；Drop 时停止采集
pub struct AudioHandle {
    /// Option 包裹便于 orchestrator take 走通道所有权
    pub pcm_rx: Option<AudioReceiver<Vec<f32>>>,
    pub level_rx: Option<AudioReceiver<f32>>,
    /// 采集中途故障（设备被拔出/驱动错误等）：生产实现经此上报错误消息，
    /// orchestrator 据此中止会话，避免状态机永久停在 Listening
    pub error_rx: Option<mpsc::UnboundedReceiver<String>>,
    stop_tx: Option<std::sync::mpsc::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl AudioHandle {
    /// 无采集线程的测试用句柄（mock AudioBackend 使用）
    pub fn detached(
        pcm_rx: mpsc::UnboundedReceiver<Vec<f32>>,
        level_rx: mpsc::UnboundedReceiver<f32>,
    ) -> Self {
        Self {
            pcm_rx: Some(AudioReceiver::Unbounded(pcm_rx)),
            level_rx: Some(AudioReceiver::Unbounded(level_rx)),
            error_rx: None,
            stop_tx: None,
            thread: None,
        }
    }

    /// 生产实现构造（kotone-platform-windows 的 CpalBackend 使用）
    pub fn with_thread(
        pcm_rx: mpsc::UnboundedReceiver<Vec<f32>>,
        level_rx: mpsc::UnboundedReceiver<f32>,
        error_rx: mpsc::UnboundedReceiver<String>,
        stop_tx: std::sync::mpsc::Sender<()>,
        thread: std::thread::JoinHandle<()>,
    ) -> Self {
        Self {
            pcm_rx: Some(AudioReceiver::Unbounded(pcm_rx)),
            level_rx: Some(AudioReceiver::Unbounded(level_rx)),
            error_rx: Some(error_rx),
            stop_tx: Some(stop_tx),
            thread: Some(thread),
        }
    }

    /// 真实设备使用的有界通道构造。音频回调绝不能阻塞；队列满时由平台后端
    /// 丢弃当前块并通过 error_rx 中止会话，而不是继续占用内存。
    pub fn with_bounded_thread(
        pcm_rx: mpsc::Receiver<Vec<f32>>,
        level_rx: mpsc::Receiver<f32>,
        error_rx: mpsc::UnboundedReceiver<String>,
        stop_tx: std::sync::mpsc::Sender<()>,
        thread: std::thread::JoinHandle<()>,
    ) -> Self {
        Self {
            pcm_rx: Some(AudioReceiver::Bounded(pcm_rx)),
            level_rx: Some(AudioReceiver::Bounded(level_rx)),
            error_rx: Some(error_rx),
            stop_tx: Some(stop_tx),
            thread: Some(thread),
        }
    }
}

impl Drop for AudioHandle {
    fn drop(&mut self) {
        if let Some(tx) = self.stop_tx.take() {
            let _ = tx.send(());
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// 采集来源抽象（orchestrator 依赖此 trait；测试注入 mock）
pub trait AudioBackend: Send + Sync {
    /// 打开设备并开始采集；设备打开失败返回清晰错误
    fn start(&self, device_id: &str) -> Result<AudioHandle, String>;
}
