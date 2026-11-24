//! 一个第三方 MCP 服务器的连接和它告诉过 Kotone 的东西（工具、资源、名字和图标）。
//!
//! - **懒连接**：第一次用（琴音调它的工具、玩家打开它的页面）或开关打开时才连；连不上的话 [`RETRY_AFTER`] 之内
//!   再用马上回答“没连上”，不每次都等启动超时。玩家点“重新连接”不受这个限制。
//! - **掉线**：子进程退出、网络断了，下一次用的时候重新连；这一次的调用回答“这个插件现在没连上”（不说
//!   “没有这个工具”：一段对话里工具清单不变，ADR 024）。Kotone 退出时子进程跟着关（`kotone-mcp` 的 Job Object）。
//! - **工具清单记在磁盘上**（`<KOTONE_HOME>/plugins/<id>/remote-cache.json`）：Kotone 重开以后不用先把每个服务器
//!   都起一遍就知道它有哪些工具；配置变了（指纹不同）就不认旧的。
//! - **`tools/list_changed`**：服务器说工具变了就重新列，从下一段对话起生效（琴音的工具清单一段对话里不变）。

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use kotone_mcp::{McpClient, McpError, ServerInfo, Tool};
use kotone_plugin_api::resource::normalize_csp_source;
use kotone_plugin_api::{Host, Resource, ResourceUi, ToolResult, ToolSpec, APP_MIME};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::config::ServerSpec;
use crate::tools::{self, short_text};

/// 连不上以后多久之内不再自己重试。
pub const RETRY_AFTER: Duration = Duration::from_secs(15);
/// 记工具清单的文件（在插件自己的目录里）。
pub const CACHE_FILE: &str = "remote-cache.json";

/// 服务器说的自己（握手时的 `serverInfo`）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct About {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub icons: Vec<Value>,
    #[serde(default)]
    pub website_url: Option<String>,
}

impl From<&ServerInfo> for About {
    fn from(info: &ServerInfo) -> Self {
        Self {
            name: info.name.clone(),
            title: info.title.clone(),
            version: info.version.clone(),
            description: info.description.clone(),
            icons: info.icons.clone(),
            website_url: info.website_url.clone(),
        }
    }
}

/// 服务器告诉过 Kotone 的东西（记在磁盘上）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Known {
    /// 这份清单是哪一份配置连出来的（[`ServerSpec::fingerprint`]）。
    #[serde(default)]
    pub fingerprint: String,
    #[serde(default)]
    pub tools: Vec<Tool>,
    #[serde(default)]
    pub resources: Vec<kotone_mcp::Resource>,
    #[serde(default)]
    pub server: Option<About>,
}

/// 连接的状态（插件卡片上显示）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Status {
    /// 还没连（懒连接：第一次用时才连）。
    NotConnected,
    Connecting,
    Connected,
    /// 上一次连不上（`error` 里写了原因）。
    Failed,
}

struct State {
    known: Known,
    status: Status,
    error: Option<String>,
    retry_after: Option<Instant>,
    trusted: bool,
}

pub struct Server {
    id: String,
    spec: Result<ServerSpec, String>,
    fingerprint: String,
    cache: Option<PathBuf>,
    version: String,
    state: Mutex<State>,
    client: tokio::sync::Mutex<Option<Arc<McpClient>>>,
}

/// `ui://` 地址 → 回环代理路径里的一段（`r` + 16 位十六进制，FNV-1a 64）：别人的 `ui://` 地址什么字都可能有，
/// 代理的路径只认小写字母、数字和连字符。
pub fn page_slug(uri: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in uri.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("r{hash:016x}")
}

