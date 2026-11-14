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
    /// 读取请求头的期限（慢速连接防护）。
    pub header_read_timeout: Duration,
    /// 单个连接的总期限。
    pub connection_timeout: Duration,
    pub default_ttl_ms: u64,
    pub min_ttl_ms: u64,
    pub max_ttl_ms: u64,
    pub max_artifacts: usize,
    /// 产物存储总字节（RGBA 像素 + PNG）。
    pub max_store_bytes: u64,
    pub max_window_list: usize,
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
            connection_timeout: Duration::from_secs(120),
            default_ttl_ms: 300_000,
            min_ttl_ms: 1_000,
            max_ttl_ms: 1_800_000,
            max_artifacts: 32,
            max_store_bytes: 256 * 1024 * 1024,
            max_window_list: 256,
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
        })
    }
}
