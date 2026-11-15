//! 本地 OCR 模型清单：来源、许可、版本、文件大小与 SHA-256。
//!
//! 权重不随仓库或安装包分发；设置页「高级 → 模型 → 文字识别」或 `kotone-cli ocr install`
//! 按本清单下载到用户模型目录，逐文件校验大小与 SHA-256 后才原子发布。
//!
//! 来源统一为 ModelScope `RapidAI/RapidOCR` 发布标签 `v3.10.0` 的 PP-OCRv5 ONNX。全部 15 个
//! 文件的大小与 SHA-256 先取自 ModelScope 文件接口，2026-11-15 实际下载后逐个复算一致
//! （见 docs/research/2026-11-15-companion-implementation.md）。识别模型的字符表都写在 ONNX
//! 元数据 `character` 里，前后处理与输入尺寸（3×48×W）共用 `ppocr.rs`；各包的语种以实测为准。
//! 检测模型所有语种共用。PaddlePaddle 官方的 Paddle 原生格式（`.pdiparams`）与尚未进入发布
//! 标签的 PP-OCRv6 不在清单内。

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
    /// `ocr.recognize` 接受的语言代码（实测覆盖的语种）。
    pub languages: &'static [&'static str],
    /// 不传 language 时使用的语言代码。
    pub default_language: &'static str,
    /// 从右往左书写（阿拉伯字母）：识别结果需按 PaddleOCR 的规则倒序还原。
    pub right_to_left: bool,
    pub license: &'static str,
    pub license_url: &'static str,
    /// 上游项目与转换来源。
    pub upstream: &'static str,
    pub distribution: &'static str,
    pub files: &'static [ModelFile],
}

pub const KOREAN_MOBILE_ID: &str = "ppocrv5-korean-mobile";

const LICENSE: &str = "Apache-2.0";
const LICENSE_URL: &str = "https://www.apache.org/licenses/LICENSE-2.0";
const UPSTREAM: &str = "PaddleOCR PP-OCRv5 (PaddlePaddle, Apache-2.0); ONNX conversion by RapidAI/RapidOCR (Apache-2.0)";
const DISTRIBUTION: &str = "https://www.modelscope.cn/models/RapidAI/RapidOCR (tag v3.10.0)";

const DET_MOBILE: ModelFile = ModelFile {
    name: "ch_PP-OCRv5_det_mobile.onnx",
    role: "det",
    url: "https://www.modelscope.cn/models/RapidAI/RapidOCR/resolve/v3.10.0/onnx/PP-OCRv5/det/ch_PP-OCRv5_det_mobile.onnx",
    sha256: "4d97c44a20d30a81aad087d6a396b08f786c4635742afc391f6621f5c6ae78ae",
    bytes: 4_819_576,
};
const DET_SERVER: ModelFile = ModelFile {
    name: "ch_PP-OCRv5_det_server.onnx",
    role: "det",
    url: "https://www.modelscope.cn/models/RapidAI/RapidOCR/resolve/v3.10.0/onnx/PP-OCRv5/det/ch_PP-OCRv5_det_server.onnx",
    sha256: "0f8846b1d4bba223a2a2f9d9b44022fbc22cc019051a602b41a7fda9667e4cad",
    bytes: 88_118_768,
};

pub const CH_MOBILE: ModelPackage = ModelPackage {
    id: "ppocrv5-ch-mobile",
    backend_id: "local-ppocrv5-ch",
    display_name: "PP-OCRv5 mobile 中英日（本地，CPU）",
    description: "简体 / 繁体中文、英文、日文聊天；国服与多数英文客户端选它",
    recommended: true,
    version: "PP-OCRv5 — det ch_PP-OCRv5_det_mobile + rec ch_PP-OCRv5_rec_mobile; ONNX export from RapidOCR v3.10.0",
    languages: &["zh", "zh-Hant", "ja", "en"],
    default_language: "zh",
    right_to_left: false,
    license: LICENSE,
    license_url: LICENSE_URL,
    upstream: UPSTREAM,
    distribution: DISTRIBUTION,
    files: &[DET_MOBILE, ModelFile {
            name: "ch_PP-OCRv5_rec_mobile.onnx",
            role: "rec",
            url: "https://www.modelscope.cn/models/RapidAI/RapidOCR/resolve/v3.10.0/onnx/PP-OCRv5/rec/ch_PP-OCRv5_rec_mobile.onnx",
            sha256: "5825fc7ebf84ae7a412be049820b4d86d77620f204a041697b0494669b1742c5",
            bytes: 16_631_306,
        }],
};

