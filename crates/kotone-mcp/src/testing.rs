//! 仅供测试（特性 `test-support`）：进程内的 Streamable HTTP 测试服务器，用 rmcp 自己的服务端
//! （有会话，回复走 SSE，和 OP.GG 的服务器一样）。工具和资源用协议原样的 JSON 给，工具怎么回由测试写；
//! 服务器记下收到了什么（几个 POST、握手几次、`clientInfo`、每次调用的参数……），测试拿来核对会话复用这类行为。

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ErrorCode, Implementation,
    InitializeRequestParams, InitializeResult, ListResourcesResult, ListToolsResult,
    PaginatedRequestParams, ReadResourceRequestParams, ReadResourceResponse, ReadResourceResult,
    ServerCapabilities, ServerConfig,
};
use rmcp::service::RequestContext;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{ErrorData, RoleServer, ServerHandler};
use serde_json::{json, Value};

/// 工具的回复：`Ok` 是 `CallToolResult` 的 JSON（`{"content": [...], "isError": ...}`），`Err` 是 JSON-RPC 错误（码、消息）。
pub type Reply = Result<Value, (i32, String)>;
pub type ToolHandler =
    Arc<dyn Fn(String, Value) -> Pin<Box<dyn Future<Output = Reply> + Send>> + Send + Sync>;

/// 把一个 async 闭包包成 [`ToolHandler`]。
pub fn handler<F, Fut>(f: F) -> ToolHandler
where
    F: Fn(String, Value) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Reply> + Send + 'static,
{
    Arc::new(move |name, args| Box::pin(f(name, args)))
}

/// 一段纯文字的工具回复。
pub fn text_reply(text: &str) -> Reply {
    Ok(json!({"content": [{"type": "text", "text": text}]}))
}

/// 服务器记下的东西。
#[derive(Debug, Clone, Default)]
pub struct TestLog {
    /// HTTP 请求数（按方法）。
    pub posts: usize,
    pub gets: usize,
    pub deletes: usize,
    /// 带着 `Mcp-Session-Id` 的 POST 数。
    pub posts_with_session: usize,
    /// `initialize` 次数（= 建了几个会话）。
    pub initializes: usize,
    /// 最后一次 `initialize` 的参数（`clientInfo`、`protocolVersion`……）。
    pub initialize: Option<Value>,
    /// 每次 `tools/call` 的名字和参数。
    pub calls: Vec<(String, Value)>,
    pub user_agent: Option<String>,
    /// 最后一个请求里 `x-` 开头的头。
    pub custom_headers: BTreeMap<String, String>,
}

#[derive(Clone)]
struct TestHandler {
    tools: Arc<Vec<rmcp::model::Tool>>,
    resources: Arc<Value>,
    reply: ToolHandler,
    log: Arc<Mutex<TestLog>>,
}

impl ServerHandler for TestHandler {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
        )
        .with_server_info(Implementation::new("kotone-test-server", "1.0.0"))
        .with_instructions("测试服务器")
    }

    async fn initialize(
        &self,
        request: InitializeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<InitializeResult, ErrorData> {
        {
            let mut log = self.log.lock().unwrap();
            log.initializes += 1;
            log.initialize = serde_json::to_value(&request).ok();
        }
        context.peer.set_peer_info(request.clone());
        self.negotiate_initialize(&request)
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(self.tools.as_ref().clone()))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let name = request.name.to_string();
        let args = request.arguments.map(Value::Object).unwrap_or(Value::Null);
        self.log
            .lock()
            .unwrap()
            .calls
            .push((name.clone(), args.clone()));
        match (self.reply)(name, args).await {
            Ok(value) => serde_json::from_value::<CallToolResult>(value)
                .map(Into::into)
                .map_err(|e| ErrorData::internal_error(e.to_string(), None)),
            Err((code, message)) => Err(ErrorData::new(ErrorCode(code), message, None)),
        }
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        let list = self
            .resources
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|r| serde_json::from_value(r.clone()).ok())
                    .collect()
            })
            .unwrap_or_default();
        Ok(ListResourcesResult::with_all_items(list))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, ErrorData> {
        let found = self.resources.as_array().and_then(|items| {
            items
                .iter()
                .find(|r| r.get("uri").and_then(Value::as_str) == Some(request.uri.as_str()))
        });
        let Some(found) = found else {
            return Err(ErrorData::resource_not_found(
                format!("没有 {}", request.uri),
                None,
            ));
        };
        let contents = json!({"contents": [{
            "uri": request.uri,
            "mimeType": found.get("mimeType").cloned().unwrap_or(Value::Null),
            "text": found.get("text").cloned().unwrap_or(json!("")),
            "_meta": found.get("_meta").cloned().unwrap_or(json!({})),
        }]});
        serde_json::from_value::<ReadResourceResult>(contents)
            .map(Into::into)
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))
    }
}

