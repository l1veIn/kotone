//! 模型清单 + 下载管理（docs/development.md §3.3、ADR-003）。
//!
//! 自管理布局（不依赖 Tauri externalBin，kotone-cli 同样可用）：
//! - 模型：`~/.kotone/models/`（sherpa-onnx 多文件模型一个目录一套）
//!
//! 清单内置：ID / 大小 / URL / SHA256。下载经 download.rs（流式 + 校验 + 原子落盘 +
//! 镜像回退，download.source 策略见 settings）。
//!
//! silero VAD 例外：随应用本体分发（`include_bytes!` 内嵌，`ensure_vad_model`
//! 解包落盘），不出现在用户可管理的模型清单；download/delete 仍保留
//! `silero-vad` 分支作 CLI 兜底与旧文件清理。

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

use kotone_core::settings;
use kotone_core::stt::SessionConfig;

use crate::download::{self, Progress};

/// 输出形态：Kotone 运行时只区分这两种循环。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ModelIo {
    Streaming,
    Offline,
}

/// 权重从哪来：本机 sherpa，或远程 HTTP。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ModelBackend {
    Sherpa,
    Remote,
}

/// sherpa / 远程后端的打开配方。加新家族只加一个配方，不注册新引擎。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ModelRecipe {
    ZipformerTransducer,
    SenseVoice,
    FunasrNano,
    OpenaiCompat,
}

/// 模型自己声明的可配项（语言、连接、密钥等）。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelConfigField {
    pub key: String,
    pub label: String,
    /// string / enum / secret / connection
    pub kind: String,
    pub default: String,
    pub options: Vec<ModelConfigOption>,
    pub required: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelConfigOption {
    pub value: String,
    pub label: String,
}

/// 模型信息（跨后端统一列出：已下载/可下载）
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
    pub io: ModelIo,
    pub backend: ModelBackend,
    pub recipe: ModelRecipe,
    pub config_schema: Vec<ModelConfigField>,
}

// ---------- 路径 ----------

/// 模型目录：settings.models.dir 非空时用自定义目录，否则默认 ~/.kotone/models。
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

