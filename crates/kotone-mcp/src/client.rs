//! 一个连上的 MCP 服务器。
//!
//! 握手用 `initialize`（协议 2025-11-25；服务器只会旧版本的，回它自己的版本，照样能用）。Goose 用 rmcp 的
//! `Auto`（先发 2026-07-28 的 `server/discover`，不认再退回 `initialize`）；这里先不用：现在几乎所有服务器都还是旧版，
//! 而不认识 `server/discover` 又不回话的 stdio 服务器要等 10 秒才退回，正好撞上启动超时。等服务器普遍到 2026-07-28 再换
//! （双向门：只改 [`McpClient::connect`] 里的一行）。

use std::sync::Arc;
use std::time::Duration;

use rmcp::model::{
    CallToolRequestParams, ClientCapabilities, ClientConfig, Implementation, ProtocolVersion,
    ReadResourceRequestParams,
};
use rmcp::service::{NotificationContext, RunningService};
use rmcp::{ClientHandler, Peer, RoleClient, ServiceExt};
use serde_json::Value;
use tokio::sync::{watch, Mutex};

use crate::config::{ServerConfig, Transport};
use crate::error::{McpError, Stage};
use crate::stdio::{self, ServerProcess};
use crate::types::{convert, Resource, ResourceContents, ServerInfo, Tool, ToolCallResult};

/// 关连接时等 rmcp 收尾（关传输、HTTP 的 DELETE 会话）最多多久。
const CLOSE_GRACE: Duration = Duration::from_secs(2);

/// 客户端这一侧：报“我是谁”，收服务器的通知。
struct Handler {
    info: ClientConfig,
    tools_changed: watch::Sender<u64>,
}

impl ClientHandler for Handler {
    fn get_info(&self) -> ClientConfig {
        self.info.clone()
    }

    async fn on_tool_list_changed(&self, _context: NotificationContext<RoleClient>) {
        self.tools_changed.send_modify(|n| *n = n.wrapping_add(1));
    }
}

pub struct McpClient {
    peer: Peer<RoleClient>,
    running: Mutex<Option<RunningService<RoleClient, Handler>>>,
    process: Mutex<Option<ServerProcess>>,
    process_id: Option<u32>,
    info: ServerInfo,
    tool_timeout: Duration,
    tools_changed: watch::Receiver<u64>,
}

impl McpClient {
    /// 连上（子进程：起程序 + 握手；HTTP：握手），超过 `startup_timeout` 算失败（子进程会被结束）。
    pub async fn connect(config: &ServerConfig) -> Result<Self, McpError> {
        let (tx, rx) = watch::channel(0u64);
        let handler = Handler {
            info: ClientConfig::new(
                ClientCapabilities::default(),
                Implementation::new(config.client_name.clone(), config.client_version.clone()),
            )
            .with_protocol_version(ProtocolVersion::LATEST_WITH_INITIALIZE),
            tools_changed: tx,
        };
        let startup = config.startup_timeout;
        let timed_out = McpError::Timeout {
            stage: Stage::Startup,
            after: startup,
        };
        match &config.transport {
            Transport::Stdio {
                command,
                args,
                env,
                cwd,
            } => {
                let spawned = stdio::spawn(command, args, env, cwd.as_deref())?;
                let mut process = spawned.process;
                let started =
                    tokio::time::timeout(startup, handler.serve((spawned.stdout, spawned.stdin)))
                        .await;
                let failure = match started {
                    Ok(Ok(running)) => {
                        let process_id = process.id();
                        return Ok(Self::new(running, Some(process), process_id, config, rx));
                    }
                    Ok(Err(error)) => McpError::from(error),
                    Err(_) => timed_out,
                };
                let note = process.failure_note().await;
                process.kill().await;
                Err(with_note(failure, &note))
            }
            Transport::Http { url, headers } => {
                let transport =
                    crate::http_transport::transport(url, headers, &config.client_name)?;
                match tokio::time::timeout(startup, handler.serve(transport)).await {
                    Ok(Ok(running)) => Ok(Self::new(running, None, None, config, rx)),
                    Ok(Err(error)) => Err(McpError::from(error)),
                    Err(_) => Err(timed_out),
                }
            }
        }
    }

