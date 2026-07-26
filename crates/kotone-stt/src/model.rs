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
    pub url: &'static str,
    pub sha256: Option<&'static str>,
    pub size_bytes: u64,
}

/// 多文件模型清单条目（下载为 ~/.kotone/models/<dir>/<files>）
pub struct MultiFileModel {
    pub id: &'static str,
    pub engine_id: &'static str,
    pub display_name: &'static str,
    pub dir: &'static str,
    pub files: &'static [ModelFile],
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
    },
];

/// sherpa 引擎的默认模型（engineOptions 未配置或配置了未知 id 时的兜底）
pub fn sherpa_default_model() -> &'static str {
    SHERPA_MODELS[0].id
}

/// SenseVoice 引擎的默认模型（单模型清单，恒为清单条目）
pub fn sensevoice_default_model() -> &'static str {
    SHERPA_MODELS
        .iter()
        .find(|m| m.engine_id == "sherpa-onnx-sensevoice")
        .expect("SenseVoice 模型清单缺失")
        .id
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
    match (engine_id, configured) {
        // sherpa 系：配置的 id 必须属于该引擎自己的清单（跨引擎 id 不认），否则
        // 兜底该引擎清单默认（config.json 早期默认值 zipformer-zh-small 是占位串）
        ("sherpa-onnx-zipformer-zh", Some(id)) | ("sherpa-onnx-sensevoice", Some(id))
            if SHERPA_MODELS
                .iter()
                .any(|m| m.id == id && m.engine_id == engine_id) =>
        {
            id
        }
        ("sherpa-onnx-zipformer-zh", _) => sherpa_default_model().to_string(),
        ("sherpa-onnx-sensevoice", _) => sensevoice_default_model().to_string(),
        (_, Some(id)) => id,
        _ => "ggml-small".to_string(),
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
        return download_multi(m, progress);
    }
    Err(format!(
        "未知模型：{id}（可选：{}）",
        model_ids().join(", ")
    ))
}

/// 多文件模型：逐文件下载，聚合进度（已完成文件字节 + 当前文件进度）
fn download_multi(m: &MultiFileModel, progress: Progress<'_>) -> Result<(), String> {
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

/// 可选模型 ID 列表（错误提示用）
fn model_ids() -> Vec<&'static str> {
    MODELS
        .iter()
        .map(|m| m.id)
        .chain(SHERPA_MODELS.iter().map(|m| m.id))
        .chain([VAD_MODEL_ID])
        .collect()
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
        "sherpa-onnx-zipformer-zh" | "sherpa-onnx-sensevoice" => {
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
        }
        _ => return Err(format!("引擎 {engine_id} 暂不支持模型切换")),
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
        let mut ids: Vec<_> = SHERPA_MODELS.iter().map(|m| m.id).collect();
        let n = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), n, "sherpa 模型 ID 应唯一");
        for m in SHERPA_MODELS {
            assert!(
                m.engine_id == "sherpa-onnx-zipformer-zh" || m.engine_id == "sherpa-onnx-sensevoice",
                "{} 的 engine_id 未注册：{}",
                m.id,
                m.engine_id
            );
            assert!(!m.files.is_empty(), "{}", m.id);
            for f in m.files {
                assert!(f.url.starts_with("https://"), "{}", f.name);
                assert!(f.url.ends_with(f.name), "{} URL 应以文件名结尾", f.name);
                assert!(f.size_bytes > 0, "{}", f.name);
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
