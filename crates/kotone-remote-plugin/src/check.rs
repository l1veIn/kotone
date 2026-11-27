//! 插件作者发布前的检查（`kotone-cli plugin check`，插件标准 0.1 第七节，ADR 047 D4 第 7 条）：
//! 照 Cortico 的 `pnpm check:extension`、MCP Inspector，连上作者的服务器，**用宿主同一份代码**看一遍 Kotone 会怎么待它。
//!
//! - **连接**：照 Kotone 现在的握手（`initialize`，2025-11-25）连一次；再用 2026-07-28（`server/discover`）连一次，
//!   两次都报告（第二次连不上只是提醒：现在的 Kotone 不用它）。
//! - **清单**：读 `kotone://manifest`（[`crate::server::read_manifest`]，和宿主一样），按 [`manifest::parse_text`]、
//!   [`manifest::merge`] 合成（服务器没给时照样认 Kotone 带的预设）：标准版本、不认识的字段、问题、事件的全名、游戏 id。
//!   不读玩家本地的 `manifest.json`（那是玩家的，不是作者发布的）。
//! - **工具**：名字合不合规矩（[`tools::valid_tool_name`]）、`_meta.ui`（[`tools::ui_meta`]）；**页面**：`ui://` 读不读得到、
//!   类型对不对、CSP 里会被丢掉的地址（[`public_csp_source`]）。
//! - **观察项**：每个真调一次，结果照宿主的 [`watch::result_value`]、[`watch::changed`]、[`watch::fresh`] 过一遍：
//!   有没有 `structuredContent`、`field` / `list` / `id` 找不找得到；事件 `data` 登记的字段结果里没有只是提醒；
//!   间隔低于下限时说按下限算（[`Watch::interval`]）。
//!
//! 三级：✓ 没问题、! 提醒（Kotone 照样能用，或者只是某一项用不了）、✗ 要先改（Kotone 用不了这一项，或者作者写错了）。
//! 不碰凭据库：Key（`${input:名字}`）由调用的人先填进 [`kotone_mcp::ServerConfig`]。

use std::collections::{BTreeMap, BTreeSet};

use kotone_mcp::{Handshake, McpClient, ServerConfig, Tool};
use kotone_plugin_api::resource::public_csp_source;
use kotone_plugin_api::APP_MIME;
use serde::Serialize;
use serde_json::{Map, Value};

use crate::config::ServerSpec;
use crate::manifest::{self, Compare, Effective, Source, Watch, HOST_STANDARD, MANIFEST_URI};
use crate::server::{app_pages, read_manifest};
use crate::tools;
use crate::watch::{self, Failure, Seen, ToolFact};

/// 一行的结论。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Level {
    /// ✓
    Ok,
    /// !：Kotone 照样能用，或者只是这一项用不了。
    Warn,
    /// ✗：要先改。
    Fail,
}

impl Level {
    pub fn mark(self) -> &'static str {
        match self {
            Level::Ok => "✓",
            Level::Warn => "!",
            Level::Fail => "✗",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Line {
    pub level: Level,
    pub text: String,
}

/// 一节（连接、清单、工具、页面、观察项）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Section {
    pub title: String,
    pub lines: Vec<Line>,
}

impl Section {
    fn new(title: &str) -> Self {
        Self {
            title: title.to_string(),
            lines: Vec::new(),
        }
    }

    fn push(&mut self, level: Level, text: impl Into<String>) {
        self.lines.push(Line {
            level,
            text: text.into(),
        });
    }

    fn ok(&mut self, text: impl Into<String>) {
        self.push(Level::Ok, text);
    }

    fn warn(&mut self, text: impl Into<String>) {
        self.push(Level::Warn, text);
    }

    fn fail(&mut self, text: impl Into<String>) {
        self.push(Level::Fail, text);
    }
}

/// 一次检查的结果。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    /// 插件 id（事件全名的前缀）。
    pub plugin: String,
    /// 运行什么 / 连哪里（“运行 程序 参数…”、“连接 网址”）。
    pub target: String,
    pub sections: Vec<Section>,
    /// Kotone 会照着用的清单（几层合起来以后，[`manifest::merge`]）；没连上是 None。
    pub manifest: Option<Effective>,
}