pub const CH_SERVER: ModelPackage = ModelPackage {
    id: "ppocrv5-ch-server",
    backend_id: "local-ppocrv5-ch-server",
    display_name: "PP-OCRv5 server 中英日（本地，高精度）",
    description: "同上语种，检测与识别都用 server 模型，更准但约 173 MB、CPU 上更慢",
    recommended: false,
    version: "PP-OCRv5 — det ch_PP-OCRv5_det_server + rec ch_PP-OCRv5_rec_server; ONNX export from RapidOCR v3.10.0",
    languages: &["zh", "zh-Hant", "ja", "en"],
    default_language: "zh",
    right_to_left: false,
    license: LICENSE,
    license_url: LICENSE_URL,
    upstream: UPSTREAM,
    distribution: DISTRIBUTION,
    files: &[DET_SERVER, ModelFile {
            name: "ch_PP-OCRv5_rec_server.onnx",
            role: "rec",
            url: "https://www.modelscope.cn/models/RapidAI/RapidOCR/resolve/v3.10.0/onnx/PP-OCRv5/rec/ch_PP-OCRv5_rec_server.onnx",
            sha256: "e09385400eaaaef34ceff54aeb7c4f0f1fe014c27fa8b9905d4709b65746562a",
            bytes: 84_577_022,
        }],
};

pub const KOREAN_MOBILE: ModelPackage = ModelPackage {
    id: KOREAN_MOBILE_ID,
    backend_id: "local-ppocrv5-korean",
    display_name: "PP-OCRv5 mobile Korean (local, CPU)",
    description: "韩文聊天（韩服）。本机 CPU 推理，不上传截图",
    recommended: false,
    version: "PP-OCRv5 — det ch_PP-OCRv5_det_mobile + rec korean_PP-OCRv5_rec_mobile; ONNX export from RapidOCR v3.10.0",
    languages: &["ko"],
    default_language: "ko",
    right_to_left: false,
    license: LICENSE,
    license_url: LICENSE_URL,
    upstream: UPSTREAM,
    distribution: DISTRIBUTION,
    files: &[DET_MOBILE, ModelFile {
            name: "korean_PP-OCRv5_rec_mobile.onnx",
            role: "rec",
            url: "https://www.modelscope.cn/models/RapidAI/RapidOCR/resolve/v3.10.0/onnx/PP-OCRv5/rec/korean_PP-OCRv5_rec_mobile.onnx",
            sha256: "cd6e2ea50f6943ca7271eb8c56a877a5a90720b7047fe9c41a2e541a25773c9b",
            bytes: 13_488_748,
        }],
};

pub const EN_MOBILE: ModelPackage = ModelPackage {
    id: "ppocrv5-en-mobile",
    backend_id: "local-ppocrv5-en",
    display_name: "PP-OCRv5 mobile English（本地）",
    description: "只认英文字母、数字与符号，比中英日包更小",
    recommended: false,
    version: "PP-OCRv5 — det ch_PP-OCRv5_det_mobile + rec en_PP-OCRv5_rec_mobile; ONNX export from RapidOCR v3.10.0",
    languages: &["en"],
    default_language: "en",
    right_to_left: false,
    license: LICENSE,
    license_url: LICENSE_URL,
    upstream: UPSTREAM,
    distribution: DISTRIBUTION,
    files: &[DET_MOBILE, ModelFile {
            name: "en_PP-OCRv5_rec_mobile.onnx",
            role: "rec",
            url: "https://www.modelscope.cn/models/RapidAI/RapidOCR/resolve/v3.10.0/onnx/PP-OCRv5/rec/en_PP-OCRv5_rec_mobile.onnx",
            sha256: "c3461add59bb4323ecba96a492ab75e06dda42467c9e3d0c18db5d1d21924be8",
            bytes: 7_872_351,
        }],
};

