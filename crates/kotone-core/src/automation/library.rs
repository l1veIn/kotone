//! 已保存的自动化（定义，ADR 013 §2）：名字、说明、入参声明、动作、创建者。存在
//! `~/.kotone/automations.json`。运行时先按声明校验入参（[`validate_args`]），再由调度器生成
//! 执行记录；执行记录保存源码快照，之后修改或删除定义不影响已安排的执行。

use std::{path::PathBuf, sync::Mutex};

use serde_json::{Map, Value};

use super::{now_ms, write_atomic, SpeakAction, MAX_NAME_CHARS, MAX_SOURCE_CHARS};
use crate::games::{validate_trigger, EventCatalog, EventTrigger};
use crate::reminders::{validate_content, DeliveryKind};

pub const MAX_AUTOMATIONS: usize = 200;
pub const MAX_PARAMS: usize = 16;
const MAX_DESCRIPTION_CHARS: usize = 200;
const MAX_LABEL_CHARS: usize = 40;
const MAX_TEXT_PARAM_CHARS: u32 = 200;
const MAX_CHOICES: usize = 16;
const STORE_VERSION: u32 = 1;
/// “让她做”那句话最多多少字。
pub const MAX_PROMPT_CHARS: usize = 500;

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

/// 定义里的动作（不含执行时才有的目标窗口与入参）。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum AutomationBody {
    Speak(SpeakAction),
    Program(ProgramBody),
    /// 让她做（ADR 036 D8）：一句交给她的话，触发时放进主会话让她跑一轮（每次花一次 AI 调用）。
    Prompt(PromptBody),
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptBody {
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgramBody {
    pub source: String,
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
/// 播报类自动化里表示“几秒后再说”的参数名。
pub const DELAY_PARAM: &str = "delay_seconds";

/// 已保存的自动化。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
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
}

/// 新建或覆盖时提交的内容（`id` 为空 = 新建）。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
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

/// 播报文字里的 `{参数名}`。
fn placeholders(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find('{') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('}') else { break };
        let name = &after[..end];
        if is_identifier(name) || name == DELAY_PARAM {
            out.push(name.to_string());
        }
        rest = &after[end + 1..];
    }
    out
}

/// 按入参生成这一次的播报：`{参数名}` 换成值；`delaySeconds` 给出几秒后再说（毫秒）。
pub fn render_speak(speak: &SpeakAction, args: &Map<String, Value>) -> (String, u64) {
    let mut text = speak.text.clone();
    for (name, value) in args {
        let shown = match value {
            Value::String(s) => s.clone(),
            Value::Bool(b) => {
                if *b {
                    "是".into()
                } else {
                    "否".into()
                }
            }
            Value::Number(n) => n.to_string(),
            other => other.to_string(),
        };
        text = text.replace(&format!("{{{name}}}"), &shown);
    }
    let delay = args
        .get(DELAY_PARAM)
        .and_then(Value::as_f64)
        .filter(|v| v.is_finite() && *v > 0.0)
        .map(|v| (v * 1000.0) as u64)
        .unwrap_or(0);
    (text, delay)
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
        match &mut self.action {
            AutomationBody::Speak(speak) => {
                speak.text = speak.text.trim().to_string();
                validate_content(&speak.text, &speak.delivery)?;
                for spec in &self.params {
                    let simple = matches!(
                        spec.kind,
                        ParamKind::Number { .. }
                            | ParamKind::Text { .. }
                            | ParamKind::Boolean
                            | ParamKind::Choice { .. }
                    );
                    if !simple {
                        return Err(format!(
                            "播报类自动化的参数 {} 只能是数字、文字、开关或选项",
                            spec.name
                        ));
                    }
                    if spec.name == DELAY_PARAM && !matches!(spec.kind, ParamKind::Number { .. }) {
                        return Err(format!("{DELAY_PARAM}（几秒后再说）要是数字"));
                    }
                }
                for name in placeholders(&speak.text) {
                    if !self.params.iter().any(|p| p.name == name) {
                        return Err(format!("播报里的 {{{name}}} 没有对应的参数"));
                    }
                }
            }
            AutomationBody::Program(program) => {
                program.source = program.source.trim().to_string();
                let chars = program.source.chars().count();
                if chars == 0 || chars > MAX_SOURCE_CHARS {
                    return Err(format!("自动化内容为空或超过 {MAX_SOURCE_CHARS} 字"));
                }
            }
            AutomationBody::Prompt(prompt) => {
                prompt.text = prompt.text.trim().to_string();
                let chars = prompt.text.chars().count();
                if chars == 0 || chars > MAX_PROMPT_CHARS {
                    return Err(format!(
                        "“让她做”的那句话不能为空，最多 {MAX_PROMPT_CHARS} 字"
                    ));
                }
                if !self.params.is_empty() {
                    return Err("“让她做”不用参数：触发它的游戏事件会一起交给她".into());
                }
            }
        }
        Ok(self)
    }
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredAutomations {
    version: u32,
    next_id: u64,
    automations: Vec<Automation>,
    /// 已经装过的内置自动化（删了也记着，不再装回）。
    #[serde(default)]
    builtin: Vec<String>,
}

