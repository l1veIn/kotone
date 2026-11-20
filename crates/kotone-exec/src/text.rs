//! 文本：命令输出和文件的解码（UTF-8 / UTF-16 / 中文 Windows 的 GBK）、截断。

use encoding_rs::Encoding;

/// 命令输出交给模型的上限（字），保留头尾各一半（照 Claude Code 的 3 万字、Codex 的头尾截断）。
pub const OUTPUT_LIMIT_CHARS: usize = 30_000;

/// 文件的编码（写回时保持不变）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileEncoding {
    Utf8,
    Utf8Bom,
    Utf16Le,
    Utf16Be,
    /// 系统的 ANSI 代码页（中文 Windows 是 GBK）。
    Ansi(&'static Encoding),
}

impl FileEncoding {
    pub fn label(self) -> &'static str {
        match self {
            FileEncoding::Utf8 => "utf-8",
            FileEncoding::Utf8Bom => "utf-8-bom",
            FileEncoding::Utf16Le => "utf-16le",
            FileEncoding::Utf16Be => "utf-16be",
            FileEncoding::Ansi(e) => e.name(),
        }
    }

    /// 按这个编码写出；ANSI 编码写不下的字符改用 UTF-8（返回实际用的编码）。
    pub fn encode(self, text: &str) -> (Vec<u8>, FileEncoding) {
        match self {
            FileEncoding::Utf8 => (text.as_bytes().to_vec(), self),
            FileEncoding::Utf8Bom => {
                let mut out = vec![0xEF, 0xBB, 0xBF];
                out.extend_from_slice(text.as_bytes());
                (out, self)
            }
            FileEncoding::Utf16Le => {
                let mut out = vec![0xFF, 0xFE];
                out.extend(text.encode_utf16().flat_map(|u| u.to_le_bytes()));
                (out, self)
            }
            FileEncoding::Utf16Be => {
                let mut out = vec![0xFE, 0xFF];
                out.extend(text.encode_utf16().flat_map(|u| u.to_be_bytes()));
                (out, self)
            }
            FileEncoding::Ansi(enc) => {
                let (bytes, _, had_errors) = enc.encode(text);
                if had_errors {
                    (text.as_bytes().to_vec(), FileEncoding::Utf8)
                } else {
                    (bytes.into_owned(), self)
                }
            }
        }
    }
}

/// 看起来是二进制（不是 UTF-16 却有 NUL）。
pub fn looks_binary(bytes: &[u8]) -> bool {
    if bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF]) {
        return false;
    }
    bytes.iter().take(8_000).any(|&b| b == 0)
}

/// 解码一个文本文件。
pub fn decode_file(bytes: &[u8]) -> (String, FileEncoding) {
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return (
            String::from_utf8_lossy(rest).into_owned(),
            FileEncoding::Utf8Bom,
        );
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        let units: Vec<u16> = rest
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .collect();
        return (String::from_utf16_lossy(&units), FileEncoding::Utf16Le);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        let units: Vec<u16> = rest
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_be_bytes(*c))
            .collect();
        return (String::from_utf16_lossy(&units), FileEncoding::Utf16Be);
    }
    match std::str::from_utf8(bytes) {
        Ok(s) => (s.to_string(), FileEncoding::Utf8),
        Err(_) => {
            let enc = ansi_encoding();
            let (text, _) = enc.decode_without_bom_handling(bytes);
            (text.into_owned(), FileEncoding::Ansi(enc))
        }
    }
}

/// 命令输出：整段是 UTF-8 就直接用；否则逐行试 UTF-8，不行按控制台代码页（中文 Windows 是 GBK）。
pub fn decode_output(bytes: &[u8]) -> String {
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    if let Ok(s) = std::str::from_utf8(bytes) {
        return s.to_string();
    }
    let enc = oem_encoding();
    let mut out = String::with_capacity(bytes.len());
    for line in bytes.split_inclusive(|&b| b == b'\n') {
        match std::str::from_utf8(line) {
            Ok(s) => out.push_str(s),
            Err(_) => out.push_str(&enc.decode_without_bom_handling(line).0),
        }
    }
    out
}

