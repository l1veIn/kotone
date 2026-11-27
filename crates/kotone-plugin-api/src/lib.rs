//! kotone-plugin-api：插件契约（ADR 038 D5），宿主和插件之间唯一的缝。
//!
//! 照 MCP 和 MCP Apps（<https://modelcontextprotocol.io/docs/extensions/apps>）：
//! - **清单** [`Manifest`]（`plugins/<id>/plugin.toml`）：我是谁、挂在哪些游戏上、带哪个游戏中心适配器、
//!   数据开关和风险提示、许可和出处（`[[notices]]`）、页面（`ui://kotone/<id>/<页面>`）、给她的两段话；
//! - **工具** [`ToolSpec`]：照 MCP 的 `tools/list`，加上 MCP Apps 的 `_meta.ui`：`visibility`
//!   （`model` 给她、`app` 给页面）和 `resourceUri`（[`ToolSpec::page`]）。说明和参数（JSON Schema）写在
//!   插件的 Rust 代码里，和工具的实现放在一起；
//! - **工具结果** [`ToolResult`]：照 MCP 的 `CallToolResult`：`content` 给她（只有这份进她的上下文），
//!   `structured_content` 给页面（存进对话记录，聊天卡片用它），`ui` 用哪个页面显示；
//! - [`Plugin`]：插件实现；[`Host`]：宿主交给插件的本体能力（设置、游戏中心、后台任务……）；
//! - **MCP App 资源和素材**（[`resource`] 模块，ADR 039 D5、D6）：清单里的页面可以由插件做成独立的 HTML
//!   （[`Plugin::resource`]），Kotone 和外部宿主显示同一份；图片这类素材经 [`Plugin::asset`] 取；
//! - **推送**（[`push`] 模块，ADR 043 D3–D5）：清单 `[[pushes]]` 声明的“时机 → 推送”（卡片 / 一行 / 语音），
//!   插件经 [`Host::push`] 发，宿主按清单和玩家的开关执行。
//!
//! 插件 crate 只依赖这个 crate、`kotone-core` 的纯数据类型和自己的第三方库；可以用系统适配层
//! （`kotone-platform-windows`）里没有状态的系统调用（音频设备、重采样、打字注入……），不碰 Tauri、桌面端、
//! 她（`kotone-agent` / `kotone-ai`）；全进程只有一份的东西（Esc 和呼叫键的钩子、停下她、免问）只经 [`Host`]
//! （ADR 038“怎么定的”，2026-11-24）。
//! 一期的插件都是内置的，编译在一起（桌面端 `plugins.rs` 里一张静态表）。

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

pub use kotone_core::games::{EventSpec, Observation, Phase};
pub use kotone_core::hotkey_commands::{CommandKind, CommandSpec};
pub use kotone_core::inject::CancelToken;
pub use kotone_core::send_to_game::provider::{
    ChatSend, ChatSent, ChatStop, SendToGameSpec, StopCode,
};

mod events;
pub use events::event_catalog;
pub mod push;
pub use push::{
    Push, PushCard, PushCell, PushForm, PushLine, PushOutcome, PushPriority, PushSpec, PushTone,
};
pub mod resource;
pub use resource::{valid_asset_path, Asset, Csp, Resource, ResourceUi, APP_MIME};

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
    /// 给她的话（写进系统提示）。
    #[serde(default)]
    pub agent: AgentText,
    /// 游戏中心的事（ADR 036 D4）：插件登记自己会报的事（`[[events]]`：类型、中文说明、`data` 字段和
    /// 说明、要的数据开关）。类型以 `<适配器>.` 开头（没有适配器的插件以 `<插件 id>.` 开头，ADR 043 D10）；
    /// 只有登记过的才报得出去。见 [`event_catalog`]。
    #[serde(default)]
    pub events: Vec<EventSpec>,
    /// 能绑热键的命令（ADR 040 D1，`[[commands]]`：id、名字、一句说明、按一下 / 按住、要不要语音识别）。
    /// 插件只登记命令，不登记键（D7）；也没有默认键这个字段（D4）。完整名是 `<插件 id>.<命令 id>`。
    /// 第二种命令（ADR 040 第 3 期）：`tool` + `args`，按下时宿主调插件**自己的**一个给她的工具、带这些固定参数
    /// （照这个工具的权限；英雄联盟的一键喊话）。这种命令按一下、不听；工具在不在、给不给她由 [`Manifest::check_tools`] 查。
    #[serde(default)]
    pub commands: Vec<CommandSpec>,
    /// 发到游戏的发法（ADR 041 D3，`[[send_to_game]]`：给哪个游戏、前台是哪些程序的窗口时交给插件、要的数据开关）。
    /// 宿主按前台挑：前台是登记的窗口就调 [`Plugin::send_to_game`]，否则用本体的通用发法。只给内置插件（D7）。
    #[serde(default)]
    pub send_to_game: Vec<SendToGameSpec>,
    /// 内置推送（ADR 043 D4，`[[pushes]]`：id、名字、时机、做什么、样子、多要紧、默认开不开、要的数据开关）。插件页上
    /// 每条一个开关，宿主执行开关；插件经 [`Host::push`] 推，`trigger` 是这里的 id。见 [`push`] 模块。
    #[serde(default)]
    pub pushes: Vec<PushSpec>,
    /// 许可和出处（ADR 043 D8，`[[notices]]`：id、标题、一段话、链接）：数据和图片从哪来、按什么许可用、答应过别人什么。
    /// 和数据开关的 `risk` 一起放进插件的“风险和许可”弹窗（宿主画：风险在前，这些在后），不在设置页上一条条摆出来。
    #[serde(default)]
    pub notices: Vec<NoticeSpec>,
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
    /// 风险提示（为什么可能有风险、出处）。空 = 没有。界面上只在插件的“风险和许可”弹窗里显示（ADR 043 D8）：
    /// 第一次打开这个开关、或者它默认开着而玩家第一次打开插件页时弹一次；文字里的 `github.com/…` 这类地址做成链接。
    #[serde(default)]
    pub risk: String,
}

