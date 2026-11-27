//! 游戏中心能给的事（ADR 036 D4 的目录）：每种事的类型、中文说明、`data` 里的字段和说明。
//!
//! **只有一种登记方式**（原则 15）：本体只登记任何游戏都有的 `game.*`（[`generic_events`]）；游戏专用的事由带
//! 适配器的插件在自己的 `plugin.toml` 里用 `[[events]]` 登记（`kotone-plugin-api` 的 `Manifest`，ADR 038 D3
//! 第 10 条），宿主按插件的开关合成一份 [`EventCatalog`]。自动化保存时的校验、`game.state` 里“每个游戏能给哪些事”、
//! 界面里的选择都用这一份。
//!
//! 插件关着（或它要的数据开关关着）时，它登记的事还在目录里：保存校验认得，已经保存的自动化不删、不报废，
//! 只是标着现在为什么不会发生（[`CatalogEntry::off`]），保存时说清楚（[`EventCatalog::notes`]，原则 21）。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use super::EventTrigger;

/// 通用事件的范围（`game.*`），插件的适配器不能用这个名字。
pub const GENERIC_SCOPE: &str = "game";
/// 本体那一组的适配器 id（只有进程，所有有进程名的游戏配置都有）。
pub const GENERIC_ADAPTER: &str = "generic";

/// 一种事。插件在 `plugin.toml` 里这样写（`data.<字段>` 是 TOML 的点号键）：
///
/// ```toml
/// [[events]]
/// type = "<适配器>.<名字>"
/// label = "中文说明"
/// needs = ["<数据开关>"]
/// data.team = "哪一边：ally（我方）/ enemy（对面）"
/// private = ["<字段>"]       # 可选：不给她看的字段（见 EventSpec::private）
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventSpec {
    /// `<范围>.<名字>`：小写、点分隔，名字是 snake_case 的过去式或状态（D2）。
    #[serde(rename = "type")]
    pub kind: String,
    /// 中文说明：界面上的名字（插件的文案以它为准），也给她看。
    pub label: String,
    /// `data` 里的字段（camelCase）→ 说明。自动化里和字段同名的参数会自动填上（D6）。
    #[serde(default)]
    pub data: BTreeMap<String, String>,
    /// 插件登记的：要插件里哪些数据开关开着才会发生（本体的为空）。
    #[serde(default)]
    pub needs: Vec<String>,
    /// `data` 里不给她看的字段（ADR 043 D10：语音输入发出去的那句话，只经那条推送给她的说明交给她，玩家关掉推送就不给）：
    /// `game.state` 的最近几件事里去掉（[`EventCatalog::data_for_her`]）；自动化照样拿到整份（那是玩家自己的自动化）。
    /// 要是 `data` 里登记过的字段。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub private: Vec<String>,
}

/// 说明最多几个字。
pub const MAX_LABEL_CHARS: usize = 64;

