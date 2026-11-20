//! 边生成边念 `speak`（ADR 029 第八节）：模型的工具参数是一段一段流过来的 JSON，
//! 从还没写完的 `{"text": "…` 里取出到目前为止的字，交给语音合成。
//!
//! 每次都从拼到现在的整段参数重新取（参数只有一两句话，便宜），所以名字、参数分几段到都没关系；
//! 结果总是上一次结果的延长。写到一半的转义（`\`、`\u4e`）先不算，等后面的字到了再取。

/// 拼到现在的参数里 `text` 的值（解开转义）；还没写到 `text` 时为空。
pub fn text_so_far(raw: &str) -> String {
    let mut p = Parser {
        s: raw.as_bytes(),
        i: 0,
    };
    p.ws();
    if !p.eat(b'{') {
        return String::new();
    }
    loop {
        p.ws();
        if p.peek() != Some(b'"') {
            return String::new();
        }
        let (key, closed) = p.string();
        if !closed {
            return String::new();
        }
        p.ws();
        if !p.eat(b':') {
            return String::new();
        }
        p.ws();
        if key == "text" {
            return if p.peek() == Some(b'"') {
                p.string().0
            } else {
                String::new()
            };
        }
        if !p.skip_value() {
            return String::new();
        }
        p.ws();
        if !p.eat(b',') {
            return String::new();
        }
    }
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }

    fn eat(&mut self, b: u8) -> bool {
        if self.peek() == Some(b) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\n' | b'\r' | b'\t')) {
            self.i += 1;
        }
    }

    /// 从引号开始读一个字符串；返回（解开转义的字，有没有读到结尾的引号）。
    fn string(&mut self) -> (String, bool) {
        self.i += 1;
        let mut out = String::new();
        let mut run = self.i;
        while let Some(b) = self.peek() {
            match b {
                b'"' => {
                    out.push_str(&String::from_utf8_lossy(&self.s[run..self.i]));
                    self.i += 1;
                    return (out, true);
                }
                b'\\' => {
                    out.push_str(&String::from_utf8_lossy(&self.s[run..self.i]));
                    match self.escape() {
                        Some(c) => out.push_str(&c),
                        // 转义还没写完：到这里为止
                        None => return (out, false),
                    }
                    run = self.i;
                }
                _ => self.i += 1,
            }
        }
        // 没写完（输入是 &str，按字切开，不会停在一个汉字中间）
        out.push_str(&String::from_utf8_lossy(&self.s[run..]));
        (out, false)
    }

    /// 读一个转义（`\` 开头）；没写完返回 None。
    fn escape(&mut self) -> Option<String> {
        let c = *self.s.get(self.i + 1)?;
        let simple = match c {
            b'"' => Some('"'),
            b'\\' => Some('\\'),
            b'/' => Some('/'),
            b'b' => Some('\u{8}'),
            b'f' => Some('\u{c}'),
            b'n' => Some('\n'),
            b'r' => Some('\r'),
            b't' => Some('\t'),
            _ => None,
        };
        if let Some(ch) = simple {
            self.i += 2;
            return Some(ch.to_string());
        }
        if c != b'u' {
            // 不认识的转义：原样跳过
            self.i += 2;
            return Some(String::new());
        }
        let first = self.hex4(self.i + 2)?;
        if (0xD800..0xDC00).contains(&first) {
            // 高代理：要等低代理到了才是一个字
            if self.s.get(self.i + 6) != Some(&b'\\') || self.s.get(self.i + 7) != Some(&b'u') {
                if self.i + 8 > self.s.len() {
                    return None;
                }
                self.i += 6;
                return Some(String::new());
            }
            let second = self.hex4(self.i + 8)?;
            self.i += 12;
            let code = 0x10000 + ((first - 0xD800) << 10) + (second.wrapping_sub(0xDC00) & 0x3FF);
            return Some(char::from_u32(code).map(String::from).unwrap_or_default());
        }
        self.i += 6;
        Some(char::from_u32(first).map(String::from).unwrap_or_default())
    }

    fn hex4(&self, at: usize) -> Option<u32> {
        let digits = self.s.get(at..at + 4)?;
        u32::from_str_radix(std::str::from_utf8(digits).ok()?, 16).ok()
    }

    /// 跳过一个不是 `text` 的值；没写完返回 false。
    fn skip_value(&mut self) -> bool {
        if self.peek() == Some(b'"') {
            return self.string().1;
        }
        let mut depth = 0i32;
        while let Some(b) = self.peek() {
            match b {
                b'"' => {
                    if !self.string().1 {
                        return false;
                    }
                    continue;
                }
                b'{' | b'[' => depth += 1,
                b'}' | b']' if depth == 0 => return true,
                b'}' | b']' => depth -= 1,
                b',' if depth == 0 => return true,
                _ => {}
            }
            self.i += 1;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::text_so_far;

    #[test]
    fn reads_the_text_as_it_streams() {
        let full = r#"{"text": "好的，\"大龙\"刷了\n第二句中😀完"}"#;
        let want = "好的，\"大龙\"刷了\n第二句中😀完";
        assert_eq!(text_so_far(full), want);
        // 一个字节一个字节地到：每次都是最终结果的开头，而且越来越长
        let mut last = String::new();
        for end in 0..=full.len() {
            let Some(prefix) = full.get(..end) else {
                continue;
            };
            let now = text_so_far(prefix);
            assert!(want.starts_with(&now), "{prefix:?} → {now:?}");
            assert!(now.len() >= last.len(), "{prefix:?}");
            last = now;
        }
        assert_eq!(last, want);
    }

    #[test]
    fn other_keys_and_unfinished_starts() {
        let full = r#"{"other": {"a": [1, "x,}"]}, "text":"中文"}"#;
        assert_eq!(text_so_far(full), "中文");
        assert_eq!(text_so_far(""), "");
        assert_eq!(text_so_far("{"), "");
        assert_eq!(text_so_far(r#"{"text": 3}"#), "");
        assert_eq!(text_so_far(r#"{"tex"#), "");
    }
}
