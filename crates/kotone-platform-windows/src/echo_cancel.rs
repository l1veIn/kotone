//! 回声消除的麦克风采集（ADR 033 第三节）：通话里她说话时也能听玩家开口。
//!
//! 照 orb（dsh-orb-cordis，MIT，Copyright (c) 2026 mini-yifan）
//! `packages/voice/native/windows-voice-capture.cpp` 的做法：
//! - **参考信号 = 默认扬声器的回采**（WASAPI loopback；cpal 在输出设备上建输入流就是回采）：
//!   同一输出设备上任何程序的声音都在里面——她的声音、游戏声、从音箱出来的队友语音都能消掉。
//! - 另开一路一直写静音的输出流：扬声器上没声音时 loopback 不出数据，参考就断了。
//! - 关降噪、关自动增益，开高通；**不设通信类别**（那样 Windows 会把游戏音量压低 80%）。
//! - 麦克风和参考按 WASAPI 给的时间戳（同一个 QPC 时钟）排先后，10 ms 一帧：时间在前的参考帧先喂
//!   `process_render`，再处理这一帧麦克风（orb 修“念的时候转写走样”靠的就是逐帧对齐）。
//!
//! 回声消除用纯 Rust 的 sonora（WebRTC AudioProcessing / AEC3 的移植，BSD-3-Clause）：不带 C++，
//! 不影响 sherpa-onnx 的 /MT 链接。
//!
//! 输出契约同 [`crate::audio::CpalBackend`]：16 kHz 单声道、50 ms 一块、附电平；Drop 句柄即停。

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use tokio::sync::mpsc;

use kotone_core::audio::{AudioBackend, AudioHandle, CHUNK_SAMPLES, TARGET_SAMPLE_RATE};

use crate::audio::Resampler;

/// 10 ms 一帧（16 kHz）。
const FRAME: usize = (TARGET_SAMPLE_RATE / 100) as usize;
const FRAME_NS: u64 = 10_000_000;
/// 两次回调之间时间对不上超过这么多，就当中间断过、重新对时。
const RESYNC_NS: u64 = 30_000_000;
/// 麦克风这一帧到了以后，最多等参考信号多久（还没到就当扬声器没声音）。
const RENDER_WAIT: Duration = Duration::from_millis(80);
/// 参考帧最多攒多少（2 秒），防止麦克风停了参考还在涨。
const MAX_RENDER_FRAMES: usize = 200;
/// 同 `audio.rs`：最多缓存 2 秒 PCM；电平只留最新几个。
const PCM_QUEUE_CAPACITY: usize = 40;
const LEVEL_QUEUE_CAPACITY: usize = 4;

/// 回声消除的麦克风。打不开扬声器回采时返回错误，由调用方退回普通采集。
pub struct EchoCancelledBackend;

impl AudioBackend for EchoCancelledBackend {
    fn start(&self, device_id: &str) -> Result<AudioHandle, String> {
        let (pcm_tx, pcm_rx) = mpsc::channel::<Vec<f32>>(PCM_QUEUE_CAPACITY);
        let (level_tx, level_rx) = mpsc::channel::<f32>(LEVEL_QUEUE_CAPACITY);
        let (error_tx, error_rx) = mpsc::unbounded_channel::<String>();
        let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();
        let (open_tx, open_rx) = std::sync::mpsc::channel::<Result<(), String>>();
        let device_id = device_id.to_string();
        let thread = std::thread::Builder::new()
            .name("kotone-echo-cancel".into())
            .spawn(move || {
                let streams = match open_streams(&device_id, &error_tx) {
                    Ok(streams) => streams,
                    Err(e) => {
                        let _ = open_tx.send(Err(e));
                        return;
                    }
                };
                let _ = open_tx.send(Ok(()));
                run(&streams, &stop_rx, &pcm_tx, &level_tx, &error_tx);
                drop(streams);
            })
            .map_err(|e| format!("无法启动回声消除线程：{e}"))?;
        match open_rx.recv() {
            Ok(Ok(())) => Ok(AudioHandle::with_bounded_thread(
                pcm_rx, level_rx, error_rx, stop_tx, thread,
            )),
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(e)
            }
            Err(_) => {
                let _ = thread.join();
                Err("回声消除线程异常退出".into())
            }
        }
    }
}

