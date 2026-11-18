//! 事件总线：Kotone 主动通知 Agent 的反向通道（`GET /v1/events`、`event.read`）。
//!
//! - 事件类型化（`user.request`、`runtime.phase`、`agent.reply.delivered` …），序号单调递增；
//! - 有界保留（数量上限），读取方用游标续读；游标绑定实例随机数——服务重启后旧游标
//!   返回 `EVENT_CURSOR_STALE`，被容量回收的位置返回 `EVENT_CURSOR_EXPIRED`，绝不静默跳过；
//! - 读取不改变总线状态，多个消费者互不影响（至少一次送达，按事件 ID 去重）；
//! - 等待新事件是异步的，不占用工具执行名额或阻塞线程。

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};

/// 一条事件。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    pub seq: u64,
    /// 全局唯一 ID（`evt_<实例随机数>_<序号>`），消费者据此去重。
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub at_unix_ms: u64,
    pub data: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CursorError {
    Invalid(String),
    /// 游标来自另一个服务实例（桌面端重启过）。
    Stale,
    /// 游标之后的事件已被容量回收。
    Expired {
        oldest_seq: u64,
    },
}

/// 读取起点（无游标时）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadFrom {
    Oldest,
    Latest,
}

#[derive(Default)]
struct Inner {
    events: VecDeque<Event>,
    next_seq: u64,
    closed: bool,
}

pub struct EventBus {
    inner: Mutex<Inner>,
    changed: tokio::sync::watch::Sender<u64>,
    nonce: String,
    max: usize,
}

