//! 音频采集生产实现：cpal（docs/development.md §5.1）
//!
//! 端口（AudioBackend trait / AudioHandle / AudioDevice）在 kotone-core；
//! 本模块是 cpal 生产实现。采集线程内部完成：
//!   设备打开（默认/指定）→ 原生格式转 f32 → 混成 mono → 线性重采样到 16kHz
//!   → 每 800 采样（50ms）一个 chunk 推入 pcm 通道；停止时再排出不足 50ms 的尾块。

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use tokio::sync::mpsc;

use kotone_core::audio::{
    AudioBackend, AudioDevice, AudioHandle, CHUNK_SAMPLES, TARGET_SAMPLE_RATE,
};

/// 枚举可用输入设备；首条为 "default" 伪设备（跟随系统默认）
pub fn list_devices() -> Vec<AudioDevice> {
    let host = cpal::default_host();
    let mut out = Vec::new();
    if let Some(d) = host.default_input_device() {
        if let Ok(name) = d.name() {
            out.push(AudioDevice {
                id: "default".into(),
                name: format!("系统默认（{name}）"),
            });
        }
    }
    if let Ok(devices) = host.input_devices() {
        for d in devices {
            if let Ok(name) = d.name() {
                out.push(AudioDevice {
                    id: name.clone(),
                    name,
                });
            }
        }
    }
    out
}

/// 枚举可用输出（播放）设备；首条为 "default" 伪设备（跟随系统默认）。
/// 采集用输入、虚拟声卡灌音用输出（CABLE Input 是输出设备）。
pub fn list_output_devices() -> Vec<AudioDevice> {
    let host = cpal::default_host();
    let mut out = Vec::new();
    if let Some(d) = host.default_output_device() {
        if let Ok(name) = d.name() {
            out.push(AudioDevice {
                id: "default".into(),
                name: format!("系统默认（{name}）"),
            });
        }
    }
    if let Ok(devices) = host.output_devices() {
        for d in devices {
            if let Ok(name) = d.name() {
                out.push(AudioDevice {
                    id: name.clone(),
                    name,
                });
            }
        }
    }
    out
}

/// 生产实现：cpal
pub struct CpalBackend;

impl AudioBackend for CpalBackend {
    fn start(&self, device_id: &str) -> Result<AudioHandle, String> {
        let (pcm_tx, pcm_rx) = mpsc::unbounded_channel::<Vec<f32>>();
        let (level_tx, level_rx) = mpsc::unbounded_channel::<f32>();
        // 采集中途故障（拔设备/驱动错误）经此上报 orchestrator，避免永久 Listening
        let (error_tx, error_rx) = mpsc::unbounded_channel::<String>();
        let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();
        // 设备打开结果回传：start 同步返回清晰错误，而不是等回调失败
        let (open_tx, open_rx) = std::sync::mpsc::channel::<Result<(), String>>();

        let device_id = device_id.to_string();
        let thread = std::thread::spawn(move || {
            match open_stream(&device_id, pcm_tx, level_tx, error_tx) {
                Ok(input) => {
                    let _ = open_tx.send(Ok(()));
                    // 流存活期间阻塞等待停止信号；stream 随作用域结束释放
                    let _ = stop_rx.recv();
                    input.stop_and_flush();
                }
                Err(e) => {
                    kotone_core::log::log(&format!("audio capture open failed: {e}"));
                    let _ = open_tx.send(Err(e));
                }
            }
        });

        match open_rx.recv() {
            Ok(Ok(())) => Ok(AudioHandle::with_thread(
                pcm_rx, level_rx, error_rx, stop_tx, thread,
            )),
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(e)
            }
            Err(_) => {
                let _ = thread.join();
                Err("音频采集线程异常退出".into())
            }
        }
    }
}

struct OpenedInput {
    stream: cpal::Stream,
    state: std::sync::Arc<std::sync::Mutex<CaptureState>>,
    pcm_tx: mpsc::UnboundedSender<Vec<f32>>,
    level_tx: mpsc::UnboundedSender<f32>,
}

