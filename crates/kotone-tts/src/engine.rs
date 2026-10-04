use std::path::Path;

use kotone_core::speech::{SpeechRequest, SpeechVoice};
use sherpa_onnx::{
    GenerationConfig, OfflineTts, OfflineTtsConfig, OfflineTtsKokoroModelConfig,
    OfflineTtsModelConfig,
};

pub const KOKORO_MODEL_DIR: &str = "kokoro-int8-multi-lang-v1_1";
pub const DEFAULT_VOICE: &str = "kokoro:zf_001";
pub const REQUIRED_FILES: &[&str] = &[
    "model.int8.onnx",
    "voices.bin",
    "tokens.txt",
    "lexicon-us-en.txt",
    "lexicon-zh.txt",
    "phone-zh.fst",
    "date-zh.fst",
    "number-zh.fst",
    "LICENSE",
    "dict/jieba.dict.utf8",
    "dict/hmm_model.utf8",
    "dict/user.dict.utf8",
    "espeak-ng-data/phontab",
    "espeak-ng-data/phonindex",
    "espeak-ng-data/phondata",
    "espeak-ng-data/intonations",
    "espeak-ng-data/en_dict",
];
pub const REQUIRED_DIRS: &[&str] = &["dict", "espeak-ng-data"];

pub fn model_ready(directory: &Path) -> bool {
    REQUIRED_FILES.iter().all(|file| {
        directory
            .join(file)
            .metadata()
            .is_ok_and(|meta| meta.is_file() && meta.len() > 0)
    }) && REQUIRED_DIRS
        .iter()
        .all(|name| directory.join(name).is_dir())
}

// Official v1.1 speaker mapping, not the incompatible Kokoro v1.0 voice list.
const FEMALE_VOICES: &[u8] = &[
    1, 2, 3, 4, 5, 6, 7, 8, 17, 18, 19, 21, 22, 23, 24, 26, 27, 28, 32, 36, 38, 39, 40, 42, 43, 44,
    46, 47, 48, 49, 51, 59, 60, 67, 70, 71, 72, 73, 74, 75, 76, 77, 78, 79, 83, 84, 85, 86, 87, 88,
    90, 92, 93, 94, 99,
];

pub fn kokoro_voices() -> Vec<SpeechVoice> {
    // Three stock voices provide a short, honest listening comparison. The
    // backend also accepts every official Chinese female ID in FEMALE_VOICES.
    [1, 3, 17]
        .into_iter()
        .map(|id| SpeechVoice {
            id: format!("kokoro:zf_{id:03}"),
            name: format!("中文女声 {id:03}{}", if id == 1 { "（默认）" } else { "" }),
        })
        .collect()
}

fn speaker_id(voice: Option<&str>) -> Result<i32, String> {
    let voice = voice
        .filter(|id| !id.trim().is_empty())
        .unwrap_or(DEFAULT_VOICE);
    let suffix = voice
        .strip_prefix("kokoro:zf_")
        .ok_or("请选择 Kokoro 中文女声")?;
    let number: u8 = suffix.parse().map_err(|_| "本地语音声线编号无效")?;
    FEMALE_VOICES
        .iter()
        .position(|id| *id == number)
        .map(|index| index as i32 + 3)
        .ok_or_else(|| "本地语音声线不存在".into())
}

fn speed(rate: i32) -> f32 {
    // A mildly brisk default matches short teammate-style prompts; this changes
    // duration through the model, not pitch by altering the playback sample rate.
    (1.06 * 2.0_f32.powf(rate as f32 / 20.0)).clamp(0.75, 1.5)
}

pub struct SynthesizedSpeech {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
}

pub struct KokoroEngine(OfflineTts);

impl KokoroEngine {
    pub fn open(directory: &Path) -> Result<Self, String> {
        if !directory.to_str().is_some_and(str::is_ascii) {
            return Err("本地语音模型路径必须为纯英文路径，请在模型设置选择英文目录".into());
        }
        for name in REQUIRED_FILES {
            if !directory
                .join(name)
                .metadata()
                .is_ok_and(|meta| meta.is_file() && meta.len() > 0)
            {
                return Err(format!(
                    "琴音本地语音模型尚未安装或不完整，请在助手设置下载（缺少 {name}）"
                ));
            }
        }
        let path = |name: &str| -> Result<String, String> {
            let file = directory.join(name);
            if !file.exists() {
                return Err(format!(
                    "琴音本地语音模型尚未安装或不完整，请在助手设置下载（缺少 {name}）"
                ));
            }
            let value = file.to_str().ok_or("本地语音模型路径无法编码")?;
            if value.contains(['\0', ',']) {
                return Err("本地语音模型路径不能包含空字符或英文逗号".into());
            }
            Ok(value.to_string())
        };
        let model = OfflineTtsKokoroModelConfig {
            model: Some(path("model.int8.onnx")?),
            voices: Some(path("voices.bin")?),
            tokens: Some(path("tokens.txt")?),
            data_dir: Some(path("espeak-ng-data")?),
            dict_dir: Some(path("dict")?),
            lexicon: Some(format!(
                "{},{}",
                path("lexicon-us-en.txt")?,
                path("lexicon-zh.txt")?
            )),
            ..Default::default()
        };
        let config = OfflineTtsConfig {
            model: OfflineTtsModelConfig {
                kokoro: model,
                num_threads: 2,
                provider: Some("cpu".into()),
                ..Default::default()
            },
            rule_fsts: Some(format!(
                "{},{},{}",
                path("phone-zh.fst")?,
                path("date-zh.fst")?,
                path("number-zh.fst")?
            )),
            max_num_sentences: 1,
            silence_scale: 0.2,
            ..Default::default()
        };
        let engine =
            OfflineTts::create(&config).ok_or("无法载入 Kokoro 模型，请重新下载完整模型")?;
        if engine.num_speakers() != 103 || engine.sample_rate() != 24_000 {
            return Err("本地语音模型版本不匹配，需要 Kokoro v1.1 中文版".into());
        }
        Ok(Self(engine))
    }

    pub fn synthesize(
        &self,
        request: &SpeechRequest,
        mut should_continue: impl FnMut() -> bool + 'static,
    ) -> Result<SynthesizedSpeech, String> {
        request.validate()?;
        let config = GenerationConfig {
            sid: speaker_id(request.voice_id.as_deref())?,
            speed: speed(request.rate),
            silence_scale: 0.2,
            ..Default::default()
        };
        let audio = self
            .0
            .generate_with_config(
                &request.text,
                &config,
                Some(move |_: &[f32], _: f32| should_continue()),
            )
            .ok_or("本地语音生成失败")?;
        let samples = audio.samples();
        if samples.is_empty() || samples.iter().any(|value| !value.is_finite()) {
            return Err("本地语音未生成可播放的音频".into());
        }
        Ok(SynthesizedSpeech {
            samples: samples.to_vec(),
            sample_rate: audio.sample_rate() as u32,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stock_voice_ids_match_the_official_v11_mapping() {
        assert_eq!(speaker_id(None).unwrap(), 3);
        assert_eq!(speaker_id(Some("kokoro:zf_017")).unwrap(), 11);
        assert_eq!(speaker_id(Some("kokoro:zf_099")).unwrap(), 57);
        assert!(speaker_id(Some("kokoro:zf_009")).is_err());
        assert!(speaker_id(Some("system-voice")).is_err());
    }

    #[test]
    fn rate_mapping_keeps_supported_range_and_increases_speed() {
        assert!(speed(-10) >= 0.75);
        assert!(speed(10) <= 1.5);
        assert!(speed(-1) < speed(0) && speed(0) < speed(1));
    }
}