fn nonce() -> String {
    let mut b = [0u8; 6];
    let _ = getrandom::getrandom(&mut b);
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// 事件类型过滤：精确匹配，或以 `.*` 结尾的前缀匹配（`agent.*`）。
pub fn type_matches(filters: &[String], kind: &str) -> bool {
    filters.is_empty()
        || filters.iter().any(|f| match f.strip_suffix(".*") {
            Some(prefix) => kind == prefix || kind.starts_with(&format!("{prefix}.")),
            None => f == kind,
        })
}

impl EventBus {
    pub fn new(max: usize) -> Self {
        Self {
            inner: Mutex::new(Inner {
                next_seq: 1,
                ..Inner::default()
            }),
            changed: tokio::sync::watch::channel(0).0,
            nonce: nonce(),
            max: max.max(1),
        }
    }

    fn cursor_for(&self, seq: u64) -> String {
        format!("ev1.{}.{seq}", self.nonce)
    }

    /// 发布事件，返回序号；关闭后返回 None（不再接受）。
    pub fn publish(&self, kind: &str, data: Value, now_unix_ms: u64) -> Option<u64> {
        self.push(kind, data, now_unix_ms, false)
    }

    /// 发布最后一个事件并在同一把锁内关闭。等待者被唤醒时一定同时看到这条事件和
    /// `closed: true`；分两步做时，长轮询可能读到事件而 `closed` 仍为 false，客户端
    /// 随即再发一次请求，只能收到“服务已关闭”。
    pub fn publish_last(&self, kind: &str, data: Value, now_unix_ms: u64) -> Option<u64> {
        self.push(kind, data, now_unix_ms, true)
    }

    fn push(&self, kind: &str, data: Value, now_unix_ms: u64, last: bool) -> Option<u64> {
        let seq = {
            let mut inner = self.inner.lock().unwrap();
            if inner.closed {
                return None;
            }
            let seq = inner.next_seq;
            inner.next_seq += 1;
            inner.events.push_back(Event {
                seq,
                id: format!("evt_{}_{seq}", self.nonce),
                kind: kind.to_string(),
                at_unix_ms: now_unix_ms,
                data,
            });
            while inner.events.len() > self.max {
                inner.events.pop_front();
            }
            inner.closed = last;
            seq
        };
        self.changed
            .send_replace(if last { seq.saturating_add(1) } else { seq });
        Some(seq)
    }

    /// 关闭：唤醒所有等待者，之后不再发布。
    pub fn close(&self) {
        self.inner.lock().unwrap().closed = true;
        let head = self.head();
        self.changed.send_replace(head.saturating_add(1));
    }

    pub fn is_closed(&self) -> bool {
        self.inner.lock().unwrap().closed
    }

    /// 当前最后一个序号（没有事件时为 0）。
    pub fn head(&self) -> u64 {
        self.inner.lock().unwrap().next_seq - 1
    }

    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 解析游标，返回“已读到的序号”。
    pub fn resolve(&self, cursor: Option<&str>, from: ReadFrom) -> Result<u64, CursorError> {
        let inner = self.inner.lock().unwrap();
        let head = inner.next_seq - 1;
        let oldest = inner
            .events
            .front()
            .map(|e| e.seq)
            .unwrap_or(inner.next_seq);
        let Some(cursor) = cursor else {
            return Ok(match from {
                ReadFrom::Oldest => oldest.saturating_sub(1),
                ReadFrom::Latest => head,
            });
        };
        let parts: Vec<&str> = cursor.split('.').collect();
        if parts.len() != 3 || parts[0] != "ev1" {
            return Err(CursorError::Invalid("游标格式无效".into()));
        }
        let seq: u64 = parts[2]
            .parse()
            .map_err(|_| CursorError::Invalid("游标序号无效".into()))?;
        if parts[1] != self.nonce {
            return Err(CursorError::Stale);
        }
        if seq > head {
            return Err(CursorError::Invalid(
                "游标来自未来（序号大于当前头部）".into(),
            ));
        }
        if seq + 1 < oldest {
            return Err(CursorError::Expired { oldest_seq: oldest });
        }
        Ok(seq)
    }

    /// 读取 `after` 之后、符合过滤的事件（最多 `limit` 条）。返回 JSON 结果。
    pub fn read_after(&self, after: u64, limit: usize, types: &[String]) -> Value {
        let inner = self.inner.lock().unwrap();
        let head = inner.next_seq - 1;
        let oldest = inner
            .events
            .front()
            .map(|e| e.seq)
            .unwrap_or(inner.next_seq);
        let mut page = Vec::new();
        let mut last_scanned = after;
        let mut has_more = false;
        for event in inner.events.iter().filter(|e| e.seq > after) {
            if page.len() >= limit {
                has_more = true;
                break;
            }
            last_scanned = event.seq;
            if type_matches(types, &event.kind) {
                page.push(event.clone());
            }
        }
        // 没有更多匹配事件时，游标推进到头部（跳过不匹配的事件，不重复扫描）
        let next = if has_more { last_scanned } else { head };
        json!({
            "events": page,
            "cursor": self.cursor_for(next),
            "hasMore": has_more,
            "headSeq": head,
            "oldestSeq": oldest,
            "closed": inner.closed,
        })
    }

    /// 等待序号超过 `after` 的事件出现（或关闭），最多 `wait`。返回是否有新事件。
    pub async fn wait_after(&self, after: u64, wait: Duration) -> bool {
        let mut rx = self.changed.subscribe();
        if self.head() > after || self.is_closed() {
            return true;
        }
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            match tokio::time::timeout_at(deadline, rx.changed()).await {
                Ok(Ok(())) => {
                    if self.head() > after || self.is_closed() {
                        return true;
                    }
                }
                _ => return false,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_last_event_and_closed_are_seen_together() {
        let bus = EventBus::new(16);
        bus.publish("a", Value::Null, 1).unwrap();
        let seq = bus.publish_last("app.stopping", Value::Null, 2).unwrap();
        let read = bus.read_after(seq - 1, 10, &[]);
        assert_eq!(read["events"][0]["type"], "app.stopping");
        assert_eq!(read["closed"], true);
        assert!(bus.publish("late", Value::Null, 3).is_none());
        assert!(bus.publish_last("again", Value::Null, 4).is_none());
    }

    fn kinds(v: &Value) -> Vec<String> {
        v["events"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["type"].as_str().unwrap().to_string())
            .collect()
    }

    #[test]
    fn cursors_resume_filter_and_detect_stale_or_expired_positions() {
        let bus = EventBus::new(3);
        assert_eq!(bus.resolve(None, ReadFrom::Oldest), Ok(0));
        for kind in [
            "app.started",
            "user.request",
            "runtime.phase",
            "user.request",
        ] {
            bus.publish(kind, json!({}), 1);
        }
        // 容量 3：序号 1 已回收
        let all = bus.read_after(bus.resolve(None, ReadFrom::Oldest).unwrap(), 10, &[]);
        assert_eq!(
            kinds(&all),
            vec!["user.request", "runtime.phase", "user.request"]
        );
        assert_eq!(all["oldestSeq"], 2);
        let only = bus.read_after(1, 10, &["user.*".into()]);
        assert_eq!(kinds(&only), vec!["user.request", "user.request"]);
        let cursor = only["cursor"].as_str().unwrap().to_string();
        assert_eq!(
            bus.resolve(Some(&cursor), ReadFrom::Oldest),
            Ok(4),
            "过滤后游标推进到头部"
        );
        // 分页
        let page = bus.read_after(1, 1, &[]);
        assert_eq!(page["hasMore"], true);
        let next = bus
            .resolve(page["cursor"].as_str(), ReadFrom::Oldest)
            .unwrap();
        assert_eq!(next, 2);
        // 过期、陈旧、无效、来自未来
        let stale_seq = format!("ev1.{}.0", bus.nonce);
        assert_eq!(
            bus.resolve(Some(&stale_seq), ReadFrom::Oldest),
            Err(CursorError::Expired { oldest_seq: 2 })
        );
        assert_eq!(
            bus.resolve(Some("ev1.000000000000.3"), ReadFrom::Oldest),
            Err(CursorError::Stale)
        );
        assert!(matches!(
            bus.resolve(Some("garbage"), ReadFrom::Oldest),
            Err(CursorError::Invalid(_))
        ));
        let future = format!("ev1.{}.99", bus.nonce);
        assert!(matches!(
            bus.resolve(Some(&future), ReadFrom::Oldest),
            Err(CursorError::Invalid(_))
        ));
        assert_eq!(bus.resolve(None, ReadFrom::Latest), Ok(4));
        assert!(type_matches(&["agent.*".into()], "agent.reply.delivered"));
        assert!(!type_matches(&["agent.*".into()], "agentx.y"));
    }

    #[tokio::test]
    async fn waiting_wakes_on_publish_and_on_close() {
        let bus = std::sync::Arc::new(EventBus::new(10));
        let b = bus.clone();
        let waiter = tokio::spawn(async move { b.wait_after(0, Duration::from_secs(5)).await });
        tokio::time::sleep(Duration::from_millis(30)).await;
        bus.publish("user.request", json!({"text": "hi"}), 1);
        assert!(waiter.await.unwrap());
        assert!(
            !bus.wait_after(1, Duration::from_millis(30)).await,
            "无新事件时超时返回 false"
        );
        let b = bus.clone();
        let waiter = tokio::spawn(async move { b.wait_after(1, Duration::from_secs(5)).await });
        tokio::time::sleep(Duration::from_millis(30)).await;
        bus.close();
        assert!(waiter.await.unwrap(), "关闭唤醒等待者");
        assert_eq!(bus.publish("x", json!({}), 1), None);
    }
}