fn snake_name(name: &str) -> bool {
    name.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

fn camel_field(name: &str) -> bool {
    name.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        && name.chars().all(|c| c.is_ascii_alphanumeric())
}

impl EventSpec {
    /// 检查写法（D2）：类型是 `<scope>.<名字>`，名字小写加下划线；有说明；字段名 camelCase、各有说明。
    pub fn check(&self, scope: &str) -> Result<(), String> {
        let name = self
            .kind
            .strip_prefix(scope)
            .and_then(|rest| rest.strip_prefix('.'));
        if !name.is_some_and(snake_name) {
            return Err(format!(
                "事件 {} 要写成 {scope}.<名字>（名字用小写字母、数字和下划线）",
                self.kind
            ));
        }
        let label = self.label.trim();
        if label.is_empty() || label.chars().count() > MAX_LABEL_CHARS {
            return Err(format!(
                "事件 {} 要有中文说明（不超过 {MAX_LABEL_CHARS} 字）",
                self.kind
            ));
        }
        if let Some((field, _)) = self
            .data
            .iter()
            .find(|(field, about)| !camel_field(field) || about.trim().is_empty())
        {
            return Err(format!(
                "事件 {} 的 data.{field} 要用 camelCase，并且写上说明",
                self.kind
            ));
        }
        if let Some(field) = self.private.iter().find(|f| !self.data.contains_key(*f)) {
            return Err(format!(
                "事件 {} 的 private 里的 {field} 要是 data 里登记过的字段",
                self.kind
            ));
        }
        Ok(())
    }
}

/// `data` 的字段：（名字，说明）。
type Fields = &'static [(&'static str, &'static str)];

/// 本体登记的：（类型，说明，字段）。游戏专用的事不写在这里，由插件登记。
const GENERIC: &[(&str, &str, Fields)] = &[
    ("game.started", "游戏启动了", &[]),
    (
        "game.stopped",
        "游戏退出了",
        &[("elapsedSeconds", "这次玩了多少秒")],
    ),
    ("game.focused", "切回游戏窗口", &[]),
    ("game.unfocused", "离开游戏窗口", &[]),
    (
        "game.phase_changed",
        "阶段变了",
        &[
            ("from", "之前的阶段"),
            (
                "to",
                "现在的阶段：stopped / running / menu / queue / pre_match / loading / in_match / post_match",
            ),
            ("native", "游戏自己的阶段名"),
        ],
    ),
    (
        "game.match_started",
        "一局开始了（选英雄、加载或进对局，先到哪个算哪个）",
        &[],
    ),
    (
        "game.match_ended",
        "一局结束了",
        &[("elapsedSeconds", "这一局多少秒")],
    ),
];

/// 任何游戏都有的事（不需要适配器）。
pub fn generic_events() -> Vec<EventSpec> {
    GENERIC
        .iter()
        .map(|(kind, label, fields)| EventSpec {
            kind: (*kind).to_string(),
            label: (*label).to_string(),
            data: fields
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                .collect(),
            needs: Vec::new(),
            private: Vec::new(),
        })
        .collect()
}

/// 合并后的目录里的一种事。
#[derive(Debug, Clone, PartialEq)]
pub struct CatalogEntry {
    pub spec: EventSpec,
    /// `generic` 或插件带的适配器 id。
    pub adapter: String,
    /// 登记它的插件（id，界面上的名字）；本体的为 None。
    pub plugin: Option<(String, String)>,
    /// 挂在哪些游戏配置上；空 = 所有有进程名的游戏配置。
    pub games: Vec<String>,
    /// 现在为什么不会发生（“英雄联盟插件关着”）；None = 会发生。
    pub off: Option<String>,
}

/// 合并后的事件目录：本体的 `game.*`，加上开着、关着的插件登记的（宿主按插件开关算好 [`CatalogEntry::off`]）。
#[derive(Debug, Clone, PartialEq)]
pub struct EventCatalog {
    entries: Vec<CatalogEntry>,
}

impl Default for EventCatalog {
    fn default() -> Self {
        Self::generic()
    }
}

impl EventCatalog {
    /// 只有本体的 `game.*`。
    pub fn generic() -> Self {
        Self {
            entries: generic_events()
                .into_iter()
                .map(|spec| CatalogEntry {
                    spec,
                    adapter: GENERIC_ADAPTER.to_string(),
                    plugin: None,
                    games: Vec::new(),
                    off: None,
                })
                .collect(),
        }
    }

    /// 加一种插件登记的事。和已有的重名时不加，返回错误（两个插件不能登记同一种事）。
    pub fn add(&mut self, entry: CatalogEntry) -> Result<(), String> {
        if let Some(have) = self.get(&entry.spec.kind) {
            let owner = |e: &CatalogEntry| {
                e.plugin
                    .as_ref()
                    .map_or_else(|| "本体".to_string(), |(_, name)| format!("{name}插件"))
            };
            return Err(format!(
                "事件 {} 已经由{}登记了，{}不能再登记",
                entry.spec.kind,
                owner(have),
                owner(&entry)
            ));
        }
        self.entries.push(entry);
        Ok(())
    }

    /// 全部（本体的在前，插件的按登记顺序）。
    pub fn entries(&self) -> &[CatalogEntry] {
        &self.entries
    }

    pub fn get(&self, kind: &str) -> Option<&CatalogEntry> {
        self.entries.iter().find(|e| e.spec.kind == kind)
    }

    /// 这个写法（一种事，或 `<范围>.*`）对上的事。
    fn matching<'a>(&'a self, pattern: &'a str) -> impl Iterator<Item = &'a CatalogEntry> + 'a {
        self.entries
            .iter()
            .filter(move |e| match pattern.strip_suffix(".*") {
                Some(prefix) => e
                    .spec
                    .kind
                    .strip_prefix(prefix)
                    .is_some_and(|rest| rest.starts_with('.')),
                None => e.spec.kind == pattern,
            })
    }

    /// 给她看的一件事的数据（`game.state` 的最近几件事）：去掉这种事登记为不给她看的字段（[`EventSpec::private`]）。
    /// 自动化拿到的是事件本身，不经过这里。
    pub fn data_for_her(&self, kind: &str, data: &Map<String, Value>) -> Map<String, Value> {
        let private = self
            .get(kind)
            .map_or(&[][..], |e| e.spec.private.as_slice());
        data.iter()
            .filter(|(field, _)| !private.contains(field))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    /// 认不认得这个写法（插件关着也认得）。
    pub fn knows(&self, pattern: &str) -> bool {
        self.matching(pattern).next().is_some()
    }

    /// 触发里那几种事的 `data` 字段（camelCase，去重，按出现的先后）：自动化里和它们同名的参数会自动填上
    /// （[`crate::automation::library::event_args`]）。
    pub fn data_fields<'a>(&'a self, trigger: &'a EventTrigger) -> Vec<&'a str> {
        let mut out: Vec<&str> = Vec::new();
        for pattern in &trigger.event {
            for entry in self.matching(pattern) {
                for field in entry.spec.data.keys() {
                    if !out.contains(&field.as_str()) {
                        out.push(field);
                    }
                }
            }
        }
        out
    }

    /// `game.state` 里的事件目录：按适配器分组（本体的在前）；会发生的列在 `events`（类型、说明、`data` 字段和
    /// 说明），有现在不会发生的写一句 `off`（为什么、几种、怎么办），不列出来，免得她拿来写自动化。
    pub fn summary(&self) -> Value {
        let mut adapters: Vec<&str> = Vec::new();
        for e in &self.entries {
            if !adapters.contains(&e.adapter.as_str()) {
                adapters.push(&e.adapter);
            }
        }
        let groups: Vec<Value> = adapters
            .into_iter()
            .map(|adapter| {
                let entries: Vec<&CatalogEntry> = self
                    .entries
                    .iter()
                    .filter(|e| e.adapter == adapter)
                    .collect();
                let first = entries[0];
                let games = if adapter == GENERIC_ADAPTER || first.games.is_empty() {
                    "所有有进程名的游戏配置".to_string()
                } else {
                    first.games.join("、")
                };
                let mut group = json!({
                    "adapter": adapter,
                    "games": games,
                    "events": entries.iter().filter(|e| e.off.is_none()).map(|e| json!({
                        "type": e.spec.kind,
                        "label": e.spec.label,
                        "data": e.spec.data.iter().map(|(field, about)| {
                            let about = if e.spec.private.contains(field) {
                                format!("{about}（只交给自动化，这里的最近几件事里不显示）")
                            } else {
                                about.clone()
                            };
                            (field.clone(), Value::String(about))
                        }).collect::<Map<String, Value>>(),
                    })).collect::<Vec<_>>(),
                });
                if let Some((_, name)) = &first.plugin {
                    group["plugin"] = json!(name);
                }
                let off: Vec<&CatalogEntry> = entries
                    .iter()
                    .copied()
                    .filter(|e| e.off.is_some())
                    .collect();
                if let Some(reason) = off.first().and_then(|e| e.off.as_deref()) {
                    let which = if off.len() == entries.len() {
                        "这些事现在都不会发生"
                    } else {
                        "另有几种事现在不会发生"
                    };
                    group["off"] = json!(format!("{reason}：{which}；到「插件」里打开以后才有"));
                }
                group
            })
            .collect();
        Value::Array(groups)
    }

    /// 保存时的提醒（原则 21）：触发里的事现在不会发生时（插件关着、它要的数据开关关着），说清为什么、怎么办。
    /// 触发关着、或者事都会发生时为空。保存照常成功，运行时只是收不到。
    pub fn notes(&self, trigger: &EventTrigger) -> Vec<String> {
        if !trigger.enabled {
            return Vec::new();
        }
        // 原因 → 不会发生的那几种（按出现的顺序）
        let mut off: Vec<(&str, Vec<&str>)> = Vec::new();
        let mut some_on = false;
        for pattern in &trigger.event {
            let hits: Vec<&CatalogEntry> = self.matching(pattern).collect();
            if hits.iter().any(|e| e.off.is_none()) {
                some_on = true;
                continue;
            }
            let Some(reason) = hits.first().and_then(|e| e.off.as_deref()) else {
                continue; // 不认得的写法：保存校验会报
            };
            let what = match hits.as_slice() {
                [one] if !pattern.ends_with(".*") => one.spec.label.as_str(),
                _ => pattern.as_str(),
            };
            match off.iter_mut().find(|(r, _)| *r == reason) {
                Some((_, list)) => list.push(what),
                None => off.push((reason, vec![what])),
            }
        }
        off.into_iter()
            .map(|(reason, list)| {
                if some_on {
                    format!(
                        "{reason}，其中“{}”不会发生；到「插件」里打开以后照常触发",
                        list.join("”“")
                    )
                } else {
                    format!("{reason}，这个触发不会发生；到「插件」里打开以后照常触发")
                }
            })
            .collect()
    }
}
