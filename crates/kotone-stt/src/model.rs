//! 模型与运行时清单 + 下载管理（docs/development.md §3.3、ADR-003）。
//!
//! 自管理布局（不依赖 Tauri externalBin，kotone-cli 同样可用）：
//! - whisper-cli 运行时：`~/.kotone/bin/`（whisper-cli.exe + whisper.dll + ggml*.dll）
//! - 模型：`~/.kotone/models/`（ggml-small.bin 等）
//!
//! 清单内置：ID / 大小 / URL / SHA256。下载经 download.rs（流式 + 校验 + 原子落盘）。

use std::fs;
use std::path::PathBuf;

use kotone_core::settings;

use crate::download::{self, Progress};

/// 模型信息（跨引擎统一列出：已下载/可下载）
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub id: String,
    pub engine_id: String,
    pub display_name: String,
    pub size_bytes: u64,
    pub download_url: String,
    pub sha256: String,
    pub downloaded: bool,
}

/// 模型清单条目（静态内置）
pub struct ModelManifest {
    pub id: &'static str,
    pub engine_id: &'static str,
    pub display_name: &'static str,
    pub file: &'static str,
    pub size_bytes: u64,
    pub url: &'static str,
    pub sha256: &'static str,
}

/// whisper.cpp ggml 模型清单（SHA256 取自 HuggingFace LFS oid，2025-01 核对）。
/// 默认 ggml-small（中文效果与体积的平衡点，~466MB）。
pub const MODELS: &[ModelManifest] = &[
    ModelManifest {
        id: "ggml-tiny",
        engine_id: "whisper-cpp-sidecar",
        display_name: "whisper tiny（最快，精度较低）",
        file: "ggml-tiny.bin",
        size_bytes: 77_691_713,
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin",
        sha256: "be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21",
    },
    ModelManifest {
        id: "ggml-base",
        engine_id: "whisper-cpp-sidecar",
        display_name: "whisper base（快，精度一般）",
        file: "ggml-base.bin",
        size_bytes: 147_951_465,
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin",
        sha256: "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe",
    },
    ModelManifest {
        id: "ggml-small",
        engine_id: "whisper-cpp-sidecar",
        display_name: "whisper small（默认，中文推荐）",
        file: "ggml-small.bin",
        size_bytes: 487_601_967,
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin",
        sha256: "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b",
    },
];

/// whisper-cli 运行时的清单条目 ID（作为伪模型出现在 list/download 中）
pub const WHISPER_BIN_ID: &str = "whisper-cli";

/// whisper.cpp 发布包（Windows x64 CPU 版，避开 CUDA 依赖；钉死版本保证 SHA256 稳定）
pub const WHISPER_BIN_VERSION: &str = "v1.9.1";
pub const WHISPER_BIN_URL: &str =
    "https://github.com/ggml-org/whisper.cpp/releases/download/v1.9.1/whisper-bin-x64.zip";
/// whisper-bin-x64.zip 整包 SHA256（v1.9.1，本地实测）
pub const WHISPER_BIN_SHA256: &str =
    "7d8be46ecd31828e1eb7a2ecdd0d6b314feafd82163038ab6092594b0a063539";
pub const WHISPER_BIN_SIZE: u64 = 7_982_101;

// ---------- 路径 ----------

/// ~/.kotone/bin/
pub fn bin_dir() -> PathBuf {
    settings::kotone_dir().join("bin")
}

/// 模型目录：settings.models.dir 非空时用自定义目录，否则默认 ~/.kotone/models。
/// 注意 bin 目录（whisper-cli 运行时）恒为 ~/.kotone/bin，不受自定义路径影响。
pub fn models_dir() -> PathBuf {
    models_dir_from(&settings::load())
}

/// 从给定配置推导模型目录（纯函数，便于壳侧与测试复用）
pub fn models_dir_from(s: &settings::Settings) -> PathBuf {
    let dir = s.models.dir.trim();
    if dir.is_empty() {
        settings::kotone_dir().join("models")
    } else {
        PathBuf::from(dir)
    }
}

/// whisper-cli 可执行文件路径
pub fn whisper_cli_path() -> PathBuf {
    bin_dir().join("whisper-cli.exe")
}

// ---------- sherpa-onnx 多文件模型 ----------

/// 多文件模型中的单个文件（sha256 可选：git 内小文件无 LFS oid，用 size 兜底校验）
pub struct ModelFile {
    pub name: &'static str,
    /// 逐文件直下 URL；所属模型走整包（archive）时置空
    pub url: &'static str,
    pub sha256: Option<&'static str>,
    pub size_bytes: u64,
}

/// 整包下载源（tar.bz2）：部分模型（如 X-ASR 流式变体）只在 k2-fsa GitHub
/// releases 以 tar.bz2 发布，无逐文件镜像——下载整包后按 files 白名单解压校验
pub struct ArchiveSource {
    pub url: &'static str,
    pub sha256: &'static str,
    pub size_bytes: u64,
}

/// 多文件模型清单条目（下载为 ~/.kotone/models/<dir>/<files>）
pub struct MultiFileModel {
    pub id: &'static str,
    pub engine_id: &'static str,
    pub display_name: &'static str,
    pub dir: &'static str,
    pub files: &'static [ModelFile],
    /// Some = 整包下载解压（files 的 url 置空）；None = 逐文件直下
    pub archive: Option<ArchiveSource>,
}

