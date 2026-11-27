//! 观察项的运行（插件标准 0.1 第 3.4 节，ADR 047 D5；照 Home Assistant 的 REST 传感器、Zapier / n8n 的轮询触发）：
//! 隔一阵子调一次只读工具，比较结果，变了就经宿主的 `game_event` 发一件事（`<插件 id>.<名字>`）。
//!
//! - **比较**是纯函数（[`changed`]、[`fresh`]）：第一次只记底、不发；`changed` 取一个字段，变了（且是 `to` 之一）才发；
//!   `new` 按 id 去重，记底的 id 最多 [`MAX_SEEN_IDS`] 个（列表也只看前这么多个，免得旧的被挤掉又算成新的）。
//!   事件内容按不可信处理：剥掉看不见的字符，单个字符串截到 [`MAX_EVENT_TEXT`] 字（[`clean`]）。
//! - **什么时候查**（[`gate`]）：玩家打开过这个观察项、确认的就是现在这份内容（[`digest`]：工具、参数、比较，加上插件配置的
//!   指纹，内容一变就要重新确认）、插件开着、服务器信任过、服务器上有这个工具、工具标了只读或者玩家在确认里明确同意过；
//!   `games` 非空时其中一个游戏开着（退出后再查 [`GAME_GRACE`]：很多工具跑完会关游戏，[`game_now`]）。
//! - **定时**（[`Timing`]）：间隔不低于下限（本机 10 秒；别的网址、`games` 为空时 60 秒）；失败退避（间隔 ×2，最多 10 分钟），
//!   连续失败 [`MAX_FAILURES`] 次停下，玩家点“重试”（[`Watcher::retry`]）或关了再开才重新开始。
//!   工具没给 `structuredContent` 的直接停下（只有 Kotone 预设里的观察项可以读文字里的 JSON，[`result_value`]）。
//! - 同一轮里几个观察项调同一个工具、同样的参数（一条龙的“跑完了”“出错了”都看 `get_run_status`）只调一次。
//! - 玩家的决定记在插件设置 `plugins.<id>.settings.watches.<事件名>`（[`Choice`]，和信任一样存在设置里）；记底、失败次数
//!   只在内存里（Kotone 重开以后第一次查又是记底）。
//! - 事件**只进自动化**：宿主（桌面端 `remote_plugins.rs`）收下以后交给游戏中心的“当……的时候”，不进她的上下文、不推送
//!   （ADR 047 D5、ADR 043 D11）。

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use kotone_mcp::{Tool, ToolCallResult};
use kotone_plugin_api::Host;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use crate::manifest::{
    self, Compare, Effective, Source, Watch, GAME_GRACE, LOCAL_FLOOR, MAX_BACKOFF, MAX_EVENT_TEXT,
    MAX_FAILURES, MAX_SEEN_IDS, REMOTE_FLOOR,
};
use crate::sanitize;
use crate::server::Server;
use crate::tools;

/// 玩家的决定记在插件设置的哪个键（`plugins.<id>.settings.watches.<事件名>`）。
pub const SETTINGS_KEY: &str = "watches";
/// 第三方事件的来源（游戏中心事件的 `source` 是 `third-party/<插件 id>`）。
pub const EVENT_SOURCE_PREFIX: &str = "third-party/";
/// 什么都不用查时多久醒一次（设置变了、点了重试会马上叫醒）。
const SLEEP: Duration = Duration::from_secs(3600);

/// 现在开着哪些游戏（游戏配置 id；宿主给，桌面端是游戏中心的状态）。
pub type GamesProbe = Arc<dyn Fn() -> Vec<String> + Send + Sync>;

/// 玩家对一个观察项的决定（插件设置里 `watches.<事件名>`）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Choice {
    /// 开着。
    pub on: bool,
    /// 确认时这个观察项的内容（[`digest`]）：和现在的不一样就要重新确认。
    pub confirmed: String,
    /// 确认时工具没标只读，玩家明确同意了照样定时调它。
    pub allow_unmarked: bool,
}

/// 插件设置里的全部决定（写错的那一项当没有）。
pub fn choices(settings: &Value) -> BTreeMap<String, Choice> {
    settings
        .get(SETTINGS_KEY)
        .and_then(Value::as_object)
        .map(|map| {
            map.iter()
                .filter_map(|(event, v)| {
                    serde_json::from_value::<Choice>(v.clone())
                        .ok()
                        .map(|c| (event.clone(), c))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// 键按字母排的 JSON（指纹要和键的顺序无关）。
fn canonical(value: &Value, out: &mut String) {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push('{');
            for (i, key) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&Value::String((*key).clone()).to_string());
                out.push(':');
                canonical(&map[*key], out);
            }
            out.push('}');
        }
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                canonical(item, out);
            }
            out.push(']');
        }
        other => out.push_str(&other.to_string()),
    }
}

fn canonical_text(value: &Value) -> String {
    let mut out = String::new();
    canonical(value, &mut out);
    out
}

/// 玩家确认的是什么：工具、参数、比较方式（字段、`to`、列表、id），加上插件配置的指纹（配置变了也要重新确认）。
/// 间隔不在里面（多久查一次不改变查的是什么）。
pub fn digest(fingerprint: &str, watch: &Watch) -> String {
    let value = json!({
        "fingerprint": fingerprint,
        "tool": watch.tool,
        "arguments": watch.arguments,
        "compare": watch.compare,
    });
    Sha256::digest(canonical_text(&value).as_bytes())
        .iter()
        .take(12)
        .map(|b| format!("{b:02x}"))
        .collect()
}

// ---------- 什么时候查 ----------

/// 服务器上这个工具的情况（看连上时记下的工具清单）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolFact {
    /// 还不知道它有哪些工具（没连上过）。
    Unknown,
    Missing,
    /// 标了只读（`readOnlyHint: true`）。
    ReadOnly,
    /// 没标只读。
    Unmarked,
}

