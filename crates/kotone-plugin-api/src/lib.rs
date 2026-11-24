//! kotone-plugin-api：插件契约（ADR 038 D5），宿主和插件之间唯一的缝。
//!
//! 照 MCP 和 MCP Apps（<https://modelcontextprotocol.io/docs/extensions/apps>）：
//! - **清单** [`Manifest`]（`plugins/<id>/plugin.toml`）：我是谁、挂在哪些游戏上、带哪个游戏中心适配器、
//!   数据开关和风险提示、页面（`ui://kotone/<id>/<页面>`）、覆盖层、给她的两段话；
//! - **工具** [`ToolSpec`]：照 MCP 的 `tools/list`，加上 MCP Apps 的 `_meta.ui`：`visibility`
//!   （`model` 给她、`app` 给页面）和 `resourceUri`（[`ToolSpec::page`]）。说明和参数（JSON Schema）写在
//!   插件的 Rust 代码里，和工具的实现放在一起；
//! - **工具结果** [`ToolResult`]：照 MCP 的 `CallToolResult`：`content` 给她（只有这份进她的上下文），
//!   `structured_content` 给页面（存进对话记录，聊天卡片用它），`ui` 用哪个页面显示；
//! - [`Plugin`]：插件实现；[`Host`]：宿主交给插件的本体能力（设置、游戏中心、后台任务……）。
//!
//! 插件 crate 只依赖这个 crate、`kotone-core` 的纯数据类型和自己的第三方库，不碰 Tauri。
//! 一期的插件都是内置的，编译在一起（桌面端 `plugins.rs` 里一张静态表）。

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

pub use kotone_core::games::{Observation, Phase};

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

// ---------- 清单 ----------

/// `plugin.toml`。字段名在文件里是 snake_case；发给界面时是 camelCase。
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all(serialize = "camelCase"))]
pub struct Manifest {
    /// 小写字母、数字和连字符（`lol`、`voice-input`）；设置在 `plugins.<id>`，页面在 `ui://kotone/<id>/…`。
    pub id: String,
    /// 界面上的名字（“英雄联盟”）。
    pub name: String,
    /// 一句话：这个插件能做什么（插件列表里显示）。
    pub description: String,
    /// 挂在哪些游戏配置上（游戏配置 id）；空 = 不针对某个游戏。
    #[serde(default)]
    pub games: Vec<String>,
    /// 带的游戏中心适配器（ADR 036 D4），例如 `lol`。
    #[serde(default)]
    pub adapter: Option<String>,
    /// 工具名的前缀（不写 = 插件 id）。搬进插件以前就有的工具保留原来的名字（例如游戏语音的 `voice_chat.*`），
    /// 玩家设过的权限和她的习惯都不用改。
    #[serde(default)]
    pub tool_prefix: Option<String>,
    #[serde(default = "yes")]
    pub enabled_by_default: bool,
    /// 插件里的数据开关，各带风险提示（ADR 038 D3 第 8 条）。
    #[serde(default)]
    pub sources: Vec<SourceSpec>,
    /// 页面（右侧栏、左侧栏打开的完整页面；聊天里是它的小尺寸快照）。
    #[serde(default)]
    pub pages: Vec<PageSpec>,
    /// 游戏上方的覆盖层页面。
    #[serde(default)]
    pub overlays: Vec<PageSpec>,
    /// 给她的话（写进系统提示）。
    #[serde(default)]
    pub agent: AgentText,
}

fn yes() -> bool {
    true
}

/// 插件里的一个数据开关。
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all(serialize = "camelCase"))]
pub struct SourceSpec {
    pub id: String,
    pub name: String,
    /// 开着时用到什么、做什么。
    #[serde(default)]
    pub description: String,
    #[serde(default = "yes")]
    pub enabled_by_default: bool,
    /// 风险提示（为什么可能有风险、出处）。空 = 没有。
    #[serde(default)]
    pub risk: String,
}

/// 一个页面或覆盖层。
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all(serialize = "camelCase"))]
pub struct PageSpec {
    /// `ui://kotone/<插件 id>/<页面>`。
    pub uri: String,
    pub title: String,
    /// 要哪些数据开关开着才显示。
    #[serde(default)]
    pub needs: Vec<String>,
}