/// sherpa-onnx 的部分原生词表读取器在 Windows 上无法安全打开非 ASCII 路径，
/// 并会直接终止宿主进程。模型目录因此统一要求使用纯英文（ASCII）路径。
pub fn validate_models_dir_path(path: &Path) -> Result<(), String> {
    if path.as_os_str().to_string_lossy().is_ascii() {
        Ok(())
    } else {
        Err(format!(
            "模型存储路径必须使用纯英文路径（可包含英文字母、数字、空格和英文符号），\
             例如 D:\\KotoneModels。当前路径：{}",
            path.display()
        ))
    }
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

/// 与官方整包内容一致的逐文件镜像。revision 固定到不可变提交，避免 master
/// 后续变化影响已经发布的客户端；每个文件仍按清单 SHA256 独立校验。
pub struct FileMirrorSource {
    pub base_url: &'static str,
    pub revision: &'static str,
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
    /// 逐文件镜像（按优先级）；auto/mirror 依次尝试，auto 最后回退官方
    pub file_mirrors: &'static [FileMirrorSource],
    /// 国内整包镜像（tar.bz2）。官方仍走 files / archive；auto/mirror 优先尝试这里。
    pub archive_mirrors: &'static [ArchiveSource],
    /// dest 文件名 → 镜像仓相对路径；空片 = 与 dest 同名
    pub remote_names: &'static [(&'static str, &'static str)],
    pub io: ModelIo,
    pub backend: ModelBackend,
    pub recipe: ModelRecipe,
    pub config_schema: &'static [StaticConfigField],
}

pub struct StaticConfigField {
    pub key: &'static str,
    pub label: &'static str,
    pub kind: &'static str,
    pub default: &'static str,
    pub options: &'static [(&'static str, &'static str)],
    pub required: bool,
}

const SENSEVOICE_LANGUAGE_SCHEMA: &[StaticConfigField] = &[StaticConfigField {
    key: "language",
    label: "识别语言",
    kind: "enum",
    default: "auto",
    options: &[
        ("auto", "自动"),
        ("zh", "中文"),
        ("en", "英语"),
        ("ja", "日语"),
        ("ko", "韩语"),
        ("yue", "粤语"),
    ],
    required: false,
}];

const REMOTE_CONNECTION_SCHEMA: &[StaticConfigField] = &[StaticConfigField {
    key: "connectionId",
    label: "API 连接",
    kind: "connection",
    default: "",
    options: &[],
    required: true,
}];

pub const REMOTE_OPENAI_STT_ID: &str = "openai-compat-stt";
pub const REMOTE_OPENAI_ENGINE_ID: &str = "remote-openai-compat";
pub const SHERPA_STREAMING_ENGINE_ID: &str = "sherpa-streaming";
pub const SHERPA_OFFLINE_ENGINE_ID: &str = "sherpa-offline";

/// sherpa-onnx 模型清单（ADR-004）。默认引擎 X-ASR（六引擎评测冠军：CER 0.008、
/// 首字 71ms、162MB；见 docs/development.md §11 v15），其模型走 archive 整包下载。
/// SHA256 取自 HuggingFace LFS oid 或本地实算（各条目注释注明）；git 内小文件
/// 无 LFS oid，仅按大小校验。
pub const SHERPA_MODELS: &[MultiFileModel] = &[
    // X-ASR（流式 zipformer transducer，中英+标点）：官方在 k2-fsa GitHub
    // releases 发布 tar.bz2；Kotone 的 ModelScope 仓库提供内容完全一致的逐文件
    // 国内镜像。auto 默认走 ModelScope，失败后回退官方整包。
    // 整包及各文件 SHA256 均与官方发布资产核对（2026-07）。
    MultiFileModel {
        id: "x-asr-480ms-streaming-zh-en-punct-int8-2026-06-05",
        engine_id: SHERPA_STREAMING_ENGINE_ID,
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
        file_mirrors: &[
            // Kotone 项目方控制的主镜像
            FileMirrorSource {
                base_url: "https://www.modelscope.cn/models/yangchen1258/sherpa-onnx-x-asr-480ms-streaming-zipformer-transducer-zh-en-punct-int8-2026-06-05",
                revision: "aa50faf0a9e45f6ea8913762151d47679ba468d7",
            },
            // 社区备用镜像；内容 SHA256 与官方及主镜像完全一致
            FileMirrorSource {
                base_url: "https://www.modelscope.cn/models/bujidc/sherpa-onnx-x-asr-480ms-streaming-zipformer-transducer-zh-en-punct-int8-2026-06-05",
                revision: "57f0a27e56d43b36f350be2ecc4a2200232d13e7",
            },
        ],
        archive_mirrors: &[],
        remote_names: &[],
        io: ModelIo::Streaming,
        backend: ModelBackend::Sherpa,
        recipe: ModelRecipe::ZipformerTransducer,
        config_schema: &[],
    },
    // SenseVoice（非流式、多语言）：model.int8.onnx 的 SHA256 取自 HF resolve
    // 固定到不可变提交 2365bae...；tokens.txt SHA256 由该提交内容实算。
    MultiFileModel {
        id: "sense-voice-zh-en-ja-ko-yue-2024-07-17",
        engine_id: SHERPA_OFFLINE_ENGINE_ID,
        display_name: "sherpa SenseVoice 多语言（int8，非流式高准）",
        dir: "sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17",
        files: &[
            ModelFile {
                name: "model.int8.onnx",
                url: "https://huggingface.co/csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17/resolve/2365baeacb507f821a0c8120fcee3d484dba7a07/model.int8.onnx",
                sha256: Some("c71f0ce00bec95b07744e116345e33d8cbbe08cef896382cf907bf4b51a2cd51"),
                size_bytes: 239_233_841,
            },
            ModelFile {
                name: "tokens.txt",
                url: "https://huggingface.co/csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17/resolve/2365baeacb507f821a0c8120fcee3d484dba7a07/tokens.txt",
                sha256: Some("f449eb28dc567533d7fa59be34e2abca8784f771850c78a47fb731a31429a1dc"),
                size_bytes: 315_894,
            },
        ],
        archive: None,
        file_mirrors: &[
            // 社区镜像；model.int8.onnx / tokens.txt SHA256 与官方 HF 提交 2365bae 一致
            FileMirrorSource {
                base_url: "https://www.modelscope.cn/models/fengge2024/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17",
                revision: "9bd9398d89294cbf1964af126ff13e6890394cbc",
            },
        ],
        archive_mirrors: &[],
        remote_names: &[],
        io: ModelIo::Offline,
        backend: ModelBackend::Sherpa,
        recipe: ModelRecipe::SenseVoice,
        config_schema: SENSEVOICE_LANGUAGE_SCHEMA,
    },
    // FunASR-Nano（非流式，encoder_adaptor + LLM + embedding）：HF 逐文件直下。
    // 固定到不可变提交 6f16bd3...；merges.txt SHA256 由该提交内容实算。
    // tokenizer 传 Qwen3-0.6B 目录。
    // 许可证：FunASR 系自定义 Model License（见 HF 仓库 LICENSE）。
    MultiFileModel {
        id: "funasr-nano-int8-2025-12-30",
        engine_id: SHERPA_OFFLINE_ENGINE_ID,
        display_name: "FunASR-Nano 中英日（官方 Hugging Face，非流式）",
        dir: "sherpa-onnx-funasr-nano-int8-2025-12-30",
        files: &[
            ModelFile {
                name: "encoder_adaptor.int8.onnx",
                url: "https://huggingface.co/csukuangfj/sherpa-onnx-funasr-nano-int8-2025-12-30/resolve/6f16bd378457e13f36ccf3910df9017f96c346fb/encoder_adaptor.int8.onnx",
                sha256: Some("f36dea2e30fbc33b5db1d7a7265cc976c5e5586c77b042d5adb1ad27c72db422"),
                size_bytes: 237_792_748,
            },
            ModelFile {
                name: "llm.int8.onnx",
                url: "https://huggingface.co/csukuangfj/sherpa-onnx-funasr-nano-int8-2025-12-30/resolve/6f16bd378457e13f36ccf3910df9017f96c346fb/llm.int8.onnx",
                sha256: Some("dfbf9aa3be41bccc257587f151e15c63fbe1b549f2b517f5ccd5bdce3bf4322a"),
                size_bytes: 600_356_593,
            },
            ModelFile {
                name: "embedding.int8.onnx",
                url: "https://huggingface.co/csukuangfj/sherpa-onnx-funasr-nano-int8-2025-12-30/resolve/6f16bd378457e13f36ccf3910df9017f96c346fb/embedding.int8.onnx",
                sha256: Some("95e61cd0c9c3b9543339a4cf973c95c116815e745ccc1e0285cbd81f76d18644"),
                size_bytes: 155_584_380,
            },
            ModelFile {
                name: "Qwen3-0.6B/merges.txt",
                url: "https://huggingface.co/csukuangfj/sherpa-onnx-funasr-nano-int8-2025-12-30/resolve/6f16bd378457e13f36ccf3910df9017f96c346fb/Qwen3-0.6B/merges.txt",
                sha256: Some("8831e4f1a044471340f7c0a83d7bd71306a5b867e95fd870f74d0c5308a904d5"),
                size_bytes: 1_671_853,
            },
            ModelFile {
                name: "Qwen3-0.6B/tokenizer.json",
                url: "https://huggingface.co/csukuangfj/sherpa-onnx-funasr-nano-int8-2025-12-30/resolve/6f16bd378457e13f36ccf3910df9017f96c346fb/Qwen3-0.6B/tokenizer.json",
                sha256: Some("aeb13307a71acd8fe81861d94ad54ab689df773318809eed3cbe794b4492dae4"),
                size_bytes: 11_422_654,
            },
            ModelFile {
                name: "Qwen3-0.6B/vocab.json",
                url: "https://huggingface.co/csukuangfj/sherpa-onnx-funasr-nano-int8-2025-12-30/resolve/6f16bd378457e13f36ccf3910df9017f96c346fb/Qwen3-0.6B/vocab.json",
                sha256: Some("ca10d7e9fb3ed18575dd1e277a2579c16d108e32f27439684afa0e10b1440910"),
                size_bytes: 2_776_833,
            },
        ],
        archive: None,
        file_mirrors: &[],
        // 不要接 fuyuantech 那个 tar.bz2：包是按魔搭原版 zengshuishui/FunASR-nano-onnx
        // 打的（encoder_adaptor.int8.onnx = 238277200），和 sherpa-onnx 官方 HF
        // csukuangfj 提交 6f16bd3（237792748）不是同一份文件。整包 SHA256 能过，
        // 解压后按官方体积校验必失败，用户等于白下 800MB+。
        archive_mirrors: &[],
        remote_names: &[],
        io: ModelIo::Offline,
        backend: ModelBackend::Sherpa,
        recipe: ModelRecipe::FunasrNano,
        config_schema: &[],
    },
    // FunASR-Nano 魔搭社区版：同一套 sherpa-onnx FunASR-Nano 引擎，权重来自
    // zengshuishui/FunASR-nano-onnx（国内直下）。文件布局按 sherpa 约定落盘
    // （llm.int8.onnx 在模型根目录），镜像仓里对应 llm_int8/llm.int8.onnx。
    // revision 钉在 8d423385…（其后只改了 README）。
    MultiFileModel {
        id: "funasr-nano-int8-modelscope",
        engine_id: SHERPA_OFFLINE_ENGINE_ID,
        display_name: "FunASR-Nano 中英日（魔搭社区，国内直下）",
        dir: "sherpa-onnx-funasr-nano-int8-modelscope",
        files: &[
            ModelFile {
                name: "encoder_adaptor.int8.onnx",
                url: "https://www.modelscope.cn/models/zengshuishui/FunASR-nano-onnx/resolve/8d423385704f584e94cd66a76dd08493a55456e2/encoder_adaptor.int8.onnx",
                sha256: Some("d0246c823f2c34133ae0efee395d8a189c8f92643e3432f866939ee34d34492c"),
                size_bytes: 238_277_200,
            },
            ModelFile {
                name: "llm.int8.onnx",
                url: "https://www.modelscope.cn/models/zengshuishui/FunASR-nano-onnx/resolve/8d423385704f584e94cd66a76dd08493a55456e2/llm_int8/llm.int8.onnx",
                sha256: Some("80a8f9734595f5e0769ff866775f55965fb9ea68125d6af7b62eba2a70ad24ff"),
                size_bytes: 600_025_527,
            },
            ModelFile {
                name: "embedding.int8.onnx",
                url: "https://www.modelscope.cn/models/zengshuishui/FunASR-nano-onnx/resolve/8d423385704f584e94cd66a76dd08493a55456e2/embedding.int8.onnx",
                sha256: Some("a05d2816e284fcca29a5dccb2c14b9edeb638fd983a84cd4a447248889b6a408"),
                size_bytes: 155_583_106,
            },
            ModelFile {
                name: "Qwen3-0.6B/merges.txt",
                url: "https://www.modelscope.cn/models/zengshuishui/FunASR-nano-onnx/resolve/8d423385704f584e94cd66a76dd08493a55456e2/Qwen3-0.6B/merges.txt",
                sha256: Some("8831e4f1a044471340f7c0a83d7bd71306a5b867e95fd870f74d0c5308a904d5"),
                size_bytes: 1_671_853,
            },
            ModelFile {
                name: "Qwen3-0.6B/tokenizer.json",
                url: "https://www.modelscope.cn/models/zengshuishui/FunASR-nano-onnx/resolve/8d423385704f584e94cd66a76dd08493a55456e2/Qwen3-0.6B/tokenizer.json",
                sha256: Some("aeb13307a71acd8fe81861d94ad54ab689df773318809eed3cbe794b4492dae4"),
                size_bytes: 11_422_654,
            },
            ModelFile {
                name: "Qwen3-0.6B/vocab.json",
                url: "https://www.modelscope.cn/models/zengshuishui/FunASR-nano-onnx/resolve/8d423385704f584e94cd66a76dd08493a55456e2/Qwen3-0.6B/vocab.json",
                sha256: Some("ca10d7e9fb3ed18575dd1e277a2579c16d108e32f27439684afa0e10b1440910"),
                size_bytes: 2_776_833,
            },
        ],
        archive: None,
        file_mirrors: &[FileMirrorSource {
            base_url: "https://www.modelscope.cn/models/zengshuishui/FunASR-nano-onnx",
            revision: "8d423385704f584e94cd66a76dd08493a55456e2",
        }],
        archive_mirrors: &[],
        remote_names: &[("llm.int8.onnx", "llm_int8/llm.int8.onnx")],
        io: ModelIo::Offline,
        backend: ModelBackend::Sherpa,
        recipe: ModelRecipe::FunasrNano,
        config_schema: &[],
    },
];

#[derive(Clone)]
struct IntegrityCacheEntry {
    len: u64,
    modified: Option<SystemTime>,
    sha256: String,
}

static INTEGRITY_CACHE: OnceLock<Mutex<std::collections::HashMap<PathBuf, IntegrityCacheEntry>>> =
    OnceLock::new();

fn integrity_cache() -> &'static Mutex<std::collections::HashMap<PathBuf, IntegrityCacheEntry>> {
    INTEGRITY_CACHE.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

fn invalidate_integrity_cache(path: &std::path::Path) {
    integrity_cache().lock().unwrap().remove(path);
}

/// 下载/解压后的完整性校验缓存。同一进程内重复点「下载」时跳过已核过的文件。
/// 长度/mtime 变化后重算；缓存不落盘。设置页列举不走这条路径。
fn sha256_file_cached(path: &std::path::Path) -> Result<String, String> {
    let before = fs::metadata(path).map_err(|e| format!("无法读取 {}：{e}", path.display()))?;
    let modified = before.modified().ok();
    let cache = integrity_cache();
    if let Some(entry) = cache.lock().unwrap().get(path) {
        if entry.len == before.len() && entry.modified == modified {
            return Ok(entry.sha256.clone());
        }
    }

    let sha256 = download::sha256_file(path)?;
    let after = fs::metadata(path).map_err(|e| format!("无法读取 {}：{e}", path.display()))?;
    if before.len() != after.len() || modified != after.modified().ok() {
        return Err(format!("校验期间文件发生变化：{}", path.display()));
    }
    cache.lock().unwrap().insert(
        path.to_path_buf(),
        IntegrityCacheEntry {
            len: after.len(),
            modified,
            sha256: sha256.clone(),
        },
    );
    Ok(sha256)
}

fn verify_file_integrity(
    path: &std::path::Path,
    expected_size: u64,
    expected_sha256: &str,
) -> Result<(), String> {
    let actual_size = fs::metadata(path).map_err(|_| "缺失".to_string())?.len();
    if actual_size != expected_size {
        return Err(format!(
            "大小不符：期望 {expected_size} 字节，实际 {actual_size} 字节"
        ));
    }
    let actual = sha256_file_cached(path)?;
    if !actual.eq_ignore_ascii_case(expected_sha256) {
        return Err(format!(
            "SHA256 不符：期望 {expected_sha256}，实际 {actual}"
        ));
    }
    Ok(())
}

fn verify_model_file(path: &std::path::Path, file: &ModelFile) -> Result<(), String> {
    let expected = file
        .sha256
        .ok_or_else(|| "模型清单缺少 SHA256".to_string())?;
    verify_file_integrity(path, file.size_bytes, expected)
}

/// 设置页列举 / is_ready / 启动缺文件提示：只看存在和大小。
/// SHA256 留给下载完成后的校验；打开设置不该扫数百 MB。
fn model_file_present(path: &std::path::Path, file: &ModelFile) -> Result<(), String> {
    let actual_size = fs::metadata(path).map_err(|_| "缺失".to_string())?.len();
    if actual_size != file.size_bytes {
        return Err(format!(
            "大小不符：期望 {} 字节，实际 {actual_size} 字节",
            file.size_bytes
        ));
    }
    Ok(())
}

/// 旧家族引擎 id 对应的打开配方（兼容 engineOptions / CLI）。
pub fn family_recipe(engine_id: &str) -> Option<ModelRecipe> {
    match engine_id {
        "sherpa-onnx-x-asr-zh-en" => Some(ModelRecipe::ZipformerTransducer),
        "sherpa-onnx-sensevoice" => Some(ModelRecipe::SenseVoice),
        "sherpa-onnx-funasr-nano" => Some(ModelRecipe::FunasrNano),
        _ => None,
    }
}

pub fn is_sherpa_engine(engine_id: &str) -> bool {
    engine_id == SHERPA_STREAMING_ENGINE_ID
        || engine_id == SHERPA_OFFLINE_ENGINE_ID
        || family_recipe(engine_id).is_some()
}

pub fn model_belongs_to_engine(model_id: &str, engine_id: &str) -> bool {
    let Some(m) = SHERPA_MODELS.iter().find(|m| m.id == model_id) else {
        return false;
    };
    if m.engine_id == engine_id {
        return true;
    }
    family_recipe(engine_id).is_some_and(|recipe| m.recipe == recipe)
}

/// 指定 sherpa 系引擎清单中的默认模型（该引擎清单首条；无清单时 None）
pub fn multi_file_default_model(engine_id: &str) -> Option<&'static str> {
    if let Some(recipe) = family_recipe(engine_id) {
        return SHERPA_MODELS
            .iter()
            .find(|m| m.recipe == recipe)
            .map(|m| m.id);
    }
    SHERPA_MODELS
        .iter()
        .find(|m| m.engine_id == engine_id)
        .map(|m| m.id)
}

