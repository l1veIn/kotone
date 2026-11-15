//! DeepSeek 在线 OCR 后端（`POST {base}/v1/tools/ocr`，multipart：`file` + `purpose`）。
//!
//! 依据 2026-11-15 核对的官方文档 <https://api-docs.deepseek.com/zh-cn/guides/ocr/>：
//! - 识别开始后状态码已是 200；失败/超时时响应体是 `{"error":{...}}` 而不是结果，
//!   因此 HTTP 200 也要检查 `error`；
//! - 结果 `object="ocr.result"`，`pages[].finish_reason="length"` 表示该页文本被截断；
//! - 逐页识别的文本可能带 `<｜ref｜>…<｜/ref｜><｜box｜>[[x1,y1,x2,y2]]<｜/box｜>`，
//!   坐标按页面宽高归一化到 0–1000；没有标记时没有几何；
//! - 客户端断开不会取消远端识别（仍计费），账号并发上限 15。
//!
//! 连接与 API key 复用 kotone 连接目录与系统凭据库；只上传调用方指定产物的 PNG 字节。
//! 本后端不提供置信度（null），也不会在失败时改走本地或其他后端。

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use kotone_core::connection::ResolvedConnection;
use kotone_core::ocr::{
    OcrBackend, OcrBackendInfo, OcrBackendKind, OcrCancel, OcrError, OcrGranularity, OcrInput,
    OcrObservation, OcrOutput, OcrParams, OcrRect, OcrWarning,
};

pub const BACKEND_ID: &str = "deepseek-ocr";
pub const API_DOC: &str = "https://api-docs.deepseek.com/zh-cn/guides/ocr/";
const MAX_UPLOAD_BYTES: usize = 100 * 1024 * 1024;
const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
const MAX_REMOTE_MESSAGE: usize = 300;
const PURPOSES: &[&str] = &["text_recognition", "rich_layout_recognition"];

/// 解析当前配置的连接：`Ok(None)` 表示用户未启用在线 OCR；`Err` 为解析失败原因。
pub type ConnectionSource =
    Arc<dyn Fn() -> Result<Option<ResolvedConnection>, String> + Send + Sync>;

pub struct DeepSeekOcr {
    source: ConnectionSource,
}

impl DeepSeekOcr {
    pub fn new(source: ConnectionSource) -> Self {
        Self { source }
    }
}

/// 由连接 base URL 推导 OCR 端点。只接受 https，或用于测试的 loopback http。
pub fn ocr_endpoint(base_url: &str) -> Result<reqwest::Url, String> {
    let url = reqwest::Url::parse(base_url.trim())
        .map_err(|_| "连接的接口地址不是有效 URL".to_string())?;
    let loopback = matches!(
        url.host_str(),
        Some("127.0.0.1") | Some("localhost") | Some("[::1]")
    );
    match url.scheme() {
        "https" => {}
        "http" if loopback => {}
        _ => return Err("在线 OCR 只允许 https 接口地址".into()),
    }
    let mut path = url.path().trim_end_matches('/').to_string();
    if let Some(stripped) = path.strip_suffix("/v1") {
        path = stripped.to_string();
    }
    let mut out = url.clone();
    out.set_path(&format!("{path}/v1/tools/ocr"));
    out.set_query(None);
    out.set_fragment(None);
    Ok(out)
}

fn is_loopback(url: &reqwest::Url) -> bool {
    matches!(
        url.host_str(),
        Some("127.0.0.1") | Some("localhost") | Some("[::1]")
    )
}

pub fn options_schema() -> Value {
    json!({
        "type": ["object", "null"],
        "additionalProperties": false,
        "properties": {
            "purpose": {"enum": PURPOSES, "default": "text_recognition",
                "description": "text_recognition：纯文字，通常没有几何；rich_layout_recognition：带 <｜ref｜>/<｜box｜> 版面块（block 粒度、0–1000 归一化坐标）"}
        }
    })
}

fn parse_purpose(options: &Value) -> Result<&'static str, OcrError> {
    let map = match options {
        Value::Null => return Ok(PURPOSES[0]),
        Value::Object(m) => m,
        _ => {
            return Err(OcrError::UnsupportedOption {
                option: "options".into(),
                message: "options 必须是对象".into(),
            })
        }
    };
    let mut purpose = PURPOSES[0];
    for (key, value) in map {
        match key.as_str() {
            "purpose" => {
                purpose = PURPOSES
                    .iter()
                    .copied()
                    .find(|p| value.as_str() == Some(p))
                    .ok_or_else(|| OcrError::UnsupportedOption {
                        option: key.clone(),
                        message: format!("purpose 只接受 {}", PURPOSES.join(" / ")),
                    })?;
            }
            other => {
                return Err(OcrError::UnsupportedOption {
                    option: other.into(),
                    message: format!("{BACKEND_ID} 只支持选项 purpose"),
                })
            }
        }
    }
    Ok(purpose)
}

