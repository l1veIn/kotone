//! 自己加的热键（ADR 040 第 3 期，D5）：热键表 `hotkeys.bindings` 里不绑命令、写着“要做什么”的条目。
//!
//! 四种，每条都有自己的 `id`（行 id 是 `custom.<id>`）和可选的 `label`（玩家起的名字）：
//! - 跟她说一句（快捷指令）：`{"key", "say": "帮我看看队友说了什么", "id", "label"?}`；
//! - 运行自动化：`{"key", "tool": "automation.run", "args": {"id": "auto-1", "args": {…}}, "id", "label"?}`
//!   ——就是调工具 `automation.run`（一种东西一种登记，原则 15）；
//! - 调一个工具：`{"key", "tool", "args"?, "id", "label"?, "confirmed"?}`；
//! - 打开一个页面：`{"key", "page": "ui://…", "id", "label"?}`。
//!
//! **绑的时候问权限（D9）**：调工具（运行自动化也是）照这个工具在她那张权限表里的规则判一次
//! （`kotone_agent::permissions::binding_decision`）：允许的不问；要问的，玩家在「设置 → 热键」确认一次，确认时这个工具的
//! 副作用记进这一条的 `confirmed`（[`Settings::confirm_custom`]），按键时不再问；工具后来多了没确认过的副作用就要重新确认
//! （照浏览器扩展更新多要权限时先停用）。**`confirmed` 只有那个确认命令能写**：补丁（界面的通用保存、她和外部 Agent 的
//! `settings.update`、命令行）写进来的，只要这一条做的事（工具、参数）和改之前不一样、或者确认是这次才写上的，就抹掉
//! （[`Settings::drop_unconfirmed`]，两条补丁的路都调）。第三方插件的配置变了，它的工具的确认清掉（信任也一起作废）。
//! 现在能不能按由宿主照工具目录和权限算（[`CustomState`]，本体不认识工具）。
//!
//! **快捷指令的旧字段 `assistant.companion.quickMessages` 是镜像**（照第 2 期别的热键旧字段，原则 19）：表里前
//! [`MAX_QUICK_MESSAGES`] 条“说一句 / 运行已保存的自动化”每次保存都写回去（只写过得了旧版本 `CompanionConfig::validate`
//! 的），旧版本照旧能用；第 9 条以后、工具、页面不进镜像。ADR 040 以前的版本保存时丢掉整个 `hotkeys`，再装回新版本就
//! 从镜像重新搬，镜像以外的那些就没了。
//! - **读配置**（[`Settings::adopt_quick_messages_on_load`]）：旧字段里有、表里哪儿都没有这个 id 的快捷指令加进表——
//!   第 2 期的配置、ADR 040 以前的配置都靠它搬进来；表里已有的以表为准（和别的热键“有表以表为准”一样）。
//! - **一次修改里**（[`Settings::adopt_quick_message_edits`]，`reconcile_hotkeys` 在表没动时调）：旧字段变了就是旧写法
//!   （`settings.update` 整组替换快捷指令、还没改的界面、代码直接改了内存字段）：按 id 比，加的加、删的删、改的改，
//!   顺序照旧字段。补丁同时改了表时以表为准。
//! - 没有 `id` 的条目（她或外部 Agent 手写的）在下一次修改时补上（[`Settings::number_custom_entries`]）。

use std::collections::HashSet;

use serde_json::{Map, Value};

use crate::assistant::{QuickMessage, MAX_QUICK_MESSAGES};
use crate::hotkey::quick_message_slot;
use crate::hotkey_bindings::{Binding, HotkeysConfig};
use crate::settings::Settings;

