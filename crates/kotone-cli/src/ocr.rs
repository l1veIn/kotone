//! `kotone-cli ocr`：本地 OCR 模型的列表、安装入口与就绪检查（docs/tool-service.md §OCR）。
//! 不带 `--model` 时沿用 M2 的韩文包，行为与以前相同。
//!
//! - `status`：逐文件核对模型存在、大小与 SHA-256，并报告本构建的 ONNX Runtime 是否可用；
//!   不访问网络，不连接桌面实例。
//! - `install`：显式下载（或 `--from-dir` 离线复制）模型到用户模型目录；先写临时文件，
//!   校验大小与 SHA-256 后才原子发布。普通测试从不调用真实下载。
//!
//! 桌面实例在每次 `ocr.backends` / `ocr.recognize` 时重新检查模型，安装后无需重启即可使用。
//!
//! 退出码：0 成功/就绪；1 未就绪（status）或安装失败；2 用法错误。

use std::path::{Path, PathBuf};

use clap::Subcommand;
use serde_json::{json, Value};

use kotone_core::ocr::OcrBackend;
use kotone_ocr::install::{self, InstallSource, PackageState, Verifier};
use kotone_ocr::manifest::{self, ModelPackage, KOREAN_MOBILE_ID};
use kotone_ocr::LocalPpOcr;

