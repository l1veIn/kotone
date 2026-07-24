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

/// ~/.kotone/models/
pub fn models_dir() -> PathBuf {
    settings::kotone_dir().join("models")
}

/// whisper-cli 可执行文件路径
pub fn whisper_cli_path() -> PathBuf {
    bin_dir().join("whisper-cli.exe")
}

/// 模型文件路径
pub fn model_path(model_id: &str) -> Option<PathBuf> {
    MODELS
        .iter()
        .find(|m| m.id == model_id)
        .map(|m| models_dir().join(m.file))
}

/// 引擎当前活动模型 ID（读 config.json 的 engineOptions，默认 ggml-small）
pub fn active_model(engine_id: &str) -> String {
    let s = settings::load();
    s.engine_options
        .get(engine_id)
        .and_then(|o| o.get("model"))
        .and_then(|m| m.as_str())
        .unwrap_or("ggml-small")
        .to_string()
}

/// whisper-cli 运行时就绪（exe + 核心 DLL 都在）
pub fn bin_installed() -> bool {
    whisper_cli_path().exists() && bin_dir().join("whisper.dll").exists()
}

// ---------- 清单查询 ----------

/// 列出全部模型 + whisper-cli 运行时条目
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

/// 下载模型或 whisper-cli 运行时（id = ggml-* / whisper-cli），进度经回调外发。
/// 阻塞实现：调用方放阻塞线程。
pub fn download(id: &str, progress: Progress<'_>) -> Result<(), String> {
    if id == WHISPER_BIN_ID {
        return download_bin(progress);
    }
    let m = MODELS
        .iter()
        .find(|m| m.id == id)
        .ok_or_else(|| format!("未知模型：{id}（可选：{}）", model_ids().join(", ")))?;
    let dest = models_dir().join(m.file);
    download::download_file(m.url, &dest, Some(m.sha256), progress)
}

/// 可选模型 ID 列表（错误提示用）
fn model_ids() -> Vec<&'static str> {
    MODELS.iter().map(|m| m.id).collect()
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

// ---------- 活动模型切换 ----------

/// 切换引擎的活动模型：写入 config.json 的 engineOptions[engine_id].model。
/// 模型文件须已下载（否则切了也用不了）。
pub fn set_active(engine_id: &str, model_id: &str) -> Result<(), String> {
    let manifest = MODELS
        .iter()
        .find(|m| m.id == model_id && m.engine_id == engine_id);
    match engine_id {
        "whisper-cpp-sidecar" => {
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
        assert_eq!(items.len(), MODELS.len() + 1);
        assert!(items.iter().any(|i| i.id == WHISPER_BIN_ID));
        let small = items.iter().find(|i| i.id == "ggml-small").unwrap();
        assert_eq!(small.engine_id, "whisper-cpp-sidecar");
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
}