/// 给她的话：都可以空。插件关着（或要的数据开关关着、给她的工具一个都没有）时不写进系统提示。
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all(serialize = "camelCase"))]
pub struct AgentText {
    /// 能力概览里的一句（“你能做什么”）。
    #[serde(default)]
    pub overview: String,
    /// 主会话“工作方式”里的一条（什么时候用哪个工具、怎么说）。
    #[serde(default)]
    pub rule: String,
}

/// 插件 id 的写法。
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 32
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !id.starts_with('-')
        && !id.ends_with('-')
}

impl Manifest {
    /// 读 `plugin.toml` 并检查：id 的写法、页面地址在自己的 `ui://kotone/<id>/` 下、
    /// `needs` 只写自己有的数据开关、开关和页面不重名。
    pub fn parse(text: &str) -> Result<Self, String> {
        // Windows 上检出的文件是 CRLF（core.autocrlf）：多行字符串里的换行统一成 LF，给她的话两边一样
        let text = text.replace("\r\n", "\n");
        let manifest: Manifest =
            toml::from_str(&text).map_err(|e| format!("plugin.toml 不对：{e}"))?;
        manifest.check()?;
        Ok(manifest)
    }

    fn check(&self) -> Result<(), String> {
        if !valid_id(&self.id) {
            return Err(format!(
                "插件 id “{}” 只能用小写字母、数字和连字符",
                self.id
            ));
        }
        if self.name.trim().is_empty() || self.description.trim().is_empty() {
            return Err(format!("插件 {} 要有名字和说明", self.id));
        }
        let mut seen = std::collections::HashSet::new();
        for source in &self.sources {
            if !valid_id(&source.id) || !seen.insert(source.id.as_str()) {
                return Err(format!(
                    "插件 {} 的数据开关 “{}” 重名或写法不对",
                    self.id, source.id
                ));
            }
        }
        let mut uris = std::collections::HashSet::new();
        for page in self.pages.iter().chain(&self.overlays) {
            if !self.owns_uri(&page.uri) || !uris.insert(page.uri.as_str()) {
                return Err(format!(
                    "插件 {} 的页面 {} 要写成 ui://kotone/{}/<页面>，并且不重名",
                    self.id, page.uri, self.id
                ));
            }
            self.check_needs(&page.uri, &page.needs)?;
        }
        Ok(())
    }

    /// 页面地址是不是这个插件的（`ui://kotone/<id>/<页面>`）。
    pub fn owns_uri(&self, uri: &str) -> bool {
        uri.strip_prefix("ui://kotone/")
            .and_then(|rest| rest.strip_prefix(self.id.as_str()))
            .and_then(|rest| rest.strip_prefix('/'))
            .is_some_and(|page| {
                !page.is_empty()
                    && page
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
            })
    }

    /// `needs` 里写的数据开关都要是自己的。
    pub fn check_needs(&self, what: &str, needs: &[String]) -> Result<(), String> {
        match needs.iter().find(|n| self.source(n).is_none()) {
            Some(unknown) => Err(format!(
                "插件 {} 的 {what} 要的数据开关 {unknown} 不存在",
                self.id
            )),
            None => Ok(()),
        }
    }

    pub fn source(&self, id: &str) -> Option<&SourceSpec> {
        self.sources.iter().find(|s| s.id == id)
    }

    /// 工具名的前缀（`tool_prefix`，不写就是插件 id）。
    pub fn tool_prefix(&self) -> &str {
        self.tool_prefix.as_deref().unwrap_or(&self.id)
    }

    /// 检查插件代码给的工具：名字以 `<前缀>.` 开头（或就是前缀），页面是自己的，`needs` 是自己的开关，不重名。
    pub fn check_tools(&self, tools: &[ToolSpec]) -> Result<(), String> {
        let mut names = std::collections::HashSet::new();
        let prefix = self.tool_prefix();
        for tool in tools {
            let ours = tool.name == prefix || tool.name.starts_with(&format!("{prefix}."));
            if !ours || !names.insert(tool.name.as_str()) {
                return Err(format!(
                    "插件 {} 的工具 {} 要以 “{prefix}.” 开头，并且不重名",
                    self.id, tool.name
                ));
            }
            if tool.visibility.is_empty() {
                return Err(format!("工具 {} 至少要给她或给页面", tool.name));
            }
            if let Some(page) = &tool.page {
                if !self.pages.iter().any(|p| &p.uri == page) {
                    return Err(format!("工具 {} 用的页面 {page} 不在清单里", tool.name));
                }
            }
            self.check_needs(&tool.name, &tool.needs)?;
            if tool.parameters.get("type").and_then(Value::as_str) != Some("object") {
                return Err(format!("工具 {} 的参数顶层要是 object", tool.name));
            }
        }
        Ok(())
    }
}