enum Resolved {
    NotConfigured,
    Error(String),
    MissingKey(ResolvedConnection),
    InvalidEndpoint(ResolvedConnection, String),
    Ready(ResolvedConnection, reqwest::Url, String),
}

impl DeepSeekOcr {
    fn resolve(&self) -> Resolved {
        match (self.source)() {
            Ok(None) => Resolved::NotConfigured,
            Err(e) => Resolved::Error(e),
            Ok(Some(conn)) => {
                let key = conn
                    .api_key
                    .as_deref()
                    .map(str::trim)
                    .filter(|k| !k.is_empty())
                    .map(str::to_string);
                match (key, ocr_endpoint(&conn.base_url)) {
                    (_, Err(e)) => Resolved::InvalidEndpoint(conn, e),
                    (None, _) => Resolved::MissingKey(conn),
                    (Some(key), Ok(url)) => Resolved::Ready(conn, url, key),
                }
            }
        }
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max).collect::<String>())
    }
}

/// 文本里的一个版面块。
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedBlock {
    pub label: Option<String>,
    /// 0–1000 归一化 [x1, y1, x2, y2]。
    pub boxes: Vec<[f64; 4]>,
    pub text: String,
}

const REF_OPEN: &str = "<｜ref｜>";
const REF_CLOSE: &str = "<｜/ref｜>";
const BOX_OPEN: &str = "<｜box｜>";
const BOX_CLOSE: &str = "<｜/box｜>";

/// 去掉残留的 `<｜…｜>` 特殊标记。
fn strip_tokens(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find("<｜") {
        out.push_str(&rest[..start]);
        match rest[start..].find("｜>") {
            Some(end) => rest = &rest[start + end + "｜>".len()..],
            None => {
                rest = &rest[start..];
                break;
            }
        }
    }
    out.push_str(rest);
    out
}

/// 解析带 ref/box 标记的页面文本。返回 (块, 是否出现了标记, 无效框数量)。
pub fn parse_marked_text(text: &str) -> (Vec<ParsedBlock>, bool, usize) {
    if !text.contains(REF_OPEN) && !text.contains(BOX_OPEN) {
        return (Vec::new(), false, 0);
    }
    let mut blocks = Vec::new();
    let mut invalid = 0usize;
    let mut rest = text;
    // 第一个标记之前的前导文字：无几何
    let first = rest
        .find(REF_OPEN)
        .or_else(|| rest.find(BOX_OPEN))
        .unwrap_or(rest.len());
    let preamble = strip_tokens(&rest[..first]).trim().to_string();
    if !preamble.is_empty() {
        blocks.push(ParsedBlock {
            label: None,
            boxes: Vec::new(),
            text: preamble,
        });
    }
    rest = &rest[first..];
    while !rest.is_empty() {
        let mut label = None;
        if let Some(after) = rest.strip_prefix(REF_OPEN) {
            match after.find(REF_CLOSE) {
                Some(end) => {
                    label = Some(after[..end].trim().to_string());
                    rest = &after[end + REF_CLOSE.len()..];
                }
                None => {
                    rest = after;
                }
            }
        }
        let mut boxes = Vec::new();
        let trimmed = rest.trim_start();
        if let Some(after) = trimmed.strip_prefix(BOX_OPEN) {
            match after.find(BOX_CLOSE) {
                Some(end) => {
                    match serde_json::from_str::<Vec<Vec<f64>>>(after[..end].trim()) {
                        Ok(list) => {
                            for b in list {
                                let valid = b.len() == 4
                                    && b.iter()
                                        .all(|v| v.is_finite() && (0.0..=1000.0).contains(v))
                                    && b[2] >= b[0]
                                    && b[3] >= b[1];
                                if valid {
                                    boxes.push([b[0], b[1], b[2], b[3]]);
                                } else {
                                    invalid += 1;
                                }
                            }
                        }
                        Err(_) => invalid += 1,
                    }
                    rest = &after[end + BOX_CLOSE.len()..];
                }
                None => {
                    invalid += 1;
                    rest = after;
                }
            }
        }
        let next = rest.find(REF_OPEN).unwrap_or(rest.len());
        let content = strip_tokens(&rest[..next]).trim().to_string();
        rest = &rest[next..];
        // 内容为空时（grounding 形式）ref 本身就是文字；否则 ref 是类别
        let (label, text) = match (label, content.is_empty()) {
            (Some(l), true) => (None, l),
            (l, _) => (l, content),
        };
        if text.is_empty() && boxes.is_empty() {
            continue;
        }
        blocks.push(ParsedBlock { label, boxes, text });
    }
    (blocks, true, invalid)
}

