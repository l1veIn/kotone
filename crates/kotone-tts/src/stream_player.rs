//! 边收边播：声音一块块追加进 [`PcmQueue`]，输出设备从队列里取着播；可以随时停；
//! 记下已经播出去多少样本，用来算哪几句已经完整念出来了（打断时只把念出来的部分记进对话，
//! 照 LiveKit Agents / Pipecat 的做法）。
//!
//! 设备抽象成 [`AudioDevice`]：正式用 [`DefaultOutputDevice`]（cpal 默认输出设备，与
//! `player.rs` 同样的设备选择、采样格式与线性插值重采样）；测试用假设备。
//! 队列空了不立刻关设备（句子之间、等合成时会短暂没有声音），空了 [`IDLE_CLOSE`] 才关，
//! 有新声音再打开。

use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Condvar, Mutex,
    },
    time::{Duration, Instant},
};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

/// 队列空了这么久就先关掉输出设备，有新声音再打开。
pub const IDLE_CLOSE: Duration = Duration::from_secs(3);
/// 最后一块声音交给设备以后再留这么久，让设备把缓冲里的尾音放完。
const DRAIN: Duration = Duration::from_millis(200);

/// 一次从队列里取声音的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pulled {
    /// 取到这么多个样本（单声道、队列采样率、已乘音量）。
    Audio(usize),
    /// 暂时没有声音，后面还有。
    Waiting,
    /// 全部播完。
    Finished,
    /// 被停了。
    Stopped,
}

/// [`AudioDevice::play`] 为什么返回。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayEnd {
    Finished,
    Stopped,
    /// 队列空了 [`IDLE_CLOSE`]，先关设备；有新声音再调用一次 `play`。
    Idle,
}

/// 追加式 PCM 队列：合成那边往里加，设备这边往外取。
pub struct PcmQueue {
    state: Mutex<QueueState>,
    changed: Condvar,
    stopped: AtomicBool,
}

struct QueueState {
    rate: u32,
    volume: f32,
    /// 临时压低（玩家开口打断时，ADR 033）：1 = 不压。
    gain: f32,
    samples: VecDeque<f32>,
    appended: u64,
    consumed: u64,
    input_done: bool,
    first_audio: Option<Instant>,
}

impl PcmQueue {
    /// 音量 0–100。采样率由第一次 [`append`](Self::append) 决定。
    pub fn new(volume: u16) -> Self {
        Self {
            state: Mutex::new(QueueState {
                rate: 0,
                volume: volume.min(100) as f32 / 100.0,
                gain: 1.0,
                samples: VecDeque::new(),
                appended: 0,
                consumed: 0,
                input_done: false,
                first_audio: None,
            }),
            changed: Condvar::new(),
            stopped: AtomicBool::new(false),
        }
    }

    /// 追加一段单声道声音。采样率与队列不同时先线性插值重采样到队列采样率。
    /// 返回追加后的总样本数（= 这段声音结束的位置）。停了以后不再追加。
    pub fn append(&self, samples: &[f32], rate: u32) -> u64 {
        let mut state = self.state.lock().unwrap();
        if self.stopped.load(Ordering::SeqCst) || rate == 0 {
            return state.appended;
        }
        if state.rate == 0 {
            state.rate = rate;
        }
        if rate == state.rate {
            state.samples.extend(samples.iter().copied());
            state.appended += samples.len() as u64;
        } else {
            let resampled = resample(samples, rate, state.rate);
            state.appended += resampled.len() as u64;
            state.samples.extend(resampled);
        }
        let end = state.appended;
        drop(state);
        self.changed.notify_all();
        end
    }

    /// 不会再追加了：取完就算播完。
    pub fn finish_input(&self) {
        self.state.lock().unwrap().input_done = true;
        self.changed.notify_all();
    }