/// 运行已保存的自动化的工具。
pub const TOOL_AUTOMATION_RUN: &str = "automation.run";
/// 自己加的那一行在热键表里的 id 前缀：`custom.<条目 id>`。
pub const ROW_PREFIX: &str = "custom.";
/// 自己加的条目最多几条（只防配置文件失控；快捷指令的旧写法照旧最多 8 条）。
pub const MAX_CUSTOM: usize = 100;
/// 跟她说的一句最多几个字（和快捷指令一样）。
pub const MAX_SAY_CHARS: usize = 200;
/// 名字最多几个字（和快捷指令一样）。
pub const MAX_LABEL_CHARS: usize = 40;
/// 条目 id、自动化 id 最长（和快捷指令一样）。
pub const MAX_ID_LEN: usize = 64;
/// 运行自动化的入参最大（字节，和快捷指令一样）。
pub const MAX_ARGS_BYTES: usize = 4_096;

/// 自己加的一条要做的事。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Action<'a> {
    /// 跟她说一句（快捷指令）。
    Say(&'a str),
    /// 运行一个已保存的自动化（`automation.run {id, args}`）。
    Automation {
        id: &'a str,
        args: Option<&'a Map<String, Value>>,
    },
    /// 调一个工具，带固定参数。
    Tool {
        tool: &'a str,
        args: Option<&'a Value>,
    },
    /// 打开一个页面。
    Page(&'a str),
}

impl Action<'_> {
    /// 给界面和 `hotkeys.status` 的种类名。
    pub fn kind(&self) -> &'static str {
        match self {
            Action::Say(_) => "say",
            Action::Automation { .. } => "automation",
            Action::Tool { .. } => "tool",
            Action::Page(_) => "page",
        }
    }
}

impl<'a> Action<'a> {
    /// 调的是哪个工具、带什么参数（运行自动化就是 `automation.run {id, args}`）；说一句、打开页面为 None。
    pub fn tool_call(&self) -> Option<(&'a str, Value)> {
        match self {
            Action::Automation { id, args } => {
                let mut call = Map::new();
                call.insert("id".into(), (*id).into());
                if let Some(args) = args {
                    call.insert("args".into(), Value::Object((*args).clone()));
                }
                Some((TOOL_AUTOMATION_RUN, Value::Object(call)))
            }
            Action::Tool { tool, args } => Some((
                *tool,
                args.cloned().unwrap_or_else(|| Value::Object(Map::new())),
            )),
            Action::Say(_) | Action::Page(_) => None,
        }
    }
}

/// 自己加的调工具、打开页面现在能不能按（宿主照工具目录、插件开关和权限算；本体不认识工具）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CustomState {
    /// 能按。
    Ready,
    /// 插件关着：不登记、也不占键（和插件的命令一样，打开插件就回来）。
    Off,
    /// 要玩家在「设置 → 热键」确认一次（D9）：确认之前不登记。里面是这个工具会做什么（权限的原因）。
    NeedsConfirm(String),
    /// 按不了（找不到、不能绑、权限里设成了禁止、只读模式）：不登记，里面是原因和下一步。
    Blocked(String),
}

/// `automation.run` 的参数是不是“运行一个已保存的自动化”：只有 `id`（非空的字）和可选的 `args`（对象）。
fn saved_automation(args: Option<&Value>) -> Option<(&str, Option<&Map<String, Value>>)> {
    let object = args?.as_object()?;
    if object.keys().any(|k| k != "id" && k != "args") {
        return None;
    }
    let id = object
        .get("id")?
        .as_str()
        .map(str::trim)
        .filter(|id| !id.is_empty())?;
    let inner = match object.get("args") {
        None => None,
        Some(Value::Object(map)) => Some(map),
        Some(_) => return None,
    };
    Some((id, inner))
}

impl Binding {
    /// 跟她说的那句（`say`）。
    pub fn say(&self) -> Option<&str> {
        self.0.get("say").and_then(Value::as_str)
    }