impl Report {
    /// 某一级有几行。
    pub fn count(&self, level: Level) -> usize {
        self.sections
            .iter()
            .flat_map(|s| &s.lines)
            .filter(|l| l.level == level)
            .count()
    }

    /// 所有行（节名、行）。
    pub fn lines(&self) -> impl Iterator<Item = (&str, &Line)> {
        self.sections
            .iter()
            .flat_map(|s| s.lines.iter().map(move |l| (s.title.as_str(), l)))
    }

    /// 退出码：有 ✗ 是 1；`strict` 时有 ! 也是 1；否则 0。
    pub fn exit_code(&self, strict: bool) -> i32 {
        let bad = self.count(Level::Fail) > 0 || (strict && self.count(Level::Warn) > 0);
        i32::from(bad)
    }
}

/// 检查什么、怎么连。
#[derive(Debug, Clone)]
pub struct Options {
    /// 插件 id（玩家在 `mcp.json` 里给的短名；事件全名 = `<id>.<名字>`）。
    pub id: String,
    /// 配置（本机还是网址、预设按网址认）。
    pub spec: ServerSpec,
    /// 填好 Key 的连接配置（[`ServerSpec::resolve`]）。握手方式这里会改，别的照用。
    pub config: ServerConfig,
    /// 这一版 Kotone 认识的游戏 id（None = 不查）。
    pub known_games: Option<Vec<String>>,
    /// 再用 2026-07-28 连一次。
    pub modern: bool,
}

/// 结果里的值写成一小段（给人看）。
fn short_json(value: &Value) -> String {
    tools::short_text(&value.to_string(), 80)
}

fn join(items: &[String]) -> String {
    items.join("、")
}

/// 连上、看一遍、断开。
pub async fn run(options: &Options) -> Report {
    let mut report = Report {
        plugin: options.id.clone(),
        target: if options.spec.is_stdio() {
            format!("运行 {}", options.spec.summary())
        } else {
            format!("连接 {}", options.spec.summary())
        },
        sections: Vec::new(),
        manifest: None,
    };
    let mut connect = Section::new("连接");
    let config = options.config.clone().handshake(Handshake::Initialize);
    let client = match McpClient::connect(&config).await {
        Ok(client) => client,
        Err(e) => {
            connect.fail(format!("2025-11-25（initialize）连不上：{e}"));
            report.sections.push(connect);
            return report;
        }
    };
    let info = client.server_info().clone();
    connect.ok(format!(
        "2025-11-25（initialize）连上了：服务器说自己是 {} {}，谈好的协议 {}",
        if info.name.is_empty() {
            "（没写名字）"
        } else {
            info.name.as_str()
        },
        info.version,
        info.protocol_version
    ));
    let tools = match client.list_tools().await {
        Ok(tools) => tools,
        Err(e) => {
            connect.fail(format!(
                "列不出工具（tools/list）：{e}。Kotone 会当成连不上"
            ));
            report.sections.push(connect);
            client.close().await;
            return report;
        }
    };
    // 不是每个服务器都有资源（没有时回“方法不存在”，和宿主一样当没有）
    let resources = client.list_resources().await.unwrap_or_default();
    let (text, error) = read_manifest(&client).await;

    if options.modern {
        let modern = options.config.clone().handshake(Handshake::Discover);
        match McpClient::connect(&modern).await {
            Ok(other) => {
                let (again, _) = read_manifest(&other).await;
                let names = |list: &[Tool]| list.iter().map(|t| t.name.clone()).collect::<Vec<_>>();
                let same_tools = other
                    .list_tools()
                    .await
                    .is_ok_and(|list| names(&list) == names(&tools));
                if again == text && same_tools {
                    connect.ok("2026-07-28（server/discover）也连得上，清单和工具清单一样");
                } else {
                    connect.warn(
                        "2026-07-28（server/discover）连得上，但读到的清单或工具清单和 2025-11-25 下不一样",
                    );
                }
                other.close().await;
            }
            Err(e) => connect.warn(format!(
                "2026-07-28（server/discover）连不上：{e}。现在的 Kotone 用 initialize，不影响；以后 Kotone 先试 server/discover、不认再退回"
            )),
        }
    }
    report.sections.push(connect);

    let effective = manifest_section(&mut report, options, &info.name, text, error);
    tools_section(&mut report, &tools);
    pages_section(&mut report, &client, &resources, &tools).await;
    watches_section(&mut report, options, &client, &tools, &effective).await;
    report.manifest = Some(effective);
    client.close().await;
    report
}