impl OpenedInput {
    /// 先停回调，再把不足 50ms 的尾音作为最后一个短 chunk 送出。
    fn stop_and_flush(self) {
        let _ = self.stream.pause();
        drop(self.stream);
        self.state
            .lock()
            .unwrap()
            .flush(&self.pcm_tx, &self.level_tx);
    }
}

/// Windows 音频端点的默认格式属性损坏/不被驱动接受时，WASAPI 会返回
/// AUDCLNT_E_UNSUPPORTED_FORMAT。把难以理解的 CPAL/Win32 原始错误换成可执行提示。
fn default_config_error_message(device_name: &str, detail: &str) -> String {
    if detail.contains("-2004287480")
        || detail.contains("0x88890008")
        || detail.contains("88890008")
    {
        return format!(
            "麦克风格式异常，请打开 Windows 声音设置并重设默认格式。设备「{device_name}」返回 0x88890008；若仍失败，请关闭音频增强、重新插拔或重装设备"
        );
    }
    format!("读取设备「{device_name}」默认输入配置失败: {detail}")
}

/// 在线程内打开设备并构建输入流
fn open_stream(
    device_id: &str,
    pcm_tx: mpsc::UnboundedSender<Vec<f32>>,
    level_tx: mpsc::UnboundedSender<f32>,
    error_tx: mpsc::UnboundedSender<String>,
) -> Result<OpenedInput, String> {
    let host = cpal::default_host();
    let device = if device_id == "default" || device_id.is_empty() {
        host.default_input_device()
            .ok_or_else(|| "未找到系统默认音频输入设备（请检查麦克风连接与系统权限）".to_string())?
    } else {
        host.input_devices()
            .map_err(|e| format!("枚举音频输入设备失败: {e}"))?
            .find(|d| d.name().map(|n| n == device_id).unwrap_or(false))
            .ok_or_else(|| format!("未找到音频输入设备: {device_id}"))?
    };

    let device_name = device.name().unwrap_or_else(|_| device_id.into());
    let config = device.default_input_config().map_err(|e| {
        let detail = e.to_string();
        default_config_error_message(&device_name, &detail)
    })?;

    let sample_rate = config.sample_rate().0;
    let channels = config.channels() as usize;
    let stream_config: cpal::StreamConfig = config.clone().into();
    let state = std::sync::Arc::new(std::sync::Mutex::new(CaptureState::new(sample_rate)));

    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => {
            let state = state.clone();
            let pcm_tx = pcm_tx.clone();
            let level_tx = level_tx.clone();
            let error_tx = error_tx.clone();
            device.build_input_stream(
                &stream_config,
                move |data: &[f32], _| {
                    state
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .push_interleaved(data, channels, &pcm_tx, &level_tx)
                },
                move |e| {
                    eprintln!("[kotone audio] 采集错误: {e}");
                    // 上报 orchestrator：中止会话并提示（避免永久 Listening 无感知）
                    let _ = error_tx.send(format!("录音设备出错：{e}"));
                },
                None,
            )
        }
        cpal::SampleFormat::I16 => {
            let state = state.clone();
            let pcm_tx = pcm_tx.clone();
            let level_tx = level_tx.clone();
            let error_tx = error_tx.clone();
            device.build_input_stream(
                &stream_config,
                move |data: &[i16], _| {
                    let f: Vec<f32> = data
                        .iter()
                        .map(|s| cpal::Sample::to_sample::<f32>(*s))
                        .collect();
                    state
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .push_interleaved(&f, channels, &pcm_tx, &level_tx);
                },
                move |e| {
                    eprintln!("[kotone audio] 采集错误: {e}");
                    let _ = error_tx.send(format!("录音设备出错：{e}"));
                },
                None,
            )
        }
        cpal::SampleFormat::U16 => {
            let state = state.clone();
            let pcm_tx = pcm_tx.clone();
            let level_tx = level_tx.clone();
            let error_tx = error_tx.clone();
            device.build_input_stream(
                &stream_config,
                move |data: &[u16], _| {
                    let f: Vec<f32> = data
                        .iter()
                        .map(|s| cpal::Sample::to_sample::<f32>(*s))
                        .collect();
                    state
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .push_interleaved(&f, channels, &pcm_tx, &level_tx);
                },
                move |e| {
                    eprintln!("[kotone audio] 采集错误: {e}");
                    let _ = error_tx.send(format!("录音设备出错：{e}"));
                },
                None,
            )
        }
        fmt => return Err(format!("设备「{device_name}」采样格式不支持: {fmt:?}")),
    }
    .map_err(|e| format!("打开设备「{device_name}」失败: {e}"))?;

    stream
        .play()
        .map_err(|e| format!("启动设备「{device_name}」采集失败: {e}"))?;
    Ok(OpenedInput {
        stream,
        state,
        pcm_tx,
        level_tx,
    })
}

