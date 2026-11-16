//! 已保存的自动化（定义，ADR 013 §2）：名字、说明、入参声明、动作、创建者。存在
//! `~/.kotone/automations.json`。运行时先按声明校验入参（[`validate_args`]），再由调度器生成
//! 执行记录；执行记录保存源码快照，之后修改或删除定义不影响已安排的执行。

use std::{path::PathBuf, sync::Mutex};

use serde_json::{Map, Value};

use super::{now_ms, write_atomic, SpeakAction, MAX_NAME_CHARS, MAX_SOURCE_CHARS};
use crate::reminders::validate_content;

pub const MAX_AUTOMATIONS: usize = 200;
pub const MAX_PARAMS: usize = 16;
const MAX_DESCRIPTION_CHARS: usize = 200;
const MAX_LABEL_CHARS: usize = 40;
const MAX_TEXT_PARAM_CHARS: u32 = 200;
const MAX_CHOICES: usize = 16;
const STORE_VERSION: u32 = 1;

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
    fn normalized(mut self) -> Result<Self, String> {
        self.name = self.name.trim().to_string();
        self.description = self.description.trim().to_string();
        if self.name.is_empty() || self.name.chars().count() > MAX_NAME_CHARS {
            return Err(format!("自动化名字不能为空，最多 {MAX_NAME_CHARS} 字"));
        }
        if self.description.chars().count() > MAX_DESCRIPTION_CHARS {
            return Err(format!("说明最多 {MAX_DESCRIPTION_CHARS} 字"));
        }
        validate_params(&mut self.params)?;
        match &mut self.action {
            AutomationBody::Speak(speak) => {
                speak.text = speak.text.trim().to_string();
                validate_content(&speak.text, &speak.delivery)?;
                if !self.params.is_empty() {
                    return Err("播报类自动化不支持参数".into());
                }
            }
            AutomationBody::Program(program) => {
                program.source = program.source.trim().to_string();
                let chars = program.source.chars().count();
                if chars == 0 || chars > MAX_SOURCE_CHARS {
                    return Err(format!("自动化内容为空或超过 {MAX_SOURCE_CHARS} 字"));
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
}

impl Default for StoredAutomations {
    fn default() -> Self {
        Self {
            version: STORE_VERSION,
            next_id: 1,
            automations: vec![],
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
    pub fn save(&self, draft: AutomationDraft, by: CreatedBy) -> Result<Automation, String> {
        let draft = draft.normalized()?;
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
                };
                next.automations.push(automation.clone());
                automation
            }
        };
        self.persist(&next)?;
        *state = next;
        Ok(saved)
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
    fn args_are_checked_against_the_declaration_and_defaults_filled() {
        let params = press_every().normalized().unwrap().params;
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
            (speak_with_params, "不支持参数"),
            (empty, "为空"),
            (no_name, "名字"),
        ] {
            let err = draft.normalized().unwrap_err();
            assert!(err.contains(why), "{err}");
        }
        assert!(serde_json::from_value::<ParamSpec>(json!({"name": "x", "type": "file"})).is_err());
    }

    #[test]
    fn library_creates_updates_deletes_and_persists() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("automations.json");
        let library = AutomationLibrary::open(path.clone()).unwrap();
        let saved = library.save(press_every(), CreatedBy::Agent).unwrap();
        assert_eq!(saved.id, "auto-1");
        assert_eq!(saved.name, "连按某键");
        assert_eq!(saved.params[1].label, "interval");
        let mut edit = press_every();
        edit.id = Some(saved.id.clone());
        edit.name = "连按".into();
        let updated = library.save(edit, CreatedBy::User).unwrap();
        assert_eq!(updated.created_by, CreatedBy::Agent, "覆盖保留创建者");
        assert_eq!(updated.created_at_ms, saved.created_at_ms);
        let mut missing = press_every();
        missing.id = Some("auto-99".into());
        assert!(library.save(missing, CreatedBy::User).is_err());
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
            library.save(press_every(), CreatedBy::User).unwrap().id,
            "auto-2"
        );
    }
}