#[derive(Subcommand, Debug)]
pub enum OcrCommand {
    /// 检查本地 OCR 模型（存在、大小、SHA-256）与本构建的推理运行时
    Status {
        /// 模型 ID（缺省 ppocrv5-korean-mobile；全部 ID 见 `kotone-cli ocr list`）
        #[arg(long, default_value = KOREAN_MOBILE_ID)]
        model: String,
        /// 模型包目录（缺省：<模型根>/ocr/<模型 ID>；韩文包另可用 KOTONE_OCR_MODEL_DIR）
        #[arg(long)]
        model_dir: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// 安装本地 OCR 模型（PP-OCRv5；显式访问网络）
    Install {
        /// 模型 ID（缺省 ppocrv5-korean-mobile；全部 ID 见 `kotone-cli ocr list`）
        #[arg(long, default_value = KOREAN_MOBILE_ID)]
        model: String,
        #[arg(long)]
        model_dir: Option<PathBuf>,
        /// 从本地目录复制同名文件（离线安装；仍逐文件校验 SHA-256）
        #[arg(long)]
        from_dir: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// 列出可安装的本地 OCR 模型（ID、语种、大小）
    List {
        #[arg(long)]
        json: bool,
    },
}

fn find_package(id: &str) -> Option<&'static ModelPackage> {
    let package = manifest::package(id);
    if package.is_none() {
        let ids: Vec<&str> = manifest::PACKAGES.iter().map(|p| p.id).collect();
        eprintln!("未知 OCR 模型 {id}（可选：{}）", ids.join(", "));
    }
    package
}

fn default_dir(package: &ModelPackage) -> PathBuf {
    let root = kotone_stt::model::models_dir();
    if package.id == KOREAN_MOBILE_ID {
        kotone_ocr::local_model_dir(&root)
    } else {
        manifest::package_dir(&root, package)
    }
}

fn status_json(dir: &Path, package: &ModelPackage) -> (bool, Value) {
    let backend = LocalPpOcr::with_package(dir.to_path_buf(), *package);
    let info = backend.info();
    let value = json!({
        "backend": info.id,
        "ready": info.ready,
        "state": info.state,
        "reason": info.reason,
        "languages": info.languages,
        "modelDir": dir.display().to_string(),
        "details": info.details,
    });
    (info.ready, value)
}

pub async fn run(command: OcrCommand) -> i32 {
    match command {
        OcrCommand::List { json } => {
            let list: Vec<Value> = manifest::PACKAGES
                .iter()
                .map(|p| json!({"id": p.id, "backend": p.backend_id, "languages": p.languages, "bytes": p.total_bytes(), "description": p.description}))
                .collect();
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&list).unwrap_or_default()
                );
            } else {
                for p in manifest::PACKAGES {
                    println!(
                        "{:<28} {:>6.1} MB  {:<24} {}",
                        p.id,
                        p.total_bytes() as f64 / 1_000_000.0,
                        p.languages.join(","),
                        p.description
                    );
                }
            }
            0
        }
        OcrCommand::Status {
            model,
            model_dir,
            json,
        } => {
            let Some(package) = find_package(&model) else {
                return 2;
            };
            let dir = model_dir.unwrap_or_else(|| default_dir(package));
            let (ready, value) = status_json(&dir, package);
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&value).unwrap_or_default()
                );
            } else {
                println!("后端      {}", value["backend"].as_str().unwrap_or(""));
                println!("状态      {}", value["state"].as_str().unwrap_or(""));
                if let Some(reason) = value["reason"].as_str() {
                    println!("原因      {reason}");
                }
                println!("模型目录  {}", dir.display());
                println!(
                    "运行时    onnxruntime {}",
                    value["details"]["runtime"]["version"]
                        .as_str()
                        .unwrap_or("不可用")
                );
                println!("模型版本  {}", package.version);
                println!("许可      {}", package.license);
                for f in package.files {
                    println!(
                        "  {} ({}, {} 字节, sha256 {})",
                        f.name, f.role, f.bytes, f.sha256
                    );
                }
            }
            if ready {
                0
            } else {
                1
            }
        }
        OcrCommand::Install {
            model,
            model_dir,
            from_dir,
            json,
        } => {
            let Some(package) = find_package(&model) else {
                return 2;
            };
            let dir = model_dir.unwrap_or_else(|| default_dir(package));
            if let Some(src) = &from_dir {
                if !src.is_dir() {
                    eprintln!("--from-dir {} 不是目录", src.display());
                    return 2;
                }
            }
            let source = match &from_dir {
                Some(src) => InstallSource::FromDir(src),
                None => InstallSource::Download,
            };
            if !json {
                eprintln!(
                    "安装 {}（{} 字节，{}）到 {}",
                    package.id,
                    package.total_bytes(),
                    package.distribution,
                    dir.display()
                );
            }
            let quiet = json;
            // 每跨过 1 MiB 或完成时刷新一次单行进度
            let last_mib = std::sync::atomic::AtomicU64::new(u64::MAX);
            let progress = move |name: &str, done: u64, total: u64| {
                let mib = done >> 20;
                let previous = last_mib.swap(mib, std::sync::atomic::Ordering::Relaxed);
                if quiet || (previous == mib && done != total) {
                    return;
                }
                eprint!(
                    "\r  {name}: {:.1}/{:.1} MiB",
                    done as f64 / 1_048_576.0,
                    total as f64 / 1_048_576.0
                );
                if done == total {
                    eprintln!();
                }
            };
            let started = std::time::Instant::now();
            match install::install(package, &dir, source, &progress).await {
                Ok(report) => {
                    let state = Verifier::new().check(&dir, package).state;
                    let value = json!({
                        "ok": state == PackageState::Ready,
                        "modelDir": dir.display().to_string(),
                        "files": report.files.iter().map(|(n, a)| json!({"name": n, "action": a})).collect::<Vec<_>>(),
                        "elapsedMs": started.elapsed().as_millis() as u64,
                    });
                    if json {
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&value).unwrap_or_default()
                        );
                    } else {
                        for (name, action) in &report.files {
                            println!(
                                "{name}: {}",
                                if *action == "kept" {
                                    "已存在，校验通过"
                                } else {
                                    "已安装并校验"
                                }
                            );
                        }
                        println!(
                            "本地 OCR 模型 {} 就绪。桌面实例下次识别时加载，在「设置 → 模型 → 文字识别」里选用它。",
                            package.id
                        );
                    }
                    0
                }
                Err(error) => {
                    if json {
                        println!("{}", json!({"ok": false, "error": error}));
                    } else {
                        eprintln!("安装失败: {error}");
                    }
                    1
                }
            }
        }
    }
}
