//! 一个第三方 MCP 服务器的连接和它告诉过 Kotone 的东西（工具、资源、名字和图标）。
//!
//! - **懒连接**：第一次用（琴音调它的工具、玩家打开它的页面）或开关打开时才连；连不上的话 [`RETRY_AFTER`] 之内
//!   再用马上回答“没连上”，不每次都等启动超时。玩家点“重新连接”不受这个限制。
//! - **掉线**：子进程退出、网络断了，下一次用的时候重新连；这一次的调用回答“这个插件现在没连上”（不说
//!   “没有这个工具”：一段对话里工具清单不变，ADR 024）。Kotone 退出时子进程跟着关（`kotone-mcp` 的 Job Object）。
//! - **工具清单记在磁盘上**（`<KOTONE_HOME>/plugins/<id>/remote-cache.json`）：Kotone 重开以后不用先把每个服务器
//!   都起一遍就知道它有哪些工具；配置变了（指纹不同）就不认旧的。
//! - **`tools/list_changed`**：服务器说工具变了就重新列，从下一段对话起生效（琴音的工具清单一段对话里不变）。
//! - **Kotone 清单**（插件标准 0.1，[`crate::manifest`]）：每次连上都 `resources/read kotone://manifest`（不看
//!   `resources/list` 列没列、不看握手的返回），读到的原文和工具清单一起记在磁盘上，没连上时也能显示；
//!   读不到（资源不存在、方法不存在、超时）= 它没给。变了的话 [`Server::manifest_changes`] 的计数加一。

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use kotone_mcp::{McpClient, McpError, ServerInfo, Tool, ToolCallResult};
use kotone_plugin_api::resource::public_csp_source;
use kotone_plugin_api::{Host, Resource, ResourceUi, ToolResult, ToolSpec, APP_MIME};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::config::ServerSpec;
use crate::manifest::{self, Effective};
use crate::tools::{self, short_text};

/// 连不上以后多久之内不再自己重试。
pub const RETRY_AFTER: Duration = Duration::from_secs(15);
/// 记工具清单的文件（在插件自己的目录里）。
pub const CACHE_FILE: &str = "remote-cache.json";
/// 读 `kotone://manifest` 最多等多久：不认这个资源又不回话的服务器不能把连接拖到请求超时（60 秒）。
pub const MANIFEST_TIMEOUT: Duration = Duration::from_secs(5);

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
    /// 服务器给的 Kotone 清单的原文（`kotone://manifest`；没给 = None）。存原文不存读出来的样子：
    /// 以后的 Kotone 认识更多字段时照样从原文读。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest: Option<String>,
    /// 服务器给了、但用不了的原因（太大、不是文字），原文不存。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest_error: Option<String>,
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
    /// 这次连接填进去的 Key（`${input:名字}` 的值）：出错的话里出现了就换成 `***`（[`Server::scrub`]）。
    secrets: Vec<String>,
}

