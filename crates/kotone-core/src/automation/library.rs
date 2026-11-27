//! 已保存的自动化（定义，ADR 013 §2）：名字、说明、入参声明、程序、创建者。存在
//! `~/.kotone/automations.json`。运行时先按声明校验入参（[`validate_args`]），再由调度器生成
//! 执行记录；执行记录保存源码快照，之后修改或删除定义不影响已安排的执行。
//!
//! 动作只有程序（ADR 045 D1）：说一句、弹幕、推送、交给她都是程序里的函数。旧写法（“播报”“让她做”）
//! 由私有类型读进来、转成等价的程序（[`AutomationLibrary::open`] 转完存盘；工具 `automation.save`
//! 收到的旧写法在解析 [`AutomationDraft`] 时就转好）。

use std::{collections::BTreeMap, path::PathBuf, sync::Mutex};

use serde_json::{Map, Value};

use super::{now_ms, reminder_source, write_atomic, SpeakAction, MAX_NAME_CHARS, MAX_SOURCE_CHARS};
use crate::games::{validate_trigger, EventCatalog, EventTrigger, GameEvent};
use crate::reminders::{validate_content, DeliveryKind};

pub const MAX_AUTOMATIONS: usize = 200;
pub const MAX_PARAMS: usize = 16;
const MAX_DESCRIPTION_CHARS: usize = 200;
const MAX_LABEL_CHARS: usize = 40;
const MAX_TEXT_PARAM_CHARS: u32 = 200;
const MAX_CHOICES: usize = 16;
const STORE_VERSION: u32 = 1;
/// 旧写法“让她做”那句话最多多少字（和脚本引擎里 `ask()` 的上限一样）。
const MAX_PROMPT_CHARS: usize = 500;

/// 入参类型。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ParamKind {
    /// 数字；`integer` 时必须是整数。
    Number {
        #[serde(default)]
        min: Option<f64>,
        #[serde(default)]
        max: Option<f64>,
        #[serde(default)]
        integer: bool,
    },
    /// 文字（不能有换行）。
    Text {
        #[serde(default = "default_text_max")]
        max_length: u32,
    },
    Boolean,
    /// 几个固定选项之一。
    Choice {
        options: Vec<String>,
    },
    /// 按键名，如 `Q`、`Enter`、`Ctrl+Q`。
    Key,
    /// 窗口客户区比例区域 `{x, y, w, h}`。
    Region,
    /// 颜色 `{r, g, b}`。
    Color,
    /// 模板图片（`find_image` 用）：`{image: "<sha256>"}`，指向 `~/.kotone/automation-images/`
    /// 里的 PNG。提交时可以给 `{png: "<base64>"}`，由桌面端存成文件后换成引用。
    Image,
}

/// 图片引用是否合法（64 位小写十六进制 sha256）。
pub fn is_image_id(id: &str) -> bool {
    id.len() == 64 && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn default_text_max() -> u32 {
    MAX_TEXT_PARAM_CHARS
}

/// 一个入参的声明。没有 `default` 的参数调用时必须给。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParamSpec {
    /// 程序里的名字（`args.<name>`）：小写字母、数字、下划线，字母开头。
    pub name: String,
    /// 界面上的名字；缺省同 `name`。
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub description: String,
    #[serde(flatten)]
    pub kind: ParamKind,
    #[serde(default)]
    pub default: Option<Value>,
}

/// 定义里的动作（不含执行时才有的目标窗口与入参）：只有程序（ADR 045 D1）。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum AutomationBody {
    Program(ProgramBody),
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgramBody {
    pub source: String,
}

impl AutomationBody {
    pub fn source(&self) -> &str {
        let AutomationBody::Program(program) = self;
        &program.source
    }

    /// 程序里用到了什么（用不用游戏窗口、会不会叫她）。
    pub fn uses(&self) -> ProgramUses {
        ProgramUses::of(self.source())
    }
}

/// 旧写法（rc.13 及以前）的“让她做”（ADR 036 D8）：只为读进来转成 `ask("…")`。
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct PromptBody {
    text: String,
}

/// 读进来的动作：现在的程序，或旧写法（转成程序，ADR 045 D1）。
#[derive(serde::Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum StoredBody {
    Program(ProgramBody),
    Speak(SpeakAction),
    Prompt(PromptBody),
}

impl StoredBody {
    /// 工具 / 界面交来的旧写法先照旧规矩查一遍（话不能为空、回答方式不能重复、`{参数}` 要有对应的参数），
    /// 免得转出一段跑不起来的程序。存盘里的旧定义当初查过，不再查。
    fn check(&self, params: &[ParamSpec]) -> Result<(), String> {
        match self {
            StoredBody::Program(_) => Ok(()),
            StoredBody::Speak(speak) => {
                validate_content(&speak.text, &speak.delivery)?;
                for name in placeholders(&speak.text) {
                    if !params.iter().any(|p| p.name == name) {
                        return Err(format!("播报里的 {{{name}}} 没有对应的参数"));
                    }
                }
                Ok(())
            }
            StoredBody::Prompt(prompt) => {
                let chars = prompt.text.trim().chars().count();
                if chars == 0 || chars > MAX_PROMPT_CHARS {
                    return Err(format!(
                        "“让她做”的那句话不能为空，最多 {MAX_PROMPT_CHARS} 字"
                    ));
                }
                Ok(())
            }
        }
    }

    /// 转成程序；第二项：是不是旧写法。
    fn upgrade(self, params: &[ParamSpec]) -> (AutomationBody, bool) {
        let source = match self {
            StoredBody::Program(program) => return (AutomationBody::Program(program), false),
            StoredBody::Speak(speak) => speak_source(params, speak.text.trim(), &speak.delivery),
            StoredBody::Prompt(prompt) => format!("ask({})", lua_string(prompt.text.trim())),
        };
        (AutomationBody::Program(ProgramBody { source }), true)
    }
}

/// Luau 字符串字面量（双引号；反斜杠、引号、换行和控制字符转义）。
pub(crate) fn lua_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{{{:x}}}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// 旧播报文字里的 `{参数名}`。
fn placeholders(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find('{') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('}') else { break };
        let name = &after[..end];
        if is_identifier(name) {
            out.push(name.to_string());
        }
        rest = &after[end + 1..];
    }
    out
}

/// 旧播报 → 一句 `say` / `toast`（ADR 045 D1）：`"该{text}了"` → `say("该" .. args.text .. "了")`。
/// 显示和以前的 `{参数}` 替换一样：文字原样，数字 `tostring`，开关“是 / 否”。只要语音 `say(X)`，只要弹幕
/// `toast(X)`，两者都要 `say(X, { toast = true })`（一次投递，语音和弹幕同时出现）。
fn speak_source(params: &[ParamSpec], text: &str, delivery: &[DeliveryKind]) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut literal = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('{') {
        literal.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let param = after.find('}').and_then(|end| {
            let name = &after[..end];
            params.iter().find(|p| p.name == name).map(|p| (p, end))
        });
        match param {
            Some((spec, end)) => {
                if !literal.is_empty() {
                    parts.push(lua_string(&std::mem::take(&mut literal)));
                }
                let arg = format!("args.{}", spec.name);
                parts.push(match spec.kind {
                    ParamKind::Text { .. } | ParamKind::Choice { .. } | ParamKind::Key => arg,
                    ParamKind::Boolean => format!("({arg} and \"是\" or \"否\")"),
                    _ => format!("tostring({arg})"),
                });
                rest = &after[end + 1..];
            }
            None => {
                literal.push('{');
                rest = after;
            }
        }
    }
    literal.push_str(rest);
    if !literal.is_empty() || parts.is_empty() {
        parts.push(lua_string(&literal));
    }
    let said = parts.join(" .. ");
    // 和提醒同一个规矩：选的是哪段固定程序，就照它的样子写（`args.text` 换成这里拼出来的话）。
    reminder_source(delivery).replace("args.text", &said)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CreatedBy {
    User,
    Companion,
    Agent,
}

/// 定时（类似 crontab，本地时间）：到点用 `args` 运行一次。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Schedule {
    /// 五段 `分 时 日 月 周`，或 `@daily` 等（见 [`super::cron`]）。
    pub cron: String,
    #[serde(default = "enabled_by_default")]
    pub enabled: bool,
    /// 到点运行时用的入参（没给的用参数默认值）。
    #[serde(default)]
    pub args: Map<String, Value>,
}

fn enabled_by_default() -> bool {
    true
}

/// 内置的“X 秒之后提醒我 xxx”（第一次打开时装上；玩家删了不再装回）。
pub const BUILTIN_REMIND_ID: &str = "builtin-remind-later";
/// 表示“几秒后再开始”的参数名（ADR 045 D7：任何自动化声明了这个数字参数都生效，见 [`delay_ms`]）。
pub const DELAY_PARAM: &str = "delay_seconds";

/// 已保存的自动化。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", from = "StoredAutomation")]
pub struct Automation {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub params: Vec<ParamSpec>,
    pub action: AutomationBody,
    pub created_by: CreatedBy,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    /// 定时运行；None = 只手动 / 由琴音运行。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule: Option<Schedule>,
    /// 游戏事件触发（ADR 036 D6）；None = 不按事件运行。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on: Option<EventTrigger>,
    /// 玩家确认过这段程序可以调的工具（ADR 046 D3）：工具 → 确认时它的副作用。只有 Kotone 窗口能写
    /// （[`AutomationLibrary::confirm`]）；提交的草稿带它直接拒；程序一改就清掉（谁改的都算），只改名字、参数、
    /// 触发不动它。rc.13 读到这个字段会忽略。
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub confirmed: BTreeMap<String, Vec<String>>,
}

/// 存盘里的自动化：和 [`Automation`] 一样，只是动作还可能是旧写法。
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredAutomation {
    id: String,
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    params: Vec<ParamSpec>,
    action: StoredBody,
    created_by: CreatedBy,
    created_at_ms: u64,
    updated_at_ms: u64,
    #[serde(default)]
    schedule: Option<Schedule>,
    #[serde(default)]
    on: Option<EventTrigger>,
    #[serde(default)]
    confirmed: BTreeMap<String, Vec<String>>,
}

impl StoredAutomation {
    /// 第二项：动作是不是旧写法（转过了要存盘）。
    fn upgrade(self) -> (Automation, bool) {
        let (action, upgraded) = self.action.upgrade(&self.params);
        let automation = Automation {
            id: self.id,
            name: self.name,
            description: self.description,
            params: self.params,
            action,
            created_by: self.created_by,
            created_at_ms: self.created_at_ms,
            updated_at_ms: self.updated_at_ms,
            schedule: self.schedule,
            on: self.on,
            // 旧写法转出来的程序不调工具，没有确认可留
            confirmed: if upgraded {
                BTreeMap::new()
            } else {
                self.confirmed
            },
        };
        (automation, upgraded)
    }
}

impl From<StoredAutomation> for Automation {
    fn from(stored: StoredAutomation) -> Self {
        stored.upgrade().0
    }
}

/// 新建或覆盖时提交的内容（`id` 为空 = 新建）。`action` 收旧写法（工具 `automation.save` 已发布的
/// `speak` / `prompt`），解析时就转成等价的程序（ADR 045 D1）。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", try_from = "DraftFields")]
pub struct AutomationDraft {
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub params: Vec<ParamSpec>,
    pub action: AutomationBody,
    #[serde(default)]
    pub schedule: Option<Schedule>,
    #[serde(default)]
    pub on: Option<EventTrigger>,
}

/// 提交上来的样子（不认识的字段直接拒，打错字不会悄悄失效）。
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DraftFields {
    #[serde(default)]
    id: Option<String>,
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    params: Vec<ParamSpec>,
    action: StoredBody,
    #[serde(default)]
    schedule: Option<Schedule>,
    #[serde(default)]
    on: Option<EventTrigger>,
    /// 只为认出来、说清楚再拒（ADR 046 D3）：确认只有玩家在 Kotone 窗口里能做。
    #[serde(default)]
    confirmed: Option<serde::de::IgnoredAny>,
}

impl TryFrom<DraftFields> for AutomationDraft {
    type Error = String;

    fn try_from(fields: DraftFields) -> Result<Self, String> {
        if fields.confirmed.is_some() {
            return Err(
                "保存时不能带 confirmed：要调的工具由玩家在「自动化」页点“允许”确认".into(),
            );
        }
        fields.action.check(&fields.params)?;
        let (action, _) = fields.action.upgrade(&fields.params);
        Ok(Self {
            id: fields.id,
            name: fields.name,
            description: fields.description,
            params: fields.params,
            action,
            schedule: fields.schedule,
            on: fields.on,
        })
    }
}

fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_lowercase())
        && name.len() <= 32
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// 与脚本引擎同一规则：字母、数字、空格、`+`，最多 32 个字符（宿主再精确解析）。
fn is_key_name(key: &str) -> bool {
    !key.trim().is_empty()
        && key.len() <= 32
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == ' ')
}

fn unit(v: Option<f64>) -> Option<f64> {
    v.filter(|v| v.is_finite() && (0.0..=1.0).contains(v))
}