pub const LATIN_MOBILE: ModelPackage = ModelPackage {
    id: "ppocrv5-latin-mobile",
    backend_id: "local-ppocrv5-latin",
    display_name: "PP-OCRv5 mobile 拉丁字母（本地）",
    description: "法、德、西等拉丁字母语言（越南文实测识别不好，不在支持范围）",
    recommended: false,
    version: "PP-OCRv5 — det ch_PP-OCRv5_det_mobile + rec latin_PP-OCRv5_rec_mobile; ONNX export from RapidOCR v3.10.0",
    languages: &["en", "fr", "de", "es"],
    default_language: "en",
    right_to_left: false,
    license: LICENSE,
    license_url: LICENSE_URL,
    upstream: UPSTREAM,
    distribution: DISTRIBUTION,
    files: &[DET_MOBILE, ModelFile {
            name: "latin_PP-OCRv5_rec_mobile.onnx",
            role: "rec",
            url: "https://www.modelscope.cn/models/RapidAI/RapidOCR/resolve/v3.10.0/onnx/PP-OCRv5/rec/latin_PP-OCRv5_rec_mobile.onnx",
            sha256: "b20bd37c168a570f583afbc8cd7925603890efbcdc000a59e22c269d160b5f5a",
            bytes: 7_904_513,
        }],
};

pub const CYRILLIC_MOBILE: ModelPackage = ModelPackage {
    id: "ppocrv5-cyrillic-mobile",
    backend_id: "local-ppocrv5-cyrillic",
    display_name: "PP-OCRv5 mobile 西里尔字母（本地）",
    description: "俄文、乌克兰文等西里尔字母语言",
    recommended: false,
    version: "PP-OCRv5 — det ch_PP-OCRv5_det_mobile + rec cyrillic_PP-OCRv5_rec_mobile; ONNX export from RapidOCR v3.10.0",
    languages: &["ru", "uk"],
    default_language: "ru",
    right_to_left: false,
    license: LICENSE,
    license_url: LICENSE_URL,
    upstream: UPSTREAM,
    distribution: DISTRIBUTION,
    files: &[DET_MOBILE, ModelFile {
            name: "cyrillic_PP-OCRv5_rec_mobile.onnx",
            role: "rec",
            url: "https://www.modelscope.cn/models/RapidAI/RapidOCR/resolve/v3.10.0/onnx/PP-OCRv5/rec/cyrillic_PP-OCRv5_rec_mobile.onnx",
            sha256: "90f761b4bfcce0c8c561c0cb5c887b0971d3ec01c32164bdf7374a35b0982711",
            bytes: 8_074_092,
        }],
};

pub const ESLAV_MOBILE: ModelPackage = ModelPackage {
    id: "ppocrv5-eslav-mobile",
    backend_id: "local-ppocrv5-eslav",
    display_name: "PP-OCRv5 mobile 东斯拉夫（本地）",
    description: "俄文、乌克兰文（东斯拉夫语言）",
    recommended: false,
    version: "PP-OCRv5 — det ch_PP-OCRv5_det_mobile + rec eslav_PP-OCRv5_rec_mobile; ONNX export from RapidOCR v3.10.0",
    languages: &["ru", "uk"],
    default_language: "ru",
    right_to_left: false,
    license: LICENSE,
    license_url: LICENSE_URL,
    upstream: UPSTREAM,
    distribution: DISTRIBUTION,
    files: &[DET_MOBILE, ModelFile {
            name: "eslav_PP-OCRv5_rec_mobile.onnx",
            role: "rec",
            url: "https://www.modelscope.cn/models/RapidAI/RapidOCR/resolve/v3.10.0/onnx/PP-OCRv5/rec/eslav_PP-OCRv5_rec_mobile.onnx",
            sha256: "08705d6721849b1347d26187f15a5e362c431963a2a62bfff4feac578c489aab",
            bytes: 7_911_802,
        }],
};

pub const TH_MOBILE: ModelPackage = ModelPackage {
    id: "ppocrv5-th-mobile",
    backend_id: "local-ppocrv5-th",
    display_name: "PP-OCRv5 mobile 泰文（本地）",
    description: "泰文聊天",
    recommended: false,
    version: "PP-OCRv5 — det ch_PP-OCRv5_det_mobile + rec th_PP-OCRv5_rec_mobile; ONNX export from RapidOCR v3.10.0",
    languages: &["th"],
    default_language: "th",
    right_to_left: false,
    license: LICENSE,
    license_url: LICENSE_URL,
    upstream: UPSTREAM,
    distribution: DISTRIBUTION,
    files: &[DET_MOBILE, ModelFile {
            name: "th_PP-OCRv5_rec_mobile.onnx",
            role: "rec",
            url: "https://www.modelscope.cn/models/RapidAI/RapidOCR/resolve/v3.10.0/onnx/PP-OCRv5/rec/th_PP-OCRv5_rec_mobile.onnx",
            sha256: "de541dd83161c241ff426f7ecfd602a0ba77d686cf3ab9a6c255ea82fd08006e",
            bytes: 7_915_294,
        }],
};