/// sherpa-onnx 模型清单（ADR-004）。默认双语流式 Zipformer（int8 编码器，~200MB）。
/// SHA256 取自 HuggingFace LFS oid（2025-01 核对）；tokens.txt 为 git 内小文件，
/// 无 LFS oid，仅按大小校验。
pub const SHERPA_MODELS: &[MultiFileModel] = &[
    MultiFileModel {
        id: "zipformer-bilingual-zh-en-2023-02-20",
        engine_id: "sherpa-onnx-zipformer-zh",
        display_name: "sherpa 流式 Zipformer 中英双语（int8，低延迟）",
        dir: "sherpa-onnx-streaming-zipformer-bilingual-zh-en-2023-02-20",
        files: &[
            ModelFile {
                name: "encoder-epoch-99-avg-1.int8.onnx",
                url: "https://huggingface.co/csukuangfj/sherpa-onnx-streaming-zipformer-bilingual-zh-en-2023-02-20/resolve/main/encoder-epoch-99-avg-1.int8.onnx",
                sha256: Some("8fa764187a261844f859d7143ebaa563af5d10adfece4c18a8f414c88cba2a9b"),
                size_bytes: 181_895_032,
            },
            ModelFile {
                name: "decoder-epoch-99-avg-1.onnx",
                url: "https://huggingface.co/csukuangfj/sherpa-onnx-streaming-zipformer-bilingual-zh-en-2023-02-20/resolve/main/decoder-epoch-99-avg-1.onnx",
                sha256: Some("2e3b5ec371f8899ee6acd829fd753ba45772df57a91bdf37cde3136354e7db7d"),
                size_bytes: 13_876_452,
            },
            ModelFile {
                name: "joiner-epoch-99-avg-1.int8.onnx",
                url: "https://huggingface.co/csukuangfj/sherpa-onnx-streaming-zipformer-bilingual-zh-en-2023-02-20/resolve/main/joiner-epoch-99-avg-1.int8.onnx",
                sha256: Some("1ed689c5ed19dbaa725d9d191bb4822b5f4855a39e1ffd28cbc1f340d25b2ee0"),
                size_bytes: 3_228_404,
            },
            ModelFile {
                name: "tokens.txt",
                url: "https://huggingface.co/csukuangfj/sherpa-onnx-streaming-zipformer-bilingual-zh-en-2023-02-20/resolve/main/tokens.txt",
                sha256: None,
                size_bytes: 56_317,
            },
        ],
        archive: None,
    },
    // SenseVoice（非流式、多语言）：model.int8.onnx 的 SHA256 取自 HF resolve
    // 头 X-Linked-ETag（LFS sha256，2025-01 核对）；tokens.txt 为 git 内小文件，
    // 无 LFS oid，仅按大小校验（同 zipformer tokens.txt 惯例）
    MultiFileModel {
        id: "sense-voice-zh-en-ja-ko-yue-2024-07-17",
        engine_id: "sherpa-onnx-sensevoice",
        display_name: "sherpa SenseVoice 多语言（int8，非流式高准）",
        dir: "sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17",
        files: &[
            ModelFile {
                name: "model.int8.onnx",
                url: "https://huggingface.co/csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17/resolve/main/model.int8.onnx",
                sha256: Some("c71f0ce00bec95b07744e116345e33d8cbbe08cef896382cf907bf4b51a2cd51"),
                size_bytes: 239_233_841,
            },
            ModelFile {
                name: "tokens.txt",
                url: "https://huggingface.co/csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17/resolve/main/tokens.txt",
                sha256: None,
                size_bytes: 315_894,
            },
        ],
        archive: None,
    },
    // X-ASR（流式 zipformer transducer，中英+标点）：仅在 k2-fsa GitHub releases
    // 以整包 tar.bz2 发布，无逐文件镜像 → 走 archive 整包下载解压。
    // 整包及各文件 SHA256 均为本地从官方 tar 实算（2026-07 核对）。
    MultiFileModel {
        id: "x-asr-480ms-streaming-zh-en-punct-int8-2026-06-05",
        engine_id: "sherpa-onnx-x-asr-zh-en",
        display_name: "X-ASR 流式中英标点（int8，480ms 低延迟）",
        dir: "sherpa-onnx-x-asr-480ms-streaming-zipformer-transducer-zh-en-punct-int8-2026-06-05",
        files: &[
            ModelFile {
                name: "encoder.int8.onnx",
                url: "",
                sha256: Some("908596dcc137a73b95be908ca55e88caa1b3dbbe8027c171615f4b0609c5eb1e"),
                size_bytes: 155_278_641,
            },
            ModelFile {
                name: "decoder.onnx",
                url: "",
                sha256: Some("a1cbc9eac2d5e3fb6617a218c67ad6daaa7f4e0fd225f08b2c22ab0413c8c257"),
                size_bytes: 11_309_084,
            },
            ModelFile {
                name: "joiner.int8.onnx",
                url: "",
                sha256: Some("aedb7fa697b2ab43f20499826fff7c997eea7d67db77be97769aeeeb726e63b3"),
                size_bytes: 2_581_422,
            },
            ModelFile {
                name: "tokens.txt",
                url: "",
                sha256: Some("b818a60878b9aae978cbb8ad594acbd403d76d1af2e31ef4197c84e2dbdba27c"),
                size_bytes: 58_806,
            },
            ModelFile {
                name: "bpe.model",
                url: "",
                sha256: Some("f87a38025a5fdd1e4e9591f6a44bb81295097ce0b80df6f4ab9f44e52c64ca5f"),
                size_bytes: 119_265,
            },
        ],
        archive: Some(ArchiveSource {
            url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-x-asr-480ms-streaming-zipformer-transducer-zh-en-punct-int8-2026-06-05.tar.bz2",
            sha256: "fa5f63d618e5a01526e275a358bb7772e403f84808a4769fba52cffd8160bf74",
            size_bytes: 133_895_136,
        }),
    },
    // FunASR-Nano（非流式，encoder_adaptor + LLM + embedding）：HF 逐文件直下。
    // SHA256 取自 HF resolve 响应头 X-Linked-ETag（LFS sha256，2026-07 核对）；
    // merges.txt 为 git 内小文件（git sha1），仅按大小校验。tokenizer 传 Qwen3-0.6B 目录。
    // 许可证：FunASR 系自定义 Model License（见 HF 仓库 LICENSE）。
    MultiFileModel {
        id: "funasr-nano-int8-2025-12-30",
        engine_id: "sherpa-onnx-funasr-nano",
        display_name: "FunASR-Nano 中英日（int8，非流式）",
        dir: "sherpa-onnx-funasr-nano-int8-2025-12-30",
        files: &[
            ModelFile {
                name: "encoder_adaptor.int8.onnx",
                url: "https://huggingface.co/csukuangfj/sherpa-onnx-funasr-nano-int8-2025-12-30/resolve/main/encoder_adaptor.int8.onnx",
                sha256: Some("f36dea2e30fbc33b5db1d7a7265cc976c5e5586c77b042d5adb1ad27c72db422"),
                size_bytes: 237_792_748,
            },
            ModelFile {
                name: "llm.int8.onnx",
                url: "https://huggingface.co/csukuangfj/sherpa-onnx-funasr-nano-int8-2025-12-30/resolve/main/llm.int8.onnx",
                sha256: Some("dfbf9aa3be41bccc257587f151e15c63fbe1b549f2b517f5ccd5bdce3bf4322a"),
                size_bytes: 600_356_593,
            },
            ModelFile {
                name: "embedding.int8.onnx",
                url: "https://huggingface.co/csukuangfj/sherpa-onnx-funasr-nano-int8-2025-12-30/resolve/main/embedding.int8.onnx",
                sha256: Some("95e61cd0c9c3b9543339a4cf973c95c116815e745ccc1e0285cbd81f76d18644"),
                size_bytes: 155_584_380,
            },
            ModelFile {
                name: "Qwen3-0.6B/merges.txt",
                url: "https://huggingface.co/csukuangfj/sherpa-onnx-funasr-nano-int8-2025-12-30/resolve/main/Qwen3-0.6B/merges.txt",
                sha256: None,
                size_bytes: 1_671_853,
            },
            ModelFile {
                name: "Qwen3-0.6B/tokenizer.json",
                url: "https://huggingface.co/csukuangfj/sherpa-onnx-funasr-nano-int8-2025-12-30/resolve/main/Qwen3-0.6B/tokenizer.json",
                sha256: Some("aeb13307a71acd8fe81861d94ad54ab689df773318809eed3cbe794b4492dae4"),
                size_bytes: 11_422_654,
            },
            ModelFile {
                name: "Qwen3-0.6B/vocab.json",
                url: "https://huggingface.co/csukuangfj/sherpa-onnx-funasr-nano-int8-2025-12-30/resolve/main/Qwen3-0.6B/vocab.json",
                sha256: Some("ca10d7e9fb3ed18575dd1e277a2579c16d108e32f27439684afa0e10b1440910"),
                size_bytes: 2_776_833,
            },
        ],
        archive: None,
    },
    // Qwen3-ASR 0.6B（非流式，Apache 2.0）：HF 逐文件直下，tokenizer 传 tokenizer 目录。
    // SHA256 取自 HF resolve 响应头 X-Linked-ETag（2026-07 核对）；
    // merges.txt 仅按大小校验（与 FunASR-Nano 同一文件，交叉印证）。
    MultiFileModel {
        id: "qwen3-asr-0.6B-int8-2026-03-25",
        engine_id: "sherpa-onnx-qwen3-asr",
        display_name: "Qwen3-ASR 0.6B 多语言（int8，非流式）",
        dir: "sherpa-onnx-qwen3-asr-0.6B-int8-2026-03-25",
        files: &[
            ModelFile {
                name: "conv_frontend.onnx",
                url: "https://huggingface.co/csukuangfj2/sherpa-onnx-qwen3-asr-0.6B-int8-2026-03-25/resolve/main/conv_frontend.onnx",
                sha256: Some("d22dc4423e0940e49884e903d2ea2f7e5567c14fc1aed97e4e26d6b8f208ef9e"),
                size_bytes: 44_148_281,
            },
            ModelFile {
                name: "encoder.int8.onnx",
                url: "https://huggingface.co/csukuangfj2/sherpa-onnx-qwen3-asr-0.6B-int8-2026-03-25/resolve/main/encoder.int8.onnx",
                sha256: Some("60748d3e6744a57c9c91e1b17424a6c2990567e8adceb0783940c03ed98fa9d9"),
                size_bytes: 182_491_662,
            },
            ModelFile {
                name: "decoder.int8.onnx",
                url: "https://huggingface.co/csukuangfj2/sherpa-onnx-qwen3-asr-0.6B-int8-2026-03-25/resolve/main/decoder.int8.onnx",
                sha256: Some("4f6885be5959ae26af3089d38ee7972c5fafbeeb1cf8d5e76eab6d8b61ca5771"),
                size_bytes: 755_914_231,
            },
            ModelFile {
                name: "tokenizer/merges.txt",
                url: "https://huggingface.co/csukuangfj2/sherpa-onnx-qwen3-asr-0.6B-int8-2026-03-25/resolve/main/tokenizer/merges.txt",
                sha256: None,
                size_bytes: 1_671_853,
            },
            ModelFile {
                name: "tokenizer/tokenizer_config.json",
                url: "https://huggingface.co/csukuangfj2/sherpa-onnx-qwen3-asr-0.6B-int8-2026-03-25/resolve/main/tokenizer/tokenizer_config.json",
                sha256: Some("4942d005604266809309cabc9f4e9cb89ce855d59b14681fdc0e1cc62ea26c4c"),
                size_bytes: 12_487,
            },
            ModelFile {
                name: "tokenizer/vocab.json",
                url: "https://huggingface.co/csukuangfj2/sherpa-onnx-qwen3-asr-0.6B-int8-2026-03-25/resolve/main/tokenizer/vocab.json",
                sha256: Some("ca10d7e9fb3ed18575dd1e277a2579c16d108e32f27439684afa0e10b1440910"),
                size_bytes: 2_776_833,
            },
        ],
        archive: None,
    },
];

