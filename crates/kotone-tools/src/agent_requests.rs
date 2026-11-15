//! 用户经 Kotone 发给 Agent 的请求（`user.request`）及其状态。
//!
//! 事件只是通知；请求本身有状态，便于多个消费者 / 插件重启后不重复处理：
//! `pending` →（`agent.request.claim`）`claimed` →（`agent.reply`）`replied`；
//! 超过有效期未领取或未回复的请求变为 `expired`（用户多半已不再等待）。
//! 领取幂等：同一消费者重复领取成功；其他消费者领取已领取的请求被拒绝。
//! 有界：最多保留 `max` 条，超出回收最旧的已结束请求，再回收最旧请求。

use std::collections::VecDeque;
use std::sync::Mutex;

use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RequestStatus {
    Pending,
    Claimed,
    Replied,
    Expired,
}

impl RequestStatus {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "pending" => Self::Pending,
            "claimed" => Self::Claimed,
            "replied" => Self::Replied,
            "expired" => Self::Expired,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRequest {
    pub request_id: String,
    /// 用户原文（数据，不是给 Kotone 的指令）。
    pub text: String,
    /// 来源：`assistant-voice` / `assistant-text` / `tool`。
    pub source: String,
    pub status: RequestStatus,
    pub created_at_unix_ms: u64,
    pub expires_at_unix_ms: u64,
    /// 提交时的前台窗口（工具服务的窗口引用与元数据）；无法获取时为 null。
    pub foreground_window: Value,
    pub claimed_by: Option<String>,
    pub claimed_at_unix_ms: Option<u64>,
    pub replies: u32,
    pub last_reply_at_unix_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RequestError {
    NotFound,
    Expired,
    AlreadyClaimed { by: String },
    Closed,
}

#[derive(Default)]
struct Inner {
    closed: bool,
    requests: VecDeque<AgentRequest>,
}

pub struct AgentRequests {
    inner: Mutex<Inner>,
    ttl_ms: u64,
    max: usize,
}

fn new_request_id() -> String {
    crate::ids::new_id("areq")
}

impl AgentRequests {
    pub fn new(ttl_ms: u64, max: usize) -> Self {
        Self {
            inner: Mutex::new(Inner::default()),
            ttl_ms,
            max: max.max(1),
        }
    }

    /// 把到期且未回复的请求标记为 expired，返回本次变为 expired 的请求。
    pub fn expire(&self, now: u64) -> Vec<AgentRequest> {
        let mut inner = self.inner.lock().unwrap();
        let mut expired = Vec::new();
        for r in inner.requests.iter_mut() {
            if matches!(r.status, RequestStatus::Pending | RequestStatus::Claimed)
                && r.expires_at_unix_ms <= now
            {
                r.status = RequestStatus::Expired;
                expired.push(r.clone());
            }
        }
        expired
    }

    pub fn submit(
        &self,
        text: String,
        source: &str,
        foreground_window: Value,
        now: u64,
    ) -> Result<AgentRequest, RequestError> {
        let mut inner = self.inner.lock().unwrap();
        if inner.closed {
            return Err(RequestError::Closed);
        }
        while inner.requests.len() >= self.max {
            let victim = inner
                .requests
                .iter()
                .position(|r| matches!(r.status, RequestStatus::Replied | RequestStatus::Expired))
                .unwrap_or(0);
            inner.requests.remove(victim);
        }
        let request = AgentRequest {
            request_id: new_request_id(),
            text,
            source: source.to_string(),
            status: RequestStatus::Pending,
            created_at_unix_ms: now,
            expires_at_unix_ms: now.saturating_add(self.ttl_ms),
            foreground_window,
            claimed_by: None,
            claimed_at_unix_ms: None,
            replies: 0,
            last_reply_at_unix_ms: None,
        };
        inner.requests.push_back(request.clone());
        Ok(request)
    }

    pub fn list(&self, status: Option<RequestStatus>, now: u64) -> Vec<AgentRequest> {
        self.expire(now);
        self.inner
            .lock()
            .unwrap()
            .requests
            .iter()
            .filter(|r| status.map_or(true, |s| r.status == s))
            .cloned()
            .collect()
    }

    pub fn get(&self, id: &str, now: u64) -> Result<AgentRequest, RequestError> {
        self.expire(now);
        self.inner
            .lock()
            .unwrap()
            .requests
            .iter()
            .find(|r| r.request_id == id)
            .cloned()
            .ok_or(RequestError::NotFound)
    }

    /// 领取；`newly` 表示本次调用才从 pending 变为 claimed。
    pub fn claim(
        &self,
        id: &str,
        consumer: &str,
        now: u64,
    ) -> Result<(AgentRequest, bool), RequestError> {
        self.expire(now);
        let mut inner = self.inner.lock().unwrap();
        if inner.closed {
            return Err(RequestError::Closed);
        }
        let r = inner
            .requests
            .iter_mut()
            .find(|r| r.request_id == id)
            .ok_or(RequestError::NotFound)?;
        match r.status {
            RequestStatus::Expired => Err(RequestError::Expired),
            RequestStatus::Pending => {
                r.status = RequestStatus::Claimed;
                r.claimed_by = Some(consumer.to_string());
                r.claimed_at_unix_ms = Some(now);
                Ok((r.clone(), true))
            }
            RequestStatus::Claimed | RequestStatus::Replied => match &r.claimed_by {
                Some(by) if by == consumer => Ok((r.clone(), false)),
                Some(by) => Err(RequestError::AlreadyClaimed { by: by.clone() }),
                // 未领取就直接回复过的请求：允许补领
                None => {
                    r.claimed_by = Some(consumer.to_string());
                    r.claimed_at_unix_ms = Some(now);
                    Ok((r.clone(), true))
                }
            },
        }
    }

    /// 记录一次已交付的回复（可多次：进度 + 最终答复）。过期请求仍可回复，由调用方决定。
    pub fn record_reply(&self, id: &str, now: u64) -> Result<AgentRequest, RequestError> {
        let mut inner = self.inner.lock().unwrap();
        if inner.closed {
            return Err(RequestError::Closed);
        }
        let r = inner
            .requests
            .iter_mut()
            .find(|r| r.request_id == id)
            .ok_or(RequestError::NotFound)?;
        if r.status != RequestStatus::Expired {
            r.status = RequestStatus::Replied;
        }
        r.replies += 1;
        r.last_reply_at_unix_ms = Some(now);
        Ok(r.clone())
    }

    pub fn close(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.closed = true;
        inner.requests.clear();
    }

    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().requests.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn lifecycle_claims_are_exclusive_and_idempotent() {
        let store = AgentRequests::new(1_000, 4);
        let r = store
            .submit("队友在说啥".into(), "assistant-voice", json!(null), 0)
            .unwrap();
        assert_eq!(r.status, RequestStatus::Pending);
        let (c, newly) = store.claim(&r.request_id, "dsh:s1", 10).unwrap();
        assert!(newly);
        assert_eq!(c.status, RequestStatus::Claimed);
        let (_, newly) = store.claim(&r.request_id, "dsh:s1", 11).unwrap();
        assert!(!newly, "同一消费者重复领取幂等");
        assert_eq!(
            store.claim(&r.request_id, "other", 12),
            Err(RequestError::AlreadyClaimed {
                by: "dsh:s1".into()
            })
        );
        let replied = store.record_reply(&r.request_id, 20).unwrap();
        assert_eq!(
            (replied.status, replied.replies),
            (RequestStatus::Replied, 1)
        );
        assert_eq!(store.claim("nope", "x", 0), Err(RequestError::NotFound));
    }

    #[test]
    fn unclaimed_requests_expire_and_capacity_is_bounded() {
        let store = AgentRequests::new(100, 2);
        let a = store.submit("a".into(), "tool", json!(null), 0).unwrap();
        assert_eq!(store.expire(100).len(), 1);
        assert_eq!(
            store.claim(&a.request_id, "x", 100),
            Err(RequestError::Expired)
        );
        assert_eq!(store.list(Some(RequestStatus::Pending), 100).len(), 0);
        store.submit("b".into(), "tool", json!(null), 100).unwrap();
        store.submit("c".into(), "tool", json!(null), 100).unwrap();
        assert_eq!(store.len(), 2, "先回收已结束的请求");
        assert_eq!(store.get(&a.request_id, 100), Err(RequestError::NotFound));
        store.close();
        assert_eq!(
            store.submit("d".into(), "tool", json!(null), 0),
            Err(RequestError::Closed)
        );
    }
}