/// 一条许可或出处（`[[notices]]`，ADR 043 D8）。
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all(serialize = "camelCase"))]
pub struct NoticeSpec {
    /// 小写字母、数字和连字符（`aramhex-data`）；界面文案的键是 `plugin_<插件>_notice_<id>_title / _body`。
    pub id: String,
    /// 标题（“海斗海克斯数据：aramhex-data”）。
    pub title: String,
    /// 一段话：用了什么、按什么许可、我们怎么做的。
    pub body: String,
    /// 链接（`https://…`，界面上点了用系统浏览器打开）：仓库、许可证原文、答应过的 issue。
    #[serde(default)]
    pub links: Vec<String>,
}

/// 一个页面。
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
    /// `needs` 只写自己有的数据开关、开关和页面不重名、登记的游戏事件（`[[events]]`）写法对。
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
        for page in &self.pages {
            if !self.owns_uri(&page.uri) || !uris.insert(page.uri.as_str()) {
                return Err(format!(
                    "插件 {} 的页面 {} 要写成 ui://kotone/{}/<页面>，并且不重名",
                    self.id, page.uri, self.id
                ));
            }
            self.check_needs(&page.uri, &page.needs)?;
        }
        let mut commands = std::collections::HashSet::new();
        for command in &self.commands {
            if !kotone_core::hotkey_commands::valid_command_id(&command.id)
                || !commands.insert(command.id.as_str())
                || command.name.trim().is_empty()
            {
                return Err(format!(
                    "插件 {} 的命令 “{}” 重名、写法不对或者没有名字（只能用小写字母、数字和连字符）",
                    self.id, command.id
                ));
            }
            self.check_tool_command(command)?;
        }
        let mut chat_games = std::collections::HashSet::new();
        for chat in &self.send_to_game {
            if !self.games.contains(&chat.game) || !chat_games.insert(chat.game.as_str()) {
                return Err(format!(
                    "插件 {} 的发到游戏（send_to_game）给的游戏 “{}” 要写在 games 里，并且一个游戏只写一项",
                    self.id, chat.game
                ));
            }
            if chat.windows.is_empty()
                || !chat
                    .windows
                    .iter()
                    .all(|w| kotone_core::send_to_game::provider::valid_window(w))
            {
                return Err(format!(
                    "插件 {} 给游戏 {} 的发到游戏要写 windows（程序名.exe，不带路径）",
                    self.id, chat.game
                ));
            }
            self.check_needs(&format!("发到游戏（{}）", chat.game), &chat.needs)?;
        }
        self.check_pushes()?;
        self.check_notices()?;
        self.check_events()
    }

    /// 许可和出处（`[[notices]]`）：id 写法对、不重名，有标题和正文，链接都是 `https://` 开头、不带空白。
    fn check_notices(&self) -> Result<(), String> {
        let mut seen = std::collections::HashSet::new();
        for notice in &self.notices {
            if !valid_id(&notice.id) || !seen.insert(notice.id.as_str()) {
                return Err(format!(
                    "插件 {} 的许可说明 “{}” 重名或写法不对（只能用小写字母、数字和连字符）",
                    self.id, notice.id
                ));
            }
            if notice.title.trim().is_empty() || notice.body.trim().is_empty() {
                return Err(format!(
                    "插件 {} 的许可说明 {} 要写 title 和 body",
                    self.id, notice.id
                ));
            }
            let full = |link: &str| {
                link.strip_prefix("https://")
                    .is_some_and(|rest| !rest.is_empty() && !rest.chars().any(char::is_whitespace))
            };
            if let Some(bad) = notice.links.iter().find(|link| !full(link)) {
                return Err(format!(
                    "插件 {} 的许可说明 {} 的链接 “{bad}” 要是 https:// 开头的完整地址",
                    self.id, notice.id
                ));
            }
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

    /// 调工具的命令（ADR 040 第 3 期）：只调自己的工具（插件之间不直接调用，也免得插件登记一个调 `settings.update` 的
    /// 预设）、按一下、不听、参数是对象。
    fn check_tool_command(&self, command: &CommandSpec) -> Result<(), String> {
        let Some(tool) = &command.tool else {
            return match command.args {
                Some(_) => Err(format!(
                    "插件 {} 的命令 “{}” 写了 args 没写 tool",
                    self.id, command.id
                )),
                None => Ok(()),
            };
        };
        let prefix = self.tool_prefix();
        if tool != prefix && !tool.starts_with(&format!("{prefix}.")) {
            return Err(format!(
                "插件 {} 的命令 “{}” 只能调自己的工具（以 “{prefix}.” 开头），不是 {tool}",
                self.id, command.id
            ));
        }
        if command.listens || command.kind != CommandKind::Press {
            return Err(format!(
                "插件 {} 的命令 “{}” 调工具：只能按一下（kind = press）、不听（listens = false）",
                self.id, command.id
            ));
        }
        if command.args.as_ref().is_some_and(|a| !a.is_object()) {
            return Err(format!(
                "插件 {} 的命令 “{}” 的 args 要是表（对象）",
                self.id, command.id
            ));
        }
        Ok(())
    }

    /// 检查插件代码给的工具：名字以 `<前缀>.` 开头（或就是前缀），页面是自己的，`needs` 是自己的开关，不重名；
    /// 调工具的命令（`[[commands]]` 的 `tool`）调的是这里给她的工具。
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
        for command in &self.commands {
            let Some(name) = &command.tool else { continue };
            if !tools.iter().any(|t| &t.name == name && t.for_model()) {
                return Err(format!(
                    "插件 {} 的命令 “{}” 调的 {name} 不是它给她的工具（热键和她调的是同一批工具）",
                    self.id, command.id
                ));
            }
        }
        Ok(())
    }

    /// 清单里的页面（`pages`）。
    pub fn page(&self, uri: &str) -> Option<&PageSpec> {
        self.pages.iter().find(|p| p.uri == uri)
    }

    /// 检查插件给的 MCP App 资源（[`Plugin::resource`]，对应规则见 [`resource`] 模块）：地址是清单 `pages` 里的、
    /// 类型是 [`APP_MIME`]、有内容、CSP 声明的每一项都是规范的源（[`resource::normalize_csp_source`] 不改它）。
    pub fn check_resource(&self, resource: &Resource) -> Result<(), String> {
        if self.page(&resource.uri).is_none() {
            return Err(format!(
                "插件 {} 的资源 {} 不是清单 pages 里的页面",
                self.id, resource.uri
            ));
        }
        if resource.mime_type != APP_MIME || resource.text.trim().is_empty() {
            return Err(format!(
                "资源 {} 要是非空的 {APP_MIME}（现在是 {}）",
                resource.uri, resource.mime_type
            ));
        }
        if let Some(bad) = resource
            .ui
            .csp
            .all()
            .find(|d| resource::normalize_csp_source(d).as_deref() != Some(d.as_str()))
        {
            return Err(format!(
                "资源 {} 的 CSP 声明 “{bad}” 要写成源（https://example.com）或主机（*.example.com），不能带路径、通配全部或关键字",
                resource.uri
            ));
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

    /// 工具服务 `system.capabilities` 里的一项（外部 Agent、MCP 看到的样子），`plugin` 写明是哪个插件的。
    pub fn service_spec(&self, plugin: &str) -> Value {
        let mut spec = self.body_service_spec();
        spec["plugin"] = json!(plugin);
        spec
    }

    /// 本体自己用 `ToolSpec` 登记的工具（不属于哪个插件，例如 `game_chat.send`，ADR 039 D2）在工具服务里的样子：
    /// 和插件的一样，只是没有 `plugin`。
    pub fn body_service_spec(&self) -> Value {
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

// ---------- 截图识别 ----------

/// 截图识别出来的一行字（[`Host::screen_text`]）：位置是前台窗口客户区的比例 `[x, y, w, h]`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScreenText {
    pub text: String,
    pub rect: [f64; 4],
}

/// 本地文字识别能不能用（[`Host::local_ocr`]）：`screen_text(.., local_only = true)` 用的是哪个本地模型，或者为什么用不了。
/// 和全局的文字识别选择无关（ADR 043 D7：海斗三选一固定用本地识别）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum LocalOcr {
    /// 能用：用的是这个本地模型（清单 id 和名字）。
    #[serde(rename_all = "camelCase")]
    Ready { model: String, name: String },
    /// 一个能用的本地模型都没装：推荐下载这个（清单 id、名字、一共多少字节）。[`Host::download_local_ocr`] 下的就是它。
    #[serde(rename_all = "camelCase")]
    Missing {
        model: String,
        name: String,
        bytes: u64,
    },
    /// 用不了（原因，玩家看得懂的一句）。
    #[serde(rename_all = "camelCase")]
    Failed { reason: String },
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

    /// MCP App 资源：清单 `pages` 里某个页面做成的独立 HTML（对应规则见 [`resource`] 模块）。默认没有：
    /// 页面还是宿主里的 Svelte 页面。宿主只拿清单 `pages` 里、插件和数据开关都开着的地址来问，给出的资源
    /// 要过 [`Manifest::check_resource`]。要快（宿主在列资源时也会问）：HTML 一般编进插件里。
    fn resource(&self, uri: &str) -> Option<Resource> {
        let _ = uri;
        None
    }

    /// 素材（英雄头像、图标……）：App 在 Kotone 里经回环代理的 `/assets/<插件 id>/<path>` 取。`path` 已由宿主
    /// 检查过（[`valid_asset_path`]），插件关着时宿主不会问。默认没有（代理回 404）。素材不进安装包（ADR 039 D6）：
    /// 从哪取、磁盘缓存多大由插件自己管。
    fn asset<'a>(&'a self, host: Arc<dyn Host>, path: &'a str) -> BoxFuture<'a, Option<Asset>> {
        let _ = (host, path);
        Box::pin(async { None })
    }

    /// 发到游戏（ADR 041 D3）：清单 `[[send_to_game]]` 登记过、前台是登记的窗口时宿主调这里。行已经整理好、频道已经
    /// 算好（[`ChatSend`]）。规矩：截止时间前一定返回；**不按键**（按键只经宿主的入口，`tests/boundaries.rs` 管着）；
    /// 不重试；`Ok` 至少发了一行。插件关着、`needs` 有关着的时候宿主不会调（插件自己可以再看一遍）。默认没有。
    fn send_to_game<'a>(
        &'a self,
        host: Arc<dyn Host>,
        request: &'a ChatSend,
    ) -> BoxFuture<'a, Result<ChatSent, ChatStop>> {
        let _ = (host, request);
        Box::pin(async {
            Err(ChatStop::new(
                StopCode::NoChatHere,
                "这个插件不提供发到游戏",
            ))
        })
    }
}

