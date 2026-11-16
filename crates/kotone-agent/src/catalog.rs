//! 内置 Agent 能用的工具清单（ADR 014 §3、ADR 015 §1 第 3 层）。
//!
//! 来源有三种，合成一张表：
//! - **工具服务**：`system.capabilities` 里的工具，与外部 Agent 同一份实现；
//!   给外部 Agent 的通道类工具（`agent.*`、`event.read`、`assistant.submit`）和旧陪玩流程
//!   （`companion.ask` / `companion.check`，会再调一次模型）不交给内置 Agent；
//! - **内置动作**：播报（`reply.say`）、提醒（`reminder.create`）、发到游戏聊天
//!   （`game_chat.send`，替代 `companion.type`，会写进执行记录）和查找工具（`tools.find`）；
//! - **已保存的自动化**：每个一个工具 `saved.<id>`（最多 [`MAX_SAVED_TOOLS`] 个）。
//!
//! 八十多个工具不全量交给模型：[`CORE_TOOLS`] 与已保存的自动化始终提供，其余按分组
//! 由模型先用 `tools.find` 查，查到的才在本段对话里可用。

use kotone_core::automation::library::{Automation, AutomationBody, ParamKind};
use serde_json::{json, Map, Value};

pub const TOOL_FIND: &str = "tools.find";
pub const TOOL_SAY: &str = "reply.say";
pub const TOOL_REMIND: &str = "reminder.create";
pub const TOOL_GAME_CHAT: &str = "game_chat.send";
/// 已保存自动化工具名的前缀。
pub const SAVED_PREFIX: &str = "saved.";
/// 交给模型的已保存自动化最多几个（ADR 014“已确认”）。
pub const MAX_SAVED_TOOLS: usize = 30;

/// 游戏内快速模式只提供这些（另加已保存的自动化；不提供 tools.find），
/// 让模型少想几步、快点出结果（ADR 014 §4）。
pub const GAME_TOOLS: &[&str] = &[
    TOOL_SAY,
    TOOL_REMIND,
    TOOL_GAME_CHAT,
    "companion.read_chat",
    "automation.list",
    "automation.run",
    "automation.runs",
    "automation.stop",
];

/// 始终提供的工具（另加已保存的自动化）。
pub const CORE_TOOLS: &[&str] = &[
    TOOL_FIND,
    TOOL_SAY,
    TOOL_REMIND,
    TOOL_GAME_CHAT,
    "companion.read_chat",
    "window.active",
    "game.list",
    "game.focus",
    "automation.list",
    "automation.get",
    "automation.run",
    "automation.runs",
    "automation.stop",
    "automation.save",
];

/// 不交给内置 Agent 的工具服务工具。
pub const EXCLUDED: &[&str] = &[
    // 给外部 Agent 的通道
    "event.read",
    "agent.requests",
    "agent.request.claim",
    "agent.reply",
    "assistant.submit",
    // 旧的单次陪玩流程（会再调一次模型）；打字由 game_chat.send 代替
    "companion.ask",
    "companion.check",
    "companion.type",
    // 由 tools.find 代替；PNG 正文会塞满上下文
    "system.capabilities",
    "artifact.get",
];

