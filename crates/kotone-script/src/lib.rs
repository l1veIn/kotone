//! Kotone 自动化的脚本引擎（docs/product-evolution-game-tools.md §12）：大模型写的 Luau 程序在沙盒里运行，
//! 只能调用这里注册的白名单函数。
//!
//! - **宿主端口** [`ScriptHost`]：按键、截图判断、OCR、播报、时钟都由调用方实现（桌面端接真实
//!   输入与截图，测试注入假实现），本 crate 不依赖 Windows / Tauri。
//! - **沙盒**：Luau `sandbox` 模式（全局表只读、每个脚本独立环境），并移除 `loadstring`、`require`、
//!   `getfenv` / `setfenv`、`collectgarbage`；没有文件、网络、进程能力。内存有上限。
//! - **护栏**（[`Limits`]）：总时长、执行步数（中断回调，防死循环）、按键总数、两次按键最小间隔；
//!   取消标记随时生效；游戏窗口失去焦点时按键与打字先暂停等待。
//! - **数据不是指令**：`ocr()` 返回不透明的 `ScreenText`，只能问 `:has("文字")` / `:count()`，
//!   拿不到字符串，因此屏幕上的字无法拼进 `type_text` / `say` 的内容。

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use mlua::{Function, Lua, LuaOptions, StdLib, Table, UserData, UserDataMethods, Value, VmState};

/// 窗口客户区内的区域（0..=1 的比例）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Region {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// 按键动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAction {
    Press,
    Down,
    Up,
}

/// 脚本能用到的全部外部能力。实现必须只作用于启动脚本时的游戏窗口。
pub trait ScriptHost: Send + Sync {
    /// 单调时钟（毫秒）。
    fn now_ms(&self) -> u64;
    /// 等待（可被取消）；返回 false 表示被取消。
    fn sleep(&self, ms: u64, cancel: &Cancel) -> bool;
    /// 游戏窗口是否在前台。
    fn game_focused(&self) -> bool;
    fn key(&self, action: KeyAction, key: &str) -> Result<(), String>;
    fn type_text(&self, text: &str) -> Result<(), String>;
    /// 区域平均亮度（0..=255）。
    fn brightness(&self, region: Region) -> Result<f64, String>;
    /// 某点颜色（窗口比例坐标）。
    fn color_at(&self, x: f64, y: f64) -> Result<[u8; 3], String>;
    /// 区域内识别出的文字行。
    fn ocr(&self, region: Region) -> Result<Vec<String>, String>;
    fn say(&self, text: &str) -> Result<(), String>;
    fn toast(&self, text: &str) -> Result<(), String>;
}

/// 取消标记（停止热键、说“停”、Esc、工具 `script.stop`）。
#[derive(Debug, Clone, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// 运行上限。
#[derive(Debug, Clone)]
pub struct Limits {
    pub max_duration_ms: u64,
    /// 中断回调计数上限（Luau 在函数调用与循环回边处触发），防止纯计算死循环。
    pub max_steps: u64,
    pub max_keys: u64,
    /// 两次按键（含打字）之间的最小间隔；过快时自动等待而不是报错。
    pub min_key_interval_ms: u64,
    /// 判断条件的最短轮询间隔（`wait_until` / `every`）。
    pub min_poll_ms: u64,
    pub memory_bytes: usize,
    pub pause_when_unfocused: bool,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_duration_ms: 30 * 60 * 1000,
            max_steps: 5_000_000,
            max_keys: 10_000,
            min_key_interval_ms: 50,
            min_poll_ms: 50,
            memory_bytes: 16 * 1024 * 1024,
            pause_when_unfocused: true,
        }
    }
}