impl Server {
    /// `data_dir`：插件自己的目录（记工具清单）；None = 不记（测试）。
    pub fn new(
        id: &str,
        spec: Result<ServerSpec, String>,
        data_dir: Option<PathBuf>,
        version: &str,
    ) -> Self {
        let fingerprint = spec
            .as_ref()
            .map(ServerSpec::fingerprint)
            .unwrap_or_default();
        let cache = data_dir.map(|dir| dir.join(CACHE_FILE));
        let known = cache
            .as_ref()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|text| serde_json::from_str::<Known>(&text).ok())
            .filter(|k| !fingerprint.is_empty() && k.fingerprint == fingerprint)
            .unwrap_or_default();
        let error = spec.as_ref().err().map(|e| format!("配置不对：{e}"));
        Self {
            id: id.to_string(),
            spec,
            fingerprint,
            cache,
            version: version.to_string(),
            state: Mutex::new(State {
                known,
                status: if error.is_some() {
                    Status::Failed
                } else {
                    Status::NotConnected
                },
                error,
                retry_after: None,
                trusted: false,
            }),
            client: tokio::sync::Mutex::new(None),
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn spec(&self) -> &Result<ServerSpec, String> {
        &self.spec
    }

    /// 配置的指纹（配置写错了是空的）。
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }

    pub fn set_trusted(&self, trusted: bool) {
        self.state.lock().unwrap().trusted = trusted;
    }

    pub fn trusted(&self) -> bool {
        self.state.lock().unwrap().trusted
    }

    pub fn status(&self) -> Status {
        self.state.lock().unwrap().status
    }

    pub fn error(&self) -> Option<String> {
        self.state.lock().unwrap().error.clone()
    }

    pub fn known(&self) -> Known {
        self.state.lock().unwrap().known.clone()
    }

    /// 知道它有哪些工具了（连上过，或者磁盘上记着）。
    pub fn has_tools(&self) -> bool {
        !self.state.lock().unwrap().known.tools.is_empty()
    }

    /// 全部工具在 Kotone 里的样子（给她的和只给页面的；名字不合规矩的、重名的不要）。
    pub fn tool_specs(&self) -> Vec<ToolSpec> {
        let state = self.state.lock().unwrap();
        let mut seen = std::collections::HashSet::new();
        state
            .known
            .tools
            .iter()
            .filter(|t| seen.insert(t.name.clone()))
            .filter_map(|t| tools::spec(&self.id, t, state.trusted))
            .collect()
    }

    /// 它的 MCP App 页面：`(ui:// 地址, 标题)`，来自 `resources/list` 里的 `ui://` 资源和工具的 `_meta.ui.resourceUri`。
    pub fn app_pages(&self) -> Vec<(String, String)> {
        let known = self.known();
        let mut pages: Vec<(String, String)> = Vec::new();
        for resource in &known.resources {
            if resource.uri.starts_with("ui://") && !pages.iter().any(|(u, _)| *u == resource.uri) {
                let title = resource.title.as_deref().unwrap_or(&resource.name);
                pages.push((resource.uri.clone(), short_text(title, 40)));
            }
        }
        for tool in &known.tools {
            if let Some(uri) = tools::ui_meta(tool).1 {
                if !pages.iter().any(|(u, _)| *u == uri) {
                    let title = tool.title.as_deref().unwrap_or(&tool.name);
                    pages.push((uri, short_text(title, 40)));
                }
            }
        }
        pages
    }

    /// 这个 `ui://` 地址是不是这个服务器给的（归属按“哪个服务器给的”记，不按地址的写法）。
    pub fn owns_uri(&self, uri: &str) -> bool {
        self.app_pages().iter().any(|(u, _)| u == uri)
    }

    /// 回环代理路径里的一段 → 页面地址。
    pub fn uri_for_slug(&self, slug: &str) -> Option<String> {
        self.app_pages()
            .into_iter()
            .map(|(uri, _)| uri)
            .find(|uri| page_slug(uri) == slug)
    }

