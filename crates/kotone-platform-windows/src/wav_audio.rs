//! wav 直灌音频后端（`WavFileBackend`）：把 16kHz wav 文件当作采集设备，
//! 按真实节奏（或 N 倍速）把 PCM 喂给 orchestrator。
//!
//! 归属说明：ADR-001 规定 ports 在 core、实现在适配器 crate——本后端是
//! `AudioBackend` 端口的一种「虚拟采集设备」实现（与 CpalBackend 同级），
//! 故放 platform crate。实现本身是纯 Rust 跨平台，不碰 Win32。
//!
//! 用途：无人值守自动化测试（`kotone-cli listen --wav`），不依赖真人说话；
//! 与虚拟声卡回路（playback）互补：直灌路径不经过任何系统音频栈。

use std::path::PathBuf;

use tokio::sync::mpsc;

use kotone_core::audio::{AudioBackend, AudioHandle, CHUNK_SAMPLES, TARGET_SAMPLE_RATE};

/// wav 文件直灌后端：`start` 把文件内容按 50ms chunk 节奏推入 PCM/RMS 通道，
/// 喂完关闭通道（orchestrator 的 pump 见 None 收尾）。`device_id` 参数忽略。
pub struct WavFileBackend {
    path: PathBuf,
    /// 喂入速度倍率：1.0 = 实时；2.0 = 两倍速；0 = 全速（不 sleep）
    speed: f64,
}

impl WavFileBackend {
    pub fn new(path: impl Into<PathBuf>, speed: f64) -> Self {
        Self {
            path: path.into(),
            speed,
        }
    }

    /// 文件音频时长（毫秒），供调用方计算会话等待时间
    pub fn audio_ms(&self) -> Result<u64, String> {
        let pcm = kotone_core::eval::read_wav(&self.path)?;
        Ok(pcm.len() as u64 * 1000 / TARGET_SAMPLE_RATE as u64)
    }
}

impl AudioBackend for WavFileBackend {
    fn start(&self, _device_id: &str) -> Result<AudioHandle, String> {
        let pcm = kotone_core::eval::read_wav(&self.path)?;
        let (pcm_tx, pcm_rx) = mpsc::unbounded_channel::<Vec<f32>>();
        let (level_tx, level_rx) = mpsc::unbounded_channel::<f32>();
        let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();

        let speed = self.speed;
        let thread = std::thread::spawn(move || {
            let pace = if speed > 0.0 {
                // 每 chunk 对应 50ms 音频；speed 倍速 → 墙钟 50/speed ms
                std::time::Duration::from_secs_f64(0.05 / speed)
            } else {
                std::time::Duration::ZERO
            };
            for chunk in pcm.chunks(CHUNK_SAMPLES) {
                if stop_rx.try_recv().is_ok() {
                    return; // 会话取消/提前结束
                }
                let rms =
                    (chunk.iter().map(|s| s * s).sum::<f32>() / chunk.len() as f32).sqrt();
                if pcm_tx.send(chunk.to_vec()).is_err() {
                    return; // 接收端已关闭
                }
                let _ = level_tx.send(rms);
                if pace > std::time::Duration::ZERO {
                    std::thread::sleep(pace);
                }
            }
            // 文件喂完：tx 随线程结束 drop，通道关闭 → pump 收尾
        });

        Ok(AudioHandle::with_thread(pcm_rx, level_rx, stop_tx, thread))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 造一个临时 wav（1 秒渐变信号）
    fn make_wav(dir: &tempfile::TempDir, samples: usize) -> PathBuf {
        let path = dir.path().join("t.wav");
        let pcm: Vec<f32> = (0..samples).map(|i| (i as f32 / samples as f32) - 0.5).collect();
        kotone_core::eval::write_wav(&path, &pcm).unwrap();
        path
    }

    #[test]
    fn feeds_exact_content_and_closes_channel() {
        let dir = tempfile::tempdir().unwrap();
        let path = make_wav(&dir, 16000); // 1s
        let backend = WavFileBackend::new(&path, 0.0); // 全速
        let mut handle = backend.start("ignored").unwrap();

        // 等喂入线程结束（1s 音频全速 < 1s 墙钟）
        let pcm_rx = handle.pcm_rx.take().unwrap();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut chunks = Vec::new();
            let mut rx = pcm_rx;
            while let Some(c) = rx.blocking_recv() {
                chunks.push(c);
            }
            let _ = done_tx.send(chunks);
        });
        let chunks = done_rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("通道应在文件喂完后关闭");

        let total: usize = chunks.iter().map(|c| c.len()).sum();
        assert_eq!(total, 16000, "喂入总采样数应与文件一致");
        assert!(chunks.iter().all(|c| c.len() == CHUNK_SAMPLES));
        // 内容一致（16bit 量化误差内）
        let flat: Vec<f32> = chunks.into_iter().flatten().collect();
        let expect: Vec<f32> = (0..16000).map(|i| (i as f32 / 16000.0) - 0.5).collect();
        for (a, b) in flat.iter().zip(expect.iter()) {
            assert!((a - b).abs() < 1e-3, "{a} vs {b}");
        }
        // 电平通道同样有 20 条 RMS
        let mut levels = 0;
        let mut level_rx = handle.level_rx.take().unwrap();
        while level_rx.blocking_recv().is_some() {
            levels += 1;
        }
        assert_eq!(levels, 20);
    }

    #[test]
    fn partial_last_chunk_kept() {
        let dir = tempfile::tempdir().unwrap();
        let path = make_wav(&dir, CHUNK_SAMPLES + 100); // 不足两个 chunk
        let backend = WavFileBackend::new(&path, 0.0);
        let mut handle = backend.start("x").unwrap();
        let mut rx = handle.pcm_rx.take().unwrap();
        let mut total = 0;
        while let Some(c) = rx.blocking_recv() {
            total += c.len();
        }
        assert_eq!(total, CHUNK_SAMPLES + 100, "尾部不足一个 chunk 的样本也要喂出");
    }

    #[test]
    fn pacing_is_realtime_at_speed_1() {
        let dir = tempfile::tempdir().unwrap();
        let path = make_wav(&dir, 16000); // 1s 音频
        let backend = WavFileBackend::new(&path, 1.0);
        let started = std::time::Instant::now();
        let mut handle = backend.start("x").unwrap();
        let mut rx = handle.pcm_rx.take().unwrap();
        while rx.blocking_recv().is_some() {}
        let elapsed = started.elapsed();
        // 20 个 chunk × 50ms = 1000ms；Windows 定时器粒度留余量
        assert!(elapsed.as_millis() >= 900, "实时节奏应 ≈1s: {elapsed:?}");
        assert!(elapsed.as_millis() < 2000, "不应显著超实时: {elapsed:?}");
    }

    #[test]
    fn speed_10_is_faster_than_realtime() {
        let dir = tempfile::tempdir().unwrap();
        let path = make_wav(&dir, 16000);
        let backend = WavFileBackend::new(&path, 10.0);
        let started = std::time::Instant::now();
        let mut handle = backend.start("x").unwrap();
        let mut rx = handle.pcm_rx.take().unwrap();
        while rx.blocking_recv().is_some() {}
        assert!(
            started.elapsed().as_millis() < 500,
            "10 倍速喂 1s 音频应 ≈100ms: {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn audio_ms_reports_duration() {
        let dir = tempfile::tempdir().unwrap();
        let path = make_wav(&dir, 24000); // 1.5s
        let backend = WavFileBackend::new(&path, 0.0);
        assert_eq!(backend.audio_ms().unwrap(), 1500);
    }
}