fn source_note(from: Source) -> &'static str {
    match from {
        Source::Preset => "（Kotone 带的预设里）",
        Source::Local => "（本地文件里）",
        Source::Server | Source::None => "",
    }
}

fn manifest_section(
    report: &mut Report,
    options: &Options,
    server_name: &str,
    text: Option<String>,
    error: Option<String>,
) -> Effective {
    let mut s = Section::new("Kotone 清单（kotone://manifest）");
    let server = match (&text, error) {
        (Some(text), _) => Some(manifest::parse_text(text)),
        (None, Some(error)) => Some(Err(error)),
        (None, None) => None,
    };
    let url = options.spec.url();
    let name = (!server_name.is_empty()).then_some(server_name);
    let preset = manifest::preset_for(url, name);
    match (&server, &preset) {
        (Some(Ok(parsed)), _) => {
            s.ok(format!(
                "服务器给了清单（{} 字节）",
                text.as_deref().map_or(0, str::len)
            ));
            if parsed.newer_standard {
                s.warn(format!(
                    "standard 写的是 {}，比这一版 Kotone 认识的 {HOST_STANDARD} 新：照认识的字段读，插件页会提示“这个插件按更新的标准写的”",
                    parsed.manifest.standard
                ));
            } else {
                s.ok(format!("照标准 {} 写的", parsed.manifest.standard));
            }
        }
        // 读不出来的原因在 merge 的问题里（✗）
        (Some(Err(_)), _) => {}
        (None, Some((file, _))) => s.warn(format!(
            "服务器没给清单（读不到 {MANIFEST_URI}）：Kotone 会用它带的预设 {file}"
        )),
        (None, None) => s.warn(format!(
            "服务器没给清单（读不到 {MANIFEST_URI}）：Kotone 照样能用它的工具，只是没有事件、不挂在游戏上"
        )),
    }
    let effective = manifest::merge(server, preset.map(|(_, parsed)| parsed), None);
    if effective.source == Source::Preset {
        s.warn(format!(
            "下面是 Kotone 带的预设 {}，不是服务器给的",
            effective.preset.as_deref().unwrap_or("")
        ));
    }
    for note in &effective.unknown {
        s.warn(format!(
            "不认识的字段 {}{}：Kotone 忽略它，插件页会列出来（按更新的标准写的，或者拼错了）",
            note.text,
            source_note(note.from)
        ));
    }
    for note in &effective.problems {
        s.fail(format!("{}{}", note.text, source_note(note.from)));
    }
    let m = &effective.manifest;
    if effective.source != Source::None {
        if m.games.is_empty() {
            s.ok("games 是空的：不挂在哪个游戏上（观察项一直查，间隔下限 60 秒）");
        } else {
            let unknown: Vec<String> = match &options.known_games {
                Some(known) => m
                    .games
                    .iter()
                    .filter(|g| !known.contains(g))
                    .cloned()
                    .collect(),
                None => Vec::new(),
            };
            s.ok(format!("挂在游戏 {} 上", join(&m.games)));
            if !unknown.is_empty() {
                s.warn(format!(
                    "这一版 Kotone 没有游戏 {}：保留着，插件页会提示（游戏 id 见 crates/kotone-core/resources/profiles/）",
                    join(&unknown)
                ));
            }
        }
        for event in &m.events {
            let fields: Vec<String> = event.data.keys().cloned().collect();
            s.ok(format!(
                "事件 {}：{}{}",
                manifest::event_type(&options.id, &event.name),
                event.label,
                if fields.is_empty() {
                    String::new()
                } else {
                    format!("（data：{}）", join(&fields))
                }
            ));
        }
        if let Some(risk) = &m.risk {
            s.ok(format!(
                "有风险说明（{} 字）：第一次点开插件时在“风险和许可”弹窗里显示",
                risk.chars().count()
            ));
        }
    }
    report.sections.push(s);
    effective
}

