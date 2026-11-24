//! 游戏语音的转发与混音（ADR 020）：真麦克风 + 琴音“说给队友”的语音 → 虚拟麦克风 CABLE Input。
//!
//! 照 cpal `examples/feedback.rs`：一个输入流（真麦克风）把声音放进缓冲，一个输出流（CABLE Input）
//! 从缓冲里取；琴音的语音放进另一个缓冲，输出时混在一起。另外两点：
//! - 麦克风缓冲最多留 [`MAX_MIC_LATENCY_MS`]，多了丢最旧的，延迟不会越积越大；
//! - 琴音说话时把玩家的声音压到 [`DUCK_GAIN`]（照常见的语音软件“闪避”），两个人的声音不叠在一起。
//!
//! cpal 的流不能跨线程，所以两个流都在自己的线程里打开并持有，`stop` / 丢弃时一起关。

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use kotone_platform_windows::audio::Resampler;

/// 麦克风缓冲最多多少毫秒（超出丢最旧的）。
pub const MAX_MIC_LATENCY_MS: u32 = 200;
/// 琴音说话时玩家声音的音量。
pub const DUCK_GAIN: f32 = 0.5;
/// 琴音的语音最多排多少秒。
pub const MAX_SPEECH_SECONDS: u32 = 60;

/// 混音的两个缓冲（都已经是输出采样率、单声道）。纯逻辑，便于测试。
#[derive(Debug)]
pub struct Mixer {
    rate: u32,
    mic: VecDeque<f32>,
    speech: VecDeque<f32>,
    pub mic_gain: f32,
}

impl Mixer {
    pub fn new(rate: u32) -> Self {
        Self {
            rate,
            mic: VecDeque::new(),
            speech: VecDeque::new(),
            mic_gain: 1.0,
        }
    }

    fn cap(&self, ms: u32) -> usize {
        (u64::from(self.rate) * u64::from(ms) / 1000) as usize
    }

    pub fn push_mic(&mut self, samples: &[f32]) {
        self.mic.extend(samples);
        let cap = self.cap(MAX_MIC_LATENCY_MS);
        if self.mic.len() > cap {
            let drop = self.mic.len() - cap;
            self.mic.drain(..drop);
        }
    }

    /// 排进一句琴音的语音；返回这句要放多少毫秒。超过上限的部分丢掉。
    pub fn push_speech(&mut self, samples: &[f32]) -> u64 {
        let room = self
            .cap(MAX_SPEECH_SECONDS * 1000)
            .saturating_sub(self.speech.len());
        let take = samples.len().min(room);
        self.speech.extend(&samples[..take]);
        take as u64 * 1000 / u64::from(self.rate.max(1))
    }

    pub fn speaking(&self) -> bool {
        !self.speech.is_empty()
    }

    /// 清掉还没放的琴音语音（停止）。
    pub fn clear_speech(&mut self) {
        self.speech.clear();
    }

    /// 取下一个输出采样。
    pub fn next_sample(&mut self) -> f32 {
        let speech = self.speech.pop_front();
        let mic = self.mic.pop_front().unwrap_or(0.0) * self.mic_gain;
        let out = match speech {
            Some(s) => s + mic * DUCK_GAIN,
            None => mic,
        };
        out.clamp(-1.0, 1.0)
    }
}

/// 启动参数。
#[derive(Debug, Clone)]
pub struct RouterConfig {
    /// 输出设备名（VB-CABLE 的 CABLE Input）。
    pub output: String,
    /// 真麦克风：None = 不转发玩家的声音；Some("default") = 系统默认录音设备；其他 = 设备名。
    pub mic: Option<String>,
    pub mic_gain: f32,
}

struct Shared {
    mixer: Mutex<Mixer>,
    error: Mutex<Option<String>>,
}