    /// 给界面的样子（插件卡片）：连接状态、错误、服务器说的自己（剥掉看不见的字符、限长）、页面、工具数。
    pub fn describe(&self) -> Value {
        let state = self.state.lock().unwrap();
        let server = state.known.server.as_ref().map(|about| {
            json!({
                "name": short_text(&about.name, 60),
                "title": about.title.as_deref().map(|t| short_text(t, 60)),
                "version": short_text(&about.version, 30),
                "description": about.description.as_deref().map(|d| short_text(d, 300)),
                "icons": about.icons.iter().filter(|i| icon_ok(i)).cloned().collect::<Vec<_>>(),
                "websiteUrl": about.website_url.as_deref().filter(|u| u.starts_with("https://") || u.starts_with("http://")),
            })
        });
        let skipped: Vec<String> = state
            .known
            .tools
            .iter()
            .map(|t| t.name.clone())
            .filter(|n| !tools::valid_tool_name(n))
            .collect();
        let tool_count = state.known.tools.len() - skipped.len();
        drop(state);
        let pages: Vec<Value> = self
            .app_pages()
            .into_iter()
            .map(|(uri, title)| json!({"uri": uri, "title": title, "slug": page_slug(&uri)}))
            .collect();
        let state = self.state.lock().unwrap();
        json!({
            "status": state.status,
            "error": state.error,
            "trusted": state.trusted,
            "server": server,
            "pages": pages,
            "toolCount": tool_count,
            "skippedTools": skipped,
        })
    }

    fn fail(&self, message: String, retry_later: bool) -> String {
        let mut state = self.state.lock().unwrap();
        state.status = Status::Failed;
        state.error = Some(message.clone());
        state.retry_after = retry_later.then(|| Instant::now() + RETRY_AFTER);
        message
    }

    fn remember(&self, known: Known) {
        if let Some(path) = &self.cache {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Ok(text) = serde_json::to_string(&known) {
                let _ = std::fs::write(path, text);
            }
        }
        self.state.lock().unwrap().known = known;
    }

    fn update_tools(&self, tools: Vec<Tool>) {
        let mut known = self.known();
        known.tools = tools;
        self.remember(known);
    }

    /// 连上（已经连着就用现有的）。`force`：不管上次失败后的等待（玩家点“重新连接”、刚打开开关），也重连已有的。
    pub async fn connect(
        self: &Arc<Self>,
        host: &Arc<dyn Host>,
        force: bool,
    ) -> Result<Arc<McpClient>, String> {
        let mut slot = self.client.lock().await;
        if let Some(client) = slot.as_ref() {
            if client.is_connected() && !force {
                return Ok(client.clone());
            }
        }
        if let Some(old) = slot.take() {
            old.close().await;
        }
        if !force {
            let state = self.state.lock().unwrap();
            if let (Some(until), Some(error)) = (state.retry_after, &state.error) {
                if Instant::now() < until {
                    return Err(error.clone());
                }
            }
        }
        let spec = match &self.spec {
            Ok(spec) => spec,
            Err(e) => return Err(self.fail(format!("配置不对：{e}"), false)),
        };
        let config = match spec.resolve(&|name: &str| host.secret(name)) {
            Ok(config) => config.client("kotone", &self.version),
            Err(missing) => {
                return Err(self.fail(
                    format!(
                        "还没填 Key：{}（在左侧栏「插件」里这个插件的卡片上填）",
                        missing.join("、")
                    ),
                    false,
                ))
            }
        };
        {
            let mut state = self.state.lock().unwrap();
            state.status = Status::Connecting;
            state.error = None;
        }
        let client = match McpClient::connect(&config).await {
            Ok(client) => Arc::new(client),
            Err(e) => return Err(self.fail(e.to_string(), true)),
        };
        let tools = match client.list_tools().await {
            Ok(tools) => tools,
            Err(e) => {
                client.close().await;
                return Err(self.fail(format!("连上了，但列不出工具：{e}"), true));
            }
        };
        // 不是每个服务器都有资源（没有时回“方法不存在”）
        let resources = client.list_resources().await.unwrap_or_default();
        self.remember(Known {
            fingerprint: self.fingerprint.clone(),
            tools,
            resources,
            server: Some(About::from(client.server_info())),
        });
        {
            let mut state = self.state.lock().unwrap();
            state.status = Status::Connected;
            state.error = None;
            state.retry_after = None;
        }
        self.watch(host, &client);
        *slot = Some(client.clone());
        Ok(client)
    }

