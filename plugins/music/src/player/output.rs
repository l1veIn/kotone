//! 输出：解码出来的立体声放进 [`Frames`]，系统默认扬声器（cpal）从里面取着放。设备选择、采样格式、
//! 回调里线性插值重采样的做法照 `kotone-tts/src/stream_player.rs`，这里换成立体声。

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait};

/// 放的这一首的控制：暂停、停、音量（含她说话时的压低）、跳转、放到哪了。
#[derive(Default)]
pub(crate) struct Control {
    pub paused: AtomicBool,
    pub stopped: AtomicBool,
    gain_bits: AtomicU32,
    /// 已经交给设备的源采样帧数（= 放到哪了）。
    pub consumed: AtomicU64,
    /// 源采样率（解码出来才知道）。
    pub rate: AtomicU32,
    seek: Mutex<Option<u64>>,
}

impl Control {
    pub(crate) fn new(gain: f32) -> Arc<Self> {
        let control = Self::default();
        control.set_gain(gain);
        Arc::new(control)
    }

    pub(crate) fn gain(&self) -> f32 {
        f32::from_bits(self.gain_bits.load(Ordering::Relaxed))
    }

    pub(crate) fn set_gain(&self, gain: f32) {
        self.gain_bits
            .store(gain.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }

    pub(crate) fn request_seek(&self, ms: u64) {
        *self.seek.lock().unwrap() = Some(ms);
    }

    pub(crate) fn take_seek(&self) -> Option<u64> {
        self.seek.lock().unwrap().take()
    }

    pub(crate) fn position_ms(&self) -> u64 {
        let rate = self.rate.load(Ordering::Relaxed) as u64;
        (self.consumed.load(Ordering::Relaxed) * 1000)
            .checked_div(rate)
            .unwrap_or(0)
    }
}

/// 解码好的立体声（左右交错），等着放。
#[derive(Default)]
pub(crate) struct Frames {
    samples: Mutex<VecDeque<f32>>,
}

impl Frames {
    pub(crate) fn push_stereo(&self, interleaved: &[f32]) {
        self.samples
            .lock()
            .unwrap()
            .extend(interleaved.iter().copied());
    }

    /// 还有多少帧没放。
    pub(crate) fn frames(&self) -> usize {
        self.samples.lock().unwrap().len() / 2
    }

    pub(crate) fn clear(&self) {
        self.samples.lock().unwrap().clear();
    }

    /// 最多取 `out.len() / 2` 帧，返回取到的帧数。
    fn pull(&self, out: &mut [f32]) -> usize {
        let mut samples = self.samples.lock().unwrap();
        let n = (out.len() / 2).min(samples.len() / 2);
        for (slot, sample) in out.iter_mut().zip(samples.drain(..n * 2)) {
            *slot = sample;
        }
        n
    }
}

/// 把任意声道数的交错样本变成立体声（单声道两边一样，多声道取前两个）。
pub(crate) fn to_stereo(interleaved: &[f32], channels: usize, out: &mut Vec<f32>) {
    out.clear();
    match channels {
        0 => {}
        1 => {
            for s in interleaved {
                out.push(*s);
                out.push(*s);
            }
        }
        2 => out.extend_from_slice(interleaved),
        n => {
            for frame in interleaved.chunks_exact(n) {
                out.push(frame[0]);
                out.push(frame[1]);
            }
        }
    }
}

/// 回调里的立体声线性插值重采样：源采样率 → 设备采样率，跨回调保留位置。
pub(crate) struct Resampler {
    step: f64,
    fraction: f64,
    current: [f32; 2],
    next: [f32; 2],
    buffer: Vec<f32>,
    position: usize,
    len: usize,
}

impl Resampler {
    pub(crate) fn new(from: u32, to: u32) -> Self {
        Self {
            step: from.max(1) as f64 / to.max(1) as f64,
            fraction: 1.0,
            current: [0.0; 2],
            next: [0.0; 2],
            // 一次最多取约 10 ms
            buffer: vec![0.0; ((from as usize / 100).max(16)) * 2],
            position: 0,
            len: 0,
        }
    }

    fn source(&mut self, frames: &Frames, control: &Control) -> Option<[f32; 2]> {
        if self.position == self.len {
            let n = frames.pull(&mut self.buffer);
            if n == 0 {
                return None;
            }
            control.consumed.fetch_add(n as u64, Ordering::Relaxed);
            self.len = n;
            self.position = 0;
        }
        let i = self.position * 2;
        self.position += 1;
        Some([self.buffer[i], self.buffer[i + 1]])
    }

    /// 下一个输出帧；没东西可放（还没解码出来）就是 `None`（放静音，位置不前进）。
    pub(crate) fn next_frame(&mut self, frames: &Frames, control: &Control) -> Option<[f32; 2]> {
        while self.fraction >= 1.0 {
            let sample = self.source(frames, control)?;
            self.current = self.next;
            self.next = sample;
            self.fraction -= 1.0;
        }
        let t = self.fraction as f32;
        let out = [
            self.current[0] + (self.next[0] - self.current[0]) * t,
            self.current[1] + (self.next[1] - self.current[1]) * t,
        ];
        self.fraction += self.step;
        Some(out)
    }