/// SenseVoice 引擎的默认模型（单模型清单，恒为清单条目）
pub fn sensevoice_default_model() -> &'static str {
    multi_file_default_model("sherpa-onnx-sensevoice").expect("SenseVoice 模型清单缺失")
}

// ---------- silero VAD 模型（ADR-007，单文件，随本体分发） ----------

/// silero VAD 模型 ID（download/delete 用；VAD 已本体分发，不在用户清单里）
pub const VAD_MODEL_ID: &str = "silero-vad";
/// VAD 伪引擎 ID（VAD 不是 STT 引擎；delete 清理路径用）
pub const VAD_ENGINE_ID: &str = "vad-silero";
pub const VAD_MODEL_FILE: &str = "silero_vad.onnx";
/// sherpa-onnx 官方 release 托管的 silero VAD（与 VAD 示例同一来源，钉死 URL 保证 SHA256 稳定）
pub const VAD_MODEL_URL: &str =
    "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/silero_vad.onnx";
/// silero_vad.onnx SHA256（2025-07 本地下载实测）
pub const VAD_MODEL_SHA256: &str =
    "9e2449e1087496d8d4caba907f23e0bd3f78d91fa552479bb9c23ac09cbb1fd6";
pub const VAD_MODEL_SIZE: u64 = 643_854;

/// VAD 模型内嵌字节（随应用本体分发，不再让用户在 UI 里下载）
const VAD_BUNDLED_BYTES: &[u8] = include_bytes!("../assets/silero_vad.onnx");

/// silero VAD 模型路径（~/.kotone/models/silero_vad.onnx）
pub fn vad_model_path() -> PathBuf {
    models_dir().join(VAD_MODEL_FILE)
}

/// silero VAD 模型就绪（存在且大小、SHA256 均匹配）
pub fn vad_model_ready() -> bool {
    verify_file_integrity(&vad_model_path(), VAD_MODEL_SIZE, VAD_MODEL_SHA256).is_ok()
}

/// 确保 VAD 模型已落盘（本体分发兜底：应用启动/切到 VAD 模式时调用）。
/// 已就绪 → Ok(false)；缺失/大小不符 → 把内嵌字节原子写入 vad_model_path() → Ok(true)。
pub fn ensure_vad_model() -> Result<bool, String> {
    ensure_vad_model_in(&models_dir())
}

/// 可传 models 目录的内部实现（models_dir 依赖全局配置，测试注入临时目录隔离）
fn ensure_vad_model_in(models: &PathBuf) -> Result<bool, String> {
    let dest = models.join(VAD_MODEL_FILE);
    if verify_file_integrity(&dest, VAD_MODEL_SIZE, VAD_MODEL_SHA256).is_ok() {
        return Ok(false);
    }
    fs::create_dir_all(models).map_err(|e| format!("无法创建目录 {}：{e}", models.display()))?;
    // 原子落盘：先写临时文件再 rename（对齐 download 的落盘风格）
    let tmp = models.join(format!("{VAD_MODEL_FILE}.tmp"));
    fs::write(&tmp, VAD_BUNDLED_BYTES)
        .map_err(|e| format!("写入 VAD 模型失败 {}：{e}", tmp.display()))?;
    invalidate_integrity_cache(&dest);
    fs::rename(&tmp, &dest).map_err(|e| format!("落盘 VAD 模型失败 {}：{e}", dest.display()))?;
    verify_file_integrity(&dest, VAD_MODEL_SIZE, VAD_MODEL_SHA256)
        .map_err(|e| format!("VAD 模型校验失败：{e}"))?;
    Ok(true)
}

/// 多文件模型目录
pub fn multi_model_dir(model_id: &str) -> Option<PathBuf> {
    SHERPA_MODELS
        .iter()
        .find(|m| m.id == model_id)
        .map(|m| models_dir().join(m.dir))
}

/// 多文件模型是否齐备（全部文件存在且大小匹配）。不扫 SHA256。
pub fn multi_model_ready(model_id: &str) -> bool {
    multi_model_missing(model_id).is_empty()
}

/// 按给定配置中的模型目录检查齐备性，避免设置事务期间再次从磁盘读取另一版本配置。
pub fn multi_model_ready_from(s: &settings::Settings, model_id: &str) -> bool {
    multi_model_missing_in(&models_dir_from(s), model_id).is_empty()
}

/// 多文件模型齐备性检查的明细：返回缺失/大小不符的文件描述列表（空 = 齐备）。
/// 不计算 SHA256——设置页列举和引擎 is_ready 只需要知道文件在不在。
/// 用于启动失败时把「到底缺哪个文件」写进日志与错误消息——0.1.5 曾出现
/// 运行中 stop/start 后误报「模型未下载」，布尔检查无法定位原因。
pub fn multi_model_missing(model_id: &str) -> Vec<String> {
    multi_model_missing_in(&models_dir(), model_id)
}

fn multi_model_missing_in(models: &Path, model_id: &str) -> Vec<String> {
    let Some(m) = SHERPA_MODELS.iter().find(|m| m.id == model_id) else {
        return vec![format!("未知模型 id: {model_id}")];
    };
    let dir = models.join(m.dir);
    m.files
        .iter()
        .filter_map(|file| {
            let path = dir.join(file.name);
            model_file_present(&path, file)
                .err()
                .map(|error| format!("{}（{error}）", file.name))
        })
        .collect()
}

/// 下载跳过判定：文件必须存在、大小匹配且 SHA256 匹配。
/// 仅用于下载路径，避免把同大小损坏文件当成已完成。
fn multi_model_verified(model_id: &str) -> bool {
    let Some(m) = SHERPA_MODELS.iter().find(|m| m.id == model_id) else {
        return false;
    };
    let dir = models_dir().join(m.dir);
    m.files
        .iter()
        .all(|file| verify_model_file(&dir.join(file.name), file).is_ok())
}

/// 引擎当前活动模型 ID（读 config.json 的 engineOptions；
/// 缺省按引擎给默认：sherpa 系 → 各引擎清单默认模型，无模型引擎 → "default"）
pub fn active_model(engine_id: &str) -> String {
    active_model_from(&settings::load(), engine_id)
}

/// 会话配置里的 `model`（由 session_options 注入）优先，否则回退磁盘配置。
pub fn model_id_from_cfg(cfg: &SessionConfig, engine_id: &str) -> String {
    if let Some(id) = cfg
        .options
        .get("model")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        if is_remote_model(id) || model_belongs_to_engine(id, engine_id) {
            return id.to_string();
        }
    }
    active_model(engine_id)
}