    /// 条目 id（自己加的才有；空串算没有）。
    pub fn id(&self) -> Option<&str> {
        self.0
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.trim().is_empty())
    }

    /// 玩家起的名字（没起为空）。
    pub fn label(&self) -> &str {
        self.0.get("label").and_then(Value::as_str).unwrap_or("")
    }

    /// 绑的时候玩家确认过的副作用（D9）；没确认过为 None（确认过、工具没有副作用的是空的）。
    pub fn confirmed(&self) -> Option<Vec<&str>> {
        self.0
            .get("confirmed")?
            .as_array()?
            .iter()
            .map(Value::as_str)
            .collect()
    }

    /// 这一条做的事：去掉键、名字、确认以后的样子（命令、工具 + 参数、页面、说的话，还有 id）。
    fn what(&self) -> Value {
        match &self.0 {
            Value::Object(object) => Value::Object(
                object
                    .iter()
                    .filter(|(k, _)| !matches!(k.as_str(), "key" | "label" | "confirmed"))
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
            ),
            other => other.clone(),
        }
    }

    /// 自己加的条目要做的事；绑命令的、读不懂的（种类不止一个、值的类型不对、不是对象）为 None。
    pub fn action(&self) -> Option<Action<'_>> {
        let object = self.0.as_object()?;
        if object.contains_key("command") {
            return None;
        }
        let kinds = ["say", "tool", "page"]
            .iter()
            .filter(|k| object.contains_key(**k))
            .count();
        if kinds != 1 {
            return None;
        }
        if let Some(text) = object.get("say") {
            return text.as_str().map(Action::Say);
        }
        if let Some(page) = object.get("page") {
            return page.as_str().map(Action::Page);
        }
        let tool = object.get("tool")?.as_str()?;
        let args = object.get("args");
        if tool == TOOL_AUTOMATION_RUN {
            if let Some((id, args)) = saved_automation(args) {
                return Some(Action::Automation { id, args });
            }
        }
        Some(Action::Tool { tool, args })
    }
}

/// 表里自己加的条目，按表的顺序：(在表里的下标, 条目, 要做的事)。
pub fn custom_entries(
    hotkeys: &HotkeysConfig,
) -> impl Iterator<Item = (usize, &Binding, Action<'_>)> {
    hotkeys
        .bindings
        .iter()
        .enumerate()
        .filter_map(|(index, binding)| binding.action().map(|action| (index, binding, action)))
}

/// 自己加的那一行的 id：`custom.<条目 id>`；还没有 id 的（下一次修改时补上）是 `custom.#<下标>`。
pub fn row_id(index: usize, binding: &Binding) -> String {
    match binding.id() {
        Some(id) => format!("{ROW_PREFIX}{id}"),
        None => format!("{ROW_PREFIX}#{index}"),
    }
}

/// 行 id（`custom.<id>` / `custom.#<下标>`）→ 表里那一条的下标。
pub fn find_row(hotkeys: &HotkeysConfig, row: &str) -> Option<usize> {
    let rest = row.strip_prefix(ROW_PREFIX)?;
    if let Some(index) = rest.strip_prefix('#') {
        let index: usize = index.parse().ok()?;
        return hotkeys
            .bindings
            .get(index)
            .filter(|b| b.action().is_some() && b.id().is_none())
            .map(|_| index);
    }
    custom_entries(hotkeys)
        .find(|(_, binding, _)| binding.id() == Some(rest))
        .map(|(index, _, _)| index)
}

/// 外面写的 id → 自己加的那一行的 id：`custom.<id>` 原样；`quick-message-n`（第 1、2 期的写法，`hotkeys.conflicts`
/// 的 `except`、`set_hotkey`）是镜像的第 n 条。别的为 None。
pub fn resolve_row(settings: &Settings, id: &str) -> Option<String> {
    if id.starts_with(ROW_PREFIX) {
        return Some(id.to_string());
    }
    let slot = quick_message_slot(id)?;
    quick_message_mirror(&settings.hotkeys)
        .get(slot)
        .map(|message| format!("{ROW_PREFIX}{}", message.id))
}