    /// 立刻停：设备这边下一次取声音就拿到 [`Pulled::Stopped`]，剩下的丢掉。
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        self.state.lock().unwrap().samples.clear();
        self.changed.notify_all();
    }

    pub fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::SeqCst)
    }

    /// 临时压低 / 恢复音量（0–1，1 = 原音量），下一次取声音就生效。玩家开口打断时先压低，
    /// 识别出来不算打断再恢复（ADR 033，照 LiveKit 的“误打断后接着说”）。
    pub fn set_gain(&self, gain: f32) {
        self.state.lock().unwrap().gain = gain.clamp(0.0, 1.0);
    }

    /// 设备这边：取最多 `out.len()` 个样本。在音频回调里调用，只短暂加锁。
    pub fn pull(&self, out: &mut [f32]) -> Pulled {
        if self.is_stopped() {
            return Pulled::Stopped;
        }
        let mut state = self.state.lock().unwrap();
        let n = out.len().min(state.samples.len());
        if n == 0 {
            return if state.input_done {
                Pulled::Finished
            } else {
                Pulled::Waiting
            };
        }
        let volume = state.volume * state.gain;
        for (slot, sample) in out.iter_mut().zip(state.samples.drain(..n)) {
            *slot = sample * volume;
        }
        state.consumed += n as u64;
        state.first_audio.get_or_insert_with(Instant::now);
        Pulled::Audio(n)
    }

    /// 等队列有变化（追加、结束、停），最多等 `timeout`。
    pub fn wait(&self, timeout: Duration) {
        let state = self.state.lock().unwrap();
        if !state.samples.is_empty() || state.input_done || self.is_stopped() {
            return;
        }
        let _ = self.changed.wait_timeout(state, timeout).unwrap();
    }

    /// 有声音可取、已经结束或已经停了。
    pub fn ready(&self) -> bool {
        let state = self.state.lock().unwrap();
        !state.samples.is_empty() || state.input_done || self.is_stopped()
    }

    /// 输入已结束且全部取完。
    pub fn is_drained(&self) -> bool {
        let state = self.state.lock().unwrap();
        state.input_done && state.samples.is_empty()
    }

    pub fn rate(&self) -> u32 {
        self.state.lock().unwrap().rate
    }

    pub fn appended(&self) -> u64 {
        self.state.lock().unwrap().appended
    }

    /// 已经交给设备的样本数。
    pub fn consumed(&self) -> u64 {
        self.state.lock().unwrap().consumed
    }

    /// 已经播出去的毫秒数（交给设备为准，不含设备缓冲里的几十毫秒）。
    pub fn played_ms(&self) -> u64 {
        let state = self.state.lock().unwrap();
        if state.rate == 0 {
            0
        } else {
            state.consumed * 1000 / state.rate as u64
        }
    }

    /// 第一块声音交给设备的时刻。
    pub fn first_audio_at(&self) -> Option<Instant> {
        self.state.lock().unwrap().first_audio
    }
}

/// 线性插值重采样（与 `player.rs` 相同的方法）。
fn resample(samples: &[f32], from: u32, to: u32) -> Vec<f32> {
    if samples.is_empty() || from == 0 || to == 0 {
        return Vec::new();
    }
    let step = from as f64 / to as f64;
    let count = ((samples.len() as f64) / step).floor().max(1.0) as usize;
    (0..count)
        .map(|i| {
            let position = i as f64 * step;
            let index = position as usize;
            let current = samples[index.min(samples.len() - 1)];
            let next = samples.get(index + 1).copied().unwrap_or(current);
            current + (next - current) * (position - index as f64) as f32
        })
        .collect()
}

/// 输出设备：从队列取声音播放。必须在返回前关掉设备。
pub trait AudioDevice: Send + Sync {
    /// 打开设备，一直播到队列播完、被停，或空了 [`IDLE_CLOSE`]。
    fn play(&self, queue: &Arc<PcmQueue>) -> Result<PlayEnd, String>;
}

/// 系统默认输出设备（cpal），与整段播放（`player.rs`）选同一个设备、同样的格式。
pub struct DefaultOutputDevice;