// ---------- 工具 ----------

/// 谁能看到这个工具（照 MCP Apps `_meta.ui.visibility`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Visibility {
    /// 进她（和外部 Agent）的工具清单。
    Model,
    /// 插件自己的页面能调。
    App,
}

/// 一个工具（照 MCP `tools/list` 的一项 + MCP Apps 的 `_meta.ui`）。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolSpec {
    /// `<插件 id>.<名字>`（`lol.scout`）；插件只有一个工具时可以就叫插件 id（`music`）。
    pub name: String,
    /// 给她看的说明：什么时候用、给什么、怎么说。
    pub description: String,
    /// 参数的 JSON Schema（顶层是 object）。
    pub parameters: Value,
    pub visibility: Vec<Visibility>,
    /// 结果用哪个页面显示（`ui://kotone/<id>/<页面>`，照 MCP Apps `_meta.ui.resourceUri`）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page: Option<String>,
    /// 要哪些数据开关开着；有一个关着，这个工具就不出现。
    pub needs: Vec<String>,
    /// 会动什么（`input`、`plays-media`……），权限界面和工具服务用。
    pub side_effects: Vec<String>,
    /// 结果里有别人写的字（玩家名、聊天、网页）：按不可信数据交给她。
    pub untrusted: bool,
    /// 马上返回（约 1–2 秒内）：主会话（和玩家说话的那段对话）也拿这个工具（ADR 029）；
    /// 否则只给后台。
    pub instant: bool,
}

impl ToolSpec {
    /// 给她也给页面、不马上返回、结果可信、不要求数据开关。用下面几个方法改。
    pub fn new(name: &str, description: &str, parameters: Value) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            parameters,
            visibility: vec![Visibility::Model, Visibility::App],
            page: None,
            needs: Vec::new(),
            side_effects: Vec::new(),
            untrusted: false,
            instant: false,
        }
    }

    /// 只给页面用（翻页、搜索、详情），不进她的工具清单。
    pub fn app_only(mut self) -> Self {
        self.visibility = vec![Visibility::App];
        self
    }

    pub fn page(mut self, uri: &str) -> Self {
        self.page = Some(uri.into());
        self
    }

    pub fn needs(mut self, sources: &[&str]) -> Self {
        self.needs = sources.iter().map(|s| s.to_string()).collect();
        self
    }

    pub fn side_effects(mut self, effects: &[&str]) -> Self {
        self.side_effects = effects.iter().map(|s| s.to_string()).collect();
        self
    }

    pub fn untrusted(mut self) -> Self {
        self.untrusted = true;
        self
    }

    pub fn instant(mut self) -> Self {
        self.instant = true;
        self
    }

    pub fn for_model(&self) -> bool {
        self.visibility.contains(&Visibility::Model)
    }

    pub fn for_app(&self) -> bool {
        self.visibility.contains(&Visibility::App)
    }

    /// 工具服务 `system.capabilities` 里的一项（外部 Agent、MCP 看到的样子）。
    pub fn service_spec(&self, plugin: &str) -> Value {
        let mut ui = Map::new();
        ui.insert("visibility".into(), json!(self.visibility));
        if let Some(page) = &self.page {
            ui.insert("resourceUri".into(), json!(page));
        }
        json!({
            "name": self.name,
            "version": 1,
            "status": "implemented",
            "description": self.description,
            "sideEffects": self.side_effects,
            "dataEgress": "none",
            "ready": true,
            "notReadyReason": null,
            "inputSchema": self.parameters,
            "outputSchema": {"type": "object"},
            "plugin": plugin,
            "untrustedResult": self.untrusted,
            "_meta": {"ui": Value::Object(ui)},
        })
    }
}

/// 结果用哪个页面显示、带什么参数（照 MCP Apps `_meta.ui.resourceUri`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiRef {
    pub uri: String,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub params: Value,
}

/// 工具结果（照 MCP 的 `CallToolResult`）。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolResult {
    /// 给她的：只有这份进她的上下文（她拿来说话），要短。
    pub content: Value,
    /// 给页面的数据：她那一刻看到的样子，存进对话记录，聊天卡片用它。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub structured_content: Option<Value>,
    /// 用哪个页面显示。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ui: Option<UiRef>,
}

