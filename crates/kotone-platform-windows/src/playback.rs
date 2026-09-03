//! wav 播放：cpal 输出流把 16kHz mono wav 播到指定输出设备。
//!
//! 典型用法（无人值守音频回路）：播到虚拟声卡 "CABLE Input"（输出设备），
//! 采集侧 audioDeviceId 配 "CABLE Output"（输入设备），`play` 与
//! `listen --no-hotkey --duration` 联合构成全自动链路
//! （scripts/e2e-virtual-audio.sh）。
//!
//! 另含内置提示音（录制/发送，见 `SfxKind` / `play_sfx`）：同步合成、
//! 非阻塞播放，供热键音效反馈使用（无悬浮窗/全屏游戏场景。

use std::path::Path;
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use kotone_core::audio::TARGET_SAMPLE_RATE;

use crate::audio::Resampler;

/// 播放 wav 到输出设备，阻塞直至播完。
/// `device_match`：None/空 = 系统默认输出；Some(子串) = 设备名包含匹配（如 "CABLE Input"）。
pub fn play_wav(path: &Path, device_match: Option<&str>) -> Result<(), String> {
    let pcm = kotone_core::eval::read_wav(path)?; // 16kHz mono f32
    let host = cpal::default_host();
    let device = match device_match {
        None | Some("") => host
            .default_output_device()
            .ok_or_else(|| "未找到系统默认音频输出设备".to_string())?,
        Some(sub) => host
            .output_devices()
            .map_err(|e| format!("枚举音频输出设备失败: {e}"))?
            .find(|d| d.name().map(|n| n.contains(sub)).unwrap_or(false))
            .ok_or_else(|| {
                format!("未找到名称包含「{sub}」的输出设备（kotone-cli devices 可查全部输出设备）")
            })?,
    };
    play_pcm_blocking(&device, &pcm)
}

/// 内置提示音库：录制音（开始监听）/ 发送音（消息发出），各 3 种。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SfxId {
    /// 上扬扫频（录制音默认，「起」）
    Rise,
    /// 双音上行「叮—咚」（微信语音起手式）
    DingUp,
    /// 三连上滑「啁啾」
    ChirpUp,
    /// 下沉扫频（发送音默认，「落」）
    Fall,
    /// 低频双击「叩—叩」
    Knock,
    /// 双音下行「叮咚—」
    DingDown,
}

impl SfxId {
    /// 配置里的稳定 id（soundFeedback.record/send.soundId）
    pub fn from_id(id: &str) -> Option<SfxId> {
        match id {
            "rise" => Some(SfxId::Rise),
            "ding-up" => Some(SfxId::DingUp),
            "chirp-up" => Some(SfxId::ChirpUp),
            "fall" => Some(SfxId::Fall),
            "knock" => Some(SfxId::Knock),
            "ding-down" => Some(SfxId::DingDown),
            _ => None,
        }
    }

    /// 事件默认音：录制 → 上扬；发送 → 下沉（未知 id 回退用）
    pub fn default_for(is_record: bool) -> SfxId {
        if is_record {
            SfxId::Rise
        } else {
            SfxId::Fall
        }
    }
}

/// 播放内置提示音（非阻塞：线程内播完自动销毁输出流）。
/// `gain` 0..=1 为音量增益（调用侧已做配置 clamp，这里再兜底一次）。
/// 失败只输出 stderr，绝不打断语音链路。
pub fn play_sfx(id: SfxId, gain: f32) {
    let pcm = synth_sfx(id, gain.clamp(0.0, 1.0));
    std::thread::spawn(move || {
        let host = cpal::default_host();
        let result = host
            .default_output_device()
            .ok_or_else(|| "未找到系统默认音频输出设备".to_string())
            .and_then(|device| play_pcm_blocking(&device, &pcm));
        if let Err(error) = result {
            eprintln!("[kotone sfx] 提示音播放失败: {error}");
        }
    });
}

