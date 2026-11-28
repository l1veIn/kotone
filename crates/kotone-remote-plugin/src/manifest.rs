//! Kotone 清单（插件标准 0.1，ADR 047 D3–D5；给插件作者看的写法在 `docs/plugin-standard.md`）：
//! 第三方插件说自己挂在哪个游戏上、会发哪些事件、怎么从只读工具里产生事件（观察项）、风险说明。
//!
//! - **从哪来**（[`merge`]）：服务器自己给的（MCP 资源 [`MANIFEST_URI`]）→ 没给时用 Kotone 带的预设
//!   （`presets/*.json`，[`preset_for`]）→ 再**只加**玩家本地的 `<KOTONE_HOME>/plugins/<id>/manifest.json`
//!   （[`LOCAL_FILE`]）里的事件和观察项；`standard`、`risk` 只认前两种。
//! - **读法**（[`parse_text`]，D4 第 2 条）：不认识的字段忽略、把路径列出来给作者看；`experimental` 开头的忽略、不列；
//!   `_meta` 里的键一律忽略（Kotone 的前缀 [`META_PREFIX`] 留给以后）；缺的用默认值；类型不对的当缺、
//!   不合格的条目丢掉，都列进“问题”。整份不是 JSON 对象、超过 [`MAX_BYTES`] = 没给清单。
//! - 内置插件的 `plugin.toml` 不走这里（照旧 `deny_unknown_fields`，ADR 047 D1：内层不是标准）。
//!
//! 观察项在这里解析（[`Watch`]），轮询、比较、发事件在 [`crate::watch`]（契约第 3 节的数在这里：[`LOCAL_FLOOR`] 等）。
//! 玩家在插件页“事件”一块里自己加的观察项写进本地文件（[`add_local`]、[`remove_local`]）。
//!
//! 照谁做：读法照 MCP 自己的“不认识的字段忽略”和 VS Code 扩展清单（`engines` 写最低版本）；
//! 观察项照 Home Assistant 的 REST 传感器、Zapier / n8n 的轮询触发（ADR 047 D5）。

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::sanitize;

/// 服务器给清单用的 MCP 资源（`resources/read`，不依赖 `resources/list` 里列不列它）。
pub const MANIFEST_URI: &str = "kotone://manifest";
/// 清单资源的类型。
pub const MANIFEST_MIME: &str = "application/json";
/// 清单最大多大（读回来的文字、本地文件）：超了当没给。
pub const MAX_BYTES: usize = 64 * 1024;
/// 这一版 Kotone 照的标准版本。
pub const HOST_STANDARD: &str = "0.1";
/// Kotone 在 `_meta` 里的前缀（和应用标识一致，照 MCP 的反向域名约定）。0.1 里没有字段用到，留给以后的工具级提示。
pub const META_PREFIX: &str = "com.l1vein.kotone/";
/// 玩家本地的清单（在插件自己的目录 `<KOTONE_HOME>/plugins/<id>/` 里）。
pub const LOCAL_FILE: &str = "manifest.json";
/// 事件说明最长多少字。
pub const MAX_LABEL: usize = 64;
/// 风险说明最长多少字（超出的截掉）。
pub const MAX_RISK: usize = 2000;
/// 事件数据字段的说明最长多少字（超出的截掉）。
pub const MAX_FIELD_ABOUT: usize = 200;
/// 观察项不写间隔时多久查一次（秒）。
pub const DEFAULT_EVERY: u64 = 60;
/// 间隔下限：本机的服务器（子进程、127.0.0.1 / localhost）。
pub const LOCAL_FLOOR: u64 = 10;
/// 间隔下限：别的网址；`games` 为空（一直轮询）时也用它。
pub const REMOTE_FLOOR: u64 = 60;
/// `new` 记底的 id 最多存多少个（旧的丢掉）。
pub const MAX_SEEN_IDS: usize = 500;
/// 事件内容里单个字符串截到多少字。
pub const MAX_EVENT_TEXT: usize = 500;
/// 游戏退出后再查最后一次的宽限。
pub const GAME_GRACE: Duration = Duration::from_secs(180);
/// 失败退避的上限。
pub const MAX_BACKOFF: Duration = Duration::from_secs(600);
/// 连续失败几次停下。
pub const MAX_FAILURES: u32 = 5;

/// 生效的清单（几层合起来以后）。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KotoneManifest {
    /// 写清单时照的标准版本（最低要求）。
    pub standard: String,
    /// 挂在哪些游戏上（游戏 id，和本体的游戏列表同一套；不认识的也留着）。
    pub games: Vec<String>,
    pub events: Vec<EventDecl>,
    pub watches: Vec<Watch>,
    /// 风险说明（进这个插件的“风险和许可”弹窗，ADR 043）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub risk: Option<String>,
}

impl Default for KotoneManifest {
    fn default() -> Self {
        Self {
            standard: HOST_STANDARD.to_string(),
            games: Vec::new(),
            events: Vec::new(),
            watches: Vec::new(),
            risk: None,
        }
    }
}

/// 登记的一个事件。宿主里完整的名字是 `<插件 id>.<name>`（[`event_type`]）。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventDecl {
    /// snake_case：小写字母开头，小写字母、数字、下划线。
    pub name: String,
    /// 一句中文说明（≤ [`MAX_LABEL`] 字）。
    pub label: String,
    /// 数据字段（camelCase）→ 说明。
    pub data: BTreeMap<String, String>,
}

/// 一个观察项（ADR 047 D5）：隔一阵子调一次只读工具，比较结果，变了就发 `event`。每个事件最多一个观察项。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Watch {
    /// 发哪个事件（`events[].name`）。
    pub event: String,
    /// 服务器上工具的原名（不带 `<id>.`）。
    pub tool: String,
    pub arguments: Map<String, Value>,
    /// 清单里写的间隔（秒，没写是 [`DEFAULT_EVERY`]）；真用的间隔看 [`Watch::interval`]。
    pub every_seconds: u64,
    #[serde(flatten)]
    pub compare: Compare,
}

/// 两种比较。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "compare", rename_all = "camelCase")]
pub enum Compare {
    /// 状态变了：取 `field`（点分隔的路径），第一次只记底；之后变了（且是 `to` 之一，`to` 空 = 不限）才发。
    Changed { field: String, to: Vec<String> },
    /// 新条目：`list`（路径，没写 = 结果本身）是数组，按元素的 `id` 字段去重，第一次只记底。
    New { list: Option<String>, id: String },
}