/// 结果里含屏幕文字、游戏聊天等不受用户控制内容的工具：结果按不可信数据交给模型。
pub const UNTRUSTED_RESULTS: &[&str] = &[
    "companion.read_chat",
    "ocr.recognize",
    "chat.track.ingest",
    "chat.track.read",
    "window.list",
    "window.active",
    "game.list",
    "game.focus",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// 工具服务（`ToolService::call`）。
    Service,
    /// 内置动作（经 `BuiltinHost`）。
    Builtin,
    /// `tools.find`。
    Meta,
    /// 已保存的自动化（经 `automation.run`）。
    Saved { id: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolEntry {
    pub name: String,
    pub description: String,
    /// 交给模型的参数 JSON Schema（顶层总是 object）。
    pub parameters: Value,
    pub side_effects: Vec<String>,
    pub group: &'static str,
    pub source: Source,
    /// 结果按不可信数据交给模型。
    pub untrusted: bool,
}

/// 分组：(id, 中文名, 查找时也匹配的关键词)。
pub const GROUPS: &[(&str, &str, &str)] = &[
    (
        "game",
        "游戏（看聊天、打字、播报、提醒、打开 / 切换 / 关闭游戏）",
        "游戏 聊天 队友 打字 发送 播报 语音 弹幕 提醒 打开 启动 切换 关闭 窗口 chat type say remind launch focus close",
    ),
    (
        "screen",
        "屏幕与窗口（截图、识别文字、窗口）",
        "屏幕 截图 画面 窗口 识别 文字 ocr screen window capture image",
    ),
    (
        "automation",
        "自动化（运行、保存、停止、记录）",
        "自动化 程序 脚本 按键 运行 保存 停止 记录 automation run",
    ),
    (
        "settings",
        "设置（游戏配置、热键、连接、声音、输入）",
        "设置 配置 游戏配置 热键 连接 密钥 麦克风 输入法 开机 管理员 settings profile hotkey",
    ),
    (
        "models",
        "模型（语音识别、文字识别、语音合成）",
        "模型 下载 语音识别 文字识别 语音合成 model download",
    ),
    (
        "app",
        "应用（状态、历史、诊断、退出）",
        "应用 状态 历史 诊断 日志 退出 重启 app history diagnostics",
    ),
];

pub fn group_of(name: &str) -> &'static str {
    let prefix = name.split('.').next().unwrap_or(name);
    match prefix {
        "tools" => "meta",
        "reply" | "reminder" | "game_chat" | "game" | "companion" | "chat" => "game",
        "window" | "screen" | "image" | "artifact" | "ocr" => "screen",
        "automation" | "saved" => "automation",
        "settings" | "profiles" | "hotkeys" | "connections" | "audio" | "input" | "postprocess"
        | "autostart" | "elevation" => "settings",
        "models" | "speech" | "stt" => "models",
        _ => "app",
    }
}

/// 顶层统一成 `{"type": "object", ...}`：工具服务里有 `["object", "null"]` 的写法，
/// 部分服务商只接受 object。
fn normalize_schema(schema: &Value) -> Value {
    let mut schema = match schema {
        Value::Object(map) => map.clone(),
        _ => Map::new(),
    };
    schema.insert("type".into(), json!("object"));
    schema.entry("properties").or_insert_with(|| json!({}));
    Value::Object(schema)
}

fn delivery_schema() -> Value {
    json!({"type": "array", "items": {"enum": ["voice", "danmaku"]}, "maxItems": 2,
           "description": "缺省为用户设置的默认方式"})
}

/// 内置动作与查找工具。
pub fn builtin_entries() -> Vec<ToolEntry> {
    let groups: Vec<&str> = GROUPS.iter().map(|(id, _, _)| *id).collect();
    vec![
        ToolEntry {
            name: TOOL_FIND.into(),
            description: "查找并启用更多工具。只有常用工具一开始就可用；需要改设置、管理模型、看窗口列表、截图识别等时，先用关键词（中文或英文）或分组查找，查到的工具从下一步起可以调用。不带参数时列出分组。".into(),
            parameters: json!({"type": "object", "additionalProperties": false, "properties": {
                "query": {"type": "string", "maxLength": 100, "description": "关键词，如“热键”“下载模型”“window”"},
                "group": {"enum": groups, "description": "列出并启用这一组的全部工具"}
            }}),
            side_effects: Vec::new(),
            group: "meta",
            source: Source::Meta,
            untrusted: false,
        },
        ToolEntry {
            name: TOOL_SAY.into(),
            description: "现在就用语音和 / 或弹幕对玩家说一句话（玩家在游戏里、看不到聊天窗口时用）。简短口语，不加表情和 Markdown。".into(),
            parameters: json!({"type": "object", "additionalProperties": false, "required": ["text"], "properties": {
                "text": {"type": "string", "minLength": 1, "maxLength": 300},
                "delivery": delivery_schema()
            }}),
            side_effects: vec!["speaks-or-shows-text".into()],
            group: "game",
            source: Source::Builtin,
            untrusted: false,
        },
        ToolEntry {
            name: TOOL_REMIND.into(),
            description: "过一段时间用语音和 / 或弹幕提醒玩家（如“5 分钟后提醒我买装备”）。会出现在「自动化 → 进行中」，可以用 automation.runs 查看、automation.stop 取消。".into(),
            parameters: json!({"type": "object", "additionalProperties": false, "required": ["delaySeconds", "text"], "properties": {
                "delaySeconds": {"type": "integer", "minimum": 1, "maximum": 604800},
                "text": {"type": "string", "minLength": 1, "maxLength": 300, "description": "到时说的话，写成对玩家本人说的口吻"},
                "delivery": delivery_schema()
            }}),
            side_effects: vec!["creates-reminder".into()],
            group: "game",
            source: Source::Builtin,
            untrusted: false,
        },
        ToolEntry {
            name: TOOL_GAME_CHAT.into(),
            description: "替玩家把一句话打进当前游戏的聊天框（按游戏配置的开聊天框键、频道与输入方式），默认按发送键（跟随用户的“自动发送”设置）。只在玩家明确要你替他说话或发送翻译时用；写成玩家本人的口吻，不加署名和引号。发出的内容会记在「自动化 → 记录」里。".into(),
            parameters: json!({"type": "object", "additionalProperties": false, "required": ["text"], "properties": {
                "text": {"type": "string", "minLength": 1, "maxLength": 200, "description": "单行，不含换行"},
                "send": {"type": "boolean", "description": "缺省跟随用户设置；false 只打进聊天框不发送"}
            }}),
            side_effects: vec!["input".into(), "records-run".into()],
            group: "game",
            source: Source::Builtin,
            untrusted: false,
        },
    ]
}

/// 工具服务 `system.capabilities` 的 `tools` 数组 → 条目（去掉 [`EXCLUDED`]）。
pub fn service_entries(specs: &[Value]) -> Vec<ToolEntry> {
    specs
        .iter()
        .filter_map(|spec| {
            let name = spec.get("name")?.as_str()?;
            if EXCLUDED.contains(&name) {
                return None;
            }
            let mut description = spec
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            if spec.get("ready").and_then(Value::as_bool) == Some(false) {
                let reason = spec
                    .get("notReadyReason")
                    .and_then(Value::as_str)
                    .unwrap_or("未就绪");
                description = format!("（当前不可用：{reason}）{description}");
            }
            let side_effects = spec
                .get("sideEffects")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            Some(ToolEntry {
                name: name.to_string(),
                description,
                parameters: normalize_schema(spec.get("inputSchema").unwrap_or(&Value::Null)),
                side_effects,
                group: group_of(name),
                source: Source::Service,
                untrusted: UNTRUSTED_RESULTS.contains(&name),
            })
        })
        .collect()
}

/// 已保存的自动化 → 工具。有必填的图片参数（模型给不出）时返回 None。
pub fn saved_entry(automation: &Automation) -> Option<ToolEntry> {
    let mut properties = Map::new();
    let mut required = Vec::new();
    for param in &automation.params {
        let mut schema = match &param.kind {
            ParamKind::Number { min, max, integer } => {
                let mut s = json!({"type": if *integer { "integer" } else { "number" }});
                if let Some(min) = min {
                    s["minimum"] = json!(min);
                }
                if let Some(max) = max {
                    s["maximum"] = json!(max);
                }
                s
            }
            ParamKind::Text { max_length } => json!({"type": "string", "maxLength": max_length}),
            ParamKind::Boolean => json!({"type": "boolean"}),
            ParamKind::Choice { options } => json!({"type": "string", "enum": options}),
            ParamKind::Key => {
                json!({"type": "string", "description": "按键名，如 Q、Enter、Ctrl+Q"})
            }
            ParamKind::Region => json!({
                "type": "object", "required": ["x", "y", "w", "h"],
                "description": "窗口客户区比例区域（0..1）",
                "properties": {
                    "x": {"type": "number", "minimum": 0, "maximum": 1},
                    "y": {"type": "number", "minimum": 0, "maximum": 1},
                    "w": {"type": "number", "minimum": 0, "maximum": 1},
                    "h": {"type": "number", "minimum": 0, "maximum": 1}
                }
            }),
            ParamKind::Color => json!({
                "type": "object", "required": ["r", "g", "b"],
                "properties": {
                    "r": {"type": "integer", "minimum": 0, "maximum": 255},
                    "g": {"type": "integer", "minimum": 0, "maximum": 255},
                    "b": {"type": "integer", "minimum": 0, "maximum": 255}
                }
            }),
            ParamKind::Image => {
                // 模型给不出图片：没有缺省模板时整个自动化不交给模型。
                param.default.as_ref()?;
                continue;
            }
        };
        let label = if param.label.is_empty() {
            &param.name
        } else {
            &param.label
        };
        let mut text = label.clone();
        if !param.description.is_empty() {
            text = format!("{text}：{}", param.description);
        }
        if let Some(default) = &param.default {
            text = format!("{text}（缺省 {default}）");
        } else {
            required.push(json!(param.name));
        }
        schema["description"] = json!(text);
        properties.insert(param.name.clone(), schema);
    }
    properties.insert(
        "delaySeconds".into(),
        json!({"type": "integer", "minimum": 0, "maximum": 604800, "description": "几秒后开始；缺省立即"}),
    );
    let kind = match automation.action {
        AutomationBody::Speak(_) => "播报",
        AutomationBody::Program(_) => "程序，对当前前台的游戏窗口执行",
    };
    let mut description = format!("运行已保存的自动化「{}」（{kind}）", automation.name);
    if !automation.description.is_empty() {
        description = format!("{description}：{}", automation.description);
    }
    description.push_str("。返回执行记录 id，结束后结果会回到对话里。");
    Some(ToolEntry {
        name: format!("{SAVED_PREFIX}{}", automation.id),
        description,
        parameters: json!({"type": "object", "additionalProperties": false,
                           "properties": properties, "required": required}),
        side_effects: vec!["input".into(), "captures-screen".into(), "may-speak".into()],
        group: "automation",
        source: Source::Saved {
            id: automation.id.clone(),
        },
        untrusted: false,
    })
}

/// 一次请求可用的全部工具。
#[derive(Debug, Clone, Default)]
pub struct Catalog {
    entries: Vec<ToolEntry>,
}

impl Catalog {
    /// `specs`：工具服务的工具说明；`saved`：已允许交给 AI 的已保存自动化（调用方已过滤，
    /// 这里再按上限截断）。
    pub fn build(specs: &[Value], saved: &[Automation]) -> Self {
        let mut entries = builtin_entries();
        entries.extend(service_entries(specs));
        entries.extend(saved.iter().filter_map(saved_entry).take(MAX_SAVED_TOOLS));
        Self { entries }
    }

    pub fn get(&self, name: &str) -> Option<&ToolEntry> {
        self.entries.iter().find(|e| e.name == name)
    }

    pub fn entries(&self) -> &[ToolEntry] {
        &self.entries
    }

    /// 是否始终提供（核心工具与已保存的自动化）。
    pub fn is_core(entry: &ToolEntry) -> bool {
        CORE_TOOLS.contains(&entry.name.as_str()) || matches!(entry.source, Source::Saved { .. })
    }

    /// 按关键词或分组查找（不含始终提供的工具）。关键词按空白拆开，每个词都要在
    /// 名字、说明或分组关键词里出现。
    pub fn find(&self, query: Option<&str>, group: Option<&str>) -> Vec<&ToolEntry> {
        let words: Vec<String> = query
            .unwrap_or("")
            .split_whitespace()
            .map(str::to_lowercase)
            .collect();
        self.entries
            .iter()
            .filter(|e| !Self::is_core(e))
            .filter(|e| group.is_none_or(|g| e.group == g))
            .filter(|e| {
                let keywords = GROUPS
                    .iter()
                    .find(|(id, _, _)| *id == e.group)
                    .map(|(_, label, words)| format!("{label} {words}"))
                    .unwrap_or_default();
                let haystack = format!("{} {} {}", e.name, e.description, keywords).to_lowercase();
                words.iter().all(|w| haystack.contains(w.as_str()))
            })
            .collect()
    }
}
