//! 统一模型清单的共用类型（docs/product-evolution-game-tools.md §11）。
//!
//! ASR / OCR / TTS 的本地模型与在线模型放在同一张清单里，以 `category` 区分。
//! 各类别的清单数据由各自的 crate 提供（`kotone-stt` / `kotone-ocr` / `kotone-tts`），
//! 这里只定义共用的描述结构，不含下载、推理或网络实现。
//!
//! 约定：
//! - 本地模型带下载地址、大小和逐文件 SHA-256；在线模型 `size_bytes = 0`，
//!   通过 `kind = connection` 的配置字段绑定 AI 连接，密钥只在凭据管理器里。
//! - 现有 ASR 条目（`kotone_stt::model::ModelInfo`）保持原有字段，只多一个
//!   `category = "asr"`；不传类别的调用一律按 ASR 处理。

/// 模型类别。
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum ModelCategory {
    /// 语音识别（现有清单）。
    #[default]
    Asr,
    /// 文字识别。
    Ocr,
    /// 语音合成。
    Tts,
}

impl ModelCategory {
    pub const ALL: [ModelCategory; 3] = [Self::Asr, Self::Ocr, Self::Tts];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Asr => "asr",
            Self::Ocr => "ocr",
            Self::Tts => "tts",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "" | "asr" => Some(Self::Asr),
            "ocr" => Some(Self::Ocr),
            "tts" => Some(Self::Tts),
            _ => None,
        }
    }
}

/// 模型自己声明的可配项（语言、连接、声音等）。ASR 清单沿用同一结构。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelConfigOption {
    pub value: String,
    pub label: String,
}

/// 静态配置项（清单常量用），转换为 [`ModelConfigField`] 输出。
pub struct StaticConfigField {
    pub key: &'static str,
    pub label: &'static str,
    pub kind: &'static str,
    pub default: &'static str,
    pub options: &'static [(&'static str, &'static str)],
    pub required: bool,
}

pub fn schema_of(fields: &[StaticConfigField]) -> Vec<ModelConfigField> {
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

/// 部署形态：本机推理或在线接口。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CatalogBackend {
    Local,
    Remote,
}

/// 本地模型的单个文件（逐文件校验 SHA-256）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogFile {
    pub name: String,
    pub url: String,
    pub sha256: String,
    pub size_bytes: u64,
}

/// OCR / TTS 清单条目（ASR 仍用 `kotone_stt::model::ModelInfo`，字段是它的子集加扩展）。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogModel {
    pub category: ModelCategory,
    pub id: String,
    /// 实际执行的后端 ID（OCR：`ocr.backends` 的 id；TTS：引擎 id）。
    pub engine_id: String,
    pub display_name: String,
    /// 一句话说明（适用语种、精度/速度取舍、数据外发等）。
    pub description: String,
    /// 下载总大小；在线模型与系统自带为 0。
    pub size_bytes: u64,
    pub backend: CatalogBackend,
    /// 本地：文件已下载且校验通过；在线 / 系统自带：恒为 true。
    pub downloaded: bool,
    /// 能否立刻使用（本地已安装 / 在线已配置）。
    pub ready: bool,
    /// 机器可读状态：`ready` / `not-installed` / `invalid` / `not-configured` / `unavailable`。
    pub state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// 是否为该类别当前选中的模型。
    pub active: bool,
    /// 默认推荐（设置页高亮）。
    pub recommended: bool,
    pub languages: Vec<String>,
    /// 数据外发：`none` / `selected-image` / `text`。
    pub data_egress: String,
    pub config_schema: Vec<ModelConfigField>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_url: Option<String>,
    pub license: String,
    /// 本地模型的文件清单（手动导入时按这些文件名放置）。
    pub files: Vec<CatalogFile>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn category_parses_and_defaults_to_asr() {
        assert_eq!(ModelCategory::parse(""), Some(ModelCategory::Asr));
        assert_eq!(ModelCategory::parse("ocr"), Some(ModelCategory::Ocr));
        assert_eq!(ModelCategory::parse("tts"), Some(ModelCategory::Tts));
        assert_eq!(ModelCategory::parse("vision"), None);
        assert_eq!(ModelCategory::default(), ModelCategory::Asr);
        assert_eq!(serde_json::to_value(ModelCategory::Tts).unwrap(), "tts");
    }
}