impl Watch {
    /// 真用的间隔：不低于下限（本机 [`LOCAL_FLOOR`]，别的网址 [`REMOTE_FLOOR`]；`games` 为空、一直轮询时也是
    /// [`REMOTE_FLOOR`]）。
    pub fn interval(&self, local: bool, has_games: bool) -> Duration {
        let floor = if local && has_games {
            LOCAL_FLOOR
        } else {
            REMOTE_FLOOR
        };
        Duration::from_secs(self.every_seconds.max(floor))
    }
}

/// 宿主里事件的完整名字：`<插件 id>.<name>`。
pub fn event_type(plugin: &str, name: &str) -> String {
    format!("{plugin}.{name}")
}

/// 按点分隔的路径取值（`run.status`、`items.0.state`；空路径 = 它本身）。数字段在数组里当下标。
pub fn lookup<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    if path.is_empty() {
        return Some(value);
    }
    path.split('.')
        .try_fold(value, |current, part| match current {
            Value::Object(map) => map.get(part),
            Value::Array(items) => part.parse::<usize>().ok().and_then(|i| items.get(i)),
            _ => None,
        })
}

/// 一层清单（服务器的、预设、本地的）读出来的样子。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Parsed {
    pub manifest: KotoneManifest,
    /// 写了 `standard` 但比 [`HOST_STANDARD`] 新。
    pub newer_standard: bool,
    /// 不认识的字段（路径：`colour`、`watches[0].retry`）。
    pub unknown: Vec<String>,
    /// 类型不对、不合格、丢掉的条目（一句话）。
    pub problems: Vec<String>,
}

/// 清单从哪来。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Source {
    /// 服务器自己给的（`kotone://manifest`）。
    Server,
    /// Kotone 带的预设。
    Preset,
    /// 只有玩家本地的 `manifest.json`。
    Local,
    /// 没有清单。
    #[default]
    None,
}

/// 一条给作者看的提示，带它出在哪一层。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Note {
    pub from: Source,
    pub text: String,
}

/// 合起来以后的结果：生效的清单、它从哪来、给作者看的提示。
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Effective {
    pub source: Source,
    pub manifest: KotoneManifest,
    /// 玩家本地的 `manifest.json` 加了东西进来。
    pub local: bool,
    /// 清单（服务器的或预设）写的标准比这一版 Kotone 新。
    pub newer_standard: bool,
    pub unknown: Vec<Note>,
    pub problems: Vec<Note>,
    /// 每个观察项（按事件名）是哪一层的：预设的才能把文字当 JSON 读，玩家自己加的（`local`）才能在界面上删。
    pub watch_from: BTreeMap<String, Source>,
    /// 用上了 Kotone 带的哪份预设（预设的 `id`，如 `zzz-onedragon`；自动化模板按它找插件）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
}

/// 读一份清单的文字。`Err` = 当没给清单（太大、不是 JSON、不是对象），里面是给插件页的一句话。
pub fn parse_text(text: &str) -> Result<Parsed, String> {
    if text.len() > MAX_BYTES {
        return Err(format!(
            "清单太大了（{} KiB，最多 64 KiB），当没给",
            text.len().div_ceil(1024)
        ));
    }
    let value: Value = serde_json::from_str(text.trim_start_matches('\u{FEFF}'))
        .map_err(|e| format!("清单不是合法的 JSON（{e}），当没给"))?;
    parse_value(&value)
}

/// 读一份清单（JSON）。`Err` = 不是对象，当没给。
pub fn parse_value(value: &Value) -> Result<Parsed, String> {
    let object = value.as_object().ok_or("清单要是一个 JSON 对象，当没给")?;
    let mut reader = Reader::default();
    let manifest = reader.manifest(object);
    let newer_standard = newer(&manifest.standard);
    Ok(Parsed {
        manifest,
        newer_standard,
        unknown: reader.unknown,
        problems: reader.problems,
    })
}

/// 版本号 `主.次`（可以多一段补丁号）。
fn version(text: &str) -> Option<(u64, u64)> {
    let mut parts = text.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    match (parts.next(), parts.next()) {
        (None, _) => Some((major, minor)),
        (Some(patch), None) if patch.parse::<u64>().is_ok() => Some((major, minor)),
        _ => None,
    }
}

/// 比这一版 Kotone 照的标准新。
pub fn newer(standard: &str) -> bool {
    match (version(standard), version(HOST_STANDARD)) {
        (Some(theirs), Some(ours)) => theirs > ours,
        _ => false,
    }
}

#[derive(Default)]
struct Reader {
    unknown: Vec<String>,
    problems: Vec<String>,
}

fn path(parent: &str, key: &str) -> String {
    if parent.is_empty() {
        key.to_string()
    } else {
        format!("{parent}.{key}")
    }
}