pub const EL_MOBILE: ModelPackage = ModelPackage {
    id: "ppocrv5-el-mobile",
    backend_id: "local-ppocrv5-el",
    display_name: "PP-OCRv5 mobile 希腊文（本地）",
    description: "希腊文聊天",
    recommended: false,
    version: "PP-OCRv5 — det ch_PP-OCRv5_det_mobile + rec el_PP-OCRv5_rec_mobile; ONNX export from RapidOCR v3.10.0",
    languages: &["el"],
    default_language: "el",
    right_to_left: false,
    license: LICENSE,
    license_url: LICENSE_URL,
    upstream: UPSTREAM,
    distribution: DISTRIBUTION,
    files: &[DET_MOBILE, ModelFile {
            name: "el_PP-OCRv5_rec_mobile.onnx",
            role: "rec",
            url: "https://www.modelscope.cn/models/RapidAI/RapidOCR/resolve/v3.10.0/onnx/PP-OCRv5/rec/el_PP-OCRv5_rec_mobile.onnx",
            sha256: "b4368bccd557123c702b7549fee6cd1e94b581337d1c9b65310f109131542b7f",
            bytes: 7_832_350,
        }],
};

pub const ARABIC_MOBILE: ModelPackage = ModelPackage {
    id: "ppocrv5-arabic-mobile",
    backend_id: "local-ppocrv5-arabic",
    display_name: "PP-OCRv5 mobile 阿拉伯字母（本地）",
    description: "阿拉伯文（按从右往左的阅读顺序还原；一个词一个框）",
    recommended: false,
    version: "PP-OCRv5 — det ch_PP-OCRv5_det_mobile + rec arabic_PP-OCRv5_rec_mobile; ONNX export from RapidOCR v3.10.0",
    languages: &["ar"],
    default_language: "ar",
    right_to_left: true,
    license: LICENSE,
    license_url: LICENSE_URL,
    upstream: UPSTREAM,
    distribution: DISTRIBUTION,
    files: &[DET_MOBILE, ModelFile {
            name: "arabic_PP-OCRv5_rec_mobile.onnx",
            role: "rec",
            url: "https://www.modelscope.cn/models/RapidAI/RapidOCR/resolve/v3.10.0/onnx/PP-OCRv5/rec/arabic_PP-OCRv5_rec_mobile.onnx",
            sha256: "c1192e632d0baa9146ae5b756a0e635e3dc63c1733737ebfd1629e87144e9295",
            bytes: 8_023_828,
        }],
};

pub const DEVANAGARI_MOBILE: ModelPackage = ModelPackage {
    id: "ppocrv5-devanagari-mobile",
    backend_id: "local-ppocrv5-devanagari",
    display_name: "PP-OCRv5 mobile 天城文（本地）",
    description: "印地文等天城文语言",
    recommended: false,
    version: "PP-OCRv5 — det ch_PP-OCRv5_det_mobile + rec devanagari_PP-OCRv5_rec_mobile; ONNX export from RapidOCR v3.10.0",
    languages: &["hi"],
    default_language: "hi",
    right_to_left: false,
    license: LICENSE,
    license_url: LICENSE_URL,
    upstream: UPSTREAM,
    distribution: DISTRIBUTION,
    files: &[DET_MOBILE, ModelFile {
            name: "devanagari_PP-OCRv5_rec_mobile.onnx",
            role: "rec",
            url: "https://www.modelscope.cn/models/RapidAI/RapidOCR/resolve/v3.10.0/onnx/PP-OCRv5/rec/devanagari_PP-OCRv5_rec_mobile.onnx",
            sha256: "d6f0a906580e3fa6b324a318718f1f31f268b6ea8ef985f91c2012a37f52c91e",
            bytes: 7_940_361,
        }],
};