    /// 服务器说工具变了（`notifications/tools/list_changed`）就重新列；连接断了这个任务就结束。
    fn watch(self: &Arc<Self>, host: &Arc<dyn Host>, client: &Arc<McpClient>) {
        let mut changes = client.tools_changed();
        let client = Arc::downgrade(client);
        let server = Arc::downgrade(self);
        host.spawn(Box::pin(async move {
            while changes.changed().await.is_ok() {
                let (Some(client), Some(server)) = (client.upgrade(), server.upgrade()) else {
                    break;
                };
                if let Ok(tools) = client.list_tools().await {
                    server.update_tools(tools);
                }
            }
        }));
    }

    /// 断开（开关关了、删掉了）。子进程连同它启动的进程一起结束。
    pub async fn disconnect(&self) {
        if let Some(client) = self.client.lock().await.take() {
            client.close().await;
        }
        let mut state = self.state.lock().unwrap();
        if state.status != Status::Failed || self.spec.is_ok() {
            state.status = Status::NotConnected;
            state.retry_after = None;
        }
    }

    /// 连接断了（这次调用发现的）：丢掉它，下一次用时重连。
    async fn lost(&self, client: &Arc<McpClient>, error: &McpError) {
        let mut slot = self.client.lock().await;
        if slot.as_ref().is_some_and(|c| Arc::ptr_eq(c, client)) {
            if let Some(old) = slot.take() {
                old.close().await;
            }
        }
        let mut state = self.state.lock().unwrap();
        state.status = Status::NotConnected;
        state.error = Some(error.to_string());
    }

    /// “这个插件现在没连上”的说法（琴音照着跟玩家说，原则 21：说下一步）。
    pub fn offline(&self, reason: &str) -> String {
        format!(
            "「{}」插件现在没连上（{reason}）。玩家可以在左侧栏的「插件」里重新连接",
            self.id
        )
    }

    /// 调一个工具（`tool` 是 Kotone 里的名字 `<id>.<原名>`）。
    pub async fn call(
        self: &Arc<Self>,
        host: &Arc<dyn Host>,
        tool: &str,
        arguments: &Value,
    ) -> Result<ToolResult, String> {
        let original = tools::server_name(&self.id, tool)
            .ok_or_else(|| format!("{tool} 不是「{}」插件的工具", self.id))?
            .to_string();
        let arguments = match arguments {
            Value::Null | Value::Object(_) => arguments.clone(),
            _ => return Err("参数要是一个 JSON 对象".into()),
        };
        let page = self
            .known()
            .tools
            .iter()
            .find(|t| t.name == original)
            .and_then(|t| tools::ui_meta(t).1);
        let client = self
            .connect(host, false)
            .await
            .map_err(|e| self.offline(&e))?;
        match client.call_tool(&original, arguments).await {
            Ok(result) => tools::to_result(result, page),
            Err(
                error @ (McpError::Closed
                | McpError::Connect(_)
                | McpError::Spawn(_)
                | McpError::Http(_)),
            ) => {
                self.lost(&client, &error).await;
                Err(self.offline(&error.to_string()))
            }
            Err(McpError::Server { message, .. }) => Err(format!(
                "「{}」插件的 {original} 出错：{}",
                self.id,
                short_text(&message, 500)
            )),
            Err(error) => Err(format!("「{}」插件的 {original} 出错：{error}", self.id)),
        }
    }