impl AudioDevice for DefaultOutputDevice {
    fn play(&self, queue: &Arc<PcmQueue>) -> Result<PlayEnd, String> {
        let rate = queue.rate();
        if rate == 0 {
            return Ok(if queue.is_stopped() {
                PlayEnd::Stopped
            } else {
                PlayEnd::Finished
            });
        }
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or("未找到系统默认音频输出设备")?;
        let supported = device
            .default_output_config()
            .map_err(|error| error.to_string())?;
        let config: cpal::StreamConfig = supported.clone().into();
        kotone_core::log::log(&format!(
            "speech stream: opening \"{}\" at {} Hz for {rate} Hz audio",
            device.name().unwrap_or_default(),
            config.sample_rate.0
        ));
        let complete = Arc::new(AtomicBool::new(false));
        let error = Arc::new(Mutex::new(None::<String>));
        let resampler = Resampler::new(rate, config.sample_rate.0);
        let stream = match supported.sample_format() {
            cpal::SampleFormat::F32 => build::<f32>(
                &device,
                &config,
                queue.clone(),
                resampler,
                complete.clone(),
                error.clone(),
            ),
            cpal::SampleFormat::I16 => build::<i16>(
                &device,
                &config,
                queue.clone(),
                resampler,
                complete.clone(),
                error.clone(),
            ),
            cpal::SampleFormat::U16 => build::<u16>(
                &device,
                &config,
                queue.clone(),
                resampler,
                complete.clone(),
                error.clone(),
            ),
            format => return Err(format!("暂不支持音频输出格式：{format}")),
        }
        .map_err(|error| format!("无法打开语音输出设备：{error}"))?;
        stream
            .play()
            .map_err(|error| format!("无法播放语音：{error}"))?;
        let mut last_consumed = queue.consumed();
        let mut last_progress = Instant::now();
        let mut finished_at = None;
        loop {
            if queue.is_stopped() {
                return Ok(PlayEnd::Stopped);
            }
            if let Some(error) = error.lock().unwrap().take() {
                return Err(error);
            }
            if complete.load(Ordering::SeqCst)
                && finished_at.get_or_insert_with(Instant::now).elapsed() >= DRAIN
            {
                return Ok(PlayEnd::Finished);
            }
            let consumed = queue.consumed();
            if consumed != last_consumed {
                last_consumed = consumed;
                last_progress = Instant::now();
            } else if last_progress.elapsed() >= IDLE_CLOSE + DRAIN && !queue.ready() {
                return Ok(PlayEnd::Idle);
            }
            queue.wait(Duration::from_millis(10));
            std::thread::sleep(Duration::from_millis(5));
        }
        // `stream` drops on every return path, closing the device before the caller
        // marks playback released.
    }
}

/// 回调里的线性插值重采样：队列采样率 → 设备采样率，跨回调保留插值位置。
struct Resampler {
    step: f64,
    fraction: f64,
    current: f32,
    next: f32,
    buffer: Vec<f32>,
    position: usize,
    len: usize,
}

impl Resampler {
    fn new(from: u32, to: u32) -> Self {
        Self {
            step: from as f64 / to.max(1) as f64,
            fraction: 1.0,
            current: 0.0,
            next: 0.0,
            // 一次最多取约 10 ms：已播位置最多比实际超前这么多。
            buffer: vec![0.0; (from as usize / 100).max(16)],
            position: 0,
            len: 0,
        }
    }

    fn source(&mut self, queue: &PcmQueue, complete: &AtomicBool) -> Option<f32> {
        if self.position == self.len {
            match queue.pull(&mut self.buffer) {
                Pulled::Audio(n) => {
                    self.len = n;
                    self.position = 0;
                }
                Pulled::Finished => {
                    complete.store(true, Ordering::SeqCst);
                    return None;
                }
                Pulled::Waiting | Pulled::Stopped => return None,
            }
        }
        let sample = self.buffer[self.position];
        self.position += 1;
        Some(sample)
    }

    fn sample(&mut self, queue: &PcmQueue, complete: &AtomicBool) -> f32 {
        if queue.is_stopped() {
            return 0.0;
        }
        while self.fraction >= 1.0 {
            match self.source(queue, complete) {
                Some(sample) => {
                    self.current = self.next;
                    self.next = sample;
                    self.fraction -= 1.0;
                }
                // 等合成或已经播完：先放静音，位置不前进
                None => return 0.0,
            }
        }
        let out = self.current + (self.next - self.current) * self.fraction as f32;
        self.fraction += self.step;
        out
    }
}