/// 指定 sherpa 系引擎清单中的默认模型（该引擎清单首条；无清单时 None）
pub fn multi_file_default_model(engine_id: &str) -> Option<&'static str> {
    SHERPA_MODELS
        .iter()
        .find(|m| m.engine_id == engine_id)
        .map(|m| m.id)
}

/// sherpa 引擎的默认模型（engineOptions 未配置或配置了未知 id 时的兜底）
pub fn sherpa_default_model() -> &'static str {
    multi_file_default_model("sherpa-onnx-zipformer-zh").expect("sherpa 模型清单缺失")
}

/// SenseVoice 引擎的默认模型（单模型清单，恒为清单条目）
pub fn sensevoice_default_model() -> &'static str {
    multi_file_default_model("sherpa-onnx-sensevoice").expect("SenseVoice 模型清单缺失")
}

// ---------- silero VAD 模型（ADR-007，单文件） ----------

/// silero VAD 模型 ID（list/download 用）
pub const VAD_MODEL_ID: &str = "silero-vad";
/// VAD 伪引擎 ID（ModelInfo.engine_id 字段；VAD 不是 STT 引擎）
pub const VAD_ENGINE_ID: &str = "vad-silero";
pub const VAD_MODEL_FILE: &str = "silero_vad.onnx";
/// sherpa-onnx 官方 release 托管的 silero VAD（与 VAD 示例同一来源，钉死 URL 保证 SHA256 稳定）
pub const VAD_MODEL_URL: &str =
    "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/silero_vad.onnx";
/// silero_vad.onnx SHA256（2025-07 本地下载实测）
pub const VAD_MODEL_SHA256: &str =
    "9e2449e1087496d8d4caba907f23e0bd3f78d91fa552479bb9c23ac09cbb1fd6";
pub const VAD_MODEL_SIZE: u64 = 643_854;

/// silero VAD 模型路径（~/.kotone/models/silero_vad.onnx）
pub fn vad_model_path() -> PathBuf {
    models_dir().join(VAD_MODEL_FILE)
}

/// silero VAD 模型就绪（存在且大小匹配）
pub fn vad_model_ready() -> bool {
    fs::metadata(vad_model_path())
        .map(|md| md.len() == VAD_MODEL_SIZE)
        .unwrap_or(false)
}

/// 多文件模型目录
pub fn multi_model_dir(model_id: &str) -> Option<PathBuf> {
    SHERPA_MODELS
        .iter()
        .find(|m| m.id == model_id)
        .map(|m| models_dir().join(m.dir))
}

/// 多文件模型是否齐备（全部文件存在且大小匹配）
pub fn multi_model_ready(model_id: &str) -> bool {
    let Some(m) = SHERPA_MODELS.iter().find(|m| m.id == model_id) else {
        return false;
    };
    let dir = models_dir().join(m.dir);
    m.files.iter().all(|f| {
        fs::metadata(dir.join(f.name))
            .map(|md| md.len() == f.size_bytes)
            .unwrap_or(false)
    })
}

/// 模型文件路径
pub fn model_path(model_id: &str) -> Option<PathBuf> {
    MODELS
        .iter()
        .find(|m| m.id == model_id)
        .map(|m| models_dir().join(m.file))
}

/// 引擎当前活动模型 ID（读 config.json 的 engineOptions；
/// 缺省按引擎给默认：whisper → ggml-small，sherpa 系 → 各引擎清单默认模型）
pub fn active_model(engine_id: &str) -> String {
    active_model_from(&settings::load(), engine_id)
}

/// 从给定配置推导活动模型 ID（纯函数：壳侧用 SharedState 推导 restartNeeded，
/// 避免磁盘/内存双读不一致）
pub fn active_model_from(s: &settings::Settings, engine_id: &str) -> String {
    let configured = s
        .engine_options
        .get(engine_id)
        .and_then(|o| o.get("model"))
        .and_then(|m| m.as_str())
        .map(str::to_string);
    let is_multi = SHERPA_MODELS.iter().any(|m| m.engine_id == engine_id);
    match (is_multi, configured) {
        // sherpa 系：配置的 id 必须属于该引擎自己的清单（跨引擎 id 不认），否则
        // 兜底该引擎清单默认（config.json 早期默认值 zipformer-zh-small 是占位串）
        (true, Some(id))
            if SHERPA_MODELS
                .iter()
                .any(|m| m.id == id && m.engine_id == engine_id) =>
        {
            id
        }
        (true, _) => multi_file_default_model(engine_id)
            .expect("sherpa 系引擎模型清单缺失")
            .to_string(),
        (false, Some(id)) => id,
        (false, None) => "ggml-small".to_string(),
    }
}

/// whisper-cli 运行时就绪（exe + 核心 DLL 都在）
pub fn bin_installed() -> bool {
    whisper_cli_path().exists() && bin_dir().join("whisper.dll").exists()
}

// ---------- 清单查询 ----------

/// 列出全部模型（whisper 单文件 + sherpa 多文件）+ whisper-cli 运行时条目
pub fn list() -> Result<Vec<ModelInfo>, String> {
    let mut out: Vec<ModelInfo> = MODELS
        .iter()
        .map(|m| ModelInfo {
            id: m.id.into(),
            engine_id: m.engine_id.into(),
            display_name: m.display_name.into(),
            size_bytes: m.size_bytes,
            download_url: m.url.into(),
            sha256: m.sha256.into(),
            downloaded: models_dir().join(m.file).exists(),
        })
        .collect();
    out.extend(SHERPA_MODELS.iter().map(|m| ModelInfo {
        id: m.id.into(),
        engine_id: m.engine_id.into(),
        display_name: m.display_name.into(),
        size_bytes: m.files.iter().map(|f| f.size_bytes).sum(),
        download_url: m.files.first().map(|f| f.url.into()).unwrap_or_default(),
        sha256: String::new(), // 多文件条目逐文件校验，聚合字段留空
        downloaded: multi_model_ready(m.id),
    }));
    out.push(ModelInfo {
        id: VAD_MODEL_ID.into(),
        engine_id: VAD_ENGINE_ID.into(),
        display_name: "silero VAD 语音活动检测（one-shot 静音判停用）".into(),
        size_bytes: VAD_MODEL_SIZE,
        download_url: VAD_MODEL_URL.into(),
        sha256: VAD_MODEL_SHA256.into(),
        downloaded: vad_model_ready(),
    });
    out.push(ModelInfo {
        id: WHISPER_BIN_ID.into(),
        engine_id: "whisper-cpp-sidecar".into(),
        display_name: format!("whisper-cli 运行时（{WHISPER_BIN_VERSION}，CPU 版）"),
        size_bytes: WHISPER_BIN_SIZE,
        download_url: WHISPER_BIN_URL.into(),
        sha256: WHISPER_BIN_SHA256.into(),
        downloaded: bin_installed(),
    });
    Ok(out)
}

// ---------- 下载 ----------

/// 下载模型或 whisper-cli 运行时（id = ggml-* / zipformer-* / whisper-cli），
/// 进度经回调外发（多文件模型聚合进度）。阻塞实现：调用方放阻塞线程。
pub fn download(id: &str, progress: Progress<'_>) -> Result<(), String> {
    if id == WHISPER_BIN_ID {
        return download_bin(progress);
    }
    if id == VAD_MODEL_ID {
        let dest = vad_model_path();
        return download::download_file(VAD_MODEL_URL, &dest, Some(VAD_MODEL_SHA256), progress);
    }
    if let Some(m) = MODELS.iter().find(|m| m.id == id) {
        let dest = models_dir().join(m.file);
        return download::download_file(m.url, &dest, Some(m.sha256), progress);
    }
    if let Some(m) = SHERPA_MODELS.iter().find(|m| m.id == id) {
        let result = download_multi(m, progress);
        if result.is_ok() {
            maybe_export_bpe_vocab(&models_dir().join(m.dir));
        }
        return result;
    }
    Err(format!(
        "未知模型：{id}（可选：{}）",
        model_ids().join(", ")
    ))
}