    fn new(
        running: RunningService<RoleClient, Handler>,
        process: Option<ServerProcess>,
        process_id: Option<u32>,
        config: &ServerConfig,
        tools_changed: watch::Receiver<u64>,
    ) -> Self {
        let info = running
            .peer_info()
            .map(|p| {
                let server = p.server_info.as_ref();
                ServerInfo {
                    name: server.map(|s| s.name.clone()).unwrap_or_default(),
                    version: server.map(|s| s.version.clone()).unwrap_or_default(),
                    title: server.and_then(|s| s.title.clone()),
                    description: server.and_then(|s| s.description.clone()),
                    // 图标经 JSON 交出去（协议原样的字段），不露 rmcp 的类型
                    icons: server
                        .and_then(|s| s.icons.as_ref())
                        .and_then(|icons| serde_json::to_value(icons).ok())
                        .and_then(|v| v.as_array().cloned())
                        .unwrap_or_default(),
                    website_url: server.and_then(|s| s.website_url.clone()),
                    instructions: p.instructions.clone(),
                    protocol_version: p.protocol_version.to_string(),
                }
            })
            .unwrap_or_default();
        Self {
            peer: running.peer().clone(),
            running: Mutex::new(Some(running)),
            process: Mutex::new(process),
            process_id,
            info,
            tool_timeout: config.tool_timeout,
            tools_changed,
        }
    }

    /// 握手时服务器说的自己。
    pub fn server_info(&self) -> &ServerInfo {
        &self.info
    }

    /// 子进程服务器的进程号（HTTP 的没有）。
    pub fn process_id(&self) -> Option<u32> {
        self.process_id
    }

    /// 连接还在（程序没退出、没调过 [`McpClient::close`]）。
    pub fn is_connected(&self) -> bool {
        !self.peer.is_transport_closed()
    }

    /// 服务器每发一次 `notifications/tools/list_changed`，这里的数加一。
    pub fn tools_changed(&self) -> watch::Receiver<u64> {
        self.tools_changed.clone()
    }

    async fn request<T, F>(&self, future: F) -> Result<T, McpError>
    where
        F: std::future::Future<Output = Result<T, rmcp::ServiceError>>,
    {
        if !self.is_connected() {
            return Err(McpError::Closed);
        }
        // 超时的时候丢掉这个请求，rmcp 会给服务器发 notifications/cancelled
        match tokio::time::timeout(self.tool_timeout, future).await {
            Ok(result) => result.map_err(McpError::from),
            Err(_) => Err(McpError::Timeout {
                stage: Stage::Request,
                after: self.tool_timeout,
            }),
        }
    }

    /// 所有工具（翻完所有页）。
    pub async fn list_tools(&self) -> Result<Vec<Tool>, McpError> {
        let tools = self.request(self.peer.list_all_tools()).await?;
        tools.iter().map(convert).collect()
    }

    /// 调一个工具。`arguments` 是 JSON 对象或 `null`。
    pub async fn call_tool(
        &self,
        name: &str,
        arguments: Value,
    ) -> Result<ToolCallResult, McpError> {
        let mut params = CallToolRequestParams::new(name.to_string());
        match arguments {
            Value::Null => {}
            Value::Object(map) => params = params.with_arguments(map),
            _ => return Err(McpError::Config("工具的参数要是一个 JSON 对象".into())),
        }
        let result = self.request(self.peer.call_tool(params)).await?;
        convert(&result)
    }

    /// 所有资源（翻完所有页）。
    pub async fn list_resources(&self) -> Result<Vec<Resource>, McpError> {
        let resources = self.request(self.peer.list_all_resources()).await?;
        resources.iter().map(convert).collect()
    }

    /// 读一个资源（例如 MCP Apps 的 `ui://…` 页面）。
    pub async fn read_resource(&self, uri: &str) -> Result<Vec<ResourceContents>, McpError> {
        let result = self
            .request(
                self.peer
                    .read_resource(ReadResourceRequestParams::new(uri.to_string())),
            )
            .await?;
        result.contents.iter().map(convert).collect()
    }

    /// 关掉：rmcp 收尾（关标准输入 / HTTP 结束会话），子进程等它自己退出，然后连同它启动的进程一起结束。
    /// 关过以后的请求都回 [`McpError::Closed`]。不调这个、直接丢掉也行：Job 的句柄关掉时进程一样会结束。
    pub async fn close(&self) {
        if let Some(mut running) = self.running.lock().await.take() {
            let _ = running.close_with_timeout(CLOSE_GRACE).await;
        }
        if let Some(process) = self.process.lock().await.take() {
            process.shutdown().await;
        }
    }
}

/// 把启动失败时子进程的说明接在错误后面。
fn with_note(error: McpError, note: &str) -> McpError {
    match error {
        // 握手前输出就关了：多半是程序自己退出了，它的 stderr 最能说明原因
        McpError::Closed if note.is_empty() => {
            McpError::Spawn("程序启动后还没握手就关掉了输出".into())
        }
        McpError::Closed => McpError::Spawn(note.to_string()),
        McpError::Spawn(m) if !note.is_empty() => McpError::Spawn(format!("{m}。{note}")),
        other => other,
    }
}

impl std::fmt::Debug for McpClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpClient")
            .field("server", &self.info)
            .field("process_id", &self.process_id)
            .finish()
    }
}

// McpClient 在插件里放进 Arc 跨任务共用。
const _: fn() = || {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Arc<McpClient>>();
};
