//! 语音唤醒（ADR 017）：sherpa-onnx 关键词检测（KWS）。
//!
//! - 模型 `sherpa-onnx-kws-zipformer-wenetspeech-3.3M-2024-01-01`（中文，拼音声母 + 韵母建模）：
//!   不随安装包分发，用户开启唤醒时从 k2-fsa 官方 GitHub release 下载整包（约 33 MB），只解出要用的
//!   4 个文件（约 5.5 MB）。模型仓库标注 Apache-2.0；训练数据 WenetSpeech 的许可只允许非商用，
//!   所以由用户自行下载，不由 Kotone 分发（见 THIRD_PARTY_NOTICES.md）。
//! - decoder 用 fp32：int8 decoder 在快语速“你好琴音”上漏检（调研实测）。
//! - 唤醒词是预置的 token 序列（`sherpa-onnx-cli text2token --tokens-type ppinyin` 转出并实测过）；
//!   不提供两个音节的“琴音”，实测会把“秦音也来了”判成唤醒。

use std::path::{Path, PathBuf};

use crate::model::{models_dir, ArchiveSource, ModelFile, MultiFileModel};

pub const KWS_MODEL_ID: &str = "kws-zipformer-wenetspeech-3.3m";
const KWS_DIR: &str = "wake/sherpa-onnx-kws-zipformer-wenetspeech-3.3M-2024-01-01";
const ENCODER: &str = "encoder-epoch-12-avg-2-chunk-16-left-64.int8.onnx";
const DECODER: &str = "decoder-epoch-12-avg-2-chunk-16-left-64.onnx";
const JOINER: &str = "joiner-epoch-12-avg-2-chunk-16-left-64.int8.onnx";
const TOKENS: &str = "tokens.txt";

/// 唤醒模型清单（不在语音识别模型列表里，只给唤醒用）。
pub static KWS_MODEL: MultiFileModel = MultiFileModel {
    id: KWS_MODEL_ID,
    engine_id: "kws",
    display_name: "唤醒词检测（WenetSpeech 3.3M）",
    dir: KWS_DIR,
    files: &[
        ModelFile {
            name: ENCODER,
            url: "",
            sha256: Some("dd784973fc9d2fabb3b800d6dcd20fc3b0ca84f8e2415afe54b032878e447f4d"),
            size_bytes: 4_777_666,
        },
        ModelFile {
            name: DECODER,
            url: "",
            sha256: Some("fb581d6734511676e246e0dff2fea01b31b0913176cb3ca64576dbab0a177774"),
            size_bytes: 675_349,
        },
        ModelFile {
            name: JOINER,
            url: "",
            sha256: Some("f79760052b87239e325f0567c752ad3130b30d92effb847d4307743c20c59a24"),
            size_bytes: 65_242,
        },
        ModelFile {
            name: TOKENS,
            url: "",
            sha256: Some("72316508d9119696145abc6f1f8cdc46287535c34e5ce7e595f845cb1499cf2e"),
            size_bytes: 1_627,
        },
    ],
    archive: Some(ArchiveSource {
        url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/kws-models/sherpa-onnx-kws-zipformer-wenetspeech-3.3M-2024-01-01.tar.bz2",
        sha256: "b2f7c89690dc8ce4c6ed6afeab7cd800c36ad1421fb6b6302b4a4b194cf7f35f",
        size_bytes: 32_654_866,
    }),
    // ModelScope 上同名仓库的文件与官方包不是同一份导出（SHA256 不同），不能当镜像。
    file_mirrors: &[],
    archive_mirrors: &[],
    remote_names: &[],
    io: crate::model::ModelIo::Streaming,
    backend: crate::model::ModelBackend::Sherpa,
    recipe: crate::model::ModelRecipe::ZipformerTransducer,
    config_schema: &[],
};

pub fn kws_model_dir() -> PathBuf {
    models_dir().join(KWS_DIR)
}

/// 模型文件是否齐全（只看存在和大小；SHA256 在下载时校验）。
pub fn kws_model_ready() -> bool {
    kws_model_ready_in(&kws_model_dir())
}

fn kws_model_ready_in(dir: &Path) -> bool {
    KWS_MODEL.files.iter().all(|f| {
        std::fs::metadata(dir.join(f.name))
            .map(|m| m.len() == f.size_bytes)
            .unwrap_or(false)
    })
}

/// 下载唤醒模型（阻塞；与其它模型下载共用互斥与取消）。
pub fn download_kws_model(progress: crate::download::Progress<'_>) -> Result<(), String> {
    crate::model::download_model(&KWS_MODEL, progress)
}

/// 预置唤醒词：(配置值, 显示名, token 序列)。
pub const PHRASES: &[(&str, &str, &str)] = &[
    ("qinyin-qinyin", "琴音琴音", "q ín y īn q ín y īn"),
    ("nihao-qinyin", "你好琴音", "n ǐ h ǎo q ín y īn"),
];

/// 灵敏度 → (keywords_threshold, keywords_score)。阈值越低越容易触发。
pub fn sensitivity_params(sensitivity: &str) -> (f32, f32) {
    match sensitivity {
        "low" => (0.35, 1.0),
        "high" => (0.15, 1.5),
        _ => (0.25, 1.0),
    }
}

