//! 不可变、有界的图像产物存储。
//!
//! - 产物创建后不可修改；裁剪生成新产物并记录父产物、根产物与到根图的平移变换。
//! - 每个产物有 TTL；存储按数量与总字节上限回收最旧产物。
//! - 已释放 / 过期 / 被回收的 ID 留有限墓碑，返回可区分的错误；从未存在的 ID 为 NotFound。
//! - 锁内只做哈希表操作；裁剪、PNG 编码、哈希均在锁外完成。

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use serde::Serialize;

use crate::image::RgbaImage;

/// 墓碑保留数量上限。
const MAX_TOMBSTONES: usize = 1024;

pub trait Clock: Send + Sync {
    fn now_unix_ms(&self) -> u64;
}

#[derive(Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_unix_ms(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ArtifactKind {
    Capture,
    Crop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RectJson {
    pub x: i64,
    pub y: i64,
    pub width: u32,
    pub height: u32,
}

/// 从当前产物坐标映射回根产物：`root = offset + scale * p`。M1 只有裁剪，scale 恒为 1。
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RootTransform {
    pub offset_x: i64,
    pub offset_y: i64,
    pub scale_x: f64,
    pub scale_y: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WarningJson {
    pub code: String,
    pub message: String,
}

/// 原始采集来源；整条裁剪链共享，不随裁剪改变。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureSource {
    pub frame_id: String,
    pub window_id: String,
    pub capture_method: String,
    /// 采集时客户区的屏幕物理坐标。
    pub client_rect_screen: RectJson,
    pub dpi: Option<u32>,
    pub captured_at_unix_ms: u64,
    pub process_name: Option<String>,
    pub warnings: Vec<WarningJson>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactMeta {
    pub artifact_id: String,
    pub kind: ArtifactKind,
    pub mime_type: &'static str,
    pub width: u32,
    pub height: u32,
    /// PNG 字节数。
    pub byte_length: usize,
    /// PNG 字节的 SHA-256（十六进制）。
    pub sha256: String,
    pub created_at_unix_ms: u64,
    pub expires_at_unix_ms: u64,
    pub ttl_ms: u64,
    pub parent_artifact_id: Option<String>,
    pub root_artifact_id: String,
    pub coordinate_space: &'static str,
    pub root_transform: RootTransform,
    /// 本产物在根产物中的位置。
    pub root_rect: RectJson,
    /// 本产物在屏幕上的物理位置（采集时刻）。
    pub screen_rect: RectJson,
    pub source: CaptureSource,
}

pub const COORDINATE_SPACE: &str = "artifact-physical-pixels";
pub const MIME_PNG: &str = "image/png";

pub struct ArtifactRecord {
    pub meta: ArtifactMeta,
    pub image: RgbaImage,
    pub png: Arc<[u8]>,
}

impl ArtifactRecord {
    fn cost(&self) -> u64 {
        (self.image.byte_len() + self.png.len()) as u64
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gone {
    Released,
    Expired,
    Evicted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LookupError {
    NotFound,
    Gone(Gone),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReleaseOutcome {
    Released,
    AlreadyGone(Gone),
}

impl ReleaseOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Released => "released",
            Self::AlreadyGone(Gone::Released) => "already-released",
            Self::AlreadyGone(Gone::Expired) => "already-expired",
            Self::AlreadyGone(Gone::Evicted) => "already-evicted",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InsertError {
    /// 单个产物就超过存储总容量。
    TooLarge { bytes: u64, max_bytes: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoreStats {
    pub count: usize,
    pub total_bytes: u64,
    pub max_artifacts: usize,
    pub max_store_bytes: u64,
}

#[derive(Default)]
struct Inner {
    live: HashMap<String, Arc<ArtifactRecord>>,
    /// 插入顺序（回收最旧）。
    order: VecDeque<String>,
    total_bytes: u64,
    tombstones: HashMap<String, Gone>,
    tomb_order: VecDeque<String>,
}

impl Inner {
    fn bury(&mut self, id: String, why: Gone) {
        if self.tombstones.insert(id.clone(), why).is_none() {
            self.tomb_order.push_back(id);
        }
        while self.tomb_order.len() > MAX_TOMBSTONES {
            if let Some(old) = self.tomb_order.pop_front() {
                self.tombstones.remove(&old);
            }
        }
    }

    fn remove(&mut self, id: &str, why: Gone) -> bool {
        let Some(record) = self.live.remove(id) else {
            return false;
        };
        self.total_bytes -= record.cost();
        self.order.retain(|x| x != id);
        self.bury(id.to_string(), why);
        true
    }

    fn purge_expired(&mut self, now: u64) {
        let expired: Vec<String> = self
            .live
            .iter()
            .filter(|(_, r)| r.meta.expires_at_unix_ms <= now)
            .map(|(id, _)| id.clone())
            .collect();
        for id in expired {
            self.remove(&id, Gone::Expired);
        }
    }
}

pub struct ArtifactStore {
    inner: Mutex<Inner>,
    clock: Arc<dyn Clock>,
    max_artifacts: usize,
    max_bytes: u64,
}

impl ArtifactStore {
    pub fn new(clock: Arc<dyn Clock>, max_artifacts: usize, max_bytes: u64) -> Self {
        Self {
            inner: Mutex::new(Inner::default()),
            clock,
            max_artifacts: max_artifacts.max(1),
            max_bytes,
        }
    }

    pub fn now(&self) -> u64 {
        self.clock.now_unix_ms()
    }

    /// 插入新产物；必要时回收最旧产物。返回被回收的 ID。
    pub fn insert(&self, record: ArtifactRecord) -> Result<Vec<String>, InsertError> {
        let cost = record.cost();
        if cost > self.max_bytes {
            return Err(InsertError::TooLarge {
                bytes: cost,
                max_bytes: self.max_bytes,
            });
        }
        let now = self.now();
        let mut inner = self.inner.lock().unwrap();
        inner.purge_expired(now);
        let mut evicted = Vec::new();
        while inner.live.len() >= self.max_artifacts || inner.total_bytes + cost > self.max_bytes {
            let Some(oldest) = inner.order.front().cloned() else {
                break;
            };
            inner.remove(&oldest, Gone::Evicted);
            evicted.push(oldest);
        }
        let id = record.meta.artifact_id.clone();
        inner.total_bytes += cost;
        inner.order.push_back(id.clone());
        inner.live.insert(id, Arc::new(record));
        Ok(evicted)
    }

    pub fn get(&self, id: &str) -> Result<Arc<ArtifactRecord>, LookupError> {
        let now = self.now();
        let mut inner = self.inner.lock().unwrap();
        inner.purge_expired(now);
        if let Some(record) = inner.live.get(id) {
            return Ok(record.clone());
        }
        match inner.tombstones.get(id) {
            Some(why) => Err(LookupError::Gone(*why)),
            None => Err(LookupError::NotFound),
        }
    }

    /// 幂等释放：重复释放或释放已过期产物返回 AlreadyGone，不报错。
    pub fn release(&self, id: &str) -> Result<ReleaseOutcome, LookupError> {
        let now = self.now();
        let mut inner = self.inner.lock().unwrap();
        inner.purge_expired(now);
        if inner.remove(id, Gone::Released) {
            return Ok(ReleaseOutcome::Released);
        }
        match inner.tombstones.get(id) {
            Some(why) => Ok(ReleaseOutcome::AlreadyGone(*why)),
            None => Err(LookupError::NotFound),
        }
    }

    pub fn stats(&self) -> StoreStats {
        let now = self.now();
        let mut inner = self.inner.lock().unwrap();
        inner.purge_expired(now);
        StoreStats {
            count: inner.live.len(),
            total_bytes: inner.total_bytes,
            max_artifacts: self.max_artifacts,
            max_store_bytes: self.max_bytes,
        }
    }

    /// 服务关闭时丢弃全部产物（内存释放；产物从不落盘）。
    pub fn clear(&self) {
        let mut inner = self.inner.lock().unwrap();
        *inner = Inner::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image::coordinate_fixture;
    use crate::testing::ManualClock;

    fn record(id: &str, now: u64, ttl: u64, side: u32) -> ArtifactRecord {
        let image = coordinate_fixture(side, side);
        let png: Arc<[u8]> = crate::image::encode_png(&image).unwrap().into();
        let rect = RectJson {
            x: 0,
            y: 0,
            width: side,
            height: side,
        };
        ArtifactRecord {
            meta: ArtifactMeta {
                artifact_id: id.into(),
                kind: ArtifactKind::Capture,
                mime_type: MIME_PNG,
                width: side,
                height: side,
                byte_length: png.len(),
                sha256: crate::image::sha256_hex(&png),
                created_at_unix_ms: now,
                expires_at_unix_ms: now + ttl,
                ttl_ms: ttl,
                parent_artifact_id: None,
                root_artifact_id: id.into(),
                coordinate_space: COORDINATE_SPACE,
                root_transform: RootTransform {
                    offset_x: 0,
                    offset_y: 0,
                    scale_x: 1.0,
                    scale_y: 1.0,
                },
                root_rect: rect,
                screen_rect: rect,
                source: CaptureSource {
                    frame_id: "frm_test".into(),
                    window_id: "win_test".into(),
                    capture_method: "fake".into(),
                    client_rect_screen: rect,
                    dpi: Some(96),
                    captured_at_unix_ms: now,
                    process_name: None,
                    warnings: vec![],
                },
            },
            image,
            png,
        }
    }

    #[test]
    fn ttl_expiry_is_distinguished_from_unknown() {
        let clock = ManualClock::at(1_000);
        let store = ArtifactStore::new(clock.clone(), 8, u64::MAX);
        store.insert(record("a", 1_000, 500, 4)).unwrap();
        assert!(store.get("a").is_ok());
        clock.advance(499);
        assert!(store.get("a").is_ok());
        clock.advance(1);
        assert_eq!(store.get("a").err(), Some(LookupError::Gone(Gone::Expired)));
        assert_eq!(store.get("never").err(), Some(LookupError::NotFound));
        assert_eq!(store.stats().count, 0);
        assert_eq!(store.stats().total_bytes, 0);
        // 释放已过期产物是幂等成功
        assert_eq!(
            store.release("a"),
            Ok(ReleaseOutcome::AlreadyGone(Gone::Expired))
        );
    }

    #[test]
    fn release_is_idempotent_and_blocks_reads() {
        let clock = ManualClock::at(0);
        let store = ArtifactStore::new(clock, 8, u64::MAX);
        store.insert(record("a", 0, 10_000, 4)).unwrap();
        assert_eq!(store.release("a"), Ok(ReleaseOutcome::Released));
        assert_eq!(
            store.release("a"),
            Ok(ReleaseOutcome::AlreadyGone(Gone::Released))
        );
        assert_eq!(
            store.get("a").err(),
            Some(LookupError::Gone(Gone::Released))
        );
        assert_eq!(store.release("missing"), Err(LookupError::NotFound));
    }

    #[test]
    fn capacity_by_count_evicts_oldest() {
        let clock = ManualClock::at(0);
        let store = ArtifactStore::new(clock, 2, u64::MAX);
        store.insert(record("a", 0, 10_000, 4)).unwrap();
        store.insert(record("b", 0, 10_000, 4)).unwrap();
        let evicted = store.insert(record("c", 0, 10_000, 4)).unwrap();
        assert_eq!(evicted, vec!["a".to_string()]);
        assert_eq!(store.get("a").err(), Some(LookupError::Gone(Gone::Evicted)));
        assert!(store.get("b").is_ok() && store.get("c").is_ok());
        assert_eq!(store.stats().count, 2);
    }

    #[test]
    fn capacity_by_bytes_evicts_and_rejects_oversized() {
        let clock = ManualClock::at(0);
        let one = record("x", 0, 10_000, 8).cost();
        let store = ArtifactStore::new(clock, 100, one * 2);
        store.insert(record("a", 0, 10_000, 8)).unwrap();
        store.insert(record("b", 0, 10_000, 8)).unwrap();
        assert_eq!(store.stats().total_bytes, one * 2);
        let evicted = store.insert(record("c", 0, 10_000, 8)).unwrap();
        assert_eq!(evicted, vec!["a".to_string()]);
        assert_eq!(store.stats().total_bytes, one * 2);
        let huge = record("h", 0, 10_000, 64);
        assert!(matches!(
            store.insert(huge),
            Err(InsertError::TooLarge { .. })
        ));
        // 拒绝超大产物不影响现有产物
        assert_eq!(store.stats().count, 2);
    }

    #[test]
    fn expired_entries_free_capacity_before_eviction() {
        let clock = ManualClock::at(0);
        let store = ArtifactStore::new(clock.clone(), 2, u64::MAX);
        store.insert(record("short", 0, 100, 4)).unwrap();
        store.insert(record("long", 0, 10_000, 4)).unwrap();
        clock.advance(200);
        let evicted = store.insert(record("new", 200, 10_000, 4)).unwrap();
        assert!(evicted.is_empty(), "过期产物应先被清理而非回收存活产物");
        assert!(store.get("long").is_ok());
        assert_eq!(
            store.get("short").err(),
            Some(LookupError::Gone(Gone::Expired))
        );
    }

    #[test]
    fn tombstones_are_bounded() {
        let clock = ManualClock::at(0);
        let store = ArtifactStore::new(clock, 4, u64::MAX);
        for i in 0..(MAX_TOMBSTONES + 10) {
            let id = format!("t{i}");
            store.insert(record(&id, 0, 10_000, 1)).unwrap();
            store.release(&id).unwrap();
        }
        let inner = store.inner.lock().unwrap();
        assert!(inner.tombstones.len() <= MAX_TOMBSTONES);
        assert_eq!(inner.tombstones.len(), inner.tomb_order.len());
    }
}