fn union_px(boxes: &[[f64; 4]], width: u32, height: u32) -> Option<OcrRect> {
    if boxes.is_empty() {
        return None;
    }
    let (w, h) = (f64::from(width), f64::from(height));
    let x0 = boxes.iter().map(|b| b[0]).fold(f64::MAX, f64::min) / 1000.0 * w;
    let y0 = boxes.iter().map(|b| b[1]).fold(f64::MAX, f64::min) / 1000.0 * h;
    let x1 = boxes.iter().map(|b| b[2]).fold(f64::MIN, f64::max) / 1000.0 * w;
    let y1 = boxes.iter().map(|b| b[3]).fold(f64::MIN, f64::max) / 1000.0 * h;
    Some(OcrRect {
        x: x0,
        y: y0,
        width: (x1 - x0).max(0.0),
        height: (y1 - y0).max(0.0),
    })
}

/// 把一页文本转成观测：有 box 标记 → block（含块内行、无逐行几何）；无标记 → 逐行、无几何。
pub fn page_observations(
    text: &str,
    width: u32,
    height: u32,
    warnings: &mut Vec<OcrWarning>,
) -> Vec<OcrObservation> {
    let (blocks, marked, invalid) = parse_marked_text(text);
    if invalid > 0 {
        warnings.push(OcrWarning::new(
            "OCR_BOX_INVALID",
            format!("{invalid} 个版面坐标无效（不在 0–1000 或格式错误），对应块按无几何处理"),
        ));
    }
    if !marked {
        return text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(|line| OcrObservation {
                text: line.to_string(),
                granularity: OcrGranularity::Line,
                bbox: None,
                polygon: None,
                confidence: None,
                label: None,
                lines: Vec::new(),
            })
            .collect();
    }
    blocks
        .into_iter()
        .map(|b| {
            let lines: Vec<String> = b
                .text
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(str::to_string)
                .collect();
            let bbox = union_px(&b.boxes, width, height);
            if b.boxes.len() > 1 {
                warnings.push(OcrWarning::new(
                    "OCR_BLOCK_MULTI_BOX",
                    "一个版面块带多个坐标框，已取并集",
                ));
            }
            OcrObservation {
                text: b.text,
                granularity: if bbox.is_some() {
                    OcrGranularity::Block
                } else {
                    OcrGranularity::Page
                },
                polygon: bbox.map(|r| {
                    [
                        [r.x, r.y],
                        [r.x + r.width, r.y],
                        [r.x + r.width, r.y + r.height],
                        [r.x, r.y + r.height],
                    ]
                }),
                bbox,
                confidence: None,
                label: b.label,
                lines,
            }
        })
        .collect()
}

fn remote_retryable(status: u16, error_type: Option<&str>, message: &str) -> bool {
    if matches!(status, 408 | 429 | 500 | 503 | 504) {
        return true;
    }
    if status == 200 {
        let m = message.to_ascii_lowercase();
        return m.contains("temporarily unavailable")
            || m.contains("internal server error")
            || matches!(
                error_type,
                Some("service_unavailable_error") | Some("internal_error")
            );
    }
    false
}

fn error_object(body: &Value) -> Option<(Option<String>, String)> {
    let error = body.get("error")?;
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("unknown error");
    let error_type = error
        .get("type")
        .and_then(Value::as_str)
        .or_else(|| error.get("code").and_then(Value::as_str))
        .map(str::to_string);
    Some((error_type, truncate(message, MAX_REMOTE_MESSAGE)))
}