/// 多文件模型：整包（archive）或逐文件下载，聚合进度（已完成文件字节 + 当前文件进度）
fn download_multi(m: &MultiFileModel, progress: Progress<'_>) -> Result<(), String> {
    if let Some(archive) = &m.archive {
        return download_archive(m, archive, progress);
    }
    let dir = models_dir().join(m.dir);
    fs::create_dir_all(&dir).map_err(|e| format!("无法创建目录 {}：{e}", dir.display()))?;
    let total: u64 = m.files.iter().map(|f| f.size_bytes).sum();
    let mut done: u64 = 0;
    for f in m.files {
        let dest = dir.join(f.name);
        // 已存在且大小匹配 → 跳过（重跑时天然续传）
        if fs::metadata(&dest).map(|md| md.len() == f.size_bytes).unwrap_or(false) {
            done += f.size_bytes;
            progress(done, Some(total));
            continue;
        }
        let base = done;
        download::download_file(f.url, &dest, f.sha256, &|d, t| {
            // 单文件 total 不可靠时仍保证聚合 total 准确
            let _ = t;
            progress(base + d, Some(total));
        })?;
        done += f.size_bytes;
        progress(done, Some(total));
    }
    Ok(())
}

/// 整包模型（tar.bz2）：下载整包校验后，只按 files 白名单提取到模型目录，
/// 再逐文件校验大小 + SHA256，最后删除整包（失败同样清理，不留半成品）。
fn download_archive(
    m: &MultiFileModel,
    archive: &ArchiveSource,
    progress: Progress<'_>,
) -> Result<(), String> {
    // 已齐备 → 幂等直接完成
    if multi_model_ready(m.id) {
        progress(archive.size_bytes, Some(archive.size_bytes));
        return Ok(());
    }
    let dir = models_dir().join(m.dir);
    fs::create_dir_all(&dir).map_err(|e| format!("无法创建目录 {}：{e}", dir.display()))?;
    let tar_path = models_dir().join(format!("{}.tar.bz2", m.dir));

    let result = download_archive_inner(m, archive, &tar_path, &dir, progress);
    // 成功失败都删整包：失败不留半成品，成功不再占用磁盘
    let _ = fs::remove_file(&tar_path);
    result
}

fn download_archive_inner(
    m: &MultiFileModel,
    archive: &ArchiveSource,
    tar_path: &PathBuf,
    dir: &PathBuf,
    progress: Progress<'_>,
) -> Result<(), String> {
    download::download_file(archive.url, tar_path, Some(archive.sha256), progress)?;

    let file = fs::File::open(tar_path)
        .map_err(|e| format!("无法打开整包 {}：{e}", tar_path.display()))?;
    let decoder = bzip2::read::BzDecoder::new(file);
    let mut tar = tar::Archive::new(decoder);

    let mut extracted = 0usize;
    let entries = tar
        .entries()
        .map_err(|e| format!("整包损坏（tar 解析失败）：{e}"))?;
    for entry in entries {
        let mut entry = entry.map_err(|e| format!("整包读取失败：{e}"))?;
        let path = entry
            .path()
            .map_err(|e| format!("整包条目路径异常：{e}"))?
            .to_path_buf();
        let name = path.to_string_lossy().replace('\\', "/");
        // 白名单匹配：tar 条目通常带顶层目录前缀，按 files 相对路径后缀匹配；
        // 清单 name 是受控值（不含 .. / 盘符），dir.join(name) 无穿越风险
        let Some(f) = m
            .files
            .iter()
            .find(|f| name == f.name || name.ends_with(&format!("/{}", f.name)))
        else {
            continue;
        };
        let out = dir.join(f.name);
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("无法创建目录 {}：{e}", parent.display()))?;
        }
        entry
            .unpack(&out)
            .map_err(|e| format!("解压 {} 失败：{e}", f.name))?;
        extracted += 1;
    }
    if extracted != m.files.len() {
        return Err(format!(
            "整包内容不符：期望 {} 个文件，实际提取 {extracted} 个（包结构可能已变更）",
            m.files.len()
        ));
    }

    // 逐文件校验（大小 + SHA256）：整包校验只保证包本身，提取后逐文件再核一遍
    for f in m.files {
        let out = dir.join(f.name);
        let md = fs::metadata(&out)
            .map_err(|e| format!("解压后缺少 {}：{e}", out.display()))?;
        if md.len() != f.size_bytes {
            return Err(format!(
                "文件 {} 大小不符：期望 {}，实际 {}",
                f.name,
                f.size_bytes,
                md.len()
            ));
        }
        if let Some(expected) = f.sha256 {
            let actual = download::sha256_file(&out)?;
            if !actual.eq_ignore_ascii_case(expected) {
                return Err(format!(
                    "文件 {} SHA256 校验失败：期望 {expected}，实际 {actual}",
                    f.name
                ));
            }
        }
    }
    Ok(())
}

/// 可选模型 ID 列表（错误提示用）
fn model_ids() -> Vec<&'static str> {
    MODELS
        .iter()
        .map(|m| m.id)
        .chain(SHERPA_MODELS.iter().map(|m| m.id))
        .chain([VAD_MODEL_ID])
        .collect()
}

// ---------- bpe.vocab（cjkchar+bpe 模型热词用的文本词表） ----------

/// 解析 sentencepiece bpe.model（二进制 protobuf），导出 sherpa-onnx 热词用的
/// 文本 bpe.vocab（每行「token<TAB>score」，对齐官方 scripts/export_bpe_vocab.py）。
/// 只依赖 protobuf wire format（pieces=field 1；Piece.piece=field 1 string，
/// Piece.score=field 2 fixed32 float），不引 sentencepiece 依赖。
/// 返回词片数。
pub fn export_bpe_vocab(bpe_model: &std::path::Path, out: &std::path::Path) -> Result<usize, String> {
    let data = fs::read(bpe_model)
        .map_err(|e| format!("无法读取 {}：{e}", bpe_model.display()))?;
    let mut pieces: Vec<(String, f32)> = Vec::new();
    let mut i = 0usize;
    while i < data.len() {
        let key = pb_varint(&data, &mut i)?;
        let (field, wire) = (key >> 3, key & 7);
        if field == 1 && wire == 2 {
            let n = pb_varint(&data, &mut i)? as usize;
            if i + n > data.len() {
                return Err("bpe.model 损坏（piece 消息截断）".into());
            }
            let msg = &data[i..i + n];
            i += n;
            pieces.push(pb_parse_piece(msg)?);
        } else {
            pb_skip(&data, &mut i, wire)?;
        }
    }
    if pieces.is_empty() {
        return Err("bpe.model 中未解析到任何词片（不是 sentencepiece 模型？）".into());
    }
    let mut text = String::new();
    for (piece, score) in &pieces {
        text.push_str(piece);
        text.push('\t');
        text.push_str(&score.to_string());
        text.push('\n');
    }
    fs::write(out, text).map_err(|e| format!("无法写入 {}：{e}", out.display()))?;
    Ok(pieces.len())
}

/// 文本 bpe.vocab 格式探测（P0 防御：C++ 解析失败会直接 exit 进程，
/// 任何不合格输入都不得传下去）。抽样式检查：前 8KB 必须是合法 UTF-8、
/// 无 NUL，且抽到的每个非空行都是「token score」两列、score 可解析为浮点。
pub fn is_valid_bpe_vocab(path: &std::path::Path) -> bool {
    use std::io::Read;
    let Ok(mut f) = fs::File::open(path) else {
        return false;
    };
    let mut buf = vec![0u8; 8192];
    let Ok(n) = f.read(&mut buf) else {
        return false;
    };
    if n == 0 {
        return false;
    }
    // 8KB 边界可能截断多字节字符：退到最近的有效 UTF-8 前缀
    let text = match std::str::from_utf8(&buf[..n]) {
        Ok(t) => t,
        Err(e) if e.valid_up_to() > 0 => std::str::from_utf8(&buf[..e.valid_up_to()]).unwrap(),
        Err(_) => return false,
    };
    if text.contains('\0') {
        return false;
    }
    let mut checked = 0usize;
    for line in text.lines() {
        let line = line.trim_end();
        if line.is_empty() {
            continue;
        }
        let mut it = line.split_whitespace();
        let (Some(_token), Some(score)) = (it.next(), it.next()) else {
            return false;
        };
        if score.parse::<f64>().is_err() {
            return false;
        }
        checked += 1;
        if checked >= 16 {
            break;
        }
    }
    checked > 0
}

/// 解析可用的 bpe.vocab 路径（热词门控用）：
/// 已存在且格式合格 → 直接用；缺失/不合格但目录里有 bpe.model → 现场导出
/// 兜底（覆盖修复前已下载的老安装）；都不行 → None（调用方降级，不传 C 侧）。
pub fn resolve_bpe_vocab(dir: &std::path::Path, vocab_name: &str) -> Option<PathBuf> {
    let vocab = dir.join(vocab_name);
    if is_valid_bpe_vocab(&vocab) {
        return Some(vocab);
    }
    let model = dir.join("bpe.model");
    if model.is_file()
        && export_bpe_vocab(&model, &vocab).is_ok()
        && is_valid_bpe_vocab(&vocab)
    {
        kotone_core::log::log(&format!(
            "已从 bpe.model 导出热词词表 {}",
            vocab.display()
        ));
        return Some(vocab);
    }
    None
}

