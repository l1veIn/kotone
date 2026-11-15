//! 在线合成返回的 WAV（RIFF PCM16 / IEEE float）解码为单声道 f32。
//!
//! 流式服务常把 data 块长度写成 0 或 0xFFFFFFFF，这里按“剩余全部字节”处理。

use crate::SynthesizedSpeech;

fn u16_at(b: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(i..i + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?))
}

pub fn decode(bytes: &[u8]) -> Result<SynthesizedSpeech, String> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("语音服务返回的不是 WAV 音频".into());
    }
    let mut pos = 12;
    let mut format: Option<(u16, u16, u32, u16)> = None;
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let declared = u32_at(bytes, pos + 4).unwrap_or(0) as usize;
        let body = pos + 8;
        if id == b"fmt " {
            let tag = u16_at(bytes, body).ok_or("WAV 格式块不完整")?;
            let channels = u16_at(bytes, body + 2).ok_or("WAV 格式块不完整")?;
            let rate = u32_at(bytes, body + 4).ok_or("WAV 格式块不完整")?;
            let bits = u16_at(bytes, body + 14).ok_or("WAV 格式块不完整")?;
            format = Some((tag, channels, rate, bits));
        } else if id == b"data" {
            let (tag, channels, rate, bits) = format.ok_or("WAV 缺少格式块")?;
            let end = if declared == 0
                || declared == u32::MAX as usize
                || body + declared > bytes.len()
            {
                bytes.len()
            } else {
                body + declared
            };
            return pcm(&bytes[body..end], tag, channels, rate, bits);
        }
        pos = body.saturating_add(declared + (declared & 1));
    }
    Err("WAV 缺少音频数据".into())
}

fn pcm(
    data: &[u8],
    tag: u16,
    channels: u16,
    rate: u32,
    bits: u16,
) -> Result<SynthesizedSpeech, String> {
    if channels == 0 || !(8_000..=96_000).contains(&rate) {
        return Err("WAV 声道数或采样率无效".into());
    }
    // 0xFFFE = WAVE_FORMAT_EXTENSIBLE；子格式按位深推断（整型 16 位 / 浮点 32 位）
    let interleaved: Vec<f32> = match (tag, bits) {
        (1 | 0xFFFE, 16) => data
            .chunks_exact(2)
            .map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32_768.0)
            .collect(),
        (3 | 0xFFFE, 32) => data
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect(),
        _ => return Err(format!("暂不支持的 WAV 编码（格式 {tag}，{bits} 位）")),
    };
    let channels = channels as usize;
    let samples: Vec<f32> = interleaved
        .chunks(channels)
        .map(|frame| frame.iter().sum::<f32>() / frame.len() as f32)
        .collect();
    if samples.is_empty() || samples.iter().any(|v| !v.is_finite()) {
        return Err("语音服务没有返回可播放的音频".into());
    }
    Ok(SynthesizedSpeech {
        samples,
        sample_rate: rate,
    })
}

#[cfg(test)]
pub(crate) fn encode_pcm16(samples: &[i16], rate: u32, data_len_override: Option<u32>) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len_override.unwrap_or(data_len).to_le_bytes());
    for s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pcm16_and_streaming_lengths_decode() {
        let wav = encode_pcm16(&[0, 16_384, -16_384], 24_000, None);
        let speech = decode(&wav).unwrap();
        assert_eq!(speech.sample_rate, 24_000);
        assert_eq!(speech.samples, vec![0.0, 0.5, -0.5]);
        let streamed = encode_pcm16(&[1, 2, 3, 4], 24_000, Some(u32::MAX));
        assert_eq!(decode(&streamed).unwrap().samples.len(), 4);
        assert!(decode(b"ID3not-a-wav").is_err());
        assert!(decode(&encode_pcm16(&[], 24_000, None)).is_err());
    }
}