/// 一路声音：混单声道、重采样到 16 kHz、攒成 10 ms 一帧，每帧带开始时刻（相对共同起点的纳秒）。
struct Side {
    resampler: Resampler,
    channels: usize,
    pending: Vec<f32>,
    /// `pending[0]` 的时刻。
    pending_at: u64,
    frames: VecDeque<(u64, Vec<f32>)>,
    /// 最近一次有数据的时刻（判断参考是不是断了）。
    last_data: Option<Instant>,
}

impl Side {
    fn new(src_rate: u32, channels: usize) -> Self {
        Self {
            resampler: Resampler::new(src_rate, TARGET_SAMPLE_RATE),
            channels: channels.max(1),
            pending: Vec::with_capacity(FRAME * 4),
            pending_at: 0,
            frames: VecDeque::new(),
            last_data: None,
        }
    }

    /// 一次回调的数据：`at` 是这批第一个样本的时刻（纳秒）。
    fn push(&mut self, data: &[f32], at: u64) {
        let mono: Vec<f32> = data
            .chunks(self.channels)
            .map(|frame| frame.iter().sum::<f32>() / frame.len() as f32)
            .collect();
        let resampled = self.resampler.process(&mono);
        let expected = self.pending_at + samples_ns(self.pending.len());
        if self.pending.is_empty() || expected.abs_diff(at) > RESYNC_NS {
            // 第一次、或中间断过：从这批的时刻重新算
            self.pending.clear();
            self.pending_at = at;
        }
        self.pending.extend_from_slice(&resampled);
        while self.pending.len() >= FRAME {
            let frame: Vec<f32> = self.pending.drain(..FRAME).collect();
            self.frames.push_back((self.pending_at, frame));
            self.pending_at += FRAME_NS;
        }
        self.last_data = Some(Instant::now());
    }
}

fn samples_ns(samples: usize) -> u64 {
    samples as u64 * 1_000_000_000 / u64::from(TARGET_SAMPLE_RATE)
}

struct Streams {
    _microphone: cpal::Stream,
    _loopback: cpal::Stream,
    _keep_alive: cpal::Stream,
    microphone: Arc<Mutex<Side>>,
    reference: Arc<Mutex<Side>>,
}

/// 打开麦克风、扬声器回采、一直写静音的输出流（都在这个线程上，句柄随线程结束释放）。
fn open_streams(
    device_id: &str,
    error_tx: &mpsc::UnboundedSender<String>,
) -> Result<Streams, String> {
    let host = cpal::default_host();
    let (microphone_device, _) = crate::audio::input_device(&host, device_id)?;
    let speaker = host
        .default_output_device()
        .ok_or_else(|| "未找到系统默认音频输出设备，没法做回声消除".to_string())?;
    let microphone_config = microphone_device
        .default_input_config()
        .map_err(|e| format!("读取麦克风格式失败: {e}"))?;
    let speaker_config = speaker
        .default_output_config()
        .map_err(|e| format!("读取扬声器格式失败: {e}"))?;
    // 两路共用一个时间起点（WASAPI 的时间戳都来自 QPC）
    let origin: Arc<OnceLock<cpal::StreamInstant>> = Arc::new(OnceLock::new());

    let microphone = Arc::new(Mutex::new(Side::new(
        microphone_config.sample_rate().0,
        microphone_config.channels() as usize,
    )));
    let reference = Arc::new(Mutex::new(Side::new(
        speaker_config.sample_rate().0,
        speaker_config.channels() as usize,
    )));
    let microphone_stream = build_input(
        &microphone_device,
        &microphone_config,
        feeder(microphone.clone(), origin.clone()),
        error_tx.clone(),
    )
    .map_err(|e| format!("打开麦克风失败: {e}"))?;
    // 在输出设备上建输入流 = WASAPI 回采（cpal 自动加 AUDCLNT_STREAMFLAGS_LOOPBACK）
    let loopback_stream = build_input(
        &speaker,
        &speaker_config,
        feeder(reference.clone(), origin),
        error_tx.clone(),
    )
    .map_err(|e| format!("打开扬声器回采失败: {e}"))?;
    let keep_alive =
        build_silence(&speaker, &speaker_config).map_err(|e| format!("打开静音输出失败: {e}"))?;
    keep_alive
        .play()
        .map_err(|e| format!("启动静音输出失败: {e}"))?;
    loopback_stream
        .play()
        .map_err(|e| format!("启动扬声器回采失败: {e}"))?;
    microphone_stream
        .play()
        .map_err(|e| format!("启动麦克风失败: {e}"))?;
    kotone_core::log::log(&format!(
        "echo cancel: microphone \"{}\" {} Hz x{}, reference \"{}\" {} Hz x{}",
        microphone_device.name().unwrap_or_default(),
        microphone_config.sample_rate().0,
        microphone_config.channels(),
        speaker.name().unwrap_or_default(),
        speaker_config.sample_rate().0,
        speaker_config.channels(),
    ));
    Ok(Streams {
        _microphone: microphone_stream,
        _loopback: loopback_stream,
        _keep_alive: keep_alive,
        microphone,
        reference,
    })
}

