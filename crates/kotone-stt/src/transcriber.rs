//! 连续转写（ADR 021“耳朵”）：一路 16 kHz 单声道 PCM 进来，Silero VAD 切出一句一句，
//! 每句用 SenseVoice（中英日韩粤，自动判别语种）识别成文字。
//!
//! 做法照 auto-caption（MIT）的 sherpa-onnx SenseVoice + 端点检测；VAD 参数比按键说话宽一些
//! （游戏语音里一句话中间常有停顿）。sherpa-onnx 的 Rust 绑定拿不到 SenseVoice 判出的语种，
//! 这里按文字的书写系统粗判（[`guess_language`]），只作提示。

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

/// 一句识别结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Utterance {
    pub text: String,
    pub language: &'static str,
    pub duration_ms: u64,
}

#[cfg(all(feature = "engine-sherpa", feature = "vad-silero"))]
mod imp {
    use std::path::Path;

    use sherpa_onnx::{
        OfflineRecognizer, OfflineRecognizerConfig, OfflineSenseVoiceModelConfig, VadModelConfig,
        VoiceActivityDetector,
    };

    use super::{guess_language, Utterance};

    /// 连续转写器（只在一个线程里用）。
    pub struct Transcriber {
        vad: VoiceActivityDetector,
        recognizer: OfflineRecognizer,
    }

    impl Transcriber {
        /// `model_dir`：SenseVoice 模型目录（含 `model.int8.onnx`、`tokens.txt`）。
        pub fn new(model_dir: &Path) -> Result<Self, String> {
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

            let model = model_dir.join("model.int8.onnx");
            let tokens = model_dir.join("tokens.txt");
            if !model.exists() || !tokens.exists() {
                return Err(
                    "还没有下载 SenseVoice 模型（设置 → 模型 → 语音识别 → SenseVoice 多语言）"
                        .into(),
                );
            }
            let mut config = OfflineRecognizerConfig::default();
            config.model_config.sense_voice = OfflineSenseVoiceModelConfig {
                model: Some(model.to_string_lossy().into_owned()),
                language: Some("auto".into()),
                use_itn: true,
            };
            config.model_config.tokens = Some(tokens.to_string_lossy().into_owned());
            config.model_config.num_threads = 2;
            config.model_config.provider = Some("cpu".into());
            let recognizer = OfflineRecognizer::create(&config).ok_or("SenseVoice 模型加载失败")?;
            Ok(Self { vad, recognizer })
        }

        /// 喂一段 16 kHz 单声道 PCM；返回这段里说完的句子。
        pub fn push(&mut self, samples: &[f32]) -> Vec<Utterance> {
            self.vad.accept_waveform(samples);
            self.drain()
        }

        /// 结束时把还没说完的一句也识别出来。
        pub fn flush(&mut self) -> Vec<Utterance> {
            self.vad.flush();
            self.drain()
        }

        fn drain(&mut self) -> Vec<Utterance> {
            let mut out = Vec::new();
            while let Some(segment) = self.vad.front() {
                let audio = segment.samples().to_vec();
                self.vad.pop();
                if audio.len() < 16000 / 4 {
                    continue;
                }
                let stream = self.recognizer.create_stream();
                stream.accept_waveform(16000, &audio);
                self.recognizer.decode(&stream);
                let text = stream
                    .get_result()
                    .map(|r| r.text.trim().to_string())
                    .unwrap_or_default();
                if text.chars().filter(|c| c.is_alphanumeric()).count() < 2 {
                    continue;
                }
                out.push(Utterance {
                    language: guess_language(&text),
                    duration_ms: audio.len() as u64 * 1000 / 16000,
                    text,
                });
            }
            out
        }
    }
}

#[cfg(all(feature = "engine-sherpa", feature = "vad-silero"))]
pub use imp::Transcriber;

#[cfg(test)]
mod tests {
    use super::guess_language;

    #[test]
    fn guesses_the_script() {
        assert_eq!(guess_language("뒤에 적 있어요"), "ko");
        assert_eq!(guess_language("後ろに敵がいる"), "ja");
        assert_eq!(guess_language("сзади враг"), "ru");
        assert_eq!(guess_language("enemy behind the house"), "en");
        assert_eq!(guess_language("房子后面有人"), "zh");
        assert_eq!(guess_language("123 !!"), "");
    }

    /// 真机：`KOTONE_SENSEVOICE_DIR` 指向 SenseVoice 模型目录，`KOTONE_SPEECH_WAV` 是 16 位单声道 wav。
    #[cfg(all(feature = "engine-sherpa", feature = "vad-silero"))]
    #[test]
    #[ignore = "needs the SenseVoice model and a speech wav"]
    fn transcribes_a_wav() {
        let dir = std::env::var("KOTONE_SENSEVOICE_DIR").unwrap();
        let wav = std::fs::read(std::env::var("KOTONE_SPEECH_WAV").unwrap()).unwrap();
        let rate = u32::from_le_bytes(wav[24..28].try_into().unwrap());
        let data = wav.windows(4).position(|w| w == b"data").unwrap() + 8;
        let pcm: Vec<f32> = wav[data..]
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0)
            .collect();
        // 线性插值重采样到 16 kHz（测试用）
        let step = rate as f64 / 16_000.0;
        let mono: Vec<f32> = (0..(pcm.len() as f64 / step) as usize)
            .map(|i| {
                let x = i as f64 * step;
                let (j, f) = (x as usize, (x.fract()) as f32);
                let a = pcm[j];
                let b = *pcm.get(j + 1).unwrap_or(&a);
                a + (b - a) * f
            })
            .collect();
        let mut t = super::Transcriber::new(std::path::Path::new(&dir)).unwrap();
        let mut out = Vec::new();
        for chunk in mono.chunks(800) {
            out.extend(t.push(chunk));
        }
        out.extend(t.flush());
        println!("{out:?}");
        assert!(!out.is_empty());
    }
}