/// 下载后处理：模型含 bpe.model 时顺手导出 bpe.vocab（尽力而为，失败不影响下载结果）
fn maybe_export_bpe_vocab(dir: &std::path::Path) {
    let model = dir.join("bpe.model");
    let vocab = dir.join("bpe.vocab");
    if !model.is_file() || is_valid_bpe_vocab(&vocab) {
        return;
    }
    match export_bpe_vocab(&model, &vocab) {
        Ok(n) => kotone_core::log::log(&format!(
            "已导出热词词表 {}（{n} 词片）",
            vocab.display()
        )),
        Err(e) => kotone_core::log::log(&format!("导出 bpe.vocab 失败（热词降级）：{e}")),
    }
}

fn pb_varint(b: &[u8], i: &mut usize) -> Result<u64, String> {
    let mut v: u64 = 0;
    let mut shift = 0u32;
    loop {
        if *i >= b.len() {
            return Err("protobuf 截断（varint）".into());
        }
        let byte = b[*i];
        *i += 1;
        v |= ((byte & 0x7f) as u64) << shift;
        if byte & 0x80 == 0 {
            return Ok(v);
        }
        shift += 7;
        if shift >= 64 {
            return Err("protobuf varint 过长".into());
        }
    }
}

fn pb_skip(b: &[u8], i: &mut usize, wire: u64) -> Result<(), String> {
    match wire {
        0 => {
            pb_varint(b, i)?;
        }
        1 => *i += 8,
        2 => *i += pb_varint(b, i)? as usize,
        5 => *i += 4,
        _ => return Err(format!("protobuf 未知 wire type {wire}")),
    }
    if *i > b.len() {
        return Err("protobuf 截断（字段）".into());
    }
    Ok(())
}

/// 解析 sentencepiece Piece 子消息 → (token, score)
fn pb_parse_piece(msg: &[u8]) -> Result<(String, f32), String> {
    let mut piece: Option<String> = None;
    let mut score: f32 = 0.0;
    let mut j = 0usize;
    while j < msg.len() {
        let key = pb_varint(msg, &mut j)?;
        let (field, wire) = (key >> 3, key & 7);
        match (field, wire) {
            (1, 2) => {
                let n = pb_varint(msg, &mut j)? as usize;
                if j + n > msg.len() {
                    return Err("bpe.model 损坏（piece token 截断）".into());
                }
                piece = Some(
                    std::str::from_utf8(&msg[j..j + n])
                        .map_err(|_| "bpe.model 损坏（token 非 UTF-8）".to_string())?
                        .to_string(),
                );
                j += n;
            }
            (2, 5) => {
                if j + 4 > msg.len() {
                    return Err("bpe.model 损坏（piece score 截断）".into());
                }
                score = f32::from_le_bytes(msg[j..j + 4].try_into().unwrap());
                j += 4;
            }
            _ => pb_skip(msg, &mut j, wire)?,
        }
    }
    piece
        .map(|p| (p, score))
        .ok_or_else(|| "bpe.model 损坏（piece 缺少 token）".to_string())
}

/// 下载并安装 whisper-cli 运行时：
/// zip 下载校验后，只取运行必需文件（whisper-cli.exe / whisper.dll / ggml*.dll），
/// 先解压到 staging 目录再逐个 rename 进 bin/（同卷原子），最后清理。
fn download_bin(progress: Progress<'_>) -> Result<(), String> {
    let bin = bin_dir();
    let staging = bin.join(".staging");
    let zip_path = bin.join(".whisper-bin.zip");

    let _ = fs::remove_dir_all(&staging);
    fs::create_dir_all(&staging).map_err(|e| format!("无法创建目录 {}：{e}", staging.display()))?;

    // 失败时清场，不留半成品
    let result = download_bin_inner(&zip_path, &staging, &bin, progress);
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
        let _ = fs::remove_file(&zip_path);
    }
    result
}

fn download_bin_inner(
    zip_path: &PathBuf,
    staging: &PathBuf,
    bin: &PathBuf,
    progress: Progress<'_>,
) -> Result<(), String> {
    download::download_file(WHISPER_BIN_URL, zip_path, Some(WHISPER_BIN_SHA256), progress)?;

    let file = fs::File::open(zip_path).map_err(|e| format!("无法打开 {WHISPER_BIN_URL}：{e}"))?;
    let mut zip =
        zip::ZipArchive::new(file).map_err(|e| format!("whisper 发布包损坏（zip 解析失败）：{e}"))?;

    let mut extracted = 0usize;
    for i in 0..zip.len() {
        let mut entry = zip
            .by_index(i)
            .map_err(|e| format!("whisper 发布包读取失败：{e}"))?;
        let name = entry.name().rsplit('/').next().unwrap_or("").to_string();
        let wanted = name == "whisper-cli.exe"
            || name == "whisper.dll"
            || (name.starts_with("ggml") && name.ends_with(".dll"));
        if !wanted {
            continue;
        }
        let out = staging.join(&name);
        let mut f = fs::File::create(&out)
            .map_err(|e| format!("无法写入 {}：{e}", out.display()))?;
        std::io::copy(&mut entry, &mut f).map_err(|e| format!("解压 {name} 失败：{e}"))?;
        extracted += 1;
    }
    if extracted == 0 || !staging.join("whisper-cli.exe").exists() {
        return Err("whisper 发布包中未找到 whisper-cli.exe（包结构可能已变更）".into());
    }

    // staging → bin/ 逐个 rename（同卷原子；旧文件先删）
    for entry in fs::read_dir(staging).map_err(|e| format!("读取 staging 失败：{e}"))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let dest = bin.join(entry.file_name());
        if dest.exists() {
            fs::remove_file(&dest).map_err(|e| format!("无法替换 {}：{e}", dest.display()))?;
        }
        fs::rename(entry.path(), &dest).map_err(|e| format!("安装 {} 失败：{e}", dest.display()))?;
    }

    let _ = fs::remove_dir_all(staging);
    let _ = fs::remove_file(zip_path);
    Ok(())
}

// ---------- 模型目录迁移与删除 ----------

/// 目录迁移报告：moved = 成功移动的顶层条目名；failed = 移动失败的条目（需重新下载）
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrateReport {
    pub moved: Vec<String>,
    pub failed: Vec<String>,
}

/// 把 `src` 目录的全部顶层条目移动到 `dst`（fs::rename 优先，跨卷回退复制+删除）。
/// 逐条目容错：单个条目失败不阻断其余迁移，失败名记入 report.failed。
pub fn migrate_dir_contents(src: &PathBuf, dst: &PathBuf) -> Result<MigrateReport, String> {
    if src == dst {
        return Err("新目录与当前目录相同".into());
    }
    if !src.exists() {
        // 旧目录不存在（还没下载过模型）：直接建空新目录即可
        fs::create_dir_all(dst).map_err(|e| format!("无法创建目录 {}：{e}", dst.display()))?;
        return Ok(MigrateReport::default());
    }
    fs::create_dir_all(dst).map_err(|e| format!("无法创建目录 {}：{e}", dst.display()))?;
    let mut report = MigrateReport::default();
    for entry in fs::read_dir(src).map_err(|e| format!("读取目录 {} 失败：{e}", src.display()))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let target = dst.join(&name);
        if target.exists() {
            // 目标已有同名条目：视为已迁移（保留目标，删除源以完成合并）
            if remove_entry(&entry.path()).is_ok() {
                report.moved.push(name);
            } else {
                report.failed.push(name);
            }
            continue;
        }
        match fs::rename(entry.path(), &target) {
            Ok(()) => report.moved.push(name),
            Err(_) => {
                // 跨卷等情况：复制 + 删除
                if copy_entry(&entry.path(), &target).and_then(|_| remove_entry(&entry.path())).is_ok()
                {
                    report.moved.push(name);
                } else {
                    report.failed.push(name);
                }
            }
        }
    }
    Ok(report)
}

fn remove_entry(p: &PathBuf) -> std::io::Result<()> {
    if p.is_dir() {
        fs::remove_dir_all(p)
    } else {
        fs::remove_file(p)
    }
}

