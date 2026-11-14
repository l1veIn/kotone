//! v1 协议：请求与通用结果信封、错误码、计时。
//!
//! 所有端点（包括认证失败、路由错误）都返回同一种 [`Envelope`]；工具执行层面的
//! 成功/失败用 HTTP 200 + `ok` 字段表达，传输/认证/请求格式问题使用对应 HTTP 状态码。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 协议名（信封 `protocol` 字段）。
pub const PROTOCOL: &str = "kotone.tools";
/// 协议主版本；不兼容变化会提升该值并改变路由前缀。
pub const PROTOCOL_VERSION: u32 = 1;
/// 响应头：服务实例 ID（客户端据此确认没有连到别的进程）。
pub const INSTANCE_HEADER: &str = "x-kotone-instance-id";
/// 可选请求头：GET 端点的请求 ID。
pub const REQUEST_ID_HEADER: &str = "x-kotone-request-id";

pub const PATH_CAPABILITIES: &str = "/v1/capabilities";
pub const PATH_STATUS: &str = "/v1/status";
pub const PATH_TOOLS_CALL: &str = "/v1/tools/call";

/// `POST /v1/tools/call` 请求体。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ToolCallRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    pub tool: String,
    #[serde(default)]
    pub arguments: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
}

/// 通用结果信封。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Envelope {
    pub protocol: String,
    pub protocol_version: u32,
    pub request_id: String,
    /// 服务端总会填写；客户端在连接前就失败时为 null。
    pub instance_id: Option<String>,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ToolError>,
    pub timing: Timing,
}

impl Envelope {
    pub fn success(
        request_id: String,
        instance_id: Option<String>,
        tool: Option<String>,
        result: Value,
        timing: Timing,
    ) -> Self {
        Self {
            protocol: PROTOCOL.into(),
            protocol_version: PROTOCOL_VERSION,
            request_id,
            instance_id,
            ok: true,
            tool,
            result: Some(result),
            error: None,
            timing,
        }
    }

    pub fn failure(
        request_id: String,
        instance_id: Option<String>,
        tool: Option<String>,
        error: ToolError,
        timing: Timing,
    ) -> Self {
        Self {
            protocol: PROTOCOL.into(),
            protocol_version: PROTOCOL_VERSION,
            request_id,
            instance_id,
            ok: false,
            tool,
            result: None,
            error: Some(error),
            timing,
        }
    }
}

/// 计时（毫秒）。`totalMs` 为服务端处理总耗时；其余为可选分阶段耗时，
/// 如 `captureMs` / `cropMs` / `encodeMs` / `queueMs`。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Timing {
    pub total_ms: u64,
    #[serde(flatten)]
    pub phases: BTreeMap<String, u64>,
}

/// 错误体：至少包含 code / message / retryable。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ToolError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

impl ToolError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code: code.as_str().into(),
            message: message.into(),
            retryable: code.retryable(),
            details: None,
        }
    }

    pub fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }
}

/// 稳定错误码。新增码只追加；客户端应把未知码按 `retryable` 字段处理。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    // ---- 传输 / 认证 / 请求格式（服务端） ----
    Unauthorized,
    ForbiddenOrigin,
    RouteNotFound,
    MethodNotAllowed,
    PayloadTooLarge,
    UnsupportedMediaType,
    InvalidRequest,
    // ---- 工具调用 ----
    ToolNotFound,
    InvalidArguments,
    UnsupportedProtocolVersion,
    Busy,
    Timeout,
    Internal,
    PlatformUnsupported,
    // ---- 窗口 / 捕获 ----
    WindowNotFound,
    WindowGone,
    WindowIdentityChanged,
    WindowMinimized,
    WindowNotVisible,
    WindowNotForeground,
    InvalidGeometry,
    ImageTooLarge,
    TargetChanged,
    CaptureFailed,
    // ---- 产物 ----
    ArtifactNotFound,
    ArtifactExpired,
    ArtifactReleased,
    // ---- 客户端发现 / 连接 ----
    ServiceNotRunning,
    DescriptorInsecure,
    DescriptorInvalid,
    StaleDescriptor,
    ServiceUnreachable,
    InstanceMismatch,
    TransportError,
    ResponseInvalid,
}