pub fn tool_fact(tools: &[Tool], name: &str) -> ToolFact {
    if tools.is_empty() {
        return ToolFact::Unknown;
    }
    match tools.iter().find(|t| t.name == name) {
        None => ToolFact::Missing,
        Some(t) if tools::read_only(t) => ToolFact::ReadOnly,
        Some(_) => ToolFact::Unmarked,
    }
}

/// 一个观察项现在的样子（插件页“事件”一块）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum State {
    /// 没开（没打开过，或者关了）。
    Off,
    /// 要玩家（重新）确认。
    Confirm { reason: String },
    /// 开着，但现在查不了（插件关着、没信任、服务器上没有这个工具）。
    Blocked { reason: String },
    /// 开着，等游戏开。
    Waiting,
    /// 在查。
    Polling,
    /// 连续失败停下了（或者工具给不了结构化结果）。
    Stopped { reason: String },
}

/// 判断要的事实。
#[derive(Debug, Clone)]
pub struct Facts<'a> {
    pub choice: Option<&'a Choice>,
    pub digest: &'a str,
    pub enabled: bool,
    pub trusted: bool,
    pub tool_name: &'a str,
    pub tool: ToolFact,
    pub stopped: Option<&'a str>,
    /// `games` 为空，或者其中一个游戏开着 / 刚退出（[`game_now`]）。
    pub game_ok: bool,
}

/// 这个观察项现在查不查（契约第 3 节“什么时候轮询”）。
pub fn gate(f: &Facts) -> State {
    let Some(choice) = f.choice.filter(|c| c.on) else {
        return State::Off;
    };
    if choice.confirmed != f.digest {
        let reason = if choice.confirmed.is_empty() {
            "还没确认过".to_string()
        } else {
            "观察项的内容变了（调的工具、参数、比较方式，或者插件的配置），要重新确认".to_string()
        };
        return State::Confirm { reason };
    }
    if !f.enabled {
        return State::Blocked {
            reason: "插件关着".into(),
        };
    }
    if !f.trusted {
        return State::Blocked {
            reason: "还没信任这个插件（在它的设置里信任以后才查）".into(),
        };
    }
    match f.tool {
        ToolFact::Missing => {
            return State::Blocked {
                reason: format!("服务器上没有工具 {}", f.tool_name),
            }
        }
        ToolFact::Unmarked if !choice.allow_unmarked => {
            return State::Confirm {
                reason: format!("工具 {} 没标只读，要你在确认里明确同意", f.tool_name),
            }
        }
        _ => {}
    }
    if let Some(reason) = f.stopped {
        return State::Stopped {
            reason: reason.to_string(),
        };
    }
    if f.game_ok {
        State::Polling
    } else {
        State::Waiting
    }
}

/// 现在能不能查、事件挂在哪个游戏上：`games` 为空 → 能查，游戏是空的；其中一个游戏开着 → 它；
/// 刚退出（`grace` 之内）→ 它；都不是 → None（等）。`last`：上一次看到开着的游戏和时间。
pub fn game_now(
    games: &[String],
    running: &[String],
    last: Option<&(String, Instant)>,
    now: Instant,
    grace: Duration,
) -> Option<String> {
    if games.is_empty() {
        return Some(String::new());
    }
    if let Some(game) = games.iter().find(|g| running.contains(g)) {
        return Some(game.clone());
    }
    last.filter(|(game, at)| games.contains(game) && now.saturating_duration_since(*at) <= grace)
        .map(|(game, _)| game.clone())
}

// ---------- 定时 ----------

/// 定时用的数（默认是标准里的；测试把“一秒”调小）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timing {
    /// 清单里的“一秒”。
    pub second: Duration,
    pub local_floor: Duration,
    pub remote_floor: Duration,
    pub grace: Duration,
    pub max_backoff: Duration,
    /// 等游戏开时多久看一眼。
    pub idle: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            second: Duration::from_secs(1),
            local_floor: Duration::from_secs(LOCAL_FLOOR),
            remote_floor: Duration::from_secs(REMOTE_FLOOR),
            grace: GAME_GRACE,
            max_backoff: MAX_BACKOFF,
            idle: Duration::from_secs(2),
        }
    }
}

impl Timing {
    /// 真用的间隔：不低于下限（本机且 `games` 非空 [`LOCAL_FLOOR`]，否则 [`REMOTE_FLOOR`]）。
    pub fn interval(&self, watch: &Watch, local: bool, has_games: bool) -> Duration {
        let floor = if local && has_games {
            self.local_floor
        } else {
            self.remote_floor
        };
        let every = u32::try_from(watch.every_seconds).unwrap_or(u32::MAX);
        self.second.saturating_mul(every).max(floor)
    }

    /// 连续失败 `failures` 次以后多久再试：间隔 ×2^次数，最多 `max_backoff`（间隔本来就更长的照间隔）。
    pub fn backoff(&self, interval: Duration, failures: u32) -> Duration {
        let wait = interval.saturating_mul(1u32 << failures.min(16));
        wait.min(self.max_backoff.max(interval))
    }
}

// ---------- 比较（纯函数） ----------

/// 这次没查成：`Retry` 记一次失败、退避；`Stop` 直接停下（工具给不了结构化结果，再查也一样）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    Retry(String),
    Stop(String),
}

fn first_text(result: &ToolCallResult) -> Option<&str> {
    result
        .content
        .iter()
        .find(|b| b.get("type").and_then(Value::as_str) == Some("text"))
        .and_then(|b| b.get("text"))
        .and_then(Value::as_str)
}

/// 工具结果里拿来比较的那份：`structuredContent`；没有时只有预设的观察项（`text_ok`）可以把第一段文字当 JSON 读。
pub fn result_value(result: &ToolCallResult, text_ok: bool) -> Result<Value, Failure> {
    if result.is_error {
        let said = first_text(result)
            .map(|t| tools::short_text(t, 200))
            .unwrap_or_default();
        return Err(Failure::Retry(if said.is_empty() {
            "工具说出错了，没说原因".into()
        } else {
            format!("工具说出错了：{said}")
        }));
    }
    if let Some(value) = &result.structured_content {
        return Ok(value.clone());
    }
    if text_ok {
        return first_text(result)
            .and_then(|t| serde_json::from_str::<Value>(t.trim()).ok())
            .ok_or_else(|| Failure::Retry("结果里没有 structuredContent，文字也不是 JSON".into()));
    }
    Err(Failure::Stop(
        "这个工具没有给结构化结果（structuredContent），观察项用不了：要插件作者让它返回 structuredContent".into(),
    ))
}