pub struct Server {
    id: String,
    spec: Result<ServerSpec, String>,
    fingerprint: String,
    /// 插件自己的目录（工具清单的缓存、玩家本地的 `manifest.json`）。
    dir: Option<PathBuf>,
    cache: Option<PathBuf>,
    /// 磁盘上记着的工具清单是另一份配置连出来的：那份配置的指纹（[`Server::replaced_fingerprint`]）。
    replaced: Option<String>,
    version: String,
    state: Mutex<State>,
    client: tokio::sync::Mutex<Option<Arc<McpClient>>>,
    /// 服务器给的清单变了一次就加一（宿主据此重建插件的 `Manifest`：`games` 跟着清单）。
    manifest_changed: tokio::sync::watch::Sender<u64>,
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
        let cache = data_dir.as_ref().map(|dir| dir.join(CACHE_FILE));
        let stored = cache
            .as_ref()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|text| serde_json::from_str::<Known>(&text).ok());
        let replaced = stored
            .as_ref()
            .map(|k| k.fingerprint.clone())
            .filter(|old| !old.is_empty() && !fingerprint.is_empty() && *old != fingerprint);
        let known = stored
            .filter(|k| !fingerprint.is_empty() && k.fingerprint == fingerprint)
            .unwrap_or_default();
        let error = spec.as_ref().err().map(|e| format!("配置不对：{e}"));
        Self {
            id: id.to_string(),
            spec,
            fingerprint,
            dir: data_dir,
            cache,
            replaced,
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
                secrets: Vec::new(),
            }),
            client: tokio::sync::Mutex::new(None),
            manifest_changed: tokio::sync::watch::channel(0).0,
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

    /// 这个插件上次连上时用的是另一份配置（程序、参数、网址改过，或者删了又加了一个同名的）：那份配置的指纹。
    /// 看的是磁盘上记着的工具清单是哪份配置连出来的（Kotone 关着时改的也认得出）。宿主据此清掉“总是允许”。
    pub fn replaced_fingerprint(&self) -> Option<&str> {
        self.replaced.as_deref()
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

    /// 生效的 Kotone 清单（[`manifest::merge`]）：服务器上次给的（记在磁盘上的原文）→ 没给时 Kotone 带的预设
    /// （按网址、服务器自报的名字认）→ 加上玩家本地的 `manifest.json`。每次现读本地文件（玩家手改了马上看得到）。
    pub fn kotone_manifest(&self) -> Effective {
        let known = self.known();
        let server = match (known.manifest, known.manifest_error) {
            (Some(text), _) => Some(manifest::parse_text(&text)),
            (None, Some(error)) => Some(Err(error)),
            (None, None) => None,
        };
        let url = self.spec.as_ref().ok().and_then(ServerSpec::url);
        let name = known.server.as_ref().map(|about| about.name.as_str());
        let preset = manifest::preset_for(url, name).map(|(_, parsed)| parsed);
        let local = self.dir.as_deref().and_then(manifest::read_local);
        let mut out = manifest::merge(server, preset, local);
        if out.source == manifest::Source::Preset {
            out.preset = manifest::preset_id(url, name);
        }
        out
    }

    /// 插件自己的目录（玩家本地的 `manifest.json` 在这里；测试里可能没有）。
    pub fn dir(&self) -> Option<&std::path::Path> {
        self.dir.as_deref()
    }

    /// 服务器给的清单（或者它自报的名字，预设按它认）每变一次，这里的数加一。
    pub fn manifest_changes(&self) -> tokio::sync::watch::Receiver<u64> {
        self.manifest_changed.subscribe()
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

    /// 出错的话里去掉填进去的 Key。这些话会进插件卡片，也会经“没连上”发给模型（[`Server::offline`]）：
    /// 连不上时 HTTP 库的错误、程序在 stderr 说的话、服务器报的错都可能带着地址或参数里的 Key。
    pub fn scrub(&self, text: &str) -> String {
        redact(text, &self.state.lock().unwrap().secrets)
    }

    fn fail(&self, message: String, retry_later: bool) -> String {
        let message = self.scrub(&message);
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
            Ok(config) => {
                self.state.lock().unwrap().secrets =
                    spec.keys().iter().filter_map(|n| host.secret(n)).collect();
                config.client("kotone", &self.version)
            }
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
        let (manifest, manifest_error) = read_manifest(&client).await;
        let before = self.known();
        let server = About::from(client.server_info());
        let changed = before.manifest != manifest
            || before.manifest_error != manifest_error
            || before.server.as_ref().map(|s| &s.name) != Some(&server.name);
        self.remember(Known {
            fingerprint: self.fingerprint.clone(),
            tools,
            resources,
            server: Some(server),
            manifest,
            manifest_error,
        });
        if changed {
            self.manifest_changed
                .send_modify(|n| *n = n.wrapping_add(1));
        }
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
        let message = self.scrub(&error.to_string());
        let mut state = self.state.lock().unwrap();
        state.status = Status::NotConnected;
        state.error = Some(message);
    }

    /// “这个插件现在没连上”的说法（琴音照着跟玩家说，原则 21：说下一步）。
    pub fn offline(&self, reason: &str) -> String {
        format!(
            "「{}」插件现在没连上（{}）。玩家可以在左侧栏的「插件」里重新连接",
            self.id,
            self.scrub(reason)
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
        let result = self.call_raw(host, &original, arguments).await?;
        tools::to_result(result, page)
    }

    /// 按服务器上的原名调一个工具，结果原样交回（观察项要 `structuredContent`，[`crate::watch`]）。
    /// 连不上、掉线、服务器报错都变成一句给人看的话（Key 换成 `***`）。
    pub async fn call_raw(
        self: &Arc<Self>,
        host: &Arc<dyn Host>,
        original: &str,
        arguments: Value,
    ) -> Result<ToolCallResult, String> {
        let client = self
            .connect(host, false)
            .await
            .map_err(|e| self.offline(&e))?;
        match client.call_tool(original, arguments).await {
            Ok(result) => Ok(result),
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
                short_text(&self.scrub(&message), 500)
            )),
            Err(error) => Err(format!(
                "「{}」插件的 {original} 出错：{}",
                self.id,
                self.scrub(&error.to_string())
            )),
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
            .map_err(|e| self.scrub(&format!("读不了 {uri}：{e}")))?;
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

/// 读服务器给的 Kotone 清单（`resources/read kotone://manifest`）：`(原文, 用不了的原因)`。
/// 资源不存在（-32002）、方法不存在、别的错、超时 = 没给（两个都是 None），不算连接出错。
async fn read_manifest(client: &McpClient) -> (Option<String>, Option<String>) {
    let read = tokio::time::timeout(
        MANIFEST_TIMEOUT,
        client.read_resource(manifest::MANIFEST_URI),
    )
    .await;
    let Ok(Ok(contents)) = read else {
        return (None, None);
    };
    let item = contents
        .iter()
        .find(|c| c.uri == manifest::MANIFEST_URI && c.text.is_some())
        .or_else(|| contents.iter().find(|c| c.text.is_some()));
    match item.and_then(|c| c.text.as_ref()) {
        Some(text) if text.len() > manifest::MAX_BYTES => (
            None,
            Some(format!(
                "服务器给的清单太大了（{} KiB，最多 64 KiB），当没给",
                text.len().div_ceil(1024)
            )),
        ),
        Some(text) => (Some(text.clone()), None),
        None if contents.is_empty() => (None, None),
        None => (
            None,
            Some("服务器给的清单不是文字（要是 application/json 的文字内容），当没给".into()),
        ),
    }
}

/// `_meta.ui` → 资源的显示要求（CSP、边框）；声明写法不对的、本机和局域网的地址丢掉（[`public_csp_source`]：
/// App 跑在回环代理上，别人的页面不能借它碰你电脑上、局域网里的服务）。
fn resource_ui(meta: Option<&Value>) -> Option<ResourceUi> {
    let ui = meta?.get("ui")?;
    let mut parsed: ResourceUi = serde_json::from_value(ui.clone()).ok()?;
    for list in [
        &mut parsed.csp.connect_domains,
        &mut parsed.csp.resource_domains,
        &mut parsed.csp.frame_domains,
        &mut parsed.csp.base_uri_domains,
    ] {
        *list = list.iter().filter_map(|d| public_csp_source(d)).collect();
    }
    Some(parsed)
}

/// 一段话里出现的 Key 换成 `***`（太短的不换：会把正常的字换掉，也不像 Key）。
fn redact(text: &str, secrets: &[String]) -> String {
    let mut out = text.to_string();
    for secret in secrets.iter().filter(|s| s.trim().chars().count() >= 4) {
        out = out.replace(secret.as_str(), "***");
        let trimmed = secret.trim();
        if trimmed != secret {
            out = out.replace(trimmed, "***");
        }
    }
    out
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
            "connectDomains": ["wss://live.example.com", "http://127.0.0.1:8080", "http://localhost:3000",
                               "https://192.168.1.2", "http://router.local"],
            "frameDomains": ["http://10.0.0.1"],
            "baseUriDomains": ["https://127.0.0.1"]
        }, "prefersBorder": false, "domain": "x.example.com"}});
        let ui = resource_ui(Some(&meta)).unwrap();
        assert_eq!(ui.csp.resource_domains, vec!["https://cdn.example.com"]);
        assert_eq!(
            ui.csp.connect_domains,
            vec!["wss://live.example.com"],
            "别人的页面不能声明本机和局域网的地址"
        );
        assert!(ui.csp.frame_domains.is_empty() && ui.csp.base_uri_domains.is_empty());
        assert_eq!(ui.prefers_border, Some(false));
        assert!(resource_ui(Some(&json!({}))).is_none());
        assert!(icon_ok(&json!({"src": "https://x.example.com/i.png"})));
        assert!(icon_ok(&json!({"src": "data:image/png;base64,AAAA"})));
        assert!(!icon_ok(&json!({"src": "javascript:alert(1)"})));
        assert!(!icon_ok(&json!({"src": "file:///c:/x.png"})));
    }

    /// 出错的话里的 Key 换掉（会进卡片、经“没连上”发给模型）；太短的不换。
    #[test]
    fn keys_are_scrubbed_from_errors() {
        let secrets = vec![
            "sk-live-123456".to_string(),
            "Bearer abc123".to_string(),
            "ab".to_string(),
        ];
        assert_eq!(
            redact(
                "连不上 http://x.example.com/mcp?token=sk-live-123456：Bearer abc123 不对，ab",
                &secrets
            ),
            "连不上 http://x.example.com/mcp?token=***：*** 不对，ab"
        );
        let server = Server::new("x", Err("坏了".into()), None, "test");
        assert_eq!(
            server.scrub("sk-live-123456"),
            "sk-live-123456",
            "没连过：没有要换的"
        );
        server.state.lock().unwrap().secrets = secrets;
        assert!(!server.offline("for sk-live-123456").contains("sk-live"));
    }
}
