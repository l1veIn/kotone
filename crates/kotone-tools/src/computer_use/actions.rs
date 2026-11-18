//! `game.act` 的动作清单：名字与参数照抄 Anthropic computer use
//! （`computer_toolset_20260801`、anthropic-quickstarts `computer-use-demo/tools/computer.py`，MIT），
//! 另加两个游戏用的补充（GameVerse 的 `key_down` / `key_up`，Cradle 的相对移动 `mouse_move_by`）。
//!
//! 坐标 `coordinate` 是最近一次 `game.look` 截图里的像素 `[x, y]`；也可以用 `element` 填
//! 截图上的编号（文字识别出的元素）。

use serde_json::{Map, Value};

/// 一次 `game.act` 最多几步（dsh-computer-use-win `batch`：1–20）。
pub const MAX_ACTIONS: usize = 20;
/// `hold_key` / `wait` 的时长上限（秒）。
pub const MAX_DURATION_S: f64 = 10.0;
/// `type` 一次最多几个字。
pub const MAX_TYPE_CHARS: usize = 1_000;
/// `key` 的 `repeat` 上限。
pub const MAX_REPEAT: u32 = 100;
/// `scroll_amount` 上限（滚轮格数）。
pub const MAX_SCROLL: u32 = 50;
/// 一份计划里 `hold_key` 与 `wait` 合计最长多久（毫秒）：整次 `game.act` 要在工具期限
/// （30 秒）内做完并再看一眼。
pub const MAX_PLAN_WAIT_MS: u64 = 20_000;
/// `mouse_move_by` 单方向上限（像素）。
pub const MAX_MOVE_BY: i32 = 5_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Left,
    Right,
    Middle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollDirection {
    Up,
    Down,
    Left,
    Right,
}

/// 点在哪里：截图像素，或截图上的元素编号。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Target {
    Coordinate(f64, f64),
    Element(u32),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// `left_click` / `right_click` / `middle_click` / `double_click` / `triple_click`。
    /// `target` 为空时在鼠标当前位置点；`modifiers` 是点击时按住的键（如 `shift`）。
    Click {
        button: Button,
        count: u8,
        target: Option<Target>,
        modifiers: Option<String>,
    },
    /// `left_click_drag`。
    Drag {
        start: Target,
        end: Target,
    },
    MouseMove {
        target: Target,
    },
    LeftMouseDown,
    LeftMouseUp,
    Scroll {
        direction: ScrollDirection,
        amount: u32,
        target: Option<Target>,
        modifiers: Option<String>,
    },
    Type {
        text: String,
    },
    Key {
        keys: String,
        repeat: u32,
    },
    HoldKey {
        keys: String,
        duration_ms: u64,
    },
    Wait {
        duration_ms: u64,
    },
    KeyDown {
        keys: String,
    },
    KeyUp {
        keys: String,
    },
    MouseMoveBy {
        dx: i32,
        dy: i32,
    },
}

impl Action {
    /// 动作名（与输入时的名字一致）。
    pub fn name(&self) -> &'static str {
        match self {
            Self::Click {
                button: Button::Left,
                count: 1,
                ..
            } => "left_click",
            Self::Click {
                button: Button::Left,
                count: 2,
                ..
            } => "double_click",
            Self::Click {
                button: Button::Left,
                ..
            } => "triple_click",
            Self::Click {
                button: Button::Right,
                ..
            } => "right_click",
            Self::Click {
                button: Button::Middle,
                ..
            } => "middle_click",
            Self::Drag { .. } => "left_click_drag",
            Self::MouseMove { .. } => "mouse_move",
            Self::LeftMouseDown => "left_mouse_down",
            Self::LeftMouseUp => "left_mouse_up",
            Self::Scroll { .. } => "scroll",
            Self::Type { .. } => "type",
            Self::Key { .. } => "key",
            Self::HoldKey { .. } => "hold_key",
            Self::Wait { .. } => "wait",
            Self::KeyDown { .. } => "key_down",
            Self::KeyUp { .. } => "key_up",
            Self::MouseMoveBy { .. } => "mouse_move_by",
        }
    }

    /// 会按下、点击或输入（`wait`、`mouse_move` 这类不算）。
    pub fn sends_input(&self) -> bool {
        !matches!(self, Self::Wait { .. })
    }

    /// 给气泡看的简短中文说明（“正在操作：……”）。
    pub fn describe(&self) -> String {
        match self {
            Self::Click { button, count, .. } => match (button, count) {
                (Button::Left, 1) => "点击".into(),
                (Button::Left, 2) => "双击".into(),
                (Button::Left, _) => "三击".into(),
                (Button::Right, _) => "右键点击".into(),
                (Button::Middle, _) => "中键点击".into(),
            },
            Self::Drag { .. } => "拖动".into(),
            Self::MouseMove { .. } => "移动鼠标".into(),
            Self::LeftMouseDown => "按住左键".into(),
            Self::LeftMouseUp => "松开左键".into(),
            Self::Scroll { .. } => "滚动".into(),
            Self::Type { .. } => "打字".into(),
            Self::Key { keys, .. } => format!("按 {keys}"),
            Self::HoldKey { keys, .. } => format!("按住 {keys}"),
            Self::Wait { .. } => "等待".into(),
            Self::KeyDown { keys } => format!("按下 {keys}"),
            Self::KeyUp { keys } => format!("松开 {keys}"),
            Self::MouseMoveBy { .. } => "转动视角".into(),
        }
    }
}