/// 脚本如何结束。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// 正常跑完，或脚本调用了 `stop()`。
    Finished,
    /// 被取消（停止热键 / 说“停” / 工具）。
    Cancelled,
    /// 超过总时长、执行步数或按键次数。
    LimitReached(String),
    /// 语法错误、调用了不存在的函数、参数不合法、宿主失败。
    Error(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunReport {
    pub outcome: Outcome,
    pub keys: u64,
    pub duration_ms: u64,
    /// `log()` 输出与暂停记录（最多 200 条）。
    pub log: Vec<String>,
}

const STOP: &str = "__kotone_script_stop__";
const CANCELLED: &str = "__kotone_script_cancelled__";
const LIMIT: &str = "__kotone_script_limit__:";

struct State {
    host: Arc<dyn ScriptHost>,
    limits: Limits,
    cancel: Cancel,
    started: u64,
    keys: AtomicU64,
    last_key: Mutex<Option<u64>>,
    log: Mutex<Vec<String>>,
}

impl State {
    fn deadline_check(&self) -> mlua::Result<()> {
        if self.cancel.is_cancelled() {
            return Err(mlua::Error::runtime(CANCELLED));
        }
        if self.host.now_ms().saturating_sub(self.started) > self.limits.max_duration_ms {
            return Err(mlua::Error::runtime(format!("{LIMIT}超过总时长上限")));
        }
        Ok(())
    }

    fn sleep(&self, ms: u64) -> mlua::Result<()> {
        self.deadline_check()?;
        if ms > 0 && !self.host.sleep(ms, &self.cancel) {
            return Err(mlua::Error::runtime(CANCELLED));
        }
        self.deadline_check()
    }

    fn push_log(&self, line: String) {
        let mut log = self.log.lock().unwrap();
        if log.len() < 200 {
            log.push(line);
        }
    }

    /// 按键前：等游戏回到前台、遵守最小间隔、计数。
    fn before_input(&self) -> mlua::Result<()> {
        if self.limits.pause_when_unfocused && !self.host.game_focused() {
            self.push_log("游戏不在前台，按键暂停".into());
            while !self.host.game_focused() {
                self.sleep(200)?;
            }
            self.push_log("游戏回到前台，继续".into());
        }
        let wait = {
            let last = self.last_key.lock().unwrap();
            last.map(|t| (t + self.limits.min_key_interval_ms).saturating_sub(self.host.now_ms()))
        };
        if let Some(ms) = wait.filter(|ms| *ms > 0) {
            self.sleep(ms)?;
        }
        if self.keys.fetch_add(1, Ordering::SeqCst) + 1 > self.limits.max_keys {
            return Err(mlua::Error::runtime(format!("{LIMIT}按键次数超过上限")));
        }
        *self.last_key.lock().unwrap() = Some(self.host.now_ms());
        Ok(())
    }
}

fn host_err(e: String) -> mlua::Error {
    mlua::Error::runtime(e)
}

/// 按键名：字母、数字、F1–F24、常用功能键，可带 Ctrl/Alt/Shift 组合（交给宿主再精确解析）。
fn check_key(key: &str) -> mlua::Result<()> {
    let ok = !key.is_empty()
        && key.len() <= 32
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == ' ');
    if ok {
        Ok(())
    } else {
        Err(mlua::Error::runtime(format!("按键名不合法：{key}")))
    }
}

fn region_from(table: &Table) -> mlua::Result<Region> {
    let r = Region {
        x: table.get("x")?,
        y: table.get("y")?,
        width: table.get("w")?,
        height: table.get("h")?,
    };
    let unit = |v: f64| v.is_finite() && (0.0..=1.0).contains(&v);
    if !(unit(r.x) && unit(r.y) && unit(r.width) && unit(r.height))
        || r.width <= 0.0
        || r.height <= 0.0
        || r.x + r.width > 1.0001
        || r.y + r.height > 1.0001
    {
        return Err(mlua::Error::runtime(
            "区域必须用窗口比例（0 到 1）表示，且不能超出窗口",
        ));
    }
    Ok(r)
}

/// OCR 结果：只能做判断，拿不到原文。
struct ScreenText(Vec<String>);

impl UserData for ScreenText {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        // 识别器常把同一句话切成几段（几个框），所以既按单行找，也按阅读顺序拼起来找；
        // 空白一律忽略，大小写不敏感。
        methods.add_method("has", |_, this, needle: String| {
            let squash = |s: &str| {
                s.chars()
                    .filter(|c| !c.is_whitespace())
                    .collect::<String>()
                    .to_lowercase()
            };
            let needle = squash(&needle);
            if needle.is_empty() {
                return Ok(false);
            }
            let joined: String = this.0.iter().map(|l| squash(l)).collect();
            Ok(joined.contains(&needle))
        });
        methods.add_method("count", |_, this, ()| Ok(this.0.len()));
    }
}

fn opt<T: mlua::FromLua>(opts: &Option<Table>, key: &str) -> mlua::Result<Option<T>> {
    match opts {
        Some(t) => t.get::<Option<T>>(key),
        None => Ok(None),
    }
}