pub struct VoiceRouter {
    shared: Arc<Shared>,
    rate: u32,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

fn find_device(host: &cpal::Host, name: &str, input: bool) -> Result<cpal::Device, String> {
    if input && (name.is_empty() || name == "default") {
        return host
            .default_input_device()
            .ok_or_else(|| "找不到系统默认麦克风".to_string());
    }
    let mut devices: Box<dyn Iterator<Item = cpal::Device>> = if input {
        Box::new(
            host.input_devices()
                .map_err(|e| format!("枚举录音设备失败：{e}"))?,
        )
    } else {
        Box::new(
            host.output_devices()
                .map_err(|e| format!("枚举播放设备失败：{e}"))?,
        )
    };
    devices
        .find(|d| d.name().is_ok_and(|n| n == name))
        .ok_or_else(|| format!("找不到音频设备「{name}」"))
}

fn output_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    shared: Arc<Shared>,
) -> Result<cpal::Stream, String>
where
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    let channels = usize::from(config.channels.max(1));
    let error = shared.clone();
    device
        .build_output_stream(
            config,
            move |output: &mut [T], _| {
                let mut mixer = shared.mixer.lock().unwrap_or_else(|p| p.into_inner());
                for frame in output.chunks_mut(channels) {
                    frame.fill(T::from_sample(mixer.next_sample()));
                }
            },
            move |e| *error.error.lock().unwrap() = Some(format!("虚拟麦克风输出出错：{e}")),
            None,
        )
        .map_err(|e| format!("无法打开虚拟麦克风：{e}"))
}

fn input_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    out_rate: u32,
    shared: Arc<Shared>,
) -> Result<cpal::Stream, String>
where
    T: cpal::SizedSample,
    f32: cpal::FromSample<T>,
{
    let channels = usize::from(config.channels.max(1));
    let mut resampler = Resampler::new(config.sample_rate.0, out_rate);
    let error = shared.clone();
    device
        .build_input_stream(
            config,
            move |data: &[T], _| {
                let mono: Vec<f32> = data
                    .chunks(channels)
                    .map(|f| {
                        f.iter()
                            .map(|s| <f32 as cpal::FromSample<T>>::from_sample_(*s))
                            .sum::<f32>()
                            / f.len() as f32
                    })
                    .collect();
                let samples = resampler.process(&mono);
                shared
                    .mixer
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .push_mic(&samples);
            },
            move |e| *error.error.lock().unwrap() = Some(format!("麦克风出错：{e}")),
            None,
        )
        .map_err(|e| format!("无法打开麦克风：{e}"))
}

fn open(config: &RouterConfig, shared: &Arc<Shared>) -> Result<(Vec<cpal::Stream>, u32), String> {
    let host = cpal::default_host();
    let out_device = find_device(&host, &config.output, false)?;
    let out_config = out_device
        .default_output_config()
        .map_err(|e| format!("读取虚拟麦克风格式失败：{e}"))?;
    let rate = out_config.sample_rate().0;
    shared.mixer.lock().unwrap().rate = rate;
    let stream_config: cpal::StreamConfig = out_config.clone().into();
    let output = match out_config.sample_format() {
        cpal::SampleFormat::F32 => {
            output_stream::<f32>(&out_device, &stream_config, shared.clone())
        }
        cpal::SampleFormat::I16 => {
            output_stream::<i16>(&out_device, &stream_config, shared.clone())
        }
        cpal::SampleFormat::U16 => {
            output_stream::<u16>(&out_device, &stream_config, shared.clone())
        }
        other => Err(format!("暂不支持虚拟麦克风的格式 {other}")),
    }?;
    let mut streams = vec![output];
    if let Some(mic) = &config.mic {
        let in_device = find_device(&host, mic, true)?;
        if in_device
            .name()
            .is_ok_and(|n| crate::cable::is_cable(&n, crate::cable::CABLE_OUTPUT))
        {
            return Err("真麦克风不能选 CABLE Output（会把自己的声音循环回去）".into());
        }
        let in_config = in_device
            .default_input_config()
            .map_err(|e| format!("读取麦克风格式失败：{e}"))?;
        let stream_config: cpal::StreamConfig = in_config.clone().into();
        let input = match in_config.sample_format() {
            cpal::SampleFormat::F32 => {
                input_stream::<f32>(&in_device, &stream_config, rate, shared.clone())
            }
            cpal::SampleFormat::I16 => {
                input_stream::<i16>(&in_device, &stream_config, rate, shared.clone())
            }
            cpal::SampleFormat::U16 => {
                input_stream::<u16>(&in_device, &stream_config, rate, shared.clone())
            }
            other => Err(format!("暂不支持麦克风的格式 {other}")),
        }?;
        streams.push(input);
    }
    for stream in &streams {
        stream.play().map_err(|e| format!("无法开始转发：{e}"))?;
    }
    Ok((streams, rate))
}