/// 事件名：小写字母开头，小写字母、数字、下划线，最长 64。
fn snake_case(name: &str) -> bool {
    name.len() <= 64
        && name.starts_with(|c: char| c.is_ascii_lowercase())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// 数据字段名：小写字母开头，字母、数字，最长 64。
fn camel_case(name: &str) -> bool {
    name.len() <= 64
        && name.starts_with(|c: char| c.is_ascii_lowercase())
        && name.chars().all(|c| c.is_ascii_alphanumeric())
}

/// 别人写的字：剥掉看不见的字符、去掉首尾空白。
fn text(value: &str) -> String {
    sanitize::strip_invisible(value).trim().to_string()
}

impl Reader {
    /// 这一层里不认识的字段：列出来（`experimental` 开头的、`_meta` 不列）。
    fn unknown_keys(&mut self, at: &str, object: &Map<String, Value>, known: &[&str]) {
        for key in object.keys() {
            if known.contains(&key.as_str()) || key == "_meta" || key.starts_with("experimental") {
                continue;
            }
            self.unknown.push(path(at, key));
        }
    }

    fn problem(&mut self, text: String) {
        self.problems.push(text);
    }

    /// 字符串字段：缺 / null = None；类型不对 = None 并记一条问题。
    fn string(&mut self, object: &Map<String, Value>, at: &str, key: &str) -> Option<String> {
        match object.get(key) {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) => Some(text(s)),
            Some(_) => {
                self.problem(format!("{} 要是字符串，当没写", path(at, key)));
                None
            }
        }
    }

    /// 数组字段：缺 / null = 空；类型不对 = 空并记一条问题。
    fn array<'v>(&mut self, object: &'v Map<String, Value>, key: &str) -> &'v [Value] {
        match object.get(key) {
            None | Some(Value::Null) => &[],
            Some(Value::Array(items)) => items,
            Some(_) => {
                self.problem(format!("{key} 要是数组，当没写"));
                &[]
            }
        }
    }

    fn manifest(&mut self, object: &Map<String, Value>) -> KotoneManifest {
        self.unknown_keys(
            "",
            object,
            &["standard", "games", "events", "watches", "risk", "$schema"],
        );
        let standard = match self.string(object, "", "standard") {
            None => HOST_STANDARD.to_string(),
            Some(s) if version(&s).is_some() => s,
            Some(s) => {
                self.problem(format!(
                    "standard 要写成 \"0.1\" 这样的版本号（写的是 \"{s}\"），当 {HOST_STANDARD}"
                ));
                HOST_STANDARD.to_string()
            }
        };
        let mut games: Vec<String> = Vec::new();
        for (i, item) in self.array(object, "games").iter().enumerate() {
            match item.as_str().map(text) {
                Some(id) if !id.is_empty() && id.chars().count() <= 64 => {
                    if !games.contains(&id) {
                        games.push(id);
                    }
                }
                _ => self.problem(format!("games[{i}] 要是游戏 id（字符串），丢掉")),
            }
        }
        let mut events: Vec<EventDecl> = Vec::new();
        for (i, item) in self.array(object, "events").iter().enumerate() {
            if let Some(event) = self.event(i, item) {
                if events.iter().any(|e| e.name == event.name) {
                    self.problem(format!("events[{i}]：事件 {} 重名了，丢掉", event.name));
                } else {
                    events.push(event);
                }
            }
        }
        let mut watches: Vec<Watch> = Vec::new();
        for (i, item) in self.array(object, "watches").iter().enumerate() {
            if let Some(watch) = self.watch(i, item) {
                if watches.iter().any(|w| w.event == watch.event) {
                    self.problem(format!(
                        "watches[{i}]：事件 {} 已经有一个观察项了，丢掉",
                        watch.event
                    ));
                } else {
                    watches.push(watch);
                }
            }
        }
        let risk = match object.get("risk") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) => {
                let s = sanitize::clean(s, MAX_RISK);
                let s = s.trim();
                (!s.is_empty()).then(|| s.to_string())
            }
            Some(_) => {
                self.problem("risk 要是字符串，当没写".into());
                None
            }
        };
        KotoneManifest {
            standard,
            games,
            events,
            watches,
            risk,
        }
    }

    fn event(&mut self, i: usize, item: &Value) -> Option<EventDecl> {
        let at = format!("events[{i}]");
        let Some(object) = item.as_object() else {
            self.problem(format!("{at} 要是对象，丢掉"));
            return None;
        };
        self.unknown_keys(&at, object, &["name", "label", "data"]);
        let name = self.string(object, &at, "name").unwrap_or_default();
        if !snake_case(&name) {
            self.problem(format!(
                "{at}：name 要是 snake_case（小写字母开头，小写字母、数字、下划线），丢掉"
            ));
            return None;
        }
        let label = self.string(object, &at, "label").unwrap_or_default();
        if label.is_empty() || label.chars().count() > MAX_LABEL {
            self.problem(format!(
                "{at}（{name}）：label 必填，最多 {MAX_LABEL} 字，丢掉"
            ));
            return None;
        }
        let mut data = BTreeMap::new();
        match object.get("data") {
            None | Some(Value::Null) => {}
            Some(Value::Object(fields)) => {
                for (field, about) in fields {
                    match about.as_str() {
                        Some(about) if camel_case(field) => {
                            data.insert(field.clone(), sanitize::clean(about, MAX_FIELD_ABOUT));
                        }
                        Some(_) => self.problem(format!(
                            "{at}.data.{field}：字段名要是 camelCase（小写字母开头，字母、数字），丢掉"
                        )),
                        None => self.problem(format!(
                            "{at}.data.{field} 要是一句说明（字符串），丢掉"
                        )),
                    }
                }
            }
            Some(_) => self.problem(format!("{at}.data 要是对象（字段 → 说明），当没写")),
        }
        Some(EventDecl { name, label, data })
    }

    fn watch(&mut self, i: usize, item: &Value) -> Option<Watch> {
        let at = format!("watches[{i}]");
        let Some(object) = item.as_object() else {
            self.problem(format!("{at} 要是对象，丢掉"));
            return None;
        };
        self.unknown_keys(
            &at,
            object,
            &[
                "event",
                "tool",
                "arguments",
                "everySeconds",
                "compare",
                "field",
                "to",
                "list",
                "id",
            ],
        );
        let event = self.string(object, &at, "event").unwrap_or_default();
        if event.is_empty() {
            self.problem(format!("{at}：event 必填，丢掉"));
            return None;
        }
        let tool = self.string(object, &at, "tool").unwrap_or_default();
        if tool.is_empty() || tool.chars().count() > 128 {
            self.problem(format!("{at}（{event}）：tool 必填（工具的原名），丢掉"));
            return None;
        }
        let arguments = match object.get("arguments") {
            None | Some(Value::Null) => Map::new(),
            Some(Value::Object(map)) => map.clone(),
            Some(_) => {
                self.problem(format!("{at}.arguments 要是对象，当 {{}}"));
                Map::new()
            }
        };
        let every_seconds = match object.get("everySeconds") {
            None | Some(Value::Null) => DEFAULT_EVERY,
            Some(value) => match value.as_u64() {
                Some(n) => n,
                None => {
                    self.problem(format!(
                        "{at}.everySeconds 要是整数（秒），当 {DEFAULT_EVERY}"
                    ));
                    DEFAULT_EVERY
                }
            },
        };
        let compare = match self.string(object, &at, "compare").as_deref() {
            Some("changed") => {
                let field = self.string(object, &at, "field").unwrap_or_default();
                if field.is_empty() {
                    self.problem(format!(
                        "{at}（{event}）：compare 是 changed 时要写 field，丢掉"
                    ));
                    return None;
                }
                let to = match object.get("to") {
                    None | Some(Value::Null) => Vec::new(),
                    Some(Value::String(s)) => vec![text(s)],
                    Some(Value::Array(items)) => {
                        let mut out = Vec::new();
                        for (j, v) in items.iter().enumerate() {
                            match v.as_str() {
                                Some(s) => out.push(text(s)),
                                None => self.problem(format!("{at}.to[{j}] 要是字符串，丢掉")),
                            }
                        }
                        out
                    }
                    Some(_) => {
                        self.problem(format!("{at}.to 要是字符串或字符串数组，当没写"));
                        Vec::new()
                    }
                };
                Compare::Changed { field, to }
            }
            Some("new") => Compare::New {
                list: self.string(object, &at, "list").filter(|s| !s.is_empty()),
                id: self
                    .string(object, &at, "id")
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "id".into()),
            },
            _ => {
                self.problem(format!(
                    "{at}（{event}）：compare 要是 \"changed\" 或 \"new\"，丢掉"
                ));
                return None;
            }
        };
        Some(Watch {
            event,
            tool,
            arguments,
            every_seconds,
            compare,
        })
    }
}

