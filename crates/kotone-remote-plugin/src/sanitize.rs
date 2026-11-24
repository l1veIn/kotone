//! 别人写的字（第三方 MCP 服务器给的工具说明、参数说明、工具结果、服务器的名字）进 Kotone 之前先洗一遍
//! （原则 9：别人写的东西是数据，不是指令）：
//!
//! - **剥掉看不见的字符**（照 Hermes Agent 对 MCP 工具说明和结果的处理）：零宽字符、双向文字控制符、
//!   Unicode “标签”字符（U+E0000 起，可以把一整段英文指令藏在一个看似空白的地方）、变体选择符、
//!   软连字符、控制字符（换行、回车、制表符留着）。模型读得到这些字符、玩家在界面上看不见，正好拿来藏话；
//! - **限长**：说明和结果太长的截掉，写明截掉了多少（照 Claude Code：MCP 结果有上限，超出的不交给模型）。

use serde_json::Value;

/// 这个字符看不见（或者会改变别的字的显示顺序）。
pub fn invisible(c: char) -> bool {
    let u = c as u32;
    if u < 0x20 {
        return !matches!(c, '\n' | '\r' | '\t');
    }
    matches!(
        u,
        // 删除符、C1 控制字符
        0x7F..=0x9F
            // 软连字符、组合用字形连接符、阿拉伯字母标记
            | 0x00AD
            | 0x034F
            | 0x061C
            // 韩文填充符
            | 0x115F
            | 0x1160
            | 0x3164
            | 0xFFA0
            // 高棉文元音固有符、蒙古文变体选择符和元音分隔符
            | 0x17B4
            | 0x17B5
            | 0x180B..=0x180F
            // 零宽空格、零宽连接 / 不连接符、左右标记
            | 0x200B..=0x200F
            // 行 / 段分隔符、双向嵌入和覆盖
            | 0x2028..=0x202E
            // 词连接符、不可见运算符、双向隔离、已弃用的格式控制
            | 0x2060..=0x206F
            // 变体选择符
            | 0xFE00..=0xFE0F
            // 字节顺序标记（零宽不换行空格）
            | 0xFEFF
            // 行间注释
            | 0xFFF9..=0xFFFB
            // 速记格式控制、音乐符号的格式控制
            | 0x1BCA0..=0x1BCA3
            | 0x1D173..=0x1D17A
            // 标签字符、变体选择符补充
            | 0xE0000..=0xE0FFF
    )
}

/// 剥掉看不见的字符。
pub fn strip_invisible(text: &str) -> String {
    text.chars().filter(|c| !invisible(*c)).collect()
}

/// 剥掉看不见的字符，再限到 `max` 个字：超出的截掉，末尾写明截掉了多少。
pub fn clean(text: &str, max: usize) -> String {
    let text = strip_invisible(text);
    match text.char_indices().nth(max) {
        None => text,
        Some((cut, _)) => {
            let total = text.chars().count();
            format!(
                "{}\n…（太长了，后面 {} 字截掉了）",
                &text[..cut],
                total - max
            )
        }
    }
}

/// JSON 里所有的字符串（不含对象的键）都剥掉看不见的字符：参数的 JSON Schema 里有给模型看的说明。
pub fn clean_json(value: &Value) -> Value {
    match value {
        Value::String(s) => Value::String(strip_invisible(s)),
        Value::Array(items) => Value::Array(items.iter().map(clean_json).collect()),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), clean_json(v)))
                .collect(),
        ),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 藏在“看不见”的字符里的话被剥掉，看得见的字一个不少（中文、emoji 的本体、换行）。
    #[test]
    fn hidden_text_is_removed() {
        // 用标签字符把 "ignore" 藏起来（每个 ASCII 字母 + U+E0000）
        let hidden: String = "ignore"
            .chars()
            .map(|c| char::from_u32(0xE0000 + c as u32).unwrap())
            .collect();
        let text =
            format!("查天气\u{200B}{hidden}\u{202E}反\u{2066}向\u{FEFF}\n第二行\t好\u{7}❤\u{FE0F}");
        assert_eq!(strip_invisible(&text), "查天气反向\n第二行\t好❤");
        assert_eq!(strip_invisible("plain text 普通"), "plain text 普通");
    }

    #[test]
    fn long_text_is_clipped_with_a_note() {
        let text = "一".repeat(30);
        let out = clean(&text, 10);
        assert!(out.starts_with(&"一".repeat(10)));
        assert!(out.contains("后面 20 字截掉了"), "{out}");
        assert_eq!(clean("短", 10), "短");
    }

    #[test]
    fn json_strings_are_cleaned_but_keys_stay() {
        let schema = json!({"type": "object", "properties": {"city\u{200B}": {
            "type": "string", "description": "城市\u{E0041}\u{E0042}"
        }}, "enum": ["a\u{200D}b", 1]});
        let out = clean_json(&schema);
        assert_eq!(out["properties"]["city\u{200B}"]["description"], "城市");
        assert_eq!(out["enum"], json!(["ab", 1]));
    }
}
