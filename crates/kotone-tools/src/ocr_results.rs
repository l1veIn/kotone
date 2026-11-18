//! 驻留的 OCR 结果：`ocr.recognize` 提交后保存（有界、TTL），供 `chat.track.ingest`
//! 按 `ocrResultId` 引用，从而让跟踪使用服务端记录的帧、坐标与后端来源，而不是调用方
//! 转述的数据。与产物存储一样：提交与关闭在同一把锁内检查，关闭后不再接受任何结果。

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use crate::tracking::{IngestLine, Region};

const MAX_TOMBSTONES: usize = 256;

/// 一次识别的跟踪所需信息（坐标已映射到根产物）。
#[derive(Debug, Clone)]
pub struct StoredOcr {
    pub id: String,
    pub expires_at_unix_ms: u64,
    pub artifact_id: String,
    pub frame_id: String,
    pub window_id: String,
    pub captured_at_unix_ms: u64,
    pub roi: Region,
    pub frame_size: (u32, u32),
    pub backend_id: String,
    pub lines: Vec<IngestLine>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OcrLookupError {
    NotFound,
    Expired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OcrCommitError {
    Closed,
    DeadlineExpired,
}

#[derive(Default)]
struct Inner {
    closed: bool,
    live: HashMap<String, Arc<StoredOcr>>,
    order: VecDeque<String>,
    tombstones: VecDeque<String>,
}

impl Inner {
    fn bury(&mut self, id: String) {
        self.tombstones.push_back(id);
        while self.tombstones.len() > MAX_TOMBSTONES {
            self.tombstones.pop_front();
        }
    }

    fn purge(&mut self, now: u64) {
        let expired: Vec<String> = self
            .live
            .values()
            .filter(|r| r.expires_at_unix_ms <= now)
            .map(|r| r.id.clone())
            .collect();
        for id in expired {
            self.live.remove(&id);
            self.order.retain(|x| x != &id);
            self.bury(id);
        }
    }
}

pub struct OcrResults {
    inner: Mutex<Inner>,
    max: usize,
}

impl OcrResults {
    pub fn new(max: usize) -> Self {
        Self {
            inner: Mutex::new(Inner::default()),
            max: max.max(1),
        }
    }

    /// 提交闸门：锁内核对未关闭、未过期后插入；超出容量回收最旧结果。
    pub fn insert_gated(
        &self,
        record: StoredOcr,
        now: u64,
        deadline: std::time::Instant,
    ) -> Result<(), OcrCommitError> {
        let mut inner = self.inner.lock().unwrap();
        if inner.closed {
            return Err(OcrCommitError::Closed);
        }
        if std::time::Instant::now() >= deadline {
            return Err(OcrCommitError::DeadlineExpired);
        }
        inner.purge(now);
        while inner.live.len() >= self.max {
            let Some(oldest) = inner.order.pop_front() else {
                break;
            };
            inner.live.remove(&oldest);
            inner.bury(oldest);
        }
        inner.order.push_back(record.id.clone());
        inner.live.insert(record.id.clone(), Arc::new(record));
        Ok(())
    }

    pub fn get(&self, id: &str, now: u64) -> Result<Arc<StoredOcr>, OcrLookupError> {
        let mut inner = self.inner.lock().unwrap();
        inner.purge(now);
        if let Some(r) = inner.live.get(id) {
            return Ok(r.clone());
        }
        if inner.tombstones.iter().any(|t| t == id) {
            Err(OcrLookupError::Expired)
        } else {
            Err(OcrLookupError::NotFound)
        }
    }

    pub fn purge(&self, now: u64) {
        self.inner.lock().unwrap().purge(now);
    }

    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().live.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn close(&self) {
        let mut inner = self.inner.lock().unwrap();
        *inner = Inner {
            closed: true,
            ..Inner::default()
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn rec(id: &str, expires: u64) -> StoredOcr {
        StoredOcr {
            id: id.into(),
            expires_at_unix_ms: expires,
            artifact_id: "a".into(),
            frame_id: "f".into(),
            window_id: "w".into(),
            captured_at_unix_ms: 0,
            roi: Region {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            },
            frame_size: (1, 1),
            backend_id: "b".into(),
            lines: vec![],
        }
    }

    #[test]
    fn bounded_ttl_and_gate() {
        let store = OcrResults::new(2);
        let later = Instant::now() + Duration::from_secs(60);
        store.insert_gated(rec("a", 100), 0, later).unwrap();
        store.insert_gated(rec("b", 1000), 0, later).unwrap();
        store.insert_gated(rec("c", 1000), 0, later).unwrap();
        assert_eq!(
            store.get("a", 0).unwrap_err(),
            OcrLookupError::Expired,
            "容量回收"
        );
        assert!(store.get("b", 999).is_ok());
        assert_eq!(
            store.get("b", 1000).unwrap_err(),
            OcrLookupError::Expired,
            "TTL"
        );
        assert_eq!(store.get("zzz", 0).unwrap_err(), OcrLookupError::NotFound);
        let past = Instant::now() - Duration::from_millis(1);
        assert_eq!(
            store.insert_gated(rec("d", 1000), 0, past),
            Err(OcrCommitError::DeadlineExpired)
        );
        store.close();
        assert_eq!(
            store.insert_gated(rec("e", 1000), 0, later),
            Err(OcrCommitError::Closed)
        );
        assert!(store.is_empty());
    }
}