fn check_value(spec: &ParamSpec, value: &Value) -> Result<Value, String> {
    let name = &spec.name;
    match &spec.kind {
        ParamKind::Number { min, max, integer } => {
            let n = value
                .as_f64()
                .filter(|n| n.is_finite())
                .ok_or_else(|| format!("参数 {name} 要是数字"))?;
            if *integer && n.fract() != 0.0 {
                return Err(format!("参数 {name} 要是整数"));
            }
            if min.is_some_and(|m| n < m) || max.is_some_and(|m| n > m) {
                return Err(format!(
                    "参数 {name} 超出范围（{} 到 {}）",
                    min.map_or("-∞".into(), |m| m.to_string()),
                    max.map_or("∞".into(), |m| m.to_string())
                ));
            }
            Ok(value.clone())
        }
        ParamKind::Text { max_length } => {
            let s = value
                .as_str()
                .ok_or_else(|| format!("参数 {name} 要是文字"))?;
            if s.chars().count() > *max_length as usize || s.chars().any(char::is_control) {
                return Err(format!(
                    "参数 {name} 超过 {max_length} 字或含有换行等控制字符"
                ));
            }
            Ok(value.clone())
        }
        ParamKind::Boolean => value
            .as_bool()
            .map(Value::Bool)
            .ok_or_else(|| format!("参数 {name} 要是 true 或 false")),
        ParamKind::Choice { options } => {
            let s = value
                .as_str()
                .ok_or_else(|| format!("参数 {name} 要是文字"))?;
            if options.iter().any(|o| o == s) {
                Ok(value.clone())
            } else {
                Err(format!("参数 {name} 只能是：{}", options.join(" / ")))
            }
        }
        ParamKind::Key => match value.as_str() {
            Some(key) if is_key_name(key) => Ok(Value::String(key.trim().into())),
            _ => Err(format!("参数 {name} 要是按键名，如 Q、Enter、Ctrl+Q")),
        },
        ParamKind::Region => {
            let get = |k: &str| unit(value.get(k).and_then(Value::as_f64));
            let (Some(x), Some(y), Some(w), Some(h)) = (get("x"), get("y"), get("w"), get("h"))
            else {
                return Err(format!(
                    "参数 {name} 要是区域 {{x, y, w, h}}（窗口比例 0 到 1）"
                ));
            };
            let extra = value.as_object().is_some_and(|o| o.len() != 4);
            if extra || w <= 0.0 || h <= 0.0 || x + w > 1.0001 || y + h > 1.0001 {
                return Err(format!(
                    "参数 {name} 的区域不合法：只能有 x、y、w、h，且不能超出窗口"
                ));
            }
            Ok(serde_json::json!({"x": x, "y": y, "w": w, "h": h}))
        }
        ParamKind::Color => {
            let get = |k: &str| value.get(k).and_then(Value::as_u64).filter(|v| *v <= 255);
            match (
                get("r"),
                get("g"),
                get("b"),
                value.as_object().map(Map::len),
            ) {
                (Some(r), Some(g), Some(b), Some(3)) => {
                    Ok(serde_json::json!({"r": r, "g": g, "b": b}))
                }
                _ => Err(format!("参数 {name} 要是颜色 {{r, g, b}}（0 到 255）")),
            }
        }
        ParamKind::Image => match value
            .as_object()
            .filter(|o| o.len() == 1)
            .and_then(|o| o.get("image"))
            .and_then(Value::as_str)
        {
            Some(id) if is_image_id(id) => Ok(serde_json::json!({"image": id})),
            _ => Err(format!(
                "参数 {name} 要是图片（{{png: base64}}，保存后变成 {{image: <编号>}}）"
            )),
        },
    }
}

/// 按声明校验调用时的入参：不认识的参数、缺少必填参数、类型或范围不对都报错；
/// 返回补上缺省值、规范化之后的入参。
pub fn validate_args(
    params: &[ParamSpec],
    given: &Map<String, Value>,
) -> Result<Map<String, Value>, String> {
    if let Some(unknown) = given.keys().find(|k| !params.iter().any(|p| &p.name == *k)) {
        return Err(format!("没有名为 {unknown} 的参数"));
    }
    let mut out = Map::new();
    for spec in params {
        let value = match given.get(&spec.name).filter(|v| !v.is_null()) {
            Some(value) => value,
            None => match &spec.default {
                Some(default) => default,
                None => return Err(format!("缺少参数 {}", spec.name)),
            },
        };
        out.insert(spec.name.clone(), check_value(spec, value)?);
    }
    Ok(out)
}

/// 几秒后再开始（ADR 045 D7，毫秒）：声明了数字参数 `delay_seconds` 的自动化，运行时就晚这么多秒开始
/// （以前只对播报有效）。和工具的 `delaySeconds` 相加，合计不超过 7 天由调度器查。
pub fn delay_ms(params: &[ParamSpec], args: &Map<String, Value>) -> u64 {
    let declared = params
        .iter()
        .any(|p| p.name == DELAY_PARAM && matches!(p.kind, ParamKind::Number { .. }));
    if !declared {
        return 0;
    }
    args.get(DELAY_PARAM)
        .and_then(Value::as_f64)
        .filter(|v| v.is_finite() && *v > 0.0)
        .map(|v| (v * 1000.0) as u64)
        .unwrap_or(0)
}