/// 回调里只做混声道、重采样、按时刻切帧（短暂加锁）。
fn feeder(
    side: Arc<Mutex<Side>>,
    origin: Arc<OnceLock<cpal::StreamInstant>>,
) -> impl FnMut(&[f32], cpal::StreamInstant) + Send + 'static {
    move |data, at| {
        let origin = origin.get_or_init(|| at.sub(Duration::from_secs(3_600)).unwrap_or(at));
        let ns = at
            .duration_since(origin)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        side.lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(data, ns);
    }
}

fn build_input<F>(
    device: &cpal::Device,
    config: &cpal::SupportedStreamConfig,
    mut on_data: F,
    error_tx: mpsc::UnboundedSender<String>,
) -> Result<cpal::Stream, String>
where
    F: FnMut(&[f32], cpal::StreamInstant) + Send + 'static,
{
    let stream_config: cpal::StreamConfig = config.clone().into();
    let on_error = move |e: cpal::StreamError| {
        let _ = error_tx.send(format!("录音设备出错：{e}"));
    };
    match config.sample_format() {
        cpal::SampleFormat::F32 => device.build_input_stream(
            &stream_config,
            move |data: &[f32], info: &cpal::InputCallbackInfo| {
                on_data(data, info.timestamp().capture)
            },
            on_error,
            None,
        ),
        cpal::SampleFormat::I16 => device.build_input_stream(
            &stream_config,
            move |data: &[i16], info: &cpal::InputCallbackInfo| {
                let f: Vec<f32> = data
                    .iter()
                    .map(|s| cpal::Sample::to_sample::<f32>(*s))
                    .collect();
                on_data(&f, info.timestamp().capture)
            },
            on_error,
            None,
        ),
        cpal::SampleFormat::U16 => device.build_input_stream(
            &stream_config,
            move |data: &[u16], info: &cpal::InputCallbackInfo| {
                let f: Vec<f32> = data
                    .iter()
                    .map(|s| cpal::Sample::to_sample::<f32>(*s))
                    .collect();
                on_data(&f, info.timestamp().capture)
            },
            on_error,
            None,
        ),
        format => return Err(format!("采样格式不支持: {format:?}")),
    }
    .map_err(|e| e.to_string())
}

/// 一直写静音的输出流：让 loopback 在扬声器没声音时也出数据。
fn build_silence(
    device: &cpal::Device,
    config: &cpal::SupportedStreamConfig,
) -> Result<cpal::Stream, String> {
    let stream_config: cpal::StreamConfig = config.clone().into();
    let on_error = |e: cpal::StreamError| {
        kotone_core::log::log(&format!("echo cancel: silent output error: {e}"));
    };
    match config.sample_format() {
        cpal::SampleFormat::F32 => device.build_output_stream(
            &stream_config,
            |data: &mut [f32], _: &cpal::OutputCallbackInfo| data.fill(0.0),
            on_error,
            None,
        ),
        cpal::SampleFormat::I16 => device.build_output_stream(
            &stream_config,
            |data: &mut [i16], _: &cpal::OutputCallbackInfo| data.fill(0),
            on_error,
            None,
        ),
        cpal::SampleFormat::U16 => device.build_output_stream(
            &stream_config,
            |data: &mut [u16], _: &cpal::OutputCallbackInfo| data.fill(u16::MAX / 2 + 1),
            on_error,
            None,
        ),
        format => return Err(format!("采样格式不支持: {format:?}")),
    }
    .map_err(|e| e.to_string())
}

