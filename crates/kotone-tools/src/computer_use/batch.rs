//! `game.act` 的批量执行，规则照抄 dsh-computer-use-win `batch`（MIT）：
//!
//! - 先校验整份计划（坐标在截图内、编号存在、键名认识），有一处不对就一步都不做；
//! - 按顺序执行，第一处失败就停，后面每一步记 `Not executed: an earlier action failed.`；
//! - 每步之前检查：取消、玩家接手、游戏仍在前台（宿主负责等待）、窗口尺寸没变；
//! - 停下时已完成的步骤保留，并给出 `stopReason`；
//! - 结果只说动作已发出（`sent`），有没有生效由模型看新的截图判断。
//!
//! 时序细节：先移过去、等 100 ms 再点（UI-TARS）；打字 50 字一组、组间 12 ms（Anthropic
//! `computer.py`）；本次按下没松开的键与鼠标键在结束时全部松开（UI-TARS 的提示词要求、
//! GameVerse 的 `key_down` / `key_up`）。

use super::actions::{Action, Button, ScrollDirection, Target};
use super::observe::Observation;

/// 移过去之后等多久再点（UI-TARS）。
pub const SETTLE_MS: u64 = 100;
/// 打字分组（Anthropic `computer.py`：`TYPING_GROUP_SIZE = 50`、`TYPING_DELAY_MS = 12`）。
pub const TYPE_CHUNK_CHARS: usize = 50;
pub const TYPE_CHUNK_GAP_MS: u64 = 12;
/// `key` 重复按时两次之间的间隔。
pub const REPEAT_GAP_MS: u64 = 50;
/// 拖动时中途移动几次（游戏常常只认“拖着移动”的过程）。
pub const DRAG_STEPS: u32 = 10;
pub const DRAG_STEP_MS: u64 = 10;

/// 为什么停下。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    /// 停止热键、语音“停下”、本轮被取消。
    Cancelled,
    /// 玩家动了键盘或鼠标（ADR 019 决定 7：接手 = 暂停）。
    PlayerInput,
    /// 窗口尺寸变了，旧坐标作废（DSH `WINDOW_CHANGED`）。
    WindowChanged,
    WindowClosed,
    /// 游戏一直不在前台。
    NotForeground,
    /// 点击点被别的程序的窗口盖住（DSH `POINT_COVERED`）。
    PointCovered,
    /// 某一步执行失败。
    ActionFailed,
}

impl StopReason {
    pub fn code(self) -> &'static str {
        match self {
            Self::Cancelled => "cancelled",
            Self::PlayerInput => "player_input",
            Self::WindowChanged => "window_changed",
            Self::WindowClosed => "window_closed",
            Self::NotForeground => "not_foreground",
            Self::PointCovered => "point_covered",
            Self::ActionFailed => "action_failed",
        }
    }

    /// 给模型的说明。
    pub fn hint(self) -> &'static str {
        match self {
            Self::Cancelled => "玩家让你停下了：不要再自动重试，等玩家的下一句话。",
            Self::PlayerInput => {
                "玩家接手了（动了键盘或鼠标）：停下，问玩家要不要你继续，不要自己重试。"
            }
            Self::WindowChanged => "游戏窗口尺寸变了，旧坐标和编号已作废：先 game.look 再操作。",
            Self::WindowClosed => "游戏窗口已经关闭。",
            Self::NotForeground => "游戏一直不在前台：请玩家切回游戏，或先用 game.focus。",
            Self::PointCovered => {
                "要点的位置被别的程序窗口盖住了，没有点：请玩家把它移开，或换个位置。"
            }
            Self::ActionFailed => "有一步执行失败，后面的没有执行：看新的截图再决定。",
        }
    }
}

/// 宿主报告的中断。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Halt {
    Stop(StopReason),
    Fail(String),
}

impl From<String> for Halt {
    fn from(message: String) -> Self {
        Self::Fail(message)
    }
}