/// 超过 `limit` 字时保留头尾各一半，中间写明省略了多少。返回（文本，是否截断）。
pub fn truncate_middle(text: &str, limit: usize) -> (String, bool) {
    let total = text.chars().count();
    if total <= limit {
        return (text.to_string(), false);
    }
    let half = limit / 2;
    let head: String = text.chars().take(half).collect();
    let tail: String = text.chars().skip(total - half).collect();
    (
        format!("{head}\n…（中间省略 {} 字）…\n{tail}", total - 2 * half),
        true,
    )
}

fn encoding_for_code_page(cp: u32) -> &'static Encoding {
    match cp {
        936 => encoding_rs::GBK,
        950 => encoding_rs::BIG5,
        932 => encoding_rs::SHIFT_JIS,
        949 => encoding_rs::EUC_KR,
        65001 => encoding_rs::UTF_8,
        1251 | 866 => encoding_rs::WINDOWS_1251,
        _ => encoding_rs::WINDOWS_1252,
    }
}

#[cfg(windows)]
fn ansi_encoding() -> &'static Encoding {
    // SAFETY: 无参数的系统查询。
    encoding_for_code_page(unsafe { windows::Win32::Globalization::GetACP() })
}

#[cfg(windows)]
fn oem_encoding() -> &'static Encoding {
    // SAFETY: 无参数的系统查询。
    encoding_for_code_page(unsafe { windows::Win32::Globalization::GetOEMCP() })
}

#[cfg(not(windows))]
fn ansi_encoding() -> &'static Encoding {
    encoding_rs::GBK
}

#[cfg(not(windows))]
fn oem_encoding() -> &'static Encoding {
    encoding_rs::GBK
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_round_trip_in_their_own_encoding() {
        for enc in [
            FileEncoding::Utf8,
            FileEncoding::Utf8Bom,
            FileEncoding::Utf16Le,
            FileEncoding::Utf16Be,
            FileEncoding::Ansi(encoding_rs::GBK),
        ] {
            let (bytes, used) = enc.encode("中文 abc\r\n第二行");
            assert_eq!(used, enc);
            let (text, detected) = decode_file(&bytes);
            assert_eq!(text, "中文 abc\r\n第二行");
            match enc {
                FileEncoding::Ansi(_) => assert!(matches!(detected, FileEncoding::Ansi(_))),
                other => assert_eq!(detected, other),
            }
        }
    }

    #[test]
    fn ansi_falls_back_to_utf8_for_unmappable_text() {
        let (_, used) = FileEncoding::Ansi(encoding_rs::GBK).encode("😀");
        assert_eq!(used, FileEncoding::Utf8);
    }

    #[test]
    fn mixed_output_decodes_line_by_line() {
        let mut bytes = "UTF-8 行\n".as_bytes().to_vec();
        bytes.extend(encoding_rs::GBK.encode("GBK 行\n").0.iter());
        let text = decode_output(&bytes);
        assert!(text.contains("UTF-8 行"));
        if oem_encoding() == encoding_rs::GBK {
            assert!(text.contains("GBK 行"));
        }
    }

    #[test]
    fn binary_is_detected_but_utf16_is_not() {
        assert!(looks_binary(&[0x4D, 0x5A, 0x00, 0x01]));
        assert!(!looks_binary(&FileEncoding::Utf16Le.encode("a").0));
        assert!(!looks_binary("中文".as_bytes()));
    }

    #[test]
    fn truncation_keeps_head_and_tail() {
        let text: String = (0..100)
            .map(|i| char::from(b'a' + (i % 26) as u8))
            .collect();
        let (out, cut) = truncate_middle(&text, 20);
        assert!(cut);
        assert!(out.starts_with(&text[..10]));
        assert!(out.ends_with(&text[90..]));
        assert!(out.contains("省略 80 字"));
        assert_eq!(truncate_middle("短", 20), ("短".to_string(), false));
    }
}
