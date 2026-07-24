//! 音频采集端口（core）：AudioBackend trait + AudioHandle/AudioDevice。
//! 生产实现（CpalBackend）在 kotone-platform-windows；orchestrator 面向 trait 编程，测试注入 mock。
//!
//! 契约：16kHz mono f32 PCM chunk 推入 `pcm_rx`，RMS 电平推入 `level_rx`；
//! Drop AudioHandle 即停止采集。

use tokio::sync::mpsc;

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
    pub pcm_rx: Option<mpsc::UnboundedReceiver<Vec<f32>>>,
    pub level_rx: Option<mpsc::UnboundedReceiver<f32>>,
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
            pcm_rx: Some(pcm_rx),
            level_rx: Some(level_rx),
            stop_tx: None,
            thread: None,
        }
    }

    /// 生产实现构造（kotone-platform-windows 的 CpalBackend 使用）
    pub fn with_thread(
        pcm_rx: mpsc::UnboundedReceiver<Vec<f32>>,
        level_rx: mpsc::UnboundedReceiver<f32>,
        stop_tx: std::sync::mpsc::Sender<()>,
        thread: std::thread::JoinHandle<()>,
    ) -> Self {
        Self {
            pcm_rx: Some(pcm_rx),
            level_rx: Some(level_rx),
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