/// 把模型常写的 xdotool 键名（Anthropic 的示例用它们：`Return`、`ctrl+s`、`Page_Down`……）
/// 换成 Kotone 的键名（`Enter`、`Ctrl+S`、`PageDown`）。不认识的原样保留，交给宿主校验。
pub fn normalize_keys(spec: &str) -> String {
    spec.split('+')
        .map(|part| {
            let p = part.trim();
            let lower = p.to_ascii_lowercase();
            let mapped = match lower.as_str() {
                "return" | "enter" | "kp_enter" => "Enter",
                "escape" | "esc" => "Esc",
                "backspace" | "back_space" => "Backspace",
                "ctrl" | "control" | "control_l" | "control_r" | "ctrl_l" | "ctrl_r" => "Ctrl",
                "shift" | "shift_l" | "shift_r" => "Shift",
                "alt" | "alt_l" | "alt_r" | "option" => "Alt",
                "super" | "super_l" | "super_r" | "win" | "windows" | "meta" | "cmd" => "Win",
                "page_up" | "pageup" | "prior" | "pgup" => "PageUp",
                "page_down" | "pagedown" | "next" | "pgdn" => "PageDown",
                "caps_lock" | "capslock" => "CapsLock",
                "space" => "Space",
                "tab" => "Tab",
                "insert" | "ins" => "Insert",
                "delete" | "del" => "Delete",
                _ => return p.to_string(),
            };
            mapped.to_string()
        })
        .collect::<Vec<_>>()
        .join("+")
}

fn str_field<'a>(o: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    o.get(key).and_then(Value::as_str)
}

fn need_str(o: &Map<String, Value>, key: &str, action: &str) -> Result<String, String> {
    match str_field(o, key) {
        Some(s) if !s.trim().is_empty() => Ok(s.to_string()),
        _ => Err(format!("{action} 需要 {key}")),
    }
}

fn number(v: &Value) -> Option<f64> {
    v.as_f64().filter(|n| n.is_finite())
}

fn coordinate(o: &Map<String, Value>, key: &str) -> Result<Option<(f64, f64)>, String> {
    match o.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(a)) if a.len() == 2 => match (number(&a[0]), number(&a[1])) {
            (Some(x), Some(y)) if x >= 0.0 && y >= 0.0 => Ok(Some((x, y))),
            _ => Err(format!("{key} 必须是两个非负数 [x, y]")),
        },
        Some(_) => Err(format!("{key} 必须是 [x, y]")),
    }
}

/// `coordinate` 或 `element`（两个都给时报错）。
fn target(o: &Map<String, Value>) -> Result<Option<Target>, String> {
    let coord = coordinate(o, "coordinate")?;
    let element = match o.get("element") {
        None | Some(Value::Null) => None,
        Some(v) => Some(
            v.as_u64()
                .filter(|n| (1..=u64::from(u32::MAX)).contains(n))
                .ok_or("element 必须是截图上的编号（从 1 开始）")? as u32,
        ),
    };
    match (coord, element) {
        (Some(_), Some(_)) => Err("coordinate 和 element 只能给一个".into()),
        (Some((x, y)), None) => Ok(Some(Target::Coordinate(x, y))),
        (None, Some(id)) => Ok(Some(Target::Element(id))),
        (None, None) => Ok(None),
    }
}

fn duration_ms(o: &Map<String, Value>, action: &str) -> Result<u64, String> {
    let d = o
        .get("duration")
        .and_then(number)
        .ok_or_else(|| format!("{action} 需要 duration（秒）"))?;
    if !(0.0..=MAX_DURATION_S).contains(&d) {
        return Err(format!("{action} 的 duration 要在 0–{MAX_DURATION_S} 秒"));
    }
    Ok((d * 1000.0).round() as u64)
}

fn modifiers(o: &Map<String, Value>) -> Option<String> {
    str_field(o, "text")
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(normalize_keys)
}

