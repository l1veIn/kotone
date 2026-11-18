//! 服务资源上限。默认值写入 capabilities，调用方据此规划请求。

use std::time::Duration;

#[derive(Debug, Clone)]
pub struct ServiceLimits {
    /// 单个请求体上限（字节）。
    pub max_request_bytes: usize,
    /// 单张图像像素数上限（捕获与裁剪）。
    pub max_image_pixels: u64,
    /// 同时执行的工具调用数（含仍在工作线程中收尾的超时调用）。
    pub max_concurrent_calls: usize,
    /// 同时保持的 TCP 连接数。
    pub max_connections: usize,
    pub default_timeout_ms: u64,
    pub max_timeout_ms: u64,
    /// 读取请求头的期限（慢速连接防护；也约束 keep-alive 连接两次请求之间的空闲）。
    pub header_read_timeout: Duration,
    /// 读取请求体的期限。
    pub body_read_timeout: Duration,
    /// 连接寿命：到期后连接进入优雅关闭——不再接受新请求，空闲连接立即关闭，
    /// 正在处理的请求继续完成（仍受各自工具期限约束）。
    pub connection_lifetime: Duration,
    /// 优雅关闭后的硬上限（应对不读取响应的客户端）。
    pub connection_grace: Duration,
    /// 空闲维护（回收到期产物）的周期。
    pub maintenance_interval: Duration,
    pub default_ttl_ms: u64,
    pub min_ttl_ms: u64,
    pub max_ttl_ms: u64,
    pub max_artifacts: usize,
    /// 产物存储总字节（RGBA 像素 + PNG）。
    pub max_store_bytes: u64,
    pub max_window_list: usize,
    /// 同时进行的本地 OCR 推理数（每次推理已使用多线程）。
    pub local_ocr_concurrency: usize,
    /// 同时进行的在线 OCR 请求数。
    pub online_ocr_concurrency: usize,
    /// 驻留的 OCR 结果（供 chat.track.ingest 引用）上限与存活时间。
    pub max_ocr_results: usize,
    pub ocr_result_ttl_ms: u64,
    pub track: crate::tracking::TrackLimits,
    /// 事件总线保留的事件数。
    pub max_events: usize,
    /// 同时长轮询事件的等待者上限（不占工具执行名额）。
    pub max_event_waiters: usize,
    /// 用户转交 Agent 的请求有效期与保留数量。
    pub agent_request_ttl_ms: u64,
    pub max_agent_requests: usize,
    /// 已受理、尚未播完的 Agent 回复上限（`agent.reply` 默认不等待交付完成）。
    pub max_pending_replies: usize,
}

impl Default for ServiceLimits {
    fn default() -> Self {
        Self {
            max_request_bytes: 64 * 1024,
            max_image_pixels: 16_777_216,
            max_concurrent_calls: 4,
            max_connections: 16,
            default_timeout_ms: 5_000,
            max_timeout_ms: 30_000,
            header_read_timeout: Duration::from_secs(5),
            body_read_timeout: Duration::from_secs(10),
            connection_lifetime: Duration::from_secs(120),
            // 体读取 10s + 最长工具期限 30s + 写出余量
            connection_grace: Duration::from_secs(60),
            maintenance_interval: Duration::from_secs(5),
            default_ttl_ms: 300_000,
            min_ttl_ms: 1_000,
            max_ttl_ms: 1_800_000,
            max_artifacts: 32,
            max_store_bytes: 256 * 1024 * 1024,
            max_window_list: 256,
            local_ocr_concurrency: 1,
            online_ocr_concurrency: 2,
            max_ocr_results: 64,
            ocr_result_ttl_ms: 600_000,
            track: crate::tracking::TrackLimits::default(),
            max_events: 1000,
            max_event_waiters: 8,
            agent_request_ttl_ms: 600_000,
            max_agent_requests: 64,
            max_pending_replies: 8,
        }
    }
}

impl ServiceLimits {
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "maxRequestBytes": self.max_request_bytes,
            "maxImagePixels": self.max_image_pixels,
            "maxConcurrentCalls": self.max_concurrent_calls,
            "defaultTimeoutMs": self.default_timeout_ms,
            "maxTimeoutMs": self.max_timeout_ms,
            "defaultTtlMs": self.default_ttl_ms,
            "minTtlMs": self.min_ttl_ms,
            "maxTtlMs": self.max_ttl_ms,
            "maxArtifacts": self.max_artifacts,
            "maxStoreBytes": self.max_store_bytes,
            "maxWindowList": self.max_window_list,
            "localOcrConcurrency": self.local_ocr_concurrency,
            "onlineOcrConcurrency": self.online_ocr_concurrency,
            "maxOcrResults": self.max_ocr_results,
            "ocrResultTtlMs": self.ocr_result_ttl_ms,
            "maxPreprocessOperations": crate::preprocess::MAX_OPERATIONS,
            "chatTrack": self.track.to_json(),
            "maxEvents": self.max_events,
            "maxEventWaiters": self.max_event_waiters,
            "agentRequestTtlMs": self.agent_request_ttl_ms,
            "maxAgentRequests": self.max_agent_requests,
            "maxPendingReplies": self.max_pending_replies,
        })
    }
}