/// 登记热键要看的那部分：每条的键、是哪一条、什么种类，加上 `hotkeys` 下面别的东西。改了快捷指令跟她说的话、
/// 自动化的入参、名字不用重新登记（按下时才读），加一条没绑键的也不用，这两份就一样。
pub fn registration_view(hotkeys: &HotkeysConfig) -> Value {
    let bindings: Vec<Value> = hotkeys
        .bindings
        .iter()
        .enumerate()
        .filter_map(|(index, binding)| match binding.action() {
            Some(_) if binding.key().trim().is_empty() => None,
            Some(action) => Some(serde_json::json!({
                "key": binding.key(),
                "row": row_id(index, binding),
                "kind": action.kind(),
                // 确认了 / 确认作废了：从不登记变成登记，或者反过来
                "confirmed": binding.confirmed(),
            })),
            None => Some(binding.0.clone()),
        })
        .collect();
    serde_json::json!({"bindings": bindings, "extra": hotkeys.extra})
}

/// 这一条能不能写成一条快捷指令（旧字段的一条）：说一句、运行已保存的自动化，而且过得了旧版本的校验。
pub fn quick_message_of(binding: &Binding) -> Option<QuickMessage> {
    let id = binding.id()?;
    if id.trim().len() > MAX_ID_LEN {
        return None;
    }
    let label = binding.label();
    if label.chars().count() > MAX_LABEL_CHARS {
        return None;
    }
    let mut message = QuickMessage {
        id: id.to_string(),
        label: label.to_string(),
        hotkey: binding.key().to_string(),
        ..Default::default()
    };
    match binding.action()? {
        Action::Say(text) => {
            let count = text.trim().chars().count();
            if count == 0 || count > MAX_SAY_CHARS {
                return None;
            }
            message.text = text.to_string();
        }
        Action::Automation { id, args } => {
            let args = args.cloned().unwrap_or_default();
            if id.len() > MAX_ID_LEN
                || serde_json::to_vec(&args).map_or(true, |b| b.len() > MAX_ARGS_BYTES)
            {
                return None;
            }
            message.automation_id = id.to_string();
            message.args = args;
        }
        Action::Tool { .. } | Action::Page(_) => return None,
    }
    Some(message)
}

/// 一条快捷指令 → 表里的一条。
pub fn binding_of(message: &QuickMessage) -> Binding {
    let mut object = Map::new();
    object.insert("key".into(), message.hotkey.clone().into());
    if message.runs_automation() {
        let mut args = Map::new();
        args.insert("id".into(), message.automation_id.trim().into());
        if !message.args.is_empty() {
            args.insert("args".into(), Value::Object(message.args.clone()));
        }
        object.insert("tool".into(), TOOL_AUTOMATION_RUN.into());
        object.insert("args".into(), Value::Object(args));
    } else {
        object.insert("say".into(), message.text.clone().into());
    }
    object.insert("id".into(), message.id.clone().into());
    if !message.label.is_empty() {
        object.insert("label".into(), message.label.clone().into());
    }
    Binding(Value::Object(object))
}

/// 一条快捷指令照表的写法规范一下（运行自动化的那种不留 `text`；自动化 id 去掉空格）；写不进镜像的为 None。
fn canonical(message: &QuickMessage) -> Option<QuickMessage> {
    quick_message_of(&binding_of(message))
}

/// 全部能写成快捷指令的条目（`companion.status` 的 `quickMessages`、`companion.ask {quickMessageId}` 用，不止 8 条）。
/// 同一个 id 只算第一条。
pub fn quick_messages(hotkeys: &HotkeysConfig) -> Vec<QuickMessage> {
    let mut seen = HashSet::new();
    custom_entries(hotkeys)
        .filter_map(|(_, binding, _)| quick_message_of(binding))
        .filter(|message| seen.insert(message.id.trim().to_string()))
        .collect()
}

/// 镜像：表里前 8 条能写成快捷指令的（旧字段 `assistant.companion.quickMessages`）。
pub fn quick_message_mirror(hotkeys: &HotkeysConfig) -> Vec<QuickMessage> {
    let mut all = quick_messages(hotkeys);
    all.truncate(MAX_QUICK_MESSAGES);
    all
}