/// 采集状态：mono 混合 + 重采样 + 按 chunk 推送
struct CaptureState {
    resampler: Resampler,
    pending: Vec<f32>,
}

impl CaptureState {
    fn new(src_rate: u32) -> Self {
        Self {
            resampler: Resampler::new(src_rate, TARGET_SAMPLE_RATE),
            pending: Vec::with_capacity(CHUNK_SAMPLES * 2),
        }
    }

    /// 输入交错多声道 f32，混 mono → 重采样 → 每 50ms 发 pcm + rms
    fn push_interleaved(
        &mut self,
        data: &[f32],
        channels: usize,
        pcm_tx: &mpsc::UnboundedSender<Vec<f32>>,
        level_tx: &mpsc::UnboundedSender<f32>,
    ) {
        let ch = channels.max(1);
        let mono: Vec<f32> = data
            .chunks(ch)
            .map(|frame| frame.iter().sum::<f32>() / frame.len() as f32)
            .collect();
        let resampled = self.resampler.process(&mono);
        self.pending.extend_from_slice(&resampled);

        while self.pending.len() >= CHUNK_SAMPLES {
            let chunk: Vec<f32> = self.pending.drain(..CHUNK_SAMPLES).collect();
            let rms = (chunk.iter().map(|s| s * s).sum::<f32>() / chunk.len() as f32).sqrt();
            // 接收端已关闭（会话结束）时静默丢弃
            let _ = pcm_tx.send(chunk);
            let _ = level_tx.send(rms);
        }
    }

    /// 停止采集时排出最后一个不足 50ms 的短块，避免松键丢失句末音素。
    fn flush(
        &mut self,
        pcm_tx: &mpsc::UnboundedSender<Vec<f32>>,
        level_tx: &mpsc::UnboundedSender<f32>,
    ) {
        if self.pending.is_empty() {
            return;
        }
        let chunk = std::mem::take(&mut self.pending);
        let rms = (chunk.iter().map(|s| s * s).sum::<f32>() / chunk.len() as f32).sqrt();
        let _ = pcm_tx.send(chunk);
        let _ = level_tx.send(rms);
    }
}

/// 线性插值重采样器（任意 src → dst），跨 chunk 保持相位连续。
/// pub：wav 直灌后端（wav_audio）与播放（playback）复用。
pub struct Resampler {
    step: f64,     // src_rate / dst_rate
    next_pos: f64, // 下一个输出点在源坐标中的位置（相对当前 chunk）
}

impl Resampler {
    pub fn new(src_rate: u32, dst_rate: u32) -> Self {
        Self {
            step: src_rate as f64 / dst_rate as f64,
            next_pos: 0.0,
        }
    }