    /// 跳转以后：插值位置从头来。
    pub(crate) fn reset(&mut self) {
        self.fraction = 1.0;
        self.position = 0;
        self.len = 0;
    }
}

/// 打开系统默认扬声器开始取声音。返回流（要留在这个线程里，丢掉就关）和设备采样率。
pub(crate) fn open(
    frames: Arc<Frames>,
    control: Arc<Control>,
    resampler: Arc<Mutex<Resampler>>,
    error: Arc<Mutex<Option<String>>>,
) -> Result<(cpal::Stream, u32), String> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or("没找到系统默认的音频输出设备")?;
    let supported = device.default_output_config().map_err(|e| e.to_string())?;
    let config: cpal::StreamConfig = supported.clone().into();
    let rate = config.sample_rate.0;
    let stream = match supported.sample_format() {
        cpal::SampleFormat::F32 => {
            build::<f32>(&device, &config, frames, control, resampler, error)
        }
        cpal::SampleFormat::I16 => {
            build::<i16>(&device, &config, frames, control, resampler, error)
        }
        cpal::SampleFormat::U16 => {
            build::<u16>(&device, &config, frames, control, resampler, error)
        }
        format => return Err(format!("不支持的音频输出格式：{format}")),
    }
    .map_err(|e| format!("打不开音频输出设备：{e}"))?;
    Ok((stream, rate))
}

fn build<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    frames: Arc<Frames>,
    control: Arc<Control>,
    resampler: Arc<Mutex<Resampler>>,
    error: Arc<Mutex<Option<String>>>,
) -> Result<cpal::Stream, cpal::BuildStreamError>
where
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    let channels = config.channels.max(1) as usize;
    device.build_output_stream(
        config,
        move |output: &mut [T], _| {
            let silent =
                control.paused.load(Ordering::Relaxed) || control.stopped.load(Ordering::Relaxed);
            let gain = control.gain();
            let mut resampler = resampler.lock().unwrap();
            for frame in output.chunks_mut(channels) {
                let [l, r] = if silent {
                    [0.0, 0.0]
                } else {
                    resampler
                        .next_frame(&frames, &control)
                        .unwrap_or([0.0, 0.0])
                };
                let (l, r) = ((l * gain).clamp(-1.0, 1.0), (r * gain).clamp(-1.0, 1.0));
                if channels == 1 {
                    frame[0] = T::from_sample((l + r) / 2.0);
                } else {
                    for (i, slot) in frame.iter_mut().enumerate() {
                        *slot = T::from_sample(match i {
                            0 => l,
                            1 => r,
                            _ => (l + r) / 2.0,
                        });
                    }
                }
            }
        },
        move |failure| {
            *error.lock().unwrap() = Some(format!("音频输出设备出错：{failure}"));
        },
        None,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drain(resampler: &mut Resampler, frames: &Frames, control: &Control) -> Vec<[f32; 2]> {
        std::iter::from_fn(|| resampler.next_frame(frames, control)).collect()
    }

    #[test]
    fn same_rate_passes_frames_through() {
        let frames = Frames::default();
        let control = Control::default();
        frames.push_stereo(&[0.1, -0.1, 0.2, -0.2, 0.3, -0.3]);
        let mut r = Resampler::new(48_000, 48_000);
        let out = drain(&mut r, &frames, &control);
        // 第一帧是起始的静音（插值从 0 开始），之后一一对应
        assert_eq!(out.len(), 3);
        assert_eq!(out[1], [0.1, -0.1]);
        assert_eq!(out[2], [0.2, -0.2]);
        assert_eq!(control.consumed.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn upsampling_44k1_to_48k_makes_more_frames_and_keeps_channels_apart() {
        let frames = Frames::default();
        let control = Control::default();
        let source: Vec<f32> = (0..441)
            .flat_map(|i| [i as f32 / 441.0, -(i as f32) / 441.0])
            .collect();
        frames.push_stereo(&source);
        let mut r = Resampler::new(44_100, 48_000);
        let out = drain(&mut r, &frames, &control);
        assert!((478..=481).contains(&out.len()), "{}", out.len());
        assert!(
            out.iter().all(|[l, r]| (l + r).abs() < 1e-6),
            "left = -right everywhere"
        );
        assert!(
            out.windows(2).skip(1).all(|w| w[1][0] >= w[0][0]),
            "monotonic ramp"
        );
        assert_eq!(frames.frames(), 0);
        r.reset();
        assert!(
            r.next_frame(&frames, &control).is_none(),
            "nothing left: silence"
        );
    }

    #[test]
    fn stereo_conversion_and_control() {
        let mut out = Vec::new();
        to_stereo(&[0.5, 0.25], 1, &mut out);
        assert_eq!(out, vec![0.5, 0.5, 0.25, 0.25]);
        to_stereo(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0], 3, &mut out);
        assert_eq!(out, vec![1.0, 2.0, 4.0, 5.0]);
        let control = Control::new(0.5);
        assert_eq!(control.gain(), 0.5);
        control.set_gain(3.0);
        assert_eq!(control.gain(), 1.0);
        control.rate.store(44_100, Ordering::Relaxed);
        control.consumed.store(44_100 * 3, Ordering::Relaxed);
        assert_eq!(control.position_ms(), 3000);
        control.request_seek(1234);
        assert_eq!(control.take_seek(), Some(1234));
        assert_eq!(control.take_seek(), None);
    }
}
