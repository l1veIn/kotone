//! 已实现工具的目录与 JSON Schema。
//!
//! 只列出本服务真实实现并可调用的工具；路线图中的 OCR、消息跟踪、热键、工作流、
//! TTS/弹幕、发送等工具不在此处出现，调用它们返回 `TOOL_NOT_FOUND`。

use serde_json::{json, Value};

use crate::limits::ServiceLimits;

pub const TOOL_SYSTEM_CAPABILITIES: &str = "system.capabilities";
pub const TOOL_SYSTEM_STATUS: &str = "system.status";
pub const TOOL_WINDOW_LIST: &str = "window.list";
pub const TOOL_WINDOW_ACTIVE: &str = "window.active";
pub const TOOL_SCREEN_CAPTURE: &str = "screen.capture";
pub const TOOL_IMAGE_CROP: &str = "image.crop";
pub const TOOL_ARTIFACT_GET: &str = "artifact.get";
pub const TOOL_ARTIFACT_RELEASE: &str = "artifact.release";

/// 本版本实现的全部工具名（顺序即 capabilities 中的顺序）。
pub const IMPLEMENTED_TOOLS: &[&str] = &[
    TOOL_SYSTEM_CAPABILITIES,
    TOOL_SYSTEM_STATUS,
    TOOL_WINDOW_LIST,
    TOOL_WINDOW_ACTIVE,
    TOOL_SCREEN_CAPTURE,
    TOOL_IMAGE_CROP,
    TOOL_ARTIFACT_GET,
    TOOL_ARTIFACT_RELEASE,
];

/// 平台/后端就绪情况，决定窗口与捕获工具的 `ready` 字段。
#[derive(Debug, Clone)]
pub struct Readiness {
    pub windows_ready: bool,
    pub capture_ready: bool,
    pub reason: Option<String>,
}

fn nullable(schema_type: &str) -> Value {
    json!([schema_type, "null"])
}

fn rect_schema(description: &str) -> Value {
    json!({
        "type": "object",
        "description": description,
        "required": ["x", "y", "width", "height"],
        "additionalProperties": false,
        "properties": {
            "x": {"type": "integer"},
            "y": {"type": "integer"},
            "width": {"type": "integer", "minimum": 0},
            "height": {"type": "integer", "minimum": 0}
        }
    })
}

fn nullable_rect_schema(description: &str) -> Value {
    let mut schema = rect_schema(description);
    schema["type"] = nullable("object");
    schema
}

fn warning_schema() -> Value {
    json!({
        "type": "object",
        "required": ["code", "message"],
        "additionalProperties": false,
        "properties": {
            "code": {"type": "string"},
            "message": {"type": "string"}
        }
    })
}

fn artifact_id_schema() -> Value {
    json!({"type": "string", "minLength": 1, "maxLength": 64, "description": "由本服务返回的不透明产物 ID；不接受文件路径"})
}

fn window_id_schema() -> Value {
    json!({"type": "string", "minLength": 1, "maxLength": 64, "description": "window.list / window.active 返回的不透明窗口引用"})
}

fn ttl_schema(limits: &ServiceLimits) -> Value {
    json!({
        "type": "integer",
        "minimum": limits.min_ttl_ms,
        "maximum": limits.max_ttl_ms,
        "description": format!("产物存活毫秒数，缺省 {}", limits.default_ttl_ms)
    })
}

pub fn window_schema() -> Value {
    json!({
        "type": "object",
        "required": [
            "windowId", "title", "className", "processName", "pid", "isForeground",
            "isVisible", "isMinimized", "isCloaked", "clientRect", "monitorRect", "dpi",
            "scale", "coordinateSpace", "identityBinding"
        ],
        "additionalProperties": false,
        "properties": {
            "windowId": window_id_schema(),
            "title": {"type": "string"},
            "className": {"type": "string"},
            "processName": {"type": nullable("string"), "description": "可执行文件名，不含目录"},
            "pid": {"type": "integer"},
            "isForeground": {"type": "boolean"},
            "isVisible": {"type": "boolean"},
            "isMinimized": {"type": "boolean"},
            "isCloaked": {"type": "boolean"},
            "clientRect": nullable_rect_schema("客户区的屏幕物理像素坐标；最小化时为 null"),
            "monitorRect": nullable_rect_schema("所在显示器的屏幕物理像素范围"),
            "dpi": {"type": nullable("integer")},
            "scale": {"type": nullable("number"), "description": "dpi / 96"},
            "coordinateSpace": {"const": "screen-physical-pixels"},
            "identityBinding": {"enum": ["handle+pid+process-start-time", "handle+pid"]}
        }
    })
}