/// 从给定配置推导活动模型 ID（纯函数：壳侧用 SharedState 推导 restartNeeded，
/// 避免磁盘/内存双读不一致）
pub fn active_model_from(s: &settings::Settings, engine_id: &str) -> String {
    let selected = s.active_model_id.trim();
    if !selected.is_empty() {
        if is_remote_model(selected) {
            if engine_id == REMOTE_OPENAI_ENGINE_ID || !is_sherpa_engine(engine_id) {
                return selected.to_string();
            }
        } else if model_belongs_to_engine(selected, engine_id) {
            return selected.to_string();
        }
    }
    let configured = s
        .engine_options
        .get(engine_id)
        .and_then(|o| o.get("model"))
        .and_then(|m| m.as_str())
        .map(str::to_string);
    if is_sherpa_engine(engine_id) {
        if let Some(id) = configured {
            if model_belongs_to_engine(&id, engine_id) {
                return id;
            }
        }
        return multi_file_default_model(engine_id)
            .expect("sherpa 系引擎模型清单缺失")
            .to_string();
    }
    match configured {
        Some(id) => id,
        None => "default".to_string(),
    }
}

// ---------- 清单查询 ----------

fn schema_of(fields: &'static [StaticConfigField]) -> Vec<ModelConfigField> {
    fields
        .iter()
        .map(|f| ModelConfigField {
            key: f.key.into(),
            label: f.label.into(),
            kind: f.kind.into(),
            default: f.default.into(),
            options: f
                .options
                .iter()
                .map(|(value, label)| ModelConfigOption {
                    value: (*value).into(),
                    label: (*label).into(),
                })
                .collect(),
            required: f.required,
        })
        .collect()
}

fn sherpa_info(m: &MultiFileModel) -> ModelInfo {
    ModelInfo {
        id: m.id.into(),
        engine_id: m.engine_id.into(),
        display_name: m.display_name.into(),
        size_bytes: m.files.iter().map(|f| f.size_bytes).sum(),
        download_url: m
            .file_mirrors
            .first()
            .map(|mirror| mirror.base_url.into())
            .or_else(|| {
                m.files
                    .iter()
                    .find(|f| !f.url.is_empty())
                    .map(|f| f.url.into())
            })
            .or_else(|| m.archive.as_ref().map(|a| a.url.into()))
            .unwrap_or_default(),
        sha256: String::new(),
        downloaded: multi_model_ready(m.id),
        io: m.io,
        backend: m.backend,
        recipe: m.recipe,
        config_schema: schema_of(m.config_schema),
    }
}

fn remote_openai_info() -> ModelInfo {
    ModelInfo {
        id: REMOTE_OPENAI_STT_ID.into(),
        engine_id: REMOTE_OPENAI_ENGINE_ID.into(),
        display_name: "在线识别（OpenAI 兼容）".into(),
        size_bytes: 0,
        download_url: String::new(),
        sha256: String::new(),
        downloaded: true,
        io: ModelIo::Offline,
        backend: ModelBackend::Remote,
        recipe: ModelRecipe::OpenaiCompat,
        config_schema: schema_of(REMOTE_CONNECTION_SCHEMA),
    }
}

/// 列出全部用户可管理模型（sherpa 多文件 + 远程条目；silero VAD 不在清单内）
pub fn list() -> Result<Vec<ModelInfo>, String> {
    let mut out: Vec<ModelInfo> = SHERPA_MODELS.iter().map(sherpa_info).collect();
    out.push(remote_openai_info());
    Ok(out)
}

pub fn recipe_of(id: &str) -> Option<ModelRecipe> {
    if id == REMOTE_OPENAI_STT_ID {
        return Some(ModelRecipe::OpenaiCompat);
    }
    SHERPA_MODELS.iter().find(|m| m.id == id).map(|m| m.recipe)
}

pub fn engine_id_of(id: &str) -> Option<&'static str> {
    if id == REMOTE_OPENAI_STT_ID {
        return Some(REMOTE_OPENAI_ENGINE_ID);
    }
    SHERPA_MODELS.iter().find(|m| m.id == id).map(|m| m.engine_id)
}

pub fn is_remote_model(id: &str) -> bool {
    id == REMOTE_OPENAI_STT_ID
}

/// 手动安装指引：官方页 + 目标目录 + 应放置的文件清单。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInstallFile {
    pub name: String,
    pub url: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInstallGuide {
    pub id: String,
    pub display_name: String,
    pub dest_dir: String,
    pub dest_subdir: String,
    pub page_url: String,
    pub files: Vec<ModelInstallFile>,
}

/// 自动下载失败后的兜底：告诉用户去哪下、放到哪。
pub fn install_guide(id: &str) -> Result<ModelInstallGuide, String> {
    let m = SHERPA_MODELS
        .iter()
        .find(|m| m.id == id)
        .ok_or_else(|| format!("未知模型：{id}"))?;
    let dest_dir = models_dir().join(m.dir);
    Ok(ModelInstallGuide {
        id: m.id.into(),
        display_name: m.display_name.into(),
        dest_dir: dest_dir.to_string_lossy().into_owned(),
        dest_subdir: m.dir.into(),
        page_url: official_page_url(m),
        files: m
            .files
            .iter()
            .map(|f| ModelInstallFile {
                name: f.name.into(),
                url: f.url.into(),
                size_bytes: f.size_bytes,
            })
            .collect(),
    })
}

fn official_page_url(m: &MultiFileModel) -> String {
    if let Some(file) = m
        .files
        .iter()
        .find(|f| f.url.starts_with("https://huggingface.co/"))
    {
        if let Some(page) = huggingface_repo_page(file.url) {
            return page;
        }
    }
    if let Some(mirror) = m.file_mirrors.first() {
        return mirror.base_url.to_string();
    }
    if let Some(file) = m
        .files
        .iter()
        .find(|f| f.url.contains("modelscope.cn/models/"))
    {
        if let Some(page) = modelscope_repo_page(file.url) {
            return page;
        }
    }
    if let Some(archive) = &m.archive {
        return archive.url.to_string();
    }
    if let Some(archive) = m.archive_mirrors.first() {
        return archive.url.to_string();
    }
    m.file_mirrors
        .first()
        .map(|mirror| mirror.base_url.to_string())
        .unwrap_or_default()
}

fn huggingface_repo_page(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://huggingface.co/")?;
    let mut parts = rest.split('/');
    let ns = parts.next()?;
    let repo = parts.next()?;
    if ns.is_empty() || repo.is_empty() {
        return None;
    }
    Some(format!("https://huggingface.co/{ns}/{repo}"))
}

fn modelscope_repo_page(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://www.modelscope.cn/models/")?;
    let mut parts = rest.split('/');
    let ns = parts.next()?;
    let repo = parts.next()?;
    if ns.is_empty() || repo.is_empty() {
        return None;
    }
    Some(format!("https://www.modelscope.cn/models/{ns}/{repo}"))
}

// ---------- 下载 ----------

/// 下载模型（id = 清单内任意模型 / silero-vad），进度经回调外发
/// （多文件模型聚合进度）。阻塞实现：调用方放阻塞线程。
/// 下载源策略（镜像 / 回退）取 settings.download，见 download.rs。
/// 全局互斥 + 取消标记（P2-⑦）：并发重复下载被拒绝；cancel_download 可中断。
pub fn download(id: &str, progress: Progress<'_>) -> Result<(), String> {
    validate_models_dir_path(&models_dir())?;
    let _download_guard = download::begin_download()?;
    download_inner(id, progress)
}

/// 请求取消当前下载（幂等；Tauri IPC 用）
pub fn cancel_download() {
    download::request_cancel();
}

/// 模型清单内模型的总大小（磁盘空间预检用；未知返回 None）
pub fn model_size_bytes(id: &str) -> Option<u64> {
    if id == VAD_MODEL_ID {
        return Some(VAD_MODEL_SIZE);
    }
    SHERPA_MODELS
        .iter()
        .find(|m| m.id == id)
        .map(|m| m.files.iter().map(|f| f.size_bytes).sum())
}