fn build<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    queue: Arc<PcmQueue>,
    mut resampler: Resampler,
    complete: Arc<AtomicBool>,
    error: Arc<Mutex<Option<String>>>,
) -> Result<cpal::Stream, cpal::BuildStreamError>
where
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    let channels = config.channels.max(1) as usize;
    device.build_output_stream(
        config,
        move |output: &mut [T], _| {
            for frame in output.chunks_mut(channels) {
                let sample = resampler.sample(&queue, &complete);
                frame.fill(T::from_sample(sample.clamp(-1.0, 1.0)));
            }
        },
        move |failure| {
            *error.lock().unwrap() = Some(format!("语音输出设备错误：{failure}"));
        },
        None,
    )
}

#[cfg(test)]
pub(crate) mod fake {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    /// 假设备：按时钟取声音（`speed` 倍实时速度），记下取到的声音。没有声音时不攒欠账。
    pub(crate) struct FakeDevice {
        pub speed: f64,
        pub idle: Duration,
        pub opened: AtomicUsize,
        pub played: Mutex<Vec<f32>>,
        pub fail: bool,
    }

    impl FakeDevice {
        pub(crate) fn new(speed: f64) -> Self {
            Self {
                speed,
                idle: Duration::from_millis(300),
                opened: AtomicUsize::new(0),
                played: Mutex::new(Vec::new()),
                fail: false,
            }
        }
    }