/// 合成内置提示音：16kHz mono f32（零资源文件；后续「自定义音效」功能替换播放源）。
fn synth_sfx(id: SfxId, gain: f32) -> Vec<f32> {
    let gain = gain.clamp(0.0, 1.0);
    let mut out = Vec::new();
    match id {
        SfxId::Rise => push_sweep(&mut out, 0.150, 620.0, 1020.0, gain),
        SfxId::Fall => push_sweep(&mut out, 0.200, 980.0, 560.0, gain),
        SfxId::DingUp => {
            push_tone(&mut out, 660.0, 0.060, gain);
            push_gap(&mut out);
            push_tone(&mut out, 990.0, 0.090, gain);
        }
        SfxId::DingDown => {
            push_tone(&mut out, 990.0, 0.070, gain);
            push_gap(&mut out);
            push_tone(&mut out, 660.0, 0.100, gain);
        }
        SfxId::ChirpUp => {
            push_tone(&mut out, 520.0, 0.040, gain);
            push_gap(&mut out);
            push_tone(&mut out, 760.0, 0.040, gain);
            push_gap(&mut out);
            push_tone(&mut out, 1080.0, 0.070, gain);
        }
        SfxId::Knock => {
            push_tone(&mut out, 205.0, 0.045, gain);
            push_gap(&mut out);
            push_tone(&mut out, 190.0, 0.050, gain * 0.9);
        }
    }
    out
}

/// 线性扫频段：相位积分（频率连续变化无相位跳变），12ms 起音 + 尾段 35% 淡出。
fn push_sweep(out: &mut Vec<f32>, duration_s: f32, start_hz: f32, end_hz: f32, gain: f32) {
    let rate = TARGET_SAMPLE_RATE as f32;
    let n = (rate * duration_s) as usize;
    let attack = ((rate * 0.012) as usize).max(1);
    let release_start = n as f32 * 0.65;
    let mut phase = 0.0f32;
    for i in 0..n {
        let t = i as f32 / n as f32;
        let freq = start_hz + (end_hz - start_hz) * t;
        phase += std::f32::consts::TAU * freq / rate;
        out.push(phase.sin() * 0.85 * envelope(i, n, attack, release_start) * gain);
    }
}

/// 定频音段：6ms 起音 + 尾段 20% 淡出（段间由 push_gap 隔开，无声无爆音）。
fn push_tone(out: &mut Vec<f32>, freq_hz: f32, duration_s: f32, gain: f32) {
    let rate = TARGET_SAMPLE_RATE as f32;
    let n = (rate * duration_s) as usize;
    let attack = ((rate * 0.006) as usize).max(1);
    let release_start = n as f32 * 0.80;
    let mut phase = 0.0f32;
    for i in 0..n {
        phase += std::f32::consts::TAU * freq_hz / rate;
        // 定频音可听感偏挤，默认幅度 0.75（扫频用 0.85）
        let amp = if i < attack {
            (i as f32 / attack as f32) * 0.75
        } else {
            0.75
        };
        let amp = if (i as f32) > release_start {
            amp * (1.0 - ((i as f32) - release_start) / (n as f32 - release_start))
        } else {
            amp
        };
        out.push(phase.sin() * amp * gain);
    }
}

/// 5ms 段间静音（双音/双击分隔，避免相位不连续爆音）
fn push_gap(out: &mut Vec<f32>) {
    let n = (TARGET_SAMPLE_RATE as f32 * 0.005) as usize;
    out.resize(out.len() + n, 0.0);
}

/// 包络：线性起音 + 尾部线性淡出
fn envelope(i: usize, n: usize, attack: usize, release_start: f32) -> f32 {
    let mut amp = 1.0;
    if i < attack {
        amp *= i as f32 / attack as f32;
    }
    let rel = i as f32;
    if rel > release_start {
        amp *= 1.0 - (rel - release_start) / (n as f32 - release_start);
    }
    amp
}