pub fn artifact_schema(extra_required: &[&str], extra_properties: Value) -> Value {
    let mut required = vec![
        "artifactId",
        "kind",
        "mimeType",
        "width",
        "height",
        "byteLength",
        "sha256",
        "createdAtUnixMs",
        "expiresAtUnixMs",
        "ttlMs",
        "parentArtifactId",
        "rootArtifactId",
        "coordinateSpace",
        "rootTransform",
        "rootRect",
        "screenRect",
        "source",
    ];
    required.extend_from_slice(extra_required);
    let mut properties = json!({
        "artifactId": artifact_id_schema(),
        "kind": {"enum": ["capture", "crop"]},
        "mimeType": {"const": "image/png"},
        "width": {"type": "integer", "minimum": 1},
        "height": {"type": "integer", "minimum": 1},
        "byteLength": {"type": "integer", "minimum": 1, "description": "PNG 字节数"},
        "sha256": {"type": "string", "description": "PNG 字节的 SHA-256 十六进制"},
        "createdAtUnixMs": {"type": "integer"},
        "expiresAtUnixMs": {"type": "integer"},
        "ttlMs": {"type": "integer"},
        "parentArtifactId": {"type": nullable("string")},
        "rootArtifactId": {"type": "string"},
        "coordinateSpace": {"const": "artifact-physical-pixels"},
        "rootTransform": {
            "type": "object",
            "description": "rootPoint = offset + scale * point（M1 只有裁剪，scale 恒为 1）",
            "required": ["offsetX", "offsetY", "scaleX", "scaleY"],
            "additionalProperties": false,
            "properties": {
                "offsetX": {"type": "integer"},
                "offsetY": {"type": "integer"},
                "scaleX": {"type": "number"},
                "scaleY": {"type": "number"}
            }
        },
        "rootRect": rect_schema("本产物在根产物中的位置（根产物物理像素）"),
        "screenRect": rect_schema("本产物在采集时刻的屏幕物理像素位置"),
        "source": {
            "type": "object",
            "required": [
                "frameId", "windowId", "captureMethod", "clientRectScreen", "dpi",
                "capturedAtUnixMs", "processName", "warnings"
            ],
            "additionalProperties": false,
            "properties": {
                "frameId": {"type": "string", "description": "一次采集的标识；不同帧的裁剪坐标不可混用"},
                "windowId": window_id_schema(),
                "captureMethod": {"type": "string"},
                "clientRectScreen": rect_schema("采集时客户区的屏幕物理像素坐标"),
                "dpi": {"type": nullable("integer")},
                "capturedAtUnixMs": {"type": "integer"},
                "processName": {"type": nullable("string")},
                "warnings": {"type": "array", "items": warning_schema()}
            }
        }
    });
    if let (Some(base), Some(extra)) = (properties.as_object_mut(), extra_properties.as_object()) {
        for (k, v) in extra {
            base.insert(k.clone(), v.clone());
        }
    }
    json!({
        "type": "object",
        "required": required,
        "additionalProperties": false,
        "properties": properties
    })
}

fn empty_args() -> Value {
    json!({"type": ["object", "null"], "additionalProperties": false, "properties": {}})
}

fn spec(
    name: &str,
    description: &str,
    side_effects: &[&str],
    ready: bool,
    reason: Option<&String>,
    input: Value,
    output: Value,
) -> Value {
    json!({
        "name": name,
        "version": 1,
        "status": "implemented",
        "description": description,
        "sideEffects": side_effects,
        "dataEgress": "none",
        "ready": ready,
        "notReadyReason": if ready { Value::Null } else { json!(reason) },
        "inputSchema": input,
        "outputSchema": output,
    })
}