/// 游戏中心里现在在玩的那个游戏（[`Host::playing_game`]）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Playing {
    /// 游戏配置 id（报事件时用它）。
    pub game: String,
    /// 这一局的编号（游戏中心起的，一局里不变）；不在一局里、或者这个游戏分不出局（只看进程的游戏）时为空。
    pub match_id: String,
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
    /// 用她的声音在玩家的扬声器上说一句（和 `synthesize` 同一句时走缓存）。和她自己的话走同一条路：
    /// 排在她正在说的话、提醒、填充话后面，和她拿同一把播报锁，放的时候算她在出声（[`Host::she_is_audible`]）；
    /// 玩家让她停下时一起停。放不出来（又不是被停下的）时改用弹幕显示这句，原因交回来。返回的 future 放完才结束，
    /// 但调用方不用等它放完：工具里可以交给 [`Host::spawn`] 就返回（她常在同一步里接着用 `speak` 说话，工具等着
    /// 这一遍会和她的话互相等）。见 ADR 020 末尾实施记录、宿主的 `assistant::speak_in_turn`。
    fn speak(&self, text: &str) -> BoxFuture<'static, Result<(), String>> {
        let _ = text;
        Box::pin(async { Err("这里没有语音".to_string()) })
    }
    /// 玩家在插件页面上点了按钮，等于玩家对她说了这句话：出现在聊天里，算玩家亲口说的（能交给后台，
    /// ADR 030 第 2 节）。只在玩家操作页面时调（插件只在“只给页面用”的工具里调它）。
    fn player_says(&self, text: &str) -> Result<(), String> {
        let _ = text;
        Err("这里不能替玩家说话".to_string())
    }
    /// 她的耳朵（ADR 021 的 `voice.listen`）现在在听哪些程序（程序名）。
    fn listening(&self) -> Vec<String> {
        Vec::new()
    }
    /// 当前可接收玩家消息的主对话中，这个后台任务正在监听的进程。任务未运行时返回 None；
    /// 运行中但尚未打开音源时返回空列表。
    fn background_task_listeners(&self, _name: &str) -> Option<Vec<String>> {
        None
    }
    /// 她这会儿在出声没有（边生成边念的回答、应答、提醒）：放声音的插件用它压低自己（ADR 034 D7）。
    fn she_is_audible(&self) -> bool {
        false
    }
    /// 存在 Windows 凭据库里的秘密（登录凭据之类），按插件分开（凭据库里的名字是 `<插件 id>:<name>`）。
    /// 不进日志、不进 config.json。
    fn secret(&self, name: &str) -> Option<String> {
        let _ = name;
        None
    }
    fn set_secret(&self, name: &str, value: &str) -> Result<(), String> {
        let _ = (name, value);
        Err("这里不能存凭据".to_string())
    }
    fn delete_secret(&self, name: &str) {
        let _ = name;
    }
    /// 网页登录窗口：宿主开的独立小窗口（无痕，关掉网页数据就没了，每个插件一个），给要在官网上登录的插件
    /// （例如网易云）。已经开着就再让玩家看到它。玩家正看着 Kotone 时（他在插件页上点的）拿到前面；
    /// 玩家在游戏或别的程序里时（例如外部 Agent 调了页面的工具）只放在后面、闪任务栏，不抢前台。
    fn open_web_window(&self, url: &str, title: &str) -> Result<(), String> {
        let _ = (url, title);
        Err("这里不能开网页窗口".to_string())
    }
    /// 这个插件的网页登录窗口还开着没有（玩家可能自己关了）。
    fn web_window_open(&self) -> bool {
        false
    }
    /// 网页登录窗口里某个网址的 Cookie（名字、值）。是登录凭据：不要记进日志。
    fn web_window_cookies(
        &self,
        url: &str,
    ) -> BoxFuture<'static, Result<Vec<(String, String)>, String>> {
        let _ = url;
        Box::pin(async { Err("没有网页窗口".to_string()) })
    }
    fn close_web_window(&self) {}
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
    /// 游戏中心：报一件游戏专用的事（`<适配器>.<名字>`）。`kind` 要是清单 `[[events]]` 里登记过的，宿主不收没登记的。
    fn game_event(
        &self,
        game: &str,
        kind: &str,
        source: &str,
        data: Map<String, Value>,
        game_time: Option<f64>,
    );
    /// 游戏中心里现在在玩的游戏：在跑的游戏里窗口在前台的那个；没有在前台的、只有一个在跑时就是它；都不是 = None。
    /// 不是游戏插件、自己不知道是哪个游戏的插件（同传、游戏语音）报事件（[`Host::game_event`]）、按局分卡片时用。
    fn playing_game(&self) -> Option<Playing> {
        None
    }
    /// 给插件自己的页面发事件（界面上是 `kotone://plugin` 事件，带 `plugin`、`event`、`payload`）。
    fn emit(&self, event: &str, payload: Value) {
        let _ = (event, payload);
    }
    /// 推送（ADR 043 D3–D5，见 [`push`] 模块）：卡片发进聊天、一行显示在悬浮窗、语音她说一句固定的短话，全都记进聊天；
    /// **不调模型、不花钱**。`note` 那一句作为一条隐藏的说明追加在对话末尾（她正忙时等她这一轮说完，不插进去；发给模型的
    /// 部分只追加，前缀缓存照常命中，ADR 024）。宿主按清单核对：`trigger` 要是 `[[pushes]]` 里的、开着、`needs` 的数据开关
    /// 开着，否则 [`PushOutcome::Off`]；只留这一条声明过的样子、截字（[`Push::shaped`]）。同一个 `key` 再推一次：内容变了
    /// 原地更新（[`PushOutcome::Updated`]），一样就什么都不做（[`PushOutcome::Unchanged`]）。
    ///
    /// **只给内置插件**（单向门，ADR 039 D3）：往聊天和她的上下文里放东西是很大的权力。第三方插件（远程插件、MCP 服务器）
    /// 的宿主不实现这个方法，用这里的默认实现一律拒绝。
    fn push(&self, push: Push) -> Result<PushOutcome, String> {
        let _ = push;
        Err("推送只给内置插件".to_string())
    }
    /// 这条推送结束了（例如三选一的卡片从屏幕上消失）：悬浮窗收起它的那一行；聊天里的记录留着。之后同一个 `key` 再推，
    /// 照旧是更新那一条。默认什么都不做。
    fn push_done(&self, trigger: &str, key: &str) {
        let _ = (trigger, key);
    }
    /// 这条推送现在推不推得出去（清单里有、开着、要的数据开关开着；规则和 [`Host::push`] 收推送时一样）。推送前要先做
    /// 花力气的事（例如海斗三选一每秒截图识别）的插件先看一眼，关着就不做；开关照旧由宿主执行，不靠插件查。
    /// 默认是 false（默认的 [`Host::push`] 一律拒绝）。
    fn push_enabled(&self, trigger: &str) -> bool {
        let _ = trigger;
        false
    }
    /// 发到游戏（ADR 041 第 3 期，英雄联盟的一键喊话）：把几行字交给本体**唯一的发话入口**——和她的 `game_chat.send`、
    /// 语音输入、自动化同一个入口：前台是哪个游戏、走哪条路（客户端在前台交给登记了发法的插件，对局里是本体按键）、
    /// 频道、一把输入锁、等修饰键、不补发都在入口里。插件拿到的只是“经入口发一段话”，自己不按键
    /// （`plugins_never_press_keys_themselves`）。`channel`：`team` / `all`，None = 游戏配置的默认频道（队伍）。
    /// 成了返回和 `game_chat.send` 一样的结果（`route`、`channelName`、`sentLines`、`receipt`……）；没发出去返回原因。
    ///
    /// 只在有人调这个插件的工具时能发（她、外部 Agent、玩家的热键；谁发的就照谁的规矩），插件自己在后台不能发。
    /// **只给内置插件**（单向门，ADR 041 D7）：第三方插件的宿主用这里的默认实现，一律拒绝。
    fn send_to_game(
        &self,
        text: &str,
        channel: Option<&str>,
    ) -> BoxFuture<'static, Result<Value, String>> {
        let _ = (text, channel);
        Box::pin(async { Err("发到游戏只给内置插件".to_string()) })
    }
    /// 截前台窗口的一块区域（客户区比例 `[x, y, w, h]`）并识别文字。前台不是 `process` 这个进程的窗口时不截（报错，
    /// 以“前台不是”开头）。Kotone 自己盖在游戏上的窗口（悬浮窗）不进截图，不会被认进去。
    /// `local_only = false`：用玩家在「设置 → 模型 → 文字识别」选的模型（可能是在线的）。
    /// `local_only = true`：**固定用本地模型，和全局的选择无关**（ADR 043 D7）：全局选的是本地的就用它，否则用装好的
    /// 第一个本地模型；一个都没有时报错（先看 [`Host::local_ocr`]）。
    fn screen_text(
        &self,
        process: &str,
        region: [f64; 4],
        local_only: bool,
    ) -> BoxFuture<'static, Result<Vec<ScreenText>, String>> {
        let _ = (process, region, local_only);
        Box::pin(async { Err("这里没有截图识别".to_string()) })
    }
    /// 本地文字识别能不能用（`screen_text(.., local_only = true)` 会用哪个模型；没有时推荐下载哪个、多大）。
    /// 要快：插件在循环里看它（只核对文件，不加载模型）。
    fn local_ocr(&self) -> LocalOcr {
        LocalOcr::Failed {
            reason: "这里没有本地文字识别".to_string(),
        }
    }
    /// 下载 [`LocalOcr::Missing`] 里推荐的那个本地文字识别模型（和「设置 → 模型 → 文字识别」的下载同一套：同一时间只下
    /// 一个、逐文件校验）。**不改**全局的文字识别选择。下完（或者本来就装好了）返回 Ok。只在玩家点了插件页面上的
    /// “下载”时调。
    fn download_local_ocr(&self) -> BoxFuture<'static, Result<(), String>> {
        Box::pin(async { Err("这里不能下载文字识别模型".to_string()) })
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

    /// 命令（ADR 040 D1）：清单里只登记命令，不带默认键（D4：写了 default_key 读不进来）。
    #[test]
    fn commands_are_registered_without_keys() {
        let base = "id = \"demo\"
name = \"示例\"
description = \"说明\"
";
        let ok = Manifest::parse(&format!(
            "{base}[[commands]]
id = \"talk\"
name = \"说话\"
description = \"按住说话\"
kind = \"hold\"
listens = true

[[commands]]
id = \"next\"
name = \"下一首\"
description = \"\"
"
        ))
        .unwrap();
        assert_eq!(ok.commands.len(), 2);
        assert_eq!(ok.commands[0].kind, CommandKind::Hold);
        assert!(ok.commands[0].listens);
        assert_eq!(ok.commands[1].kind, CommandKind::Press, "不写就是按一下");
        for bad in [
            "[[commands]]
id = \"talk\"
name = \"说话\"
description = \"\"
default_key = \"F8\"
",
            "[[commands]]
id = \"Talk\"
name = \"说话\"
description = \"\"
",
            "[[commands]]
id = \"a\"
name = \"甲\"
description = \"\"
[[commands]]
id = \"a\"
name = \"乙\"
description = \"\"
",
            "[[commands]]
id = \"a\"
name = \"甲\"
description = \"\"
kind = \"double\"
",
        ] {
            assert!(Manifest::parse(&format!("{base}{bad}")).is_err(), "{bad}");
        }
        // 不写 [[commands]] 的插件（还有第三方插件合成的清单）就是没有命令
        assert!(Manifest::parse(base).unwrap().commands.is_empty());
    }

    /// 调工具的命令（ADR 040 第 3 期）：只调自己的、给她的工具；按一下、不听；参数是表。
    #[test]
    fn tool_commands_call_the_plugins_own_tools() {
        let base = "id = \"demo\"
name = \"示例\"
description = \"说明\"
";
        let command = |extra: &str| {
            format!(
                "{base}[[commands]]
id = \"shout\"
name = \"喊一句\"
description = \"发到队伍聊天\"
{extra}
"
            )
        };
        let ok = Manifest::parse(&command(
            "tool = \"demo.shout\"\nargs = { what = \"flash\", side = \"enemy\" }",
        ))
        .unwrap();
        let spec = &ok.commands[0];
        assert_eq!(spec.tool.as_deref(), Some("demo.shout"));
        assert_eq!(
            spec.args,
            Some(serde_json::json!({"what": "flash", "side": "enemy"}))
        );
        let shout = ToolSpec::new("demo.shout", "喊", serde_json::json!({"type": "object"}));
        ok.check_tools(std::slice::from_ref(&shout)).unwrap();
        assert!(ok.check_tools(&[]).is_err(), "工具不在");
        assert!(
            ok.check_tools(&[shout.clone().app_only()]).is_err(),
            "只给页面的不能绑"
        );
        for bad in [
            "tool = \"settings.update\"",
            "tool = \"other.shout\"",
            "tool = \"demo.shout\"\nlistens = true",
            "tool = \"demo.shout\"\nkind = \"hold\"",
            "tool = \"demo.shout\"\nargs = [1, 2]",
            "args = { what = \"flash\" }",
        ] {
            assert!(Manifest::parse(&command(bad)).is_err(), "{bad}");
        }
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

    /// 许可和出处（ADR 043 D8，`[[notices]]`）：读得进来、发给界面是 camelCase；写错的读不进来。
    #[test]
    fn notices_are_parsed_and_checked() {
        let notice = |id: &str, title: &str, body: &str, links: &str| {
            format!(
                "{SAMPLE}\n[[notices]]\nid = \"{id}\"\ntitle = \"{title}\"\nbody = \"{body}\"\nlinks = [{links}]\n"
            )
        };
        let ok = notice(
            "aramhex-data",
            "海斗海克斯数据",
            "CC BY-NC 4.0：署名、不商用",
            "\"https://github.com/czgreat/aramhex-data\"",
        );
        let m = Manifest::parse(&ok).unwrap();
        assert_eq!(m.notices.len(), 1);
        assert_eq!(
            m.notices[0].links,
            ["https://github.com/czgreat/aramhex-data"]
        );
        let v = serde_json::to_value(&m).unwrap();
        assert_eq!(v["notices"][0]["id"], "aramhex-data");
        assert_eq!(v["notices"][0]["title"], "海斗海克斯数据");
        // 没写 = 没有；没写链接 = 空
        assert!(Manifest::parse(SAMPLE).unwrap().notices.is_empty());
        let no_links =
            format!("{SAMPLE}\n[[notices]]\nid = \"riot\"\ntitle = \"拳头\"\nbody = \"声明\"\n");
        assert!(Manifest::parse(&no_links).unwrap().notices[0]
            .links
            .is_empty());

        // 写错的：id 写法、重名、空标题 / 正文、不是 https、带空白、不认得的字段
        let bad = [
            notice("Riot", "拳头", "声明", ""),
            format!(
                "{ok}\n[[notices]]\nid = \"aramhex-data\"\ntitle = \"又一条\"\nbody = \"重名\"\n"
            ),
            notice("riot", " ", "声明", ""),
            notice("riot", "拳头", "", ""),
            notice("riot", "拳头", "声明", "\"http://example.com\""),
            notice("riot", "拳头", "声明", "\"https://\""),
            notice("riot", "拳头", "声明", "\"https://exa mple.com\""),
            format!("{ok}color = \"red\"\n"),
        ];
        for text in bad {
            assert!(Manifest::parse(&text).is_err(), "{text}");
        }
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
        // 本体的工具（ADR 039）：一样的样子，只是没有 plugin
        let mut body = spec.body_service_spec();
        assert!(body.get("plugin").is_none());
        body["plugin"] = json!("lol");
        assert_eq!(body, v);

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

    /// 覆盖层去掉了（ADR 043 D7）：清单里写 `[[overlays]]` 读不进来（不认得的字段不收）；本地文字识别的状态
    /// 是 `{state, ...}`（camelCase）。
    #[test]
    fn overlays_are_gone_and_local_ocr_status_serializes() {
        let old = format!(
            "{SAMPLE}\n[[overlays]]\nuri = \"ui://kotone/lol/augment-scores\"\ntitle = \"海克斯评分\"\n"
        );
        assert!(Manifest::parse(&old).is_err(), "覆盖层不再是清单的字段");
        let m = Manifest::parse(SAMPLE).unwrap();
        assert!(serde_json::to_value(&m).unwrap().get("overlays").is_none());
        let missing = LocalOcr::Missing {
            model: "ppocrv5-ch-mobile".into(),
            name: "PP-OCRv5 mobile".into(),
            bytes: 21_450_882,
        };
        assert_eq!(
            serde_json::to_value(&missing).unwrap(),
            json!({"state": "missing", "model": "ppocrv5-ch-mobile", "name": "PP-OCRv5 mobile", "bytes": 21_450_882})
        );
        let failed: LocalOcr =
            serde_json::from_value(json!({"state": "failed", "reason": "坏了"})).unwrap();
        assert_eq!(
            failed,
            LocalOcr::Failed {
                reason: "坏了".into()
            }
        );
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

    /// 资源只能是清单 `pages` 里的页面、类型对、CSP 声明写成源。
    #[test]
    fn resources_follow_the_manifest_pages() {
        let m = Manifest::parse(SAMPLE).unwrap();
        assert_eq!(m.page("ui://kotone/lol/match").unwrap().title, "这局");
        assert!(m.page("ui://kotone/lol/augment-scores").is_none());
        let ok = Resource::app("ui://kotone/lol/match", "<!doctype html><html></html>").csp(Csp {
            resource_domains: vec!["https://raw.communitydragon.org".into()],
            connect_domains: vec!["wss://*.example.com".into()],
            ..Csp::default()
        });
        assert!(m.check_resource(&ok).is_ok());
        let not_a_page = Resource::app("ui://kotone/lol/augment-scores", "<html></html>");
        assert!(m.check_resource(&not_a_page).is_err());
        let other = Resource::app("ui://kotone/music/match", "<html></html>");
        assert!(m.check_resource(&other).is_err());
        let mut wrong_type = ok.clone();
        wrong_type.mime_type = "text/html".into();
        assert!(m.check_resource(&wrong_type).is_err());
        let empty = Resource::app("ui://kotone/lol/match", "  ");
        assert!(m.check_resource(&empty).is_err());
        for bad in [
            "*",
            "https://raw.communitydragon.org/latest/",
            "'self'",
            "a.com b.com",
        ] {
            let r = ok.clone().csp(Csp {
                resource_domains: vec![bad.into()],
                ..Csp::default()
            });
            assert!(m.check_resource(&r).is_err(), "{bad}");
        }
    }

    /// 发到游戏的发法（ADR 041 D3）：给的游戏要在 `games` 里、一个游戏一项、窗口写成程序名、`needs` 是自己的开关。
    #[test]
    fn send_to_game_is_checked_against_the_manifest() {
        let chat = "\n[[send_to_game]]\ngame = \"lol\"\nwindows = [\"LeagueClientUx.exe\"]\nneeds = [\"client\"]\n";
        let m = Manifest::parse(&format!("{SAMPLE}{chat}")).unwrap();
        assert_eq!(m.send_to_game.len(), 1);
        assert!(m.send_to_game[0].covers("leagueclientux.exe"));
        let v = serde_json::to_value(&m).unwrap();
        assert_eq!(v["sendToGame"][0]["windows"][0], "LeagueClientUx.exe");
        // 不写就是没有
        assert!(Manifest::parse(SAMPLE).unwrap().send_to_game.is_empty());
        for bad in [
            chat.replace("game = \"lol\"", "game = \"dota2\""),
            format!("{chat}{chat}"),
            chat.replace("[\"LeagueClientUx.exe\"]", "[]"),
            chat.replace("LeagueClientUx.exe", "C:\\\\Riot\\\\LeagueClientUx.exe"),
            chat.replace("LeagueClientUx.exe", "LeagueClientUx"),
            chat.replace("[\"client\"]", "[\"chat\"]"),
            chat.replace("needs", "default_route"),
        ] {
            assert!(
                Manifest::parse(&format!("{SAMPLE}{bad}")).is_err(),
                "应该读不进：{bad}"
            );
        }
    }

    /// 插件不写 `resource` 时：页面都不是 App（还是宿主里的 Svelte 页面）。
    #[test]
    fn plugins_have_no_apps_by_default() {
        struct Bare(Manifest);
        impl Plugin for Bare {
            fn manifest(&self) -> &Manifest {
                &self.0
            }
            fn tools(&self) -> Vec<ToolSpec> {
                Vec::new()
            }
            fn call<'a>(
                &'a self,
                _host: Arc<dyn Host>,
                _tool: &'a str,
                _arguments: &'a Value,
            ) -> BoxFuture<'a, Result<ToolResult, String>> {
                Box::pin(async { Err("没有工具".to_string()) })
            }
        }
        let plugin = Bare(Manifest::parse(SAMPLE).unwrap());
        assert!(plugin.resource("ui://kotone/lol/match").is_none());
    }

    #[test]
    fn ids() {
        assert!(valid_id("voice-input"));
        assert!(!valid_id("Voice"));
        assert!(!valid_id("-a"));
        assert!(!valid_id(""));
    }

    /// ADR 039 D3、ADR 043 D3：推送只给内置插件。没实现它的宿主（远程插件）一律拒绝；`push_done` 什么都不做。
    #[test]
    fn pushes_are_for_built_in_plugins_only() {
        struct Remote;
        impl Host for Remote {
            fn plugin(&self) -> &str {
                "remote"
            }
            fn enabled(&self) -> bool {
                true
            }
            fn source_enabled(&self, _source: &str) -> bool {
                true
            }
            fn settings(&self) -> Value {
                json!({})
            }
            fn data_dir(&self) -> PathBuf {
                PathBuf::new()
            }
            fn spawn(&self, _task: BoxFuture<'static, ()>) {}
            fn process_ids(&self, _exe: &str) -> Vec<u32> {
                Vec::new()
            }
            fn process_command_line(&self, _pid: u32) -> Option<String> {
                None
            }
            fn game_observe(&self, _game: &str, _observation: Observation) {}
            fn game_claim(&self, _game: &str, _claimed: bool) {}
            fn game_event(
                &self,
                _game: &str,
                _kind: &str,
                _source: &str,
                _data: Map<String, Value>,
                _game_time: Option<f64>,
            ) {
            }
        }
        let push = Push {
            trigger: "opening".into(),
            key: "opening:1".into(),
            card: Some(PushCard {
                title: "开局战绩".into(),
                uri: "ui://kotone/lol/match".into(),
                params: Value::Null,
                data: json!({"players": []}),
            }),
            note: "开局战绩（海斗）。".into(),
            ..Push::default()
        };
        assert!(Remote.push(push).is_err_and(|e| e.contains("只给内置插件")));
        Remote.push_done("opening", "opening:1");
        // 发到游戏（ADR 041 第 3 期）同样是单向门：没实现的宿主一律拒绝
        struct Noop;
        impl std::task::Wake for Noop {
            fn wake(self: Arc<Self>) {}
        }
        let waker = std::task::Waker::from(Arc::new(Noop));
        let mut sending = Remote.send_to_game("亚索 闪 14:32好", Some("team"));
        let sent = sending
            .as_mut()
            .poll(&mut std::task::Context::from_waker(&waker));
        assert!(
            matches!(&sent, std::task::Poll::Ready(Err(e)) if e.contains("只给内置插件")),
            "{sent:?}"
        );
    }
}
