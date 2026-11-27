//! 测试用的第三方 stdio MCP 服务器（`src/tests.rs` 起它；`cargo test -p kotone-remote-plugin` 会先编译它）。
//! 用 rmcp 自己的服务端，装成一个“别人写的”服务器：
//!
//! - 工具：`echo {text}`（标了只读，结果带 `structuredContent`，用 `ui://remote-test/page` 显示）、`add`（没标只读）、
//!   `refresh`（只给页面）、`hidden`（结果里藏着看不见的字符）、`change_tools`（之后多一个 `extra`，
//!   发 `notifications/tools/list_changed`）、`exit`（进程马上退出：测掉线）、`bad name`（名字不合规矩）；
//! - 资源：`ui://remote-test/page`（MCP App，`_meta.ui.csp` 声明一个 CDN）；`kotone://manifest`（Kotone 清单，
//!   插件标准 0.1）**不在** `resources/list` 里，只能 `resources/read`（宿主不该靠列表找它）；
//! - `serverInfo` 带 title 和图标。

use std::sync::atomic::{AtomicBool, Ordering};

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
    ListResourcesResult, ListToolsResult, PaginatedRequestParams, ReadResourceRequestParams,
    ReadResourceResponse, ReadResourceResult, ServerCapabilities, ServerConfig,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler, ServiceExt};
use serde_json::{json, Value};

const PAGE: &str = "ui://remote-test/page";
static CHANGED: AtomicBool = AtomicBool::new(false);

#[derive(Clone)]
struct Server;

fn text(t: impl Into<String>) -> CallToolResponse {
    CallToolResult::success(vec![ContentBlock::text(t.into())]).into()
}

impl ServerHandler for Server {
    fn get_info(&self) -> ServerConfig {
        let info: Implementation = serde_json::from_value(json!({
            "name": "remote-test",
            "title": "Remote\u{200B} Test Server",
            "version": "1.2.3",
            "icons": [{"src": "data:image/png;base64,iVBORw0KGgo=", "mimeType": "image/png"},
                      {"src": "javascript:alert(1)"}]
        }))
        .unwrap();
        ServerConfig::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_tool_list_changed()
                .enable_resources()
                .build(),
        )
        .with_server_info(info)
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let mut tools = vec![
            json!({"name": "echo", "description": "Echo the text back", "inputSchema": {"type": "object",
                   "properties": {"text": {"type": "string", "description": "what\u{E0041} to echo"}}},
                   "annotations": {"readOnlyHint": true}, "_meta": {"ui": {"resourceUri": PAGE}}}),
            json!({"name": "add", "description": "Add a note", "inputSchema": {"type": "object"}}),
            json!({"name": "refresh", "description": "Page button", "inputSchema": {"type": "object"},
                   "_meta": {"ui": {"visibility": ["app"]}}}),
            json!({"name": "hidden", "description": "Hidden text", "inputSchema": {"type": "object"}}),
            json!({"name": "change_tools", "description": "Change the tool list", "inputSchema": {"type": "object"}}),
            json!({"name": "exit", "description": "Quit the process", "inputSchema": {"type": "object"}}),
            json!({"name": "bad name", "description": "Not a valid name", "inputSchema": {"type": "object"}}),
        ];
        if CHANGED.load(Ordering::SeqCst) {
            tools.push(json!({"name": "extra", "description": "Added later", "inputSchema": {"type": "object"},
                              "annotations": {"readOnlyHint": true}}));
        }
        let tools = tools
            .into_iter()
            .map(|t| serde_json::from_value(t).unwrap())
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
                let t = request
                    .arguments
                    .as_ref()
                    .and_then(|a| a.get("text"))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let mut result = CallToolResult::success(vec![ContentBlock::text(t.clone())]);
                result.structured_content = Some(json!({"echo": t}));
                Ok(result.into())
            }
            "add" => Ok(text("added")),
            "refresh" => Ok(text("refreshed")),
            "extra" => Ok(text("extra")),
            "hidden" => Ok(text("visible\u{E0069}\u{E0067}\u{200B} part")),
            "change_tools" => {
                CHANGED.store(true, Ordering::SeqCst);
                let _ = context.peer.notify_tool_list_changed().await;
                Ok(text("ok"))
            }
            "exit" => std::process::exit(0),
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
            "title": "Test page",
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
        if request.uri == "kotone://manifest" {
            let manifest = json!({
                "standard": "0.1",
                "games": ["lol"],
                "events": [{"name": "echoed", "label": "回了一句", "data": {"echo": "回的话"}}],
                "watches": [{"event": "echoed", "tool": "echo", "arguments": {"text": "hi"},
                             "compare": "changed", "field": "echo"}],
                "risk": "测试用的风险说明",
            });
            let result: ReadResourceResult = serde_json::from_value(json!({"contents": [{
                "uri": "kotone://manifest",
                "mimeType": "application/json",
                "text": manifest.to_string(),
            }]}))
            .unwrap();
            return Ok(result.into());
        }
        if request.uri != PAGE {
            return Err(ErrorData::resource_not_found(request.uri.clone(), None));
        }
        let result: ReadResourceResult = serde_json::from_value(json!({"contents": [{
            "uri": PAGE,
            "mimeType": "text/html;profile=mcp-app",
            "text": "<!doctype html><p>remote test page</p>",
            "_meta": {"ui": {"csp": {"resourceDomains": ["https://cdn.example.com/x.js", "*"]}, "prefersBorder": true}},
        }]}))
        .unwrap();
        Ok(result.into())
    }
}

#[tokio::main]
async fn main() {
    let running = Server
        .serve(rmcp::transport::stdio())
        .await
        .expect("serve stdio");
    let _ = running.waiting().await;
}