/// 解析 HTTP 响应（状态码 + 已读取的响应体）为识别结果。纯函数，便于测试。
pub fn parse_response(
    status: u16,
    body: &[u8],
    width: u32,
    height: u32,
    purpose: &str,
) -> Result<OcrOutput, OcrError> {
    let parsed: Option<Value> = serde_json::from_slice(body).ok();
    if !(200..300).contains(&status) {
        let (error_type, message) = parsed
            .as_ref()
            .and_then(error_object)
            .unwrap_or((None, format!("HTTP {status}")));
        return Err(OcrError::Remote {
            status: Some(status),
            retryable: remote_retryable(status, error_type.as_deref(), &message),
            error_type,
            message,
        });
    }
    let body = parsed.ok_or_else(|| OcrError::ResponseInvalid("响应不是有效 JSON".into()))?;
    if let Some((error_type, message)) = error_object(&body) {
        return Err(OcrError::Remote {
            status: Some(status),
            retryable: remote_retryable(status, error_type.as_deref(), &message),
            error_type,
            message,
        });
    }
    if body.get("object").and_then(Value::as_str) != Some("ocr.result") {
        return Err(OcrError::ResponseInvalid(
            "响应缺少 object=ocr.result".into(),
        ));
    }
    let pages = body
        .get("pages")
        .and_then(Value::as_array)
        .ok_or_else(|| OcrError::ResponseInvalid("响应缺少 pages 数组".into()))?;
    let mut warnings = Vec::new();
    if let Some(count) = body.get("page_count").and_then(Value::as_u64) {
        if count as usize != pages.len() {
            warnings.push(OcrWarning::new(
                "OCR_PAGE_COUNT_MISMATCH",
                format!("page_count={count} 与 pages 数量 {} 不一致", pages.len()),
            ));
        }
    }
    if pages.len() > 1 {
        warnings.push(OcrWarning::new(
            "OCR_MULTIPLE_PAGES",
            format!("单张图像返回了 {} 页，按顺序合并", pages.len()),
        ));
    }
    let mut observations = Vec::new();
    let mut raw = Vec::new();
    let mut truncated = false;
    for page in pages {
        let text = page
            .get("text")
            .and_then(Value::as_str)
            .ok_or_else(|| OcrError::ResponseInvalid("页面缺少 text".into()))?;
        match page.get("finish_reason").and_then(Value::as_str) {
            Some("stop") => {}
            Some("length") => {
                truncated = true;
                warnings.push(OcrWarning::new(
                    "OCR_TRUNCATED",
                    "在线 OCR 输出达到上限（finish_reason=length），该页文本不完整",
                ));
            }
            other => warnings.push(OcrWarning::new(
                "OCR_UNEXPECTED_FINISH_REASON",
                format!("finish_reason={}", other.unwrap_or("null")),
            )),
        }
        raw.push(text.to_string());
        observations.extend(page_observations(text, width, height, &mut warnings));
    }
    if observations.is_empty() {
        warnings.push(OcrWarning::new(
            "OCR_EMPTY_RESULT",
            "在线 OCR 正常返回但没有文字；这不等于画面中确定没有聊天",
        ));
    }
    if !observations.iter().any(|o| o.bbox.is_some()) && !observations.is_empty() {
        warnings.push(OcrWarning::new(
            "OCR_NO_GEOMETRY",
            "在线结果没有坐标框；observations 只有文字，没有几何",
        ));
    }
    Ok(OcrOutput {
        observations,
        raw_text: Some(raw.join("\n\n")),
        truncated,
        warnings,
        model_version: format!(
            "DeepSeek OCR API /v1/tools/ocr purpose={purpose}（接口未返回模型版本）"
        ),
        cold_start: false,
        phases: Vec::new(),
        extra: json!({
            "remoteId": body.get("id").and_then(Value::as_str),
            "usage": body.get("usage").cloned().unwrap_or(Value::Null),
        }),
    })
}

fn multipart(boundary: &str, purpose: &str, png: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(png.len() + 512);
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"purpose\"\r\n\r\n{purpose}\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"image.png\"\r\nContent-Type: image/png\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(png);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    body
}