/// 没用过的条目 id（`qm-<n>`，和界面新加的快捷指令一样以 `qm-` 开头；同一份输入每次得到同一个）。
fn fresh_id(taken: &HashSet<String>) -> String {
    (1..)
        .map(|n| format!("qm-{n}"))
        .find(|id| !taken.contains(id))
        .expect("总有没用过的 id")
}

/// 旧字段的快捷指令，没有 id 或 id 重复的补一个（旧版本的校验不让这样存，只有手改的配置、直接改内存的代码会有）。
fn with_ids(messages: &[QuickMessage], taken: &HashSet<String>) -> Vec<QuickMessage> {
    let mut taken = taken.clone();
    let mut seen = HashSet::new();
    messages
        .iter()
        .map(|message| {
            let mut message = message.clone();
            let id = message.id.trim().to_string();
            if id.is_empty() || !seen.insert(id) {
                message.id = fresh_id(&taken);
                seen.insert(message.id.clone());
            }
            taken.insert(message.id.clone());
            message
        })
        .collect()
}

fn taken_ids(hotkeys: &HotkeysConfig) -> HashSet<String> {
    custom_entries(hotkeys)
        .filter_map(|(_, binding, _)| binding.id().map(|id| id.trim().to_string()))
        .collect()
}

/// 新加一条自己加的热键用的 id（表里没用过的 `qm-<n>`；「设置 → 热键」的“加一个”）。
pub fn new_id(hotkeys: &HotkeysConfig) -> String {
    fresh_id(&taken_ids(hotkeys))
}

/// 换掉这一条要做的事（键、话、自动化、名字），条目上别的字段（更新的版本写的）原样留着。
fn replace_content(binding: &mut Binding, message: &QuickMessage) {
    let Value::Object(fresh) = binding_of(message).0 else {
        return;
    };
    match binding.0.as_object_mut() {
        Some(object) => {
            for field in ["key", "say", "tool", "args", "page", "label", "confirmed"] {
                object.remove(field);
            }
            object.extend(fresh);
        }
        None => binding.0 = Value::Object(fresh),
    }
}

impl Settings {
    /// 读配置时：旧字段里有、表里哪儿都没有这个 id 的快捷指令加进表（第 2 期的配置、ADR 040 以前的配置搬进来）。
    /// 表里已有的以表为准。之后照常 [`Settings::sync_hotkey_mirrors`] 写回镜像。
    pub fn adopt_quick_messages_on_load(&mut self) {
        let taken = taken_ids(&self.hotkeys);
        let incoming = with_ids(&self.assistant.companion.quick_messages, &taken);
        for message in incoming {
            if !taken.contains(message.id.trim()) {
                self.hotkeys.bindings.push(binding_of(&message));
            }
        }
    }

