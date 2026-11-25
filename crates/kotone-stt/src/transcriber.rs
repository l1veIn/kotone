//! 连续切句（ADR 021“耳朵”）：一路 16 kHz 单声道 PCM 进来，Silero VAD 切出一句一句。
//!
//! 只切句，不识别：每一句交给本体的语音识别（[`crate::listen`]，和呼叫、语音输入同一个注册表、
//! 同一份模型；2026-11-25 以前这里自己建了一个固定的 SenseVoice，见共享资源评估第二节第 7 条）。
//! VAD 参数照 auto-caption（MIT）的端点检测，比按键说话宽一些（游戏语音里一句话中间常有停顿）。
//! 识别结果拿不到模型判出的语种，按文字的书写系统粗判（[`guess_language`]），只作提示。

/// 短于 0.25 秒的段不识别（多半是噪声）。
pub const MIN_SEGMENT_SAMPLES: usize = 16_000 / 4;

/// 按书写系统粗判语种：ko / ja / ru / th / zh / en；判不出来返回空。
pub fn guess_language(text: &str) -> &'static str {
    let mut counts = [0usize; 6]; // ko ja ru th zh en
    for c in text.chars() {
        match c as u32 {
            0xAC00..=0xD7AF | 0x1100..=0x11FF | 0x3130..=0x318F => counts[0] += 1,
            0x3040..=0x30FF => counts[1] += 1,
            0x0400..=0x04FF => counts[2] += 1,
            0x0E00..=0x0E7F => counts[3] += 1,
            0x4E00..=0x9FFF => counts[4] += 1,
            0x41..=0x5A | 0x61..=0x7A => counts[5] += 1,
            _ => {}
        }
    }
    // 日文里常夹汉字：有假名就算日文。
    if counts[1] > 0 {
        return "ja";
    }
    let names = ["ko", "ja", "ru", "th", "zh", "en"];
    counts
        .iter()
        .enumerate()
        .max_by_key(|(_, n)| **n)
        .filter(|(_, n)| **n > 0)
        .map(|(i, _)| names[i])
        .unwrap_or("")
}

/// 识别出来的字算不算一句话：少于 2 个字母 / 数字 / 汉字的（“嗯”、标点、空）不算。
pub fn is_sentence(text: &str) -> bool {
    text.chars().filter(|c| c.is_alphanumeric()).count() >= 2
}

/// 一句识别结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Utterance {
    pub text: String,
    pub language: &'static str,
    pub duration_ms: u64,
}

impl Utterance {
    /// 一段音频识别出来的字 → 一句话（不算一句话时 None）。
    pub fn from_text(text: &str, samples: usize) -> Option<Self> {
        let text = text.trim();
        is_sentence(text).then(|| Self {
            language: guess_language(text),
            duration_ms: samples as u64 * 1000 / 16_000,
            text: text.to_string(),
        })
    }
}

#[cfg(feature = "vad-silero")]
mod imp {
    use sherpa_onnx::{VadModelConfig, VoiceActivityDetector};

    use super::MIN_SEGMENT_SAMPLES;

    /// 连续切句器（只在一个线程里用）。
    pub struct Segmenter {
        vad: VoiceActivityDetector,
    }

    impl Segmenter {
        pub fn new() -> Result<Self, String> {
            if !crate::model::vad_model_ready() {
                crate::model::ensure_vad_model()?;
            }
            let mut vad_config = VadModelConfig::default();
            vad_config.silero_vad.model = Some(
                crate::model::vad_model_path()
                    .to_string_lossy()
                    .into_owned(),
            );
            vad_config.silero_vad.threshold = 0.5;
            vad_config.silero_vad.min_speech_duration = 0.25;
            // 一句话中间允许停顿 0.6 秒；最长 15 秒一句。
            vad_config.silero_vad.min_silence_duration = 0.6;
            vad_config.silero_vad.max_speech_duration = 15.0;
            vad_config.silero_vad.window_size = 512;
            vad_config.sample_rate = 16000;
            vad_config.num_threads = 1;
            vad_config.provider = Some("cpu".into());
            let vad = VoiceActivityDetector::create(&vad_config, 60.0)
                .ok_or("语音切句（silero VAD）创建失败")?;
            Ok(Self { vad })
        }

        /// 喂一段 16 kHz 单声道 PCM；返回这段里说完的句子（每句一段 PCM，太短的不要）。
        pub fn push(&mut self, samples: &[f32]) -> Vec<Vec<f32>> {
            self.vad.accept_waveform(samples);
            self.drain()
        }

        /// 结束时把还没说完的一句也切出来。
        pub fn flush(&mut self) -> Vec<Vec<f32>> {
            self.vad.flush();
            self.drain()
        }

        fn drain(&mut self) -> Vec<Vec<f32>> {
            let mut out = Vec::new();
            while let Some(segment) = self.vad.front() {
                let audio = segment.samples().to_vec();
                self.vad.pop();
                if audio.len() >= MIN_SEGMENT_SAMPLES {
                    out.push(audio);
                }
            }
            out
        }
    }
}

#[cfg(feature = "vad-silero")]
pub use imp::Segmenter;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guesses_the_script() {
        assert_eq!(guess_language("뒤에 적 있어요"), "ko");
        assert_eq!(guess_language("後ろに敵がいる"), "ja");
        assert_eq!(guess_language("сзади враг"), "ru");
        assert_eq!(guess_language("enemy behind the house"), "en");
        assert_eq!(guess_language("房子后面有人"), "zh");
        assert_eq!(guess_language("123 !!"), "");
    }

    #[test]
    fn short_noise_is_not_a_sentence() {
        assert_eq!(Utterance::from_text(" 嗯。 ", 16_000), None);
        assert_eq!(Utterance::from_text("", 16_000), None);
        let u = Utterance::from_text(" Enemy behind. ", 24_000).unwrap();
        assert_eq!(u.text, "Enemy behind.");
        assert_eq!(u.language, "en");
        assert_eq!(u.duration_ms, 1500);
    }
}