/// 处理循环：按时刻先喂参考、再处理麦克风，攒够 50 ms 交出去。
fn run(
    streams: &Streams,
    stop_rx: &std::sync::mpsc::Receiver<()>,
    pcm_tx: &mpsc::Sender<Vec<f32>>,
    level_tx: &mpsc::Sender<f32>,
    error_tx: &mpsc::UnboundedSender<String>,
) {
    let mut canceller = Canceller::new();
    let mut microphone: VecDeque<(u64, Vec<f32>, Instant)> = VecDeque::new();
    let mut reference: VecDeque<(u64, Vec<f32>)> = VecDeque::new();
    let mut out = Output::default();
    loop {
        match stop_rx.recv_timeout(Duration::from_millis(5)) {
            Ok(()) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
        }
        let arrived = Instant::now();
        microphone.extend(
            streams
                .microphone
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .frames
                .drain(..)
                .map(|(at, frame)| (at, frame, arrived)),
        );
        let reference_alive = {
            let mut side = streams.reference.lock().unwrap_or_else(|p| p.into_inner());
            reference.extend(side.frames.drain(..));
            side.last_data
                .is_some_and(|at| at.elapsed() < Duration::from_millis(200))
        };
        while reference.len() > MAX_RENDER_FRAMES {
            reference.pop_front();
        }
        while let Some((at, _, arrived)) = microphone.front() {
            let at = *at;
            // 参考还没跟上这一帧：扬声器那路活着就稍等，等太久就当没声音
            let caught_up = reference.back().is_some_and(|(t, _)| *t + FRAME_NS > at);
            if !caught_up && reference_alive && arrived.elapsed() < RENDER_WAIT {
                break;
            }
            let mut fed = false;
            while reference.front().is_some_and(|(t, _)| *t <= at) {
                let (_, frame) = reference.pop_front().unwrap();
                canceller.render(&frame);
                fed = true;
            }
            if !fed {
                canceller.render(&[0.0; FRAME]);
            }
            let (_, frame, _) = microphone.pop_front().unwrap();
            let cleaned = canceller.capture(&frame);
            out.push(&cleaned, pcm_tx, level_tx, error_tx);
        }
    }
    out.flush(pcm_tx, level_tx);
    canceller.log_stats();
}

/// sonora 的回声消除（照 orb 的配置：AEC3 + 高通，降噪和自动增益关掉）。
struct Canceller {
    apm: sonora::AudioProcessing,
    scratch: Vec<f32>,
    frames: u64,
}

impl Canceller {
    fn new() -> Self {
        let config = sonora::Config {
            echo_canceller: Some(sonora::config::EchoCanceller::default()),
            high_pass_filter: Some(sonora::config::HighPassFilter::default()),
            ..Default::default()
        };
        let stream = sonora::StreamConfig::new(TARGET_SAMPLE_RATE, 1);
        let apm = sonora::AudioProcessing::builder()
            .config(config)
            .capture_config(stream)
            .render_config(stream)
            .build();
        Self {
            apm,
            scratch: vec![0.0; FRAME],
            frames: 0,
        }
    }

    fn render(&mut self, frame: &[f32]) {
        let _ = self
            .apm
            .process_render_f32(&[frame], &mut [self.scratch.as_mut_slice()]);
    }

    fn capture(&mut self, frame: &[f32]) -> Vec<f32> {
        // orb：参考已经按时刻对齐，告诉它没有额外延迟
        let _ = self.apm.set_stream_delay_ms(0);
        let mut out = vec![0.0; FRAME];
        if self
            .apm
            .process_capture_f32(&[frame], &mut [out.as_mut_slice()])
            .is_err()
        {
            out.copy_from_slice(frame);
        }
        self.frames += 1;
        out
    }

    fn log_stats(&self) {
        kotone_core::log::log(&format!(
            "echo cancel: stopped after {} s, stats {:?}",
            self.frames / 100,
            self.apm.statistics()
        ));
    }
}

/// 攒成 50 ms 一块交给会话（同 `audio.rs` 的 CaptureState）。
#[derive(Default)]
struct Output {
    pending: Vec<f32>,
    overrun_reported: bool,
}