impl ToolResult {
    /// 只有给她的那份。
    pub fn content(content: Value) -> Self {
        Self {
            content,
            structured_content: None,
            ui: None,
        }
    }

    /// 带页面：`data` 给页面，`uri` 显示它。
    pub fn with_page(mut self, uri: &str, data: Value) -> Self {
        self.structured_content = Some(data);
        self.ui = Some(UiRef {
            uri: uri.into(),
            params: Value::Null,
        });
        self
    }

    /// 工具服务返回给外部 Agent 的样子（照 MCP：`content`、`structuredContent`、`_meta.ui`）。
    pub fn to_service_json(&self) -> Value {
        let mut out = json!({"content": self.content});
        if let Some(data) = &self.structured_content {
            out["structuredContent"] = data.clone();
        }
        if let Some(ui) = &self.ui {
            out["_meta"] = json!({"ui": {"resourceUri": ui.uri, "params": ui.params}});
        }
        out
    }
}

// ---------- 覆盖层 ----------

/// 覆盖层盖在哪个窗口上（只在它在前台时显示）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "exe", rename_all = "camelCase")]
pub enum OverlayTarget {
    /// 这个进程的窗口（`League of Legends.exe`）。
    Process(String),
    /// 这几个进程里任何一个的窗口。
    AnyOf(Vec<String>),
}

/// 放在目标窗口客户区的哪里。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Anchor {
    /// 一块区域，按客户区的比例（0..1）。
    Rect { x: f64, y: f64, w: f64, h: f64 },
    /// 贴一个角，大小 `w` × `h`（客户区的比例）。`corner`：`top-left` / `top-right` / `bottom-left` /
    /// `bottom-right`；`margin` 离边多远（比例）。
    Corner {
        corner: String,
        margin: f64,
        w: f64,
        h: f64,
    },
}

impl Anchor {
    /// 在客户区里的位置 `[x, y, w, h]`（比例，夹在 0..1 里）。
    pub fn rect(&self) -> [f64; 4] {
        let clamp = |v: f64| v.clamp(0.0, 1.0);
        match self {
            Anchor::Rect { x, y, w, h } => {
                let (w, h) = (clamp(*w), clamp(*h));
                [clamp(*x).min(1.0 - w), clamp(*y).min(1.0 - h), w, h]
            }
            Anchor::Corner {
                corner,
                margin,
                w,
                h,
            } => {
                let (w, h, m) = (clamp(*w), clamp(*h), clamp(*margin));
                let left = corner.ends_with("left");
                let top = corner.starts_with("top");
                let x = if left { m } else { 1.0 - m - w };
                let y = if top { m } else { 1.0 - m - h };
                [clamp(x), clamp(y), w, h]
            }
        }
    }
}

/// 覆盖层上的一项（照 MCP Apps 的页面：`uri` 是插件登记的覆盖层页面，`data` 是它的数据）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayItem {
    /// 插件自己起的 id（同一个插件里不重）。
    pub id: String,
    /// `ui://kotone/<插件>/<覆盖层>`，要在清单的 `overlays` 里。
    pub uri: String,
    pub target: OverlayTarget,
    pub anchor: Anchor,
    #[serde(default)]
    pub data: Value,
}

/// 截图识别出来的一行字（[`Host::screen_text`]）：位置是前台窗口客户区的比例 `[x, y, w, h]`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScreenText {
    pub text: String,
    pub rect: [f64; 4],
}

// ---------- 插件和宿主 ----------

/// 一个插件。实现放在 `plugins/<id>/src/`，桌面端在 `plugins.rs` 里登记。
pub trait Plugin: Send + Sync + 'static {
    /// 清单（一般是 `include_str!("../plugin.toml")` 读出来的）。
    fn manifest(&self) -> &Manifest;

    /// 全部工具（给她的和只给页面的）。宿主按开关和 `needs` 决定哪些露出来。
    fn tools(&self) -> Vec<ToolSpec>;

    /// 调用一个工具。插件关着时宿主不会调；数据开关在调用时由插件自己再看一次
    /// （一段对话里她的工具清单不变，开关可能中途关了）。
    fn call<'a>(
        &'a self,
        host: Arc<dyn Host>,
        tool: &'a str,
        arguments: &'a Value,
    ) -> BoxFuture<'a, Result<ToolResult, String>>;

    /// Kotone 启动时调一次：开后台循环（[`Host::spawn`]）。循环里自己看 [`Host::enabled`] 和数据开关。
    fn start(self: Arc<Self>, host: Arc<dyn Host>) {
        let _ = host;
    }

    /// 玩家让她停下（停下热键、停止按钮、新的一句话打断）：停掉正在替她做的事（例如正在对队友说的话）。
    fn stop(&self) {}

    /// 设置保存了（玩家在界面上改的、她用 settings.update 改的、插件自己改的都算）。
    /// 跟着设置启停的东西（例如游戏语音的转发）在这里对一下。
    fn settings_changed(&self, host: Arc<dyn Host>) {
        let _ = host;
    }
}