/// 生成 capabilities 中的 `tools` 数组。
pub fn tool_specs(limits: &ServiceLimits, readiness: &Readiness) -> Vec<Value> {
    let reason = readiness.reason.as_ref();
    vec![
        spec(
            TOOL_SYSTEM_CAPABILITIES,
            "Return the tools this desktop instance actually implements, their schemas, readiness and limits.",
            &[],
            true,
            None,
            json!({
                "type": ["object", "null"],
                "additionalProperties": false,
                "properties": {"protocolVersion": {"type": "integer", "description": "期望的协议主版本；不匹配时返回 UNSUPPORTED_PROTOCOL_VERSION"}}
            }),
            capabilities_schema(),
        ),
        spec(
            TOOL_SYSTEM_STATUS,
            "Report the desktop instance, tool-service state, ASR runtime phase and artifact store usage. Never includes credentials.",
            &[],
            true,
            None,
            empty_args(),
            status_schema(),
        ),
        spec(
            TOOL_WINDOW_LIST,
            "List visible top-level windows with opaque references, process name, foreground/minimized flags and physical client rectangle/DPI. Read-only; does not change focus.",
            &["registers-window-references"],
            readiness.windows_ready,
            reason,
            json!({
                "type": ["object", "null"],
                "additionalProperties": false,
                "properties": {
                    "processName": {"type": "string", "description": "按可执行文件名精确匹配（不区分大小写）"},
                    "titleContains": {"type": "string", "description": "标题包含（不区分大小写）"},
                    "includeUntitled": {"type": "boolean", "default": false},
                    "limit": {"type": "integer", "minimum": 1, "maximum": limits.max_window_list, "default": 64}
                }
            }),
            json!({
                "type": "object",
                "required": ["windows", "matched", "truncated"],
                "additionalProperties": false,
                "properties": {
                    "windows": {"type": "array", "items": window_schema()},
                    "matched": {"type": "integer"},
                    "truncated": {"type": "boolean"}
                }
            }),
        ),
        spec(
            TOOL_WINDOW_ACTIVE,
            "Return the current foreground window, or null with reason NO_FOREGROUND_WINDOW. Read-only.",
            &["registers-window-references"],
            readiness.windows_ready,
            reason,
            empty_args(),
            json!({
                "type": "object",
                "required": ["window", "reason"],
                "additionalProperties": false,
                "properties": {
                    "window": {"oneOf": [window_schema(), {"type": "null"}]},
                    "reason": {"type": nullable("string")}
                }
            }),
        ),
        spec(
            TOOL_SCREEN_CAPTURE,
            "Capture the on-screen pixels of the target window's client area while it is the foreground window (GDI foreground visible client capture). Rejects non-foreground, minimized, hidden, cloaked or off-monitor targets instead of focusing them or capturing the desktop. See capture.limitations.",
            &["creates-artifact"],
            readiness.capture_ready,
            reason,
            json!({
                "type": "object",
                "required": ["windowId"],
                "additionalProperties": false,
                "properties": {"windowId": window_id_schema(), "ttlMs": ttl_schema(limits)}
            }),
            artifact_schema(
                &["window", "evictedArtifactIds"],
                json!({
                    "window": window_schema(),
                    "evictedArtifactIds": {"type": "array", "items": {"type": "string"}}
                }),
            ),
        ),
        spec(
            TOOL_IMAGE_CROP,
            "Create a new immutable artifact from a rectangle of an existing artifact (input artifact physical pixels, top-left origin). The input artifact is never modified. Out-of-range, negative, empty or overflowing rectangles are rejected, not clamped.",
            &["creates-artifact"],
            true,
            None,
            json!({
                "type": "object",
                "required": ["artifactId", "rect"],
                "additionalProperties": false,
                "properties": {
                    "artifactId": artifact_id_schema(),
                    "rect": rect_schema("裁剪矩形（输入产物物理像素）"),
                    "ttlMs": ttl_schema(limits)
                }
            }),
            artifact_schema(
                &["evictedArtifactIds"],
                json!({"evictedArtifactIds": {"type": "array", "items": {"type": "string"}}}),
            ),
        ),
        spec(
            TOOL_ARTIFACT_GET,
            "Return artifact metadata and, unless includeContent is false, the PNG bytes as base64. Only artifact IDs are accepted; no file paths are read or written by the service.",
            &[],
            true,
            None,
            json!({
                "type": "object",
                "required": ["artifactId"],
                "additionalProperties": false,
                "properties": {
                    "artifactId": artifact_id_schema(),
                    "includeContent": {"type": "boolean", "default": true}
                }
            }),
            artifact_schema(
                &[],
                json!({
                    "content": {
                        "type": "object",
                        "required": ["encoding", "mimeType", "data"],
                        "additionalProperties": false,
                        "properties": {
                            "encoding": {"const": "base64"},
                            "mimeType": {"const": "image/png"},
                            "data": {"type": "string"}
                        }
                    }
                }),
            ),
        ),
        spec(
            TOOL_ARTIFACT_RELEASE,
            "Release an artifact. Idempotent: releasing an already released, expired or evicted artifact succeeds with the corresponding status; never-issued IDs return ARTIFACT_NOT_FOUND.",
            &["releases-artifact"],
            true,
            None,
            json!({
                "type": "object",
                "required": ["artifactId"],
                "additionalProperties": false,
                "properties": {"artifactId": artifact_id_schema()}
            }),
            json!({
                "type": "object",
                "required": ["artifactId", "status"],
                "additionalProperties": false,
                "properties": {
                    "artifactId": {"type": "string"},
                    "status": {"enum": ["released", "already-released", "already-expired", "already-evicted"]}
                }
            }),
        ),
    ]
}

