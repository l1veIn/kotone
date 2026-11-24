//! 不需要密码学强度的随机数（weapi 的一次性密钥、设备号、requestId）：用标准库的
//! `RandomState`（每个进程随机种子）加时间和计数器，免得为这点事引入 `rand`。

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static COUNTER: AtomicU64 = AtomicU64::new(0);

pub(crate) fn u64() -> u64 {
    let mut hasher = RandomState::new().build_hasher();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    hasher.write_u128(nanos);
    hasher.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
    hasher.finish()
}

/// `n` 个从 `alphabet` 里随机取的字符。
pub(crate) fn chars(alphabet: &[u8], n: usize) -> String {
    (0..n)
        .map(|_| alphabet[(u64() % alphabet.len() as u64) as usize] as char)
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn random_strings_have_the_right_shape_and_differ() {
        let a = super::chars(b"0123456789ABCDEF", 52);
        let b = super::chars(b"0123456789ABCDEF", 52);
        assert_eq!(a.len(), 52);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }
}