fn tools_section(report: &mut Report, list: &[Tool]) {
    let mut s = Section::new("工具（tools/list）");
    let mut seen = BTreeSet::new();
    let valid: Vec<&Tool> = list
        .iter()
        .filter(|t| tools::valid_tool_name(&t.name))
        .collect();
    let read_only = valid.iter().filter(|t| tools::read_only(t)).count();
    s.ok(format!(
        "{} 个工具，{read_only} 个标了只读（annotations.readOnlyHint: true）",
        valid.len()
    ));
    let mut app_only = Vec::new();
    for tool in list {
        if !tools::valid_tool_name(&tool.name) {
            s.fail(format!(
                "工具名 \"{}\" 不合规矩（字母、数字和 _-.，1 到 128 个字）：这个工具不进 Kotone",
                tool.name
            ));
            continue;
        }
        if !seen.insert(tool.name.clone()) {
            s.warn(format!("工具 {} 重名了：Kotone 只用第一个", tool.name));
            continue;
        }
        if tool
            .description
            .as_deref()
            .or(tool.title.as_deref())
            .is_none_or(|d| d.trim().is_empty())
        {
            s.warn(format!(
                "工具 {} 没有说明（description）：她不知道什么时候该用它",
                tool.name
            ));
        }
        let meta = tool.meta.as_ref();
        let ui = meta.and_then(|m| m.get("ui"));
        let (visibility, page) = tools::ui_meta(tool);
        if let Some(values) = ui
            .and_then(|u| u.get("visibility"))
            .and_then(Value::as_array)
        {
            for v in values {
                if !matches!(v.as_str(), Some("model" | "app")) {
                    s.warn(format!(
                        "工具 {} 的 _meta.ui.visibility 里有不认识的 {v}（认 \"model\"、\"app\"）",
                        tool.name
                    ));
                }
            }
        }
        if visibility == [kotone_plugin_api::Visibility::App] {
            app_only.push(tool.name.clone());
        }
        let declared = ui
            .and_then(|u| u.get("resourceUri"))
            .or_else(|| meta.and_then(|m| m.get("ui/resourceUri")));
        if let Some(uri) = declared {
            if page.is_none() {
                s.warn(format!(
                    "工具 {} 的 _meta.ui.resourceUri（{}）不是 ui:// 地址：Kotone 不认",
                    tool.name,
                    short_json(uri)
                ));
            } else if ui.and_then(|u| u.get("resourceUri")).is_none() {
                s.warn(format!(
                    "工具 {} 用的是旧的扁平写法 _meta[\"ui/resourceUri\"]：Kotone 认，规范的写法是 _meta.ui.resourceUri",
                    tool.name
                ));
            }
        }
    }
    if !app_only.is_empty() {
        s.ok(format!(
            "只给页面用的工具（_meta.ui.visibility: [\"app\"]，不进她的清单）：{}",
            join(&app_only)
        ));
    }
    report.sections.push(s);
}

