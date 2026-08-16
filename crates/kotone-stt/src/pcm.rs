//! 16 kHz mono f32 → s16le，给在线 ASR 适配器共用。

pub(crate) fn pcm_f32_to_s16le(pcm: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(pcm.len() * 2);
    for sample in pcm {
        let clipped = sample.clamp(-1.0, 1.0);
        let int = (clipped * 32768.0).round().clamp(-32768.0, 32767.0) as i16;
        out.extend_from_slice(&int.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pcm_conversion_clamps_and_packs_s16le() {
        let bytes = pcm_f32_to_s16le(&[0.0, 1.0, -1.0, 2.0]);
        assert_eq!(bytes.len(), 8);
        assert_eq!(i16::from_le_bytes([bytes[0], bytes[1]]), 0);
        assert_eq!(i16::from_le_bytes([bytes[2], bytes[3]]), i16::MAX);
        assert_eq!(i16::from_le_bytes([bytes[4], bytes[5]]), i16::MIN);
        assert_eq!(i16::from_le_bytes([bytes[6], bytes[7]]), i16::MAX);
    }
}