fn install(lua: &Lua, state: Arc<State>) -> mlua::Result<()> {
    let g = lua.globals();
    for name in [
        "loadstring",
        "require",
        "getfenv",
        "setfenv",
        "collectgarbage",
        "print",
    ] {
        g.set(name, Value::Nil)?;
    }

    let s = state.clone();
    g.set(
        "key",
        lua.create_function(move |_, key: String| {
            check_key(&key)?;
            s.before_input()?;
            s.host.key(KeyAction::Press, &key).map_err(host_err)
        })?,
    )?;
    let s = state.clone();
    g.set(
        "key_down",
        lua.create_function(move |_, key: String| {
            check_key(&key)?;
            s.before_input()?;
            s.host.key(KeyAction::Down, &key).map_err(host_err)
        })?,
    )?;
    let s = state.clone();
    g.set(
        "key_up",
        lua.create_function(move |_, key: String| {
            check_key(&key)?;
            s.host.key(KeyAction::Up, &key).map_err(host_err)
        })?,
    )?;
    let s = state.clone();
    g.set(
        "type_text",
        lua.create_function(move |_, text: String| {
            let text = text.trim().to_string();
            if text.is_empty() || text.chars().count() > 200 || text.contains(['\r', '\n']) {
                return Err(mlua::Error::runtime("要打的字为空、超过 200 字或含有换行"));
            }
            s.before_input()?;
            s.host.type_text(&text).map_err(host_err)
        })?,
    )?;

    let s = state.clone();
    g.set("wait", lua.create_function(move |_, ms: u64| s.sleep(ms))?)?;
    let s = state.clone();
    g.set(
        "now",
        lua.create_function(move |_, ()| Ok(s.host.now_ms().saturating_sub(s.started)))?,
    )?;
    let s = state.clone();
    g.set(
        "game_focused",
        lua.create_function(move |_, ()| Ok(s.host.game_focused()))?,
    )?;

    // every(ms, fn, {for_ms=, times=})：固定节拍调用 fn；fn 返回 false 时结束。
    let s = state.clone();
    g.set(
        "every",
        lua.create_function(move |_, (ms, f, opts): (u64, Function, Option<Table>)| {
            let ms = ms.max(s.limits.min_poll_ms);
            let for_ms: Option<u64> = opt(&opts, "for_ms")?;
            let times: Option<u64> = opt(&opts, "times")?;
            let start = s.host.now_ms();
            let mut n = 0u64;
            let mut next = start;
            loop {
                s.deadline_check()?;
                if times.is_some_and(|t| n >= t)
                    || for_ms.is_some_and(|d| s.host.now_ms() - start >= d)
                {
                    return Ok(());
                }
                let keep: Value = f.call(())?;
                n += 1;
                if keep == Value::Boolean(false) {
                    return Ok(());
                }
                // 固定节拍，但不补拍：暂停（游戏不在前台）或回调太慢错过的节拍直接跳过，
                // 从现在起按原间隔继续，避免恢复时连续猛按。
                next += ms;
                let now = s.host.now_ms();
                if next < now {
                    next = now + ms;
                }
                s.sleep(next - now)?;
            }
        })?,
    )?;

    // wait_until(cond, {poll_ms=200, stable_frames=1, timeout_ms=}) → true / false（超时）
    let s = state.clone();
    g.set(
        "wait_until",
        lua.create_function(move |_, (cond, opts): (Function, Option<Table>)| {
            let poll = opt::<u64>(&opts, "poll_ms")?
                .unwrap_or(200)
                .max(s.limits.min_poll_ms);
            let stable = opt::<u64>(&opts, "stable_frames")?.unwrap_or(1).max(1);
            let timeout: Option<u64> = opt(&opts, "timeout_ms")?;
            let start = s.host.now_ms();
            let mut hits = 0u64;
            loop {
                s.deadline_check()?;
                if cond.call::<bool>(())? {
                    hits += 1;
                    if hits >= stable {
                        return Ok(true);
                    }
                } else {
                    hits = 0;
                }
                if timeout.is_some_and(|t| s.host.now_ms() - start >= t) {
                    return Ok(false);
                }
                s.sleep(poll)?;
            }
        })?,
    )?;

    g.set(
        "region",
        lua.create_function(|lua, (x, y, w, h): (f64, f64, f64, f64)| {
            let t = lua.create_table()?;
            t.set("x", x)?;
            t.set("y", y)?;
            t.set("w", w)?;
            t.set("h", h)?;
            region_from(&t)?;
            Ok(t)
        })?,
    )?;
    let s = state.clone();
    g.set(
        "brightness",
        lua.create_function(move |_, r: Table| {
            s.host.brightness(region_from(&r)?).map_err(host_err)
        })?,
    )?;
    let s = state.clone();
    g.set(
        "color_at",
        lua.create_function(move |_, (x, y): (f64, f64)| {
            if !(0.0..=1.0).contains(&x) || !(0.0..=1.0).contains(&y) {
                return Err(mlua::Error::runtime("坐标必须用窗口比例（0 到 1）表示"));
            }
            let [r, g, b] = s.host.color_at(x, y).map_err(host_err)?;
            Ok((r, g, b))
        })?,
    )?;
    let s = state.clone();
    g.set(
        "ocr",
        lua.create_function(move |_, r: Table| {
            Ok(ScreenText(s.host.ocr(region_from(&r)?).map_err(host_err)?))
        })?,
    )?;

    let s = state.clone();
    g.set(
        "say",
        lua.create_function(move |_, text: String| {
            let text = text.trim();
            if text.is_empty() || text.chars().count() > 120 {
                return Err(mlua::Error::runtime("播报内容为空或超过 120 字"));
            }
            s.host.say(text).map_err(host_err)
        })?,
    )?;
    let s = state.clone();
    g.set(
        "toast",
        lua.create_function(move |_, text: String| {
            let text = text.trim();
            if text.is_empty() || text.chars().count() > 120 {
                return Err(mlua::Error::runtime("弹幕内容为空或超过 120 字"));
            }
            s.host.toast(text).map_err(host_err)
        })?,
    )?;
    let s = state.clone();
    g.set(
        "log",
        lua.create_function(move |_, text: String| {
            s.push_log(text.chars().take(200).collect());
            Ok(())
        })?,
    )?;
    g.set(
        "stop",
        lua.create_function(|_, ()| -> mlua::Result<()> { Err(mlua::Error::runtime(STOP)) })?,
    )?;
    Ok(())
}