    /// 一次修改里表没动、旧字段的快捷指令变了（旧写法）：按 id 搬进表——旧字段里没了的从表里删，新的加进表
    /// （插在镜像那几条的后面，免得被挤出镜像），改了的照改，镜像那几条的顺序照旧字段。镜像以外的条目不动。
    pub fn adopt_quick_message_edits(&mut self) {
        let expected = quick_message_mirror(&self.hotkeys);
        let actual = with_ids(
            &self.assistant.companion.quick_messages,
            &taken_ids(&self.hotkeys),
        );
        let same = expected.len() == actual.len()
            && expected
                .iter()
                .zip(&actual)
                .all(|(e, a)| canonical(a).as_ref() == Some(e));
        if same {
            return;
        }
        let wanted: Vec<String> = actual.iter().map(|m| m.id.trim().to_string()).collect();
        let is_wanted = |b: &Binding| {
            b.action().is_some()
                && b.id()
                    .is_some_and(|id| wanted.iter().any(|w| w == id.trim()))
        };
        let bindings = &mut self.hotkeys.bindings;
        // 删：镜像里有、旧字段里没了
        let removed: Vec<&str> = expected
            .iter()
            .map(|m| m.id.trim())
            .filter(|id| !wanted.iter().any(|w| w == id))
            .collect();
        bindings.retain(|b| {
            !(b.action().is_some() && b.id().is_some_and(|id| removed.contains(&id.trim())))
        });
        // 改 / 记下新的
        let mut added = Vec::new();
        for message in &actual {
            let id = message.id.trim();
            match bindings
                .iter_mut()
                .find(|b| b.action().is_some() && b.id().is_some_and(|i| i.trim() == id))
            {
                Some(binding) => {
                    if quick_message_of(binding) != canonical(message) {
                        replace_content(binding, message);
                    }
                }
                None => added.push(binding_of(message)),
            }
        }
        let at = bindings
            .iter()
            .rposition(is_wanted)
            .map_or(bindings.len(), |i| i + 1);
        bindings.splice(at..at, added);
        // 顺序照旧字段：这几条占着的位置，按旧字段的顺序重新放
        let mut slots: Vec<usize> = Vec::new();
        let mut placed: Vec<Binding> = Vec::new();
        for id in &wanted {
            if let Some(index) = bindings
                .iter()
                .position(|b| b.action().is_some() && b.id().is_some_and(|i| i.trim() == id))
            {
                if !slots.contains(&index) {
                    slots.push(index);
                    placed.push(bindings[index].clone());
                }
            }
        }
        slots.sort_unstable();
        for (slot, binding) in slots.into_iter().zip(placed) {
            bindings[slot] = binding;
        }
    }

    /// 自己加的条目没有 id 的、id 重复的补一个（她或外部 Agent 手写的条目）。在旧写法搬完以后调（先补 id 的话，
    /// 刚补上的会被当成“旧字段里没有、删掉”）。
    pub fn number_custom_entries(&mut self) {
        let mut taken = taken_ids(&self.hotkeys);
        let mut seen = HashSet::new();
        for binding in &mut self.hotkeys.bindings {
            if binding.action().is_none() {
                continue;
            }
            let ok = binding
                .id()
                .is_some_and(|id| seen.insert(id.trim().to_string()));
            if ok {
                continue;
            }
            let id = fresh_id(&taken);
            taken.insert(id.clone());
            seen.insert(id.clone());
            if let Some(object) = binding.0.as_object_mut() {
                object.insert("id".into(), id.into());
            }
        }
    }

    /// 补丁写进来的确认不算（D9，见模块说明）：这次以后带 `confirmed` 的条目，改之前要有一条做的事一样、确认也一样的，
    /// 不然抹掉。界面的通用保存、她和外部 Agent 的 `settings.update`、命令行 `config set` 在对齐以后调；
    /// 「设置 → 热键」的确认命令在这之后才写确认（[`Settings::confirm_custom`]）。
    pub fn drop_unconfirmed(&mut self, previous: &Settings) {
        let before: Vec<(Value, Value)> = previous
            .hotkeys
            .bindings
            .iter()
            .filter_map(|b| b.0.get("confirmed").map(|c| (b.what(), c.clone())))
            .collect();
        for binding in &mut self.hotkeys.bindings {
            let Some(confirmed) = binding.0.get("confirmed").cloned() else {
                continue;
            };
            let what = binding.what();
            if before.iter().any(|(w, c)| *w == what && *c == confirmed) {
                continue;
            }
            if let Some(object) = binding.0.as_object_mut() {
                object.remove("confirmed");
            }
        }
    }