/// 宿主交给插件的本体能力（ADR 038 D5）。每个插件拿到的是自己那一份（设置、目录按插件 id 分开）。
pub trait Host: Send + Sync {
    /// 这份宿主属于哪个插件。
    fn plugin(&self) -> &str;
    /// 插件开着没有（`plugins.<id>.enabled`，没写按清单）。
    fn enabled(&self) -> bool;
    /// 插件里的某个数据开关开着没有（插件关着时也是关）。
    fn source_enabled(&self, source: &str) -> bool;
    /// Kotone 的版本（例如告诉 OP.GG 是哪个应用在用它的接口）。
    fn app_version(&self) -> String {
        String::new()
    }
    /// 她开着没有（0.1.x 升级上来、还没打开她的玩家是 false）。要她的声音、她的模型的插件看这个。
    fn companion_enabled(&self) -> bool {
        true
    }
    /// 合并进插件自己的设置（`plugins.<id>.settings`：对象按字段合并，其余整个替换），和玩家在界面上保存一样生效。
    fn update_settings(&self, patch: Value) -> Result<(), String> {
        let _ = patch;
        Err("这里不能改设置".to_string())
    }
    /// 用她现在的声音合成一句话（嘴，ADR 038 D3 第 7 条）：单声道 PCM 和采样率。系统语音合成不出 PCM，会报错。
    fn synthesize(&self, text: &str) -> BoxFuture<'static, Result<(Vec<f32>, u32), String>> {
        let _ = text;
        Box::pin(async { Err("这里没有语音合成".to_string()) })
    }
    /// 她的声音能不能合成成 PCM（系统语音不行）。
    fn can_synthesize(&self) -> bool {
        false
    }
    /// 用她的声音在玩家的扬声器上说一句（和 `synthesize` 同一句时走缓存）。
    fn speak(&self, text: &str) -> BoxFuture<'static, Result<(), String>> {
        let _ = text;
        Box::pin(async { Err("这里没有语音".to_string()) })
    }
    /// 下载一个文件到 `dest`（给了 SHA-256 就校验）。和模型下载共用“同一时间只下一个”的限制。
    fn download(
        &self,
        url: &str,
        dest: PathBuf,
        sha256: Option<&str>,
    ) -> BoxFuture<'static, Result<(), String>> {
        let _ = (url, dest, sha256);
        Box::pin(async { Err("这里不能下载".to_string()) })
    }
    /// 插件自己的设置（`plugins.<id>.settings`；没有时是空对象）。
    fn settings(&self) -> Value;
    /// 插件自己的存储目录（`<KOTONE_HOME>/plugins/<id>/`，用时才建）。
    fn data_dir(&self) -> PathBuf;
    /// 在 Kotone 的异步运行时里跑一个后台任务。
    fn spawn(&self, task: BoxFuture<'static, ()>);
    /// 叫这个名字的进程（`LeagueClientUx.exe`，不分大小写）的 pid。
    fn process_ids(&self, exe: &str) -> Vec<u32>;
    /// 某个进程的命令行（不用管理员；读不到时 None）。英雄联盟客户端的端口和令牌在这里（ADR 035）。
    fn process_command_line(&self, pid: u32) -> Option<String>;
    /// 游戏中心（ADR 036）：报某个游戏现在的阶段和状态。
    fn game_observe(&self, game: &str, observation: Observation);
    /// 游戏中心：认领（true）或交回（false）某个游戏的阶段。认领了以后进程那一路不再报阶段。
    fn game_claim(&self, game: &str, claimed: bool);
    /// 游戏中心：报一件游戏专用的事（`lol.dragon_kill`）。
    fn game_event(
        &self,
        game: &str,
        kind: &str,
        source: &str,
        data: Map<String, Value>,
        game_time: Option<f64>,
    );
    /// 给插件自己的页面发事件（界面上是 `kotone://plugin` 事件，带 `plugin`、`event`、`payload`）。
    fn emit(&self, event: &str, payload: Value) {
        let _ = (event, payload);
    }
    /// 换掉这个插件在覆盖层上的全部条目（空 = 什么都不显示）。覆盖层只有一个，在本体里：置顶、点击穿透、
    /// 只在目标窗口在前台时显示、不进截图（ADR 038 D5）。
    fn overlay(&self, items: Vec<OverlayItem>) {
        let _ = items;
    }
    /// 截前台窗口的一块区域（客户区比例 `[x, y, w, h]`）并识别文字（本体的文字识别，玩家选的模型）。
    /// 前台不是 `process` 这个进程的窗口时不截（报错）。覆盖层不会被截进去。
    /// `local_only`：只用本地模型（在线识别要十几秒、要花钱时不识别，直接报错）。
    fn screen_text(
        &self,
        process: &str,
        region: [f64; 4],
        local_only: bool,
    ) -> BoxFuture<'static, Result<Vec<ScreenText>, String>> {
        let _ = (process, region, local_only);
        Box::pin(async { Err("这里没有截图识别".to_string()) })
    }
}