impl Output {
    fn push(
        &mut self,
        samples: &[f32],
        pcm_tx: &mpsc::Sender<Vec<f32>>,
        level_tx: &mpsc::Sender<f32>,
        error_tx: &mpsc::UnboundedSender<String>,
    ) {
        self.pending.extend_from_slice(samples);
        while self.pending.len() >= CHUNK_SAMPLES {
            let chunk: Vec<f32> = self.pending.drain(..CHUNK_SAMPLES).collect();
            let rms = (chunk.iter().map(|s| s * s).sum::<f32>() / chunk.len() as f32).sqrt();
            match pcm_tx.try_send(chunk) {
                Ok(()) => {
                    let _ = level_tx.try_send(rms);
                }
                Err(mpsc::error::TrySendError::Full(_)) => {
                    if !self.overrun_reported {
                        self.overrun_reported = true;
                        let _ = error_tx.send(
                            "语音识别处理速度跟不上录音，这次识别已中止。请关闭占用 CPU 的程序后重试"
                                .into(),
                        );
                    }
                }
                Err(mpsc::error::TrySendError::Closed(_)) => return,
            }
        }
    }

    fn flush(&mut self, pcm_tx: &mpsc::Sender<Vec<f32>>, level_tx: &mpsc::Sender<f32>) {
        if self.pending.is_empty() {
            return;
        }
        let chunk = std::mem::take(&mut self.pending);
        let rms = (chunk.iter().map(|s| s * s).sum::<f32>() / chunk.len() as f32).sqrt();
        if pcm_tx.try_send(chunk).is_ok() {
            let _ = level_tx.try_send(rms);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn side_cuts_ten_millisecond_frames_with_times() {
        let mut side = Side::new(16_000, 1);
        side.push(&[0.1; 400], 1_000_000_000);
        // 400 个样本 = 2 帧 + 80 个待攒
        assert_eq!(side.frames.len(), 2);
        assert_eq!(side.frames[0].0, 1_000_000_000);
        assert_eq!(side.frames[1].0, 1_010_000_000);
        // 接着来的一批：时刻连得上，接着切
        side.push(&[0.1; 80], 1_025_000_000);
        assert_eq!(side.frames.len(), 3);
        assert_eq!(side.frames[2].0, 1_020_000_000);
        // 中间断了一秒：重新对时
        side.push(&[0.1; 160], 2_100_000_000);
        assert_eq!(side.frames.back().unwrap().0, 2_100_000_000);
    }

    #[test]
    fn stereo_48k_becomes_mono_16k() {
        let mut side = Side::new(48_000, 2);
        side.push(&[0.5; 48_000 * 2 / 100], 0);
        assert_eq!(side.frames.len(), 1);
        assert_eq!(side.frames[0].1.len(), FRAME);
    }

    /// 扬声器里放的声音原样进了麦克风（延迟 30 ms、小一点）：消掉以后明显变小。
    #[test]
    fn echo_of_the_reference_is_removed() {
        let mut canceller = Canceller::new();
        let mut state = 1u32;
        let mut noise = || {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (state >> 9) as f32 / (1u32 << 23) as f32 - 0.5
        };
        let speech: Vec<f32> = (0..16_000 * 6).map(|_| noise() * 0.4).collect();
        let delay = FRAME * 3;
        let mut before = 0.0f64;
        let mut after = 0.0f64;
        for i in 0..speech.len() / FRAME {
            let render = &speech[i * FRAME..(i + 1) * FRAME];
            let mic: Vec<f32> = (0..FRAME)
                .map(|j| {
                    let k = (i * FRAME + j) as isize - delay as isize;
                    if k >= 0 {
                        speech[k as usize] * 0.5
                    } else {
                        0.0
                    }
                })
                .collect();
            canceller.render(render);
            let cleaned = canceller.capture(&mic);
            // 只看最后两秒（前面是它在学）
            if i >= speech.len() / FRAME - 200 {
                before += mic.iter().map(|s| f64::from(s * s)).sum::<f64>();
                after += cleaned.iter().map(|s| f64::from(s * s)).sum::<f64>();
            }
        }
        let reduction_db = 10.0 * (before / after.max(1e-12)).log10();
        println!("echo reduction: {reduction_db:.1} dB");
        assert!(reduction_db > 10.0, "only {reduction_db:.1} dB");
    }
}