/// `_meta.ui.csp` 里会被 Kotone 丢掉的地址（本机、局域网、写法不对的）。
fn dropped_csp(meta: Option<&Value>) -> Vec<String> {
    let Some(csp) = meta
        .and_then(|m| m.get("ui"))
        .and_then(|u| u.get("csp"))
        .and_then(Value::as_object)
    else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for key in [
        "connectDomains",
        "resourceDomains",
        "frameDomains",
        "baseUriDomains",
    ] {
        for item in csp.get(key).and_then(Value::as_array).into_iter().flatten() {
            let text = item.as_str().unwrap_or_default();
            if public_csp_source(text).is_none() {
                out.push(format!("{key} 里的 {}", short_json(item)));
            }
        }
    }
    out
}

async fn pages_section(
    report: &mut Report,
    client: &McpClient,
    resources: &[kotone_mcp::Resource],
    list: &[Tool],
) {
    let mut s = Section::new("页面（MCP Apps，ui://）");
    let pages = app_pages(resources, list);
    if pages.is_empty() {
        s.ok("没有 ui:// 页面：点开插件卡片显示一页工具列表");
    }
    for (uri, title) in pages {
        let contents = match client.read_resource(&uri).await {
            Ok(contents) => contents,
            Err(e) => {
                s.fail(format!("{uri}（{title}）读不了：{e}"));
                continue;
            }
        };
        let Some(item) = contents.iter().find(|c| c.text.is_some()) else {
            s.fail(format!("{uri}（{title}）没有文字内容：不是网页"));
            continue;
        };
        let mime = item
            .mime_type
            .as_deref()
            .unwrap_or("")
            .to_ascii_lowercase()
            .replace(' ', "");
        let size = item.text.as_deref().map_or(0, str::len);
        if !mime.starts_with("text/html") {
            s.fail(format!(
                "{uri}（{title}）的类型是 \"{mime}\"：要是 {APP_MIME}，Kotone 不显示"
            ));
            continue;
        }
        if mime != APP_MIME {
            s.warn(format!(
                "{uri}（{title}）的类型是 \"{mime}\"：Kotone 照样显示，规范要 {APP_MIME}"
            ));
        } else {
            s.ok(format!(
                "{uri}（{title}）读得到：{} KiB，{APP_MIME}",
                size.div_ceil(1024)
            ));
        }
        let listed = resources
            .iter()
            .find(|r| r.uri == uri)
            .and_then(|r| r.meta.as_ref());
        let mut dropped = dropped_csp(item.meta.as_ref());
        if item.meta.as_ref().and_then(|m| m.get("ui")).is_none() {
            dropped = dropped_csp(listed);
        }
        if !dropped.is_empty() {
            s.warn(format!(
                "{uri} 的 _meta.ui.csp 里这些会被丢掉（本机、局域网的地址，或者写法不对）：{}",
                join(&dropped)
            ));
        }
    }
    report.sections.push(s);
}

fn compare_text(watch: &Watch) -> String {
    match &watch.compare {
        Compare::Changed { field, to } if to.is_empty() => format!("{field} 变了就发"),
        Compare::Changed { field, to } => format!("{field} 变成 {} 时发", join(to)),
        Compare::New { list, id } => format!(
            "{} 里按 {id} 出现新条目时发",
            list.as_deref().unwrap_or("结果本身")
        ),
    }
}

/// 结果里（`changed` 看顶层，`new` 看元素）能对上事件 `data` 登记的哪些字段（[`watch::registered`]，和发事件时一样）。
fn matched_fields(
    objects: &[&Map<String, Value>],
    fields: &BTreeMap<String, String>,
) -> BTreeSet<String> {
    objects
        .iter()
        .flat_map(|o| o.keys())
        .filter_map(|k| watch::registered(fields, k))
        .collect()
}

