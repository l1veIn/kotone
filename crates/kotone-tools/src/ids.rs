//! 随机标识与会话凭证。

use std::fmt;

fn random_hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    getrandom::getrandom(&mut buf).expect("操作系统随机数源不可用");
    let mut out = String::with_capacity(bytes * 2);
    for b in buf {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// 不透明标识：`<prefix>_<96 位随机十六进制>`。
pub fn new_id(prefix: &str) -> String {
    format!("{prefix}_{}", random_hex(12))
}

/// 会话凭证（256 位随机数）。`Debug`/`Display` 均脱敏，避免意外写入日志。
#[derive(Clone, PartialEq, Eq)]
pub struct SecretToken(String);

impl SecretToken {
    pub fn generate() -> Self {
        Self(random_hex(32))
    }

    pub fn from_string(value: String) -> Self {
        Self(value)
    }

    pub fn expose(&self) -> &str {
        &self.0
    }

    /// 常量时间比较（长度不同直接失败；长度本身不是秘密）。
    pub fn matches(&self, candidate: &str) -> bool {
        let a = self.0.as_bytes();
        let b = candidate.as_bytes();
        if a.len() != b.len() {
            return false;
        }
        a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
    }
}

impl fmt::Debug for SecretToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretToken(<redacted>)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_prefixed_and_unique() {
        let a = new_id("art");
        let b = new_id("art");
        assert!(a.starts_with("art_") && a.len() == 28);
        assert_ne!(a, b);
    }

    #[test]
    fn token_is_redacted_and_compared() {
        let token = SecretToken::generate();
        assert_eq!(token.expose().len(), 64);
        assert!(!format!("{token:?}").contains(token.expose()));
        assert!(token.matches(token.expose()));
        assert!(!token.matches(&token.expose()[1..]));
        let mut wrong = token.expose().to_string();
        wrong.replace_range(0..1, if wrong.starts_with('0') { "1" } else { "0" });
        assert!(!token.matches(&wrong));
    }
}
