//! 本地 CSV 驱动的屏蔽词过滤器。
//!
//! CSV 第一列是屏蔽词，第二列是可选替换词；第二列为空时按屏蔽词字符数生成等长 `*`。
//! 匹配区分大小写，较长词优先，并且替换结果不会再次触发后续规则。

use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use encoding_rs::{GB18030, UTF_16BE, UTF_16LE};
use kotone_core::postprocess::{
    NetworkAccess, ProcessError, ProcessFuture, ProcessingCancelToken, ProcessingContext,
    ProcessorCategory, ProcessorConfigField, ProcessorConfigFieldKind, ProcessorDescriptor,
    ProcessorFactory, TextDocument, TextProcessor,
};
use serde_json::Value;

const MAX_CSV_BYTES: u64 = 1024 * 1024;
const MAX_RULES: usize = 10_000;
const MAX_PATTERN_CHARS: usize = 256;
const DEFAULT_RULES_CSV: &str = include_str!("../assets/lol-zh-cn-starter.csv");

pub const PROCESSOR_ID: &str = "builtin.blocklist-filter";

/// 内置屏蔽词表 CSV 原文（含 BOM、表头与规则行），供 UI 导出与预览。
pub fn builtin_csv() -> &'static str {
    DEFAULT_RULES_CSV
}

/// 内置屏蔽词表规则条数（去重后的有效规则数，不含表头/空行）。
pub fn builtin_rule_count() -> usize {
    parse_rules(DEFAULT_RULES_CSV)
        .map(|rules| rules.len())
        .unwrap_or(0)
}

pub struct BlocklistFilterFactory;

impl ProcessorFactory for BlocklistFilterFactory {
    fn descriptor(&self) -> ProcessorDescriptor {
        ProcessorDescriptor {
            id: PROCESSOR_ID.into(),
            display_name: "屏蔽词过滤".into(),
            description: "用词库把脏话换成能发的词。自带一份默认词库，也可以换成自己的。".into(),
            category: ProcessorCategory::Utility,
            developer_only: false,
            network_access: NetworkAccess::Local,
            config_fields: vec![ProcessorConfigField {
                key: "csvPath".into(),
                display_name: "自己的词库".into(),
                description: "不选就用默认词库。选了文件就改用你的。"
                    .into(),
                kind: ProcessorConfigFieldKind::File,
                required: false,
                file_extensions: vec!["csv".into()],
                placeholder: String::new(),
                presets: Vec::new(),
                compatible_providers: Vec::new(),
            }],
        }
    }

    fn create(&self, config: &Value) -> Result<Arc<dyn TextProcessor>, String> {
        let path = config
            .get("csvPath")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|path| !path.is_empty());
        let rules = match path {
            Some(path) => load_rules(Path::new(path))?,
            None => parse_rules(DEFAULT_RULES_CSV)
                .map_err(|error| format!("内置屏蔽词表无效：{error}"))?,
        };
        Ok(Arc::new(BlocklistFilter { rules }))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BlockRule {
    pattern: String,
    replacement: String,
}

struct BlocklistFilter {
    rules: Vec<BlockRule>,
}

impl BlocklistFilter {
    fn replace(&self, text: &str, cancel: &ProcessingCancelToken) -> Result<String, ProcessError> {
        let mut output = String::with_capacity(text.len());
        let mut cursor = 0;

        while cursor < text.len() {
            if cancel.is_cancelled() {
                return Err(ProcessError::failed("后处理已取消"));
            }
            let rest = &text[cursor..];
            if let Some(rule) = self
                .rules
                .iter()
                .find(|rule| rest.starts_with(&rule.pattern))
            {
                output.push_str(&rule.replacement);
                cursor += rule.pattern.len();
            } else {
                let ch = rest.chars().next().expect("cursor 始终位于 UTF-8 字符边界");
                output.push(ch);
                cursor += ch.len_utf8();
            }
        }
        Ok(output)
    }
}

impl TextProcessor for BlocklistFilter {
    fn process<'a>(
        &'a self,
        mut input: TextDocument,
        _context: &'a ProcessingContext,
        cancel: ProcessingCancelToken,
    ) -> ProcessFuture<'a> {
        Box::pin(async move {
            input.text = self.replace(&input.text, &cancel)?;
            Ok(input)
        })
    }
}

fn load_rules(path: &Path) -> Result<Vec<BlockRule>, String> {
    let bytes = fs::read(path)
        .map_err(|error| format!("无法读取屏蔽词 CSV「{}」：{error}", path.display()))?;
    if bytes.len() as u64 > MAX_CSV_BYTES {
        return Err(format!(
            "屏蔽词 CSV 不能超过 {} MB",
            MAX_CSV_BYTES / 1024 / 1024
        ));
    }
    let text = decode_csv(&bytes)
        .map_err(|error| format!("无法读取屏蔽词 CSV「{}」：{error}", path.display()))?;
    parse_rules(&text)
}

