//! 本地 CSV 驱动的屏蔽词过滤器。
//!
//! CSV 第一列是屏蔽词，第二列是可选替换词；第二列为空时按屏蔽词字符数生成等长 `*`。
//! 匹配区分大小写，较长词优先，并且替换结果不会再次触发后续规则。

use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use kotone_core::postprocess::{
    NetworkAccess, ProcessError, ProcessFuture, ProcessingCancelToken, ProcessingContext,
    ProcessorCategory, ProcessorConfigField, ProcessorConfigFieldKind, ProcessorDescriptor,
    ProcessorFactory, TextDocument, TextProcessor,
};
use serde_json::Value;

const MAX_CSV_BYTES: u64 = 1024 * 1024;
const MAX_RULES: usize = 10_000;
const MAX_PATTERN_CHARS: usize = 256;

pub const PROCESSOR_ID: &str = "builtin.blocklist-filter";

pub struct BlocklistFilterFactory;

impl ProcessorFactory for BlocklistFilterFactory {
    fn descriptor(&self) -> ProcessorDescriptor {
        ProcessorDescriptor {
            id: PROCESSOR_ID.into(),
            display_name: "屏蔽词过滤".into(),
            description: "按本地 CSV 表替换屏蔽词；替换词留空时使用等长星号。".into(),
            category: ProcessorCategory::Utility,
            developer_only: false,
            network_access: NetworkAccess::Local,
            config_fields: vec![ProcessorConfigField {
                key: "csvPath".into(),
                display_name: "屏蔽词 CSV".into(),
                description:
                    "UTF-8 CSV，每行“屏蔽词,替换词”；第二列可留空，例如：坏蛋, 或 菜鸟,萌新。"
                        .into(),
                kind: ProcessorConfigFieldKind::File,
                required: true,
                file_extensions: vec!["csv".into()],
            }],
        }
    }

    fn create(&self, config: &Value) -> Result<Arc<dyn TextProcessor>, String> {
        let path = config
            .get("csvPath")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .ok_or_else(|| "屏蔽词过滤器尚未配置 CSV 文件".to_string())?;
        let rules = load_rules(Path::new(path))?;
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
    let metadata = fs::metadata(path)
        .map_err(|error| format!("无法读取屏蔽词 CSV「{}」：{error}", path.display()))?;
    if metadata.len() > MAX_CSV_BYTES {
        return Err(format!(
            "屏蔽词 CSV 不能超过 {} MB",
            MAX_CSV_BYTES / 1024 / 1024
        ));
    }
    let text = fs::read_to_string(path)
        .map_err(|error| format!("无法读取屏蔽词 CSV「{}」：{error}", path.display()))?;
    if text.len() as u64 > MAX_CSV_BYTES {
        return Err(format!(
            "屏蔽词 CSV 不能超过 {} MB",
            MAX_CSV_BYTES / 1024 / 1024
        ));
    }
    parse_rules(text.strip_prefix('\u{feff}').unwrap_or(&text))
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
            &PostProcessingConfig {
                enabled: true,
                pipeline: PipelineConfig {
                    id: "blocklist-test".into(),
                    steps: vec![PipelineStepConfig {
                        id: "filter".into(),
                        processor_id: PROCESSOR_ID.into(),
                        enabled: true,
                        config: serde_json::json!({ "csvPath": path }),
                        timeout_ms: 1_000,
                        on_error: StepFailurePolicy::Required,
                    }],
                },
            },
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

    #[test]
    fn descriptor_declares_a_required_local_csv_field() {
        let descriptor = BlocklistFilterFactory.descriptor();
        assert_eq!(descriptor.id, PROCESSOR_ID);
        assert!(!descriptor.developer_only);
        assert_eq!(descriptor.network_access, NetworkAccess::Local);
        assert_eq!(descriptor.config_fields.len(), 1);
        assert_eq!(descriptor.config_fields[0].key, "csvPath");
        assert!(descriptor.config_fields[0].required);
        let value = serde_json::to_value(descriptor).unwrap();
        assert_eq!(value["configFields"][0]["kind"], "file");
        assert_eq!(value["configFields"][0]["fileExtensions"][0], "csv");
    }
}