/// 一个开着的插件给她的东西（宿主按开关算好，交给 `kotone-agent` 拼工具清单和系统提示）。
#[derive(Debug, Clone, PartialEq)]
pub struct Offer {
    pub plugin: String,
    pub name: String,
    pub text: AgentText,
    /// 给她的工具（`visibility` 含 `model`、要的数据开关都开着）。
    pub tools: Vec<ToolSpec>,
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
id = "lol"
name = "英雄联盟"
description = "这局和任何玩家的战绩"
games = ["lol"]
adapter = "lol"

[[sources]]
id = "client"
name = "读客户端数据"
risk = "国服可能违反用户协议"

[[sources]]
id = "stats"
name = "用网上的统计"
enabled_by_default = false

[[pages]]
uri = "ui://kotone/lol/match"
title = "这局"
needs = ["client"]

[agent]
overview = "英雄联盟里查战绩"
rule = "- 英雄联盟：……"
"#;

    #[test]
    fn manifest_parses_with_defaults() {
        let m = Manifest::parse(SAMPLE).unwrap();
        assert_eq!(m.id, "lol");
        assert!(m.enabled_by_default);
        assert!(m.source("client").unwrap().enabled_by_default);
        assert!(!m.source("stats").unwrap().enabled_by_default);
        assert_eq!(m.adapter.as_deref(), Some("lol"));
        assert_eq!(m.pages[0].needs, vec!["client"]);
        assert_eq!(m.agent.overview, "英雄联盟里查战绩");
        // 发给界面是 camelCase
        let v = serde_json::to_value(&m).unwrap();
        assert_eq!(v["enabledByDefault"], true);
        assert_eq!(v["sources"][1]["enabledByDefault"], false);
    }

    #[test]
    fn manifest_rejects_mistakes() {
        assert!(Manifest::parse(&SAMPLE.replace("id = \"lol\"", "id = \"LoL\"")).is_err());
        assert!(Manifest::parse(
            &SAMPLE.replace("ui://kotone/lol/match", "ui://kotone/music/match")
        )
        .is_err());
        assert!(
            Manifest::parse(&SAMPLE.replace("needs = [\"client\"]", "needs = [\"memory\"]"))
                .is_err()
        );
        assert!(Manifest::parse(&format!("{SAMPLE}\nunknown = 1")).is_err());
        assert!(Manifest::parse(&SAMPLE.replace("id = \"stats\"", "id = \"client\"")).is_err());
    }

    #[test]
    fn tools_are_checked_against_the_manifest() {
        let m = Manifest::parse(SAMPLE).unwrap();
        let ok = ToolSpec::new("lol.scout", "查战绩", json!({"type": "object"}))
            .page("ui://kotone/lol/match")
            .needs(&["client"])
            .instant()
            .untrusted();
        assert!(m.check_tools(std::slice::from_ref(&ok)).is_ok());
        assert!(m.check_tools(&[ok.clone(), ok.clone()]).is_err(), "重名");
        let other = ToolSpec::new("music", "放歌", json!({"type": "object"}));
        assert!(m.check_tools(&[other]).is_err(), "别的插件的名字");
        let page =
            ToolSpec::new("lol.x", "", json!({"type": "object"})).page("ui://kotone/lol/none");
        assert!(m.check_tools(&[page]).is_err());
        let needs = ToolSpec::new("lol.x", "", json!({"type": "object"})).needs(&["memory"]);
        assert!(m.check_tools(&[needs]).is_err());
        let schema = ToolSpec::new("lol.x", "", json!({"type": "array"}));
        assert!(m.check_tools(&[schema]).is_err());
    }

