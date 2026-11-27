//! 只改 JSON 文本里某个对象的几个键，别的字节一个不动（“一键填好 Webhook”，ADR 047 第 3 期）。
//!
//! BetterGI 的 `config.json` 有几百个键；整份解析再写回会把键的顺序、缩进全换掉（serde_json 不保序），所以这里只在
//! 原文上把那几个值换掉（没有的键插在这个对象最后）。改完再整份解析一遍核对：除了这几个键，和原来的完全一样，
//! 不一样就不写。

use serde_json::Value;

struct Scanner<'a> {
    s: &'a [u8],
    i: usize,
}

type Span = (usize, usize);

impl<'a> Scanner<'a> {
    fn ws(&mut self) {
        while self.i < self.s.len() && matches!(self.s[self.i], b' ' | b'\t' | b'\r' | b'\n') {
            self.i += 1;
        }
    }

    fn expect(&mut self, c: u8) -> Result<(), String> {
        self.ws();
        if self.s.get(self.i) == Some(&c) {
            self.i += 1;
            Ok(())
        } else {
            Err(format!("第 {} 个字节应该是 {}", self.i, c as char))
        }
    }

    fn string(&mut self) -> Result<Span, String> {
        self.ws();
        let start = self.i;
        if self.s.get(self.i) != Some(&b'"') {
            return Err(format!("第 {start} 个字节应该是字符串"));
        }
        self.i += 1;
        while let Some(&c) = self.s.get(self.i) {
            self.i += 1;
            match c {
                b'\\' => self.i += 1,
                b'"' => return Ok((start, self.i)),
                _ => {}
            }
        }
        Err("字符串没有结尾".into())
    }

    /// 跳过一个值，返回它的范围。
    fn value(&mut self) -> Result<Span, String> {
        self.ws();
        let start = self.i;
        match self.s.get(self.i) {
            Some(b'"') => {
                self.string()?;
            }
            Some(b'{') | Some(b'[') => {
                let mut depth = 0usize;
                while let Some(&c) = self.s.get(self.i) {
                    match c {
                        b'"' => {
                            self.string()?;
                            continue;
                        }
                        b'{' | b'[' => depth += 1,
                        b'}' | b']' => {
                            depth -= 1;
                            if depth == 0 {
                                self.i += 1;
                                return Ok((start, self.i));
                            }
                        }
                        _ => {}
                    }
                    self.i += 1;
                }
                return Err("对象或数组没有结尾".into());
            }
            Some(_) => {
                while self.i < self.s.len()
                    && !matches!(
                        self.s[self.i],
                        b',' | b'}' | b']' | b' ' | b'\t' | b'\r' | b'\n'
                    )
                {
                    self.i += 1;
                }
                if self.i == start {
                    return Err(format!("第 {start} 个字节不是值"));
                }
            }
            None => return Err("JSON 提前结束了".into()),
        }
        Ok((start, self.i))
    }

    /// 从 `{` 开始读一个对象的成员：(键, 值的范围)，和 `}` 的位置。
    fn members(&mut self) -> Result<(Vec<(String, Span)>, usize), String> {
        self.expect(b'{')?;
        let mut out = Vec::new();
        self.ws();
        if self.s.get(self.i) == Some(&b'}') {
            let close = self.i;
            self.i += 1;
            return Ok((out, close));
        }
        loop {
            let (ks, ke) = self.string()?;
            let key: String = serde_json::from_slice(&self.s[ks..ke]).map_err(|e| e.to_string())?;
            self.expect(b':')?;
            let span = self.value()?;
            out.push((key, span));
            self.ws();
            match self.s.get(self.i) {
                Some(b',') => self.i += 1,
                Some(b'}') => {
                    let close = self.i;
                    self.i += 1;
                    return Ok((out, close));
                }
                _ => return Err(format!("第 {} 个字节应该是 , 或 }}", self.i)),
            }
        }
    }
}