pub fn capabilities_schema() -> Value {
    json!({
        "type": "object",
        "required": [
            "protocol", "protocolVersion", "instanceId", "appVersion", "platform",
            "endpoints", "capture", "limits", "tools"
        ],
        "additionalProperties": false,
        "properties": {
            "protocol": {"const": "kotone.tools"},
            "protocolVersion": {"type": "integer"},
            "instanceId": {"type": "string"},
            "appVersion": {"type": "string"},
            "platform": {"type": "string"},
            "endpoints": {"type": "array", "items": {"type": "string"}},
            "capture": {
                "type": "object",
                "required": ["method", "available", "unavailableReason", "limitations"],
                "additionalProperties": false,
                "properties": {
                    "method": {"type": "string"},
                    "available": {"type": "boolean"},
                    "unavailableReason": {"type": nullable("string")},
                    "limitations": {"type": "array", "items": {"type": "string"}}
                }
            },
            "limits": {"type": "object"},
            "tools": {"type": "array", "items": {
                "type": "object",
                "required": ["name", "version", "status", "description", "sideEffects", "dataEgress", "ready", "notReadyReason", "inputSchema", "outputSchema"],
                "additionalProperties": false,
                "properties": {
                    "name": {"type": "string"},
                    "version": {"type": "integer"},
                    "status": {"const": "implemented"},
                    "description": {"type": "string"},
                    "sideEffects": {"type": "array", "items": {"type": "string"}},
                    "dataEgress": {"const": "none"},
                    "ready": {"type": "boolean"},
                    "notReadyReason": {"type": nullable("string")},
                    "inputSchema": {"type": "object"},
                    "outputSchema": {"type": "object"}
                }
            }}
        }
    })
}

pub fn status_schema() -> Value {
    json!({
        "type": "object",
        "required": [
            "instanceId", "appVersion", "protocol", "protocolVersion", "platform", "service",
            "asr", "capture", "artifacts", "activeCalls"
        ],
        "additionalProperties": false,
        "properties": {
            "instanceId": {"type": "string"},
            "appVersion": {"type": "string"},
            "protocol": {"const": "kotone.tools"},
            "protocolVersion": {"type": "integer"},
            "platform": {"type": "string"},
            "service": {
                "type": "object",
                "required": ["state", "startedAtUnixMs", "uptimeMs"],
                "additionalProperties": false,
                "properties": {
                    "state": {"const": "running"},
                    "startedAtUnixMs": {"type": "integer"},
                    "uptimeMs": {"type": "integer"}
                }
            },
            "asr": {
                "type": "object",
                "required": ["runtimePhase", "requiredForTools"],
                "additionalProperties": false,
                "properties": {
                    "runtimePhase": {"enum": ["stopped", "starting", "running", "stopping", "unknown"]},
                    "requiredForTools": {"const": false}
                }
            },
            "capture": {
                "type": "object",
                "required": ["method", "available"],
                "additionalProperties": false,
                "properties": {"method": {"type": "string"}, "available": {"type": "boolean"}}
            },
            "artifacts": {
                "type": "object",
                "required": ["count", "totalBytes", "maxArtifacts", "maxStoreBytes"],
                "additionalProperties": false,
                "properties": {
                    "count": {"type": "integer"},
                    "totalBytes": {"type": "integer"},
                    "maxArtifacts": {"type": "integer"},
                    "maxStoreBytes": {"type": "integer"}
                }
            },
            "activeCalls": {"type": "integer"}
        }
    })
}