async fn watches_section(
    report: &mut Report,
    options: &Options,
    client: &McpClient,
    list: &[Tool],
    effective: &Effective,
) {
    let mut s = Section::new("观察项（watches）");
    let m = &effective.manifest;
    let local = options.spec.is_local();
    let has_games = !m.games.is_empty();
    for event in &m.events {
        if !m.watches.iter().any(|w| w.event == event.name) {
            s.warn(format!(
                "事件 {} 没有观察项：Kotone 现在只能靠观察项产生第三方事件（还不支持 MCP Events 订阅），这个事件发不出来",
                event.name
            ));
        }
    }
    if m.watches.is_empty() {
        s.ok("没有观察项");
    }
    for w in &m.watches {
        let head = format!("{} ← {}（{}）", w.event, w.tool, compare_text(w));
        let floor = w.interval(local, has_games).as_secs();
        if w.every_seconds < floor {
            let why = if !local {
                "别的网址"
            } else if !has_games {
                "games 为空、一直查"
            } else {
                "本机"
            };
            s.warn(format!(
                "{head}：写的每 {} 秒低于下限 {floor} 秒（{why}），按 {floor} 秒算",
                w.every_seconds
            ));
        } else {
            s.ok(format!("{head}：每 {} 秒查一次", w.every_seconds));
        }
        match watch::tool_fact(list, &w.tool) {
            ToolFact::Missing | ToolFact::Unknown => {
                s.fail(format!("{}：服务器上没有工具 {}", w.event, w.tool));
                continue;
            }
            ToolFact::Unmarked => s.warn(format!(
                "{}：工具 {} 没标只读（annotations.readOnlyHint: true）：玩家打开这个观察项时要明确同意定时调它",
                w.event, w.tool
            )),
            ToolFact::ReadOnly => {}
        }
        let text_ok = effective.watch_from.get(&w.event) == Some(&Source::Preset);
        let result = match client
            .call_tool(&w.tool, Value::Object(w.arguments.clone()))
            .await
        {
            Ok(result) => result,
            Err(e) => {
                s.fail(format!("{}：调 {} 出错：{e}", w.event, w.tool));
                continue;
            }
        };
        let value = match watch::result_value(&result, text_ok) {
            Ok(value) => value,
            Err(Failure::Stop(reason) | Failure::Retry(reason)) => {
                s.fail(format!("{}：调了一次 {}：{reason}", w.event, w.tool));
                continue;
            }
        };
        let from = if result.structured_content.is_some() {
            "structuredContent"
        } else {
            "文字里的 JSON（只有 Kotone 的预设可以这样）"
        };
        let fields = m
            .events
            .iter()
            .find(|e| e.name == w.event)
            .map(|e| e.data.clone())
            .unwrap_or_default();
        let wanted: Vec<String> = fields
            .keys()
            .filter(|f| {
                !(matches!(w.compare, Compare::Changed { .. }) && (*f == "from" || *f == "to"))
            })
            .cloned()
            .collect();
        let objects: Vec<&Map<String, Value>>;
        match &w.compare {
            Compare::Changed { field, to } => {
                // 宿主的比较：第一次只记底；找不到字段 = 没查成
                if let Err(reason) = watch::changed(&mut None, &value, field, to, &fields) {
                    s.fail(format!(
                        "{}：调了一次 {}（{from}）：{reason}，每次都会算没查成",
                        w.event, w.tool
                    ));
                    continue;
                }
                let now = manifest::lookup(&value, field).cloned().unwrap_or_default();
                s.ok(format!(
                    "{}：调了一次 {}（{from}）：{field} 现在是 {}",
                    w.event,
                    w.tool,
                    short_json(&now)
                ));
                objects = value.as_object().into_iter().collect();
                let not_scalar: Vec<String> = objects
                    .iter()
                    .flat_map(|o| o.iter())
                    .filter(|(k, v)| {
                        watch::registered(&fields, k).is_some()
                            && !matches!(
                                v,
                                Value::String(_) | Value::Number(_) | Value::Bool(_) | Value::Null
                            )
                    })
                    .map(|(k, _)| k.clone())
                    .collect();
                if !not_scalar.is_empty() {
                    s.warn(format!(
                        "{}：结果顶层的 {} 不是字符串、数字或布尔：changed 发的事件只带标量字段，这些不带",
                        w.event,
                        join(&not_scalar)
                    ));
                }
            }
            Compare::New { list: path, id } => {
                if let Err(reason) =
                    watch::fresh(&mut Seen::default(), &value, path.as_deref(), id, &fields)
                {
                    s.fail(format!(
                        "{}：调了一次 {}（{from}）：{reason}，每次都会算没查成",
                        w.event, w.tool
                    ));
                    continue;
                }
                let items: Vec<&Value> = manifest::lookup(&value, path.as_deref().unwrap_or(""))
                    .and_then(Value::as_array)
                    .map(|a| a.iter().take(manifest::MAX_SEEN_IDS).collect())
                    .unwrap_or_default();
                let with_id = items
                    .iter()
                    .filter(|item| {
                        matches!(
                            manifest::lookup(item, id),
                            Some(Value::Number(_)) | Some(Value::String(_))
                        ) && manifest::lookup(item, id) != Some(&Value::String(String::new()))
                    })
                    .count();
                let where_ = path.as_deref().unwrap_or("结果");
                if items.is_empty() {
                    s.warn(format!(
                        "{}：调了一次 {}（{from}）：{where_} 现在是空的，看不出元素里有没有 {id}",
                        w.event, w.tool
                    ));
                } else if with_id == 0 {
                    s.fail(format!(
                        "{}：调了一次 {}（{from}）：{where_} 里 {} 个元素都没有 {id}（要是字符串或数字）：一个都发不出来",
                        w.event,
                        w.tool,
                        items.len()
                    ));
                } else if with_id < items.len() {
                    s.warn(format!(
                        "{}：调了一次 {}（{from}）：{where_} 里 {} 个元素有 {} 个没有 {id}，这些不会发",
                        w.event,
                        w.tool,
                        items.len(),
                        items.len() - with_id
                    ));
                } else {
                    s.ok(format!(
                        "{}：调了一次 {}（{from}）：{where_} 里 {} 个元素，都有 {id}",
                        w.event,
                        w.tool,
                        items.len()
                    ));
                }
                objects = items.iter().filter_map(|v| v.as_object()).collect();
            }
        }
        if objects.is_empty() && !wanted.is_empty() {
            continue;
        }
        let matched = matched_fields(&objects, &fields);
        let missing: Vec<String> = wanted
            .into_iter()
            .filter(|f| !matched.contains(f))
            .collect();
        if !missing.is_empty() {
            let place = match w.compare {
                Compare::Changed { .. } => "结果顶层",
                Compare::New { .. } => "列表的元素",
            };
            s.warn(format!(
                "{}：事件 data 登记了 {}，这次{place}里没有：发事件时不带（只是提醒；snake_case 的字段名也对得上）",
                w.event,
                join(&missing)
            ));
        }
    }
    report.sections.push(s);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes_follow_the_worst_line() {
        let mut report = Report {
            plugin: "p".into(),
            target: "x".into(),
            sections: vec![Section::new("a")],
            manifest: None,
        };
        assert_eq!(report.exit_code(true), 0);
        report.sections[0].warn("提醒");
        assert_eq!(report.exit_code(false), 0, "只有 ! 是 0");
        assert_eq!(report.exit_code(true), 1, "--strict 时 ! 也算");
        report.sections[0].fail("错了");
        assert_eq!(report.exit_code(false), 1);
        assert_eq!(report.count(Level::Warn), 1);
        assert_eq!(report.lines().count(), 2);
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["sections"][0]["lines"][1]["level"], "fail");
    }

    #[test]
    fn local_and_lan_csp_entries_are_reported() {
        let meta = serde_json::json!({"ui": {"csp": {
            "connectDomains": ["https://api.example.com", "http://127.0.0.1:8080"],
            "resourceDomains": ["http://router.local"]
        }}});
        let dropped = dropped_csp(Some(&meta));
        assert_eq!(dropped.len(), 2, "{dropped:?}");
        assert!(dropped[0].contains("127.0.0.1"));
        assert!(dropped_csp(None).is_empty());
    }
}
