//! 同一个请求 2 分钟内用上次的结果（照 api-enhanced：防网易云的“IP 高频”风控，ADR 034）。

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::Value;

pub(crate) const TTL: Duration = Duration::from_secs(120);

pub(crate) struct TtlCache {
    ttl: Duration,
    entries: Mutex<HashMap<String, (Instant, Value)>>,
}

impl TtlCache {
    pub(crate) fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            entries: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) fn get(&self, key: &str) -> Option<Value> {
        let entries = self.entries.lock().unwrap();
        entries
            .get(key)
            .filter(|(at, _)| at.elapsed() < self.ttl)
            .map(|(_, value)| value.clone())
    }

    pub(crate) fn insert(&self, key: String, value: Value) {
        let mut entries = self.entries.lock().unwrap();
        let ttl = self.ttl;
        entries.retain(|_, (at, _)| at.elapsed() < ttl);
        entries.insert(key, (Instant::now(), value));
    }

    /// 登录、退出以后：缓存里的结果是另一个身份的，全部作废。
    pub(crate) fn clear(&self) {
        self.entries.lock().unwrap().clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn entries_expire_and_clear() {
        let cache = TtlCache::new(Duration::from_millis(30));
        cache.insert("a".into(), json!(1));
        assert_eq!(cache.get("a"), Some(json!(1)));
        assert_eq!(cache.get("b"), None);
        std::thread::sleep(Duration::from_millis(40));
        assert_eq!(cache.get("a"), None, "expired");
        cache.insert("a".into(), json!(2));
        cache.clear();
        assert_eq!(cache.get("a"), None);
        assert_eq!(TTL, Duration::from_secs(120));
    }
}