    /// 读它的一个 MCP App 页面（`resources/read`）。CSP 用服务器声明的（内容里的 `_meta.ui` 优先，
    /// 没有就用 `resources/list` 里那一项的），写法不对的声明丢掉。
    pub async fn read_app(
        self: &Arc<Self>,
        host: &Arc<dyn Host>,
        uri: &str,
    ) -> Result<Resource, String> {
        if !self.owns_uri(uri) {
            return Err(format!("「{}」插件没有页面 {uri}", self.id));
        }
        let client = self
            .connect(host, false)
            .await
            .map_err(|e| self.offline(&e))?;
        let contents = client
            .read_resource(uri)
            .await
            .map_err(|e| format!("读不了 {uri}：{e}"))?;
        let item = contents
            .into_iter()
            .find(|c| c.text.is_some())
            .ok_or_else(|| format!("{uri} 不是网页（没有文字内容）"))?;
        let mime = item
            .mime_type
            .as_deref()
            .unwrap_or("")
            .to_ascii_lowercase()
            .replace(' ', "");
        if !mime.starts_with("text/html") {
            return Err(format!("{uri} 不是 MCP App 的网页（类型是 {mime}）"));
        }
        let listed = self
            .known()
            .resources
            .into_iter()
            .find(|r| r.uri == uri)
            .and_then(|r| r.meta);
        let ui = resource_ui(item.meta.as_ref())
            .or_else(|| resource_ui(listed.as_ref()))
            .unwrap_or_default();
        Ok(Resource {
            uri: uri.to_string(),
            mime_type: APP_MIME.to_string(),
            text: item.text.unwrap_or_default(),
            ui,
        })
    }
}

/// `_meta.ui` → 资源的显示要求（CSP、边框）；声明写法不对的丢掉。
fn resource_ui(meta: Option<&Value>) -> Option<ResourceUi> {
    let ui = meta?.get("ui")?;
    let mut parsed: ResourceUi = serde_json::from_value(ui.clone()).ok()?;
    for list in [
        &mut parsed.csp.connect_domains,
        &mut parsed.csp.resource_domains,
        &mut parsed.csp.frame_domains,
        &mut parsed.csp.base_uri_domains,
    ] {
        *list = list
            .iter()
            .filter_map(|d| normalize_csp_source(d))
            .collect();
    }
    Some(parsed)
}

/// 图标只认 `https:` 和 `data:image/` 的地址（规范：宿主应拒绝别的协议）。
fn icon_ok(icon: &Value) -> bool {
    icon.get("src").and_then(Value::as_str).is_some_and(|src| {
        src.len() <= 200_000 && (src.starts_with("https://") || src.starts_with("data:image/"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_are_proxy_safe_and_stable() {
        let a = page_slug("ui://everything/app.html?x=中文");
        assert_eq!(a.len(), 17);
        assert!(a
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()));
        assert_eq!(a, page_slug("ui://everything/app.html?x=中文"));
        assert_ne!(a, page_slug("ui://everything/other.html"));
    }

    #[test]
    fn declared_csp_is_kept_and_bad_entries_dropped() {
        let meta = json!({"ui": {"csp": {
            "resourceDomains": ["https://cdn.example.com/lib.js", "*", "'unsafe-eval'"],
            "connectDomains": ["wss://live.example.com"]
        }, "prefersBorder": false, "domain": "x.example.com"}});
        let ui = resource_ui(Some(&meta)).unwrap();
        assert_eq!(ui.csp.resource_domains, vec!["https://cdn.example.com"]);
        assert_eq!(ui.csp.connect_domains, vec!["wss://live.example.com"]);
        assert_eq!(ui.prefers_border, Some(false));
        assert!(resource_ui(Some(&json!({}))).is_none());
        assert!(icon_ok(&json!({"src": "https://x.example.com/i.png"})));
        assert!(icon_ok(&json!({"src": "data:image/png;base64,AAAA"})));
        assert!(!icon_ok(&json!({"src": "javascript:alert(1)"})));
        assert!(!icon_ok(&json!({"src": "file:///c:/x.png"})));
    }
}