/// 把几层合成生效的清单（契约第 1 节）：
///
/// 1. 服务器给了（读得出来）就用它，**不用预设**；没给（或读不出来）才用预设；
/// 2. 玩家本地的只**加**事件和观察项：同名的事件、已经有观察项的事件不覆盖（列进问题）；
///    前两层都没有 `games` 时才用它的 `games`；它的 `risk` 不认；
/// 3. 观察项要发的事件没登记：丢掉、列进问题。
pub fn merge(
    server: Option<Result<Parsed, String>>,
    preset: Option<Parsed>,
    local: Option<Result<Parsed, String>>,
) -> Effective {
    let mut out = Effective::default();
    let mut base: Option<(Source, Parsed)> = None;
    match server {
        Some(Ok(parsed)) => base = Some((Source::Server, parsed)),
        Some(Err(text)) => out.problems.push(Note {
            from: Source::Server,
            text,
        }),
        None => {}
    }
    if base.is_none() {
        base = preset.map(|parsed| (Source::Preset, parsed));
    }
    // 每个观察项是哪一层的（问题要说清出在哪）
    let mut watches: Vec<(Source, Watch)> = Vec::new();
    if let Some((source, parsed)) = base {
        out.source = source;
        out.newer_standard = parsed.newer_standard;
        out.unknown.extend(
            parsed
                .unknown
                .into_iter()
                .map(|text| Note { from: source, text }),
        );
        out.problems.extend(
            parsed
                .problems
                .into_iter()
                .map(|text| Note { from: source, text }),
        );
        watches.extend(parsed.manifest.watches.iter().cloned().map(|w| (source, w)));
        out.manifest = KotoneManifest {
            watches: Vec::new(),
            ..parsed.manifest
        };
    }
    match local {
        Some(Ok(parsed)) => {
            let from = Source::Local;
            if out.source == Source::None {
                out.source = Source::Local;
            }
            out.unknown
                .extend(parsed.unknown.into_iter().map(|text| Note { from, text }));
            out.problems
                .extend(parsed.problems.into_iter().map(|text| Note { from, text }));
            let mut added = false;
            for event in parsed.manifest.events {
                if out.manifest.events.iter().any(|e| e.name == event.name) {
                    out.problems.push(Note {
                        from,
                        text: format!("事件 {} 已经有了，本地的这一条不覆盖", event.name),
                    });
                } else {
                    out.manifest.events.push(event);
                    added = true;
                }
            }
            for watch in parsed.manifest.watches {
                if watches.iter().any(|(_, w)| w.event == watch.event) {
                    out.problems.push(Note {
                        from,
                        text: format!("事件 {} 已经有观察项了，本地的这一个不覆盖", watch.event),
                    });
                } else {
                    watches.push((from, watch));
                    added = true;
                }
            }
            if out.manifest.games.is_empty() && !parsed.manifest.games.is_empty() {
                out.manifest.games = parsed.manifest.games;
                added = true;
            }
            if parsed.manifest.risk.is_some() {
                out.problems.push(Note {
                    from,
                    text: "risk 只认服务器给的和 Kotone 带的清单，本地写的不生效".into(),
                });
            }
            out.local = added;
        }
        Some(Err(text)) => out.problems.push(Note {
            from: Source::Local,
            text,
        }),
        None => {}
    }
    for (from, watch) in watches {
        if out.manifest.events.iter().any(|e| e.name == watch.event) {
            out.watch_from.insert(watch.event.clone(), from);
            out.manifest.watches.push(watch);
        } else {
            out.problems.push(Note {
                from,
                text: format!("观察项要发的事件 {} 没在 events 里登记，丢掉", watch.event),
            });
        }
    }
    out
}

/// 玩家本地的清单（`<dir>/manifest.json`）：没有这个文件 = None。
pub fn read_local(dir: &Path) -> Option<Result<Parsed, String>> {
    let path = dir.join(LOCAL_FILE);
    let meta = std::fs::metadata(&path).ok()?;
    if meta.len() > MAX_BYTES as u64 {
        return Some(Err(format!(
            "{LOCAL_FILE} 太大了（{} KiB，最多 64 KiB），没用上",
            meta.len().div_ceil(1024)
        )));
    }
    Some(match std::fs::read_to_string(&path) {
        Ok(text) => parse_text(&text),
        Err(e) => Err(format!("{LOCAL_FILE} 读不出来：{e}")),
    })
}

// ---- 预设 ----

/// 一份预设：`{ "id": "...", "match": {...}, "manifest": {...} }`。`id`、`match` 只在 Kotone 内部用（不属于标准）：
/// `id` 是导入这个服务器时建议的插件 id（自动化模板按它找插件），`match` 认服务器。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PresetFile {
    #[serde(default)]
    id: Option<String>,
    #[serde(rename = "match")]
    matcher: Matcher,
    manifest: Value,
}

/// 按什么认出服务器：规范化后的网址（[`normalize_url`]），或者服务器自报的名字（`serverInfo.name`）。
/// 两样写一样或都写都行，对上一样就算。
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Matcher {
    #[serde(default, deserialize_with = "one_or_many")]
    url: Vec<String>,
    #[serde(default, deserialize_with = "one_or_many")]
    name: Vec<String>,
}

fn one_or_many<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        One(String),
        Many(Vec<String>),
    }
    Ok(match OneOrMany::deserialize(d)? {
        OneOrMany::One(s) => vec![s],
        OneOrMany::Many(v) => v,
    })
}

/// Kotone 带的预设（文件名、内容，编进程序）。
const PRESETS: &[(&str, &str)] = &[(
    "zzz-onedragon.json",
    include_str!("../presets/zzz-onedragon.json"),
)];