/// “X 秒之后提醒我 xxx”：到点用语音和弹幕说一句。
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
        action: AutomationBody::Speak(SpeakAction {
            text: "{text}".into(),
            delivery: vec![DeliveryKind::Voice, DeliveryKind::Danmaku],
        }),
        created_by: CreatedBy::User,
        created_at_ms: now,
        updated_at_ms: now,
        schedule: None,
        on: None,
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
    }
}

/// 内置模板“英雄联盟进加载界面播报突破口”（ADR 036 D8）：每次触发花一次 AI 调用，所以默认关。
pub const BUILTIN_LOL_SCOUT_ID: &str = "builtin-lol-scout-on-loading";

fn builtin_lol_scout(now: u64) -> Automation {
    let mut matches = Map::new();
    matches.insert("to".into(), Value::from("loading"));
    Automation {
        id: BUILTIN_LOL_SCOUT_ID.into(),
        name: "英雄联盟进加载界面播报突破口".into(),
        description: "进加载界面时她查一下这局 10 个人的战绩，用一句话告诉你对面谁是突破口。每次花一次 AI 调用，默认关。".into(),
        params: vec![],
        action: AutomationBody::Prompt(PromptBody {
            text: "这局进加载界面了。用 lol.scout 看看对面谁是突破口，用一句话告诉我原因（说成信息，不贴标签）。".into(),
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
        let AutomationBody::Program(program) = &stored.action else {
            continue;
        };
        // 装的时候存的是 `source.trim() + "\n"`（[`program`]）
        if program.source.trim() != old.trim() {
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
    pub fn open(path: PathBuf) -> Result<Self, String> {
        let stored = match std::fs::read(&path) {
            Ok(bytes) => {
                if bytes.len() > 8 * 1024 * 1024 {
                    return Err("自动化文件超过大小上限".into());
                }
                let stored: StoredAutomations = serde_json::from_slice(&bytes)
                    .map_err(|e| format!("自动化文件无法读取，已保留原文件：{e}"))?;
                if stored.version != STORE_VERSION {
                    return Err("自动化文件版本暂不支持，已保留原文件".into());
                }
                stored
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => StoredAutomations::default(),
            Err(e) => return Err(format!("读取自动化文件失败：{e}")),
        };
        Ok(Self {
            path,
            state: Mutex::new(stored),
        })
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

    /// 新建（`draft.id` 为空）或覆盖已有定义。覆盖时保留创建者与创建时间。
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
        let mut all = vec![builtin_remind(now), builtin_lol_scout(now)];
        all.extend(builtin_programs(now));
        let missing: Vec<Automation> = all
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

    /// 内置程序的定义（测试与文档用）。
    pub fn builtin_programs_preview() -> Vec<Automation> {
        builtin_programs(0)
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

    fn press_every() -> AutomationDraft {
        serde_json::from_value(json!({
            "name": "  连按某键 ",
            "description": "每隔一段时间按一次",
            "params": [
                {"name": "key", "label": "按键", "type": "key", "default": "Q"},
                {"name": "interval", "type": "number", "min": 50, "max": 10000, "integer": true, "default": 1000},
                {"name": "area", "type": "region"},
            ],
            "action": {"kind": "program", "source": "every(args.interval, function() key(args.key) end)"},
        }))
        .unwrap()
    }

    #[test]
    fn speak_automations_take_simple_params_placeholders_and_a_delay() {
        let draft: AutomationDraft = serde_json::from_value(json!({
            "name": "提醒", "params": [
                {"name": "delay_seconds", "type": "number", "min": 1, "integer": true, "default": 5},
                {"name": "text", "type": "text"}],
            "action": {"kind": "speak", "text": "该{text}了", "delivery": ["voice"]},
        }))
        .unwrap();
        let ok = draft.clone().normalized(&EventCatalog::generic()).unwrap();
        let AutomationBody::Speak(speak) = &ok.action else {
            panic!()
        };
        let filled = validate_args(&ok.params, &args(json!({"text": "打小龙"}))).unwrap();
        assert_eq!(
            render_speak(speak, &filled),
            ("该打小龙了".to_string(), 5_000)
        );

        let mut unknown = draft.clone();
        unknown.action = serde_json::from_value(
            json!({"kind": "speak", "text": "{who}", "delivery": ["voice"]}),
        )
        .unwrap();
        assert!(unknown
            .normalized(&EventCatalog::generic())
            .unwrap_err()
            .contains("{who}"));
        let mut region = draft;
        region
            .params
            .push(spec(json!({"name": "area", "type": "region"})));
        assert!(region.normalized(&EventCatalog::generic()).is_err());
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

    #[test]
    fn the_builtin_reminder_is_installed_once_and_stays_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let library = AutomationLibrary::open(dir.path().join("automations.json")).unwrap();
        assert!(library.ensure_builtin().unwrap());
        let builtin = library.get(BUILTIN_REMIND_ID).unwrap();
        assert_eq!(builtin.name, "X 秒之后提醒我");
        // 装上的定义自己也要能通过校验
        AutomationDraft {
            id: None,
            name: builtin.name.clone(),
            description: builtin.description.clone(),
            params: builtin.params.clone(),
            action: builtin.action.clone(),
            schedule: None,
            on: None,
        }
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
        let AutomationBody::Program(p) = &a.action else {
            panic!()
        };
        assert!(p.source.contains("chat(args.text)") && !p.source.contains("key("));
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
        let AutomationBody::Program(p) = &library.get(CHAT_REPEAT_ID).unwrap().action else {
            panic!()
        };
        assert!(p.source.contains("wait(300)"), "玩家改过的程序不动");
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
        for id in BUILTIN_PROGRAM_IDS {
            let a = library.get(id).unwrap_or_else(|| panic!("{id}"));
            let draft = AutomationDraft {
                id: None,
                name: a.name.clone(),
                description: a.description.clone(),
                params: a.params.clone(),
                action: a.action.clone(),
                schedule: None,
                on: None,
            }
            .normalized(&EventCatalog::generic())
            .unwrap_or_else(|e| panic!("{id}: {e}"));
            // 缺省值本身能通过入参校验（什么都不填也能跑）
            validate_args(&draft.params, &Map::new()).unwrap_or_else(|e| panic!("{id}: {e}"));
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
        let mut speak_with_params: AutomationDraft = serde_json::from_value(json!({
            "name": "喝水", "action": {"kind": "speak", "text": "喝水", "delivery": ["voice"]},
        }))
        .unwrap();
        speak_with_params.params = press_every().params;
        let mut empty = press_every();
        empty.action = AutomationBody::Program(ProgramBody {
            source: "  ".into(),
        });
        let mut no_name = press_every();
        no_name.name = " ".into();
        for (draft, why) in [
            (bad_name, "参数名"),
            (dup, "重复"),
            (bad_default, "缺省值"),
            (speak_with_params, "只能是数字、文字、开关或选项"),
            (empty, "为空"),
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
    fn prompt_automations_with_a_game_event_trigger() {
        let draft = |value: Value| serde_json::from_value::<AutomationDraft>(value).unwrap();
        let ok = draft(json!({
            "name": "进加载界面说突破口",
            "action": {"kind": "prompt", "text": "  用 lol.scout 看看对面谁是突破口  "},
            "on": {"event": ["game.phase_changed"], "game": "lol", "match": {"to": "loading"}, "oncePerMatch": true}
        }))
        .normalized(&EventCatalog::generic())
        .unwrap();
        let AutomationBody::Prompt(prompt) = &ok.action else {
            panic!("{:?}", ok.action)
        };
        assert_eq!(prompt.text, "用 lol.scout 看看对面谁是突破口");
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
                json!({"name": "x", "action": {"kind": "prompt", "text": "说一句"},
                    "params": [{"name": "n", "type": "number"}]}),
                "不用参数",
            ),
            (
                json!({"name": "x", "action": {"kind": "speak", "text": "龙", "delivery": ["voice"]},
                    "on": {"event": ["lol.dragon"]}}),
                "没有",
            ),
        ] {
            let error = draft(bad.clone())
                .normalized(&EventCatalog::generic())
                .unwrap_err();
            assert!(error.contains(want), "{bad} → {error}");
        }
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
        assert!(matches!(a.action, AutomationBody::Prompt(_)));
        let draft = AutomationDraft {
            id: None,
            name: a.name.clone(),
            description: a.description.clone(),
            params: a.params.clone(),
            action: a.action.clone(),
            schedule: None,
            on: a.on.clone(),
        }
        .normalized(&EventCatalog::generic())
        .unwrap();
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
}
