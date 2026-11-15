//! 客户端：通过描述文件直接连接已在运行的桌面实例（不启动 kotone.exe、不启动 ASR）。

use std::path::Path;
use std::time::Duration;

use reqwest::StatusCode;

use kotone_core::tools::ToolHost;

use crate::discovery::{self, Descriptor, DiscoveryError};
use crate::protocol::{
    Envelope, ErrorCode, ToolCallRequest, ToolError, INSTANCE_HEADER, PATH_CAPABILITIES,
    PATH_STATUS, PATH_TOOLS_CALL,
};

/// 客户端接受的最大响应体（服务端 PNG 已受像素上限约束，这里再设一道上限）。
const MAX_RESPONSE_BYTES: usize = 160 * 1024 * 1024;
/// 读取类请求的固定期限。
const GET_TIMEOUT: Duration = Duration::from_secs(10);
/// 工具调用：服务端期限之外留给传输大响应的余量。
const CALL_TRANSPORT_MARGIN: Duration = Duration::from_secs(15);

pub struct ToolClient {
    descriptor: Descriptor,
    base_url: String,
    http: reqwest::Client,
}

impl std::fmt::Debug for ToolClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolClient")
            .field("descriptor", &self.descriptor)
            .finish()
    }
}

pub fn discovery_error(error: DiscoveryError) -> ToolError {
    let code = match &error {
        DiscoveryError::NotRunning { .. } => ErrorCode::ServiceNotRunning,
        DiscoveryError::Insecure(_) => ErrorCode::DescriptorInsecure,
        DiscoveryError::Invalid(_) | DiscoveryError::Io(_) => ErrorCode::DescriptorInvalid,
        DiscoveryError::ProtocolMismatch { .. } => ErrorCode::UnsupportedProtocolVersion,
        DiscoveryError::Stale(_) | DiscoveryError::AlreadyPublished { .. } => {
            ErrorCode::StaleDescriptor
        }
    };
    ToolError::new(code, error.to_string())
}

impl ToolClient {
    /// 从运行目录发现实例并校验描述文件。
    pub fn discover(dir: &Path, host: &dyn ToolHost) -> Result<Self, ToolError> {
        let descriptor = discovery::load_for_client(dir, host).map_err(discovery_error)?;
        Self::from_descriptor(descriptor)
    }

    pub fn from_descriptor(descriptor: Descriptor) -> Result<Self, ToolError> {
        let addr = descriptor.socket_addr().map_err(discovery_error)?;
        let http = reqwest::Client::builder()
            // loopback 请求绝不能经由系统/环境代理转发凭证
            .no_proxy()
            .connect_timeout(Duration::from_secs(3))
            .build()
            .map_err(|e| ToolError::new(ErrorCode::TransportError, e.to_string()))?;
        Ok(Self {
            base_url: format!("http://{addr}"),
            descriptor,
            http,
        })
    }

    pub fn instance_id(&self) -> &str {
        &self.descriptor.instance_id
    }

    pub fn descriptor(&self) -> &Descriptor {
        &self.descriptor
    }

    pub async fn capabilities(&self) -> Result<Envelope, ToolError> {
        self.send(self.http.get(self.url(PATH_CAPABILITIES)), GET_TIMEOUT)
            .await
    }

    pub async fn status(&self) -> Result<Envelope, ToolError> {
        self.send(self.http.get(self.url(PATH_STATUS)), GET_TIMEOUT)
            .await
    }

    pub async fn call(&self, request: &ToolCallRequest) -> Result<Envelope, ToolError> {
        let server_budget = Duration::from_millis(request.timeout_ms.unwrap_or(30_000));
        self.send(
            self.http.post(self.url(PATH_TOOLS_CALL)).json(request),
            server_budget + CALL_TRANSPORT_MARGIN,
        )
        .await
    }

    /// `GET /v1/events` 长轮询。`query` 为已编码的查询串（不含 `?`）；`wait_ms` 用于计算客户端超时。
    pub async fn events(&self, query: &str, wait_ms: u64) -> Result<Envelope, ToolError> {
        let url = if query.is_empty() {
            self.url(crate::protocol::PATH_EVENTS)
        } else {
            format!("{}?{query}", self.url(crate::protocol::PATH_EVENTS))
        };
        self.send(
            self.http.get(url),
            Duration::from_millis(wait_ms) + CALL_TRANSPORT_MARGIN,
        )
        .await
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base_url)
    }

    async fn send(
        &self,
        builder: reqwest::RequestBuilder,
        timeout: Duration,
    ) -> Result<Envelope, ToolError> {
        let response = builder
            .bearer_auth(&self.descriptor.token)
            .timeout(timeout)
            .send()
            .await
            .map_err(transport_error)?;
        let status = response.status();
        let header_instance = response
            .headers()
            .get(INSTANCE_HEADER)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        if header_instance.as_deref() != Some(self.descriptor.instance_id.as_str()) {
            return Err(ToolError::new(
                ErrorCode::InstanceMismatch,
                "响应来自另一个实例或非 Kotone 进程；描述文件可能已过期",
            ));
        }
        let mut response = response;
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(transport_error)? {
            if body.len() + chunk.len() > MAX_RESPONSE_BYTES {
                return Err(ToolError::new(
                    ErrorCode::ResponseInvalid,
                    "响应体超过客户端上限",
                ));
            }
            body.extend_from_slice(&chunk);
        }
        let envelope: Envelope = serde_json::from_slice(&body).map_err(|e| {
            ToolError::new(
                ErrorCode::ResponseInvalid,
                format!("响应不是有效信封（HTTP {status}）: {e}"),
            )
        })?;
        if envelope.instance_id.as_deref() != Some(self.descriptor.instance_id.as_str()) {
            return Err(ToolError::new(
                ErrorCode::InstanceMismatch,
                "信封 instanceId 与描述文件不一致",
            ));
        }
        if status == StatusCode::UNAUTHORIZED {
            // 同一实例拒绝凭证：描述文件被替换过或已损坏
            return Err(ToolError::new(
                ErrorCode::StaleDescriptor,
                "实例拒绝了描述文件中的会话凭证；请重启桌面端后重试",
            ));
        }
        Ok(envelope)
    }
}

fn transport_error(error: reqwest::Error) -> ToolError {
    if error.is_connect() {
        ToolError::new(
            ErrorCode::ServiceUnreachable,
            "描述文件存在但无法连接服务（桌面端可能已退出或崩溃）",
        )
    } else if error.is_timeout() {
        ToolError::new(ErrorCode::Timeout, "等待工具服务响应超时")
    } else {
        ToolError::new(ErrorCode::TransportError, format!("传输失败: {error}"))
    }
}