/// 只给测试的预设：认 `kotone-mcp` 测试服务器的名字（`kotone-test-server`），正式版里没有。
#[cfg(test)]
const TEST_PRESETS: &[(&str, &str)] = &[(
    "kotone-test.json",
    include_str!("../presets/kotone-test.json"),
)];

fn preset_files() -> Vec<(&'static str, &'static str)> {
    #[allow(unused_mut)]
    let mut files = PRESETS.to_vec();
    #[cfg(test)]
    files.extend_from_slice(TEST_PRESETS);
    files
}

/// 认得出这个服务器的那份预设。
fn matching_preset(url: Option<&str>, name: Option<&str>) -> Option<(&'static str, PresetFile)> {
    let url = url.map(normalize_url);
    preset_files().into_iter().find_map(|(file, text)| {
        let preset: PresetFile = serde_json::from_str(text).ok()?;
        let by_url = url
            .as_deref()
            .is_some_and(|u| preset.matcher.url.iter().any(|m| normalize_url(m) == u));
        let by_name = name
            .filter(|n| !n.is_empty())
            .is_some_and(|n| preset.matcher.name.iter().any(|m| m == n));
        (by_url || by_name).then_some((file, preset))
    })
}

/// 认得出这个服务器的预设（文件名、读出来的清单）。`url`：HTTP 服务器的网址；`name`：它自报的名字。
pub fn preset_for(url: Option<&str>, name: Option<&str>) -> Option<(&'static str, Parsed)> {
    let (file, preset) = matching_preset(url, name)?;
    parse_value(&preset.manifest)
        .ok()
        .map(|parsed| (file, parsed))
}

/// 认得出这个服务器的预设建议的插件 id（导入时用它；预设没写 `id` 时用文件名去掉 `.json`）。
pub fn preset_id(url: Option<&str>, name: Option<&str>) -> Option<String> {
    let (file, preset) = matching_preset(url, name)?;
    Some(
        preset
            .id
            .unwrap_or_else(|| file.trim_end_matches(".json").to_string()),
    )
}

// ---- 玩家在插件页里加的观察项（写进本地的 manifest.json） ----

/// 读本地文件原样的 JSON（没有文件 = 空对象）：改的时候不丢玩家手写的别的东西。
fn local_raw(dir: &Path) -> Result<Map<String, Value>, String> {
    let path = dir.join(LOCAL_FILE);
    match std::fs::read_to_string(&path) {
        Ok(text) => match serde_json::from_str::<Value>(text.trim_start_matches('\u{FEFF}')) {
            Ok(Value::Object(map)) => Ok(map),
            Ok(_) | Err(_) => Err(format!(
                "{LOCAL_FILE} 写坏了（不是一个 JSON 对象），先改好或删掉它：{}",
                path.display()
            )),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Map::new()),
        Err(e) => Err(format!("{LOCAL_FILE} 读不出来：{e}")),
    }
}

fn write_local(dir: &Path, map: Map<String, Value>) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("建不了 {}：{e}", dir.display()))?;
    let text = serde_json::to_string_pretty(&Value::Object(map)).map_err(|e| e.to_string())?;
    if text.len() > MAX_BYTES {
        return Err(format!("{LOCAL_FILE} 会超过 64 KiB，加不下了"));
    }
    std::fs::write(dir.join(LOCAL_FILE), text + "\n")
        .map_err(|e| format!("{LOCAL_FILE} 写不进去：{e}"))
}

/// 玩家在插件页加一个观察项：事件和观察项一起写进本地的 `manifest.json`（只加，同一个格式）。
/// `event` / `watch` 是清单里的写法（`{"name", "label", "data"}`、`{"event", "tool", ...}`）；先按清单的规矩读一遍，
/// 有任何问题就不写（界面上说原因）。`taken`：已经有的事件名（服务器的、预设的、本地的）。
pub fn add_local(
    dir: &Path,
    event: Value,
    watch: Value,
    taken: &[String],
) -> Result<(EventDecl, Watch), String> {
    let parsed =
        parse_value(&serde_json::json!({"events": [event.clone()], "watches": [watch.clone()]}))?;
    if let Some(problem) = parsed.problems.first() {
        return Err(problem.clone());
    }
    if let Some(field) = parsed.unknown.first() {
        return Err(format!("不认识的字段 {field}"));
    }
    let (Some(decl), Some(w)) = (
        parsed.manifest.events.first(),
        parsed.manifest.watches.first(),
    ) else {
        return Err("事件和观察项都要写".into());
    };
    if w.event != decl.name {
        return Err("观察项要发的就是这个事件".into());
    }
    if taken.contains(&decl.name) {
        return Err(format!("已经有叫 {} 的事件了，换一个名字", decl.name));
    }
    let mut map = local_raw(dir)?;
    for (key, item) in [("events", event), ("watches", watch)] {
        let list = map.entry(key).or_insert_with(|| Value::Array(Vec::new()));
        match list {
            Value::Array(items) => items.push(item),
            _ => return Err(format!("{LOCAL_FILE} 里的 {key} 不是数组，先改好它")),
        }
    }
    write_local(dir, map)?;
    Ok((decl.clone(), w.clone()))
}

/// 删掉玩家自己加的一个事件和它的观察项（只动本地的 `manifest.json`）。返回删掉了没有。
pub fn remove_local(dir: &Path, name: &str) -> Result<bool, String> {
    let mut map = local_raw(dir)?;
    let mut removed = false;
    for (key, field) in [("events", "name"), ("watches", "event")] {
        if let Some(Value::Array(items)) = map.get_mut(key) {
            let before = items.len();
            items.retain(|item| item.get(field).and_then(Value::as_str) != Some(name));
            removed |= items.len() != before;
        }
    }
    if removed {
        write_local(dir, map)?;
    }
    Ok(removed)
}

/// 网址规范化（预设按它认）：协议和主机小写、`localhost` 当 `127.0.0.1`、去掉默认端口和末尾的 `/`。
pub fn normalize_url(url: &str) -> String {
    let url = url.trim();
    let Some((scheme, rest)) = url.split_once("://") else {
        return url.to_string();
    };
    let scheme = scheme.to_ascii_lowercase();
    let cut = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(cut);
    let mut authority = authority.to_ascii_lowercase();
    if let Some(port) = match scheme.as_str() {
        "http" => Some(":80"),
        "https" => Some(":443"),
        _ => None,
    } {
        if let Some(stripped) = authority.strip_suffix(port) {
            authority = stripped.to_string();
        }
    }
    let (host, port) = split_host(&authority);
    let host = if host == "localhost" {
        "127.0.0.1"
    } else {
        host
    };
    let (path, query) = match tail.find(['?', '#']) {
        Some(i) => tail.split_at(i),
        None => (tail, ""),
    };
    let path = path.trim_end_matches('/');
    format!("{scheme}://{host}{port}{path}{query}")
}

