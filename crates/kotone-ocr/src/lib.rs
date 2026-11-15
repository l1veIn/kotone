//! kotone-ocr：工具服务 M2 的 OCR 后端实现（端口见 `kotone_core::ocr`）。
//!
//! - `local`：本地 PP-OCRv5（检测 + 按语种分包的识别模型，清单见 `manifest::PACKAGES`），
//!   ONNX Runtime CPU（`onnx` feature；运行时随 sherpa-onnx 静态链接，不新增原生库）。
//!   模型由设置页或 `install` 显式安装并逐文件校验。
//! - `catalog`：统一模型清单的 OCR 类别（本地包 + DeepSeek 在线条目、当前模型选择）。
//! - `deepseek`：DeepSeek 在线 OCR，复用连接目录与系统凭据库，只上传指定图像。
//! - 两类后端互不降级：本地失败不会改走在线，在线失败也不会改走本地。

pub mod catalog;
pub mod deepseek;
pub mod install;
pub mod local;
pub mod manifest;
pub mod onnx_meta;
#[cfg(feature = "onnx")]
pub mod ort;
pub mod ppocr;

// 确保 sherpa-onnx 静态预编译库（含 onnxruntime.lib）参与最终链接。
#[cfg(feature = "onnx")]
extern crate sherpa_onnx_sys as _;

pub use deepseek::DeepSeekOcr;
pub use local::{LocalKoreanOcr, LocalPpOcr};

/// 本地模型包目录：`KOTONE_OCR_MODEL_DIR`（测试/隔离冒烟）优先，否则 `<模型根>/ocr/<模型 ID>`。
pub fn local_model_dir(models_root: &std::path::Path) -> std::path::PathBuf {
    match std::env::var_os(manifest::MODEL_DIR_ENV) {
        Some(dir) if !dir.is_empty() => std::path::PathBuf::from(dir),
        _ => manifest::package_dir(models_root, &manifest::KOREAN_MOBILE),
    }
}