/// 给 KWS 的关键词文本（每行一个：`tokens @显示名`）。
pub fn keywords_text(phrase: &str) -> Result<String, String> {
    PHRASES
        .iter()
        .find(|(id, _, _)| *id == phrase)
        .map(|(_, label, tokens)| format!("{tokens} @{label}\n"))
        .ok_or_else(|| format!("未知的唤醒词：{phrase}"))
}

#[cfg(feature = "engine-sherpa")]
pub use detector::WakeDetector;

#[cfg(feature = "engine-sherpa")]
mod detector {
    use super::*;
    use sherpa_onnx::{KeywordSpotter, KeywordSpotterConfig, OnlineStream};

    /// 常驻的唤醒词检测器：喂 16 kHz 单声道 PCM，命中时返回唤醒词。只在一个线程里用。
    pub struct WakeDetector {
        spotter: KeywordSpotter,
        stream: OnlineStream,
    }

    impl WakeDetector {
        pub fn new(dir: &Path, phrase: &str, sensitivity: &str) -> Result<Self, String> {
            if !kws_model_ready_in(dir) {
                return Err("唤醒模型还没有下载".into());
            }
            let (threshold, score) = sensitivity_params(sensitivity);
            let path = |name: &str| Some(dir.join(name).to_string_lossy().into_owned());
            let mut config = KeywordSpotterConfig::default();
            config.model_config.transducer.encoder = path(ENCODER);
            config.model_config.transducer.decoder = path(DECODER);
            config.model_config.transducer.joiner = path(JOINER);
            config.model_config.tokens = path(TOKENS);
            config.model_config.num_threads = 1;
            config.model_config.provider = Some("cpu".into());
            config.keywords_threshold = threshold;
            config.keywords_score = score;
            config.keywords_buf = Some(keywords_text(phrase)?);
            let spotter = KeywordSpotter::create(&config).ok_or("唤醒模型加载失败")?;
            let stream = spotter.create_stream();
            Ok(Self { spotter, stream })
        }

        /// 喂一块音频；命中时返回唤醒词（并重置，准备下一次）。
        pub fn accept(&mut self, samples: &[f32]) -> Option<String> {
            self.stream.accept_waveform(16_000, samples);
            while self.spotter.is_ready(&self.stream) {
                self.spotter.decode(&self.stream);
                if let Some(result) = self.spotter.get_result(&self.stream) {
                    if !result.keyword.is_empty() {
                        self.spotter.reset(&self.stream);
                        return Some(result.keyword);
                    }
                }
            }
            None
        }

        /// 丢掉已经喂进去的音频（暂停后重新开始监听时用）。
        pub fn reset(&mut self) {
            self.stream = self.spotter.create_stream();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keywords_are_preset_and_four_syllables() {
        assert_eq!(
            keywords_text("qinyin-qinyin").unwrap(),
            "q ín y īn q ín y īn @琴音琴音\n"
        );
        assert!(keywords_text("nihao-qinyin")
            .unwrap()
            .ends_with("@你好琴音\n"));
        assert!(keywords_text("qinyin").is_err());
        for (_, _, tokens) in PHRASES {
            // 声母 + 韵母，4 个音节 → 至少 7 个 token（“你好”的 n/ǐ、h/ǎo …）
            assert!(tokens.split(' ').count() >= 7, "{tokens}");
        }
    }

    #[test]
    fn sensitivity_maps_to_thresholds() {
        assert!(sensitivity_params("low").0 > sensitivity_params("normal").0);
        assert!(sensitivity_params("high").0 < sensitivity_params("normal").0);
        assert_eq!(sensitivity_params("unknown"), sensitivity_params("normal"));
    }

    #[test]
    fn readiness_checks_every_file_size() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!kws_model_ready_in(dir.path()));
        for f in KWS_MODEL.files {
            std::fs::write(dir.path().join(f.name), vec![0u8; f.size_bytes as usize]).unwrap();
        }
        assert!(kws_model_ready_in(dir.path()));
        std::fs::write(dir.path().join(TOKENS), b"short").unwrap();
        assert!(!kws_model_ready_in(dir.path()));
    }

    /// 真实模型冒烟：`KOTONE_KWS_TEST_DIR` 指向解开的模型目录、`KOTONE_KWS_TEST_WAV` 指向
    /// 一段含“琴音琴音”的 16 kHz 单声道 WAV。普通测试不运行。
    #[cfg(feature = "engine-sherpa")]
    #[test]
    #[ignore]
    fn real_model_detects_the_phrase() {
        let dir = PathBuf::from(std::env::var("KOTONE_KWS_TEST_DIR").unwrap());
        let wav = std::env::var("KOTONE_KWS_TEST_WAV").unwrap();
        let samples = kotone_core::eval::read_wav(Path::new(&wav)).unwrap();
        let mut detector = WakeDetector::new(&dir, "qinyin-qinyin", "normal").unwrap();
        let mut hits = Vec::new();
        for chunk in samples.chunks(800) {
            if let Some(k) = detector.accept(chunk) {
                hits.push(k);
            }
        }
        // 尾部补 1 秒静音，让最后一个词能解完
        if let Some(k) = detector.accept(&vec![0.0; 16_000]) {
            hits.push(k);
        }
        assert_eq!(hits, vec!["琴音琴音".to_string()]);
    }
}