fn copy_entry(src: &PathBuf, dst: &PathBuf) -> std::io::Result<()> {
    if src.is_dir() {
        fs::create_dir_all(dst)?;
        for entry in fs::read_dir(src)? {
            let entry = entry?;
            copy_entry(&entry.path(), &dst.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        fs::copy(src, dst).map(|_| ())
    }
}

/// 删除模型的结果：was_active = 被删的是引擎当前活动模型（active 标记已清除，回退默认）
#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteOutcome {
    pub was_active: bool,
}

/// 删除已下载模型 / whisper-cli 运行时（幂等：文件不存在视为成功）。
/// 多文件模型删整个目录；whisper-cli 删 bin 下运行必需文件；
/// active 模型被删时清 engineOptions 的 active 标记（回退引擎默认模型）。
pub fn delete(id: &str) -> Result<DeleteOutcome, String> {
    delete_files_in(&models_dir(), &bin_dir(), id)?;

    // active 标记清除：被删模型恰是该引擎当前活动模型时
    let engine_id = engine_of(id).ok_or_else(|| format!("未知模型：{id}"))?;
    let mut outcome = DeleteOutcome::default();
    if id != WHISPER_BIN_ID && id != VAD_MODEL_ID && active_model(engine_id) == id {
        let mut s = settings::load();
        if let Some(opts) = s.engine_options.as_object_mut() {
            if let Some(entry) = opts.get_mut(engine_id).and_then(|e| e.as_object_mut()) {
                entry.remove("model");
            }
        }
        settings::save(&s)?;
        outcome.was_active = true;
    }
    Ok(outcome)
}

/// 模型所属引擎（VAD / whisper-cli 返回其伪引擎 ID）
fn engine_of(id: &str) -> Option<&'static str> {
    if id == WHISPER_BIN_ID {
        return Some("whisper-cpp-sidecar");
    }
    if id == VAD_MODEL_ID {
        return Some(VAD_ENGINE_ID);
    }
    if let Some(m) = MODELS.iter().find(|m| m.id == id) {
        return Some(m.engine_id);
    }
    if let Some(m) = SHERPA_MODELS.iter().find(|m| m.id == id) {
        return Some(m.engine_id);
    }
    None
}

/// 只删文件不动配置（models/bin 基目录注入，便于测试）
fn delete_files_in(models: &PathBuf, bin: &PathBuf, id: &str) -> Result<(), String> {
    if id == WHISPER_BIN_ID {
        let mut removed_any = false;
        for name in ["whisper-cli.exe", "whisper.dll"] {
            removed_any |= remove_if_exists(&bin.join(name))?;
        }
        if let Ok(rd) = fs::read_dir(bin) {
            for entry in rd.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with("ggml") && name.ends_with(".dll") {
                    removed_any |= remove_if_exists(&entry.path())?;
                }
            }
        }
        let _ = removed_any; // 幂等：一个都不在也算成功
        return Ok(());
    }
    if id == VAD_MODEL_ID {
        remove_if_exists(&models.join(VAD_MODEL_FILE))?;
        return Ok(());
    }
    if let Some(m) = MODELS.iter().find(|m| m.id == id) {
        remove_if_exists(&models.join(m.file))?;
        return Ok(());
    }
    if let Some(m) = SHERPA_MODELS.iter().find(|m| m.id == id) {
        let dir = models.join(m.dir);
        if dir.exists() {
            fs::remove_dir_all(&dir).map_err(|e| format!("删除目录 {} 失败：{e}", dir.display()))?;
        }
        return Ok(());
    }
    Err(format!("未知模型：{id}"))
}

fn remove_if_exists(p: &PathBuf) -> Result<bool, String> {
    match fs::remove_file(p) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(format!("删除 {} 失败：{e}", p.display())),
    }
}

// ---------- 活动模型切换 ----------