    impl AudioDevice for FakeDevice {
        fn play(&self, queue: &Arc<PcmQueue>) -> Result<PlayEnd, String> {
            self.opened.fetch_add(1, Ordering::SeqCst);
            if self.fail {
                return Err("语音输出设备错误：假设备坏了".into());
            }
            let per_second = queue.rate() as f64 * self.speed;
            let mut buffer = vec![0.0; ((per_second * 0.05) as usize).max(1)];
            let mut last = Instant::now();
            let mut clock = Instant::now();
            let mut credit = 0.0f64;
            loop {
                let now = Instant::now();
                credit += now.duration_since(clock).as_secs_f64() * per_second;
                clock = now;
                let want = (credit as usize).clamp(1, buffer.len());
                match queue.pull(&mut buffer[..want]) {
                    Pulled::Audio(n) => {
                        self.played.lock().unwrap().extend(&buffer[..n]);
                        credit = (credit - n as f64).max(0.0);
                        last = Instant::now();
                    }
                    Pulled::Waiting if last.elapsed() >= self.idle => return Ok(PlayEnd::Idle),
                    Pulled::Waiting => credit = 0.0,
                    Pulled::Finished => return Ok(PlayEnd::Finished),
                    Pulled::Stopped => return Ok(PlayEnd::Stopped),
                }
                std::thread::sleep(Duration::from_millis(2));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::FakeDevice;
    use super::*;

    #[test]
    fn gain_lowers_and_restores_the_next_samples() {
        let queue = PcmQueue::new(100);
        queue.append(&[0.8; 30], 24_000);
        let mut out = [0.0; 10];
        queue.set_gain(0.25);
        assert_eq!(queue.pull(&mut out), Pulled::Audio(10));
        assert!((out[0] - 0.2).abs() < 1e-6);
        queue.set_gain(1.0);
        assert_eq!(queue.pull(&mut out), Pulled::Audio(10));
        assert!((out[0] - 0.8).abs() < 1e-6);
        // 压低不算停：位置照常往前走
        assert_eq!(queue.consumed(), 20);
    }

    #[test]
    fn queue_tracks_appended_and_played_positions_with_volume() {
        let queue = PcmQueue::new(50);
        assert_eq!(queue.append(&[0.5; 240], 24_000), 240);
        assert_eq!(queue.append(&[1.0; 240], 24_000), 480);
        let mut out = [0.0; 300];
        assert_eq!(queue.pull(&mut out), Pulled::Audio(300));
        assert_eq!(out[0], 0.25);
        assert_eq!(out[299], 0.5);
        assert_eq!(queue.consumed(), 300);
        assert_eq!(queue.played_ms(), 12);
        assert!(queue.first_audio_at().is_some());
        assert_eq!(queue.pull(&mut out), Pulled::Audio(180));
        assert_eq!(queue.pull(&mut out), Pulled::Waiting);
        queue.finish_input();
        assert_eq!(queue.pull(&mut out), Pulled::Finished);
        assert!(queue.is_drained());
    }

    #[test]
    fn later_chunks_at_another_rate_are_resampled_to_the_queue_rate() {
        let queue = PcmQueue::new(100);
        queue.append(&[0.0; 10], 24_000);
        assert_eq!(queue.append(&[0.1; 160], 16_000), 10 + 240);
        assert_eq!(queue.rate(), 24_000);
    }

    #[test]
    fn stop_discards_the_rest_and_refuses_new_audio() {
        let queue = PcmQueue::new(100);
        queue.append(&[0.3; 1000], 24_000);
        let mut out = [0.0; 100];
        queue.pull(&mut out);
        queue.stop();
        assert_eq!(queue.pull(&mut out), Pulled::Stopped);
        assert_eq!(queue.append(&[0.3; 1000], 24_000), 1000);
        assert_eq!(queue.consumed(), 100);
    }

    #[test]
    fn device_resampler_interpolates_across_callbacks_and_reports_completion() {
        let queue = PcmQueue::new(100);
        queue.append(&[0.0, 1.0, 0.0, -1.0], 24_000);
        queue.finish_input();
        let complete = AtomicBool::new(false);
        let mut resampler = Resampler::new(24_000, 48_000);
        let out: Vec<f32> = (0..10)
            .map(|_| resampler.sample(&queue, &complete))
            .collect();
        // 第一个点从 0 插到第一个样本；之后每两个输出点前进一个输入样本
        assert_eq!(&out[..7], &[0.0, 0.0, 0.0, 0.5, 1.0, 0.5, 0.0]);
        assert!(complete.load(Ordering::SeqCst));
    }

    #[test]
    fn fake_device_plays_until_finished_and_closes_when_idle() {
        let queue = Arc::new(PcmQueue::new(100));
        let device = FakeDevice::new(50.0);
        queue.append(&[0.2; 4800], 24_000);
        assert_eq!(device.play(&queue).unwrap(), PlayEnd::Idle);
        queue.append(&[0.2; 2400], 24_000);
        queue.finish_input();
        assert_eq!(device.play(&queue).unwrap(), PlayEnd::Finished);
        assert_eq!(device.played.lock().unwrap().len(), 7200);
        assert_eq!(device.opened.load(Ordering::SeqCst), 2);
    }

    /// 打开真实默认输出设备，放 0.6 秒很轻的提示音再停；只在有意要出声时运行。
    #[test]
    #[ignore = "opens the real default audio output device"]
    fn real_device_streams_appended_audio_and_stops() {
        let queue = Arc::new(PcmQueue::new(10));
        let tone: Vec<f32> = (0..24_000)
            .map(|i| (i as f32 * 440.0 * std::f32::consts::TAU / 24_000.0).sin() * 0.2)
            .collect();
        queue.append(&tone[..6000], 24_000);
        let playing = queue.clone();
        let device = std::thread::spawn(move || DefaultOutputDevice.play(&playing));
        std::thread::sleep(Duration::from_millis(150));
        queue.append(&tone[6000..], 24_000);
        std::thread::sleep(Duration::from_millis(450));
        let started = Instant::now();
        queue.stop();
        let end = device.join().unwrap().unwrap();
        println!(
            "real_device_stream: played_ms={}, stop_returned_in={:?}",
            queue.played_ms(),
            started.elapsed()
        );
        assert_eq!(end, PlayEnd::Stopped);
        assert!(started.elapsed() < Duration::from_millis(100));
        assert!(queue.played_ms() > 300 && queue.played_ms() < 1000);
    }
}