/// 把自定义 CSV 的原始字节解码为 UTF-8 字符串，兼容常见编码。
///
/// 检测顺序：UTF-8 BOM → UTF-16 LE/BE BOM → 严格 UTF-8 → GB18030（GBK 超集，
/// 覆盖 Windows 中文 ANSI）。全部失败时给出明确的另存指引，而不是抛出底层 UTF-8 错误。
fn decode_csv(bytes: &[u8]) -> Result<String, String> {
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return strict_utf8(rest);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return decode_utf16(rest, UTF_16LE);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return decode_utf16(rest, UTF_16BE);
    }
    if let Ok(text) = std::str::from_utf8(bytes) {
        return Ok(text.to_string());
    }
    let (text, _, had_errors) = GB18030.decode(bytes);
    if had_errors {
        return Err(
            "文件编码无法识别（既不是 UTF-8，也不是 GBK/GB18030）。请用「记事本 → 另存为 → UTF-8」，或 WPS「另存为 CSV(UTF-8)」后重试。"
                .into(),
        );
    }
    Ok(text.into_owned())
}

fn strict_utf8(bytes: &[u8]) -> Result<String, String> {
    std::str::from_utf8(bytes)
        .map(str::to_string)
        .map_err(|_| "文件声明为 UTF-8(BOM) 但内容不是有效 UTF-8。".into())
}

fn decode_utf16(bytes: &[u8], encoding: &'static encoding_rs::Encoding) -> Result<String, String> {
    let (text, _, had_errors) = encoding.decode(bytes);
    if had_errors {
        return Err("文件声明为 UTF-16 但内容无效。请另存为 UTF-8 后重试。".into());
    }
    Ok(text.into_owned())
}

fn parse_rules(text: &str) -> Result<Vec<BlockRule>, String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut rules = Vec::new();
    let mut seen = HashSet::new();
    let mut first_record = true;

    for (index, raw_line) in text.lines().enumerate() {
        let line_number = index + 1;
        if raw_line.trim().is_empty() {
            continue;
        }
        let fields = parse_row(raw_line, line_number)?;
        if fields.len() > 2 {
            return Err(format!("屏蔽词 CSV 第 {line_number} 行超过两列"));
        }
        let pattern = fields.first().map(|value| value.trim()).unwrap_or_default();
        let replacement = fields.get(1).map(|value| value.trim()).unwrap_or_default();

        if first_record && pattern == "屏蔽词" && replacement == "替换词" {
            first_record = false;
            continue;
        }
        first_record = false;
        if pattern.is_empty() {
            return Err(format!("屏蔽词 CSV 第 {line_number} 行第一列不能为空"));
        }
        if pattern.chars().count() > MAX_PATTERN_CHARS {
            return Err(format!(
                "屏蔽词 CSV 第 {line_number} 行的屏蔽词不能超过 {MAX_PATTERN_CHARS} 个字符"
            ));
        }
        if !seen.insert(pattern.to_string()) {
            continue;
        }
        if rules.len() >= MAX_RULES {
            return Err(format!("屏蔽词 CSV 最多允许 {MAX_RULES} 条规则"));
        }
        rules.push(BlockRule {
            pattern: pattern.to_string(),
            replacement: if replacement.is_empty() {
                "*".repeat(pattern.chars().count())
            } else {
                replacement.to_string()
            },
        });
    }

    // 长词优先，长度相同时保留 CSV 顺序，避免“坏”抢先命中“坏蛋”。
    rules.sort_by_key(|rule| std::cmp::Reverse(rule.pattern.chars().count()));
    Ok(rules)
}

