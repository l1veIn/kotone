//! 放一首：一个线程里探测格式、用 symphonia 解码（MP3 / FLAC / AAC）、打开扬声器，解码出来的声音
//! 先攒在 [`Frames`] 里（最多 [`AHEAD`]），扬声器那边取着放。跳转、暂停、停都从 [`Control`] 来。

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use cpal::traits::StreamTrait;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::{FormatOptions, SeekMode, SeekTo};
use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::core::units::Time;

use super::download::{Download, Reader};
use super::output::{self, Control, Frames, Resampler};

/// 最多先解码这么多秒。
const AHEAD: u64 = 3;
/// 放完最后一帧以后再等这么久，让设备把缓冲里的尾音放完。
const TAIL: Duration = Duration::from_millis(250);

/// 这一首放得怎么样了（从放歌线程报出来）。
pub(crate) enum Event {
    /// 开始出声了；知道总长就带上。
    Started {
        duration_ms: Option<u64>,
    },
    Ended,
    Failed(String),
}

pub(crate) struct Playback {
    pub control: Arc<Control>,
    download: Arc<Download>,
    thread: Option<JoinHandle<()>>,
}

impl Playback {
    /// 开一个线程放 `download` 里的声音。`extension` 帮着认格式（mp3 / flac / m4a）。
    pub(crate) fn start(
        download: Arc<Download>,
        extension: Option<String>,
        control: Arc<Control>,
        events: Box<dyn Fn(Event) + Send>,
    ) -> Self {
        let thread = {
            let download = download.clone();
            let control = control.clone();
            std::thread::Builder::new()
                .name("kotone-music".into())
                .spawn(move || {
                    let result = run(download, extension, &control, &events);
                    if control.stopped.load(Ordering::SeqCst) {
                        return;
                    }
                    events(match result {
                        Ok(()) => Event::Ended,
                        Err(message) => Event::Failed(message),
                    });
                })
                .expect("spawn music thread")
        };
        Self {
            control,
            download,
            thread: Some(thread),
        }
    }

    /// 停：不报 Ended / Failed。
    pub(crate) fn stop(mut self) {
        self.halt();
    }

    fn halt(&mut self) {
        self.control.stopped.store(true, Ordering::SeqCst);
        self.download.cancel();
        if let Some(thread) = self.thread.take() {
            // 线程自己马上会退（读被打断、循环看到 stopped）；不在这里等，免得卡住调用的地方。
            drop(thread);
        }
    }
}

impl Drop for Playback {
    fn drop(&mut self) {
        self.halt();
    }
}

fn stopped(control: &Control) -> bool {
    control.stopped.load(Ordering::SeqCst)
}

fn run(
    download: Arc<Download>,
    extension: Option<String>,
    control: &Arc<Control>,
    events: &(dyn Fn(Event) + Send),
) -> Result<(), String> {
    let source = MediaSourceStream::new(
        Box::new(Reader::new(download)),
        MediaSourceStreamOptions::default(),
    );
    let mut hint = Hint::new();
    if let Some(ext) = &extension {
        hint.with_extension(ext);
    }
    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            source,
            &FormatOptions {
                enable_gapless: true,
                ..Default::default()
            },
            &MetadataOptions::default(),
        )
        .map_err(|e| describe(e, control))?;
    let mut format = probed.format;
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or("这首歌的文件里没有能放的音轨")?;
    let track_id = track.id;
    let params = track.codec_params.clone();
    let mut decoder = symphonia::default::get_codecs()
        .make(&params, &DecoderOptions::default())
        .map_err(|e| format!("解不了这种音频：{e}"))?;
    let rate = params.sample_rate.unwrap_or(44_100);
    control.rate.store(rate, Ordering::Relaxed);
    let duration_ms = params
        .n_frames
        .map(|frames| frames * 1000 / rate as u64)
        .filter(|ms| *ms > 0);

    let frames = Arc::new(Frames::default());
    let resampler = Arc::new(Mutex::new(Resampler::new(rate, rate)));
    let device_error = Arc::new(Mutex::new(None::<String>));
    let (stream, device_rate) = output::open(
        frames.clone(),
        control.clone(),
        resampler.clone(),
        device_error.clone(),
    )?;
    *resampler.lock().unwrap() = Resampler::new(rate, device_rate);
    stream.play().map_err(|e| format!("放不出声音：{e}"))?;
    events(Event::Started { duration_ms });

    let mut stereo = Vec::new();
    let mut ended = false;
    let mut drained_at: Option<Instant> = None;
    let mut paused = false;
    loop {
        if stopped(control) {
            return Ok(());
        }
        if let Some(error) = device_error.lock().unwrap().take() {
            return Err(error);
        }
        let now_paused = control.paused.load(Ordering::Relaxed);
        if now_paused != paused {
            paused = now_paused;
            let _ = if paused {
                stream.pause().map_err(|e| e.to_string())
            } else {
                stream.play().map_err(|e| e.to_string())
            };
        }
        if let Some(ms) = control.take_seek() {
            let time = Time::new(ms / 1000, (ms % 1000) as f64 / 1000.0);
            match format.seek(
                SeekMode::Coarse,
                SeekTo::Time {
                    time,
                    track_id: Some(track_id),
                },
            ) {
                Ok(seeked) => {
                    decoder.reset();
                    frames.clear();
                    resampler.lock().unwrap().reset();
                    let at = params
                        .time_base
                        .map(|tb| {
                            let t = tb.calc_time(seeked.actual_ts);
                            ((t.seconds as f64 + t.frac) * rate as f64) as u64
                        })
                        .unwrap_or(ms * rate as u64 / 1000);
                    control.consumed.store(at, Ordering::Relaxed);
                    ended = false;
                    drained_at = None;
                }
                Err(e) => kotone_core::log::log(&format!("music: seek failed: {e}")),
            }
        }
        if ended {
            // 解码完了：等扬声器把剩下的放完
            if frames.frames() == 0 {
                let at = *drained_at.get_or_insert_with(Instant::now);
                if at.elapsed() >= TAIL {
                    return Ok(());
                }
            }
            std::thread::sleep(Duration::from_millis(20));
            continue;
        }
        if paused || frames.frames() as u64 > AHEAD * rate as u64 {
            std::thread::sleep(Duration::from_millis(15));
            continue;
        }
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(SymphoniaError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                ended = true;
                continue;
            }
            Err(SymphoniaError::ResetRequired) => {
                decoder = symphonia::default::get_codecs()
                    .make(&params, &DecoderOptions::default())
                    .map_err(|e| format!("解不了这种音频：{e}"))?;
                continue;
            }
            Err(e) => {
                if stopped(control) {
                    return Ok(());
                }
                return Err(describe(e, control));
            }
        };
        if packet.track_id() != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(decoded) => {
                let spec = *decoded.spec();
                let mut samples = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
                samples.copy_interleaved_ref(decoded);
                output::to_stereo(samples.samples(), spec.channels.count(), &mut stereo);
                frames.push_stereo(&stereo);
            }
            // 坏了一小段：跳过这个包
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(SymphoniaError::IoError(_)) => {
                ended = true;
                continue;
            }
            Err(e) => return Err(describe(e, control)),
        }
    }
}

fn describe(error: SymphoniaError, control: &Control) -> String {
    if stopped(control) {
        return "stopped".into();
    }
    match error {
        SymphoniaError::IoError(e) => e.to_string(),
        SymphoniaError::Unsupported(what) => format!("不支持这种音频格式（{what}）"),
        other => format!("这首歌的文件读不了：{other}"),
    }
}