/// 切换引擎的活动模型：写入 config.json 的 engineOptions[engine_id].model。
/// 模型文件须已下载（否则切了也用不了）。
pub fn set_active(engine_id: &str, model_id: &str) -> Result<(), String> {
    if SHERPA_MODELS.iter().any(|m| m.engine_id == engine_id) {
        if !SHERPA_MODELS
            .iter()
            .any(|m| m.id == model_id && m.engine_id == engine_id)
        {
            return Err(format!(
                "引擎 {engine_id} 没有模型 {model_id}（可选：{}）",
                model_ids().join(", ")
            ));
        }
        if !multi_model_ready(model_id) {
            return Err(format!("模型 {model_id} 尚未下载，请先下载再切换"));
        }
    } else {
        match engine_id {
            "whisper-cpp-sidecar" => {
                let manifest = MODELS
                    .iter()
                    .find(|m| m.id == model_id && m.engine_id == engine_id);
                if manifest.is_none() {
                    return Err(format!(
                        "引擎 {engine_id} 没有模型 {model_id}（可选：{}）",
                        model_ids().join(", ")
                    ));
                }
                if !models_dir().join(manifest.unwrap().file).exists() {
                    return Err(format!("模型 {model_id} 尚未下载，请先下载再切换"));
                }
            }
            _ => return Err(format!("引擎 {engine_id} 暂不支持模型切换")),
        }
    }

    let mut s = settings::load();
    let opts = s
        .engine_options
        .as_object_mut()
        .ok_or_else(|| "config.json 的 engineOptions 不是对象".to_string())?;
    let entry = opts
        .entry(engine_id.to_string())
        .or_insert_with(|| serde_json::json!({}));
    entry["model"] = serde_json::Value::String(model_id.to_string());
    settings::save(&s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_ids_unique_and_sha256_wellformed() {
        let mut ids: Vec<_> = MODELS.iter().map(|m| m.id).collect();
        let n = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), n, "模型 ID 应唯一");
        for m in MODELS {
            assert_eq!(m.sha256.len(), 64, "{} sha256 应 64 hex", m.id);
            assert!(m.sha256.chars().all(|c| c.is_ascii_hexdigit()), "{}", m.id);
            assert!(m.url.starts_with("https://"), "{}", m.id);
            assert!(m.url.ends_with(m.file), "{} URL 应以文件名结尾", m.id);
            assert!(m.size_bytes > 1_000_000, "{}", m.id);
        }
    }

    #[test]
    fn bin_manifest_wellformed() {
        assert_eq!(WHISPER_BIN_SHA256.len(), 64);
        assert!(WHISPER_BIN_URL.contains(WHISPER_BIN_VERSION));
        assert!(WHISPER_BIN_URL.ends_with("whisper-bin-x64.zip"));
    }

    #[test]
    fn list_contains_models_and_bin_entry() {
        let items = list().unwrap();
        assert_eq!(items.len(), MODELS.len() + SHERPA_MODELS.len() + 2);
        assert!(items.iter().any(|i| i.id == WHISPER_BIN_ID));
        let small = items.iter().find(|i| i.id == "ggml-small").unwrap();
        assert_eq!(small.engine_id, "whisper-cpp-sidecar");
        let zipformer = items
            .iter()
            .find(|i| i.id == "zipformer-bilingual-zh-en-2023-02-20")
            .unwrap();
        assert_eq!(zipformer.engine_id, "sherpa-onnx-zipformer-zh");
        assert_eq!(
            zipformer.size_bytes,
            SHERPA_MODELS[0].files.iter().map(|f| f.size_bytes).sum::<u64>()
        );
        let vad = items.iter().find(|i| i.id == VAD_MODEL_ID).unwrap();
        assert_eq!(vad.engine_id, VAD_ENGINE_ID);
        assert_eq!(vad.size_bytes, VAD_MODEL_SIZE);
        assert_eq!(vad.sha256, VAD_MODEL_SHA256);
    }

    /// 回归：IPC 序列化必须 camelCase（前端按 engineId/displayName/sizeBytes 读取；
    /// 曾因缺 rename_all 导致壳端模型清单分组键 undefined、引擎页模型区块整体不渲染）
    #[test]
    fn model_info_serializes_camel_case() {
        let items = list().unwrap();
        let json = serde_json::to_value(&items[0]).unwrap();
        let obj = json.as_object().unwrap();
        for key in ["engineId", "displayName", "sizeBytes", "downloadUrl"] {
            assert!(obj.contains_key(key), "缺少 camelCase 键 {key}：{json}");
        }
        for key in ["engine_id", "display_name", "size_bytes", "download_url"] {
            assert!(!obj.contains_key(key), "不应出现 snake_case 键 {key}");
        }
    }

    #[test]
    fn vad_manifest_wellformed() {
        assert_eq!(VAD_MODEL_SHA256.len(), 64);
        assert!(VAD_MODEL_SHA256.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(VAD_MODEL_URL.starts_with("https://"));
        assert!(VAD_MODEL_URL.ends_with(VAD_MODEL_FILE));
        assert!(VAD_MODEL_SIZE > 100_000);
    }

    #[test]
    fn sherpa_manifest_wellformed() {
        const REGISTERED_ENGINES: &[&str] = &[
            "sherpa-onnx-zipformer-zh",
            "sherpa-onnx-sensevoice",
            "sherpa-onnx-x-asr-zh-en",
            "sherpa-onnx-funasr-nano",
            "sherpa-onnx-qwen3-asr",
        ];
        let mut ids: Vec<_> = SHERPA_MODELS.iter().map(|m| m.id).collect();
        let n = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), n, "sherpa 模型 ID 应唯一");
        for m in SHERPA_MODELS {
            assert!(
                REGISTERED_ENGINES.contains(&m.engine_id),
                "{} 的 engine_id 未注册：{}",
                m.id,
                m.engine_id
            );
            assert!(!m.files.is_empty(), "{}", m.id);
            if let Some(a) = &m.archive {
                // 整包条目：url 置空走 archive；archive 自身字段必须良构
                assert!(a.url.starts_with("https://"), "{}", m.id);
                assert_eq!(a.sha256.len(), 64, "{} 整包 sha256 应 64 hex", m.id);
                assert!(a.sha256.chars().all(|c| c.is_ascii_hexdigit()), "{}", m.id);
                assert!(a.size_bytes > 0, "{}", m.id);
            }
            for f in m.files {
                if m.archive.is_none() {
                    assert!(f.url.starts_with("https://"), "{}", f.name);
                    assert!(f.url.ends_with(f.name), "{} URL 应以文件名结尾", f.name);
                } else {
                    assert!(f.url.is_empty(), "{} 整包条目文件 url 应置空", f.name);
                }
                assert!(f.size_bytes > 0, "{}", f.name);
                // 文件名不得含路径穿越
                assert!(!f.name.contains(".."), "{} 文件名不得含 ..", f.name);
                if let Some(s) = f.sha256 {
                    assert_eq!(s.len(), 64, "{} sha256 应 64 hex", f.name);
                    assert!(s.chars().all(|c| c.is_ascii_hexdigit()), "{}", f.name);
                }
            }
        }
        // 关键文件齐备：encoder/decoder/joiner/tokens
        let names: Vec<_> = SHERPA_MODELS[0].files.iter().map(|f| f.name).collect();
        for need in ["encoder", "decoder", "joiner", "tokens.txt"] {
            assert!(
                names.iter().any(|n| n.contains(need)),
                "缺少关键文件 {need}"
            );
        }
        // SenseVoice 条目：模型 + tokens 两文件，且默认模型解析到该条目
        let sv = SHERPA_MODELS
            .iter()
            .find(|m| m.engine_id == "sherpa-onnx-sensevoice")
            .expect("SenseVoice 模型清单缺失");
        let sv_names: Vec<_> = sv.files.iter().map(|f| f.name).collect();
        assert!(sv_names.contains(&"model.int8.onnx"));
        assert!(sv_names.contains(&"tokens.txt"));
        assert_eq!(sensevoice_default_model(), sv.id);
    }

    #[test]
    fn new_engines_manifest_entries() {
        // X-ASR：整包发布（archive），含 encoder/decoder/joiner/tokens/bpe.model
        let x = SHERPA_MODELS
            .iter()
            .find(|m| m.engine_id == "sherpa-onnx-x-asr-zh-en")
            .expect("X-ASR 模型清单缺失");
        assert!(x.archive.is_some(), "X-ASR 应走整包下载");
        let x_names: Vec<_> = x.files.iter().map(|f| f.name).collect();
        for need in [
            "encoder.int8.onnx",
            "decoder.onnx",
            "joiner.int8.onnx",
            "tokens.txt",
            "bpe.model",
        ] {
            assert!(x_names.contains(&need), "X-ASR 缺少 {need}");
        }
        assert_eq!(multi_file_default_model("sherpa-onnx-x-asr-zh-en"), Some(x.id));

        // FunASR-Nano：encoder_adaptor/llm/embedding + Qwen3-0.6B tokenizer 目录
        let fun = SHERPA_MODELS
            .iter()
            .find(|m| m.engine_id == "sherpa-onnx-funasr-nano")
            .expect("FunASR-Nano 模型清单缺失");
        let fun_names: Vec<_> = fun.files.iter().map(|f| f.name).collect();
        for need in [
            "encoder_adaptor.int8.onnx",
            "llm.int8.onnx",
            "embedding.int8.onnx",
            "Qwen3-0.6B/merges.txt",
            "Qwen3-0.6B/tokenizer.json",
            "Qwen3-0.6B/vocab.json",
        ] {
            assert!(fun_names.contains(&need), "FunASR-Nano 缺少 {need}");
        }
        assert_eq!(
            multi_file_default_model("sherpa-onnx-funasr-nano"),
            Some(fun.id)
        );

        // Qwen3-ASR：conv_frontend/encoder/decoder + tokenizer 目录
        let qw = SHERPA_MODELS
            .iter()
            .find(|m| m.engine_id == "sherpa-onnx-qwen3-asr")
            .expect("Qwen3-ASR 模型清单缺失");
        let qw_names: Vec<_> = qw.files.iter().map(|f| f.name).collect();
        for need in [
            "conv_frontend.onnx",
            "encoder.int8.onnx",
            "decoder.int8.onnx",
            "tokenizer/merges.txt",
            "tokenizer/tokenizer_config.json",
            "tokenizer/vocab.json",
        ] {
            assert!(qw_names.contains(&need), "Qwen3-ASR 缺少 {need}");
        }
        assert_eq!(
            multi_file_default_model("sherpa-onnx-qwen3-asr"),
            Some(qw.id)
        );
    }

    #[test]
    fn new_engines_active_model_mapping() {
        // 三个新引擎共用泛化映射：未配置 → 清单默认；合法 id → 采用；跨引擎 id → 兜底
        for engine in [
            "sherpa-onnx-x-asr-zh-en",
            "sherpa-onnx-funasr-nano",
            "sherpa-onnx-qwen3-asr",
        ] {
            let default = multi_file_default_model(engine).unwrap();
            let s = settings::Settings::default();
            assert_eq!(active_model_from(&s, engine), default, "{engine} 未配置兜底");

            let mut s2 = settings::Settings::default();
            s2.engine_options[engine]["model"] = serde_json::json!(default);
            assert_eq!(active_model_from(&s2, engine), default, "{engine} 合法 id");

            let mut s3 = settings::Settings::default();
            s3.engine_options[engine]["model"] =
                serde_json::json!("zipformer-bilingual-zh-en-2023-02-20");
            assert_eq!(
                active_model_from(&s3, engine),
                default,
                "{engine} 跨引擎 id 不应被采用"
            );
        }
    }

    #[test]
    fn sensevoice_active_model_mapping() {
        // 未配置 → 清单默认；配置合法 id → 采用；配置跨引擎 id → 兜底默认
        let s = settings::Settings::default();
        assert_eq!(
            active_model_from(&s, "sherpa-onnx-sensevoice"),
            sensevoice_default_model()
        );
        let mut s2 = settings::Settings::default();
        s2.engine_options["sherpa-onnx-sensevoice"]["model"] =
            serde_json::json!("sense-voice-zh-en-ja-ko-yue-2024-07-17");
        assert_eq!(
            active_model_from(&s2, "sherpa-onnx-sensevoice"),
            "sense-voice-zh-en-ja-ko-yue-2024-07-17"
        );
        let mut s3 = settings::Settings::default();
        s3.engine_options["sherpa-onnx-sensevoice"]["model"] =
            serde_json::json!("zipformer-bilingual-zh-en-2023-02-20");
        assert_eq!(
            active_model_from(&s3, "sherpa-onnx-sensevoice"),
            sensevoice_default_model(),
            "跨引擎模型 id 不应被采用"
        );
    }

    #[test]
    fn active_model_defaults_per_engine() {
        // 未配置时（或配置了未知 id 时）的兜底按引擎区分
        let sherpa = active_model("sherpa-onnx-zipformer-zh");
        assert!(
            SHERPA_MODELS.iter().any(|m| m.id == sherpa),
            "sherpa 默认模型应在清单内：{sherpa}"
        );
    }

    #[test]
    fn model_path_only_for_known_ids() {
        assert!(model_path("ggml-small").unwrap().ends_with("ggml-small.bin"));
        assert!(model_path("no-such-model").is_none());
    }

    #[test]
    fn download_unknown_model_errors() {
        let err = download("no-such-model", &|_, _| {}).unwrap_err();
        assert!(err.contains("未知模型"), "err: {err}");
    }

    // ---------- bpe.vocab 导出 / 探测 / 门控解析（P0） ----------

    /// 手工编码一个 sentencepiece Piece（field1=token string, field2=fixed32 score），
    /// 再包一层 ModelProto pieces（field 1, wire 2）
    fn sp_model(pieces: &[(&str, f32)]) -> Vec<u8> {
        let mut out = Vec::new();
        for (token, score) in pieces {
            let mut msg = vec![0x0a, token.len() as u8];
            msg.extend_from_slice(token.as_bytes());
            msg.push(0x15);
            msg.extend_from_slice(&score.to_le_bytes());
            out.push(0x0a);
            out.push(msg.len() as u8);
            out.extend_from_slice(&msg);
        }
        out
    }

    #[test]
    fn export_bpe_vocab_from_synthetic_model() {
        let tmp = tempfile::tempdir().unwrap();
        let model = tmp.path().join("bpe.model");
        let vocab = tmp.path().join("bpe.vocab");
        fs::write(
            &model,
            sp_model(&[("<blk>", 0.0), ("▁HEL", -1.5), ("LOW", -2.0)]),
        )
        .unwrap();
        let n = export_bpe_vocab(&model, &vocab).unwrap();
        assert_eq!(n, 3);
        assert_eq!(fs::read_to_string(&vocab).unwrap(), "<blk>\t0\n▁HEL\t-1.5\nLOW\t-2\n");
        assert!(is_valid_bpe_vocab(&vocab), "导出的 vocab 应通过格式探测");
    }

    #[test]
    fn export_bpe_vocab_rejects_garbage() {
        let tmp = tempfile::tempdir().unwrap();
        let model = tmp.path().join("bpe.model");
        fs::write(&model, b"not a protobuf at all \xff\xff\xff\xff").unwrap();
        assert!(export_bpe_vocab(&model, &tmp.path().join("bpe.vocab")).is_err());
    }

    #[test]
    fn bpe_vocab_probe_accepts_text_rejects_binary() {
        let tmp = tempfile::tempdir().unwrap();
        let ok = tmp.path().join("ok.vocab");
        fs::write(&ok, "<blk>\t0\n▁AB\t-1.5\nCD\t-2.25\n").unwrap();
        assert!(is_valid_bpe_vocab(&ok));

        // 二进制 sentencepiece（真实事故文件的头部特征：含 NUL）
        let bin = tmp.path().join("bin.vocab");
        fs::write(
            &bin,
            [0x0a, 0x0e, 0x0a, 0x05, b'<', b'b', b'l', b'k', b'>', 0x15, 0, 0, 0, 0],
        )
        .unwrap();
        assert!(!is_valid_bpe_vocab(&bin), "二进制不得通过探测");

        // 单列文本 / 非浮点 score / 空文件 / 不存在
        let one_col = tmp.path().join("one.vocab");
        fs::write(&one_col, "token\nanother\n").unwrap();
        assert!(!is_valid_bpe_vocab(&one_col));
        let bad_score = tmp.path().join("bad.vocab");
        fs::write(&bad_score, "token\tnan-x\n").unwrap();
        assert!(!is_valid_bpe_vocab(&bad_score));
        let empty = tmp.path().join("empty.vocab");
        fs::write(&empty, b"").unwrap();
        assert!(!is_valid_bpe_vocab(&empty));
        assert!(!is_valid_bpe_vocab(&tmp.path().join("missing.vocab")));
    }

    #[test]
    fn resolve_bpe_vocab_exports_from_bpe_model_lazily() {
        // 只有 bpe.model 的老安装目录 → 现场导出兜底
        let tmp = tempfile::tempdir().unwrap();
        fs::write(
            tmp.path().join("bpe.model"),
            sp_model(&[("<blk>", 0.0), ("▁AB", -1.0)]),
        )
        .unwrap();
        let got = resolve_bpe_vocab(tmp.path(), "bpe.vocab").unwrap();
        assert!(got.ends_with("bpe.vocab"));
        assert!(is_valid_bpe_vocab(&got));

        // 空目录 → None（调用方降级，不传 C 侧）
        let tmp2 = tempfile::tempdir().unwrap();
        assert_eq!(resolve_bpe_vocab(tmp2.path(), "bpe.vocab"), None);

        // vocab 已是合格文本 → 直接用，不重导
        let tmp3 = tempfile::tempdir().unwrap();
        let v = tmp3.path().join("bpe.vocab");
        fs::write(&v, "x\t-0.5\n").unwrap();
        assert_eq!(resolve_bpe_vocab(tmp3.path(), "bpe.vocab"), Some(v));
    }

    // ---------- models_dir_from / active_model_from（纯函数） ----------

    #[test]
    fn models_dir_from_defaults_and_custom() {
        let s = settings::Settings::default();
        assert_eq!(models_dir_from(&s), settings::kotone_dir().join("models"));
        let mut s = settings::Settings::default();
        s.models.dir = "  ".into(); // 纯空白视为未配置
        assert_eq!(models_dir_from(&s), settings::kotone_dir().join("models"));
        s.models.dir = "D:\\models".into();
        assert_eq!(models_dir_from(&s), PathBuf::from("D:\\models"));
    }

    #[test]
    fn active_model_from_matches_disk_version_fallbacks() {
        let s = settings::Settings::default();
        // 与 active_model 同一兜底：sherpa 占位字符串 → 清单默认；whisper → ggml-small
        assert_eq!(
            active_model_from(&s, "sherpa-onnx-zipformer-zh"),
            sherpa_default_model()
        );
        assert_eq!(active_model_from(&s, "whisper-cpp-sidecar"), "ggml-small");
        let mut s = settings::Settings::default();
        s.engine_options["whisper-cpp-sidecar"]["model"] = serde_json::json!("ggml-tiny");
        assert_eq!(active_model_from(&s, "whisper-cpp-sidecar"), "ggml-tiny");
    }

    // ---------- 目录迁移 ----------

    #[test]
    fn migrate_moves_files_and_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("old");
        let dst = tmp.path().join("new");
        fs::create_dir_all(src.join("sherpa-dir")).unwrap();
        fs::write(src.join("ggml-small.bin"), b"model").unwrap();
        fs::write(src.join("sherpa-dir/tokens.txt"), b"tokens").unwrap();

        let report = migrate_dir_contents(&src, &dst).unwrap();
        assert_eq!(report.failed.len(), 0, "report: {report:?}");
        assert_eq!(report.moved.len(), 2);
        assert_eq!(fs::read(dst.join("ggml-small.bin")).unwrap(), b"model");
        assert_eq!(fs::read(dst.join("sherpa-dir/tokens.txt")).unwrap(), b"tokens");
        assert!(!src.join("ggml-small.bin").exists(), "源文件应已移走");
    }

    #[test]
    fn migrate_same_dir_errors_and_missing_src_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("m");
        assert!(migrate_dir_contents(&p, &p).is_err());
        // 旧目录不存在：只建新目录，报告为空
        let dst = tmp.path().join("dst");
        let report = migrate_dir_contents(&p, &dst).unwrap();
        assert!(report.moved.is_empty() && report.failed.is_empty());
        assert!(dst.exists());
    }

    // ---------- 删除 ----------

    #[test]
    fn delete_files_in_removes_each_kind() {
        let tmp = tempfile::tempdir().unwrap();
        let models = tmp.path().join("models");
        let bin = tmp.path().join("bin");
        let sherpa_dir = models.join(SHERPA_MODELS[0].dir);
        fs::create_dir_all(&sherpa_dir).unwrap();
        fs::create_dir_all(&bin).unwrap();
        fs::write(models.join("ggml-small.bin"), b"x").unwrap();
        fs::write(sherpa_dir.join("tokens.txt"), b"x").unwrap();
        fs::write(models.join(VAD_MODEL_FILE), b"x").unwrap();
        fs::write(bin.join("whisper-cli.exe"), b"x").unwrap();
        fs::write(bin.join("whisper.dll"), b"x").unwrap();
        fs::write(bin.join("ggml-base.dll"), b"x").unwrap();
        fs::write(bin.join("unrelated.txt"), b"x").unwrap();

        delete_files_in(&models, &bin, "ggml-small").unwrap();
        assert!(!models.join("ggml-small.bin").exists());
        delete_files_in(&models, &bin, SHERPA_MODELS[0].id).unwrap();
        assert!(!sherpa_dir.exists(), "多文件模型删整个目录");
        delete_files_in(&models, &bin, VAD_MODEL_ID).unwrap();
        assert!(!models.join(VAD_MODEL_FILE).exists());
        delete_files_in(&models, &bin, WHISPER_BIN_ID).unwrap();
        assert!(!bin.join("whisper-cli.exe").exists());
        assert!(!bin.join("ggml-base.dll").exists());
        assert!(bin.join("unrelated.txt").exists(), "无关文件不动");
        // 幂等：再删一遍不报错；未知 id 报错
        delete_files_in(&models, &bin, "ggml-small").unwrap();
        assert!(delete_files_in(&models, &bin, "no-such").is_err());
    }
}