fn classify(error: &mlua::Error) -> Outcome {
    let text = error.to_string();
    if text.contains(STOP) {
        Outcome::Finished
    } else if text.contains(CANCELLED) {
        Outcome::Cancelled
    } else if let Some(i) = text.find(LIMIT) {
        let rest = &text[i + LIMIT.len()..];
        Outcome::LimitReached(rest.lines().next().unwrap_or("").to_string())
    } else {
        Outcome::Error(
            text.lines()
                .next()
                .unwrap_or("自动化出错")
                .chars()
                .take(300)
                .collect(),
        )
    }
}

/// 运行一段脚本直到结束、取消或触发上限（阻塞；调用方放到专用线程）。
pub fn run(source: &str, host: Arc<dyn ScriptHost>, limits: Limits, cancel: Cancel) -> RunReport {
    let started = host.now_ms();
    let state = Arc::new(State {
        host: host.clone(),
        limits: limits.clone(),
        cancel: cancel.clone(),
        started,
        keys: AtomicU64::new(0),
        last_key: Mutex::new(None),
        log: Mutex::new(Vec::new()),
    });
    let result = (|| -> mlua::Result<()> {
        let libs = StdLib::STRING | StdLib::TABLE | StdLib::MATH | StdLib::UTF8;
        let lua = Lua::new_with(libs, LuaOptions::default())?;
        lua.set_memory_limit(limits.memory_bytes)?;
        install(&lua, state.clone())?;
        lua.sandbox(true)?;
        let steps = Arc::new(AtomicU64::new(0));
        let s = state.clone();
        let max_steps = limits.max_steps;
        lua.set_interrupt(move |_| {
            if steps.fetch_add(1, Ordering::Relaxed) > max_steps {
                return Err(mlua::Error::runtime(format!(
                    "{LIMIT}执行步数超过上限（可能是死循环）"
                )));
            }
            s.deadline_check()?;
            Ok(VmState::Continue)
        });
        lua.load(source).set_name("companion-script").exec()
    })();
    let outcome = match result {
        Ok(()) => Outcome::Finished,
        Err(error) => classify(&error),
    };
    let report = RunReport {
        outcome,
        keys: state.keys.load(Ordering::SeqCst),
        duration_ms: host.now_ms().saturating_sub(started),
        log: state.log.lock().unwrap().clone(),
    };
    report
}

/// 只检查语法（保存脚本、大模型写完后立即校验），不执行。
pub fn check_syntax(source: &str) -> Result<(), String> {
    let lua = Lua::new_with(StdLib::NONE, LuaOptions::default()).map_err(|e| e.to_string())?;
    lua.load(source)
        .set_name("companion-script")
        .into_function()
        .map(|_| ())
        .map_err(|e| {
            e.to_string()
                .lines()
                .next()
                .unwrap_or("语法错误")
                .to_string()
        })
}

#[cfg(test)]
mod tests;