impl VoiceRouter {
    /// 打开设备开始转发；设备打不开时返回原因。
    pub fn start(config: RouterConfig) -> Result<Self, String> {
        let shared = Arc::new(Shared {
            mixer: Mutex::new(Mixer::new(48_000)),
            error: Mutex::new(None),
        });
        shared.mixer.lock().unwrap().mic_gain = config.mic_gain.clamp(0.0, 4.0);
        let stop = Arc::new(AtomicBool::new(false));
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<u32, String>>();
        let thread_shared = shared.clone();
        let thread_stop = stop.clone();
        let thread = std::thread::Builder::new()
            .name("kotone-voice-router".into())
            .spawn(move || match open(&config, &thread_shared) {
                Ok((streams, rate)) => {
                    let _ = ready_tx.send(Ok(rate));
                    while !thread_stop.load(Ordering::SeqCst) {
                        std::thread::sleep(std::time::Duration::from_millis(50));
                    }
                    drop(streams);
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                }
            })
            .map_err(|e| format!("无法启动转发线程：{e}"))?;
        match ready_rx.recv() {
            Ok(Ok(rate)) => Ok(Self {
                shared,
                rate,
                stop,
                thread: Some(thread),
            }),
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(e)
            }
            Err(_) => {
                let _ = thread.join();
                Err("转发线程异常退出".into())
            }
        }
    }

    /// 把一句语音（任意采样率，单声道）送进虚拟麦克风；返回要放多少毫秒。
    pub fn say(&self, samples: &[f32], sample_rate: u32) -> u64 {
        let resampled = Resampler::new(sample_rate.max(1), self.rate).process(samples);
        self.shared.mixer.lock().unwrap().push_speech(&resampled)
    }

    /// 琴音的语音还在放。
    pub fn speaking(&self) -> bool {
        self.shared.mixer.lock().unwrap().speaking()
    }

    /// 停掉还没放完的琴音语音（玩家的声音照常转发）。
    pub fn hush(&self) {
        self.shared.mixer.lock().unwrap().clear_speech();
    }

    /// 设备出过的错（拔掉设备等）。
    pub fn error(&self) -> Option<String> {
        self.shared.error.lock().unwrap().clone()
    }

    pub fn sample_rate(&self) -> u32 {
        self.rate
    }
}

impl Drop for VoiceRouter {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// 从一个录音设备录 `ms` 毫秒，返回最大振幅（0–1）。`voice_chat.test` 用它从 CABLE Output
/// 录回测试音，确认虚拟麦克风通了。
pub fn measure_peak(device: &str, ms: u64) -> Result<f32, String> {
    let host = cpal::default_host();
    let input = find_device(&host, device, true)?;
    let config = input
        .default_input_config()
        .map_err(|e| format!("读取「{device}」的格式失败：{e}"))?;
    let peak = Arc::new(Mutex::new(0f32));
    fn build<T>(
        input: &cpal::Device,
        config: &cpal::StreamConfig,
        peak: Arc<Mutex<f32>>,
    ) -> Result<cpal::Stream, String>
    where
        T: cpal::SizedSample,
        f32: cpal::FromSample<T>,
    {
        input
            .build_input_stream(
                config,
                move |data: &[T], _| {
                    let loudest = data
                        .iter()
                        .map(|s| <f32 as cpal::FromSample<T>>::from_sample_(*s).abs())
                        .fold(0f32, f32::max);
                    let mut p = peak.lock().unwrap();
                    *p = p.max(loudest);
                },
                |_| {},
                None,
            )
            .map_err(|e| format!("无法录音：{e}"))
    }
    let stream_config: cpal::StreamConfig = config.clone().into();
    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => build::<f32>(&input, &stream_config, peak.clone()),
        cpal::SampleFormat::I16 => build::<i16>(&input, &stream_config, peak.clone()),
        cpal::SampleFormat::U16 => build::<u16>(&input, &stream_config, peak.clone()),
        other => Err(format!("暂不支持的录音格式 {other}")),
    }?;
    stream.play().map_err(|e| format!("无法录音：{e}"))?;
    std::thread::sleep(std::time::Duration::from_millis(ms));
    drop(stream);
    let value = *peak.lock().unwrap();
    Ok(value)
}