/// 把根对象里 `object` 这个子对象的 `sets` 几个键改成新值（没有的加在最后）。`object` 不存在或不是对象时报错，不新建。
pub fn set_in_object(text: &str, object: &str, sets: &[(&str, Value)]) -> Result<String, String> {
    let bytes = text.as_bytes();
    // 文件开头的 BOM 留着
    let body_start = if text.starts_with('\u{feff}') { 3 } else { 0 };
    let mut scanner = Scanner {
        s: bytes,
        i: body_start,
    };
    let (root, _) = scanner.members()?;
    let (_, (vs, _)) = root
        .iter()
        .find(|(k, _)| k == object)
        .ok_or_else(|| format!("配置里没有 {object}"))?;
    if bytes[*vs] != b'{' {
        return Err(format!("配置里的 {object} 不是对象"));
    }
    let mut inner = Scanner { s: bytes, i: *vs };
    let (members, close) = inner.members()?;

    // 换值：从后往前换，前面的位置不变
    let mut edits: Vec<(Span, String)> = Vec::new();
    let mut missing: Vec<(&str, &Value)> = Vec::new();
    for (key, value) in sets {
        let rendered = serde_json::to_string(value).map_err(|e| e.to_string())?;
        match members.iter().find(|(k, _)| k == key) {
            Some((_, span)) => edits.push((*span, rendered)),
            None => missing.push((key, value)),
        }
    }
    if !missing.is_empty() {
        // 照最后一个成员那一行的缩进；对象是空的就照 `{` 那一行再多两格
        let line_indent = |at: usize| -> String {
            let line_start = text[..at].rfind('\n').map(|p| p + 1).unwrap_or(0);
            text[line_start..at]
                .chars()
                .take_while(|c| *c == ' ' || *c == '\t')
                .collect()
        };
        let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
        let (anchor, indent, lead) = match members.last() {
            // 值开头那一行的缩进就是键的缩进（键和值在同一行）
            Some((_, (start, end))) => (*end, line_indent(*start), ","),
            None => (*vs + 1, format!("{}  ", line_indent(*vs)), ""),
        };
        let mut insert = String::new();
        for (i, (key, value)) in missing.iter().enumerate() {
            let sep = if i == 0 { lead } else { "," };
            insert.push_str(&format!(
                "{sep}{newline}{indent}{}: {}",
                serde_json::to_string(key).map_err(|e| e.to_string())?,
                serde_json::to_string(value).map_err(|e| e.to_string())?
            ));
        }
        if members.is_empty() {
            insert.push_str(newline);
            insert.push_str(&line_indent(close));
        }
        edits.push(((anchor, anchor), insert));
    }
    edits.sort_by_key(|e| std::cmp::Reverse(e.0 .0));
    let mut out = text.to_string();
    for ((start, end), new) in edits {
        out.replace_range(start..end, &new);
    }

    // 核对：只差这几个键
    let parse = |t: &str| {
        serde_json::from_str::<Value>(t.trim_start_matches('\u{feff}')).map_err(|e| e.to_string())
    };
    let mut want = parse(text)?;
    let got = parse(&out).map_err(|e| format!("改完不是合法的 JSON：{e}"))?;
    if let Some(obj) = want.get_mut(object).and_then(Value::as_object_mut) {
        for (key, value) in sets {
            obj.insert(key.to_string(), value.clone());
        }
    }
    if got != want {
        return Err("改完和原来的差了不止这几个键，没写".into());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 合成的、和 BetterGI 一样缩进 2 格的配置（值是假的）。
    const SAMPLE: &str = "{\r\n  \"maskWindowConfig\": {\r\n    \"maskEnabled\": true\r\n  },\r\n  \"notificationConfig\": {\r\n    \"jsNotificationEnabled\": false,\r\n    \"webhookEnabled\": false,\r\n    \"webhookEndpoint\": \"\",\r\n    \"webhookSendTo\": \"别动\",\r\n    \"discordBotTargets\": [\r\n      \"a\"\r\n    ]\r\n  },\r\n  \"hotKeyConfig\": {\r\n    \"x\": \"y\\\"z\"\r\n  }\r\n}";

    #[test]
    fn only_the_given_keys_change_and_every_other_byte_stays() {
        let sets = [
            ("webhookEnabled", json!(true)),
            (
                "webhookEndpoint",
                json!("http://127.0.0.1:38648/genshin/abc"),
            ),
        ];
        let out = set_in_object(SAMPLE, "notificationConfig", &sets).unwrap();
        let expected = SAMPLE
            .replace("\"webhookEnabled\": false", "\"webhookEnabled\": true")
            .replace(
                "\"webhookEndpoint\": \"\"",
                "\"webhookEndpoint\": \"http://127.0.0.1:38648/genshin/abc\"",
            );
        assert_eq!(out, expected);
        // 再填一次：一样
        assert_eq!(
            set_in_object(&out, "notificationConfig", &sets).unwrap(),
            out
        );
    }

    #[test]
    fn missing_keys_are_added_at_the_end_of_that_object() {
        let old = "{\n  \"notificationConfig\": {\n    \"jsNotificationEnabled\": false\n  },\n  \"other\": 1\n}\n";
        let out = set_in_object(
            old,
            "notificationConfig",
            &[("webhookEnabled", json!(true))],
        )
        .unwrap();
        assert_eq!(
            out,
            "{\n  \"notificationConfig\": {\n    \"jsNotificationEnabled\": false,\n    \"webhookEnabled\": true\n  },\n  \"other\": 1\n}\n"
        );
        let empty = "{\n  \"notificationConfig\": {}\n}";
        let out = set_in_object(
            empty,
            "notificationConfig",
            &[("webhookEnabled", json!(true))],
        )
        .unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v, json!({"notificationConfig": {"webhookEnabled": true}}));
    }

    #[test]
    fn refuses_instead_of_guessing() {
        let sets = [("webhookEnabled", json!(true))];
        assert!(
            set_in_object("{\"a\": 1}", "notificationConfig", &sets).is_err(),
            "没有这一节不新建"
        );
        assert!(set_in_object("{\"notificationConfig\": 1}", "notificationConfig", &sets).is_err());
        assert!(set_in_object("不是 JSON", "notificationConfig", &sets).is_err());
        assert!(set_in_object(
            "{\"notificationConfig\": {\"a\": [}",
            "notificationConfig",
            &sets
        )
        .is_err());
        // 带 BOM 的照样认、BOM 留着
        let bom = format!("\u{feff}{SAMPLE}");
        assert!(set_in_object(&bom, "notificationConfig", &sets)
            .unwrap()
            .starts_with('\u{feff}'));
    }
}