/// `主机[:端口]` 拆开（IPv6 的 `[::1]:8080` 也认）。
fn split_host(authority: &str) -> (&str, &str) {
    let authority = authority.rsplit('@').next().unwrap_or(authority);
    if authority.starts_with('[') {
        if let Some(end) = authority.find(']') {
            return authority.split_at(end + 1);
        }
    }
    match authority.rfind(':') {
        Some(i) => authority.split_at(i),
        None => (authority, ""),
    }
}

/// 网址的主机（小写；`localhost` 照原样）。
pub fn url_host(url: &str) -> Option<String> {
    let (_, rest) = url.trim().split_once("://")?;
    let cut = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (host, _) = split_host(&rest[..cut]);
    Some(host.to_ascii_lowercase())
}

/// 本机的地址：`127.0.0.1`（整个 127.0.0.0/8）、`localhost`、`[::1]`。
pub fn local_host(host: &str) -> bool {
    host == "localhost" || host == "[::1]" || host.starts_with("127.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse(value: Value) -> Parsed {
        parse_value(&value).unwrap()
    }

    #[test]
    fn missing_fields_take_their_defaults() {
        let p = parse(json!({}));
        assert_eq!(p.manifest, KotoneManifest::default());
        assert_eq!(p.manifest.standard, "0.1");
        assert!(p.unknown.is_empty() && p.problems.is_empty() && !p.newer_standard);
        let w = parse(json!({
            "events": [{"name": "done", "label": "跑完了"}],
            "watches": [{"event": "done", "tool": "status", "compare": "new"}]
        }));
        let watch = &w.manifest.watches[0];
        assert_eq!(watch.every_seconds, DEFAULT_EVERY);
        assert!(watch.arguments.is_empty());
        assert_eq!(
            watch.compare,
            Compare::New {
                list: None,
                id: "id".into()
            }
        );
        assert!(w.manifest.events[0].data.is_empty());
        assert!(w.problems.is_empty(), "{:?}", w.problems);
    }

    /// 契约第 2 节的例子原样读得出来、没有提示；序列化回去字段名照旧（camelCase，compare 和它的字段平铺）。
    #[test]
    fn the_contract_example_reads_cleanly() {
        let example = json!({
          "standard": "0.1",
          "games": ["zzz"],
          "events": [{"name": "run_finished", "label": "一条龙跑完了",
                      "data": {"status": "跑完时的状态", "message": "它说的原话（可能为空）"}}],
          "watches": [
            {"event": "run_finished", "tool": "get_run_status", "arguments": {}, "everySeconds": 30,
             "compare": "changed", "field": "status", "to": ["finished", "succeeded"]},
            {"event": "achievement_unlocked", "tool": "recent_achievements", "arguments": {"limit": 20},
             "everySeconds": 120, "compare": "new", "list": "achievements", "id": "id"}
          ],
          "risk": "一段给玩家看的风险说明（可空）"
        });
        let p = parse(example);
        assert!(p.unknown.is_empty(), "{:?}", p.unknown);
        assert!(p.problems.is_empty(), "{:?}", p.problems);
        assert_eq!(p.manifest.games, ["zzz"]);
        assert_eq!(p.manifest.watches.len(), 2, "合并前不查事件登记没有");
        let out = serde_json::to_value(&p.manifest.watches[0]).unwrap();
        assert_eq!(
            out,
            json!({"event": "run_finished", "tool": "get_run_status", "arguments": {}, "everySeconds": 30,
                   "compare": "changed", "field": "status", "to": ["finished", "succeeded"]})
        );
        // 只登记了 run_finished：另一个观察项合并时丢掉
        let effective = merge(Some(Ok(p)), None, None);
        assert_eq!(effective.manifest.watches.len(), 1);
        assert!(effective.problems[0].text.contains("achievement_unlocked"));
    }

    #[test]
    fn unknown_fields_are_ignored_and_listed_but_experimental_and_meta_are_not() {
        let p = parse(json!({
            "colour": "red",
            "experimentalOverlay": true,
            "_meta": {"com.l1vein.kotone/hint": 1, "other.vendor/x": 2},
            "$schema": "https://example.com/schema.json",
            "events": [{"name": "done", "label": "跑完了", "icon": "x", "experimentalSound": "a", "_meta": {}}],
            "watches": [{"event": "done", "tool": "status", "compare": "new", "retry": 3}]
        }));
        assert_eq!(p.unknown, ["colour", "events[0].icon", "watches[0].retry"]);
        assert!(p.problems.is_empty(), "{:?}", p.problems);
        assert_eq!(p.manifest.events.len(), 1);
        assert_eq!(p.manifest.watches.len(), 1, "不认识的字段不影响别的");
    }

    /// 内置自动化（`automations`）只给内置插件的 `plugin.toml`（ADR 050 第 6 条的实现情况）：第三方清单里写了也是不认识的
    /// 字段——忽略、列出来给作者看，不报错、不影响别的（别人不能借清单默认开一个会花钱的 `ask()`）。
    #[test]
    fn automations_are_not_for_third_party_plugins() {
        let p = parse(json!({
            "events": [{"name": "done", "label": "跑完了"}],
            "automations": [{"id": "builtin-x-cheer", "name": "跑完了叫她", "source": "ask('x')",
                             "on": {"event": ["x.done"]}}]
        }));
        assert_eq!(p.unknown, ["automations"]);
        assert!(p.problems.is_empty(), "{:?}", p.problems);
        assert_eq!(p.manifest.events.len(), 1);
        let out = serde_json::to_value(&p.manifest).unwrap();
        assert!(out.get("automations").is_none());
    }

    #[test]
    fn wrong_types_count_as_missing_and_bad_entries_are_dropped() {
        let p = parse(json!({
            "standard": 1,
            "games": "lol",
            "events": [
                {"name": "Bad-Name", "label": "x"},
                {"name": "ok", "label": "好", "data": {"goodField": "说明", "bad_field": "x", "n": 3}},
                {"name": "ok", "label": "重名"},
                {"name": "no_label"},
                {"name": "long", "label": "长".repeat(65)},
                "not an object"
            ],
            "watches": [
                {"event": "ok", "tool": "t", "compare": "sometimes"},
                {"event": "ok", "tool": "t", "compare": "changed"},
                {"event": "ok", "tool": "t", "compare": "changed", "field": "a.b", "to": "done",
                 "arguments": [1], "everySeconds": "30"},
                {"event": "ok", "tool": "t2", "compare": "new"},
                {"tool": "t", "compare": "new"}
            ],
            "risk": ["x"]
        }));
        let m = &p.manifest;
        assert_eq!(m.standard, "0.1");
        assert!(m.games.is_empty() && m.risk.is_none());
        assert_eq!(m.events.len(), 1);
        assert_eq!(
            m.events[0].data.keys().collect::<Vec<_>>(),
            ["goodField"],
            "不合格的字段丢掉，别的留着"
        );
        assert_eq!(m.watches.len(), 1);
        let w = &m.watches[0];
        assert_eq!(w.every_seconds, DEFAULT_EVERY);
        assert!(w.arguments.is_empty());
        assert_eq!(
            w.compare,
            Compare::Changed {
                field: "a.b".into(),
                to: vec!["done".into()]
            },
            "to 写一个字符串也行"
        );
        let all = p.problems.join("\n");
        for expected in [
            "standard 要是字符串",
            "games 要是数组",
            "events[0]",
            "bad_field",
            "data.n",
            "重名",
            "events[3]",
            "events[4]",
            "events[5]",
            "\"changed\" 或 \"new\"",
            "要写 field",
            "arguments 要是对象",
            "everySeconds 要是整数",
            "已经有一个观察项",
            "event 必填",
            "risk 要是字符串",
        ] {
            assert!(all.contains(expected), "少了“{expected}”：\n{all}");
        }
        assert!(p.unknown.is_empty());
    }

    #[test]
    fn whole_manifest_problems_mean_no_manifest() {
        assert!(parse_text("[1, 2]").unwrap_err().contains("JSON 对象"));
        assert!(parse_text("{not json").unwrap_err().contains("合法的 JSON"));
        let big = format!("{{\"risk\": \"{}\"}}", "x".repeat(MAX_BYTES));
        assert!(parse_text(&big).unwrap_err().contains("64 KiB"));
        // 带 BOM 的文件（记事本存的）照样读
        assert!(parse_text("\u{FEFF}{\"games\": [\"lol\"]}").is_ok());
    }

    #[test]
    fn a_newer_standard_is_noticed_and_still_read() {
        let p = parse(json!({"standard": "0.2", "games": ["lol"], "newThing": {}}));
        assert!(p.newer_standard);
        assert_eq!(p.manifest.standard, "0.2");
        assert_eq!(p.manifest.games, ["lol"], "认识的字段照常读");
        assert_eq!(p.unknown, ["newThing"]);
        assert!(!parse(json!({"standard": "0.1.3"})).newer_standard);
        assert!(parse(json!({"standard": "1.0"})).newer_standard);
        let bad = parse(json!({"standard": "latest"}));
        assert_eq!(bad.manifest.standard, "0.1");
        assert!(bad.problems[0].contains("版本号"));
    }

    #[test]
    fn strings_from_others_lose_invisible_characters() {
        let p = parse(json!({
            "games": ["lol\u{200B}", "lol"],
            "events": [{"name": "done", "label": "跑完\u{E0041}了", "data": {"a": "说\u{200B}明"}}],
            "risk": "  会改\u{200B}游戏文件  "
        }));
        assert_eq!(p.manifest.games, ["lol"], "剥完一样的只留一个");
        assert_eq!(p.manifest.events[0].label, "跑完了");
        assert_eq!(p.manifest.events[0].data["a"], "说明");
        assert_eq!(p.manifest.risk.as_deref(), Some("会改游戏文件"));
        let long = parse(json!({"risk": "长".repeat(MAX_RISK + 10)}));
        assert!(long.manifest.risk.unwrap().contains("截掉"));
    }

    fn layer(value: Value) -> Parsed {
        parse(value)
    }

    /// 合成规则：服务器给了就不用预设；服务器的读不出来才用预设；玩家文件只加不覆盖。
    #[test]
    fn presets_only_fill_in_when_the_server_gives_nothing() {
        let server =
            layer(json!({"games": ["lol"], "events": [{"name": "a", "label": "服务器的"}]}));
        let preset = layer(json!({"games": ["zzz"], "risk": "预设的风险",
                                   "events": [{"name": "b", "label": "预设的"}]}));
        let e = merge(Some(Ok(server)), Some(preset.clone()), None);
        assert_eq!(e.source, Source::Server);
        assert_eq!(e.manifest.games, ["lol"]);
        assert_eq!(e.manifest.events.len(), 1);
        assert!(e.manifest.risk.is_none(), "预设一点都不掺进来");

        let e = merge(None, Some(preset.clone()), None);
        assert_eq!(e.source, Source::Preset);
        assert_eq!(e.manifest.risk.as_deref(), Some("预设的风险"));

        let e = merge(
            Some(Err("清单要是一个 JSON 对象，当没给".into())),
            Some(preset),
            None,
        );
        assert_eq!(e.source, Source::Preset, "服务器给的读不出来 = 没给");
        assert_eq!(e.problems[0].from, Source::Server);

        let e = merge(None, None, None);
        assert_eq!(e.source, Source::None);
        assert_eq!(e.manifest, KotoneManifest::default());
    }

    #[test]
    fn the_local_file_only_adds() {
        let server = layer(json!({
            "standard": "0.1",
            "events": [{"name": "done", "label": "服务器的"}],
            "watches": [{"event": "done", "tool": "status", "compare": "changed", "field": "s"}]
        }));
        let local = layer(json!({
            "standard": "0.9",
            "games": ["lol"],
            "risk": "本地写的风险",
            "events": [{"name": "done", "label": "本地的同名"}, {"name": "mine", "label": "我加的"}],
            "watches": [
                {"event": "done", "tool": "other", "compare": "new"},
                {"event": "mine", "tool": "list", "compare": "new", "list": "items"},
                {"event": "ghost", "tool": "x", "compare": "new"}
            ],
            "oops": 1
        }));
        let e = merge(Some(Ok(server)), None, Some(Ok(local)));
        assert_eq!(e.source, Source::Server);
        assert!(e.local);
        assert_eq!(e.manifest.standard, "0.1", "standard 只认服务器 / 预设");
        assert!(!e.newer_standard);
        assert!(e.manifest.risk.is_none(), "risk 只认服务器 / 预设");
        assert_eq!(e.manifest.games, ["lol"], "前面都没有 games 时用本地的");
        let labels: Vec<&str> = e.manifest.events.iter().map(|e| e.label.as_str()).collect();
        assert_eq!(labels, ["服务器的", "我加的"], "同名的不覆盖");
        let tools: Vec<&str> = e.manifest.watches.iter().map(|w| w.tool.as_str()).collect();
        assert_eq!(tools, ["status", "list"]);
        assert_eq!(
            e.unknown,
            [Note {
                from: Source::Local,
                text: "oops".into()
            }]
        );
        let problems: Vec<&str> = e.problems.iter().map(|n| n.text.as_str()).collect();
        assert!(
            problems.iter().any(|p| p.contains("done 已经有了")),
            "{problems:?}"
        );
        assert!(problems.iter().any(|p| p.contains("done 已经有观察项")));
        assert!(problems.iter().any(|p| p.contains("risk 只认")));
        assert!(problems.iter().any(|p| p.contains("ghost")));
        assert!(e.problems.iter().all(|n| n.from == Source::Local));

        // 服务器有 games：本地的不用
        let with_games = layer(json!({"games": ["zzz"]}));
        let e = merge(
            Some(Ok(with_games)),
            None,
            Some(Ok(layer(json!({"games": ["lol"]})))),
        );
        assert_eq!(e.manifest.games, ["zzz"]);
        assert!(!e.local);

        // 只有本地的
        let e = merge(
            None,
            None,
            Some(Ok(layer(
                json!({"events": [{"name": "x", "label": "我的"}]}),
            ))),
        );
        assert_eq!(e.source, Source::Local);
        assert_eq!(e.manifest.events.len(), 1);
        // 本地文件坏了：说一句，别的照旧
        let e = merge(None, None, Some(Err("manifest.json 读不出来".into())));
        assert_eq!(e.source, Source::None);
        assert_eq!(e.problems[0].from, Source::Local);
    }

    #[test]
    fn local_files_are_read_from_the_plugin_directory() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read_local(dir.path()).is_none(), "没有文件 = 没有");
        std::fs::write(dir.path().join(LOCAL_FILE), "{\"games\": [\"lol\"]}").unwrap();
        assert_eq!(
            read_local(dir.path()).unwrap().unwrap().manifest.games,
            ["lol"]
        );
        std::fs::write(dir.path().join(LOCAL_FILE), "x".repeat(MAX_BYTES + 1)).unwrap();
        assert!(read_local(dir.path())
            .unwrap()
            .unwrap_err()
            .contains("64 KiB"));
    }

    #[test]
    fn presets_are_well_formed_and_match_by_url_or_name() {
        for (file, text) in preset_files() {
            let preset: PresetFile =
                serde_json::from_str(text).unwrap_or_else(|e| panic!("{file}：{e}"));
            assert!(
                !(preset.matcher.url.is_empty() && preset.matcher.name.is_empty()),
                "{file} 认不出任何服务器"
            );
            let parsed = parse_value(&preset.manifest).unwrap();
            assert!(
                parsed.unknown.is_empty() && parsed.problems.is_empty(),
                "{file}：{:?} {:?}",
                parsed.unknown,
                parsed.problems
            );
            let effective = merge(None, Some(parsed), None);
            assert!(
                effective.problems.is_empty(),
                "{file}：{:?}",
                effective.problems
            );
        }
        let (file, parsed) = preset_for(None, Some("kotone-test-server")).unwrap();
        assert_eq!(file, "kotone-test.json");
        assert_eq!(parsed.manifest.games, ["generic"]);
        assert!(preset_for(None, Some("someone-else")).is_none());
        assert!(preset_for(Some("http://127.0.0.1:9/mcp"), None).is_none());
    }

    #[test]
    fn urls_are_normalized_for_matching() {
        for (a, b) in [
            ("http://127.0.0.1:23001/mcp", "HTTP://LOCALHOST:23001/mcp/"),
            ("http://example.com/mcp", "http://Example.com:80/mcp"),
            ("https://example.com", "https://example.com:443/"),
            ("http://[::1]:8080/mcp", "http://[::1]:8080/mcp/"),
        ] {
            assert_eq!(normalize_url(a), normalize_url(b), "{a} / {b}");
        }
        assert_ne!(
            normalize_url("http://127.0.0.1:23001/mcp"),
            normalize_url("http://127.0.0.1:23002/mcp")
        );
        assert_eq!(normalize_url("http://x.com/a?b=1"), "http://x.com/a?b=1");
        assert_eq!(
            url_host("http://user@LocalHost:3000/mcp").as_deref(),
            Some("localhost")
        );
        assert!(local_host("127.0.0.1") && local_host("localhost") && local_host("[::1]"));
        assert!(!local_host("192.168.1.2") && !local_host("example.com"));
    }

    #[test]
    fn intervals_have_floors() {
        let watch = |every| Watch {
            event: "e".into(),
            tool: "t".into(),
            arguments: Map::new(),
            every_seconds: every,
            compare: Compare::New {
                list: None,
                id: "id".into(),
            },
        };
        assert_eq!(watch(1).interval(true, true), Duration::from_secs(10));
        assert_eq!(watch(30).interval(true, true), Duration::from_secs(30));
        assert_eq!(
            watch(30).interval(false, true),
            Duration::from_secs(60),
            "别的网址"
        );
        assert_eq!(
            watch(30).interval(true, false),
            Duration::from_secs(60),
            "games 为空一直轮询"
        );
        assert_eq!(watch(300).interval(false, false), Duration::from_secs(300));
    }

    #[test]
    fn paths_reach_into_objects_and_arrays() {
        let v = json!({"run": {"status": "done"}, "items": [{"state": "a"}, {"state": "b"}]});
        assert_eq!(lookup(&v, "run.status"), Some(&json!("done")));
        assert_eq!(lookup(&v, "items.1.state"), Some(&json!("b")));
        assert_eq!(lookup(&v, ""), Some(&v));
        assert_eq!(lookup(&v, "items.x"), None);
        assert_eq!(lookup(&v, "run.status.more"), None);
        assert_eq!(event_type("zzz", "run_finished"), "zzz.run_finished");
    }
}