/// 播放 16kHz mono f32 PCM 到指定输出设备，阻塞直至播完。
fn play_pcm_blocking(device: &cpal::Device, pcm: &[f32]) -> Result<(), String> {
    let device_name = device.name().unwrap_or_else(|_| "?".into());
    let config = device
        .default_output_config()
        .map_err(|e| format!("读取设备「{device_name}」默认输出配置失败: {e}"))?;

    let rate = config.sample_rate().0;
    let channels = (config.channels() as usize).max(1);

    // 一次性重采样到设备率 + mono 复制到 N 声道（回调内零计算，只搬数据）
    let mut rs = Resampler::new(TARGET_SAMPLE_RATE, rate);
    let mono = rs.process(pcm);
    let mut interleaved = Vec::with_capacity(mono.len() * channels);
    for s in &mono {
        for _ in 0..channels {
            interleaved.push(*s);
        }
    }
    let total = interleaved.len();
    let shared = Arc::new(interleaved);
    let pos = Arc::new(Mutex::new(0usize));

    let stream_config: cpal::StreamConfig = config.clone().into();
    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => {
            build_output_stream::<f32>(&device, &stream_config, shared.clone(), pos.clone())
        }
        cpal::SampleFormat::I16 => {
            build_output_stream::<i16>(&device, &stream_config, shared.clone(), pos.clone())
        }
        fmt => return Err(format!("设备「{device_name}」输出采样格式不支持: {fmt:?}")),
    }
    .map_err(|e| format!("打开设备「{device_name}」输出流失败: {e}"))?;

    stream
        .play()
        .map_err(|e| format!("启动设备「{device_name}」播放失败: {e}"))?;

    // 等待缓冲消费完（回调每次推进 pos）
    loop {
        if *pos.lock().unwrap() >= total {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    // 声卡尾部缓冲余量
    std::thread::sleep(std::time::Duration::from_millis(150));
    Ok(())
}

/// 构建输出流：回调从共享缓冲按序搬样本，播完补静音
fn build_output_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    shared: Arc<Vec<f32>>,
    pos: Arc<Mutex<usize>>,
) -> Result<cpal::Stream, cpal::BuildStreamError>
where
    T: cpal::Sample + cpal::FromSample<f32> + cpal::SizedSample,
{
    device.build_output_stream(
        config,
        move |data: &mut [T], _| {
            let mut p = pos.lock().unwrap();
            for slot in data.iter_mut() {
                let v = if *p < shared.len() { shared[*p] } else { 0.0 };
                *slot = T::from_sample(v);
                *p += 1;
            }
        },
        |e| eprintln!("[kotone play] 播放错误: {e}"),
        None,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zero_crossings(pcm: &[f32], from: usize, to: usize) -> usize {
        let mut count = 0;
        for i in from + 1..to {
            if (pcm[i] >= 0.0) != (pcm[i - 1] >= 0.0) {
                count += 1;
            }
        }
        count
    }

    fn peak_of(pcm: &[f32]) -> f32 {
        pcm.iter().fold(0.0f32, |acc, s| acc.max(s.abs()))
    }

    #[test]
    fn sfx_rise_rises_and_fall_sinks() {
        let rise = synth_sfx(SfxId::Rise, 1.0);
        let fall = synth_sfx(SfxId::Fall, 1.0);

        // 16kHz 时长匹配：150ms / 200ms
        assert_eq!(rise.len(), (TARGET_SAMPLE_RATE as f32 * 0.150) as usize);
        assert_eq!(fall.len(), (TARGET_SAMPLE_RATE as f32 * 0.200) as usize);

        // 起音防御：首样品几乎为零（无爆音）
        assert!(rise[0].abs() < 0.05);

        // 上扬扫频 → 后半段过零率更高；下沉扫频则相反
        let half = rise.len() / 2;
        assert!(
            zero_crossings(&rise, 0, half) < zero_crossings(&rise, half, rise.len()),
            "上扬音后半段频率应更高"
        );
        let half = fall.len() / 2;
        assert!(
            zero_crossings(&fall, 0, half) > zero_crossings(&fall, half, fall.len()),
            "下沉音前半段频率应更高"
        );
    }

    #[test]
    fn sfx_gain_clamps_and_scales() {
        let loud = synth_sfx(SfxId::Rise, 2.0);
        let quiet = synth_sfx(SfxId::Rise, 0.0);
        let mid = synth_sfx(SfxId::Rise, 0.5);

        assert!(peak_of(&loud) <= 0.85 + 1e-4, "gain 超 1 应被钳制");
        assert!(quiet.iter().all(|s| *s == 0.0));
        // 同一样本位置：半增益 = 全长增益的一半
        assert!((mid[1000] - loud[1000] * 0.5).abs() < 1e-4);
    }

    #[test]
    fn sfx_all_variants_valid_and_distinct() {
        let mut collected: Vec<(SfxId, Vec<f32>)> = Vec::new();
        let mut zeroed_seen = false;
        for id in [
            SfxId::Rise,
            SfxId::DingUp,
            SfxId::ChirpUp,
            SfxId::Fall,
            SfxId::Knock,
            SfxId::DingDown,
        ] {
            let pcm = synth_sfx(id, 1.0);
            assert!(!pcm.is_empty(), "{id:?} 应产出非空 PCM");
            assert!(peak_of(&pcm) <= 0.85 + 1e-4, "{id:?} 峰值超界");
            // 多段音（叮咚/啁啾/叩）应含 ≥5ms 静音分隔（80 个零样本）
            let has_gap = pcm.windows(80).any(|w| w.iter().all(|s| *s == 0.0));
            if matches!(
                id,
                SfxId::DingUp | SfxId::ChirpUp | SfxId::Knock | SfxId::DingDown
            ) {
                assert!(has_gap, "{id:?} 应有段间静音");
                zeroed_seen = true;
            }
            assert!(
                collected.iter().all(|(_, prev)| *prev != pcm),
                "{id:?} 与其他音效内容重复"
            );
            collected.push((id, pcm));
        }
        assert_eq!(collected.len(), 6);
        assert!(zeroed_seen);
    }

    #[test]
    fn sfx_ids_parse_and_fallback() {
        for id in [
            SfxId::Rise,
            SfxId::DingUp,
            SfxId::ChirpUp,
            SfxId::Fall,
            SfxId::Knock,
            SfxId::DingDown,
        ] {
            let literal = match id {
                SfxId::Rise => "rise",
                SfxId::DingUp => "ding-up",
                SfxId::ChirpUp => "chirp-up",
                SfxId::Fall => "fall",
                SfxId::Knock => "knock",
                SfxId::DingDown => "ding-down",
            };
            assert_eq!(SfxId::from_id(literal), Some(id));
        }
        assert_eq!(SfxId::from_id("rise-old"), None);
        assert_eq!(SfxId::from_id(""), None);
        assert_eq!(SfxId::default_for(true), SfxId::Rise);
        assert_eq!(SfxId::default_for(false), SfxId::Fall);
    }

    /// 真实输出设备试听（录制音 + 发送音，各播一次）。
    /// 手动执行：`cargo test -p kotone-platform-windows -- --ignored playback::tests::real_sfx`
    #[test]
    #[ignore = "真实扬声器试听：需要本机有输出设备"]
    fn real_sfx_playback_on_default_device() {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .expect("未找到系统默认音频输出设备");
        play_pcm_blocking(&device, &synth_sfx(SfxId::Rise, 0.8)).expect("录制音播放失败");
        play_pcm_blocking(&device, &synth_sfx(SfxId::Fall, 0.8)).expect("发送音播放失败");
    }
}