/// 真正按键、动鼠标的一方（桌面实现；测试用假宿主）。
pub trait ActHost {
    /// 键名（含组合键，如 `Ctrl+S`）认不认识。
    fn key_known(&self, keys: &str) -> bool;
    /// 每步之前：取消、玩家接手、游戏在前台（不在就等，有上限）、窗口尺寸没变。
    fn checkpoint(&self) -> Result<(), Halt>;
    /// 鼠标移到窗口比例 (x, y)；点被别的窗口盖住时 `Halt::Stop(PointCovered)`。
    fn move_to(&self, x: f64, y: f64) -> Result<(), Halt>;
    fn button(&self, button: Button, down: bool) -> Result<(), Halt>;
    fn click(&self, button: Button, count: u8) -> Result<(), Halt>;
    fn wheel(&self, direction: ScrollDirection, amount: u32) -> Result<(), Halt>;
    fn type_text(&self, text: &str) -> Result<(), Halt>;
    fn key_press(&self, keys: &str) -> Result<(), Halt>;
    fn key_down(&self, keys: &str) -> Result<(), Halt>;
    fn key_up(&self, keys: &str) -> Result<(), Halt>;
    fn move_by(&self, dx: i32, dy: i32) -> Result<(), Halt>;
    /// 等待；被取消或玩家接手时提前返回 `Halt::Stop`。
    fn sleep(&self, ms: u64) -> Result<(), Halt>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepStatus {
    Sent,
    Failed,
    /// 做到一半被停下（例如按住键时玩家接手）。
    Stopped,
    NotExecuted,
}

impl StepStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sent => "sent",
            Self::Failed => "failed",
            Self::Stopped => "stopped",
            Self::NotExecuted => "not_executed",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct StepResult {
    pub action: &'static str,
    pub status: StepStatus,
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ActReport {
    pub steps: Vec<StepResult>,
    pub stop: Option<StopReason>,
}

impl ActReport {
    pub fn sent(&self) -> usize {
        self.steps
            .iter()
            .filter(|s| s.status == StepStatus::Sent)
            .count()
    }
}

fn modifier_keys(action: &Action) -> Option<&str> {
    match action {
        Action::Click { modifiers, .. } | Action::Scroll { modifiers, .. } => modifiers.as_deref(),
        _ => None,
    }
}

/// 动手前校验整份计划；错误里带步号（从 1 开始）。
pub fn validate(plan: &[Action], obs: &Observation, host: &dyn ActHost) -> Result<(), String> {
    for (i, action) in plan.iter().enumerate() {
        let step = |e: String| format!("第 {} 步（{}）：{e}", i + 1, action.name());
        let targets: Vec<Target> = match action {
            Action::Click { target, .. } | Action::Scroll { target, .. } => {
                target.iter().copied().collect()
            }
            Action::Drag { start, end } => vec![*start, *end],
            Action::MouseMove { target } => vec![*target],
            _ => Vec::new(),
        };
        for target in targets {
            obs.resolve(target).map_err(|e| step(e.to_string()))?;
        }
        let keys = match action {
            Action::Key { keys, .. }
            | Action::HoldKey { keys, .. }
            | Action::KeyDown { keys }
            | Action::KeyUp { keys } => Some(keys.as_str()),
            _ => modifier_keys(action),
        };
        if let Some(keys) = keys {
            if !host.key_known(keys) {
                return Err(step(format!("不认识的键名「{keys}」")));
            }
        }
    }
    Ok(())
}

/// 本次按下、还没松开的键与鼠标键（结束时全部松开）。
#[derive(Default)]
struct Held {
    keys: Vec<String>,
    left_button: bool,
}

fn run_step(
    action: &Action,
    obs: &Observation,
    host: &dyn ActHost,
    held: &mut Held,
) -> Result<(), Halt> {
    let point = |target: Target| -> Result<(f64, f64), Halt> {
        obs.resolve(target).map_err(|e| Halt::Fail(e.to_string()))
    };
    let with_modifiers = |held: &mut Held, mods: Option<&str>, f: &dyn Fn() -> Result<(), Halt>| {
        if let Some(mods) = mods {
            host.key_down(mods)?;
            held.keys.push(mods.to_string());
        }
        let result = f();
        if let Some(mods) = mods {
            held.keys.retain(|k| k != mods);
            let up = host.key_up(mods);
            result?;
            return up;
        }
        result
    };
    match action {
        Action::Click {
            button,
            count,
            target,
            modifiers,
        } => {
            if let Some(target) = target {
                let (x, y) = point(*target)?;
                host.move_to(x, y)?;
                host.sleep(SETTLE_MS)?;
            }
            with_modifiers(held, modifiers.as_deref(), &|| host.click(*button, *count))
        }
        Action::Drag { start, end } => {
            let (sx, sy) = point(*start)?;
            let (ex, ey) = point(*end)?;
            host.move_to(sx, sy)?;
            host.sleep(SETTLE_MS)?;
            host.button(Button::Left, true)?;
            held.left_button = true;
            for i in 1..=DRAG_STEPS {
                let t = f64::from(i) / f64::from(DRAG_STEPS);
                host.move_to(sx + (ex - sx) * t, sy + (ey - sy) * t)?;
                host.sleep(DRAG_STEP_MS)?;
            }
            held.left_button = false;
            host.button(Button::Left, false)
        }
        Action::MouseMove { target } => {
            let (x, y) = point(*target)?;
            host.move_to(x, y)
        }
        Action::LeftMouseDown => {
            host.button(Button::Left, true)?;
            held.left_button = true;
            Ok(())
        }
        Action::LeftMouseUp => {
            held.left_button = false;
            host.button(Button::Left, false)
        }
        Action::Scroll {
            direction,
            amount,
            target,
            modifiers,
        } => {
            if let Some(target) = target {
                let (x, y) = point(*target)?;
                host.move_to(x, y)?;
                host.sleep(SETTLE_MS)?;
            }
            with_modifiers(held, modifiers.as_deref(), &|| {
                host.wheel(*direction, *amount)
            })
        }
        Action::Type { text } => {
            let chars: Vec<char> = text.chars().collect();
            for (i, chunk) in chars.chunks(TYPE_CHUNK_CHARS).enumerate() {
                if i > 0 {
                    host.sleep(TYPE_CHUNK_GAP_MS)?;
                }
                host.type_text(&chunk.iter().collect::<String>())?;
            }
            Ok(())
        }
        Action::Key { keys, repeat } => {
            for i in 0..*repeat {
                if i > 0 {
                    host.sleep(REPEAT_GAP_MS)?;
                }
                host.key_press(keys)?;
            }
            Ok(())
        }
        Action::HoldKey { keys, duration_ms } => {
            host.key_down(keys)?;
            held.keys.push(keys.clone());
            let waited = host.sleep(*duration_ms);
            held.keys.retain(|k| k != keys);
            let up = host.key_up(keys);
            waited?;
            up
        }
        Action::Wait { duration_ms } => host.sleep(*duration_ms),
        Action::KeyDown { keys } => {
            host.key_down(keys)?;
            if !held.keys.contains(keys) {
                held.keys.push(keys.clone());
            }
            Ok(())
        }
        Action::KeyUp { keys } => {
            held.keys.retain(|k| k != keys);
            host.key_up(keys)
        }
        Action::MouseMoveBy { dx, dy } => host.move_by(*dx, *dy),
    }
}

/// 执行一份已校验的计划。`on_step(i, action)` 在每步开始前调用（气泡显示“正在操作”）。
pub fn execute(
    plan: &[Action],
    obs: &Observation,
    host: &dyn ActHost,
    on_step: &dyn Fn(usize, &Action),
) -> ActReport {
    let mut held = Held::default();
    let mut steps = Vec::with_capacity(plan.len());
    let mut stop: Option<StopReason> = None;
    for (i, action) in plan.iter().enumerate() {
        if let Some(reason) = stop {
            steps.push(StepResult {
                action: action.name(),
                status: StepStatus::NotExecuted,
                message: Some(if reason == StopReason::ActionFailed {
                    "Not executed: an earlier action failed.".into()
                } else {
                    format!("Not executed: stopped ({}).", reason.code())
                }),
            });
            continue;
        }
        if let Err(halt) = host.checkpoint() {
            let (reason, message) = match halt {
                Halt::Stop(reason) => (reason, None),
                Halt::Fail(message) => (StopReason::ActionFailed, Some(message)),
            };
            stop = Some(reason);
            steps.push(StepResult {
                action: action.name(),
                status: StepStatus::NotExecuted,
                message: Some(
                    message
                        .unwrap_or_else(|| format!("Not executed: stopped ({}).", reason.code())),
                ),
            });
            continue;
        }
        on_step(i, action);
        let result = run_step(action, obs, host, &mut held);
        steps.push(match result {
            Ok(()) => StepResult {
                action: action.name(),
                status: StepStatus::Sent,
                message: None,
            },
            Err(Halt::Stop(reason)) => {
                stop = Some(reason);
                StepResult {
                    action: action.name(),
                    // 盖住时一下都没点：算没执行；其余是做到一半被停下。
                    status: if reason == StopReason::PointCovered {
                        StepStatus::NotExecuted
                    } else {
                        StepStatus::Stopped
                    },
                    message: Some(format!("stopped ({})", reason.code())),
                }
            }
            Err(Halt::Fail(message)) => {
                stop = Some(StopReason::ActionFailed);
                StepResult {
                    action: action.name(),
                    status: StepStatus::Failed,
                    message: Some(message),
                }
            }
        });
    }
    // 本次按下没松开的，全部松开（不论成败）。
    for keys in held.keys.iter().rev() {
        let _ = host.key_up(keys);
    }
    if held.left_button {
        let _ = host.button(Button::Left, false);
    }
    ActReport { steps, stop }
}
