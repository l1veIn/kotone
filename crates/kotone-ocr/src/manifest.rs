//! 本地 OCR 模型清单：来源、许可、版本、文件大小与 SHA-256。
//!
//! 权重不随仓库或安装包分发；`kotone-cli ocr install` 按本清单下载到用户模型目录，
//! 逐文件校验大小与 SHA-256 后才原子发布。哈希与 RapidOCR v3.10.0 发布的
//! `default_models.yaml` 一致，并已对下载结果复核（见 docs/research/game-tools-m2-implementation.md）。

/// 一个模型文件。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelFile {
    /// 安装后的文件名（模型目录内）。
    pub name: &'static str,
    /// 角色：`det`（文字检测）/ `rec`（文字识别）。
    pub role: &'static str,
    pub url: &'static str,
    pub sha256: &'static str,
    pub bytes: u64,
}

/// 一个可安装的本地 OCR 模型包。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelPackage {
    pub id: &'static str,
    /// `ocr.backends` 里对应的后端 ID。
    pub backend_id: &'static str,
    pub display_name: &'static str,
    /// 设置页的一句话说明（适用语种、精度 / 速度取舍）。
    pub description: &'static str,
    /// 统一模型清单里的默认推荐。
    pub recommended: bool,
    /// 人类可读的模型版本。
    pub version: &'static str,
    pub languages: &'static [&'static str],
    pub license: &'static str,
    pub license_url: &'static str,
    /// 上游项目与转换来源。
    pub upstream: &'static str,
    pub distribution: &'static str,
    pub files: &'static [ModelFile],
}

pub const KOREAN_MOBILE_ID: &str = "ppocrv5-korean-mobile";

/// PP-OCRv5 mobile：多语言检测（ch_PP-OCRv5_det_mobile）+ 韩文识别（korean_PP-OCRv5_rec_mobile）。
/// 字符表内嵌于识别模型的 ONNX metadata（`character`），无需单独字典文件。
pub const KOREAN_MOBILE: ModelPackage = ModelPackage {
    id: KOREAN_MOBILE_ID,
    backend_id: "local-ppocrv5-korean",
    display_name: "PP-OCRv5 mobile Korean (local, CPU)",
    description: "韩文聊天（韩服）。本机 CPU 推理，不上传截图",
    recommended: false,
    version: "PP-OCRv5 mobile — det ch_PP-OCRv5_det_mobile + rec korean_PP-OCRv5_rec_mobile; ONNX export from RapidOCR v3.10.0",
    languages: &["ko"],
    license: "Apache-2.0",
    license_url: "https://www.apache.org/licenses/LICENSE-2.0",
    upstream: "PaddleOCR PP-OCRv5 (PaddlePaddle, Apache-2.0); ONNX conversion by RapidAI/RapidOCR (Apache-2.0)",
    distribution: "https://www.modelscope.cn/models/RapidAI/RapidOCR (tag v3.10.0)",
    files: &[
        ModelFile {
            name: "ch_PP-OCRv5_det_mobile.onnx",
            role: "det",
            url: "https://www.modelscope.cn/models/RapidAI/RapidOCR/resolve/v3.10.0/onnx/PP-OCRv5/det/ch_PP-OCRv5_det_mobile.onnx",
            sha256: "4d97c44a20d30a81aad087d6a396b08f786c4635742afc391f6621f5c6ae78ae",
            bytes: 4_819_576,
        },
        ModelFile {
            name: "korean_PP-OCRv5_rec_mobile.onnx",
            role: "rec",
            url: "https://www.modelscope.cn/models/RapidAI/RapidOCR/resolve/v3.10.0/onnx/PP-OCRv5/rec/korean_PP-OCRv5_rec_mobile.onnx",
            sha256: "cd6e2ea50f6943ca7271eb8c56a877a5a90720b7047fe9c41a2e541a25773c9b",
            bytes: 13_488_748,
        },
    ],
};

/// 统一模型清单里的本地 OCR 包（按推荐顺序；未选择 OCR 模型时取第一个已安装的）。
pub const PACKAGES: &[ModelPackage] = &[KOREAN_MOBILE];

pub fn package(id: &str) -> Option<&'static ModelPackage> {
    PACKAGES.iter().find(|p| p.id == id)
}

impl ModelPackage {
    pub fn file(&self, role: &str) -> Option<&ModelFile> {
        self.files.iter().find(|f| f.role == role)
    }

    pub fn total_bytes(&self) -> u64 {
        self.files.iter().map(|f| f.bytes).sum()
    }
}

/// 模型目录：`<模型根>/ocr/<模型 ID>`。模型根与 ASR 相同（settings.models.dir 或 ~/.kotone/models）。
pub fn package_dir(models_root: &std::path::Path, package: &ModelPackage) -> std::path::PathBuf {
    models_root.join("ocr").join(package.id)
}

/// 测试/隔离冒烟用：覆盖 OCR 模型包目录（直接指向包目录）。
pub const MODEL_DIR_ENV: &str = "KOTONE_OCR_MODEL_DIR";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_is_well_formed() {
        let p = KOREAN_MOBILE;
        assert_eq!(p.files.len(), 2);
        for f in p.files {
            assert_eq!(f.sha256.len(), 64);
            assert!(f.sha256.bytes().all(|b| b.is_ascii_hexdigit()));
            assert!(f.url.starts_with("https://www.modelscope.cn/"));
            assert!(f.url.ends_with(f.name));
            assert!(f.bytes > 1_000_000);
        }
        assert!(p.file("det").is_some() && p.file("rec").is_some());
        assert_eq!(p.total_bytes(), 18_308_324);
    }
}