/// 测试音：`seconds` 秒 440 Hz，振幅 0.3，16 kHz。
pub fn test_tone(seconds: f32) -> (Vec<f32>, u32) {
    let rate = 16_000u32;
    let n = (seconds * rate as f32) as usize;
    let tone = (0..n)
        .map(|i| 0.3 * (i as f32 * 440.0 * std::f32::consts::TAU / rate as f32).sin())
        .collect();
    (tone, rate)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mic_passes_through_and_old_audio_is_dropped() {
        let mut m = Mixer::new(1000); // 1 kHz：1 毫秒 1 个采样
        m.push_mic(&[0.1; 50]);
        assert!((m.next_sample() - 0.1).abs() < 1e-6);
        // 超过 200 ms 的部分丢最旧的
        m.push_mic(&vec![0.2; 500]);
        let mut n = 0;
        while m.mic.pop_front().is_some() {
            n += 1;
        }
        assert_eq!(n, 200);
        assert_eq!(m.next_sample(), 0.0, "silence when nothing is buffered");
    }

    #[test]
    fn speech_ducks_the_player_and_is_clamped() {
        let mut m = Mixer::new(1000);
        m.push_mic(&[0.4; 10]);
        assert_eq!(m.push_speech(&[0.5; 3]), 3);
        assert!(m.speaking());
        assert!((m.next_sample() - (0.5 + 0.4 * DUCK_GAIN)).abs() < 1e-6);
        m.push_speech(&[0.95; 5]);
        m.mic_gain = 2.0;
        m.clear_speech();
        assert!(!m.speaking());
        assert!((m.next_sample() - 0.8).abs() < 1e-6, "mic gain applies");
        m.push_speech(&[0.9; 1]);
        m.push_mic(&[0.9; 1]);
        assert_eq!(m.next_sample(), 1.0, "clamped");
    }

    #[test]
    fn speech_queue_is_bounded() {
        let mut m = Mixer::new(10);
        let ms = m.push_speech(&vec![0.1; 10 * (MAX_SPEECH_SECONDS as usize + 5)]);
        assert_eq!(ms, u64::from(MAX_SPEECH_SECONDS) * 1000);
    }

    /// 真机：往 CABLE Input 放一秒 440 Hz，同时从 CABLE Output 录回来，确认声音过去了。
    #[test]
    #[ignore = "needs VB-CABLE on this machine"]
    fn tone_reaches_cable_output() {
        let cable = crate::cable::detect();
        let (Some(input), Some(output)) = (cable.input, cable.output) else {
            panic!("VB-CABLE not installed");
        };
        let host = cpal::default_host();
        let rec = find_device(&host, &output, true).unwrap();
        let rec_config = rec.default_input_config().unwrap();
        let channels = usize::from(rec_config.channels());
        let captured = Arc::new(Mutex::new(Vec::<f32>::new()));
        let sink = captured.clone();
        let stream = rec
            .build_input_stream(
                &rec_config.clone().into(),
                move |data: &[f32], _| {
                    let mut out = sink.lock().unwrap();
                    out.extend(data.chunks(channels).map(|f| f[0]));
                },
                |e| eprintln!("{e}"),
                None,
            )
            .unwrap();
        stream.play().unwrap();
        let router = VoiceRouter::start(RouterConfig {
            output: input,
            mic: None,
            mic_gain: 1.0,
        })
        .unwrap();
        let rate = 16_000;
        let tone: Vec<f32> = (0..rate)
            .map(|i| 0.3 * (i as f32 * 440.0 * std::f32::consts::TAU / rate as f32).sin())
            .collect();
        assert!(router.say(&tone, rate) >= 990);
        std::thread::sleep(std::time::Duration::from_millis(1600));
        assert!(!router.speaking());
        drop(router);
        drop(stream);
        let samples = captured.lock().unwrap();
        let peak = samples.iter().fold(0f32, |m, s| m.max(s.abs()));
        let loud = samples.iter().filter(|s| s.abs() > 0.1).count();
        println!(
            "captured {} samples, peak {peak:.3}, loud {loud}",
            samples.len()
        );
        assert!(
            peak > 0.2 && loud > 1000,
            "the tone should come out of CABLE Output"
        );
    }
}