fn download_inner(id: &str, progress: Progress<'_>) -> Result<(), String> {
    let cfg = settings::load().download;
    if id == VAD_MODEL_ID {
        let dest = vad_model_path();
        return download::download_resolved(
            VAD_MODEL_URL,
            &dest,
            Some(VAD_MODEL_SHA256),
            progress,
            &cfg,
        );
    }
    if let Some(m) = SHERPA_MODELS.iter().find(|m| m.id == id) {
        let result = download_multi(m, progress, &cfg);
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
fn download_multi(
    m: &MultiFileModel,
    progress: Progress<'_>,
    cfg: &settings::DownloadConfig,
) -> Result<(), String> {
    match cfg.source {
        settings::DownloadSource::Official => download_official(m, progress, cfg),
        settings::DownloadSource::Mirror => download_mirrors_only(m, progress),
        settings::DownloadSource::Auto => match download_preferred_mirrors(m, progress) {
            Ok(true) => Ok(()),
            Ok(false) => download_official(m, progress, cfg),
            Err(mirror_err) if download::is_cancelled(&mirror_err) => Err(mirror_err),
            Err(mirror_err) => {
                eprintln!("[download] ModelScope 镜像均失败，回退官方源：{mirror_err}");
                let official_cfg = settings::DownloadConfig {
                    source: settings::DownloadSource::Official,
                    gh_proxy: cfg.gh_proxy.clone(),
                };
                download_official(m, progress, &official_cfg).map_err(|official_err| {
                    if download::is_cancelled(&official_err) {
                        official_err
                    } else {
                        format!("ModelScope 镜像失败：{mirror_err}；官方源也失败：{official_err}")
                    }
                })
            }
        },
    }
}

fn download_official(
    m: &MultiFileModel,
    progress: Progress<'_>,
    cfg: &settings::DownloadConfig,
) -> Result<(), String> {
    if let Some(archive) = &m.archive {
        download_archive(m, archive, progress, cfg)
    } else {
        download_manifest_files(m, progress, cfg)
    }
}

/// `Ok(true)` = 已走镜像成功；`Ok(false)` = 没有镜像，调用方应走官方。
fn download_preferred_mirrors(
    m: &MultiFileModel,
    progress: Progress<'_>,
) -> Result<bool, String> {
    if !m.file_mirrors.is_empty() {
        download_file_mirrors(m, progress)?;
        return Ok(true);
    }
    if !m.archive_mirrors.is_empty() {
        download_archive_mirrors(m, progress)?;
        return Ok(true);
    }
    Ok(false)
}

fn download_mirrors_only(m: &MultiFileModel, progress: Progress<'_>) -> Result<(), String> {
    match download_preferred_mirrors(m, progress)? {
        true => Ok(()),
        false => Err(format!("{} 没有国内镜像，请将下载源改为自动或仅官方", m.id)),
    }
}

fn download_archive_mirrors(m: &MultiFileModel, progress: Progress<'_>) -> Result<(), String> {
    let mut errors = Vec::new();
    let official_cfg = settings::DownloadConfig {
        source: settings::DownloadSource::Official,
        gh_proxy: String::new(),
    };
    for (index, archive) in m.archive_mirrors.iter().enumerate() {
        match download_archive(m, archive, progress, &official_cfg) {
            Ok(()) => return Ok(()),
            Err(error) if download::is_cancelled(&error) => return Err(error),
            Err(error) => {
                errors.push(format!("镜像 {}/{}：{error}", index + 1, m.archive_mirrors.len()));
            }
        }
    }
    Err(errors.join("；"))
}

fn download_manifest_files(
    m: &MultiFileModel,
    progress: Progress<'_>,
    cfg: &settings::DownloadConfig,
) -> Result<(), String> {
    let dir = models_dir().join(m.dir);
    fs::create_dir_all(&dir).map_err(|e| format!("无法创建目录 {}：{e}", dir.display()))?;
    let total: u64 = m.files.iter().map(|f| f.size_bytes).sum();
    let mut done: u64 = 0;
    for f in m.files {
        let dest = dir.join(f.name);
        // 只有完整 SHA256 匹配才跳过；同大小损坏/被替换的文件必须重下。
        if verify_model_file(&dest, f).is_ok() {
            done += f.size_bytes;
            progress(done, Some(total));
            continue;
        }
        let base = done;
        download::download_resolved(
            f.url,
            &dest,
            f.sha256,
            &|d, t| {
                // 单文件 total 不可靠时仍保证聚合 total 准确
                let _ = t;
                progress(base + d, Some(total));
            },
            cfg,
        )?;
        invalidate_integrity_cache(&dest);
        done += f.size_bytes;
        progress(done, Some(total));
    }
    Ok(())
}

/// 依次尝试固定 revision 的逐文件镜像。镜像之间文件内容相同，因此前一源
/// 中断留下的 `.part` 可以直接在后一源通过 HTTP Range 续传。
fn download_file_mirrors(m: &MultiFileModel, progress: Progress<'_>) -> Result<(), String> {
    let mut errors = Vec::new();
    for (index, mirror) in m.file_mirrors.iter().enumerate() {
        if index > 0 {
            eprintln!("[download] 主 ModelScope 镜像失败，尝试备用镜像");
        }
        match download_file_mirror(m, mirror, progress) {
            Ok(()) => return Ok(()),
            Err(err) if download::is_cancelled(&err) => return Err(err),
            Err(err) => errors.push(err),
        }
    }
    Err(errors.join("；"))
}

fn download_file_mirror(
    m: &MultiFileModel,
    mirror: &FileMirrorSource,
    progress: Progress<'_>,
) -> Result<(), String> {
    let dir = models_dir().join(m.dir);
    fs::create_dir_all(&dir).map_err(|e| format!("无法创建目录 {}：{e}", dir.display()))?;
    let total: u64 = m.files.iter().map(|f| f.size_bytes).sum();
    let mut done = 0u64;
    for f in m.files {
        let dest = dir.join(f.name);
        if verify_model_file(&dest, f).is_ok() {
            done += f.size_bytes;
            progress(done, Some(total));
            continue;
        }
        let base = done;
        let url = file_mirror_url(mirror, remote_path(m, f.name));
        download::download_file(&url, &dest, f.sha256, &|d, _| {
            progress(base + d, Some(total));
        })
        .map_err(|e| format!("{} 下载 {} 失败：{e}", mirror.base_url, f.name))?;
        invalidate_integrity_cache(&dest);
        done += f.size_bytes;
        progress(done, Some(total));
    }
    Ok(())
}

fn file_mirror_url(mirror: &FileMirrorSource, name: &str) -> String {
    format!(
        "{}/resolve/{}/{}",
        mirror.base_url.trim_end_matches('/'),
        mirror.revision,
        name
    )
}

fn remote_path(m: &MultiFileModel, name: &'static str) -> &'static str {
    m.remote_names
        .iter()
        .find(|(dest, _)| *dest == name)
        .map(|(_, remote)| *remote)
        .unwrap_or(name)
}

/// 整包模型（tar.bz2）：下载整包校验后，只按 files 白名单提取到模型目录，
/// 再逐文件校验大小 + SHA256，最后删除完整整包；网络中断的 `.part` 保留续传。
fn download_archive(
    m: &MultiFileModel,
    archive: &ArchiveSource,
    progress: Progress<'_>,
    cfg: &settings::DownloadConfig,
) -> Result<(), String> {
    // 已校验通过 → 幂等直接完成（必须过 SHA256，不能只看大小）
    if multi_model_verified(m.id) {
        progress(archive.size_bytes, Some(archive.size_bytes));
        return Ok(());
    }
    let dir = models_dir().join(m.dir);
    fs::create_dir_all(&dir).map_err(|e| format!("无法创建目录 {}：{e}", dir.display()))?;
    let tar_path = models_dir().join(format!("{}.tar.bz2", m.dir));

    let result = download_archive_inner(m, archive, &tar_path, &dir, progress, cfg);
    // 成功失败都删整包：失败不留半成品，成功不再占用磁盘
    let _ = fs::remove_file(&tar_path);
    result
}

fn download_archive_inner(
    m: &MultiFileModel,
    archive: &ArchiveSource,
    tar_path: &Path,
    dir: &Path,
    progress: Progress<'_>,
    cfg: &settings::DownloadConfig,
) -> Result<(), String> {
    download::download_resolved(archive.url, tar_path, Some(archive.sha256), progress, cfg)?;

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
        invalidate_integrity_cache(&out);
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
        let md = fs::metadata(&out).map_err(|e| format!("解压后缺少 {}：{e}", out.display()))?;
        if md.len() != f.size_bytes {
            return Err(format!(
                "文件 {} 大小不符：期望 {}，实际 {}",
                f.name,
                f.size_bytes,
                md.len()
            ));
        }
        verify_model_file(&out, f).map_err(|error| format!("文件 {} 校验失败：{error}", f.name))?;
    }
    Ok(())
}

/// 可选模型 ID 列表（错误提示用）
fn model_ids() -> Vec<&'static str> {
    SHERPA_MODELS
        .iter()
        .map(|m| m.id)
        .chain([VAD_MODEL_ID])
        .collect()
}

// ---------- bpe.vocab（bpe / cjkchar+bpe 模型热词用的文本词表） ----------

/// 解析 sentencepiece bpe.model（二进制 protobuf），导出 sherpa-onnx 热词用的
/// 文本 bpe.vocab（每行「token<TAB>score」，对齐官方 scripts/export_bpe_vocab.py）。
/// 只依赖 protobuf wire format（pieces=field 1；Piece.piece=field 1 string，
/// Piece.score=field 2 fixed32 float），不引 sentencepiece 依赖。
/// 返回词片数。
pub fn export_bpe_vocab(
    bpe_model: &std::path::Path,
    out: &std::path::Path,
) -> Result<usize, String> {
    let data = fs::read(bpe_model).map_err(|e| format!("无法读取 {}：{e}", bpe_model.display()))?;
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
    if model.is_file() && export_bpe_vocab(&model, &vocab).is_ok() && is_valid_bpe_vocab(&vocab) {
        kotone_core::log::log(&format!("已从 bpe.model 导出热词词表 {}", vocab.display()));
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
        Ok(n) => kotone_core::log::log(&format!("已导出热词词表 {}（{n} 词片）", vocab.display())),
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
    for entry in fs::read_dir(src).map_err(|e| format!("读取目录 {} 失败：{e}", src.display()))?
    {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let target = dst.join(&name);
        if target.exists() {
            // 目标已有同名条目（重复迁移/目标目录本有内容）。不能盲目删源：
            // - 同名同内容文件：视为已迁移（保留目标，删除源完成合并）；
            // - 同名不同内容文件：删源会静默丢数据——源改名为 conflict 副本保留；
            // - 同名目录：保守处理，不合并不删除，记 failed 交用户处理。
            let src_is_file = entry.path().is_file();
            let same_content = src_is_file
                && fs::metadata(entry.path())
                    .and_then(|s| fs::metadata(&target).map(|t| (s.len(), t.len())))
                    .map(|(a, b)| a == b)
                    .unwrap_or(false)
                && download::sha256_file(&entry.path())
                    .and_then(|src_hash| {
                        download::sha256_file(&target).map(|target_hash| src_hash == target_hash)
                    })
                    .unwrap_or(false);
            if same_content {
                if remove_entry(&entry.path()).is_ok() {
                    report.moved.push(name);
                } else {
                    report.failed.push(name);
                }
            } else if src_is_file {
                let ts = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                let conflict = dst.join(format!("{name}.conflict-{ts}"));
                match fs::rename(entry.path(), &conflict) {
                    Ok(()) => {
                        kotone_core::log::log(&format!(
                            "模型迁移：目标已有同名不同内容文件，源保留为 {}",
                            conflict.display()
                        ));
                        report.moved.push(name);
                    }
                    Err(e) => {
                        kotone_core::log::log(&format!(
                            "模型迁移：同名冲突文件改名失败（{name}）: {e}"
                        ));
                        report.failed.push(name);
                    }
                }
            } else {
                report.failed.push(name);
            }
            continue;
        }
        match fs::rename(entry.path(), &target) {
            Ok(()) => report.moved.push(name),
            Err(_) => {
                // 跨卷等情况：复制 + 删除
                if copy_entry(&entry.path(), &target)
                    .and_then(|_| remove_entry(&entry.path()))
                    .is_ok()
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

/// 删除已下载模型（幂等：文件不存在视为成功）。
/// 多文件模型删整个目录；active 模型被删时清 engineOptions 的 active 标记
/// （回退引擎默认模型）。
pub fn delete(id: &str) -> Result<DeleteOutcome, String> {
    let mut s = settings::load();
    let was_active = clear_active_model(&mut s, id)?;
    delete_files_from(&s, id)?;
    if was_active {
        settings::save(&s)?;
    }
    let outcome = DeleteOutcome { was_active };
    Ok(outcome)
}

/// 只删除给定配置所指模型目录中的文件，不读写 config.json。
pub fn delete_files_from(s: &settings::Settings, id: &str) -> Result<(), String> {
    delete_files_in(&models_dir_from(s), id)
}

/// 若 id 是当前活动模型，则从配置快照移除 active 标记。返回是否发生修改。
pub fn clear_active_model(s: &mut settings::Settings, id: &str) -> Result<bool, String> {
    let engine_id = engine_of(id).ok_or_else(|| format!("未知模型：{id}"))?;
    if id == VAD_MODEL_ID || active_model_from(s, engine_id) != id {
        return Ok(false);
    }
    if let Some(opts) = s.engine_options.as_object_mut() {
        if let Some(entry) = opts
            .get_mut(engine_id)
            .and_then(|entry| entry.as_object_mut())
        {
            entry.remove("model");
        }
    }
    Ok(true)
}

/// 模型所属引擎（VAD 返回其伪引擎 ID）
fn engine_of(id: &str) -> Option<&'static str> {
    if id == VAD_MODEL_ID {
        return Some(VAD_ENGINE_ID);
    }
    if let Some(m) = SHERPA_MODELS.iter().find(|m| m.id == id) {
        return Some(m.engine_id);
    }
    None
}

/// 只删文件不动配置（models 基目录注入，便于测试）
fn delete_files_in(models: &Path, id: &str) -> Result<(), String> {
    if id == VAD_MODEL_ID {
        remove_if_exists(&models.join(VAD_MODEL_FILE))?;
        return Ok(());
    }
    if let Some(m) = SHERPA_MODELS.iter().find(|m| m.id == id) {
        let dir = models.join(m.dir);
        if dir.exists() {
            fs::remove_dir_all(&dir)
                .map_err(|e| format!("删除目录 {} 失败：{e}", dir.display()))?;
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
    let mut s = settings::load();
    set_active_in(&mut s, engine_id, model_id)?;
    settings::save(&s)
}

/// 在调用方提供的配置快照上切换活动模型，不自行读写 config.json。
pub fn set_active_in(
    s: &mut settings::Settings,
    engine_id: &str,
    model_id: &str,
) -> Result<(), String> {
    let resolved_engine = engine_id_of(model_id).unwrap_or(engine_id);
    if is_remote_model(model_id) {
        s.active_model_id = model_id.to_string();
        s.stt_engine = REMOTE_OPENAI_ENGINE_ID.to_string();
        return Ok(());
    }
    if SHERPA_MODELS.iter().any(|m| m.engine_id == resolved_engine) {
        if !SHERPA_MODELS
            .iter()
            .any(|m| m.id == model_id && m.engine_id == resolved_engine)
        {
            return Err(format!(
                "没有模型 {model_id}（可选：{}）",
                model_ids().join(", ")
            ));
        }
        if !multi_model_ready_from(s, model_id) {
            return Err(format!("模型 {model_id} 尚未下载，请先下载再切换"));
        }
    } else {
        return Err(format!("引擎 {resolved_engine} 暂不支持模型切换"));
    }

    s.active_model_id = model_id.to_string();
    s.stt_engine = resolved_engine.to_string();
    let opts = s
        .engine_options
        .as_object_mut()
        .ok_or_else(|| "config.json 的 engineOptions 不是对象".to_string())?;
    let entry = opts
        .entry(resolved_engine.to_string())
        .or_insert_with(|| serde_json::json!({}));
    entry["model"] = serde_json::Value::String(model_id.to_string());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_contains_sherpa_models_without_vad_entry() {
        let items = list().unwrap();
        assert_eq!(
            items.len(),
            SHERPA_MODELS.len() + 1,
            "VAD 随本体分发；远程 OpenAI 兼容条目应出现在清单末尾"
        );
        assert!(items.iter().any(|i| i.id == REMOTE_OPENAI_STT_ID));
        assert!(
            items.iter().all(|i| i.id != VAD_MODEL_ID),
            "清单不应包含 silero-vad 伪条目"
        );
        let x = items
            .iter()
            .find(|i| i.id == "x-asr-480ms-streaming-zh-en-punct-int8-2026-06-05")
            .unwrap();
        assert_eq!(x.engine_id, SHERPA_STREAMING_ENGINE_ID);
        assert_eq!(
            x.size_bytes,
            SHERPA_MODELS[0]
                .files
                .iter()
                .map(|f| f.size_bytes)
                .sum::<u64>()
        );
    }

    /// 回归：IPC 序列化必须 camelCase（前端按 engineId/displayName/sizeBytes 读取；
    /// 曾因缺 rename_all 导致壳端模型清单分组键 undefined、引擎页模型区块整体不渲染）
    #[test]
    fn model_info_serializes_camel_case() {
        let items = list().unwrap();
        let json = serde_json::to_value(&items[0]).unwrap();
        let obj = json.as_object().unwrap();
        for key in [
            "engineId",
            "displayName",
            "sizeBytes",
            "downloadUrl",
            "io",
            "backend",
            "recipe",
            "configSchema",
        ] {
            assert!(obj.contains_key(key), "缺少 camelCase 键 {key}：{json}");
        }
        for key in ["engine_id", "display_name", "size_bytes", "download_url"] {
            assert!(!obj.contains_key(key), "不应出现 snake_case 键 {key}");
        }
    }

    #[test]
    fn install_guide_exposes_dest_and_official_page() {
        let guide = install_guide("funasr-nano-int8-2025-12-30").unwrap();
        assert_eq!(guide.id, "funasr-nano-int8-2025-12-30");
        assert_eq!(guide.dest_subdir, "sherpa-onnx-funasr-nano-int8-2025-12-30");
        assert!(guide.dest_dir.replace('\\', "/").ends_with(&guide.dest_subdir));
        assert_eq!(
            guide.page_url,
            "https://huggingface.co/csukuangfj/sherpa-onnx-funasr-nano-int8-2025-12-30"
        );
        assert!(
            guide
                .files
                .iter()
                .any(|f| f.name == "encoder_adaptor.int8.onnx" && f.size_bytes == 237_792_748)
        );
        assert!(
            guide
                .files
                .iter()
                .any(|f| f.name == "Qwen3-0.6B/merges.txt" && !f.url.is_empty())
        );

        let json = serde_json::to_value(&guide).unwrap();
        let obj = json.as_object().unwrap();
        for key in ["displayName", "destDir", "destSubdir", "pageUrl"] {
            assert!(obj.contains_key(key), "缺少 camelCase 键 {key}：{json}");
        }
    }

    #[test]
    fn huggingface_repo_page_keeps_owner_and_repo() {
        assert_eq!(
            huggingface_repo_page(
                "https://huggingface.co/csukuangfj/sherpa-onnx-funasr-nano-int8-2025-12-30/resolve/abc/encoder_adaptor.int8.onnx"
            )
            .as_deref(),
            Some("https://huggingface.co/csukuangfj/sherpa-onnx-funasr-nano-int8-2025-12-30")
        );
        assert_eq!(huggingface_repo_page("https://example.com/x"), None);
    }

    #[test]
    fn install_guide_modelscope_funasr_points_at_community_repo() {
        let guide = install_guide("funasr-nano-int8-modelscope").unwrap();
        assert_eq!(
            guide.page_url,
            "https://www.modelscope.cn/models/zengshuishui/FunASR-nano-onnx"
        );
        assert_eq!(guide.dest_subdir, "sherpa-onnx-funasr-nano-int8-modelscope");
        assert!(guide.files.iter().any(|f| f.name == "llm.int8.onnx"));
    }

    #[test]
    fn set_active_writes_model_id_and_derived_engine() {
        let mut s = settings::Settings::default();
        s.active_model_id.clear();
        set_active_in(&mut s, "ignored", REMOTE_OPENAI_STT_ID).unwrap();
        assert_eq!(s.active_model_id, REMOTE_OPENAI_STT_ID);
        assert_eq!(s.stt_engine, REMOTE_OPENAI_ENGINE_ID);
    }

    #[test]
    fn install_guide_unknown_model_errors() {
        let err = install_guide("no-such-model").unwrap_err();
        assert!(err.contains("未知模型"), "err: {err}");
    }

    #[test]
    fn funasr_has_no_modelscope_archive_until_files_match_official_hf() {
        let funasr = SHERPA_MODELS
            .iter()
            .find(|m| m.id == "funasr-nano-int8-2025-12-30")
            .unwrap();
        assert!(
            funasr.archive_mirrors.is_empty(),
            "魔搭 FunASR 整包与官方 HF 体积不一致（238277200 vs 237792748），不能再挂 archive_mirrors"
        );
        assert!(
            funasr.file_mirrors.is_empty() && funasr.archive_mirrors.is_empty(),
            "官方 FunASR 条目不能再挂和 HF 体积不一致的魔搭整包"
        );
        let community = SHERPA_MODELS
            .iter()
            .find(|m| m.id == "funasr-nano-int8-modelscope")
            .unwrap();
        assert!(
            !community.file_mirrors.is_empty(),
            "魔搭社区版应走 zengshuishui 逐文件"
        );
        let encoder = funasr
            .files
            .iter()
            .find(|f| f.name == "encoder_adaptor.int8.onnx")
            .unwrap();
        assert_eq!(
            encoder.size_bytes, 237_792_748,
            "官方 HF encoder 体积被改了的话，先核对魔搭原版 238277200 是不是又混进来了"
        );
    }

    #[test]
    fn vad_manifest_wellformed() {
        assert_eq!(VAD_MODEL_SHA256.len(), 64);
        assert!(VAD_MODEL_SHA256.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(VAD_MODEL_URL.starts_with("https://"));
        assert!(VAD_MODEL_URL.ends_with(VAD_MODEL_FILE));
        let expected_size = VAD_MODEL_SIZE;
        assert!(expected_size > 100_000);
        // 内嵌字节与清单一致（本体分发的就是清单钉死的那个文件）
        assert_eq!(VAD_BUNDLED_BYTES.len() as u64, VAD_MODEL_SIZE);
    }

    #[test]
    fn manifest_integrity_rejects_same_size_corruption() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tokens.txt");
        fs::write(&path, b"hello kotone").unwrap();
        let file = ModelFile {
            name: "tokens.txt",
            url: "https://example.invalid/tokens.txt",
            sha256: Some("5ea673601ae0ff62c361e4ef7c54faeefd3462fec6d90f2b295ba0758762e772"),
            size_bytes: 12,
        };
        assert!(verify_model_file(&path, &file).is_ok());
        assert!(model_file_present(&path, &file).is_ok());

        fs::write(&path, b"HELLO KOTONE").unwrap();
        invalidate_integrity_cache(&path);
        let error = verify_model_file(&path, &file).unwrap_err();
        assert!(error.contains("SHA256 不符"), "error: {error}");
        // 列举/就绪只看大小：同大小损坏仍算「已下载」，哈希留给下载路径
        assert!(model_file_present(&path, &file).is_ok());

        fs::write(&path, b"short").unwrap();
        let present_err = model_file_present(&path, &file).unwrap_err();
        assert!(present_err.contains("大小不符"), "error: {present_err}");
    }

    // ---------- VAD 本体分发：ensure_vad_model ----------

    #[test]
    fn ensure_vad_model_unpacks_bundled_bytes() {
        let tmp = tempfile::tempdir().unwrap();
        let models = tmp.path().join("models");

        // 首次：解包内嵌字节落盘
        assert_eq!(ensure_vad_model_in(&models), Ok(true));
        let dest = models.join(VAD_MODEL_FILE);
        let written = fs::read(&dest).unwrap();
        assert_eq!(written.len() as u64, VAD_MODEL_SIZE);
        assert_eq!(written, VAD_BUNDLED_BYTES, "落盘内容应与内嵌字节一致");
        assert!(
            !models.join(format!("{VAD_MODEL_FILE}.tmp")).exists(),
            "临时文件应已 rename"
        );

        // 幂等：已就绪不再写
        assert_eq!(ensure_vad_model_in(&models), Ok(false));
    }

    #[test]
    fn ensure_vad_model_replaces_same_size_corruption() {
        let tmp = tempfile::tempdir().unwrap();
        let models = tmp.path().join("models");
        fs::create_dir_all(&models).unwrap();
        // 同大小但哈希不符也必须覆盖，不能把损坏文件误判为就绪。
        let dest = models.join(VAD_MODEL_FILE);
        fs::write(&dest, vec![0xABu8; VAD_MODEL_SIZE as usize]).unwrap();
        assert_eq!(ensure_vad_model_in(&models), Ok(true));
        assert_eq!(fs::read(&dest).unwrap(), VAD_BUNDLED_BYTES);

        // 大小不符（残缺文件）→ 重新解包覆盖
        fs::write(&dest, b"truncated").unwrap();
        assert_eq!(ensure_vad_model_in(&models), Ok(true));
        assert_eq!(fs::read(&dest).unwrap().len() as u64, VAD_MODEL_SIZE);
    }

    #[test]
    fn sherpa_manifest_wellformed() {
        const REGISTERED_ENGINES: &[&str] =
            &[SHERPA_STREAMING_ENGINE_ID, SHERPA_OFFLINE_ENGINE_ID];
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
            for mirror in m.file_mirrors {
                assert!(
                    mirror
                        .base_url
                        .starts_with("https://www.modelscope.cn/models/"),
                    "{} 镜像必须是 ModelScope 模型地址",
                    m.id
                );
                assert_eq!(mirror.revision.len(), 40, "{} 镜像必须固定 commit", m.id);
                assert!(
                    mirror.revision.chars().all(|c| c.is_ascii_hexdigit()),
                    "{} 镜像 revision 必须是 hex",
                    m.id
                );
            }
            for archive in m.archive_mirrors {
                assert!(
                    archive
                        .url
                        .starts_with("https://www.modelscope.cn/models/"),
                    "{} 整包镜像必须是 ModelScope 地址",
                    m.id
                );
                assert_eq!(archive.sha256.len(), 64, "{} 整包镜像 sha256 应 64 hex", m.id);
                assert!(archive.size_bytes > 0, "{}", m.id);
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
                let sha256 = f.sha256.expect("每个模型文件都必须固定 SHA256");
                assert_eq!(sha256.len(), 64, "{} sha256 应 64 hex", f.name);
                assert!(sha256.chars().all(|c| c.is_ascii_hexdigit()), "{}", f.name);
                assert!(
                    !f.url.contains("/resolve/main/"),
                    "{} URL 不得跟随 main",
                    f.name
                );
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
            .find(|m| m.recipe == ModelRecipe::SenseVoice)
            .expect("SenseVoice 模型清单缺失");
        let sv_names: Vec<_> = sv.files.iter().map(|f| f.name).collect();
        assert!(sv_names.contains(&"model.int8.onnx"));
        assert!(sv_names.contains(&"tokens.txt"));
        assert_eq!(sensevoice_default_model(), sv.id);
    }

    #[test]
    fn new_engines_manifest_entries() {
        // X-ASR：官方整包 + 两个固定 revision 的 ModelScope 逐文件镜像
        let x = SHERPA_MODELS
            .iter()
            .find(|m| m.recipe == ModelRecipe::ZipformerTransducer)
            .expect("X-ASR 模型清单缺失");
        assert!(x.archive.is_some(), "X-ASR 应走整包下载");
        assert_eq!(x.file_mirrors.len(), 2, "X-ASR 应有主、备两个国内镜像");
        for mirror in x.file_mirrors {
            let url = file_mirror_url(mirror, "encoder.int8.onnx");
            assert!(url.contains("/resolve/"));
            assert!(url.contains(mirror.revision));
            assert!(url.ends_with("/encoder.int8.onnx"));
        }
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
        assert_eq!(
            multi_file_default_model("sherpa-onnx-x-asr-zh-en"),
            Some(x.id)
        );

        // FunASR-Nano：encoder_adaptor/llm/embedding + Qwen3-0.6B tokenizer 目录
        let fun = SHERPA_MODELS
            .iter()
            .find(|m| m.id == "funasr-nano-int8-2025-12-30")
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
            Some(fun.id),
            "默认仍是官方 HF 条目（清单里该引擎第一条）"
        );
        assert!(
            fun.file_mirrors.is_empty(),
            "官方 FunASR-Nano 走 HF 逐文件，不挂魔搭镜像"
        );
        assert!(fun.archive_mirrors.is_empty());

        let fun_ms = SHERPA_MODELS
            .iter()
            .find(|m| m.id == "funasr-nano-int8-modelscope")
            .expect("缺少 FunASR-Nano 魔搭社区条目");
        assert_eq!(fun_ms.engine_id, SHERPA_OFFLINE_ENGINE_ID);
        assert_eq!(fun_ms.file_mirrors.len(), 1);
        assert!(fun_ms.file_mirrors[0]
            .base_url
            .contains("zengshuishui/FunASR-nano-onnx"));
        assert_eq!(
            remote_path(fun_ms, "llm.int8.onnx"),
            "llm_int8/llm.int8.onnx"
        );
        assert_eq!(
            remote_path(fun_ms, "encoder_adaptor.int8.onnx"),
            "encoder_adaptor.int8.onnx"
        );
        let encoder = fun_ms
            .files
            .iter()
            .find(|f| f.name == "encoder_adaptor.int8.onnx")
            .unwrap();
        assert_eq!(encoder.size_bytes, 238_277_200);

        let sv = SHERPA_MODELS
            .iter()
            .find(|m| m.recipe == ModelRecipe::SenseVoice)
            .expect("SenseVoice 模型清单缺失");
        assert_eq!(sv.file_mirrors.len(), 1);
        assert!(sv.file_mirrors[0]
            .base_url
            .contains("fengge2024/sherpa-onnx-sense-voice"));
    }

    #[test]
    fn new_engines_active_model_mapping() {
        // 各引擎共用泛化映射：未配置 → 清单默认；合法 id → 采用；跨引擎 id → 兜底
        for engine in ["sherpa-onnx-x-asr-zh-en", "sherpa-onnx-funasr-nano"] {
            let default = multi_file_default_model(engine).unwrap();
            let s = settings::Settings::default();
            assert_eq!(
                active_model_from(&s, engine),
                default,
                "{engine} 未配置兜底"
            );

            let mut s2 = settings::Settings::default();
            s2.engine_options[engine]["model"] = serde_json::json!(default);
            assert_eq!(active_model_from(&s2, engine), default, "{engine} 合法 id");

            let mut s3 = settings::Settings::default();
            s3.engine_options[engine]["model"] =
                serde_json::json!("sense-voice-zh-en-ja-ko-yue-2024-07-17");
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
            serde_json::json!("x-asr-480ms-streaming-zh-en-punct-int8-2026-06-05");
        assert_eq!(
            active_model_from(&s3, "sherpa-onnx-sensevoice"),
            sensevoice_default_model(),
            "跨引擎模型 id 不应被采用"
        );
    }

    #[test]
    fn active_model_defaults_per_engine() {
        // 未配置时（或配置了未知 id 时）的兜底按引擎区分
        let x = active_model("sherpa-onnx-x-asr-zh-en");
        assert!(
            SHERPA_MODELS.iter().any(|m| m.id == x),
            "X-ASR 默认模型应在清单内：{x}"
        );
    }

    #[test]
    fn active_model_respects_io_engine_and_ignores_other_loop() {
        let mut s = settings::Settings::default();
        s.active_model_id = "x-asr-480ms-streaming-zh-en-punct-int8-2026-06-05".into();
        assert_eq!(
            active_model_from(&s, SHERPA_STREAMING_ENGINE_ID),
            "x-asr-480ms-streaming-zh-en-punct-int8-2026-06-05"
        );
        assert_eq!(
            active_model_from(&s, "sherpa-onnx-x-asr-zh-en"),
            "x-asr-480ms-streaming-zh-en-punct-int8-2026-06-05"
        );
        assert_eq!(
            active_model_from(&s, SHERPA_OFFLINE_ENGINE_ID),
            sensevoice_default_model(),
            "流式活动模型不应污染非流式循环"
        );

        s.active_model_id = "sense-voice-zh-en-ja-ko-yue-2024-07-17".into();
        assert_eq!(
            active_model_from(&s, SHERPA_OFFLINE_ENGINE_ID),
            "sense-voice-zh-en-ja-ko-yue-2024-07-17"
        );
        assert_eq!(
            active_model_from(&s, SHERPA_STREAMING_ENGINE_ID),
            multi_file_default_model(SHERPA_STREAMING_ENGINE_ID).unwrap()
        );
    }

    #[test]
    fn model_id_from_cfg_prefers_session_options() {
        let cfg = SessionConfig {
            options: serde_json::json!({
                "model": "funasr-nano-int8-2025-12-30"
            }),
            ..SessionConfig::default()
        };
        assert_eq!(
            model_id_from_cfg(&cfg, SHERPA_OFFLINE_ENGINE_ID),
            "funasr-nano-int8-2025-12-30"
        );
        assert_eq!(
            model_id_from_cfg(&cfg, SHERPA_STREAMING_ENGINE_ID),
            multi_file_default_model(SHERPA_STREAMING_ENGINE_ID).unwrap(),
            "跨循环的 options.model 应被忽略"
        );
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
        assert_eq!(
            fs::read_to_string(&vocab).unwrap(),
            "<blk>\t0\n▁HEL\t-1.5\nLOW\t-2\n"
        );
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
            [
                0x0a, 0x0e, 0x0a, 0x05, b'<', b'b', b'l', b'k', b'>', 0x15, 0, 0, 0, 0,
            ],
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
    fn models_dir_path_requires_ascii() {
        assert!(validate_models_dir_path(Path::new("D:\\Kotone Models\\v1")).is_ok());
        let err = validate_models_dir_path(Path::new("E:\\琴音模型")).unwrap_err();
        assert!(err.contains("纯英文路径"));
        assert!(err.contains("D:\\KotoneModels"));
    }

    #[test]
    fn active_model_from_matches_disk_version_fallbacks() {
        let s = settings::Settings::default();
        // 与 active_model 同一兜底：sherpa 系未配置 → 清单默认；无清单引擎 → "default"
        assert_eq!(
            active_model_from(&s, "sherpa-onnx-x-asr-zh-en"),
            multi_file_default_model("sherpa-onnx-x-asr-zh-en").unwrap()
        );
        assert_eq!(active_model_from(&s, "mock-stream"), "default");
        // 无清单引擎配置了 id → 原样透传
        let mut s = settings::Settings::default();
        s.engine_options["mock-stream"]["model"] = serde_json::json!("any-id");
        assert_eq!(active_model_from(&s, "mock-stream"), "any-id");
    }

    // ---------- 目录迁移 ----------

    #[test]
    fn migrate_moves_files_and_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("old");
        let dst = tmp.path().join("new");
        fs::create_dir_all(src.join("sherpa-dir")).unwrap();
        fs::write(src.join("model.bin"), b"model").unwrap();
        fs::write(src.join("sherpa-dir/tokens.txt"), b"tokens").unwrap();

        let report = migrate_dir_contents(&src, &dst).unwrap();
        assert_eq!(report.failed.len(), 0, "report: {report:?}");
        assert_eq!(report.moved.len(), 2);
        assert_eq!(fs::read(dst.join("model.bin")).unwrap(), b"model");
        assert_eq!(
            fs::read(dst.join("sherpa-dir/tokens.txt")).unwrap(),
            b"tokens"
        );
        assert!(!src.join("model.bin").exists(), "源文件应已移走");
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

    /// P2-⑧：目标已有同名条目时不得静默删源——
    /// 同内容文件视为已迁移；不同内容文件（包括同大小）保留为 conflict 副本；
    /// 同名目录记 failed。
    #[test]
    fn migrate_preserves_conflicting_same_name_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("old");
        let dst = tmp.path().join("new");
        fs::create_dir_all(&src).unwrap();
        fs::create_dir_all(&dst).unwrap();

        // 1) 同名同内容文件：视为已迁移，源删除
        fs::write(src.join("same.bin"), b"abc").unwrap();
        fs::write(dst.join("same.bin"), b"abc").unwrap();
        // 2) 同名不同长度文件：源保留为 conflict 副本，不丢数据
        fs::write(src.join("diff.bin"), b"old-content").unwrap();
        fs::write(dst.join("diff.bin"), b"new-content-longer").unwrap();
        // 3) 同名同长度但内容不同：也必须保留为 conflict 副本
        fs::write(src.join("same-size.bin"), b"abc").unwrap();
        fs::write(dst.join("same-size.bin"), b"xyz").unwrap();
        // 4) 同名目录：保守记 failed，源目录原样保留
        fs::create_dir_all(src.join("dir")).unwrap();
        fs::create_dir_all(dst.join("dir")).unwrap();

        let report = migrate_dir_contents(&src, &dst).unwrap();
        assert!(
            report.moved.contains(&"same.bin".to_string()),
            "同内容文件应视为已迁移: {report:?}"
        );
        assert!(
            report.moved.contains(&"diff.bin".to_string()),
            "冲突文件应改名保留并计入 moved: {report:?}"
        );
        assert!(
            report.moved.contains(&"same-size.bin".to_string()),
            "同长度但内容不同的冲突文件也应改名保留: {report:?}"
        );
        assert!(report.failed.contains(&"dir".to_string()), "{report:?}");

        assert!(!src.join("same.bin").exists(), "同内容源应删除");
        assert_eq!(
            fs::read(dst.join("diff.bin")).unwrap(),
            b"new-content-longer"
        );
        let conflicts = fs::read_dir(&dst)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with("diff.bin.conflict-"))
            .count();
        assert_eq!(conflicts, 1, "冲突源应保留为 conflict 副本");
        let same_size_conflicts = fs::read_dir(&dst)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with("same-size.bin.conflict-"))
            .count();
        assert_eq!(
            same_size_conflicts, 1,
            "同长度冲突源也应保留为 conflict 副本"
        );
        assert!(src.join("dir").is_dir(), "同名目录冲突时源目录不应被删除");
    }

    // ---------- 删除 ----------

    #[test]
    fn delete_files_in_removes_each_kind() {
        let tmp = tempfile::tempdir().unwrap();
        let models = tmp.path().join("models");
        let sherpa_dir = models.join(SHERPA_MODELS[0].dir);
        fs::create_dir_all(&sherpa_dir).unwrap();
        fs::write(sherpa_dir.join("tokens.txt"), b"x").unwrap();
        fs::write(models.join(VAD_MODEL_FILE), b"x").unwrap();
        fs::write(models.join("unrelated.txt"), b"x").unwrap();

        delete_files_in(&models, SHERPA_MODELS[0].id).unwrap();
        assert!(!sherpa_dir.exists(), "多文件模型删整个目录");
        delete_files_in(&models, VAD_MODEL_ID).unwrap();
        assert!(!models.join(VAD_MODEL_FILE).exists());
        assert!(models.join("unrelated.txt").exists(), "无关文件不动");
        // 幂等：再删一遍不报错；未知 id 报错
        delete_files_in(&models, SHERPA_MODELS[0].id).unwrap();
        assert!(delete_files_in(&models, "no-such").is_err());
    }
}
