//! wav 播放：cpal 输出流把 16kHz mono wav 播到指定输出设备。
//!
//! 典型用法（无人值守音频回路）：播到虚拟声卡 "CABLE Input"（输出设备），
//! 采集侧 audioDeviceId 配 "CABLE Output"（输入设备），`play` 与
//! `listen --no-hotkey --duration` 联合构成全自动链路
//! （scripts/e2e-virtual-audio.sh）。

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
    let device_name = device.name().unwrap_or_else(|_| "?".into());
    let config = device
        .default_output_config()
        .map_err(|e| format!("读取设备「{device_name}」默认输出配置失败: {e}"))?;

    let rate = config.sample_rate().0;
    let channels = (config.channels() as usize).max(1);

    // 一次性重采样到设备率 + mono 复制到 N 声道（回调内零计算，只搬数据）
    let mut rs = Resampler::new(TARGET_SAMPLE_RATE, rate);
    let mono = rs.process(&pcm);
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