    /// 玩家在「设置 → 热键」确认了这一条（`row` 是 `custom.<id>`）：记下确认时这个工具的副作用（D9）。
    /// 只给确认命令用（它先查过这个工具现在确实要问）；补丁写进来的确认会被 [`Settings::drop_unconfirmed`] 抹掉。
    pub fn confirm_custom(&mut self, row: &str, effects: &[String]) -> Result<(), String> {
        let index =
            find_row(&self.hotkeys, row).ok_or("这条自己加的热键已经没有了，刷新一下再确认")?;
        let binding = &mut self.hotkeys.bindings[index];
        if binding.action().and_then(|a| a.tool_call()).is_none() {
            return Err("只有调工具、运行自动化的热键要确认".into());
        }
        if let Some(object) = binding.0.as_object_mut() {
            object.insert("confirmed".into(), effects.to_vec().into());
        }
        Ok(())
    }

    /// 这些工具（名字以 `prefix` 开头，例如第三方插件 `everything.`）的热键确认作废（插件的配置变了、删了）。
    /// 返回有没有改。
    pub fn forget_confirmations(&mut self, prefix: &str) -> bool {
        let mut changed = false;
        for binding in &mut self.hotkeys.bindings {
            let calls_it = binding
                .action()
                .and_then(|a| a.tool_call().map(|(tool, _)| tool.starts_with(prefix)))
                .unwrap_or(false);
            if calls_it {
                if let Some(object) = binding.0.as_object_mut() {
                    changed |= object.remove("confirmed").is_some();
                }
            }
        }
        changed
    }

    /// 把表写回快捷指令的旧字段（镜像）。
    pub(crate) fn sync_quick_message_mirror(&mut self) {
        self.assistant.companion.quick_messages = quick_message_mirror(&self.hotkeys);
    }

    /// 改自己加的那一条的键（`row` 是 `custom.<id>` 或 `quick-message-n`）；空 = 不绑键（条目留着）。
    pub fn set_custom_key(&mut self, row: &str, key: &str) -> Result<(), String> {
        let resolved = resolve_row(self, row).ok_or_else(|| format!("「{row}」在这里改不了"))?;
        let index =
            find_row(&self.hotkeys, &resolved).ok_or("这条自己加的热键已经没有了，刷新一下再改")?;
        if let Some(object) = self.hotkeys.bindings[index].0.as_object_mut() {
            object.insert("key".into(), key.trim().into());
        }
        Ok(())
    }
}

impl HotkeysConfig {
    /// 自己加的条目写得对不对（保存前查，说人话、说下一步）。
    pub fn validate_custom(&self) -> Result<(), String> {
        let mut count = 0;
        for (_, binding, action) in custom_entries(self) {
            count += 1;
            if binding.id().is_some_and(|id| id.trim().len() > MAX_ID_LEN) {
                return Err("自己加的热键的编号太长（最多 64 个字符）".into());
            }
            if binding.label().chars().count() > MAX_LABEL_CHARS {
                return Err(format!(
                    "热键的名字不能超过 {MAX_LABEL_CHARS} 字：「{}」",
                    binding.label()
                ));
            }
            match action {
                Action::Say(text) => {
                    let count = text.trim().chars().count();
                    if count == 0 || count > MAX_SAY_CHARS {
                        return Err(format!(
                            "快捷指令跟她说的话要 1 到 {MAX_SAY_CHARS} 字（键 {}）",
                            if binding.key().is_empty() {
                                "没设"
                            } else {
                                binding.key()
                            }
                        ));
                    }
                }
                Action::Automation { id, args } => {
                    if id.len() > MAX_ID_LEN {
                        return Err("要运行的自动化编号太长（最多 64 个字符）".into());
                    }
                    if args.is_some_and(|a| {
                        serde_json::to_vec(a).map_or(true, |b| b.len() > MAX_ARGS_BYTES)
                    }) {
                        return Err("运行自动化的入参太大（最多 4 KB）".into());
                    }
                }
                Action::Tool { .. } | Action::Page(_) => {}
            }
        }
        if count > MAX_CUSTOM {
            return Err(format!(
                "自己加的热键最多 {MAX_CUSTOM} 条，先在「设置 → 热键」删掉几条不用的"
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "hotkey_custom_tests.rs"]
mod tests;