async fn send(
    url: reqwest::Url,
    key: String,
    purpose: &'static str,
    png: Vec<u8>,
    timeout: Duration,
) -> Result<(u16, Vec<u8>, u64), OcrError> {
    let mut builder = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10).min(timeout))
        .timeout(timeout);
    if is_loopback(&url) {
        builder = builder.no_proxy();
    }
    let client = builder.build().map_err(|e| OcrError::Transport {
        message: format!("创建 HTTP 客户端失败: {e}"),
        retryable: false,
    })?;
    let mut nonce = [0u8; 12];
    let _ = getrandom::getrandom(&mut nonce);
    let boundary: String = nonce.iter().map(|b| format!("{b:02x}")).collect();
    let body = multipart(&format!("kotone{boundary}"), purpose, &png);
    let started = Instant::now();
    let mut response = client
        .post(url)
        .bearer_auth(key)
        .header(
            reqwest::header::CONTENT_TYPE,
            format!("multipart/form-data; boundary=kotone{boundary}"),
        )
        .body(body)
        .send()
        .await
        .map_err(|e| OcrError::Transport {
            message: if e.is_timeout() {
                "请求超时".into()
            } else if e.is_connect() {
                "无法连接在线 OCR 服务".into()
            } else {
                "请求发送失败".into()
            },
            retryable: true,
        })?;
    let upload_ms = started.elapsed().as_millis() as u64;
    let status = response.status().as_u16();
    if response
        .content_length()
        .is_some_and(|n| n > MAX_RESPONSE_BYTES as u64)
    {
        return Err(OcrError::ResponseInvalid("响应超出大小限制".into()));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| OcrError::Transport {
        message: if e.is_timeout() {
            "读取响应超时".into()
        } else {
            "读取响应失败".into()
        },
        retryable: true,
    })? {
        if bytes.len() + chunk.len() > MAX_RESPONSE_BYTES {
            return Err(OcrError::ResponseInvalid("响应超出大小限制".into()));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok((status, bytes, upload_ms))
}

impl OcrBackend for DeepSeekOcr {
    fn id(&self) -> &str {
        BACKEND_ID
    }

    fn kind(&self) -> OcrBackendKind {
        OcrBackendKind::Online
    }

    fn info(&self) -> OcrBackendInfo {
        let (ready, state, reason, connection, endpoint) = match self.resolve() {
            Resolved::NotConfigured => (
                false,
                "not-configured",
                Some("在线 OCR 未启用：需用户执行 `kotone-cli config set toolService.ocr.deepseekConnectionId <连接ID>` 并重启桌面端".to_string()),
                None,
                None,
            ),
            Resolved::Error(e) => (false, "credentials-error", Some(e), None, None),
            Resolved::MissingKey(c) => (
                false,
                "credentials-missing",
                Some(format!("连接 {} 没有 API key（系统凭据库）", c.id)),
                Some(c),
                None,
            ),
            Resolved::InvalidEndpoint(c, e) => (false, "invalid", Some(e), Some(c), None),
            Resolved::Ready(c, url, _) => (true, "ready", None, Some(c), Some(url)),
        };
        OcrBackendInfo {
            id: BACKEND_ID.into(),
            kind: OcrBackendKind::Online,
            display_name: "DeepSeek OCR (online)".into(),
            ready,
            state: state.into(),
            reason,
            languages: vec!["auto".into()],
            default_language: "auto".into(),
            data_egress: "selected-image".into(),
            options_schema: options_schema(),
            details: json!({
                "engine": "DeepSeek OCR API",
                "device": "remote",
                "apiDoc": API_DOC,
                "apiVerifiedAt": "2026-11-15",
                "connectionId": connection.as_ref().map(|c| c.id.clone()),
                "endpointHost": endpoint.as_ref().and_then(|u| u.host_str().map(str::to_string)),
                "modelVersion": null,
                "modelVersionNote": "the API does not report a model version",
                "egress": "only the PNG bytes of the requested artifact and the purpose field are uploaded; no window title, frame metadata, chat history or other images",
                "granularity": ["line", "block", "page"],
                "geometry": "none for plain text; block boxes only when the response carries <｜box｜> markers (0–1000 normalised, converted to input pixels); never synthesised per line",
                "confidence": null,
                "cancellation": "client-side: the HTTP request is dropped on cancel/timeout, but the remote recognition continues and is billed",
                "billing": "per recognised page (see API doc)",
                "concurrencyLimit": "15 concurrent recognitions per account (remote)",
                "fallback": "none: never falls back to local or another backend",
                "languageNote": "the API has no language parameter; only language=auto is accepted"
            }),
        }
    }

    fn recognize(
        &self,
        input: &OcrInput<'_>,
        params: &OcrParams,
        cancel: &OcrCancel,
        deadline: Instant,
    ) -> Result<OcrOutput, OcrError> {
        if params.language != "auto" {
            return Err(OcrError::UnsupportedLanguage {
                language: params.language.clone(),
                supported: vec!["auto".into()],
            });
        }
        let purpose = parse_purpose(&params.options)?;
        let (url, key) = match self.resolve() {
            Resolved::Ready(_, url, key) => (url, key),
            other => {
                let (reason, message) = match other {
                    Resolved::NotConfigured => ("not-configured", "在线 OCR 未启用".to_string()),
                    Resolved::Error(e) => ("credentials-error", e),
                    Resolved::MissingKey(c) => {
                        ("credentials-missing", format!("连接 {} 没有 API key", c.id))
                    }
                    Resolved::InvalidEndpoint(_, e) => ("invalid", e),
                    Resolved::Ready(..) => unreachable!(),
                };
                return Err(OcrError::NotReady {
                    reason: reason.into(),
                    message,
                });
            }
        };
        if input.png.is_empty() || input.png.len() > MAX_UPLOAD_BYTES {
            return Err(OcrError::InvalidInput(format!(
                "上传图像大小 {} 字节不在 1..={MAX_UPLOAD_BYTES}",
                input.png.len()
            )));
        }
        cancel.check(deadline)?;
        let remaining = deadline.saturating_duration_since(Instant::now());
        let timeout = remaining.min(Duration::from_secs(180));
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| OcrError::Engine(format!("创建网络运行时失败: {e}")))?;
        let notify = Arc::new(tokio::sync::Notify::new());
        let waker = notify.clone();
        let _registration = cancel.on_cancel(move || waker.notify_one());
        let png = input.png.to_vec();
        let started = Instant::now();
        let result = runtime.block_on(async move {
            tokio::select! {
                biased;
                _ = notify.notified() => Err(OcrError::Cancelled),
                _ = tokio::time::sleep(timeout) => Err(OcrError::DeadlineExceeded),
                r = send(url, key, purpose, png, timeout) => r,
            }
        });
        // 请求 future 在 select 结束时被丢弃（连接关闭）；远端识别仍会继续
        let (status, body, upload_ms) = match result {
            Err(OcrError::Transport { .. }) if cancel.is_cancelled() => {
                return Err(OcrError::Cancelled)
            }
            Err(OcrError::Transport { .. }) if Instant::now() >= deadline => {
                return Err(OcrError::DeadlineExceeded)
            }
            other => other?,
        };
        let mut output = parse_response(status, &body, input.width, input.height, purpose)?;
        output
            .phases
            .push(("requestMs".into(), started.elapsed().as_millis() as u64));
        output.phases.push(("uploadMs".into(), upload_ms));
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_derivation_and_scheme_rules() {
        assert_eq!(
            ocr_endpoint("https://api.deepseek.com").unwrap().as_str(),
            "https://api.deepseek.com/v1/tools/ocr"
        );
        assert_eq!(
            ocr_endpoint("https://api.deepseek.com/v1/")
                .unwrap()
                .as_str(),
            "https://api.deepseek.com/v1/tools/ocr"
        );
        assert_eq!(
            ocr_endpoint("http://127.0.0.1:9/x").unwrap().as_str(),
            "http://127.0.0.1:9/x/v1/tools/ocr"
        );
        assert!(ocr_endpoint("http://api.deepseek.com").is_err());
        assert!(ocr_endpoint("ftp://x").is_err());
        assert!(ocr_endpoint("not a url").is_err());
    }

    #[test]
    fn plain_text_has_no_geometry() {
        let body = json!({"id":"ocr-1","object":"ocr.result","created":1,"purpose":"text_recognition","page_count":1,
            "pages":[{"index":0,"text":"[팀] 가: 안녕\n\n나 (나): 미드 갈게\n","finish_reason":"stop"}],
            "usage":{"input_tokens":1,"output_tokens":2,"total_tokens":3}});
        let out = parse_response(
            200,
            body.to_string().as_bytes(),
            650,
            200,
            "text_recognition",
        )
        .unwrap();
        assert_eq!(out.observations.len(), 2);
        assert!(out
            .observations
            .iter()
            .all(|o| o.bbox.is_none() && o.confidence.is_none()));
        assert_eq!(out.observations[0].granularity, OcrGranularity::Line);
        assert_eq!(out.observations[1].text, "나 (나): 미드 갈게");
        assert!(out.warnings.iter().any(|w| w.code == "OCR_NO_GEOMETRY"));
        assert!(!out.truncated);
        assert_eq!(out.extra["usage"]["total_tokens"], 3);
    }

    #[test]
    fn block_boxes_are_block_granularity_in_pixels() {
        let text = "<｜ref｜>text<｜/ref｜><｜box｜>[[0,0,999,500]]<｜/box｜> 첫 줄\n둘째 줄\n<｜ref｜>제목<｜/ref｜><｜box｜>[[100, 600, 900, 700]]<｜/box｜>";
        let body = json!({"object":"ocr.result","page_count":1,"pages":[{"index":0,"text":text,"finish_reason":"stop"}]});
        let out = parse_response(
            200,
            body.to_string().as_bytes(),
            1000,
            200,
            "rich_layout_recognition",
        )
        .unwrap();
        assert_eq!(out.observations.len(), 2);
        let a = &out.observations[0];
        assert_eq!(a.granularity, OcrGranularity::Block);
        assert_eq!(a.label.as_deref(), Some("text"));
        assert_eq!(a.lines, vec!["첫 줄", "둘째 줄"]);
        let r = a.bbox.unwrap();
        assert_eq!((r.x, r.y, r.width, r.height), (0.0, 0.0, 999.0, 100.0));
        // grounding 形式：ref 即文字
        let b = &out.observations[1];
        assert_eq!(b.text, "제목");
        assert_eq!(b.label, None);
        assert_eq!(b.bbox.unwrap().y, 120.0);
        assert!(a.confidence.is_none());
    }

    #[test]
    fn http_200_error_object_is_an_error_not_empty_chat() {
        let body = json!({"error":{"message":"OCR did not finish in time","type":"gateway_timeout_error","param":null,"code":"gateway_timeout_error"}});
        let err = parse_response(
            200,
            format!("\n\n{body}").as_bytes(),
            10,
            10,
            "text_recognition",
        )
        .unwrap_err();
        match err {
            OcrError::Remote {
                status,
                retryable,
                error_type,
                message,
            } => {
                assert_eq!(status, Some(200));
                assert!(!retryable, "文档：识别超时重试无法解决");
                assert_eq!(error_type.as_deref(), Some("gateway_timeout_error"));
                assert!(message.contains("did not finish"));
            }
            other => panic!("{other:?}"),
        }
        let busy = json!({"error":{"message":"The OCR service is temporarily unavailable. Please try again later.","type":"service_unavailable_error"}});
        assert!(matches!(
            parse_response(200, busy.to_string().as_bytes(), 1, 1, "text_recognition"),
            Err(OcrError::Remote {
                retryable: true,
                ..
            })
        ));
        // HTTP 状态错误：429 可重试、402 不可重试，且不回显响应体以外内容
        assert!(matches!(
            parse_response(429, b"{}", 1, 1, "text_recognition"),
            Err(OcrError::Remote {
                retryable: true,
                status: Some(429),
                ..
            })
        ));
        assert!(matches!(
            parse_response(402, b"not json", 1, 1, "text_recognition"),
            Err(OcrError::Remote {
                retryable: false,
                ..
            })
        ));
    }

    #[test]
    fn truncated_empty_and_invalid_responses() {
        let body = json!({"object":"ocr.result","page_count":1,"pages":[{"index":0,"text":"앞부분","finish_reason":"length"}]});
        let out =
            parse_response(200, body.to_string().as_bytes(), 1, 1, "text_recognition").unwrap();
        assert!(out.truncated);
        assert!(out.warnings.iter().any(|w| w.code == "OCR_TRUNCATED"));
        let body = json!({"object":"ocr.result","page_count":1,"pages":[{"index":0,"text":"  \n ","finish_reason":"stop"}]});
        let out =
            parse_response(200, body.to_string().as_bytes(), 1, 1, "text_recognition").unwrap();
        assert!(out.observations.is_empty());
        assert!(out.warnings.iter().any(|w| w.code == "OCR_EMPTY_RESULT"));
        for bad in [
            &b"<html>"[..],
            br#"{"object":"chat.completion"}"#,
            br#"{"object":"ocr.result"}"#,
        ] {
            assert!(matches!(
                parse_response(200, bad, 1, 1, "text_recognition"),
                Err(OcrError::ResponseInvalid(_))
            ));
        }
    }

    #[test]
    fn invalid_boxes_degrade_to_no_geometry_with_warning() {
        let mut warnings = Vec::new();
        let obs = page_observations(
            "<｜ref｜>text<｜/ref｜><｜box｜>[[0,0,2000,10]]<｜/box｜>가나",
            100,
            100,
            &mut warnings,
        );
        assert_eq!(obs.len(), 1);
        assert!(obs[0].bbox.is_none());
        assert_eq!(obs[0].granularity, OcrGranularity::Page);
        assert!(warnings.iter().any(|w| w.code == "OCR_BOX_INVALID"));
    }

    #[test]
    fn options_and_language_are_rejected_explicitly() {
        assert_eq!(parse_purpose(&Value::Null).unwrap(), "text_recognition");
        assert!(parse_purpose(&json!({"purpose": "translate"})).is_err());
        assert!(parse_purpose(&json!({"minConfidence": 0.5})).is_err());
        let backend = DeepSeekOcr::new(Arc::new(|| Ok(None)));
        let info = backend.info();
        assert_eq!(info.state, "not-configured");
        assert!(!info.ready);
        let px = [0u8; 4];
        let input = OcrInput {
            width: 1,
            height: 1,
            rgba: &px,
            png: b"png",
        };
        let err = backend
            .recognize(
                &input,
                &OcrParams {
                    language: "ko".into(),
                    options: Value::Null,
                },
                &OcrCancel::new(),
                Instant::now() + Duration::from_secs(1),
            )
            .unwrap_err();
        assert!(matches!(err, OcrError::UnsupportedLanguage { .. }));
        let err = backend
            .recognize(
                &input,
                &OcrParams {
                    language: "auto".into(),
                    options: Value::Null,
                },
                &OcrCancel::new(),
                Instant::now() + Duration::from_secs(1),
            )
            .unwrap_err();
        assert!(matches!(err, OcrError::NotReady { ref reason, .. } if reason == "not-configured"));
    }

    fn connection(base: &str, key: Option<&str>) -> ResolvedConnection {
        ResolvedConnection {
            id: "ds".into(),
            display_name: "DeepSeek".into(),
            kind: kotone_core::connection::ConnectionKind::Remote,
            provider: "custom".into(),
            base_url: base.into(),
            model: "deepseek-chat".into(),
            api_key: key.map(str::to_string),
        }
    }

    #[test]
    fn mock_server_uploads_only_the_selected_png_with_bearer_key() {
        use wiremock::matchers::{header, method, path};
        use wiremock::{Mock, MockServer, Request, ResponseTemplate};
        let rt = tokio::runtime::Runtime::new().unwrap();
        let server = rt.block_on(MockServer::start());
        let reply = json!({"object":"ocr.result","page_count":1,"pages":[{"index":0,"text":"안녕","finish_reason":"stop"}]});
        rt.block_on(
            Mock::given(method("POST"))
                .and(path("/v1/tools/ocr"))
                .and(header("authorization", "Bearer sk-test"))
                .respond_with(ResponseTemplate::new(200).set_body_string(format!("\n\n{reply}")))
                .expect(1)
                .mount(&server),
        );
        let uri = server.uri();
        let backend = DeepSeekOcr::new(Arc::new(move || {
            Ok(Some(connection(&uri, Some("sk-test"))))
        }));
        assert_eq!(backend.info().state, "ready");
        let px = [0u8; 4];
        let png = b"\x89PNG-fixture-bytes";
        let input = OcrInput {
            width: 1,
            height: 1,
            rgba: &px,
            png,
        };
        let out = backend
            .recognize(
                &input,
                &OcrParams {
                    language: "auto".into(),
                    options: Value::Null,
                },
                &OcrCancel::new(),
                Instant::now() + Duration::from_secs(10),
            )
            .unwrap();
        assert_eq!(out.observations[0].text, "안녕");
        let requests: Vec<Request> = rt.block_on(server.received_requests()).unwrap();
        assert_eq!(requests.len(), 1);
        let body = &requests[0].body;
        let text = String::from_utf8_lossy(body);
        assert!(text.contains("name=\"purpose\"\r\n\r\ntext_recognition"));
        assert!(text.contains("filename=\"image.png\""));
        assert!(
            body.windows(png.len()).any(|w| w == png),
            "上传的是指定的 PNG 字节"
        );
        // 只有两个表单字段
        assert_eq!(text.matches("Content-Disposition").count(), 2);
        // 缺少 key：不发请求
        let backend = DeepSeekOcr::new(Arc::new(|| {
            Ok(Some(connection("https://api.deepseek.com", None)))
        }));
        assert_eq!(backend.info().state, "credentials-missing");
        rt.block_on(server.verify());
    }

    #[test]
    fn cancel_and_deadline_abort_a_slow_request() {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let rt = tokio::runtime::Runtime::new().unwrap();
        let server = rt.block_on(MockServer::start());
        rt.block_on(
            Mock::given(method("POST"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_delay(Duration::from_secs(5))
                        .set_body_string("{}"),
                )
                .mount(&server),
        );
        let uri = server.uri();
        let backend = DeepSeekOcr::new(Arc::new(move || Ok(Some(connection(&uri, Some("k"))))));
        let px = [0u8; 4];
        let input = OcrInput {
            width: 1,
            height: 1,
            rgba: &px,
            png: b"png",
        };
        let params = OcrParams {
            language: "auto".into(),
            options: Value::Null,
        };
        let started = Instant::now();
        let err = backend
            .recognize(
                &input,
                &params,
                &OcrCancel::new(),
                Instant::now() + Duration::from_millis(200),
            )
            .unwrap_err();
        assert_eq!(err, OcrError::DeadlineExceeded);
        assert!(started.elapsed() < Duration::from_secs(2));
        let cancel = OcrCancel::new();
        let c2 = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(150));
            c2.cancel();
        });
        let started = Instant::now();
        let err = backend
            .recognize(
                &input,
                &params,
                &cancel,
                Instant::now() + Duration::from_secs(10),
            )
            .unwrap_err();
        assert_eq!(err, OcrError::Cancelled);
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}