/// 足够覆盖屏蔽词表的简单 CSV 行解析：支持引号包裹、逗号和双引号转义；不允许跨行字段。
fn parse_row(line: &str, line_number: usize) -> Result<Vec<String>, String> {
    let mut fields = Vec::new();
    let mut field = String::new();
    let mut chars = line.trim_end_matches('\r').chars().peekable();
    let mut quoted = false;
    let mut quote_closed = false;

    while let Some(ch) = chars.next() {
        if quoted {
            if ch == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    field.push('"');
                } else {
                    quoted = false;
                    quote_closed = true;
                }
            } else {
                field.push(ch);
            }
            continue;
        }

        match ch {
            ',' => {
                fields.push(std::mem::take(&mut field));
                quote_closed = false;
            }
            '"' if field.trim().is_empty() && !quote_closed => {
                field.clear();
                quoted = true;
            }
            ch if quote_closed && !ch.is_whitespace() => {
                return Err(format!("屏蔽词 CSV 第 {line_number} 行引号后存在无效字符"));
            }
            ch if !quote_closed => field.push(ch),
            _ => {}
        }
    }
    if quoted {
        return Err(format!("屏蔽词 CSV 第 {line_number} 行的引号未闭合"));
    }
    fields.push(field);
    Ok(fields)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kotone_core::postprocess::{
        PipelineConfig, PipelineStepConfig, PostProcessPipeline, PostProcessingConfig,
        StepFailurePolicy,
    };
    use tempfile::tempdir;

    #[test]
    fn csv_parser_supports_header_quotes_and_empty_replacement() {
        let rules =
            parse_rules("\u{feff}屏蔽词,替换词\n坏蛋,\n菜鸟,萌新\n\"讨厌,鬼\",\"友好\"\n").unwrap();
        assert_eq!(rules.len(), 3);
        assert_eq!(rules[0].pattern, "讨厌,鬼");
        assert_eq!(rules[0].replacement, "友好");
        assert_eq!(rules[1].pattern, "坏蛋");
        assert_eq!(rules[1].replacement, "**");
    }

    #[test]
    fn csv_parser_rejects_invalid_rows() {
        assert!(parse_rules(",替换\n")
            .unwrap_err()
            .contains("第一列不能为空"));
        assert!(parse_rules("\"未闭合,替换\n")
            .unwrap_err()
            .contains("引号未闭合"));
        assert!(parse_rules("a,b,c\n").unwrap_err().contains("超过两列"));
    }

    #[test]
    fn load_rules_decodes_gbk_and_utf16_csv() {
        let dir = tempdir().unwrap();

        // GBK（Windows 中文 ANSI）：你=C4E3，好=BAC3
        let gbk_path = dir.path().join("gbk.csv");
        fs::write(
            &gbk_path,
            [0xC4u8, 0xE3, 0xBA, 0xC3, 0x2C, 0x68, 0x69, 0x0A],
        )
        .unwrap();
        let rules = load_rules(&gbk_path).unwrap();
        assert_eq!(rules[0].pattern, "你好");
        assert_eq!(rules[0].replacement, "hi");

        // UTF-16 LE（带 BOM，Excel/WPS 另存常见）；手动构造字节，
        // 因为 encoding_rs 的 UTF_16LE 编码器按 WHATWG 规范会退化为输出 UTF-8。
        let utf16_path = dir.path().join("utf16.csv");
        let with_bom: Vec<u8> = vec![
            0xFF, 0xFE, // BOM
            0x60, 0x4F, // 你
            0x7D, 0x59, // 好
            0x2C, 0x00, // ,
            0x68, 0x00, // h
            0x69, 0x00, // i
            0x0A, 0x00, // \n
        ];
        fs::write(&utf16_path, &with_bom).unwrap();
        let rules = load_rules(&utf16_path).unwrap();
        assert_eq!(rules[0].pattern, "你好");
        assert_eq!(rules[0].replacement, "hi");
    }

    #[tokio::test]
    async fn processor_masks_by_character_count_and_uses_explicit_replacements() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("blocklist.csv");
        fs::write(
            &path,
            "坏,不友好\n坏蛋,好人\n菜鸟,萌新\nbad,\n不友好,糟糕\n",
        )
        .unwrap();

        let mut registry = kotone_core::postprocess::ProcessorRegistry::new();
        registry.register(Arc::new(BlocklistFilterFactory)).unwrap();
        let pipeline = PostProcessPipeline::compile(
            &PostProcessingConfig::with_pipeline(
                true,
                PipelineConfig {
                    id: "blocklist-test".into(),
                    display_name: "屏蔽词测试".into(),
                    steps: vec![PipelineStepConfig {
                        id: "filter".into(),
                        processor_id: PROCESSOR_ID.into(),
                        enabled: true,
                        config: serde_json::json!({ "csvPath": path }),
                        timeout_ms: 1_000,
                        on_error: StepFailurePolicy::Required,
                    }],
                },
            ),
            &registry,
        )
        .unwrap();

        let result = pipeline
            .run(
                "坏蛋和菜鸟说 bad bad".into(),
                &ProcessingContext::default(),
                ProcessingCancelToken::default(),
            )
            .await
            .unwrap();

        assert_eq!(result.final_text, "好人和萌新说 *** ***");
    }

    #[tokio::test]
    async fn processor_uses_builtin_lol_starter_rules_without_custom_path() {
        let processor = BlocklistFilterFactory
            .create(&serde_json::json!({}))
            .unwrap();
        let result = processor
            .process(
                TextDocument::recognized("你真傻逼，这波牛逼"),
                &ProcessingContext::default(),
                ProcessingCancelToken::default(),
            )
            .await
            .unwrap();

        assert_eq!(result.text, "你真**，这波NB");
    }

    #[test]
    fn descriptor_declares_an_optional_local_csv_override() {
        let descriptor = BlocklistFilterFactory.descriptor();
        assert_eq!(descriptor.id, PROCESSOR_ID);
        assert!(!descriptor.developer_only);
        assert_eq!(descriptor.network_access, NetworkAccess::Local);
        assert_eq!(descriptor.config_fields.len(), 1);
        assert_eq!(descriptor.config_fields[0].key, "csvPath");
        assert!(!descriptor.config_fields[0].required);
        let value = serde_json::to_value(descriptor).unwrap();
        assert_eq!(value["configFields"][0]["kind"], "file");
        assert_eq!(value["configFields"][0]["fileExtensions"][0], "csv");
    }
}