pub const TA_MOBILE: ModelPackage = ModelPackage {
    id: "ppocrv5-ta-mobile",
    backend_id: "local-ppocrv5-ta",
    display_name: "PP-OCRv5 mobile 泰米尔文（本地）",
    description: "泰米尔文聊天",
    recommended: false,
    version: "PP-OCRv5 — det ch_PP-OCRv5_det_mobile + rec ta_PP-OCRv5_rec_mobile; ONNX export from RapidOCR v3.10.0",
    languages: &["ta"],
    default_language: "ta",
    right_to_left: false,
    license: LICENSE,
    license_url: LICENSE_URL,
    upstream: UPSTREAM,
    distribution: DISTRIBUTION,
    files: &[DET_MOBILE, ModelFile {
            name: "ta_PP-OCRv5_rec_mobile.onnx",
            role: "rec",
            url: "https://www.modelscope.cn/models/RapidAI/RapidOCR/resolve/v3.10.0/onnx/PP-OCRv5/rec/ta_PP-OCRv5_rec_mobile.onnx",
            sha256: "a42448808b7dea87597336f12438935f40353f1949e8360acd9e06b4da21bfe1",
            bytes: 7_909_926,
        }],
};

pub const TE_MOBILE: ModelPackage = ModelPackage {
    id: "ppocrv5-te-mobile",
    backend_id: "local-ppocrv5-te",
    display_name: "PP-OCRv5 mobile 泰卢固文（本地）",
    description: "泰卢固文聊天",
    recommended: false,
    version: "PP-OCRv5 — det ch_PP-OCRv5_det_mobile + rec te_PP-OCRv5_rec_mobile; ONNX export from RapidOCR v3.10.0",
    languages: &["te"],
    default_language: "te",
    right_to_left: false,
    license: LICENSE,
    license_url: LICENSE_URL,
    upstream: UPSTREAM,
    distribution: DISTRIBUTION,
    files: &[DET_MOBILE, ModelFile {
            name: "te_PP-OCRv5_rec_mobile.onnx",
            role: "rec",
            url: "https://www.modelscope.cn/models/RapidAI/RapidOCR/resolve/v3.10.0/onnx/PP-OCRv5/rec/te_PP-OCRv5_rec_mobile.onnx",
            sha256: "a3690451b50028a09a3316a1274f7c05728151ea3f8fd392696397a7fefcbd92",
            bytes: 7_923_102,
        }],
};

/// 统一模型清单里的本地 OCR 包（按推荐顺序；未选择 OCR 模型时取第一个已安装的）。
pub const PACKAGES: &[ModelPackage] = &[
    CH_MOBILE,
    CH_SERVER,
    KOREAN_MOBILE,
    EN_MOBILE,
    LATIN_MOBILE,
    CYRILLIC_MOBILE,
    ESLAV_MOBILE,
    TH_MOBILE,
    EL_MOBILE,
    ARABIC_MOBILE,
    DEVANAGARI_MOBILE,
    TA_MOBILE,
    TE_MOBILE,
];

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
        assert!(p.file("det").is_some() && p.file("rec").is_some());
        assert_eq!(p.total_bytes(), 18_308_324);
        let mut ids = std::collections::HashSet::new();
        let mut backends = std::collections::HashSet::new();
        for p in PACKAGES {
            assert!(ids.insert(p.id), "duplicate id {}", p.id);
            assert!(
                backends.insert(p.backend_id),
                "duplicate backend {}",
                p.backend_id
            );
            assert!(p.languages.contains(&p.default_language), "{}", p.id);
            assert_eq!(p.files.len(), 2);
            for f in p.files {
                assert_eq!(f.sha256.len(), 64);
                assert!(f.sha256.bytes().all(|b| b.is_ascii_hexdigit()));
                assert!(f.url.starts_with(
                    "https://www.modelscope.cn/models/RapidAI/RapidOCR/resolve/v3.10.0/onnx/PP-OCRv5/"
                ));
                assert!(f.url.ends_with(f.name));
                assert!(f.bytes > 1_000_000);
            }
        }
        assert_eq!(PACKAGES.len(), 13);
        assert_eq!(PACKAGES.iter().filter(|p| p.recommended).count(), 1);
        assert_eq!(KOREAN_MOBILE.backend_id, "local-ppocrv5-korean");
    }
}
