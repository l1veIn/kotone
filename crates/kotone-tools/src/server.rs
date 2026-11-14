//! loopback HTTP/1.1 服务端：认证、路由、请求体上限、连接上限与关闭。
//!
//! 每个端点（包括未知路径）先校验 `Authorization: Bearer <会话凭证>`；带 `Origin`
//! 头的请求（浏览器）与非 loopback `Host` 一律拒绝，防止网页借 DNS 重绑定访问。

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;

use http_body_util::{BodyExt, Full, Limited};
use hyper::body::{Bytes, Incoming};
use hyper::header::{HeaderValue, AUTHORIZATION, CACHE_CONTROL, CONTENT_LENGTH, CONTENT_TYPE};
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::{TokioIo, TokioTimer};
use serde_json::Value;
use tokio::net::TcpListener;
use tokio::sync::{watch, Semaphore};
use tokio::task::JoinSet;

use crate::ids::{new_id, SecretToken};
use crate::protocol::{
    is_valid_request_id, Envelope, ErrorCode, Timing, ToolCallRequest, ToolError, INSTANCE_HEADER,
    PATH_CAPABILITIES, PATH_STATUS, PATH_TOOLS_CALL, REQUEST_ID_HEADER,
};
use crate::service::ToolService;

struct Context {
    service: Arc<ToolService>,
    token: SecretToken,
    port: u16,
}

/// 正在运行的 HTTP 服务。丢弃或 [`ServerHandle::shutdown`] 时停止接受连接并中止在途连接。
pub struct ServerHandle {
    addr: SocketAddr,
    stop: watch::Sender<bool>,
    task: Option<tokio::task::JoinHandle<()>>,
}

impl ServerHandle {
    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    /// 发出停止信号并等待接受循环退出（在途连接被中止）。
    pub async fn shutdown(mut self) {
        let _ = self.stop.send(true);
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }

    /// 同步发出停止信号（应用退出路径不等待异步任务）。
    pub fn signal_stop(&self) {
        let _ = self.stop.send(true);
    }
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        let _ = self.stop.send(true);
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

/// 绑定 `127.0.0.1:0`（系统分配端口）并开始服务。
pub async fn serve(service: Arc<ToolService>, token: SecretToken) -> std::io::Result<ServerHandle> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let addr = listener.local_addr()?;
    let (stop, mut stop_rx) = watch::channel(false);
    let limits = service.limits().clone();
    let context = Arc::new(Context {
        service,
        token,
        port: addr.port(),
    });
    let connections = Arc::new(Semaphore::new(limits.max_connections.max(1)));
    let task = tokio::spawn(async move {
        let mut tasks = JoinSet::new();
        loop {
            tokio::select! {
                changed = stop_rx.changed() => {
                    if changed.is_err() || *stop_rx.borrow() {
                        break;
                    }
                }
                accepted = listener.accept() => {
                    let Ok((stream, peer)) = accepted else { continue };
                    if !peer.ip().is_loopback() {
                        continue;
                    }
                    // 连接数已满：直接关闭新连接，不排队
                    let Ok(permit) = connections.clone().try_acquire_owned() else {
                        drop(stream);
                        continue;
                    };
                    let _ = stream.set_nodelay(true);
                    let context = context.clone();
                    let header_timeout = limits.header_read_timeout;
                    let connection_timeout = limits.connection_timeout;
                    tasks.spawn(async move {
                        let _permit = permit;
                        let service = hyper::service::service_fn(move |req| {
                            let context = context.clone();
                            async move { Ok::<_, Infallible>(handle(context, req).await) }
                        });
                        let conn = hyper::server::conn::http1::Builder::new()
                            .timer(TokioTimer::new())
                            .header_read_timeout(header_timeout)
                            .serve_connection(TokioIo::new(stream), service);
                        let _ = tokio::time::timeout(connection_timeout, conn).await;
                    });
                }
                Some(_) = tasks.join_next(), if !tasks.is_empty() => {}
            }
        }
        tasks.abort_all();
    });
    Ok(ServerHandle {
        addr,
        stop,
        task: Some(task),
    })
}

fn json_response(
    status: StatusCode,
    instance_id: &str,
    envelope: &Envelope,
) -> Response<Full<Bytes>> {
    let body = serde_json::to_vec(envelope).unwrap_or_else(|_| b"{}".to_vec());
    let mut response = Response::new(Full::new(Bytes::from(body)));
    *response.status_mut() = status;
    let headers = response.headers_mut();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    if let Ok(value) = HeaderValue::from_str(instance_id) {
        headers.insert(INSTANCE_HEADER, value);
    }
    response
}

fn reject(
    context: &Context,
    status: StatusCode,
    code: ErrorCode,
    message: &str,
    request_id: String,
    started: Instant,
) -> Response<Full<Bytes>> {
    let envelope = Envelope::failure(
        request_id,
        Some(context.service.instance_id().to_string()),
        None,
        ToolError::new(code, message),
        Timing {
            total_ms: started.elapsed().as_millis() as u64,
            ..Default::default()
        },
    );
    let mut response = json_response(status, context.service.instance_id(), &envelope);
    if status == StatusCode::UNAUTHORIZED {
        response
            .headers_mut()
            .insert("www-authenticate", HeaderValue::from_static("Bearer"));
    }
    response
}