    pub fn process(&mut self, input: &[f32]) -> Vec<f32> {
        let mut out = Vec::new();
        if input.is_empty() {
            return out;
        }
        let len = input.len() as f64;
        while self.next_pos < len {
            let pos = self.next_pos;
            let i = pos.floor() as usize;
            let frac = (pos - i as f64) as f32;
            let a = input[i.min(input.len() - 1)];
            let b = input[(i + 1).min(input.len() - 1)];
            out.push(a + (b - a) * frac);
            self.next_pos += self.step;
        }
        self.next_pos -= len;
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_windows_mix_format_has_actionable_message() {
        let message = default_config_error_message(
            "麦克风 (PD200X Podcast Microphone)",
            "Failed to get mix format: OS Error -2004287480",
        );
        assert!(message.starts_with("麦克风格式异常"));
        assert!(message.contains("Windows 声音设置"));
        assert!(message.contains("0x88890008"));
    }

    #[test]
    fn other_default_config_errors_keep_backend_detail() {
        let message = default_config_error_message("测试设备", "device unavailable");
        assert_eq!(
            message,
            "读取设备「测试设备」默认输入配置失败: device unavailable"
        );
    }

    #[test]
    fn resampler_identity_when_rates_equal() {
        let mut r = Resampler::new(16000, 16000);
        let input: Vec<f32> = (0..100).map(|i| i as f32).collect();
        let out = r.process(&input);
        assert_eq!(out.len(), input.len());
        for (a, b) in out.iter().zip(input.iter()) {
            assert!((a - b).abs() < 1e-6);
        }
    }

    #[test]
    fn resampler_downsamples_48k_to_16k() {
        let mut r = Resampler::new(48000, 16000);
        let input = vec![0.5f32; 4800]; // 100ms @48k
        let out = r.process(&input);
        // 应输出约 1600 个采样（100ms @16k）
        assert!((out.len() as i64 - 1600).abs() <= 1, "got {}", out.len());
        assert!(out.iter().all(|&s| (s - 0.5).abs() < 1e-6));
    }

    #[test]
    fn resampler_continuous_across_chunks() {
        let mut r = Resampler::new(48000, 16000);
        let mut total = 0usize;
        for _ in 0..10 {
            total += r.process(&vec![1.0f32; 480]).len(); // 10ms 每块
        }
        // 100ms 总量 → 1600 ± 1
        assert!((total as i64 - 1600).abs() <= 1, "got {total}");
    }

    #[test]
    fn capture_state_chunks_and_rms() {
        let (pcm_tx, mut pcm_rx) = mpsc::unbounded_channel::<Vec<f32>>();
        let (level_tx, mut level_rx) = mpsc::unbounded_channel::<f32>();
        let mut st = CaptureState::new(16000);
        // 推 1 秒满幅 mono（双声道交错）
        let stereo: Vec<f32> = (0..16000).flat_map(|_| [1.0f32, 1.0f32]).collect();
        st.push_interleaved(&stereo, 2, &pcm_tx, &level_tx);
        let mut chunks = 0;
        while let Ok(c) = pcm_rx.try_recv() {
            assert_eq!(c.len(), CHUNK_SAMPLES);
            chunks += 1;
        }
        assert_eq!(chunks, 20, "1 秒应出 20 个 50ms chunk");
        let mut levels = 0;
        while let Ok(r) = level_rx.try_recv() {
            assert!((r - 1.0).abs() < 1e-5);
            levels += 1;
        }
        assert_eq!(levels, 20);
    }

    #[test]
    fn capture_state_flushes_short_tail_on_stop() {
        let (pcm_tx, mut pcm_rx) = mpsc::unbounded_channel::<Vec<f32>>();
        let (level_tx, mut level_rx) = mpsc::unbounded_channel::<f32>();
        let mut st = CaptureState::new(16000);

        // 25ms 不足一个常规 50ms chunk：旧实现会在输入流销毁时直接丢弃。
        st.push_interleaved(&vec![0.25f32; 400], 1, &pcm_tx, &level_tx);
        assert!(pcm_rx.try_recv().is_err());

        st.flush(&pcm_tx, &level_tx);
        let tail = pcm_rx.try_recv().expect("停止时应排出短尾音");
        assert_eq!(tail.len(), 400);
        assert!(tail.iter().all(|&s| (s - 0.25).abs() < 1e-6));
        assert!((level_rx.try_recv().unwrap() - 0.25).abs() < 1e-6);
        assert!(st.pending.is_empty());
    }
}
