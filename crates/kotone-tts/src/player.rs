use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

use crate::{Control, SynthesizedSpeech, CANCELLED};

pub(crate) fn play(
    audio: SynthesizedSpeech,
    volume: u16,
    control: Arc<Control>,
    ticket: u64,
) -> Result<(), String> {
    let _lease = control.begin_playback(ticket)?;
    if volume == 0 {
        return Ok(());
    }
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or("未找到系统默认音频输出设备")?;
    let supported = device
        .default_output_config()
        .map_err(|error| error.to_string())?;
    let config: cpal::StreamConfig = supported.clone().into();
    let duration = Duration::from_secs_f64(audio.samples.len() as f64 / audio.sample_rate as f64);
    let complete = Arc::new(AtomicBool::new(false));
    let error = Arc::new(Mutex::new(None::<String>));
    let data = StreamData {
        audio,
        channels: config.channels as usize,
        output_rate: config.sample_rate.0,
        volume: volume as f32 / 100.0,
        position: 0.0,
        complete: complete.clone(),
        control: control.clone(),
        ticket,
    };
    let stream = match supported.sample_format() {
        cpal::SampleFormat::F32 => stream::<f32>(&device, &config, data, error.clone()),
        cpal::SampleFormat::I16 => stream::<i16>(&device, &config, data, error.clone()),
        cpal::SampleFormat::U16 => stream::<u16>(&device, &config, data, error.clone()),
        format => return Err(format!("暂不支持音频输出格式：{format}")),
    }
    .map_err(|error| format!("无法打开语音输出设备：{error}"))?;
    if !control.current(ticket) {
        return Err(CANCELLED.into());
    }
    stream
        .play()
        .map_err(|error| format!("无法播放本地语音：{error}"))?;
    let started = Instant::now();
    let mut finished_at = None;
    loop {
        if !control.current(ticket) {
            return Err(CANCELLED.into());
        }
        if let Some(error) = error.lock().unwrap().take() {
            return Err(error);
        }
        if complete.load(Ordering::SeqCst) {
            // The callback filled the last buffer; keep the device alive while
            // it drains, otherwise its last syllable may be cut off.
            if finished_at.get_or_insert_with(Instant::now).elapsed() >= Duration::from_millis(200)
            {
                return Ok(());
            }
        }
        if started.elapsed() > duration + Duration::from_secs(3) {
            return Err("本地语音输出设备未及时完成".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    // Locals drop in reverse order: stream closes before _lease releases the
    // capture barrier on every return path, including device errors.
}

struct StreamData {
    audio: SynthesizedSpeech,
    channels: usize,
    output_rate: u32,
    volume: f32,
    position: f64,
    complete: Arc<AtomicBool>,
    control: Arc<Control>,
    ticket: u64,
}

fn stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mut data: StreamData,
    error: Arc<Mutex<Option<String>>>,
) -> Result<cpal::Stream, cpal::BuildStreamError>
where
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    device.build_output_stream(
        config,
        move |output: &mut [T], _| {
            let step = data.audio.sample_rate as f64 / data.output_rate as f64;
            for frame in output.chunks_mut(data.channels) {
                let index = data.position as usize;
                let sample = if data.control.current(data.ticket)
                    && index < data.audio.samples.len()
                {
                    let next = data
                        .audio
                        .samples
                        .get(index + 1)
                        .copied()
                        .unwrap_or(data.audio.samples[index]);
                    let fraction = (data.position - index as f64) as f32;
                    (data.audio.samples[index] * (1.0 - fraction) + next * fraction) * data.volume
                } else {
                    0.0
                };
                #[cfg(test)]
                if sample.abs() > 0.0001 {
                    data.control.rendered_audio.store(true, Ordering::SeqCst);
                }
                frame.fill(T::from_sample(sample.clamp(-1.0, 1.0)));
                data.position += step;
                if data.position as usize >= data.audio.samples.len() {
                    data.complete.store(true, Ordering::SeqCst);
                }
            }
        },
        move |failure| {
            *error.lock().unwrap() = Some(format!("语音输出设备错误：{failure}"));
        },
        None,
    )
}