/// 解析一步。只检查形状与范围；坐标是否在截图内、编号是否存在、键名是否认识，
/// 由批量执行在动手前对整份计划统一检查。
pub fn parse_action(value: &Value) -> Result<Action, String> {
    let o = value.as_object().ok_or("每一步必须是对象 {action, ...}")?;
    let name = str_field(o, "action").ok_or("缺少 action")?;
    let click = |button: Button, count: u8| -> Result<Action, String> {
        Ok(Action::Click {
            button,
            count,
            target: target(o)?,
            modifiers: modifiers(o),
        })
    };
    let action = match name {
        "left_click" => click(Button::Left, 1)?,
        "double_click" => click(Button::Left, 2)?,
        "triple_click" => click(Button::Left, 3)?,
        "right_click" => click(Button::Right, 1)?,
        "middle_click" => click(Button::Middle, 1)?,
        "left_click_drag" => {
            let (sx, sy) = coordinate(o, "start_coordinate")?
                .ok_or("left_click_drag 需要 start_coordinate")?;
            let end = target(o)?.ok_or("left_click_drag 需要 coordinate（终点）")?;
            Action::Drag {
                start: Target::Coordinate(sx, sy),
                end,
            }
        }
        "mouse_move" => Action::MouseMove {
            target: target(o)?.ok_or("mouse_move 需要 coordinate 或 element")?,
        },
        "left_mouse_down" => Action::LeftMouseDown,
        "left_mouse_up" => Action::LeftMouseUp,
        "scroll" => {
            let direction = match str_field(o, "scroll_direction") {
                Some("up") => ScrollDirection::Up,
                Some("down") => ScrollDirection::Down,
                Some("left") => ScrollDirection::Left,
                Some("right") => ScrollDirection::Right,
                _ => return Err("scroll 需要 scroll_direction（up/down/left/right）".into()),
            };
            let amount = o
                .get("scroll_amount")
                .map(|v| v.as_u64().ok_or("scroll_amount 必须是正整数"))
                .transpose()?
                .unwrap_or(3);
            if !(1..=u64::from(MAX_SCROLL)).contains(&amount) {
                return Err(format!("scroll_amount 要在 1–{MAX_SCROLL}"));
            }
            Action::Scroll {
                direction,
                amount: amount as u32,
                target: target(o)?,
                modifiers: modifiers(o),
            }
        }
        "type" => {
            let text = str_field(o, "text").unwrap_or_default().to_string();
            if text.is_empty() || text.chars().count() > MAX_TYPE_CHARS {
                return Err(format!("type 的 text 不能为空、不超过 {MAX_TYPE_CHARS} 字"));
            }
            Action::Type { text }
        }
        "key" => {
            let repeat = o
                .get("repeat")
                .map(|v| v.as_u64().ok_or("repeat 必须是正整数"))
                .transpose()?
                .unwrap_or(1);
            if !(1..=u64::from(MAX_REPEAT)).contains(&repeat) {
                return Err(format!("repeat 要在 1–{MAX_REPEAT}"));
            }
            Action::Key {
                keys: normalize_keys(&need_str(o, "text", "key")?),
                repeat: repeat as u32,
            }
        }
        "hold_key" => Action::HoldKey {
            keys: normalize_keys(&need_str(o, "text", "hold_key")?),
            duration_ms: duration_ms(o, "hold_key")?,
        },
        "wait" => Action::Wait {
            duration_ms: duration_ms(o, "wait")?,
        },
        "key_down" => Action::KeyDown {
            keys: normalize_keys(&need_str(o, "text", "key_down")?),
        },
        "key_up" => Action::KeyUp {
            keys: normalize_keys(&need_str(o, "text", "key_up")?),
        },
        "mouse_move_by" => {
            let get = |k: &str| -> Result<i32, String> {
                let v = o
                    .get(k)
                    .and_then(Value::as_i64)
                    .ok_or_else(|| format!("mouse_move_by 需要整数 {k}"))?;
                if v.unsigned_abs() > MAX_MOVE_BY as u64 {
                    return Err(format!("{k} 不能超过 ±{MAX_MOVE_BY}"));
                }
                Ok(v as i32)
            };
            Action::MouseMoveBy {
                dx: get("dx")?,
                dy: get("dy")?,
            }
        }
        "screenshot" | "zoom" => {
            return Err(format!(
                "{name} 不是动作：看画面用 game.look（zoom 是它的参数）"
            ))
        }
        other => return Err(format!("不认识的动作 {other}")),
    };
    Ok(action)
}

/// 解析整份计划（1–[`MAX_ACTIONS`] 步）；任何一步不合法都整份拒绝，错误里带步号（从 1 开始）。
pub fn parse_plan(actions: &Value) -> Result<Vec<Action>, String> {
    let list = actions.as_array().ok_or("actions 必须是数组")?;
    if list.is_empty() || list.len() > MAX_ACTIONS {
        return Err(format!("actions 要有 1–{MAX_ACTIONS} 步"));
    }
    let plan = list
        .iter()
        .enumerate()
        .map(|(i, v)| parse_action(v).map_err(|e| format!("第 {} 步：{e}", i + 1)))
        .collect::<Result<Vec<_>, _>>()?;
    let waited: u64 = plan
        .iter()
        .map(|a| match a {
            Action::HoldKey { duration_ms, .. } | Action::Wait { duration_ms } => *duration_ms,
            _ => 0,
        })
        .sum();
    if waited > MAX_PLAN_WAIT_MS {
        return Err(format!(
            "一次 game.act 里 hold_key 和 wait 合计不能超过 {} 秒，分几次做",
            MAX_PLAN_WAIT_MS / 1000
        ));
    }
    Ok(plan)
}
