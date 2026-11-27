//! “老插件”的假服务器（`tests/old_plugins.rs` 起它；`cargo test -p kotone-remote-plugin` 会先编译它）：照一份 JSON 快照回答，
//! 装成某个按旧标准写的插件。用法：`old_plugin_server <snapshot.json>`（stdio）。
//!
//! 快照（`tests/old-plugins/<名字>/snapshot.json`）里的字段，都是协议原样的 JSON：
//!
//! - `serverInfo`：握手时说的自己（`Implementation`）；
//! - `tools`：`tools/list` 的全部工具；`resources`：`resources/list` 的全部资源（没有就是空的）；
//! - `reads`：`resources/read` 的结果，地址 → `contents` 数组（`kotone://manifest` 也在这里，原文照录）；
//! - `calls`：`tools/call`，`{tool, arguments, results}`：名字和参数对得上的那一条，第 n 次调用回 `results[n]`，
//!   用完了一直回最后一份（观察项第一次记底、第二次换一份结果、之后不再变）。对不上的调用回 JSON-RPC 错误。
//!
//! 别的字段（`about` 写快照的出处）不管。

use std::sync::{Arc, Mutex};

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, Implementation, ListResourcesResult,
    ListToolsResult, PaginatedRequestParams, ReadResourceRequestParams, ReadResourceResponse,
    ReadResourceResult, ServerCapabilities, ServerConfig,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler, ServiceExt};
use serde_json::{json, Map, Value};

#[derive(Clone)]
struct Snapshot {
    data: Arc<Value>,
    /// 每一条 `calls` 被调了几次。
    counts: Arc<Mutex<Vec<usize>>>,
}

fn list(value: &Value, key: &str) -> Vec<Value> {
    value
        .get(key)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

/// 参数一样（不看键的顺序；没写参数 = `{}`）。
fn same_arguments(recorded: Option<&Value>, sent: &Map<String, Value>) -> bool {
    match recorded {
        None | Some(Value::Null) => sent.is_empty(),
        Some(Value::Object(map)) => map == sent,
        Some(_) => false,
    }
}

impl ServerHandler for Snapshot {
    fn get_info(&self) -> ServerConfig {
        let info: Implementation = serde_json::from_value(
            self.data
                .get("serverInfo")
                .cloned()
                .unwrap_or_else(|| json!({"name": "old-plugin", "version": "0.0.0"})),
        )
        .expect("快照的 serverInfo 写法不对");
        ServerConfig::new(
            ServerCapabilities::builder()
                .enable_tools()
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
        let tools = list(&self.data, "tools")
            .into_iter()
            .map(|t| serde_json::from_value(t).expect("快照的工具写法不对"))
            .collect();
        Ok(ListToolsResult::with_all_items(tools))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let sent = request.arguments.clone().unwrap_or_default();
        let calls = list(&self.data, "calls");
        let Some(index) = calls.iter().position(|c| {
            c.get("tool").and_then(Value::as_str) == Some(request.name.as_ref())
                && same_arguments(c.get("arguments"), &sent)
        }) else {
            return Err(ErrorData::invalid_params(
                format!(
                    "快照里没有这次调用：{} {}",
                    request.name,
                    Value::Object(sent)
                ),
                None,
            ));
        };
        let results = list(&calls[index], "results");
        let n = {
            let mut counts = self.counts.lock().unwrap();
            if counts.len() < calls.len() {
                counts.resize(calls.len(), 0);
            }
            counts[index] += 1;
            counts[index] - 1
        };
        let Some(result) = results.get(n).or_else(|| results.last()) else {
            return Err(ErrorData::internal_error("快照里这次调用没有结果", None));
        };
        let result: CallToolResult =
            serde_json::from_value(result.clone()).expect("快照的结果写法不对");
        Ok(result.into())
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        let resources = list(&self.data, "resources")
            .into_iter()
            .map(|r| serde_json::from_value(r).expect("快照的资源写法不对"))
            .collect();
        Ok(ListResourcesResult::with_all_items(resources))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, ErrorData> {
        let Some(contents) = self.data.get("reads").and_then(|r| r.get(&request.uri)) else {
            return Err(ErrorData::resource_not_found(request.uri.clone(), None));
        };
        let result: ReadResourceResult = serde_json::from_value(json!({ "contents": contents }))
            .expect("快照的资源内容写法不对");
        Ok(result.into())
    }
}

#[tokio::main]
async fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("用法：old_plugin_server <snapshot.json>");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("读不了 {path}：{e}"));
    let data: Value =
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("{path} 不是 JSON：{e}"));
    let server = Snapshot {
        data: Arc::new(data),
        counts: Arc::new(Mutex::new(Vec::new())),
    };
    let running = server
        .serve(rmcp::transport::stdio())
        .await
        .expect("serve stdio");
    let _ = running.waiting().await;
}