/// `last_status` → `lastStatus`（结果里的字段名按 camelCase 对上事件 `data` 里登记的字段）。
pub fn camel(key: &str) -> String {
    let mut out = String::new();
    let mut upper = false;
    for c in key.chars() {
        if c == '_' || c == '-' {
            upper = !out.is_empty();
        } else if upper {
            out.extend(c.to_uppercase());
            upper = false;
        } else {
            out.push(c);
        }
    }
    out
}

/// 结果里的这个键对上事件 `data` 里登记的哪个字段（一样，或者 camelCase 以后一样）。
fn registered(fields: &BTreeMap<String, String>, key: &str) -> Option<String> {
    if fields.contains_key(key) {
        return Some(key.to_string());
    }
    let camel = camel(key);
    fields.contains_key(&camel).then_some(camel)
}

fn truncate(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

/// 事件内容按不可信处理：字符串剥掉看不见的字符、截到 [`MAX_EVENT_TEXT`] 字；嵌套最多 4 层、每层最多 50 项。
pub fn clean(value: &Value) -> Value {
    clean_at(value, 0)
}

fn clean_at(value: &Value, depth: usize) -> Value {
    match value {
        Value::String(s) => Value::String(truncate(&sanitize::strip_invisible(s), MAX_EVENT_TEXT)),
        Value::Array(items) if depth < 4 => Value::Array(
            items
                .iter()
                .take(50)
                .map(|v| clean_at(v, depth + 1))
                .collect(),
        ),
        Value::Object(map) if depth < 4 => Value::Object(
            map.iter()
                .take(50)
                .map(|(k, v)| {
                    (
                        truncate(&sanitize::strip_invisible(k), 64),
                        clean_at(v, depth + 1),
                    )
                })
                .collect(),
        ),
        Value::Array(_) | Value::Object(_) => Value::Null,
        other => other.clone(),
    }
}

/// `to` 按文字比：字符串是它本身，别的是 JSON 写法（`true`、`3`）。
fn as_text(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn scalar(value: &Value) -> bool {
    matches!(value, Value::String(_) | Value::Number(_) | Value::Bool(_))
}

/// **changed**：取 `field`；第一次只记底；变了且是 `to` 之一（`to` 空 = 不限）才发。
/// `data` = `{from, to}`，再加结果顶层里、事件 `data` 里登记过的标量字段（`from` / `to` 以比较出来的为准）。
/// 结果里没有这个字段算没查成（底不动）。
pub fn changed(
    last: &mut Option<Value>,
    result: &Value,
    field: &str,
    to: &[String],
    fields: &BTreeMap<String, String>,
) -> Result<Option<Map<String, Value>>, String> {
    let now = manifest::lookup(result, field)
        .ok_or_else(|| format!("结果里没有字段 {field}"))?
        .clone();
    let Some(before) = last.replace(now.clone()) else {
        return Ok(None);
    };
    if before == now || (!to.is_empty() && !to.contains(&as_text(&now))) {
        return Ok(None);
    }
    let mut data = Map::new();
    if let Value::Object(top) = result {
        for (key, value) in top {
            if let Some(name) = registered(fields, key) {
                if scalar(value) && name != "from" && name != "to" {
                    data.insert(name, clean(value));
                }
            }
        }
    }
    data.insert("from".into(), clean(&before));
    data.insert("to".into(), clean(&now));
    Ok(Some(data))
}

/// **new** 记下的 id（最多 [`MAX_SEEN_IDS`] 个，最久没见的先丢）。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Seen {
    primed: bool,
    order: VecDeque<String>,
    set: HashSet<String>,
}

impl Seen {
    pub fn len(&self) -> usize {
        self.order.len()
    }

    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    fn remember(&mut self, id: String) {
        if self.set.contains(&id) {
            // 又见到了：挪到最新的一头（还在列表里的不会被挤掉）
            if let Some(i) = self.order.iter().position(|x| *x == id) {
                self.order.remove(i);
            }
        } else {
            self.set.insert(id.clone());
        }
        self.order.push_back(id);
        while self.order.len() > MAX_SEEN_IDS {
            if let Some(old) = self.order.pop_front() {
                self.set.remove(&old);
            }
        }
    }
}

fn id_text(value: &Value) -> Option<String> {
    match value {
        Value::String(s) if !s.is_empty() => Some(truncate(&sanitize::strip_invisible(s), 200)),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// **new**：`list`（路径，空 = 结果本身）是数组，按元素的 `id` 字段去重；第一次只记底；之后新出现的每个发一次，
/// `data` = 元素里事件 `data` 里登记过的字段。只看列表的前 [`MAX_SEEN_IDS`] 个；没有 id 的元素不管。
pub fn fresh(
    seen: &mut Seen,
    result: &Value,
    list: Option<&str>,
    id: &str,
    fields: &BTreeMap<String, String>,
) -> Result<Vec<Map<String, Value>>, String> {
    let path = list.unwrap_or("");
    let items = manifest::lookup(result, path)
        .and_then(Value::as_array)
        .ok_or_else(|| {
            if path.is_empty() {
                "结果不是数组（观察项写了 compare: new，没写 list）".to_string()
            } else {
                format!("结果里的 {path} 不是数组")
            }
        })?;
    let mut out = Vec::new();
    for item in items.iter().take(MAX_SEEN_IDS) {
        let Some(key) = manifest::lookup(item, id).and_then(id_text) else {
            continue;
        };
        if seen.primed && !seen.set.contains(&key) {
            let mut data = Map::new();
            if let Value::Object(map) = item {
                for (k, v) in map {
                    if let Some(name) = registered(fields, k) {
                        data.insert(name, clean(v));
                    }
                }
            }
            out.push(data);
        }
        seen.remember(key);
    }
    seen.primed = true;
    Ok(out)
}

// ---------- 运行 ----------

#[derive(Debug)]
enum Baseline {
    Changed(Option<Value>),
    New(Seen),
}

#[derive(Debug)]
struct Slot {
    digest: String,
    baseline: Baseline,
    due: Instant,
    failures: u32,
    stopped: Option<String>,
    last_error: Option<String>,
    /// 发了几件事。
    sent: u64,
}

impl Slot {
    fn new(digest: &str, compare: &Compare, now: Instant) -> Self {
        Self {
            digest: digest.to_string(),
            baseline: match compare {
                Compare::Changed { .. } => Baseline::Changed(None),
                Compare::New { .. } => Baseline::New(Seen::default()),
            },
            due: now,
            failures: 0,
            stopped: None,
            last_error: None,
            sent: 0,
        }
    }
}

#[derive(Debug, Default)]
struct Inner {
    slots: HashMap<String, Slot>,
    /// 上一次看到开着的（插件挂的）游戏和时间（宽限期用）。
    last_game: Option<(String, Instant)>,
}

/// 一次看到的事实（清单、玩家的决定、开关、信任、工具清单、开着的游戏）。
struct Snapshot {
    effective: Effective,
    choices: BTreeMap<String, Choice>,
    enabled: bool,
    trusted: bool,
    tools: Vec<Tool>,
    local: bool,
    running: Vec<String>,
}

/// 一个第三方插件的观察项（一个插件一个，和连接 [`Server`] 一样在换插件对象时共用）。
pub struct Watcher {
    plugin: String,
    server: Arc<Server>,
    timing: Timing,
    probe: Mutex<Option<GamesProbe>>,
    host: Mutex<Option<Arc<dyn Host>>>,
    inner: Mutex<Inner>,
    wake: Arc<tokio::sync::Notify>,
    started: AtomicBool,
}

impl Watcher {
    pub fn new(plugin: &str, server: Arc<Server>, timing: Timing) -> Self {
        Self {
            plugin: plugin.to_string(),
            server,
            timing,
            probe: Mutex::new(None),
            host: Mutex::new(None),
            inner: Mutex::new(Inner::default()),
            wake: Arc::new(tokio::sync::Notify::new()),
            started: AtomicBool::new(false),
        }
    }

    pub fn timing(&self) -> Timing {
        self.timing
    }

    /// 宿主告诉它现在开着哪些游戏（没给 = 当没有游戏开着：挂在游戏上的观察项一直等）。
    pub fn set_games_probe(&self, probe: GamesProbe) {
        *self.probe.lock().unwrap() = Some(probe);
        self.wake.notify_one();
    }

    /// 开始（第一次）或者对一下设置（之后每次）：记下宿主、马上看一遍。
    pub fn start(self: &Arc<Self>, host: Arc<dyn Host>) {
        *self.host.lock().unwrap() = Some(host.clone());
        self.wake.notify_one();
        if self.started.swap(true, Ordering::SeqCst) {
            return;
        }
        let weak = Arc::downgrade(self);
        host.spawn(Box::pin(async move {
            loop {
                let Some(watcher) = weak.upgrade() else {
                    break;
                };
                let wait = watcher.step().await;
                let wake = watcher.wake.clone();
                drop(watcher);
                tokio::select! {
                    _ = tokio::time::sleep(wait) => {}
                    _ = wake.notified() => {}
                }
            }
        }));
    }

    /// 停下的、退避中的观察项马上再查一次（玩家点了“重试”）。
    pub fn retry(&self, event: &str) {
        if let Some(slot) = self.inner.lock().unwrap().slots.get_mut(event) {
            slot.stopped = None;
            slot.failures = 0;
            slot.last_error = None;
            slot.due = Instant::now();
        }
        self.wake.notify_one();
    }

    fn host(&self) -> Option<Arc<dyn Host>> {
        self.host.lock().unwrap().clone()
    }

    fn snapshot(&self, host: Option<&Arc<dyn Host>>) -> Snapshot {
        let running = self
            .probe
            .lock()
            .unwrap()
            .as_ref()
            .map(|probe| probe())
            .unwrap_or_default();
        Snapshot {
            effective: self.server.kotone_manifest(),
            choices: host.map(|h| choices(&h.settings())).unwrap_or_default(),
            enabled: host.is_some_and(|h| h.enabled()),
            trusted: self.server.trusted(),
            tools: self.server.known().tools,
            local: self.server.spec().as_ref().is_ok_and(|s| s.is_local()),
            running,
        }
    }

    fn facts<'a>(
        &self,
        snap: &'a Snapshot,
        watch: &'a Watch,
        digest: &'a str,
        stopped: Option<&'a str>,
        game_ok: bool,
    ) -> Facts<'a> {
        Facts {
            choice: snap.choices.get(&watch.event),
            digest,
            enabled: snap.enabled,
            trusted: snap.trusted,
            tool_name: &watch.tool,
            tool: tool_fact(&snap.tools, &watch.tool),
            stopped,
            game_ok,
        }
    }

    /// 看一遍：到点的查一次、比较、发事件。返回多久以后再看。
    async fn step(&self) -> Duration {
        let Some(host) = self.host() else {
            return SLEEP;
        };
        let mut snap = self.snapshot(Some(&host));
        let any_on = snap
            .effective
            .manifest
            .watches
            .iter()
            .any(|w| snap.choices.get(&w.event).is_some_and(|c| c.on));
        if !any_on {
            self.inner.lock().unwrap().slots.clear();
            return SLEEP;
        }
        // 开着观察项、还不知道服务器有哪些工具：先连上（工具标没标只读要看清单）
        if snap.tools.is_empty() && snap.enabled && snap.trusted {
            let _ = self.server.connect(&host, false).await;
            snap.tools = self.server.known().tools;
        }
        let now = Instant::now();
        let games = snap.effective.manifest.games.clone();
        let has_games = !games.is_empty();
        let fingerprint = self.server.fingerprint().to_string();
        let mut wait = SLEEP;
        let mut due: Vec<Watch> = Vec::new();
        let game;
        {
            let mut inner = self.inner.lock().unwrap();
            if let Some(g) = games.iter().find(|g| snap.running.contains(g)) {
                inner.last_game = Some((g.clone(), now));
            }
            game = game_now(
                &games,
                &snap.running,
                inner.last_game.as_ref(),
                now,
                self.timing.grace,
            );
            let live: HashMap<String, String> = snap
                .effective
                .manifest
                .watches
                .iter()
                .map(|w| (w.event.clone(), digest(&fingerprint, w)))
                .collect();
            inner
                .slots
                .retain(|event, slot| live.get(event) == Some(&slot.digest));
            for watch in &snap.effective.manifest.watches {
                let digest = &live[&watch.event];
                let stopped = inner
                    .slots
                    .get(&watch.event)
                    .and_then(|s| s.stopped.clone());
                let state =
                    gate(&self.facts(&snap, watch, digest, stopped.as_deref(), game.is_some()));
                match state {
                    State::Off => {
                        inner.slots.remove(&watch.event);
                    }
                    State::Polling => {
                        let slot = inner
                            .slots
                            .entry(watch.event.clone())
                            .or_insert_with(|| Slot::new(digest, &watch.compare, now));
                        if slot.due <= now {
                            due.push(watch.clone());
                        } else {
                            wait = wait.min(slot.due - now);
                        }
                    }
                    State::Waiting => wait = wait.min(self.timing.idle),
                    _ => {}
                }
            }
        }
        if due.is_empty() {
            return wait;
        }
        let game = game.unwrap_or_default();
        let source = format!("{EVENT_SOURCE_PREFIX}{}", self.plugin);
        let mut memo: HashMap<String, Result<ToolCallResult, String>> = HashMap::new();
        for watch in due {
            let arguments = Value::Object(watch.arguments.clone());
            let key = format!("{}\u{0}{}", watch.tool, canonical_text(&arguments));
            let result = match memo.get(&key) {
                Some(result) => result.clone(),
                None => {
                    let result = self.server.call_raw(&host, &watch.tool, arguments).await;
                    memo.insert(key, result.clone());
                    result
                }
            };
            let text_ok = snap.effective.watch_from.get(&watch.event) == Some(&Source::Preset);
            let fields = snap
                .effective
                .manifest
                .events
                .iter()
                .find(|e| e.name == watch.event)
                .map(|e| e.data.clone())
                .unwrap_or_default();
            let interval = self.timing.interval(&watch, snap.local, has_games);
            let value = result
                .map_err(Failure::Retry)
                .and_then(|r| result_value(&r, text_ok));
            let after = Instant::now();
            let sends = {
                let mut inner = self.inner.lock().unwrap();
                let Some(slot) = inner.slots.get_mut(&watch.event) else {
                    continue;
                };
                let compared = value.and_then(|value| {
                    let out = match (&watch.compare, &mut slot.baseline) {
                        (Compare::Changed { field, to }, Baseline::Changed(last)) => {
                            changed(last, &value, field, to, &fields)
                                .map(|data| data.into_iter().collect::<Vec<_>>())
                        }
                        (Compare::New { list, id }, Baseline::New(seen)) => {
                            fresh(seen, &value, list.as_deref(), id, &fields)
                        }
                        _ => Err("观察项的比较方式变了".to_string()),
                    };
                    out.map_err(Failure::Retry)
                });
                match compared {
                    Ok(sends) => {
                        slot.failures = 0;
                        slot.last_error = None;
                        slot.due = after + interval;
                        slot.sent += sends.len() as u64;
                        wait = wait.min(interval);
                        sends
                    }
                    Err(Failure::Stop(reason)) => {
                        slot.stopped = Some(reason.clone());
                        slot.last_error = Some(reason);
                        Vec::new()
                    }
                    Err(Failure::Retry(reason)) => {
                        slot.failures += 1;
                        slot.last_error = Some(reason.clone());
                        if slot.failures >= MAX_FAILURES {
                            slot.stopped =
                                Some(format!("连续 {MAX_FAILURES} 次没查成，停下了：{reason}"));
                        } else {
                            let backoff = self.timing.backoff(interval, slot.failures);
                            slot.due = after + backoff;
                            wait = wait.min(backoff);
                        }
                        Vec::new()
                    }
                }
            };
            let kind = manifest::event_type(&self.plugin, &watch.event);
            for data in sends {
                host.game_event(&game, &kind, &source, data, None);
            }
        }
        wait
    }

    /// 给插件页“事件”一块：每个观察项的事件、说明、调哪个工具、多久查一次、怎么比、从哪来（预设 / 服务器 / 自己加的）、
    /// 工具标没标只读、确认要的指纹、玩家的决定、现在的样子（没开 / 要确认 / 查不了 / 等游戏 / 在查 / 停了）、上次的错。
    pub fn view(&self) -> Vec<Value> {
        let host = self.host();
        let snap = self.snapshot(host.as_ref());
        let fingerprint = self.server.fingerprint().to_string();
        let has_games = !snap.effective.manifest.games.is_empty();
        let inner = self.inner.lock().unwrap();
        let game_ok = game_now(
            &snap.effective.manifest.games,
            &snap.running,
            inner.last_game.as_ref(),
            Instant::now(),
            self.timing.grace,
        )
        .is_some();
        snap.effective
            .manifest
            .watches
            .iter()
            .map(|watch| {
                let digest = digest(&fingerprint, watch);
                let slot = inner.slots.get(&watch.event).filter(|s| s.digest == digest);
                let state = gate(&self.facts(
                    &snap,
                    watch,
                    &digest,
                    slot.and_then(|s| s.stopped.as_deref()),
                    game_ok,
                ));
                let choice = snap.choices.get(&watch.event).cloned().unwrap_or_default();
                let label = snap
                    .effective
                    .manifest
                    .events
                    .iter()
                    .find(|e| e.name == watch.event)
                    .map(|e| e.label.clone())
                    .unwrap_or_default();
                let mut out = serde_json::to_value(watch).unwrap_or_else(|_| json!({}));
                out["type"] = json!(manifest::event_type(&self.plugin, &watch.event));
                out["label"] = json!(label);
                out["intervalSeconds"] =
                    json!(self.timing.interval(watch, snap.local, has_games).as_secs());
                out["from"] = json!(snap
                    .effective
                    .watch_from
                    .get(&watch.event)
                    .copied()
                    .unwrap_or_default());
                out["readOnly"] = match tool_fact(&snap.tools, &watch.tool) {
                    ToolFact::ReadOnly => json!(true),
                    ToolFact::Unmarked => json!(false),
                    ToolFact::Missing | ToolFact::Unknown => Value::Null,
                };
                out["toolKnown"] = json!(tool_fact(&snap.tools, &watch.tool) != ToolFact::Missing);
                out["digest"] = json!(digest);
                out["on"] = json!(choice.on);
                out["confirmed"] = json!(choice.confirmed == digest);
                out["allowUnmarked"] = json!(choice.allow_unmarked);
                out["status"] = serde_json::to_value(&state).unwrap_or_else(|_| json!({}));
                out["lastError"] = json!(slot.and_then(|s| s.last_error.clone()));
                out["failures"] = json!(slot.map_or(0, |s| s.failures));
                out["sent"] = json!(slot.map_or(0, |s| s.sent));
                out
            })
            .collect()
    }

    /// 有观察项停下了：第一个的事件说明和原因（插件卡片上显示）。
    pub fn stopped(&self) -> Option<(String, String)> {
        self.view().into_iter().find_map(|w| {
            (w["status"]["state"] == "stopped").then(|| {
                (
                    w["label"].as_str().unwrap_or_default().to_string(),
                    w["status"]["reason"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string(),
                )
            })
        })
    }

    /// 自动化里这个事件现在为什么不会发生（事件目录用；不看运行时的失败、游戏开没开）：没有观察项、没打开、要重新确认、
    /// 插件关着、没信任。None = 会发生。`settings`：插件设置（`plugins.<id>.settings`）。
    pub fn event_off(&self, settings: &Value, enabled: bool, event: &str) -> Option<String> {
        let effective = self.server.kotone_manifest();
        let id = &self.plugin;
        let Some(watch) = effective.manifest.watches.iter().find(|w| w.event == event) else {
            return Some(format!("第三方插件 {id} 发不出这件事（没有观察项）"));
        };
        if !enabled {
            return Some(format!("{id}插件关着"));
        }
        let choice = choices(settings).remove(event).filter(|c| c.on);
        let digest = digest(self.server.fingerprint(), watch);
        match choice {
            None => Some(format!(
                "{id}插件的这个观察项没打开（插件页「设置 → 事件」）"
            )),
            Some(c) if c.confirmed != digest => Some(format!(
                "{id}插件的这个观察项要重新确认（插件页「设置 → 事件」）"
            )),
            Some(_) if !crate::trusted_in(settings, self.server.fingerprint()) => {
                Some(format!("还没信任{id}插件"))
            }
            Some(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{parse_value, DEFAULT_EVERY};

    fn fields(names: &[&str]) -> BTreeMap<String, String> {
        names
            .iter()
            .map(|n| (n.to_string(), "说明".to_string()))
            .collect()
    }

    fn watch(compare: Compare, every: u64) -> Watch {
        Watch {
            event: "done".into(),
            tool: "status".into(),
            arguments: Map::new(),
            every_seconds: every,
            compare,
        }
    }

    fn changed_on(field: &str, to: &[&str]) -> Compare {
        Compare::Changed {
            field: field.into(),
            to: to.iter().map(|s| s.to_string()).collect(),
        }
    }

    /// changed：第一次只记底；没变不发；变了才发，带旧值新值和顶层登记过的标量字段（snake_case 的也对得上）。
    #[test]
    fn changed_records_a_baseline_then_reports_changes() {
        let f = fields(&["from", "to", "lastStatus", "app", "durationSeconds"]);
        let mut last = None;
        let idle = json!({"state": "idle"});
        assert_eq!(
            changed(&mut last, &idle, "state", &[], &f).unwrap(),
            None,
            "第一次只记底"
        );
        assert_eq!(
            changed(&mut last, &idle, "state", &[], &f).unwrap(),
            None,
            "没变"
        );
        let running = json!({"state": "running", "app": "OneDragon", "current_node": "x"});
        let data = changed(&mut last, &running, "state", &[], &f)
            .unwrap()
            .unwrap();
        assert_eq!(
            Value::Object(data),
            json!({"from": "idle", "to": "running", "app": "OneDragon"}),
            "没登记的字段不带"
        );
        let done = json!({"state": "success", "last_status": "完成\u{200B}了", "duration_seconds": 12.5,
                           "app": {"nested": 1}, "from": "假的"});
        let data = changed(&mut last, &done, "state", &[], &f)
            .unwrap()
            .unwrap();
        assert_eq!(
            data["lastStatus"], "完成了",
            "snake_case 对上 camelCase、剥掉看不见的字符"
        );
        assert_eq!(data["durationSeconds"], 12.5);
        assert!(data.get("app").is_none(), "只带标量");
        assert_eq!(data["from"], "running", "from / to 以比较出来的为准");
    }

    #[test]
    fn changed_only_fires_for_the_listed_values_and_reads_paths() {
        let f = fields(&[]);
        let mut last = None;
        let at = |s: &str| json!({"run": {"items": [{"state": s}]}});
        let to = ["success".to_string(), "failed".to_string()];
        assert!(
            changed(&mut last, &at("idle"), "run.items.0.state", &to, &f)
                .unwrap()
                .is_none()
        );
        assert!(
            changed(&mut last, &at("running"), "run.items.0.state", &to, &f)
                .unwrap()
                .is_none(),
            "running 不在 to 里"
        );
        let data = changed(&mut last, &at("failed"), "run.items.0.state", &to, &f)
            .unwrap()
            .unwrap();
        assert_eq!(data["from"], "running", "不发的那次也更新了底");
        // 结果里没有这个字段：算没查成，底不动
        assert!(changed(&mut last, &json!({}), "run.items.0.state", &to, &f).is_err());
        assert!(
            changed(&mut last, &at("failed"), "run.items.0.state", &to, &f)
                .unwrap()
                .is_none()
        );
        // 数字、true 按文字比
        let mut n = None;
        let num = ["3".to_string()];
        changed(&mut n, &json!({"n": 2}), "n", &num, &f).unwrap();
        assert!(changed(&mut n, &json!({"n": 3}), "n", &num, &f)
            .unwrap()
            .is_some());
        // 很长的字截到 500
        let mut s = None;
        changed(&mut s, &json!({"t": "a"}), "t", &[], &f).unwrap();
        let data = changed(&mut s, &json!({"t": "长".repeat(800)}), "t", &[], &f)
            .unwrap()
            .unwrap();
        assert_eq!(data["to"].as_str().unwrap().chars().count(), MAX_EVENT_TEXT);
    }

    /// new：第一次只记底；按 id 去重；新的每个发一次，带登记过的字段；没有 id 的不管；最多记 500 个。
    #[test]
    fn new_dedupes_by_id_after_a_baseline() {
        let f = fields(&["id", "name"]);
        let mut seen = Seen::default();
        let list = |ids: &[u64]| json!({"items": ids.iter().map(|i| json!({"id": i, "name": format!("n{i}"), "secret": 1})).collect::<Vec<_>>()});
        assert!(
            fresh(&mut seen, &list(&[1, 2]), Some("items"), "id", &f)
                .unwrap()
                .is_empty(),
            "记底"
        );
        let out = fresh(&mut seen, &list(&[3, 1, 2]), Some("items"), "id", &f).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(
            Value::Object(out[0].clone()),
            json!({"id": 3, "name": "n3"})
        );
        assert!(fresh(&mut seen, &list(&[3, 1, 2]), Some("items"), "id", &f)
            .unwrap()
            .is_empty());
        // 没有 id 的元素不管；不是数组算没查成
        let odd = json!({"items": [{"name": "x"}, {"id": "s1"}]});
        assert_eq!(
            fresh(&mut seen, &odd, Some("items"), "id", &f)
                .unwrap()
                .len(),
            1
        );
        assert!(fresh(&mut seen, &json!({"items": 1}), Some("items"), "id", &f).is_err());
        // 结果本身是数组（没写 list）
        let mut top = Seen::default();
        fresh(&mut top, &json!([{"id": "a"}]), None, "id", &f).unwrap();
        assert_eq!(
            fresh(&mut top, &json!([{"id": "a"}, {"id": "b"}]), None, "id", &f)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn new_keeps_at_most_500_ids_and_only_reads_the_first_500() {
        let f = fields(&["id"]);
        let mut seen = Seen::default();
        let ids =
            |from: u64, to: u64| json!((from..to).map(|i| json!({"id": i})).collect::<Vec<_>>());
        fresh(&mut seen, &ids(0, 700), None, "id", &f).unwrap();
        assert_eq!(seen.len(), MAX_SEEN_IDS, "只看前 500 个");
        // 前 500 个还在：一个都不算新的
        assert!(fresh(&mut seen, &ids(0, 700), None, "id", &f)
            .unwrap()
            .is_empty());
        // 一批新的挤掉最久没见的
        let out = fresh(&mut seen, &ids(1000, 1100), None, "id", &f).unwrap();
        assert_eq!(out.len(), 100);
        assert_eq!(seen.len(), MAX_SEEN_IDS);
    }

    #[test]
    fn results_come_from_structured_content_or_preset_text() {
        let structured = ToolCallResult {
            content: vec![json!({"type": "text", "text": "{\"state\": \"text\"}"})],
            structured_content: Some(json!({"state": "idle"})),
            ..Default::default()
        };
        assert_eq!(
            result_value(&structured, false).unwrap(),
            json!({"state": "idle"})
        );
        let text = ToolCallResult {
            content: vec![json!({"type": "text", "text": " {\"state\": \"running\"} "})],
            ..Default::default()
        };
        assert_eq!(
            result_value(&text, true).unwrap(),
            json!({"state": "running"}),
            "预设可以读文字里的 JSON"
        );
        assert!(
            matches!(result_value(&text, false), Err(Failure::Stop(_))),
            "别的要 structuredContent"
        );
        let words = ToolCallResult {
            content: vec![json!({"type": "text", "text": "跑完了"})],
            ..Default::default()
        };
        assert!(matches!(result_value(&words, true), Err(Failure::Retry(_))));
        let error = ToolCallResult {
            content: vec![json!({"type": "text", "text": "坏了"})],
            is_error: true,
            ..Default::default()
        };
        assert_eq!(
            result_value(&error, true),
            Err(Failure::Retry("工具说出错了：坏了".into()))
        );
    }

    /// 间隔的下限和标准一致（`Watch::interval`）；退避 ×2、最多 10 分钟。
    #[test]
    fn intervals_and_backoff() {
        let t = Timing::default();
        for every in [1, 10, 30, 59, 60, 300] {
            let w = watch(changed_on("s", &[]), every);
            for (local, games) in [(true, true), (true, false), (false, true), (false, false)] {
                assert_eq!(
                    t.interval(&w, local, games),
                    w.interval(local, games),
                    "{every} {local} {games}"
                );
            }
        }
        let i = Duration::from_secs(30);
        assert_eq!(t.backoff(i, 1), Duration::from_secs(60));
        assert_eq!(t.backoff(i, 2), Duration::from_secs(120));
        assert_eq!(t.backoff(i, 5), Duration::from_secs(600), "最多 10 分钟");
        assert_eq!(
            t.backoff(Duration::from_secs(900), 1),
            Duration::from_secs(900),
            "间隔更长的照间隔"
        );
    }

    fn facts<'a>(choice: Option<&'a Choice>, digest: &'a str) -> Facts<'a> {
        Facts {
            choice,
            digest,
            enabled: true,
            trusted: true,
            tool_name: "status",
            tool: ToolFact::ReadOnly,
            stopped: None,
            game_ok: true,
        }
    }

    /// 没打开不查；确认的内容变了要重新确认；插件关着、没信任、没有工具查不了；没标只读要明确同意；停下的、等游戏的。
    #[test]
    fn the_gate_follows_the_contract() {
        let d = "abc";
        assert_eq!(gate(&facts(None, d)), State::Off);
        let off = Choice {
            on: false,
            confirmed: d.into(),
            allow_unmarked: false,
        };
        assert_eq!(gate(&facts(Some(&off), d)), State::Off);
        let on = Choice {
            on: true,
            confirmed: d.into(),
            allow_unmarked: false,
        };
        assert_eq!(gate(&facts(Some(&on), d)), State::Polling);
        assert!(
            matches!(gate(&facts(Some(&on), "changed")), State::Confirm { .. }),
            "内容变了"
        );
        let never = Choice {
            on: true,
            ..Default::default()
        };
        assert!(matches!(
            gate(&facts(Some(&never), d)),
            State::Confirm { .. }
        ));
        assert!(matches!(
            gate(&Facts {
                enabled: false,
                ..facts(Some(&on), d)
            }),
            State::Blocked { .. }
        ));
        assert!(matches!(
            gate(&Facts {
                trusted: false,
                ..facts(Some(&on), d)
            }),
            State::Blocked { .. }
        ));
        assert!(matches!(
            gate(&Facts {
                tool: ToolFact::Missing,
                ..facts(Some(&on), d)
            }),
            State::Blocked { .. }
        ));
        assert!(matches!(
            gate(&Facts {
                tool: ToolFact::Unmarked,
                ..facts(Some(&on), d)
            }),
            State::Confirm { .. }
        ));
        let agreed = Choice {
            allow_unmarked: true,
            ..on.clone()
        };
        assert_eq!(
            gate(&Facts {
                tool: ToolFact::Unmarked,
                ..facts(Some(&agreed), d)
            }),
            State::Polling
        );
        assert_eq!(
            gate(&Facts {
                tool: ToolFact::Unknown,
                ..facts(Some(&on), d)
            }),
            State::Polling,
            "连上以后再看"
        );
        assert!(matches!(
            gate(&Facts {
                stopped: Some("x"),
                ..facts(Some(&on), d)
            }),
            State::Stopped { .. }
        ));
        assert_eq!(
            gate(&Facts {
                game_ok: false,
                ..facts(Some(&on), d)
            }),
            State::Waiting
        );
    }

    #[test]
    fn games_gate_polling_with_a_grace_period() {
        let grace = Duration::from_secs(180);
        let now = Instant::now();
        let zzz = vec!["zzz".to_string()];
        assert_eq!(
            game_now(&[], &[], None, now, grace),
            Some(String::new()),
            "games 为空一直查"
        );
        assert_eq!(
            game_now(&zzz, &["lol".into(), "zzz".into()], None, now, grace),
            Some("zzz".into())
        );
        assert_eq!(
            game_now(&zzz, &["lol".into()], None, now, grace),
            None,
            "别的游戏开着不算"
        );
        let left = ("zzz".to_string(), now);
        assert_eq!(
            game_now(
                &zzz,
                &[],
                Some(&left),
                now + Duration::from_secs(179),
                grace
            ),
            Some("zzz".into()),
            "刚退出"
        );
        assert_eq!(
            game_now(
                &zzz,
                &[],
                Some(&left),
                now + Duration::from_secs(181),
                grace
            ),
            None,
            "宽限过了"
        );
    }

    /// 确认绑定内容：工具、参数、比较、配置的指纹变了就不一样；参数的键换个顺序、间隔变了还是一样。
    #[test]
    fn digests_bind_what_was_confirmed() {
        let base = watch(changed_on("state", &["success"]), 30);
        let d = digest("fp", &base);
        let mut args = base.clone();
        args.arguments.insert("b".into(), json!(1));
        args.arguments.insert("a".into(), json!(2));
        let mut reordered = base.clone();
        reordered.arguments.insert("a".into(), json!(2));
        reordered.arguments.insert("b".into(), json!(1));
        assert_eq!(digest("fp", &args), digest("fp", &reordered));
        assert_ne!(digest("fp", &args), d);
        assert_eq!(
            digest(
                "fp",
                &watch(changed_on("state", &["success"]), DEFAULT_EVERY)
            ),
            d
        );
        assert_ne!(digest("other", &base), d, "配置变了");
        assert_ne!(
            digest("fp", &watch(changed_on("state", &["failed"]), 30)),
            d
        );
        assert_ne!(
            digest(
                "fp",
                &Watch {
                    tool: "x".into(),
                    ..base.clone()
                }
            ),
            d
        );
        let parsed = parse_value(
            &json!({"watches": [{"event": "done", "tool": "status", "compare": "changed",
            "field": "state", "to": "success", "everySeconds": 30}]}),
        )
        .unwrap();
        assert_eq!(
            digest("fp", &parsed.manifest.watches[0]),
            d,
            "从清单读出来的一样"
        );
    }

    #[test]
    fn choices_are_read_from_plugin_settings() {
        let settings = json!({"trusted": "x", "watches": {
            "done": {"on": true, "confirmed": "abc"},
            "other": {"on": "yes"},
            "gone": null
        }});
        let c = choices(&settings);
        assert_eq!(c.len(), 1);
        assert_eq!(
            c["done"],
            Choice {
                on: true,
                confirmed: "abc".into(),
                allow_unmarked: false
            }
        );
        assert!(choices(&json!({})).is_empty());
        assert_eq!(camel("last_status"), "lastStatus");
        assert_eq!(camel("duration_seconds"), "durationSeconds");
        assert_eq!(camel("app"), "app");
        assert_eq!(camel("_x"), "x");
    }
}