fn host_is_loopback(host: &str, port: u16) -> bool {
    let expected = [format!("127.0.0.1:{port}"), format!("localhost:{port}")];
    expected.iter().any(|e| e.eq_ignore_ascii_case(host))
}

fn bearer(req: &Request<Incoming>) -> Option<&str> {
    let value = req.headers().get(AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = value.split_once(' ')?;
    scheme
        .eq_ignore_ascii_case("bearer")
        .then_some(token.trim())
}

async fn handle(context: Arc<Context>, req: Request<Incoming>) -> Response<Full<Bytes>> {
    let started = Instant::now();
    let header_request_id = req
        .headers()
        .get(REQUEST_ID_HEADER)
        .and_then(|v| v.to_str().ok())
        .filter(|id| is_valid_request_id(id))
        .map(str::to_string);
    let request_id = header_request_id.unwrap_or_else(|| new_id("req"));

    if req.headers().contains_key("origin") {
        return reject(
            &context,
            StatusCode::FORBIDDEN,
            ErrorCode::ForbiddenOrigin,
            "工具服务不接受浏览器跨源请求",
            request_id,
            started,
        );
    }
    let host_ok = req
        .headers()
        .get("host")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|h| host_is_loopback(h, context.port));
    if !host_ok {
        return reject(
            &context,
            StatusCode::FORBIDDEN,
            ErrorCode::ForbiddenOrigin,
            "Host 必须是本机 loopback 地址",
            request_id,
            started,
        );
    }
    if !bearer(&req).is_some_and(|t| context.token.matches(t)) {
        return reject(
            &context,
            StatusCode::UNAUTHORIZED,
            ErrorCode::Unauthorized,
            "缺少或无效的会话凭证",
            request_id,
            started,
        );
    }

    let path = req.uri().path().to_string();
    let method = req.method().clone();
    match (method, path.as_str()) {
        (Method::GET, PATH_CAPABILITIES) => {
            let envelope = Envelope::success(
                request_id,
                Some(context.service.instance_id().to_string()),
                None,
                context.service.capabilities(),
                Timing {
                    total_ms: started.elapsed().as_millis() as u64,
                    ..Default::default()
                },
            );
            json_response(StatusCode::OK, context.service.instance_id(), &envelope)
        }
        (Method::GET, PATH_STATUS) => {
            let envelope = Envelope::success(
                request_id,
                Some(context.service.instance_id().to_string()),
                None,
                context.service.status(),
                Timing {
                    total_ms: started.elapsed().as_millis() as u64,
                    ..Default::default()
                },
            );
            json_response(StatusCode::OK, context.service.instance_id(), &envelope)
        }
        (Method::POST, PATH_TOOLS_CALL) => call(context, req, request_id, started).await,
        (_, PATH_CAPABILITIES | PATH_STATUS | PATH_TOOLS_CALL) => reject(
            &context,
            StatusCode::METHOD_NOT_ALLOWED,
            ErrorCode::MethodNotAllowed,
            "不支持的 HTTP 方法",
            request_id,
            started,
        ),
        _ => reject(
            &context,
            StatusCode::NOT_FOUND,
            ErrorCode::RouteNotFound,
            "未知端点",
            request_id,
            started,
        ),
    }
}

async fn call(
    context: Arc<Context>,
    req: Request<Incoming>,
    request_id: String,
    started: Instant,
) -> Response<Full<Bytes>> {
    let max = context.service.limits().max_request_bytes;
    if let Some(content_type) = req.headers().get(CONTENT_TYPE) {
        let ok = content_type
            .to_str()
            .map(|v| {
                v.trim()
                    .to_ascii_lowercase()
                    .starts_with("application/json")
            })
            .unwrap_or(false);
        if !ok {
            return reject(
                &context,
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                ErrorCode::UnsupportedMediaType,
                "请求体必须是 application/json",
                request_id,
                started,
            );
        }
    }
    let declared = req
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());
    if declared.is_some_and(|len| len > max as u64) {
        return reject(
            &context,
            StatusCode::PAYLOAD_TOO_LARGE,
            ErrorCode::PayloadTooLarge,
            &format!("请求体超过 {max} 字节上限"),
            request_id,
            started,
        );
    }
    let body = match Limited::new(req.into_body(), max).collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(_) => {
            return reject(
                &context,
                StatusCode::PAYLOAD_TOO_LARGE,
                ErrorCode::PayloadTooLarge,
                &format!("请求体超过 {max} 字节上限或读取失败"),
                request_id,
                started,
            );
        }
    };
    let parsed: Result<ToolCallRequest, _> = serde_json::from_slice(&body);
    let mut request = match parsed {
        Ok(request) => request,
        Err(error) => {
            // 尽量回显调用方的 requestId，便于关联
            let echoed = serde_json::from_slice::<Value>(&body)
                .ok()
                .and_then(|v| {
                    v.get("requestId")
                        .and_then(|r| r.as_str())
                        .map(str::to_string)
                })
                .filter(|id| is_valid_request_id(id))
                .unwrap_or(request_id);
            return reject(
                &context,
                StatusCode::BAD_REQUEST,
                ErrorCode::InvalidRequest,
                &format!("请求信封不合法: {error}"),
                echoed,
                started,
            );
        }
    };
    if request.request_id.is_none() {
        request.request_id = Some(request_id);
    }
    let envelope = context.service.call(request).await;
    json_response(StatusCode::OK, context.service.instance_id(), &envelope)
}