/// `elapsedSeconds` → `elapsed_seconds`（参数名只能是小写字母、数字、下划线）。
pub(crate) fn snake(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_uppercase() {
            out.push('_');
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// 游戏事件触发时的入参（ADR 036 D6，事件是函数的输入）：和事件 `data` 字段同名的参数自动填上
/// （camelCase 字段对 snake_case 参数）。[`super::ProgramAction::check_ask`] 按同一个规矩认“从事件填进来的值”。
pub fn event_args(params: &[ParamSpec], event: &GameEvent) -> Map<String, Value> {
    params
        .iter()
        .filter_map(|p| {
            event
                .data
                .iter()
                .find(|(k, _)| **k == p.name || snake(k) == p.name)
                .map(|(_, v)| (p.name.clone(), v.clone()))
        })
        .collect()
}

// ---------- 程序里用到了什么（ADR 045 D4） ----------

/// 用到游戏窗口的函数：按键、打字、发到游戏聊天、鼠标、看屏幕、`game_focused`。程序里出现其中之一就绑窗口：
/// 安排时前台要是允许自动化输入的游戏、游戏不在前台就暂停、窗口关了就结束、占名额、挡她操作游戏。
pub const WINDOW_FUNCTIONS: &[&str] = &[
    "key",
    "key_down",
    "key_up",
    "type_text",
    "chat",
    "click",
    "mouse_move",
    "mouse_down",
    "mouse_up",
    "scroll",
    "brightness",
    "color_at",
    "ocr",
    "find_text",
    "find_color",
    "find_image",
    "game_focused",
];

/// 不用游戏窗口的函数：时钟、等待、说话、弹幕、推送、交给她、调工具、日志。脚本引擎的测试保证白名单里每个函数都在
/// 这两张表之一里。`tool` 不算用到游戏窗口（ADR 046 D1）：往游戏里打字、按键的工具程序里调不了（D2）。
pub const PLAIN_FUNCTIONS: &[&str] = &[
    "wait",
    "now",
    "every",
    "wait_until",
    "region",
    "say",
    "toast",
    "push",
    "ask",
    "tool",
    "log",
    "stop",
];

/// 说出或显示文字的函数（她开口说、弹幕、推送）：她运行这样的自动化，副作用是“显示或说出文字”（ADR 046 D5）。
const SPEAK_FUNCTIONS: &[&str] = &["say", "toast", "push"];

/// 程序里用到了什么：按源码保守判断，宁可多算（注释和字符串里的不算；`args.key`、`t:find`、`{ key = 1 }`
/// 不算；反引号字符串里出现的名字都算）。算漏了也不出事：窗口函数在没绑窗口的执行里一调用就报错。
/// 会调哪些工具另算（[`tools_called`]：名字要直接写，否则存不进来）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgramUses {
    /// 用到游戏窗口（[`WINDOW_FUNCTIONS`]）。
    pub window: bool,
    /// 会叫她（`ask`）：每次花一次 AI 调用；会叫她的自动化不当 `saved.*` 工具给她。
    pub asks: bool,
    /// 会说出或显示文字（`say` / `toast` / `push`）。
    pub speaks: bool,
}

impl ProgramUses {
    pub fn of(source: &str) -> Self {
        let mut uses = Self::default();
        for name in used_names(source) {
            uses.window |= WINDOW_FUNCTIONS.contains(&name);
            uses.asks |= name == "ask";
            uses.speaks |= SPEAK_FUNCTIONS.contains(&name);
        }
        uses
    }
}

// ---------- 程序里调哪些工具（ADR 046 D1） ----------

/// 工具名的格式：字母开头，只有字母、数字、`_`、`-`、`.`，最多 64 个字符，不以 `.` 结尾、没有连着的 `..`
/// （没有点的 `music`、第三方的大写和连字符都认）。和脚本引擎的 `kotone_script::is_tool_name` 同一条规矩
/// （那边的测试对着查）。
pub fn is_tool_name(name: &str) -> bool {
    name.len() <= 64
        && name.starts_with(|c: char| c.is_ascii_alphabetic())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
        && !name.ends_with('.')
        && !name.contains("..")
}

const NOT_A_LITERAL: &str =
    "tool() 的第一个参数要直接写工具名，例如 tool(\"music\", { action = \"play\" })：\
     不能用变量或拼出来的名字，也别拿 tool 当变量名";

/// 程序里会调哪些工具（按出现的先后，不重复）。和“用到游戏窗口”用同一个记号器：注释、字符串里的不算，
/// `t.tool`、`{ tool = 1 }` 不算。`tool` 只能这样用：`tool("名字", …)` / `tool "名字"`（名字直接写成 `"…"` 或 `'…'`
/// 字符串）；别的用法（变量、拼出来的名字、`local f = tool`、写进反引号字符串）一律报错——存不进来、也不让跑。
/// 运行时宿主再查一遍名字在不在这张单子上（`_G["to" .. "ol"]` 这类绕过记号器的写法到那里就报错）。
pub fn tools_called(source: &str) -> Result<Vec<String>, String> {
    let tokens = tokens(source);
    let mut out: Vec<String> = Vec::new();
    for (i, token) in tokens.iter().enumerate() {
        let name = match *token {
            Token::Quoted("tool") => return Err(NOT_A_LITERAL.into()),
            Token::Name("tool") if global_use(&tokens, i) => {
                match (tokens.get(i + 1), tokens.get(i + 2), tokens.get(i + 3)) {
                    (
                        Some(Token::Open),
                        Some(Token::Str(name)),
                        Some(Token::Comma | Token::Close),
                    ) => *name,
                    (Some(Token::Str(name)), _, _) => *name,
                    _ => return Err(NOT_A_LITERAL.into()),
                }
            }
            _ => continue,
        };
        if !is_tool_name(name) {
            return Err(format!(
                "tool(\"{}\") 的工具名不对：工具名是 music、lol.match 这样的（字母开头，只有字母、数字、_ - .，\
                 最多 64 个字符）",
                name.chars().take(64).collect::<String>()
            ));
        }
        if !out.iter().any(|n| n == name) {
            out.push(name.to_string());
        }
    }
    Ok(out)
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Token<'a> {
    Name(&'a str),
    /// 反引号字符串里的名字（插值 `{…}` 是代码）：一律算用到。
    Quoted(&'a str),
    /// `"…"` / `'…'` 字符串（引号里的原文，不解转义）。
    Str(&'a str),
    /// 单个 `.`（不是 `..`、`...`）。
    Dot,
    /// 单个 `:`（不是 `::`）。
    Colon,
    /// 单个 `=`（不是 `==`、`~=`、`<=`、`>=`）。
    Assign,
    /// `(`
    Open,
    /// `)`
    Close,
    /// `,`
    Comma,
    Other,
}

fn ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

fn ident_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// `[` 开始的长括号（`[[`、`[==[`）的等号个数；不是长括号为 None。
fn long_bracket(b: &[u8], at: usize) -> Option<usize> {
    let mut i = at + 1;
    while b.get(i) == Some(&b'=') {
        i += 1;
    }
    (b.get(i) == Some(&b'[')).then_some(i - at - 1)
}

/// 跳过长括号的内容，返回结束后的位置（没结束就到末尾）。
fn skip_long(b: &[u8], at: usize, level: usize) -> usize {
    let mut i = at + level + 2;
    while i < b.len() {
        if b[i] == b']' {
            let mut j = i + 1;
            while j < b.len() && b[j] == b'=' {
                j += 1;
            }
            if j - i - 1 == level && b.get(j) == Some(&b']') {
                return j + 1;
            }
        }
        i += 1;
    }
    b.len()
}

/// 粗切成记号：跳过注释、字符串、数字。
fn tokens(source: &str) -> Vec<Token<'_>> {
    let b = source.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c.is_ascii_whitespace() {
            i += 1;
        } else if c == b'-' && b.get(i + 1) == Some(&b'-') {
            i += 2;
            match (b.get(i) == Some(&b'['))
                .then(|| long_bracket(b, i))
                .flatten()
            {
                Some(level) => i = skip_long(b, i, level),
                None => {
                    while i < b.len() && b[i] != b'\n' {
                        i += 1;
                    }
                }
            }
        } else if c == b'"' || c == b'\'' {
            i += 1;
            let start = i;
            while i < b.len() && b[i] != c && b[i] != b'\n' {
                i += if b[i] == b'\\' { 2 } else { 1 };
            }
            let end = i.min(b.len());
            i += 1;
            out.push(Token::Str(source.get(start..end).unwrap_or("")));
        } else if c == b'`' {
            let start = i + 1;
            i += 1;
            while i < b.len() && b[i] != b'`' {
                i += if b[i] == b'\\' { 2 } else { 1 };
            }
            let end = i.min(b.len());
            let mut j = start;
            while j < end {
                if ident_start(b[j]) && (j == start || !ident_char(b[j - 1])) {
                    let k = (j..end).find(|&k| !ident_char(b[k])).unwrap_or(end);
                    out.push(Token::Quoted(&source[j..k]));
                    j = k;
                } else {
                    j += 1;
                }
            }
            i += 1;
            out.push(Token::Other);
        } else if c == b'[' && long_bracket(b, i).is_some() {
            i = skip_long(b, i, long_bracket(b, i).unwrap_or(0));
            out.push(Token::Other);
        } else if ident_start(c) {
            let start = i;
            while i < b.len() && ident_char(b[i]) {
                i += 1;
            }
            out.push(Token::Name(&source[start..i]));
        } else if c.is_ascii_digit() {
            while i < b.len() && (ident_char(b[i]) || b[i] == b'.') {
                let exponent = matches!(b[i], b'e' | b'E');
                i += 1;
                if exponent && matches!(b.get(i), Some(b'+' | b'-')) {
                    i += 1;
                }
            }
            out.push(Token::Other);
        } else {
            let next = b.get(i + 1).copied();
            let token = match c {
                b'.' if next == Some(b'.') => {
                    while i < b.len() && b[i] == b'.' {
                        i += 1;
                    }
                    out.push(Token::Other);
                    continue;
                }
                b'.' => Token::Dot,
                b':' if next == Some(b':') => {
                    i += 1;
                    Token::Other
                }
                b':' => Token::Colon,
                b'=' if next == Some(b'=') => {
                    i += 1;
                    Token::Other
                }
                b'=' => Token::Assign,
                b'~' | b'<' | b'>' | b'+' | b'-' | b'*' | b'/' | b'%' | b'^'
                    if next == Some(b'=') =>
                {
                    i += 1;
                    Token::Other
                }
                b'(' => Token::Open,
                b')' => Token::Close,
                b',' => Token::Comma,
                _ => Token::Other,
            };
            i += 1;
            out.push(token);
        }
    }
    out
}

/// 程序里当全局名字用到的名字（保守：字段 `a.b`、方法 `a:b`、表键 / 赋值目标 `b = …` 不算，`_G.b` 算）。
fn used_names(source: &str) -> Vec<&str> {
    let tokens = tokens(source);
    let mut out = Vec::new();
    for (i, token) in tokens.iter().enumerate() {
        match *token {
            Token::Quoted(name) => out.push(name),
            Token::Name(name) if global_use(&tokens, i) => out.push(name),
            _ => {}
        }
    }
    out
}

/// 第 `i` 个记号（一个名字）是不是当全局名字用：不是字段 `a.b`（`_G.b` 算）、方法 `a:b`、表键 / 赋值目标 `b = …`。
fn global_use(tokens: &[Token<'_>], i: usize) -> bool {
    let before = |n: usize| i.checked_sub(n).map(|j| tokens[j]);
    let field = match before(1) {
        Some(Token::Dot) => before(2) != Some(Token::Name("_G")),
        Some(Token::Colon) => true,
        _ => false,
    };
    let assigned = tokens.get(i + 1) == Some(&Token::Assign);
    !field && !assigned
}

// ---------- 保存时的提醒：条件写在程序里，次数和冷却照样用掉（ADR 046 的 2026-11-27 修订） ----------

/// 程序里读的入参名字（`args.<名字>`，按出现的先后、不重复）。和“用到游戏窗口”同一个记号器：注释、字符串里的不算，
/// `t.args.x` 不算；`args["x"]`、`local a = args` 这类读法认不出（只用来提醒，认漏了不出事）。
fn args_read(source: &str) -> Vec<&str> {
    let tokens = tokens(source);
    let mut out: Vec<&str> = Vec::new();
    for (i, token) in tokens.iter().enumerate() {
        if *token != Token::Name("args") || !global_use(&tokens, i) {
            continue;
        }
        if let (Some(Token::Dot), Some(Token::Name(name))) = (tokens.get(i + 1), tokens.get(i + 2))
        {
            if !out.contains(name) {
                out.push(name);
            }
        }
    }
    out
}

/// 保存时给她的提醒（不拦）：按事件触发、开了“一局一次”或冷却，程序里又读了由事件自动填的参数（[`event_args`] 的规矩：
/// 参数名对上触发里那几种事的 `data` 字段，camelCase 字段对 snake_case 参数；已经写进 `on.match` 的字段不算）。
/// 程序拿它判断、不满足就 `return` 的那一次也算触发，会占掉次数和冷却，真到该触发的时候可能就不触发了——玩家
/// 2026-11-27 让她写的“队友都倒了放首歌”就是这样：触发只写了 `team: ally`，第一个倒下的队友就用掉了一局一次。
/// 读事件的值只拿来说话（`say(args.champion .. "倒了")`）的也会提醒，所以话说成让她自己判断，不当错误。
/// 触发关着、没开次数和冷却、没读事件填的参数时为空。桌面端放进保存结果的 `triggerNotes`。
pub fn trigger_hints(automation: &Automation, events: &EventCatalog) -> Vec<String> {
    let Some(on) = automation.on.as_ref().filter(|on| on.enabled) else {
        return Vec::new();
    };
    if !on.once_per_match && on.cooldown_seconds == 0 {
        return Vec::new();
    }
    let fields = events.data_fields(on);
    let read = args_read(automation.action.source());
    let filled: Vec<String> = automation
        .params
        .iter()
        .filter(|p| read.contains(&p.name.as_str()))
        .filter_map(|p| {
            let field = fields
                .iter()
                .find(|f| **f == p.name || snake(f) == p.name)?;
            (!on.matches.contains_key(*field)).then(|| {
                if *field == p.name {
                    format!("args.{}", p.name)
                } else {
                    format!("args.{}（{field}）", p.name)
                }
            })
        })
        .collect();
    if filled.is_empty() {
        return Vec::new();
    }
    let limits = match (on.once_per_match, on.cooldown_seconds) {
        (true, 0) => "一局一次".to_string(),
        (true, s) => format!("一局一次和冷却（{s} 秒）"),
        (false, s) => format!("冷却（{s} 秒）"),
    };
    vec![format!(
        "这个触发开了{limits}，程序里又读了事件填进来的 {}：要是程序拿它判断、不满足就 return，那一次也算触发，\
         会占掉{limits}，真该触发时可能就不触发了。能写进 on.match 的条件写进去（对事件 data 的子集匹配，数组表示“或”）；\
         只是拿来说话的不用改。",
        filled.join("、")
    )]
}

// ---------- 模板（ADR 045 D9） ----------

/// 新建自动化时选的模板。和内置程序一样由测试保证能编译、只填缺省值就能跑；界面按 `id` 取文案。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Template {
    pub id: String,
    pub name: String,
    pub description: String,
    pub params: Vec<ParamSpec>,
    pub source: String,
    /// 带好的游戏事件触发（没有 = 不带，新建时玩家自己选）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on: Option<EventTrigger>,
    /// 要哪个第三方插件（ADR 047 第 2 期）：Kotone 预设的 id（`zzz-onedragon`）。触发里的事件名以 `<它>.` 开头；
    /// 第三方插件的 id 是玩家起的，界面换成用着这份预设的那个插件的 id，没有这样的插件时不列这个模板。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
}

/// 绝区零一条龙的预设建议的插件 id（`kotone-remote-plugin` 的 `presets/zzz-onedragon.json`）。
pub const ONEDRAGON_PRESET: &str = "zzz-onedragon";

/// “一条龙跑完推一行”的程序（ADR 045：通知是函数，写了才有）。`args.to` 由事件填（事件 `data.to`：`success` / `failed`）。
/// 不把它说的原话拼进推送：别人的服务器给的字可能很长、带换行，推送最多 40 字，超了这一次就推不出去。
const ONEDRAGON_SOURCE: &str = r#"
-- 绝区零一条龙跑完 / 出错时推一行（args.to 由事件自动填上：success 或 failed）
if args.to == "failed" then
  push("绝区零一条龙出错了，去看看")
else
  push("绝区零一条龙跑完了")
end
"#;

/// 五个模板：说一句、推送一条、让她做、空白程序（按键类复制内置程序），和“一条龙跑完推一行”（要先加绝区零一条龙插件）。
/// “说一句”用 `args.text`，不再支持 `{参数}` 写法；要用事件里的字，让她写或改程序。
pub fn templates() -> Vec<Template> {
    let text = |name: &str, label: &str, max_length: u32, default: &str| ParamSpec {
        name: name.into(),
        label: label.into(),
        description: String::new(),
        kind: ParamKind::Text { max_length },
        default: Some(Value::from(default)),
    };
    let template = |id: &str, name: &str, description: &str, params, source: &str| Template {
        id: id.into(),
        name: name.into(),
        description: description.into(),
        params,
        source: source.trim().to_string() + "\n",
        on: None,
        preset: None,
    };
    vec![
        template(
            "say",
            "说一句",
            "她开口说一句话。想同时弹幕就写 say(args.text, { toast = true })。",
            vec![text("text", "说什么", 200, "该看小地图了")],
            "say(args.text)",
        ),
        template(
            "push",
            "推送一条",
            "在 Kotone 聊天里记一条、悬浮窗显示一行；不出声，不调用 AI。最多 40 字。",
            vec![text("text", "推送什么", 40, "记得买真眼")],
            "push(args.text)",
        ),
        template(
            "ask",
            "让她做",
            "把一句话交给她，她看着办（查战绩、说话、调工具都行）。每次花一次 AI 调用；按游戏事件触发时每局最多 3 次。",
            vec![text(
                "request",
                "让她做什么",
                200,
                "看看现在的情况，用一句话告诉我该注意什么",
            )],
            "ask(args.request)",
        ),
        template(
            "blank",
            "空白程序",
            "自己写一段程序。要通知就写 say / toast / push，不写就安静；按键类可以复制内置的程序。",
            vec![],
            "-- 在这里写程序，例如：\n-- say(\"该买眼了\")",
        ),
        Template {
            on: Some(EventTrigger {
                event: vec![
                    format!("{ONEDRAGON_PRESET}.run_finished"),
                    format!("{ONEDRAGON_PRESET}.run_failed"),
                ],
                game: Some("zzz".into()),
                matches: Map::new(),
                for_seconds: 0,
                mode: crate::games::RunMode::Single,
                once_per_match: false,
                cooldown_seconds: 0,
                enabled: true,
            }),
            preset: Some(ONEDRAGON_PRESET.into()),
            ..template(
                "onedragon-push",
                "一条龙跑完推一行",
                "绝区零一条龙跑完或出错时，在 Kotone 聊天里记一行、悬浮窗显示一行；不出声，不调用 AI。要先加绝区零一条龙插件，并在它的「设置 → 事件」里打开“跑完了”“出错了”。",
                vec![text("to", "现在的状态（事件自动填）", 20, "success")],
                ONEDRAGON_SOURCE,
            )
        },
    ]
}

fn validate_params(params: &mut [ParamSpec]) -> Result<(), String> {
    if params.len() > MAX_PARAMS {
        return Err(format!("最多 {MAX_PARAMS} 个参数"));
    }
    for spec in params.iter_mut() {
        if !is_identifier(&spec.name) {
            return Err(format!(
                "参数名 {} 不合法：用小写字母开头，只含小写字母、数字、下划线",
                spec.name
            ));
        }
        spec.label = spec.label.trim().to_string();
        if spec.label.is_empty() {
            spec.label = spec.name.clone();
        }
        spec.description = spec.description.trim().to_string();
        if spec.label.chars().count() > MAX_LABEL_CHARS
            || spec.description.chars().count() > MAX_DESCRIPTION_CHARS
        {
            return Err(format!("参数 {} 的名字或说明太长", spec.name));
        }
        match &spec.kind {
            ParamKind::Number {
                min: Some(a),
                max: Some(b),
                ..
            } if a > b => {
                return Err(format!("参数 {} 的最小值大于最大值", spec.name));
            }
            ParamKind::Text { max_length }
                if *max_length == 0 || *max_length > MAX_TEXT_PARAM_CHARS =>
            {
                return Err(format!(
                    "参数 {} 的长度上限要在 1 到 {MAX_TEXT_PARAM_CHARS} 之间",
                    spec.name
                ));
            }
            ParamKind::Choice { options }
                if options.is_empty()
                    || options.len() > MAX_CHOICES
                    || options
                        .iter()
                        .any(|o| o.trim().is_empty() || o.chars().count() > MAX_LABEL_CHARS) =>
            {
                return Err(format!(
                    "参数 {} 要有 1 到 {MAX_CHOICES} 个选项，每个不超过 {MAX_LABEL_CHARS} 字",
                    spec.name
                ));
            }
            _ => {}
        }
        if let Some(default) = spec.default.clone().filter(|v| !v.is_null()) {
            spec.default = Some(check_value(spec, &default).map_err(|e| format!("缺省值：{e}"))?);
        } else {
            spec.default = None;
        }
    }
    let mut names: Vec<&str> = params.iter().map(|p| p.name.as_str()).collect();
    names.sort_unstable();
    if names.windows(2).any(|w| w[0] == w[1]) {
        return Err("参数名重复".into());
    }
    Ok(())
}

impl AutomationDraft {
    /// 检查并整理。游戏事件触发按合并后的事件目录 `events` 认名字（本体的 `game.*` 加插件登记的）。
    /// 旧写法在解析时已经转成程序；程序的语法由桌面端（脚本引擎）查。
    fn normalized(mut self, events: &EventCatalog) -> Result<Self, String> {
        self.name = self.name.trim().to_string();
        self.description = self.description.trim().to_string();
        if self.name.is_empty() || self.name.chars().count() > MAX_NAME_CHARS {
            return Err(format!("自动化名字不能为空，最多 {MAX_NAME_CHARS} 字"));
        }
        if self.description.chars().count() > MAX_DESCRIPTION_CHARS {
            return Err(format!("说明最多 {MAX_DESCRIPTION_CHARS} 字"));
        }
        validate_params(&mut self.params)?;
        if let Some(schedule) = &mut self.schedule {
            schedule.cron = schedule.cron.trim().to_string();
            super::cron::Cron::parse(&schedule.cron)?;
            validate_args(&self.params, &schedule.args)?;
        }
        if let Some(on) = &self.on {
            self.on = Some(validate_trigger(on, events)?);
        }
        let AutomationBody::Program(program) = &mut self.action;
        program.source = program.source.trim().to_string();
        let chars = program.source.chars().count();
        if chars == 0 || chars > MAX_SOURCE_CHARS {
            return Err(format!("自动化内容为空或超过 {MAX_SOURCE_CHARS} 字"));
        }
        for tool in tools_called(&program.source)? {
            if is_automation_tool(&tool) {
                return Err(format!(
                    "程序里不能调自动化自己的工具（{tool}）：会绕圈，也能绕过确认"
                ));
            }
        }
        Ok(self)
    }
}

/// 自动化自己的工具（运行、改、停自动化，`saved.<id>` 也是运行自动化）：程序里不能调（ADR 046 D2）。
pub fn is_automation_tool(name: &str) -> bool {
    name.starts_with("automation.") || name.starts_with("saved.")
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct StoredAutomations {
    version: u32,
    next_id: u64,
    automations: Vec<Automation>,
    /// 已经装过的内置自动化（删了也记着，不再装回）。
    builtin: Vec<String>,
}

/// 存盘文件（读）：自动化可能还是旧写法。
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredAutomationsFile {
    version: u32,
    next_id: u64,
    automations: Vec<StoredAutomation>,
    #[serde(default)]
    builtin: Vec<String>,
}

/// “X 秒之后提醒我 xxx”：到点用语音和弹幕说一句（提醒的固定程序，ADR 045 D3）。
fn builtin_remind(now: u64) -> Automation {
    Automation {
        id: BUILTIN_REMIND_ID.into(),
        name: "X 秒之后提醒我".into(),
        description: "过一会儿用语音和弹幕提醒你一句话，例如 300 秒后提醒“打小龙”。".into(),
        params: vec![
            ParamSpec {
                name: DELAY_PARAM.into(),
                label: "几秒之后".into(),
                description: "最多一天（86400 秒）".into(),
                kind: ParamKind::Number {
                    min: Some(1.0),
                    max: Some(86_400.0),
                    integer: true,
                },
                default: Some(Value::from(60)),
            },
            ParamSpec {
                name: "text".into(),
                label: "提醒什么".into(),
                description: String::new(),
                kind: ParamKind::Text { max_length: 100 },
                default: None,
            },
        ],
        action: AutomationBody::Program(ProgramBody {
            source: reminder_source(&[DeliveryKind::Voice, DeliveryKind::Danmaku]).into(),
        }),
        created_by: CreatedBy::User,
        created_at_ms: now,
        updated_at_ms: now,
        schedule: None,
        on: None,
        confirmed: BTreeMap::new(),
    }
}

fn param(name: &str, label: &str, description: &str, kind: ParamKind, default: Value) -> ParamSpec {
    ParamSpec {
        name: name.into(),
        label: label.into(),
        description: description.into(),
        kind,
        default: Some(default),
    }
}

fn number(min: f64, max: f64) -> ParamKind {
    ParamKind::Number {
        min: Some(min),
        max: Some(max),
        integer: true,
    }
}

fn program(
    id: &str,
    name: &str,
    description: &str,
    params: Vec<ParamSpec>,
    source: &str,
    now: u64,
) -> Automation {
    Automation {
        id: id.into(),
        name: name.into(),
        description: description.into(),
        params,
        action: AutomationBody::Program(ProgramBody {
            source: source.trim().to_string() + "\n",
        }),
        created_by: CreatedBy::User,
        created_at_ms: now,
        updated_at_ms: now,
        schedule: None,
        on: None,
        confirmed: BTreeMap::new(),
    }
}

/// 内置模板“英雄联盟进加载界面播报突破口”（ADR 036 D8）：每次触发花一次 AI 调用，所以默认关。
pub const BUILTIN_LOL_SCOUT_ID: &str = "builtin-lol-scout-on-loading";

/// 交给她的那句话（以前是“让她做”，ADR 045 起是程序里的 `ask()`）。
const LOL_SCOUT_ASK: &str =
    "这局进加载界面了。用 lol.scout 看看对面谁是突破口，用一句话告诉我原因（说成信息，不贴标签）。";

fn builtin_lol_scout(now: u64) -> Automation {
    let mut matches = Map::new();
    matches.insert("to".into(), Value::from("loading"));
    Automation {
        id: BUILTIN_LOL_SCOUT_ID.into(),
        name: "英雄联盟进加载界面播报突破口".into(),
        description: "进加载界面时她查一下这局 10 个人的战绩，用一句话告诉你对面谁是突破口。每次花一次 AI 调用，默认关。".into(),
        params: vec![],
        action: AutomationBody::Program(ProgramBody {
            source: format!("ask({})", lua_string(LOL_SCOUT_ASK)),
        }),
        created_by: CreatedBy::User,
        created_at_ms: now,
        updated_at_ms: now,
        schedule: None,
        on: Some(EventTrigger {
            event: vec!["game.phase_changed".into()],
            game: Some("lol".into()),
            matches,
            for_seconds: 0,
            mode: Default::default(),
            once_per_match: true,
            cooldown_seconds: 0,
            enabled: false,
        }),
        confirmed: BTreeMap::new(),
    }
}

/// 内置的常用按键程序（照 AutoHotkey / 按键精灵的常见宏：连发、连点、按住、连招、定时喊话、
/// 看到文字就按、看到颜色就按）。都是普通的已保存自动化：玩家可以改、删（删了不再装回），
/// 琴音可以直接调用（`saved.<id>`）。按键间隔不能低于脚本引擎的 50 ms，单次最长 30 分钟。
pub const BUILTIN_PROGRAM_IDS: &[&str] = &[
    "builtin-key-repeat",
    "builtin-auto-click",
    "builtin-hold-key",
    "builtin-key-sequence",
    "builtin-chat-repeat",
    "builtin-text-then-key",
    "builtin-color-then-key",
];

const CHAT_REPEAT_ID: &str = "builtin-chat-repeat";

/// 定时喊话经本体的“发到游戏”入口（ADR 041 D1）：聊天键、频道归游戏配置，等修饰键、一把输入锁都在入口里。
const CHAT_REPEAT_SOURCE: &str = r#"
for i = 1, args.times do
  chat(args.text)
  if i < args.times then
    wait(args.interval_seconds * 1000)
  end
end
"#;

/// 0.2.0-rc.8 及以前装的那份：自己按“回车—打字—回车”，开聊天框的键写在参数 `open_key` 里、发送写死回车
/// （CS2 是 U 开框，写死的回车不对）。没被改过的换成上面那份（[`refresh_builtin`]）。
const CHAT_REPEAT_SOURCE_BEFORE_ADR041: &str = r#"
for i = 1, args.times do
  key(args.open_key)
  wait(200)
  type_text(args.text)
  wait(100)
  key("Enter")
  if i < args.times then
    wait(args.interval_seconds * 1000)
  end
end
"#;

/// 早先装的内置程序里，程序一字没改过、现在的定义变了的：换成现在的程序和说明，参数照现在的定义，
/// 玩家改过的缺省值（同名参数）保留。改过程序的不动（那是玩家自己的了）。返回换了哪些。
fn refresh_builtin(automations: &mut [Automation], now: u64) -> Vec<String> {
    let current = builtin_programs(now);
    let mut refreshed = Vec::new();
    for stored in automations.iter_mut() {
        let old = match stored.id.as_str() {
            CHAT_REPEAT_ID => CHAT_REPEAT_SOURCE_BEFORE_ADR041,
            _ => continue,
        };
        // 装的时候存的是 `source.trim() + "\n"`（[`program`]）
        if stored.action.source().trim() != old.trim() {
            continue;
        }
        let Some(next) = current.iter().find(|a| a.id == stored.id) else {
            continue;
        };
        let params = next
            .params
            .iter()
            .map(|p| {
                let mut p = p.clone();
                if let Some(kept) = stored.params.iter().find(|s| s.name == p.name) {
                    p.default = kept.default.clone();
                }
                p
            })
            .collect();
        stored.params = params;
        stored.description = next.description.clone();
        stored.action = next.action.clone();
        // 程序换了：确认作废（ADR 046 D3）
        stored.confirmed.clear();
        stored.updated_at_ms = now;
        refreshed.push(stored.id.clone());
    }
    refreshed
}

fn builtin_programs(now: u64) -> Vec<Automation> {
    use serde_json::json;
    let key = |default: &str| {
        param(
            "key",
            "按哪个键",
            "例如 Q、F、Space、Ctrl+1",
            ParamKind::Key,
            json!(default),
        )
    };
    vec![
        program(
            "builtin-key-repeat",
            "每隔几毫秒按一个键",
            "连发：每隔一段时间按一次同一个键，按到时间为止（例如每 500 毫秒按一次 F 拾取）。",
            vec![
                key("F"),
                param("interval_ms", "每隔几毫秒", "最快 50 毫秒", number(50.0, 60_000.0), json!(500)),
                param("seconds", "一共按多少秒", "最长 30 分钟", number(1.0, 1_800.0), json!(60)),
            ],
            r#"
every(args.interval_ms, function()
  key(args.key)
end, { for_ms = args.seconds * 1000 })
"#,
            now,
        ),
        program(
            "builtin-auto-click",
            "连点鼠标",
            "连点器：每隔一段时间在指定位置点一下鼠标（位置取区域的中心），点到时间为止。",
            vec![
                param("area", "点哪里", "在游戏窗口里框一块，点它的中心", ParamKind::Region, json!({"x": 0.45, "y": 0.45, "w": 0.1, "h": 0.1})),
                param("button", "哪个键", "", ParamKind::Choice { options: vec!["左键".into(), "右键".into()] }, json!("左键")),
                param("interval_ms", "每隔几毫秒", "最快 50 毫秒", number(50.0, 60_000.0), json!(200)),
                param("seconds", "一共点多少秒", "最长 30 分钟", number(1.0, 1_800.0), json!(10)),
            ],
            r#"
local x = args.area.x + args.area.w / 2
local y = args.area.y + args.area.h / 2
local button = "left"
if args.button == "右键" then button = "right" end
every(args.interval_ms, function()
  click(x, y, { button = button })
end, { for_ms = args.seconds * 1000 })
"#,
            now,
        ),
        program(
            "builtin-hold-key",
            "按住一个键几秒",
            "按住不放（例如自动往前跑、蓄力），到时间松开；中途停止也会松开。",
            vec![
                key("W"),
                param("seconds", "按住几秒", "最长 30 分钟", number(1.0, 1_800.0), json!(10)),
            ],
            r#"
key_down(args.key)
wait(args.seconds * 1000)
key_up(args.key)
"#,
            now,
        ),
        program(
            "builtin-key-sequence",
            "按一串键（连招）",
            "按顺序按一串键，键之间等一会儿，可以重复几遍（例如 Q W E R 连招）。",
            vec![
                param("keys", "按哪些键", "用空格隔开，例如 Q W E R", ParamKind::Text { max_length: 100 }, json!("Q W E R")),
                param("interval_ms", "每个键之间等几毫秒", "", number(50.0, 10_000.0), json!(150)),
                param("times", "重复几遍", "", number(1.0, 100.0), json!(1)),
            ],
            r#"
for round = 1, args.times do
  for k in string.gmatch(args.keys, "%S+") do
    key(k)
    wait(args.interval_ms)
  end
end
"#,
            now,
        ),
        program(
            CHAT_REPEAT_ID,
            "每隔几秒在聊天里发一句",
            "定时喊话：往游戏聊天里发一句（聊天键和频道用游戏配置里的），隔一会儿再发，发够次数为止。别刷屏。",
            vec![
                param("text", "发什么", "", ParamKind::Text { max_length: 100 }, json!("组队来人")),
                param("interval_seconds", "每隔几秒", "", number(5.0, 1_800.0), json!(60)),
                param("times", "发几次", "", number(1.0, 20.0), json!(3)),
            ],
            CHAT_REPEAT_SOURCE,
            now,
        ),
        program(
            "builtin-text-then-key",
            "看到某段文字就点它或按键",
            "盯着画面上的一块区域，认出这段文字就点它（或按一个键），例如“接受”出现就点接受。每隔一会儿识别一次文字；用在线文字识别会按次计费。",
            vec![
                param("text", "等哪段文字", "", ParamKind::Text { max_length: 40 }, json!("接受")),
                param("area", "在哪里找", "框小一点识别更快", ParamKind::Region, json!({"x": 0.0, "y": 0.0, "w": 1.0, "h": 1.0})),
                param("click_it", "看到后点它", "关掉就按下面的键", ParamKind::Boolean, json!(true)),
                key("Enter"),
                param("check_ms", "每隔几毫秒看一次", "", number(500.0, 60_000.0), json!(1500)),
                param("minutes", "最多等几分钟", "", number(1.0, 30.0), json!(10)),
            ],
            r#"
local done = false
every(args.check_ms, function()
  local x, y = find_text(args.area, args.text)
  if x then
    if args.click_it then
      click(x, y)
    else
      key(args.key)
    end
    done = true
    return false
  end
end, { for_ms = args.minutes * 60000 })
if not done then
  toast("没等到：" .. args.text)
end
"#,
            now,
        ),
        program(
            "builtin-color-then-key",
            "看到某个颜色就按键",
            "盯着一块区域的颜色（例如血条），这个颜色出现（或消失）时按一个键（例如喝药），按完歇一会儿再看。",
            vec![
                param("area", "看哪里", "框住血条之类的一小块", ParamKind::Region, json!({"x": 0.05, "y": 0.9, "w": 0.2, "h": 0.04})),
                param("color", "什么颜色", "", ParamKind::Color, json!({"r": 200, "g": 30, "b": 30})),
                param("when", "什么时候按", "", ParamKind::Choice { options: vec!["颜色消失时".into(), "颜色出现时".into()] }, json!("颜色消失时")),
                key("1"),
                param("variation", "颜色容差", "0 = 完全一样，越大越宽松", number(0.0, 255.0), json!(30)),
                param("check_ms", "每隔几毫秒看一次", "", number(100.0, 10_000.0), json!(500)),
                param("cooldown_ms", "按完歇几毫秒", "", number(0.0, 60_000.0), json!(3000)),
                param("minutes", "一共盯几分钟", "最长 30 分钟", number(1.0, 30.0), json!(30)),
            ],
            r#"
local want_present = args.when == "颜色出现时"
every(args.check_ms, function()
  local x = find_color(args.area, args.color, { variation = args.variation })
  local present = x ~= nil
  if present == want_present then
    key(args.key)
    if args.cooldown_ms > 0 then
      wait(args.cooldown_ms)
    end
  end
end, { for_ms = args.minutes * 60000 })
"#,
            now,
        ),
    ]
}

impl Default for StoredAutomations {
    fn default() -> Self {
        Self {
            version: STORE_VERSION,
            next_id: 1,
            automations: vec![],
            builtin: vec![],
        }
    }
}

/// 已保存自动化的存储（原子写入；每次修改整份落盘）。
pub struct AutomationLibrary {
    path: PathBuf,
    state: Mutex<StoredAutomations>,
}

impl AutomationLibrary {
    /// 打开。旧写法（播报、让她做）在这里转成程序并存盘（ADR 045 D1）；`version` 还是 1（rc.13 认得
    /// `program`，读新文件不出错）。存盘失败只记日志：内存里已经转好，下次保存时一起写。
    pub fn open(path: PathBuf) -> Result<Self, String> {
        let (stored, upgraded) = match std::fs::read(&path) {
            Ok(bytes) => {
                if bytes.len() > 8 * 1024 * 1024 {
                    return Err("自动化文件超过大小上限".into());
                }
                let file: StoredAutomationsFile = serde_json::from_slice(&bytes)
                    .map_err(|e| format!("自动化文件无法读取，已保留原文件：{e}"))?;
                if file.version != STORE_VERSION {
                    return Err("自动化文件版本暂不支持，已保留原文件".into());
                }
                let mut upgraded = false;
                let automations = file
                    .automations
                    .into_iter()
                    .map(|stored| {
                        let (automation, changed) = stored.upgrade();
                        upgraded |= changed;
                        automation
                    })
                    .collect();
                let stored = StoredAutomations {
                    version: file.version,
                    next_id: file.next_id,
                    automations,
                    builtin: file.builtin,
                };
                (stored, upgraded)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                (StoredAutomations::default(), false)
            }
            Err(e) => return Err(format!("读取自动化文件失败：{e}")),
        };
        let library = Self {
            path,
            state: Mutex::new(stored),
        };
        if upgraded {
            if let Err(e) = library.persist(&library.state.lock().unwrap()) {
                crate::log::log(&format!("automation library: upgraded but not saved: {e}"));
            }
        }
        Ok(library)
    }

    /// 全部定义，按名字排序。
    pub fn list(&self) -> Vec<Automation> {
        let mut list = self.state.lock().unwrap().automations.clone();
        list.sort_by(|a, b| (&a.name, &a.id).cmp(&(&b.name, &b.id)));
        list
    }

    pub fn get(&self, id: &str) -> Option<Automation> {
        self.state
            .lock()
            .unwrap()
            .automations
            .iter()
            .find(|a| a.id == id)
            .cloned()
    }

    /// 新建（`draft.id` 为空）或覆盖已有定义。覆盖时保留创建者与创建时间；程序变了，玩家对它调工具的确认清掉
    /// （ADR 046 D3：谁改的都算；只改名字、参数、触发不动它）。程序里的工具名要直接写、不能调自动化自己的工具。
    /// `events`：合并后的事件目录（宿主按插件开关算好），游戏事件触发按它校验。
    pub fn save(
        &self,
        draft: AutomationDraft,
        by: CreatedBy,
        events: &EventCatalog,
    ) -> Result<Automation, String> {
        let draft = draft.normalized(events)?;
        let mut state = self.state.lock().unwrap();
        let mut next = state.clone();
        let now = now_ms();
        let saved = match draft.id.as_deref().filter(|id| !id.is_empty()) {
            Some(id) => {
                let existing = next
                    .automations
                    .iter_mut()
                    .find(|a| a.id == id)
                    .ok_or_else(|| format!("找不到自动化 {id}"))?;
                if existing.action.source().trim() != draft.action.source() {
                    existing.confirmed.clear();
                }
                existing.name = draft.name;
                existing.description = draft.description;
                existing.params = draft.params;
                existing.action = draft.action;
                existing.schedule = draft.schedule;
                existing.on = draft.on;
                existing.updated_at_ms = now;
                existing.clone()
            }
            None => {
                if next.automations.len() >= MAX_AUTOMATIONS {
                    return Err(format!("最多保存 {MAX_AUTOMATIONS} 个自动化，请先删除一些"));
                }
                let id = format!("auto-{}", next.next_id);
                next.next_id = next.next_id.checked_add(1).ok_or("自动化编号超出范围")?;
                let automation = Automation {
                    id,
                    name: draft.name,
                    description: draft.description,
                    params: draft.params,
                    action: draft.action,
                    created_by: by,
                    created_at_ms: now,
                    updated_at_ms: now,
                    schedule: draft.schedule,
                    on: draft.on,
                    confirmed: BTreeMap::new(),
                };
                next.automations.push(automation.clone());
                automation
            }
        };
        self.persist(&next)?;
        *state = next;
        Ok(saved)
    }

    /// 装上内置的自动化（每个只装一次：玩家删了不再装回；以后新加的内置自动化老用户也会装上）。
    /// 返回这次有没有装。
    pub fn ensure_builtin(&self) -> Result<bool, String> {
        let mut state = self.state.lock().unwrap();
        let now = now_ms();
        let missing: Vec<Automation> = all_builtin(now)
            .into_iter()
            .filter(|a| !state.builtin.contains(&a.id))
            .collect();
        let mut next = state.clone();
        let refreshed = refresh_builtin(&mut next.automations, now);
        if missing.is_empty() && refreshed.is_empty() {
            return Ok(false);
        }
        for automation in missing {
            next.builtin.push(automation.id.clone());
            if next.automations.len() < MAX_AUTOMATIONS
                && !next.automations.iter().any(|a| a.id == automation.id)
            {
                next.automations.push(automation);
            }
        }
        self.persist(&next)?;
        *state = next;
        Ok(true)
    }

    /// 全部内置自动化的定义（“X 秒之后提醒我”、突破口、按键程序；测试与文档用）。
    pub fn builtin_programs_preview() -> Vec<Automation> {
        all_builtin(0)
    }

    /// 玩家在「自动化」页点“允许”（编辑器里是“允许并保存”，ADR 046 D3）：记下这段程序可以调这些工具，`map` 是
    /// 工具 → 这时它的副作用（宿主先按权限算好哪些要确认）。**只给 Kotone 窗口的命令用**，工具服务里没有这条路。
    /// `source_seen` 是玩家看到的那份程序：和现在存的不一样（这期间被她、外部 Agent 或另一个窗口改了）就不认。
    /// 和以前的确认合在一起；程序里已经不调的工具去掉。不改 `updated_at_ms`（确认不是修改）。
    pub fn confirm(
        &self,
        id: &str,
        source_seen: &str,
        map: BTreeMap<String, Vec<String>>,
    ) -> Result<Automation, String> {
        let mut state = self.state.lock().unwrap();
        let mut next = state.clone();
        let automation = next
            .automations
            .iter_mut()
            .find(|a| a.id == id)
            .ok_or_else(|| format!("找不到自动化 {id}"))?;
        if automation.action.source().trim() != source_seen.trim() {
            return Err("这段程序刚被改过：刷新一下，看过现在的程序再允许".into());
        }
        let called = tools_called(automation.action.source())?;
        for (tool, mut effects) in map {
            if !called.contains(&tool) {
                return Err(format!("程序里没有调用 {tool}，不用允许"));
            }
            effects.sort();
            effects.dedup();
            automation.confirmed.insert(tool, effects);
        }
        automation.confirmed.retain(|tool, _| called.contains(tool));
        let confirmed = automation.clone();
        self.persist(&next)?;
        *state = next;
        Ok(confirmed)
    }

    /// 清掉以 `prefix` 开头的工具的确认（第三方插件的配置变了，确认的就不是现在这个服务器的工具了；照热键的
    /// `Settings::forget_confirmations`）。返回有没有清掉什么。
    pub fn forget_confirmations(&self, prefix: &str) -> Result<bool, String> {
        let mut state = self.state.lock().unwrap();
        let mut next = state.clone();
        let mut changed = false;
        for automation in &mut next.automations {
            let before = automation.confirmed.len();
            automation
                .confirmed
                .retain(|tool, _| !tool.starts_with(prefix));
            changed |= automation.confirmed.len() != before;
        }
        if !changed {
            return Ok(false);
        }
        self.persist(&next)?;
        *state = next;
        Ok(true)
    }

    /// 删除；返回是否存在。已安排的执行保存了快照，不受影响。
    pub fn delete(&self, id: &str) -> Result<bool, String> {
        let mut state = self.state.lock().unwrap();
        if !state.automations.iter().any(|a| a.id == id) {
            return Ok(false);
        }
        let mut next = state.clone();
        next.automations.retain(|a| a.id != id);
        self.persist(&next)?;
        *state = next;
        Ok(true)
    }

    fn persist(&self, stored: &StoredAutomations) -> Result<(), String> {
        let bytes = serde_json::to_vec(stored).map_err(|e| e.to_string())?;
        write_atomic(&self.path, &bytes, "自动化")
    }
}

fn all_builtin(now: u64) -> Vec<Automation> {
    let mut all = vec![builtin_remind(now), builtin_lol_scout(now)];
    all.extend(builtin_programs(now));
    all
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn spec(value: Value) -> ParamSpec {
        serde_json::from_value(value).unwrap()
    }

    fn args(value: Value) -> Map<String, Value> {
        serde_json::from_value(value).unwrap()
    }

    fn draft(value: Value) -> AutomationDraft {
        serde_json::from_value(value).unwrap()
    }

    fn source(draft: &AutomationDraft) -> &str {
        draft.action.source()
    }

    fn press_every() -> AutomationDraft {
        draft(json!({
            "name": "  连按某键 ",
            "description": "每隔一段时间按一次",
            "params": [
                {"name": "key", "label": "按键", "type": "key", "default": "Q"},
                {"name": "interval", "type": "number", "min": 50, "max": 10000, "integer": true, "default": 1000},
                {"name": "area", "type": "region"},
            ],
            "action": {"kind": "program", "source": "every(args.interval, function() key(args.key) end)"},
        }))
    }

    /// ADR 045 D1：旧播报转成拼接的 `say` / `toast`，显示和以前的 `{参数}` 替换一样。
    #[test]
    fn old_speak_drafts_become_programs_that_say_the_same_words() {
        let speak = |text: &str, delivery: Value, params: Value| {
            draft(json!({"name": "提醒", "params": params,
                "action": {"kind": "speak", "text": text, "delivery": delivery}}))
        };
        let text_and_delay = json!([
            {"name": "delay_seconds", "type": "number", "min": 1, "integer": true, "default": 5},
            {"name": "text", "type": "text"}]);
        let cases = [
            (
                speak("该{text}了", json!(["voice"]), text_and_delay.clone()),
                r#"say("该" .. args.text .. "了")"#,
            ),
            (
                speak(
                    "{text}",
                    json!(["voice", "danmaku"]),
                    text_and_delay.clone(),
                ),
                "say(args.text, { toast = true })",
            ),
            (
                speak(" 喝水 ", json!(["danmaku"]), json!([])),
                r#"toast("喝水")"#,
            ),
            (
                speak(
                    "{delay_seconds} 秒到了，{loud}{who}",
                    json!(["danmaku", "voice"]),
                    json!([
                        {"name": "delay_seconds", "type": "number", "default": 5},
                        {"name": "loud", "type": "boolean", "default": true},
                        {"name": "who", "type": "choice", "options": ["我", "你"], "default": "我"}]),
                ),
                r#"say(tostring(args.delay_seconds) .. " 秒到了，" .. (args.loud and "是" or "否") .. args.who, { toast = true })"#,
            ),
            (
                speak("说 \"{x\\n", json!(["voice"]), json!([])),
                r#"say("说 \"{x\\n")"#,
            ),
        ];
        for (draft, want) in cases {
            assert_eq!(source(&draft), want);
            // 转出来的定义照样能保存；延迟参数留着（对所有自动化都有效，D7）
            let ok = draft.normalized(&EventCatalog::generic()).unwrap();
            assert!(!ok.action.uses().window, "{want}");
        }
        let filled = validate_args(
            &speak("{text}", json!(["voice"]), text_and_delay.clone()).params,
            &args(json!({"text": "打小龙"})),
        )
        .unwrap();
        assert_eq!(
            delay_ms(
                &speak("{text}", json!(["voice"]), text_and_delay).params,
                &filled
            ),
            5_000
        );
        // 旧写法照旧规矩查：不认识的 {参数}、空话、重复的回答方式
        for (bad, want) in [
            (
                json!({"name": "x", "action": {"kind": "speak", "text": "{who}", "delivery": ["voice"]}}),
                "{who}",
            ),
            (
                json!({"name": "x", "action": {"kind": "speak", "text": " ", "delivery": ["voice"]}}),
                "1 到 300",
            ),
            (
                json!({"name": "x", "action": {"kind": "speak", "text": "x", "delivery": ["voice", "voice"]}}),
                "不能重复",
            ),
        ] {
            let error = serde_json::from_value::<AutomationDraft>(bad.clone())
                .unwrap_err()
                .to_string();
            assert!(error.contains(want), "{bad} → {error}");
        }
        // 不再限制参数类型（ADR 045“删掉的规矩”）
        let region = speak(
            "看这里",
            json!(["voice"]),
            json!([{"name": "area", "type": "region", "default": {"x": 0, "y": 0, "w": 1, "h": 1}}]),
        );
        assert!(region.normalized(&EventCatalog::generic()).is_ok());
    }

    /// “让她做”转 `ask("…")`：引号、反斜杠、换行转义正确；不再限制参数。
    #[test]
    fn old_prompt_drafts_become_ask_with_escaped_text() {
        let ok = draft(json!({
            "name": "进加载界面说突破口",
            "action": {"kind": "prompt", "text": "  看看\"对面\"谁是突破口\\\n说一句  "},
            "params": [{"name": "n", "type": "number", "default": 1}],
            "on": {"event": ["game.phase_changed"], "game": "lol", "match": {"to": "loading"}, "oncePerMatch": true}
        }))
        .normalized(&EventCatalog::generic())
        .unwrap();
        assert_eq!(source(&ok), r#"ask("看看\"对面\"谁是突破口\\\n说一句")"#);
        assert_eq!(
            ok.action.uses(),
            ProgramUses {
                window: false,
                asks: true,
                speaks: false
            }
        );
        assert!(ok.on.as_ref().unwrap().once_per_match);
        for (bad, want) in [
            (
                json!({"name": "x", "action": {"kind": "prompt", "text": " "}}),
                "不能为空",
            ),
            (
                json!({"name": "x", "action": {"kind": "prompt", "text": "a".repeat(501)}}),
                "500",
            ),
            (
                json!({"name": "x", "action": {"kind": "prompt", "text": "说一句"}, "when": 1}),
                "when",
            ),
        ] {
            let error = serde_json::from_value::<AutomationDraft>(bad.clone())
                .unwrap_err()
                .to_string();
            assert!(error.contains(want), "{bad} → {error}");
        }
        let unknown_event = draft(json!({"name": "x",
            "action": {"kind": "speak", "text": "龙", "delivery": ["voice"]},
            "on": {"event": ["lol.dragon"]}}));
        let error = unknown_event
            .normalized(&EventCatalog::generic())
            .unwrap_err();
        assert!(error.contains("没有"), "{error}");
        assert_eq!(
            lua_string("a\tb\u{1}c"),
            r#""a\tb\u{1}c""#,
            "控制字符用 \\u{{…}}"
        );
    }

    /// 老用户的 `automations.json`：播报、让她做转成程序并存盘，版本还是 1，编号和内置记录不动。
    #[test]
    fn an_old_library_file_is_upgraded_and_saved() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("automations.json");
        let old = json!({
            "version": 1, "nextId": 3, "builtin": [BUILTIN_REMIND_ID, BUILTIN_LOL_SCOUT_ID],
            "automations": [
                {"id": BUILTIN_REMIND_ID, "name": "X 秒之后提醒我", "params": [
                    {"name": "delay_seconds", "type": "number", "min": 1, "max": 86400, "integer": true, "default": 60},
                    {"name": "text", "type": "text", "maxLength": 100}],
                 "action": {"kind": "speak", "text": "{text}", "delivery": ["voice", "danmaku"]},
                 "createdBy": "user", "createdAtMs": 1, "updatedAtMs": 1},
                {"id": BUILTIN_LOL_SCOUT_ID, "name": "突破口", "params": [],
                 "action": {"kind": "prompt", "text": LOL_SCOUT_ASK},
                 "createdBy": "user", "createdAtMs": 1, "updatedAtMs": 1,
                 "on": {"event": ["game.phase_changed"], "enabled": false}},
                {"id": "auto-2", "name": "按 Q", "params": [],
                 "action": {"kind": "program", "source": "key('Q')"},
                 "createdBy": "agent", "createdAtMs": 1, "updatedAtMs": 1}
            ]
        });
        std::fs::write(&path, serde_json::to_vec(&old).unwrap()).unwrap();
        let library = AutomationLibrary::open(path.clone()).unwrap();
        // 老用户装过的内置项转成的程序和新装的一字不差
        let fresh = all_builtin(0);
        for id in [BUILTIN_REMIND_ID, BUILTIN_LOL_SCOUT_ID] {
            let want = fresh.iter().find(|a| a.id == id).unwrap();
            assert_eq!(library.get(id).unwrap().action, want.action, "{id}");
        }
        assert_eq!(library.get("auto-2").unwrap().action.source(), "key('Q')");
        let saved: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(saved["version"], 1);
        assert_eq!(saved["nextId"], 3);
        assert!(saved["automations"]
            .as_array()
            .unwrap()
            .iter()
            .all(|a| a["action"]["kind"] == "program" && a["action"].get("text").is_none()));
        // 已经是新写法：打开不再写盘
        let before = std::fs::metadata(&path).unwrap().modified().unwrap();
        drop(library);
        std::thread::sleep(std::time::Duration::from_millis(20));
        let reopened = AutomationLibrary::open(path.clone()).unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().modified().unwrap(),
            before
        );
        assert_eq!(reopened.list().len(), 3);
        // `Automation` 本身也认得旧写法（别处存过的定义）
        let one: Automation = serde_json::from_value(old["automations"][0].clone()).unwrap();
        assert_eq!(one.action.source(), "say(args.text, { toast = true })");
    }

    #[test]
    fn schedules_are_validated_and_saved() {
        let mut draft = press_every();
        draft.params.retain(|p| p.name != "area");
        draft.schedule = Some(Schedule {
            cron: " 0 8 * * * ".into(),
            enabled: true,
            args: args(json!({"key": "E"})),
        });
        let ok = draft.clone().normalized(&EventCatalog::generic()).unwrap();
        assert_eq!(ok.schedule.as_ref().unwrap().cron, "0 8 * * *");
        draft.schedule = Some(Schedule {
            cron: "8 点".into(),
            enabled: true,
            args: Map::new(),
        });
        assert!(draft.clone().normalized(&EventCatalog::generic()).is_err());
        draft.schedule = Some(Schedule {
            cron: "@daily".into(),
            enabled: true,
            args: args(json!({"nope": 1})),
        });
        assert!(draft.normalized(&EventCatalog::generic()).is_err());
    }

    fn as_draft(a: &Automation) -> AutomationDraft {
        AutomationDraft {
            id: None,
            name: a.name.clone(),
            description: a.description.clone(),
            params: a.params.clone(),
            action: a.action.clone(),
            schedule: a.schedule.clone(),
            on: a.on.clone(),
        }
    }

    /// 新装的“X 秒之后提醒我”直接是提醒的程序（ADR 045 D1、D3）；删了不装回。
    #[test]
    fn the_builtin_reminder_is_installed_once_and_stays_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let library = AutomationLibrary::open(dir.path().join("automations.json")).unwrap();
        assert!(library.ensure_builtin().unwrap());
        let builtin = library.get(BUILTIN_REMIND_ID).unwrap();
        assert_eq!(builtin.name, "X 秒之后提醒我");
        assert_eq!(builtin.action.source(), "say(args.text, { toast = true })");
        assert!(!builtin.action.uses().window);
        let filled = validate_args(&builtin.params, &args(json!({"text": "打小龙"}))).unwrap();
        assert_eq!(delay_ms(&builtin.params, &filled), 60_000);
        // 装上的定义自己也要能通过校验
        as_draft(&builtin)
            .normalized(&EventCatalog::generic())
            .unwrap();
        assert!(!library.ensure_builtin().unwrap());
        library.delete(BUILTIN_REMIND_ID).unwrap();
        let reopened = AutomationLibrary::open(dir.path().join("automations.json")).unwrap();
        assert!(!reopened.ensure_builtin().unwrap());
        assert!(reopened.get(BUILTIN_REMIND_ID).is_none());
    }

    /// ADR 041：老用户手里那份“定时喊话”（自己按回车）没改过的换成经入口的 `chat()`，改过的不动。
    #[test]
    fn an_unedited_old_chat_repeat_is_refreshed_and_an_edited_one_is_kept() {
        let old_params = || {
            vec![
                param(
                    "text",
                    "发什么",
                    "",
                    ParamKind::Text { max_length: 100 },
                    json!("组队来人"),
                ),
                param(
                    "open_key",
                    "打开聊天框的键",
                    "",
                    ParamKind::Key,
                    json!("Enter"),
                ),
                param(
                    "interval_seconds",
                    "每隔几秒",
                    "",
                    number(5.0, 1_800.0),
                    json!(60),
                ),
                param("times", "发几次", "", number(1.0, 20.0), json!(3)),
            ]
        };
        let installed = |source: &str| {
            let mut a = program(
                CHAT_REPEAT_ID,
                "每隔几秒在聊天里发一句",
                "旧说明",
                old_params(),
                source,
                1,
            );
            a.params[0].default = Some(json!("来个辅助")); // 玩家改过缺省的话
            a
        };
        let all_ids: Vec<String> = [BUILTIN_REMIND_ID, BUILTIN_LOL_SCOUT_ID]
            .iter()
            .chain(BUILTIN_PROGRAM_IDS)
            .map(|s| s.to_string())
            .collect();
        let write = |path: &std::path::Path, a: Automation| {
            let stored = StoredAutomations {
                version: 1,
                next_id: 1,
                automations: vec![a],
                builtin: all_ids.clone(),
            };
            std::fs::write(path, serde_json::to_vec(&stored).unwrap()).unwrap();
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("automations.json");

        write(&path, installed(CHAT_REPEAT_SOURCE_BEFORE_ADR041));
        let library = AutomationLibrary::open(path.clone()).unwrap();
        assert!(library.ensure_builtin().unwrap(), "换了要写盘");
        let a = library.get(CHAT_REPEAT_ID).unwrap();
        let source = a.action.source();
        assert!(source.contains("chat(args.text)") && !source.contains("key("));
        let names: Vec<&str> = a.params.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(
            names,
            ["text", "interval_seconds", "times"],
            "开聊天框的键归游戏配置"
        );
        assert_eq!(
            a.params[0].default,
            Some(json!("来个辅助")),
            "玩家改过的缺省值保留"
        );
        assert!(!library.ensure_builtin().unwrap(), "只换一次");
        let reopened = AutomationLibrary::open(path.clone()).unwrap();
        assert!(!reopened.ensure_builtin().unwrap());

        let edited = CHAT_REPEAT_SOURCE_BEFORE_ADR041.replace("wait(200)", "wait(300)");
        write(&path, installed(&edited));
        let library = AutomationLibrary::open(path).unwrap();
        assert!(!library.ensure_builtin().unwrap());
        let a = library.get(CHAT_REPEAT_ID).unwrap();
        assert!(
            a.action.source().contains("wait(300)"),
            "玩家改过的程序不动"
        );
    }

    #[test]
    fn builtin_programs_are_valid_installed_for_old_users_and_stay_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("automations.json");
        // 老用户：只装过“X 秒之后提醒我”
        std::fs::write(
            &path,
            json!({"version": 1, "nextId": 1, "automations": [], "builtin": [BUILTIN_REMIND_ID]})
                .to_string(),
        )
        .unwrap();
        let library = AutomationLibrary::open(path.clone()).unwrap();
        assert!(library.ensure_builtin().unwrap());
        assert!(library.get(BUILTIN_REMIND_ID).is_none(), "删过的不装回");
        assert!(
            library.get(BUILTIN_LOL_SCOUT_ID).is_some(),
            "新加的内置项老用户也装上"
        );
        for id in BUILTIN_PROGRAM_IDS {
            let a = library.get(id).unwrap_or_else(|| panic!("{id}"));
            let draft = as_draft(&a)
                .normalized(&EventCatalog::generic())
                .unwrap_or_else(|e| panic!("{id}: {e}"));
            // 缺省值本身能通过入参校验（什么都不填也能跑）
            validate_args(&draft.params, &Map::new()).unwrap_or_else(|e| panic!("{id}: {e}"));
            assert!(a.action.uses().window, "按键程序都用到游戏窗口：{id}");
        }
        assert!(!library.ensure_builtin().unwrap());
        library.delete("builtin-key-repeat").unwrap();
        let reopened = AutomationLibrary::open(path).unwrap();
        assert!(!reopened.ensure_builtin().unwrap());
        assert!(reopened.get("builtin-key-repeat").is_none());
    }

    #[test]
    fn args_are_checked_against_the_declaration_and_defaults_filled() {
        let params = press_every()
            .normalized(&EventCatalog::generic())
            .unwrap()
            .params;
        let area = json!({"x": 0.1, "y": 0.2, "w": 0.3, "h": 0.4});
        let ok = validate_args(&params, &args(json!({"area": area, "key": " E "}))).unwrap();
        assert_eq!(ok["key"], "E");
        assert_eq!(ok["interval"], 1000);
        assert_eq!(ok["area"]["w"], 0.3);
        for (bad, why) in [
            (json!({}), "缺少参数 area"),
            (json!({"area": area, "nope": 1}), "没有名为 nope"),
            (json!({"area": area, "interval": 10}), "超出范围"),
            (json!({"area": area, "interval": 100.5}), "整数"),
            (json!({"area": area, "key": "Q; rm"}), "按键名"),
            (
                json!({"area": {"x": 0.9, "y": 0, "w": 0.5, "h": 0.1}}),
                "超出窗口",
            ),
            (
                json!({"area": {"x": 0, "y": 0, "w": 0.5, "h": 0.1, "z": 1}}),
                "只能有",
            ),
            (json!({"area": [0, 0, 1, 1]}), "区域"),
        ] {
            let err = validate_args(&params, &args(bad.clone())).unwrap_err();
            assert!(err.contains(why), "{bad}: {err}");
        }
        let color = [spec(json!({"name": "c", "type": "color"}))];
        assert!(validate_args(&color, &args(json!({"c": {"r": 1, "g": 2, "b": 3}}))).is_ok());
        assert!(validate_args(&color, &args(json!({"c": {"r": 256, "g": 2, "b": 3}}))).is_err());
        let choice = [spec(
            json!({"name": "mode", "type": "choice", "options": ["快", "慢"]}),
        )];
        assert!(validate_args(&choice, &args(json!({"mode": "快"}))).is_ok());
        assert!(validate_args(&choice, &args(json!({"mode": "中"}))).is_err());
        let text = [spec(json!({"name": "msg", "type": "text", "maxLength": 4}))];
        assert!(validate_args(&text, &args(json!({"msg": "你好"}))).is_ok());
        assert!(validate_args(&text, &args(json!({"msg": "你好啊朋友"}))).is_err());
        assert!(validate_args(&text, &args(json!({"msg": "a\nb"}))).is_err());
        let image = [spec(json!({"name": "icon", "type": "image"}))];
        let id = "ab".repeat(32);
        assert!(validate_args(&image, &args(json!({"icon": {"image": id}}))).is_ok());
        assert!(validate_args(&image, &args(json!({"icon": {"image": "../x"}}))).is_err());
        assert!(validate_args(&image, &args(json!({"icon": {"png": "AAAA"}}))).is_err());
        let flag = [spec(
            json!({"name": "on", "type": "boolean", "default": false}),
        )];
        assert_eq!(validate_args(&flag, &Map::new()).unwrap()["on"], false);
    }

    #[test]
    fn bad_declarations_are_rejected_when_saving() {
        let mut bad_name = press_every();
        bad_name.params[0].name = "Key".into();
        let mut dup = press_every();
        dup.params[1].name = "key".into();
        let mut bad_default = press_every();
        bad_default.params[1].default = Some(json!(5));
        let mut empty = press_every();
        empty.action = AutomationBody::Program(ProgramBody {
            source: "  ".into(),
        });
        let mut too_long = press_every();
        too_long.action = AutomationBody::Program(ProgramBody {
            source: "x".repeat(MAX_SOURCE_CHARS + 1),
        });
        let mut no_name = press_every();
        no_name.name = " ".into();
        for (draft, why) in [
            (bad_name, "参数名"),
            (dup, "重复"),
            (bad_default, "缺省值"),
            (empty, "为空"),
            (too_long, "8000"),
            (no_name, "名字"),
        ] {
            let err = draft.normalized(&EventCatalog::generic()).unwrap_err();
            assert!(err.contains(why), "{err}");
        }
        assert!(serde_json::from_value::<ParamSpec>(json!({"name": "x", "type": "file"})).is_err());
    }

    #[test]
    fn library_creates_updates_deletes_and_persists() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("automations.json");
        let library = AutomationLibrary::open(path.clone()).unwrap();
        let saved = library
            .save(press_every(), CreatedBy::Agent, &EventCatalog::generic())
            .unwrap();
        assert_eq!(saved.id, "auto-1");
        assert_eq!(saved.name, "连按某键");
        assert_eq!(saved.params[1].label, "interval");
        let mut edit = press_every();
        edit.id = Some(saved.id.clone());
        edit.name = "连按".into();
        let updated = library
            .save(edit, CreatedBy::User, &EventCatalog::generic())
            .unwrap();
        assert_eq!(updated.created_by, CreatedBy::Agent, "覆盖保留创建者");
        assert_eq!(updated.created_at_ms, saved.created_at_ms);
        let mut missing = press_every();
        missing.id = Some("auto-99".into());
        assert!(library
            .save(missing, CreatedBy::User, &EventCatalog::generic())
            .is_err());
        drop(library);
        let library = AutomationLibrary::open(path.clone()).unwrap();
        assert_eq!(library.list().len(), 1);
        assert_eq!(library.get("auto-1").unwrap().name, "连按");
        let json: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(json["automations"][0]["params"][1]["type"], "number");
        assert_eq!(json["automations"][0]["action"]["kind"], "program");
        assert!(library.delete("auto-1").unwrap());
        assert!(!library.delete("auto-1").unwrap());
        assert!(library.list().is_empty());
        // 编号不复用
        assert_eq!(
            library
                .save(press_every(), CreatedBy::User, &EventCatalog::generic())
                .unwrap()
                .id,
            "auto-2"
        );
    }

    #[test]
    fn the_lol_scout_template_is_installed_switched_off_and_valid() {
        let dir = tempfile::tempdir().unwrap();
        let library = AutomationLibrary::open(dir.path().join("automations.json")).unwrap();
        library.ensure_builtin().unwrap();
        let a = library.get(BUILTIN_LOL_SCOUT_ID).unwrap();
        let on = a.on.clone().unwrap();
        assert!(!on.enabled, "每次花一次 AI 调用：默认关（ADR 036 D8）");
        assert!(on.once_per_match);
        assert!(a.action.source().starts_with("ask(\"这局进加载界面了。"));
        assert_eq!(
            a.action.uses(),
            ProgramUses {
                window: false,
                asks: true,
                speaks: false
            }
        );
        let draft = as_draft(&a).normalized(&EventCatalog::generic()).unwrap();
        assert_eq!(draft.on, a.on);
        // 老的自动化文件里没有 on：照兼容规则读成 None
        let old: Automation = serde_json::from_value(json!({
            "id": "auto-1", "name": "x", "action": {"kind": "program", "source": "key('Q')"},
            "createdBy": "user", "createdAtMs": 0, "updatedAtMs": 0
        }))
        .unwrap();
        assert!(old.on.is_none());
        assert!(!serde_json::to_string(&old).unwrap().contains("\"on\""));
    }

    /// ADR 045 D4：按源码保守判断用不用游戏窗口、会不会叫她。
    #[test]
    fn the_scanner_finds_window_functions_conservatively() {
        let uses = |source: &str| ProgramUses::of(source);
        for (source, window) in [
            ("key('Q')", true),
            ("  every(100, function() key_up(args.key) end)", true),
            ("local k = key", true),
            ("local f = _G.key f('Q')", true),
            ("if game_focused() then log('x') end", true),
            ("x = 1 .. key", true),
            ("x = `按 {key}`", true),
            ("x = `{ args.text }` say(x)", false),
            ("say(args.key)", false),
            ("say(t:find('key'))", false),
            (
                "local t = { key = 1, chat = 2 } say(tostring(t.key))",
                false,
            ),
            ("-- key('Q')\nsay('好')", false),
            ("--[[ key('Q')\nclick(0, 0) ]] say('好')", false),
            ("--[==[ key() ]] still comment ]==] toast('好')", false),
            ("say(\"key('Q')\") toast('chat(\\'x\\')')", false),
            ("local s = [[ key() ]] say(s)", false),
            ("local s = [=[ ]] key() ]=] say(s)", false),
            ("push('买眼') ask('看看')", false),
            ("wait(1.5e3) say('a') wait(0x10)", false),
            ("local keys = 1 local mykey = keys", false),
        ] {
            assert_eq!(uses(source).window, window, "{source}");
        }
        assert!(uses("ask('看看')").asks);
        assert!(uses("local f = ask f('x')").asks);
        assert!(!uses("say('ask')").asks);
        assert!(!uses("args.ask = 1").asks);
        // 说出或显示文字（ADR 046 D5：她运行它的副作用）
        for source in [
            "say('x')",
            "toast('x')",
            "push('x')",
            "local f = say f('x')",
        ] {
            assert!(uses(source).speaks, "{source}");
        }
        for source in [
            "log('say')",
            "tool('music')",
            "local t = { say = 1 }",
            "key('Q')",
        ] {
            assert!(!uses(source).speaks, "{source}");
        }
        // 调工具不算用到游戏窗口
        assert!(!uses("tool('music', { action = 'play' })").window);
        // 两张表没有重名
        for name in WINDOW_FUNCTIONS {
            assert!(!PLAIN_FUNCTIONS.contains(name), "{name}");
        }
    }

    /// D7：延迟只认声明成数字的 `delay_seconds`。
    #[test]
    fn delay_seconds_delays_any_automation_that_declares_it() {
        let declared = [spec(
            json!({"name": "delay_seconds", "type": "number", "default": 1.5}),
        )];
        assert_eq!(
            delay_ms(&declared, &args(json!({"delay_seconds": 1.5}))),
            1_500
        );
        assert_eq!(delay_ms(&declared, &args(json!({"delay_seconds": 0}))), 0);
        assert_eq!(delay_ms(&[], &args(json!({"delay_seconds": 5}))), 0);
        let text = [spec(json!({"name": "delay_seconds", "type": "text"}))];
        assert_eq!(delay_ms(&text, &args(json!({"delay_seconds": "5"}))), 0);
    }

    /// D9：四个模板能保存、只填缺省值就能通过入参校验（脚本引擎的测试另外真跑一遍）。
    #[test]
    fn templates_are_valid_drafts() {
        let all = templates();
        let ids: Vec<&str> = all.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, ["say", "push", "ask", "blank", "onedragon-push"]);
        // 一条龙的事件由第三方插件登记（宿主按它的清单并进目录）：这里照预设的写法登记一份
        let mut events = EventCatalog::generic();
        for (name, label) in [
            ("run_finished", "一条龙跑完了"),
            ("run_failed", "一条龙出错了"),
        ] {
            events
                .add(crate::games::CatalogEntry {
                    spec: crate::games::EventSpec {
                        kind: format!("{ONEDRAGON_PRESET}.{name}"),
                        label: label.into(),
                        data: [("to".to_string(), "现在的状态".to_string())].into(),
                        needs: Vec::new(),
                        private: Vec::new(),
                    },
                    adapter: ONEDRAGON_PRESET.into(),
                    plugin: Some((ONEDRAGON_PRESET.into(), ONEDRAGON_PRESET.into())),
                    games: vec!["zzz".into()],
                    off: None,
                })
                .unwrap();
        }
        for t in &all {
            let draft = AutomationDraft {
                id: None,
                name: t.name.clone(),
                description: t.description.clone(),
                params: t.params.clone(),
                action: AutomationBody::Program(ProgramBody {
                    source: t.source.clone(),
                }),
                schedule: None,
                on: t.on.clone(),
            }
            .normalized(&events)
            .unwrap_or_else(|e| panic!("{}: {e}", t.id));
            assert_eq!(t.on.is_some(), t.preset.is_some(), "{}", t.id);
            if let Some(on) = &draft.on {
                assert!(on.enabled, "选了模板就是要它：触发开着");
                let event = GameEvent {
                    id: "e".into(),
                    kind: on.event[1].clone(),
                    game: "zzz".into(),
                    match_id: String::new(),
                    time: 0,
                    game_time: None,
                    source: "third-party/zzz-onedragon".into(),
                    data: [
                        ("from".to_string(), json!("running")),
                        ("to".to_string(), json!("failed")),
                    ]
                    .into_iter()
                    .collect(),
                };
                assert!(crate::games::trigger_matches(on, &event));
                let args = event_args(&draft.params, &event);
                assert_eq!(args["to"], "failed", "事件的 to 填进参数");
                validate_args(&draft.params, &args).unwrap();
            }
            validate_args(&draft.params, &Map::new()).unwrap_or_else(|e| panic!("{}: {e}", t.id));
            let uses = ProgramUses::of(&t.source);
            assert!(!uses.window, "模板都不用游戏窗口：{}", t.id);
            assert_eq!(uses.asks, t.id == "ask", "{}", t.id);
        }
        assert!(!all[0].source.contains('{'), "“说一句”不用 {{参数}} 写法");
    }

    #[test]
    fn event_fields_fill_params_of_the_same_name() {
        let params = [
            spec(json!({"name": "killer_name", "type": "text"})),
            spec(json!({"name": "team", "type": "text"})),
            spec(json!({"name": "other", "type": "text", "default": "x"})),
        ];
        let event: GameEvent = serde_json::from_value(json!({
            "id": "e", "type": "demo.ace", "game": "demo", "time": 1, "source": "x",
            "data": {"killerName": "某人", "team": "enemy", "extra": 1}
        }))
        .unwrap();
        assert_eq!(
            Value::Object(event_args(&params, &event)),
            json!({"killer_name": "某人", "team": "enemy"})
        );
    }

    // ---------- ADR 046：程序里调工具 ----------

    /// D1：会调哪些工具按源码列（名字直接写成字符串）；名字不是直接写的一律报错。
    #[test]
    fn tools_called_lists_the_names_written_out_and_rejects_the_rest() {
        let called = |source: &str| tools_called(source);
        for (source, want) in [
            ("say('好')", vec![]),
            ("tool('music')", vec!["music"]),
            (
                "local r = tool(\"music\", { action = \"play\" }) tool('lol.match') tool('music')",
                vec!["music", "lol.match"],
            ),
            ("tool 'lol.match'", vec!["lol.match"]),
            (
                "local r = _G.tool('My-Plugin.Get_Thing')",
                vec!["My-Plugin.Get_Thing"],
            ),
            ("-- tool(x)\nsay('tool(x)')", vec![]),
            ("--[[ tool(args.name) ]] log(\"tool\")", vec![]),
            (
                "local t = { tool = 1 } say(tostring(t.tool)) x:tool(y)",
                vec![],
            ),
            ("local s = [[ tool(x) ]] say(s)", vec![]),
        ] {
            assert_eq!(called(source).unwrap(), want, "{source}");
        }
        for source in [
            "tool(args.name)",
            "local name = 'music' tool(name)",
            "tool('lol.' .. 'match')",
            "tool(('music'))",
            "tool([[music]])",
            "tool(`music`)",
            "local f = tool f('music')",
            "pcall(tool, 'music')",
            "local x = `{tool(args.n)}`",
            "tool",
            "tool('mu\\115ic')",
            "tool('a b')",
            "tool('')",
        ] {
            assert!(called(source).is_err(), "{source}");
        }
        assert!(called("tool(args.name)")
            .unwrap_err()
            .contains("直接写工具名"));
    }

    /// D1、D2：保存时名字要直接写；自动化自己的工具（运行、改、停自动化）不能调。
    #[test]
    fn saving_rejects_tool_names_not_written_out_and_automation_tools() {
        let with = |source: &str| {
            draft(json!({"name": "x", "action": {"kind": "program", "source": source}}))
                .normalized(&EventCatalog::generic())
        };
        assert!(with("tool('music', { action = 'play' })").is_ok());
        assert!(with("tool(args.tool_name)")
            .unwrap_err()
            .contains("直接写工具名"));
        for tool in [
            "automation.run",
            "automation.save",
            "automation.stop",
            "saved.auto-1",
        ] {
            let err = with(&format!("tool('{tool}', {{ id = 'auto-1' }})")).unwrap_err();
            assert!(err.contains("自动化自己的工具"), "{tool}: {err}");
        }
    }

    fn calls_music() -> AutomationDraft {
        draft(json!({
            "name": "队友都倒了放首歌",
            "params": [{"name": "song", "type": "text", "maxLength": 100, "default": "Legends Never Die"}],
            "action": {"kind": "program", "source": "local r = tool(\"music\", { action = \"play\", query = args.song })\nif not r then log(\"没放成\") end\ntool('settings.get')"},
            "on": {"event": ["game.phase_changed"], "game": "demo", "match": {"to": "in_game"}, "cooldownSeconds": 60}
        }))
    }

    fn effects(map: &[(&str, &[&str])]) -> BTreeMap<String, Vec<String>> {
        map.iter()
            .map(|(tool, effects)| {
                (
                    tool.to_string(),
                    effects.iter().map(|e| e.to_string()).collect(),
                )
            })
            .collect()
    }

    /// D3：确认写不进来（草稿带 `confirmed` 直接拒，只有 `confirm` 能写，还要核对玩家看到的那份程序）；程序一改
    /// 就清掉；只改名字、参数、触发不动它；第三方插件的配置变了按前缀清掉；存盘读回一样。
    #[test]
    fn confirmations_come_only_from_confirm_and_go_when_the_program_changes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("automations.json");
        let library = AutomationLibrary::open(path.clone()).unwrap();
        let catalog = EventCatalog::generic();
        // 草稿带 confirmed：说清楚再拒（她、外部 Agent 伪造不进来）
        let mut forged = serde_json::to_value(calls_music()).unwrap();
        forged["confirmed"] = json!({"music": ["plays-media"]});
        let err = serde_json::from_value::<AutomationDraft>(forged).unwrap_err();
        assert!(err.to_string().contains("「自动化」页"), "{err}");
        let saved = library
            .save(calls_music(), CreatedBy::Companion, &catalog)
            .unwrap();
        assert!(saved.confirmed.is_empty());
        let source = saved.action.source().to_string();
        // 玩家看到的不是现在这份 → 不认；程序里没有的工具 → 不认
        assert!(library
            .confirm(
                &saved.id,
                "tool('music')",
                effects(&[("music", &["plays-media"])])
            )
            .unwrap_err()
            .contains("刚被改过"));
        assert!(library
            .confirm(
                &saved.id,
                &source,
                effects(&[("game.act", &["sends-game-input"])])
            )
            .is_err());
        assert!(library
            .confirm("auto-99", &source, effects(&[("music", &["plays-media"])]))
            .is_err());
        let confirmed = library
            .confirm(
                &saved.id,
                &format!("  {source}\n"),
                effects(&[("music", &["plays-media", "plays-media"])]),
            )
            .unwrap();
        assert_eq!(confirmed.confirmed, effects(&[("music", &["plays-media"])]));
        assert_eq!(confirmed.updated_at_ms, saved.updated_at_ms, "确认不是修改");
        // 和以前的合在一起
        let both = library
            .confirm(&saved.id, &source, effects(&[("settings.get", &[])]))
            .unwrap()
            .confirmed;
        assert_eq!(
            both,
            effects(&[("music", &["plays-media"]), ("settings.get", &[])])
        );
        // 只改名字、参数缺省值、触发：确认还在
        let mut renamed = calls_music();
        renamed.id = Some(saved.id.clone());
        renamed.name = "最后一个队友倒下放歌".into();
        renamed.params[0].default = Some(json!("晴天"));
        renamed.on.as_mut().unwrap().cooldown_seconds = 120;
        let kept = library.save(renamed, CreatedBy::User, &catalog).unwrap();
        assert_eq!(kept.confirmed, both);
        // 存盘读回一样；文件里有 confirmed
        drop(library);
        let library = AutomationLibrary::open(path.clone()).unwrap();
        assert_eq!(library.get(&saved.id).unwrap().confirmed, both);
        let file: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(
            file["automations"][0]["confirmed"]["music"],
            json!(["plays-media"])
        );
        // 第三方插件的配置变了：按前缀清掉
        assert!(!library.forget_confirmations("demo.").unwrap());
        assert!(library.forget_confirmations("settings.").unwrap());
        assert_eq!(
            library.get(&saved.id).unwrap().confirmed,
            effects(&[("music", &["plays-media"])])
        );
        // 改了程序（谁改的都算，多一个空格不算）：确认没了
        let mut same = calls_music();
        same.id = Some(saved.id.clone());
        let AutomationBody::Program(program) = &mut same.action;
        program.source = format!("\n{}  ", program.source);
        let still = library.save(same, CreatedBy::Agent, &catalog).unwrap();
        assert!(!still.confirmed.is_empty(), "首尾空白不算改了程序");
        let mut edited = calls_music();
        edited.id = Some(saved.id.clone());
        let AutomationBody::Program(program) = &mut edited.action;
        program.source.push_str("\nsay('好')");
        let cleared = library.save(edited, CreatedBy::User, &catalog).unwrap();
        assert!(cleared.confirmed.is_empty());
        let file: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert!(
            file["automations"][0].get("confirmed").is_none(),
            "空的不写"
        );
    }

    /// 兼容：rc.13 的定义不认识 `confirmed`、也不拒（它的 `Automation` 没有 `deny_unknown_fields`），读新文件不出错；
    /// 文件版本号还是 1。这里照 rc.13 的样子写一份读法对着查。
    #[test]
    fn rc13_reads_a_file_with_confirmations() {
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        #[allow(dead_code)]
        struct Rc13Automation {
            id: String,
            name: String,
            #[serde(default)]
            description: String,
            #[serde(default)]
            params: Vec<ParamSpec>,
            action: AutomationBody,
            created_by: CreatedBy,
            created_at_ms: u64,
            updated_at_ms: u64,
            #[serde(default)]
            schedule: Option<Schedule>,
            #[serde(default)]
            on: Option<EventTrigger>,
        }
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        #[allow(dead_code)]
        struct Rc13File {
            version: u32,
            next_id: u64,
            automations: Vec<Rc13Automation>,
            #[serde(default)]
            builtin: Vec<String>,
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("automations.json");
        let library = AutomationLibrary::open(path.clone()).unwrap();
        let saved = library
            .save(calls_music(), CreatedBy::User, &EventCatalog::generic())
            .unwrap();
        library
            .confirm(
                &saved.id,
                saved.action.source(),
                effects(&[("music", &["plays-media"])]),
            )
            .unwrap();
        let file: Rc13File = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(file.version, 1);
        assert_eq!(file.automations[0].action, saved.action);
    }

    /// 一个“示例”游戏的事件目录：`demo.death` 带 team、champion、alliesAlive、meAlive（本体的代码里不写插件登记的事件名，
    /// kotone-plugin-api 的 tests/event_boundaries.rs）。
    fn death_catalog() -> EventCatalog {
        let mut catalog = EventCatalog::generic();
        catalog
            .add(crate::games::CatalogEntry {
                spec: crate::games::EventSpec {
                    kind: "demo.death".into(),
                    label: "有英雄阵亡".into(),
                    data: ["team", "champion", "alliesAlive", "meAlive"]
                        .iter()
                        .map(|f| (f.to_string(), format!("{f} 的说明")))
                        .collect(),
                    needs: Vec::new(),
                    private: Vec::new(),
                },
                adapter: "demo".into(),
                plugin: Some(("demo".into(), "示例".into())),
                games: vec!["demo".into()],
                off: None,
            })
            .unwrap();
        catalog
    }

    /// ADR 046 的 2026-11-27 修订：按事件触发、开了一局一次或冷却，程序里又读了事件填的参数——保存时提醒一句（不拦）：
    /// 条件写在程序里，不满足就 return 的那次也算触发。条件都在 `on.match` 里的、没开次数和冷却的、读的参数不是事件字段的、
    /// 参数只在注释和字符串里出现的、字段已经写进 `on.match` 的、触发关着的都不提醒。
    #[test]
    fn a_trigger_that_spends_its_once_per_match_on_a_program_check_gets_a_hint() {
        let catalog = death_catalog();
        let dir = tempfile::tempdir().unwrap();
        let library = AutomationLibrary::open(dir.path().join("automations.json")).unwrap();
        let save = |name: &str, on: Value, params: Value, source: &str| {
            let d = draft(json!({
                "name": name, "params": params, "on": on,
                "action": {"kind": "program", "source": source},
            }));
            library.save(d, CreatedBy::Companion, &catalog).unwrap()
        };
        let allies_alive = json!([{"name": "allies_alive", "type": "number", "default": 0}]);
        let checks =
            "if type(args.allies_alive) == \"number\" and args.allies_alive > 1 then return end\n\
                      tool(\"music\", { action = \"play\", query = \"雨爱\" })";
        // 玩家机器上那一份（摘要）：触发只写了 team，一局一次 + 120 秒冷却，程序里按 args.allies_alive 判断
        let wrong = save(
            "队友都倒了放首歌",
            json!({"event": ["demo.death"], "game": "demo", "match": {"team": "ally"},
                   "oncePerMatch": true, "cooldownSeconds": 120}),
            allies_alive.clone(),
            checks,
        );
        let hints = trigger_hints(&wrong, &catalog);
        assert_eq!(hints.len(), 1, "{hints:?}");
        for text in [
            "开了一局一次和冷却（120 秒）",
            "args.allies_alive（alliesAlive）",
            "那一次也算触发",
            "写进 on.match",
            "只是拿来说话的不用改",
        ] {
            assert!(hints[0].contains(text), "{text}：{}", hints[0]);
        }
        // 正确的写法：条件都在 on.match 里，程序只管放歌
        let right = save(
            "队友都倒了放首歌（改）",
            json!({"event": ["demo.death"], "game": "demo",
                   "match": {"team": "ally", "alliesAlive": 1, "meAlive": true}, "oncePerMatch": true}),
            json!([]),
            "tool(\"music\", { action = \"play\", query = \"雨爱\" })",
        );
        assert!(trigger_hints(&right, &catalog).is_empty());
        // 没开一局一次也没冷却：每件都会再触发，不提醒；只开冷却的提醒
        let on = |extra: Value| {
            let mut on = json!({"event": ["demo.death"], "match": {"team": "ally"}});
            on.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            on
        };
        let plain = save("不限次数", on(json!({})), allies_alive.clone(), checks);
        assert!(trigger_hints(&plain, &catalog).is_empty());
        let cooled = save(
            "只有冷却",
            on(json!({"cooldownSeconds": 60})),
            allies_alive.clone(),
            checks,
        );
        let hints = trigger_hints(&cooled, &catalog);
        assert!(
            hints.len() == 1
                && hints[0].contains("开了冷却（60 秒）")
                && !hints[0].contains("一局一次"),
            "{hints:?}"
        );
        // 读的参数不是事件的字段（玩家自己填的歌名）：不提醒
        let song = save(
            "放我选的歌",
            on(json!({"oncePerMatch": true})),
            json!([{"name": "song", "type": "text", "default": "雨爱"}]),
            "tool(\"music\", { action = \"play\", query = args.song })",
        );
        assert!(trigger_hints(&song, &catalog).is_empty());
        // 声明了事件的参数，但只在注释、字符串、别的表的字段里出现：不提醒
        let unread = save(
            "没读",
            on(json!({"oncePerMatch": true})),
            allies_alive.clone(),
            "-- args.allies_alive\nlocal t = { args = {} }\nsay(\"args.allies_alive\" .. tostring(t.args.allies_alive))",
        );
        assert!(trigger_hints(&unread, &catalog).is_empty());
        // 字段已经写进 on.match 的不算；另一个没写的照样提醒（参数名和字段同名时不写括号）；前缀 demo.* 也认
        let partly = save(
            "一半写进 match",
            json!({"event": ["demo.*"], "match": {"alliesAlive": 1}, "oncePerMatch": true}),
            json!([{"name": "allies_alive", "type": "number", "default": 0},
                   {"name": "team", "type": "text", "default": ""}]),
            "if args.team ~= \"ally\" or args.allies_alive ~= 1 then return end\nsay(\"只剩你了\")",
        );
        let hints = trigger_hints(&partly, &catalog);
        assert!(
            hints.len() == 1
                && hints[0].contains("args.team：")
                && !hints[0].contains("allies_alive"),
            "{hints:?}"
        );
        // 触发关着：不提醒
        let off = save(
            "关着",
            on(json!({"oncePerMatch": true, "enabled": false})),
            allies_alive,
            checks,
        );
        assert!(trigger_hints(&off, &catalog).is_empty());
    }
}
