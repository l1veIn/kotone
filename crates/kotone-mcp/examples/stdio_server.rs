//! 测试用的 stdio MCP 服务器（`tests/stdio.rs` 起它；`cargo test -p kotone-mcp` 会先编译它）。用 rmcp 自己的服务端。
//!
//! - 工具：`echo {text}`、`sleep {ms}`、`pid`、`spawn_child`（起一个一直跑的子进程，回它的进程号）、
//!   `env {name}`、`argv`、`change_tools`（发 `notifications/tools/list_changed`）、`fail`（工具自己说失败）；
//! - 资源：`ui://kotone-test/page`（MCP Apps 的页面类型）；
//! - `--hang`：什么都不回（测启动超时）；`--crash`：往 stderr 写一句就退出（测启动失败的说明）。

use std::time::Duration;

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
    ListResourcesResult, ListToolsResult, PaginatedRequestParams, ReadResourceRequestParams,
    ReadResourceResponse, ReadResourceResult, ServerCapabilities, ServerConfig,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler, ServiceExt};
use serde_json::{json, Value};

const PAGE: &str = "ui://kotone-test/page";

#[derive(Clone)]
struct Server;

fn text(t: impl Into<String>) -> CallToolResponse {
    CallToolResult::success(vec![ContentBlock::text(t.into())]).into()
}

fn arg<'a>(request: &'a CallToolRequestParams, name: &str) -> Option<&'a Value> {
    request.arguments.as_ref().and_then(|a| a.get(name))
}

impl ServerHandler for Server {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_tool_list_changed()
                .enable_resources()
                .build(),
        )
        .with_server_info(Implementation::new("kotone-stdio-test", "1.0.0"))
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let names = [
            "echo",
            "sleep",
            "pid",
            "spawn_child",
            "env",
            "argv",
            "change_tools",
            "fail",
        ];
        let tools = names
            .iter()
            .map(|name| {
                serde_json::from_value(json!({
                    "name": name,
                    "description": format!("test tool {name}"),
                    "inputSchema": {"type": "object"},
                    "annotations": {"readOnlyHint": *name == "echo"},
                }))
                .unwrap()
            })
            .collect();
        Ok(ListToolsResult::with_all_items(tools))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        match request.name.as_ref() {
            "echo" => {
                let t = arg(&request, "text")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let mut result = CallToolResult::success(vec![ContentBlock::text(t.clone())]);
                result.structured_content = Some(json!({"echo": t}));
                Ok(result.into())
            }
            "sleep" => {
                let ms = arg(&request, "ms").and_then(Value::as_u64).unwrap_or(1000);
                tokio::time::sleep(Duration::from_millis(ms)).await;
                Ok(text("slept"))
            }
            "pid" => Ok(text(std::process::id().to_string())),
            "spawn_child" => {
                let mut cmd = std::process::Command::new("ping");
                cmd.args(["-n", "120", "127.0.0.1"])
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null());
                #[cfg(windows)]
                {
                    use std::os::windows::process::CommandExt;
                    cmd.creation_flags(0x0800_0000);
                }
                let child = cmd
                    .spawn()
                    .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
                let pid = child.id();
                std::mem::forget(child);
                Ok(text(pid.to_string()))
            }
            "env" => {
                let name = arg(&request, "name")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                Ok(text(
                    std::env::var(name).unwrap_or_else(|_| "<unset>".to_string()),
                ))
            }
            "argv" => {
                let args: Vec<String> = std::env::args().skip(1).collect();
                Ok(text(serde_json::to_string(&args).unwrap()))
            }
            "change_tools" => {
                let _ = context.peer.notify_tool_list_changed().await;
                Ok(text("ok"))
            }
            "fail" => {
                Ok(CallToolResult::error(vec![ContentBlock::text("no such champion")]).into())
            }
            other => Err(ErrorData::invalid_params(
                format!("unknown tool {other}"),
                None,
            )),
        }
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        let page = serde_json::from_value(json!({
            "uri": PAGE,
            "name": "page",
            "mimeType": "text/html;profile=mcp-app",
        }))
        .unwrap();
        Ok(ListResourcesResult::with_all_items(vec![page]))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, ErrorData> {
        if request.uri != PAGE {
            return Err(ErrorData::resource_not_found(request.uri.clone(), None));
        }
        let result: ReadResourceResult = serde_json::from_value(json!({"contents": [{
            "uri": PAGE,
            "mimeType": "text/html;profile=mcp-app",
            "text": "<!doctype html><p>hi</p>",
            "_meta": {"ui": {"csp": {"connectDomains": []}}},
        }]}))
        .unwrap();
        Ok(result.into())
    }
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--crash") {
        eprintln!("boom: KOTONE_TEST_TOKEN is missing");
        std::process::exit(3);
    }
    if args.iter().any(|a| a == "--hang") {
        tokio::time::sleep(Duration::from_secs(3600)).await;
        return;
    }
    let running = Server
        .serve(rmcp::transport::stdio())
        .await
        .expect("serve stdio");
    let _ = running.waiting().await;
}