/// 一个跑在 `127.0.0.1` 随机端口上的测试服务器，丢掉时停（连着的连接也断）。
pub struct TestServer {
    url: String,
    log: Arc<Mutex<TestLog>>,
    task: tokio::task::JoinHandle<()>,
}

impl TestServer {
    /// `tools`：工具的 JSON 数组（`tools/list` 原样）；`resources`：资源的 JSON 数组，每项可以带 `text`（读的时候回它）。
    pub async fn start(tools: Value, resources: Value, reply: ToolHandler) -> Self {
        let tools: Vec<rmcp::model::Tool> =
            serde_json::from_value(tools).expect("工具的 JSON 不对");
        let log = Arc::new(Mutex::new(TestLog::default()));
        let handler = TestHandler {
            tools: Arc::new(tools),
            resources: Arc::new(resources),
            reply,
            log: log.clone(),
        };
        let service: StreamableHttpService<TestHandler, LocalSessionManager> =
            StreamableHttpService::new(
                move || Ok(handler.clone()),
                Default::default(),
                StreamableHttpServerConfig::default(),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("测试服务器绑不上端口");
        let url = format!("http://{}/mcp", listener.local_addr().unwrap());
        let http_log = log.clone();
        let task = tokio::spawn(async move {
            // 连接放在 JoinSet 里：服务器停的时候（这个任务被 abort）连着的连接一起断
            let mut connections = tokio::task::JoinSet::new();
            while let Ok((stream, _)) = listener.accept().await {
                while connections.try_join_next().is_some() {}
                let service = service.clone();
                let log = http_log.clone();
                connections.spawn(async move {
                    let svc = hyper::service::service_fn(
                        move |request: http::Request<hyper::body::Incoming>| {
                            record(&log, &request);
                            let mut service = service.clone();
                            async move { tower_service::Service::call(&mut service, request).await }
                        },
                    );
                    let _ = hyper::server::conn::http1::Builder::new()
                        .serve_connection(hyper_util::rt::TokioIo::new(stream), svc)
                        .await;
                });
            }
        });
        Self { url, log, task }
    }

    /// 服务器的地址（`http://127.0.0.1:<端口>/mcp`）。
    pub fn url(&self) -> &str {
        &self.url
    }

    /// 到现在为止记下的东西。
    pub fn log(&self) -> TestLog {
        self.log.lock().unwrap().clone()
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn record<B>(log: &Mutex<TestLog>, request: &http::Request<B>) {
    let mut log = log.lock().unwrap();
    let has_session = request.headers().contains_key("mcp-session-id");
    match *request.method() {
        http::Method::POST => {
            log.posts += 1;
            if has_session {
                log.posts_with_session += 1;
            }
        }
        http::Method::GET => log.gets += 1,
        http::Method::DELETE => log.deletes += 1,
        _ => {}
    }
    log.user_agent = request
        .headers()
        .get(http::header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    log.custom_headers = request
        .headers()
        .iter()
        .filter(|(name, _)| name.as_str().starts_with("x-"))
        .filter_map(|(name, value)| Some((name.to_string(), value.to_str().ok()?.to_string())))
        .collect();
}