impl ErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unauthorized => "UNAUTHORIZED",
            Self::ForbiddenOrigin => "FORBIDDEN_ORIGIN",
            Self::RouteNotFound => "ROUTE_NOT_FOUND",
            Self::MethodNotAllowed => "METHOD_NOT_ALLOWED",
            Self::PayloadTooLarge => "PAYLOAD_TOO_LARGE",
            Self::UnsupportedMediaType => "UNSUPPORTED_MEDIA_TYPE",
            Self::InvalidRequest => "INVALID_REQUEST",
            Self::ToolNotFound => "TOOL_NOT_FOUND",
            Self::InvalidArguments => "INVALID_ARGUMENTS",
            Self::UnsupportedProtocolVersion => "UNSUPPORTED_PROTOCOL_VERSION",
            Self::Busy => "BUSY",
            Self::Timeout => "TIMEOUT",
            Self::Internal => "INTERNAL",
            Self::PlatformUnsupported => "PLATFORM_UNSUPPORTED",
            Self::WindowNotFound => "WINDOW_NOT_FOUND",
            Self::WindowGone => "WINDOW_GONE",
            Self::WindowIdentityChanged => "WINDOW_IDENTITY_CHANGED",
            Self::WindowMinimized => "WINDOW_MINIMIZED",
            Self::WindowNotVisible => "WINDOW_NOT_VISIBLE",
            Self::WindowNotForeground => "WINDOW_NOT_FOREGROUND",
            Self::InvalidGeometry => "INVALID_GEOMETRY",
            Self::ImageTooLarge => "IMAGE_TOO_LARGE",
            Self::TargetChanged => "TARGET_CHANGED",
            Self::CaptureFailed => "CAPTURE_FAILED",
            Self::ArtifactNotFound => "ARTIFACT_NOT_FOUND",
            Self::ArtifactExpired => "ARTIFACT_EXPIRED",
            Self::ArtifactReleased => "ARTIFACT_RELEASED",
            Self::ServiceNotRunning => "SERVICE_NOT_RUNNING",
            Self::DescriptorInsecure => "DESCRIPTOR_INSECURE",
            Self::DescriptorInvalid => "DESCRIPTOR_INVALID",
            Self::StaleDescriptor => "STALE_DESCRIPTOR",
            Self::ServiceUnreachable => "SERVICE_UNREACHABLE",
            Self::InstanceMismatch => "INSTANCE_MISMATCH",
            Self::TransportError => "TRANSPORT_ERROR",
            Self::ResponseInvalid => "RESPONSE_INVALID",
        }
    }

    /// 同样的请求稍后重试是否可能成功。
    pub fn retryable(self) -> bool {
        matches!(
            self,
            Self::Busy
                | Self::Timeout
                | Self::WindowNotForeground
                | Self::WindowMinimized
                | Self::WindowNotVisible
                | Self::TargetChanged
                | Self::CaptureFailed
                | Self::ServiceUnreachable
                | Self::TransportError
        )
    }

    /// 全部错误码（文档与测试核对用）。
    pub const ALL: &'static [ErrorCode] = &[
        Self::Unauthorized,
        Self::ForbiddenOrigin,
        Self::RouteNotFound,
        Self::MethodNotAllowed,
        Self::PayloadTooLarge,
        Self::UnsupportedMediaType,
        Self::InvalidRequest,
        Self::ToolNotFound,
        Self::InvalidArguments,
        Self::UnsupportedProtocolVersion,
        Self::Busy,
        Self::Timeout,
        Self::Internal,
        Self::PlatformUnsupported,
        Self::WindowNotFound,
        Self::WindowGone,
        Self::WindowIdentityChanged,
        Self::WindowMinimized,
        Self::WindowNotVisible,
        Self::WindowNotForeground,
        Self::InvalidGeometry,
        Self::ImageTooLarge,
        Self::TargetChanged,
        Self::CaptureFailed,
        Self::ArtifactNotFound,
        Self::ArtifactExpired,
        Self::ArtifactReleased,
        Self::ServiceNotRunning,
        Self::DescriptorInsecure,
        Self::DescriptorInvalid,
        Self::StaleDescriptor,
        Self::ServiceUnreachable,
        Self::InstanceMismatch,
        Self::TransportError,
        Self::ResponseInvalid,
    ];
}

/// requestId 只接受短的可打印标识，避免把任意内容回显进日志或输出。
pub fn is_valid_request_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b':'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_codes_are_unique_screaming_snake() {
        let mut seen = std::collections::HashSet::new();
        for code in ErrorCode::ALL {
            let s = code.as_str();
            assert!(seen.insert(s), "duplicate {s}");
            assert!(
                s.bytes().all(|b| b.is_ascii_uppercase() || b == b'_'),
                "{s}"
            );
        }
    }

    #[test]
    fn request_validation() {
        assert!(is_valid_request_id("req-123"));
        assert!(!is_valid_request_id(""));
        assert!(!is_valid_request_id("has space"));
        assert!(!is_valid_request_id(&"a".repeat(129)));
    }

    #[test]
    fn request_rejects_unknown_fields() {
        let parsed: Result<ToolCallRequest, _> =
            serde_json::from_str(r#"{"tool":"system.status","path":"C:/x"}"#);
        assert!(parsed.is_err());
        let parsed: ToolCallRequest = serde_json::from_str(r#"{"tool":"system.status"}"#).unwrap();
        assert_eq!(parsed.arguments, Value::Null);
    }

    #[test]
    fn timing_flattens_phases() {
        let mut timing = Timing {
            total_ms: 12,
            ..Default::default()
        };
        timing.phases.insert("captureMs".into(), 7);
        let json = serde_json::to_value(&timing).unwrap();
        assert_eq!(json, serde_json::json!({"totalMs": 12, "captureMs": 7}));
    }
}