    #[test]
    fn service_spec_and_result_follow_mcp_apps() {
        let spec = ToolSpec::new("lol.player", "某个人的战绩", json!({"type": "object"}))
            .page("ui://kotone/lol/player")
            .untrusted()
            .app_only();
        assert!(!spec.for_model() && spec.for_app());
        let v = spec.service_spec("lol");
        assert_eq!(v["plugin"], "lol");
        assert_eq!(v["untrustedResult"], true);
        assert_eq!(v["_meta"]["ui"]["visibility"], json!(["app"]));
        assert_eq!(v["_meta"]["ui"]["resourceUri"], "ui://kotone/lol/player");
        assert_eq!(v["inputSchema"]["type"], "object");

        let plain = ToolResult::content(json!({"status": "ok"}));
        assert_eq!(
            plain.to_service_json(),
            json!({"content": {"status": "ok"}})
        );
        let paged = ToolResult::content(json!("两句话"))
            .with_page("ui://kotone/lol/match", json!({"players": []}));
        let v = paged.to_service_json();
        assert_eq!(v["structuredContent"], json!({"players": []}));
        assert_eq!(v["_meta"]["ui"]["resourceUri"], "ui://kotone/lol/match");
    }

    #[test]
    fn crlf_files_give_the_same_text() {
        let lf = Manifest::parse(SAMPLE).unwrap();
        let crlf = Manifest::parse(&SAMPLE.replace('\n', "\r\n")).unwrap();
        assert_eq!(lf, crlf);
    }

    #[test]
    fn overlay_items_round_trip() {
        let item = OverlayItem {
            id: "card-1".into(),
            uri: "ui://kotone/lol/augment-scores".into(),
            target: OverlayTarget::Process("League of Legends.exe".into()),
            anchor: Anchor::Rect {
                x: 0.2,
                y: 0.3,
                w: 0.15,
                h: 0.05,
            },
            data: json!({"delta": 3.2}),
        };
        let v = serde_json::to_value(&item).unwrap();
        assert_eq!(
            v["target"],
            json!({"kind": "process", "exe": "League of Legends.exe"})
        );
        assert_eq!(v["anchor"]["kind"], "rect");
        let back: OverlayItem = serde_json::from_value(v).unwrap();
        assert_eq!(back, item);
        let corner = Anchor::Corner {
            corner: "top-right".into(),
            margin: 0.01,
            w: 0.2,
            h: 0.3,
        };
        assert_eq!(serde_json::to_value(&corner).unwrap()["kind"], "corner");
        let [x, y, w, h] = corner.rect();
        assert!((x - 0.79).abs() < 1e-9 && (y - 0.01).abs() < 1e-9 && w == 0.2 && h == 0.3);
        let off = Anchor::Rect {
            x: 0.95,
            y: -0.1,
            w: 0.2,
            h: 0.1,
        }
        .rect();
        assert_eq!(off, [0.8, 0.0, 0.2, 0.1], "超出客户区的挪回来");
    }

    #[test]
    fn a_tool_prefix_keeps_old_tool_names() {
        let m = Manifest::parse(&SAMPLE.replace(
            "adapter = \"lol\"",
            "adapter = \"lol\"\ntool_prefix = \"voice_chat\"",
        ))
        .unwrap();
        assert_eq!(m.tool_prefix(), "voice_chat");
        let say = ToolSpec::new("voice_chat.say", "", json!({"type": "object"}));
        assert!(m.check_tools(&[say]).is_ok());
        let lol = ToolSpec::new("lol.scout", "", json!({"type": "object"}));
        assert!(m.check_tools(&[lol]).is_err(), "有前缀时只认前缀");
    }

    #[test]
    fn ids() {
        assert!(valid_id("voice-input"));
        assert!(!valid_id("Voice"));
        assert!(!valid_id("-a"));
        assert!(!valid_id(""));
    }
}
