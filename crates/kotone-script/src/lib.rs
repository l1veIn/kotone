//! Kotone 自动化的脚本引擎（docs/product-evolution-game-tools.md §12）：大模型写的 Luau 程序在沙盒里运行，
//! 只能调用这里注册的白名单函数。
//!
//! - **宿主端口** [`ScriptHost`]：按键、截图判断、OCR、说话、弹幕、推送、交给她、时钟都由调用方实现
//!   （桌面端接真实输入与截图，测试注入假实现），本 crate 不依赖 Windows / Tauri。
//! - **通知、交给她都是函数**（ADR 045 D2）：`say` / `toast` / `push` / `ask` 写了才有，不写就安静；
//!   宿主不替程序加“成功了”的提示。
//! - **用不用游戏窗口看程序里写了什么**（ADR 045 D4）：宿主按源码判断（kotone-core 的 `ProgramUses`）；
//!   没绑窗口的执行里，窗口函数一调用就报错（[`ScriptHost::require_window`]），不进暂停等待。
//! - **沙盒**：Luau `sandbox` 模式（全局表只读、每个脚本独立环境），并移除 `loadstring`、`require`、
//!   `getfenv` / `setfenv`、`collectgarbage`；没有文件、网络、进程能力。内存有上限。
//! - **护栏**（[`Limits`]）：总时长、执行步数（中断回调，防死循环）、按键总数、两次按键最小间隔；
//!   取消标记随时生效；游戏窗口失去焦点时按键与打字先暂停等待。
//! - **数据不是指令**：`ocr()` 返回不透明的 `ScreenText`，只能问 `:has("文字")` / `:count()` /
//!   `:find("文字")`（位置），拿不到字符串，因此屏幕上的字无法拼进 `type_text` / `chat` / `say` 的内容。
//!   `tool()` 不是后门：结果不可信的工具宿主只交回 `{status}`（kotone-core 的 `program_view`，ADR 046 D4）。
//! - **调工具**（ADR 046 D1）：`tool(name, args)` 经 [`ScriptHost::tool`]；名字格式、参数大小（≤4 KB）、两次至少隔
//!   1 秒、结果转只读表（最多 8 层、256 KB）在这里；名字在不在程序的单子上、权限、确认、30 秒、停下由宿主管。
//! - **找色 / 找图 / 找字**：`find_color` / `find_image` 参照 AutoHotkey 的 `PixelSearch` /
//!   `ImageSearch`（区域、颜色、`variation` 容差、透明色、第一个匹配），`find_text` 对区域做 OCR；
//!   都返回窗口比例坐标，可直接交给 `click`。
//! - **入参**：调用时固定、已由调用方按声明校验的值，在程序里是只读全局表 `args`（区域是
//!   `{x, y, w, h}`，颜色是 `{r, g, b}`，可直接交给 `brightness` / `ocr`）。

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

/// 鼠标动作；坐标是窗口客户区比例（0..=1）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MouseAction {
    Move {
        x: f64,
        y: f64,
    },
    /// 移到 (x, y) 再点 `count` 次（1..=3）。
    Click {
        x: f64,
        y: f64,
        button: MouseButton,
        count: u8,
    },
    /// 在当前位置按下 / 松开（拖动 = 移动、按下、移动、松开）。
    Down(MouseButton),
    Up(MouseButton),
    /// 滚轮格数，正数向上（远离用户）。
    Scroll(i32),
}

/// 一块区域的像素（RGBA，逐行）。`origin` 是这块像素在窗口里实际覆盖的区域
/// （截图按像素取整，可能比请求的区域略大），用于把像素坐标换回窗口比例。
#[derive(Debug, Clone, PartialEq)]
pub struct Pixels {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub origin: Region,
}

/// 一行识别结果；`rect` 是它在窗口里的位置（窗口比例），识别模型不给位置时为 None。
#[derive(Debug, Clone, PartialEq)]
pub struct TextLine {
    pub text: String,
    pub rect: Option<Region>,
}

/// 程序里 `tool()` 调一个工具的结果（ADR 046 D1）。
#[derive(Debug, Clone, PartialEq)]
pub enum ToolReply {
    /// 做成了：交给程序的数据。宿主已经按“数据不是指令”挑好（可信的工具是结构化结果，没有就是给她的那份内容；
    /// 不可信的只有 `{status}`，kotone-core 的 `program_view`）。脚本引擎转成只读的表（最多
    /// [`MAX_TOOL_RESULT_DEPTH`] 层、[`MAX_TOOL_RESULT_BYTES`]），太大就当没成。
    Value(serde_json::Value),
    /// 工具自己说没成（插件返回错误、工具服务 ok = false、超时）：程序拿到 `nil, 原因`，接着跑。
    Failed(String),
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
    /// 往游戏聊天里发话（经本体的“发到游戏”入口，ADR 041：游戏配置的聊天键和频道、等修饰键、一把输入锁）。
    /// `channel` 是 `"team"` / `"all"`，None = 默认频道；多行时一行发一条。
    fn chat(&self, text: &str, channel: Option<&str>) -> Result<(), String>;
    /// 区域平均亮度（0..=255）。
    fn brightness(&self, region: Region) -> Result<f64, String>;
    /// 某点颜色（窗口比例坐标）。
    fn color_at(&self, x: f64, y: f64) -> Result<[u8; 3], String>;
    /// 区域的像素（`find_color` 用）。
    fn pixels(&self, region: Region) -> Result<Pixels, String>;
    /// 区域内识别出的文字行（带位置）。
    fn ocr(&self, region: Region) -> Result<Vec<TextLine>, String>;
    /// 按 ID 取 image 类型入参对应的模板图片（`origin` 无意义）。
    fn image(&self, id: &str) -> Result<Pixels, String>;
    /// 鼠标；只作用于游戏窗口，坐标由宿主换成屏幕位置。
    fn mouse(&self, action: MouseAction) -> Result<(), String>;
    /// 她开口说（≤300 字）：阻塞到说完。`toast = true` 时语音和弹幕同时出现（一次投递）。排队、麦克风开着先等、
    /// 语音失败改弹幕、超时和停下由宿主负责；停下以后返回的错误记成“已取消”。
    fn say(&self, text: &str, toast: bool) -> Result<(), String>;
    /// 弹幕（≤300 字），不阻塞。
    fn toast(&self, text: &str) -> Result<(), String>;
    /// 推送（≤40 字、一行）：Kotone 聊天里记一条、悬浮窗显示一行，不调用模型、不出声。两次之间至少隔
    /// 1 秒由脚本引擎保证（太快自动等）。
    fn push(&self, text: &str) -> Result<(), String>;
    /// 交给她（主会话，≤500 字）：阻塞到交出去为止；交出去返回 true，没交出去（她一直忙、过时了）false。
    /// “只在保存的自动化里能用”“话里不能带从游戏事件填进来的参数值”由宿主查（kotone-core 的
    /// `ProgramAction::check_ask`），不通过就返回 Err。
    fn ask(&self, text: &str) -> Result<bool, String>;
    /// 这次执行绑没绑游戏窗口。没绑时返回 Err（说清楚为什么），窗口函数（按键、打字、发到游戏聊天、鼠标、
    /// 看屏幕）一调用就报这个错，不进“游戏不在前台就暂停”的等待；`game_focused()` 返回 false。默认绑了。
    fn require_window(&self) -> Result<(), String> {
        Ok(())
    }
    /// 调一个工具（ADR 046 D1）：阻塞到工具回来。名字格式、`args`（总是 JSON 对象，≤4 KB）、两次之间至少隔 1 秒由
    /// 脚本引擎保证。宿主负责：名字在不在这段程序的单子上（kotone-core 的 `tools_called`）、能不能调（范围、权限、
    /// 确认、插件开关），调不了返回 Err（程序出错结束，执行记录写原因）；最多等 [`TOOL_TIMEOUT_MS`]，超时回
    /// `Ok(Failed(..))`；停下就跟着停，返回的错误记成“已取消”。默认：这里调不了工具。
    fn tool(&self, name: &str, args: &serde_json::Value) -> Result<ToolReply, String> {
        let _ = (name, args);
        Err("这里调不了工具".into())
    }
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
    last_push: Mutex<Option<u64>>,
    last_tool: Mutex<Option<u64>>,
    log: Mutex<Vec<String>>,
    images: Mutex<std::collections::HashMap<String, Arc<Pixels>>>,
}

impl State {
    fn new(host: Arc<dyn ScriptHost>, limits: Limits, cancel: Cancel) -> Arc<Self> {
        Arc::new(Self {
            started: host.now_ms(),
            host,
            limits,
            cancel,
            keys: AtomicU64::new(0),
            last_key: Mutex::new(None),
            last_push: Mutex::new(None),
            last_tool: Mutex::new(None),
            log: Mutex::new(Vec::new()),
            images: Mutex::new(std::collections::HashMap::new()),
        })
    }

    /// 宿主的结果交回脚本。宿主报错时若已经停下，记成“已取消”（停下时正在说的话、正在等她的 `ask`
    /// 会以错误返回，那不是程序出错）。
    fn host_result<T>(&self, result: Result<T, String>) -> mlua::Result<T> {
        result.map_err(|e| {
            if self.cancel.is_cancelled() {
                mlua::Error::runtime(CANCELLED)
            } else {
                mlua::Error::runtime(e)
            }
        })
    }

    /// 窗口函数先问宿主：没绑窗口就立刻报错（ADR 045 D4 兜底）。
    fn require_window(&self) -> mlua::Result<()> {
        self.host_result(self.host.require_window())
    }

    /// 模板图片（每次运行内缓存）。
    fn image(&self, id: &str) -> mlua::Result<Arc<Pixels>> {
        if let Some(image) = self.images.lock().unwrap().get(id) {
            return Ok(image.clone());
        }
        let image = Arc::new(self.host_result(self.host.image(id))?);
        self.images
            .lock()
            .unwrap()
            .insert(id.to_string(), image.clone());
        Ok(image)
    }

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

    /// 输入前：先确认绑了窗口，再遵守最小间隔、等游戏回到前台、计数。前台检查放在间隔等待**之后**，
    /// 否则等待期间切走窗口，输入仍会发出去。
    fn before_input(&self) -> mlua::Result<()> {
        self.require_window()?;
        let wait = {
            let last = self.last_key.lock().unwrap();
            last.map(|t| (t + self.limits.min_key_interval_ms).saturating_sub(self.host.now_ms()))
        };
        if let Some(ms) = wait.filter(|ms| *ms > 0) {
            self.sleep(ms)?;
        }
        if self.limits.pause_when_unfocused && !self.host.game_focused() {
            self.push_log("游戏不在前台，输入暂停".into());
            while !self.host.game_focused() {
                self.sleep(200)?;
            }
            self.push_log("游戏回到前台，继续".into());
        }
        if self.keys.fetch_add(1, Ordering::SeqCst) + 1 > self.limits.max_keys {
            return Err(mlua::Error::runtime(format!(
                "{LIMIT}输入次数（按键与鼠标）超过上限"
            )));
        }
        *self.last_key.lock().unwrap() = Some(self.host.now_ms());
        Ok(())
    }

    /// 推送前：两次之间至少隔 [`MIN_PUSH_INTERVAL_MS`]，太快就等（和按键间隔一样，不报错）。
    fn before_push(&self) -> mlua::Result<()> {
        self.space_out(&self.last_push, MIN_PUSH_INTERVAL_MS)
    }

    /// 调工具前：两次之间至少隔 [`MIN_TOOL_INTERVAL_MS`]（从上一次回来算），太快就等。
    fn before_tool(&self) -> mlua::Result<()> {
        self.space_out(&self.last_tool, MIN_TOOL_INTERVAL_MS)
    }

    /// 离 `last` 记下的时刻不到 `interval` 毫秒就等到够了再继续。
    fn space_out(&self, last: &Mutex<Option<u64>>, interval: u64) -> mlua::Result<()> {
        let wait = {
            let last = last.lock().unwrap();
            last.map(|t| (t + interval).saturating_sub(self.host.now_ms()))
        };
        if let Some(ms) = wait.filter(|ms| *ms > 0) {
            self.sleep(ms)?;
        }
        Ok(())
    }
}

/// `chat()` 一次最多几个字（10 行 × 200 字；行数和每行字数由宿主的入口查）。
const MAX_CHAT_CHARS: usize = 2000;
/// `say()` / `toast()` 一次最多几个字（和提醒一样）。
pub const MAX_SAY_CHARS: usize = 300;
/// `push()` 一次最多几个字（推送一格的上限）。
pub const MAX_PUSH_CHARS: usize = 40;
/// 两次 `push()` 之间至少隔多久。
pub const MIN_PUSH_INTERVAL_MS: u64 = 1_000;
/// `ask()` 那句话最多几个字（和以前的“让她做”一样）。
pub const MAX_ASK_CHARS: usize = 500;
/// `tool()` 的工具名最多几个字符（ADR 046 D1）。
pub const MAX_TOOL_NAME_CHARS: usize = 64;
/// `tool()` 的参数（转成 JSON）最多多少字节。
pub const MAX_TOOL_ARGS_BYTES: usize = 4_096;
/// 交给程序的工具结果（JSON）最多多少字节；超过就当没成。
pub const MAX_TOOL_RESULT_BYTES: usize = 256 * 1024;
/// 交给程序的工具结果最多几层表。
pub const MAX_TOOL_RESULT_DEPTH: usize = 8;
/// 两次 `tool()` 之间至少隔多久（从上一次回来算，太快自动等）。
pub const MIN_TOOL_INTERVAL_MS: u64 = 1_000;
/// 宿主等一个工具最多多久（到了回 [`ToolReply::Failed`]）。脚本引擎打断不了阻塞中的宿主调用，由宿主照这个数等。
pub const TOOL_TIMEOUT_MS: u64 = 30_000;
/// 工具没成时交给程序的原因最多几个字。
const MAX_TOOL_REASON_CHARS: usize = 300;

/// 工具名的格式（ADR 046 D1）：字母开头，只有字母、数字、`_`、`-`、`.`，最多 [`MAX_TOOL_NAME_CHARS`] 个字符，
/// 不以 `.` 结尾、没有连着的 `..`。没有点的（`music`）、第三方的大写和连字符都认。kotone-core 的
/// `library::is_tool_name` 是同一条规矩（测试对着查）。
pub fn is_tool_name(name: &str) -> bool {
    name.len() <= MAX_TOOL_NAME_CHARS
        && name.starts_with(|c: char| c.is_ascii_alphabetic())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
        && !name.ends_with('.')
        && !name.contains("..")
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

/// OCR 结果：只能做判断（`:has` / `:count`）和取位置（`:find`），拿不到原文。
struct ScreenText(Vec<TextLine>);

fn squash(s: &str) -> Vec<char> {
    s.chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect()
}

impl ScreenText {
    /// 识别器常把同一句话切成几段（几个框），所以按阅读顺序拼起来找；空白一律忽略，
    /// 大小写不敏感。返回匹配覆盖的（行号, 行内起点, 行内终点）范围。
    fn locate(&self, needle: &str) -> Option<Vec<(usize, usize, usize)>> {
        let needle = squash(needle);
        if needle.is_empty() {
            return None;
        }
        let mut joined = Vec::new();
        for (line, text) in self.0.iter().enumerate() {
            for (pos, c) in squash(&text.text).into_iter().enumerate() {
                joined.push((c, line, pos));
            }
        }
        let start = joined
            .windows(needle.len())
            .position(|w| w.iter().map(|(c, _, _)| *c).eq(needle.iter().copied()))?;
        let span = &joined[start..start + needle.len()];
        let mut ranges: Vec<(usize, usize, usize)> = Vec::new();
        for &(_, line, pos) in span {
            match ranges.last_mut() {
                Some(last) if last.0 == line => last.2 = pos + 1,
                _ => ranges.push((line, pos, pos + 1)),
            }
        }
        Some(ranges)
    }

    /// 匹配文字的中心（窗口比例）。只在一行内：按字符比例估计横向位置；跨行：取这些行的外接框中心。
    fn find(&self, needle: &str) -> mlua::Result<Option<(f64, f64)>> {
        let Some(ranges) = self.locate(needle) else {
            return Ok(None);
        };
        let rects: Vec<(Region, usize, usize, usize)> = ranges
            .iter()
            .map(|&(line, a, b)| {
                let rect = self.0[line].rect.ok_or_else(|| {
                    mlua::Error::runtime("当前文字识别模型不提供文字位置，没法找文字在哪")
                })?;
                Ok((rect, squash(&self.0[line].text).len().max(1), a, b))
            })
            .collect::<mlua::Result<_>>()?;
        if let [(rect, len, a, b)] = rects.as_slice() {
            let mid = (*a + *b) as f64 / 2.0 / *len as f64;
            return Ok(Some((
                rect.x + rect.width * mid,
                rect.y + rect.height / 2.0,
            )));
        }
        let left = rects.iter().map(|r| r.0.x).fold(f64::MAX, f64::min);
        let top = rects.iter().map(|r| r.0.y).fold(f64::MAX, f64::min);
        let right = rects
            .iter()
            .map(|r| r.0.x + r.0.width)
            .fold(f64::MIN, f64::max);
        let bottom = rects
            .iter()
            .map(|r| r.0.y + r.0.height)
            .fold(f64::MIN, f64::max);
        Ok(Some(((left + right) / 2.0, (top + bottom) / 2.0)))
    }
}

impl UserData for ScreenText {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("has", |_, this, needle: String| {
            Ok(this.locate(&needle).is_some())
        });
        methods.add_method("count", |_, this, ()| Ok(this.0.len()));
        methods.add_method("find", |_, this, needle: String| {
            Ok(match this.find(&needle)? {
                Some((x, y)) => (Some(x), Some(y)),
                None => (None, None),
            })
        });
    }
}

/// 颜色参数：`{r, g, b}` 表或 `"#RRGGBB"`。
fn color_from(value: &Value) -> mlua::Result<[u8; 3]> {
    let bad = || mlua::Error::runtime("颜色要写成 {r=…, g=…, b=…}（0 到 255）或 \"#RRGGBB\"");
    match value {
        Value::Table(t) => {
            let get = |k: &str| -> mlua::Result<u8> {
                let v: f64 = t.get(k).map_err(|_| bad())?;
                if v.fract() == 0.0 && (0.0..=255.0).contains(&v) {
                    Ok(v as u8)
                } else {
                    Err(bad())
                }
            };
            Ok([get("r")?, get("g")?, get("b")?])
        }
        Value::String(s) => {
            let s = s.to_str().map_err(|_| bad())?;
            let hex = s
                .strip_prefix('#')
                .filter(|h| h.len() == 6)
                .ok_or_else(bad)?;
            let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).map_err(|_| bad());
            Ok([byte(0)?, byte(2)?, byte(4)?])
        }
        _ => Err(bad()),
    }
}

fn near(p: &[u8], color: [u8; 3], variation: u8) -> bool {
    p.iter().zip(color).all(|(a, b)| a.abs_diff(b) <= variation)
}

/// 像素坐标（可为小数，表示像素中心）→ 窗口比例。
fn to_window(pixels: &Pixels, px: f64, py: f64) -> (f64, f64) {
    let o = pixels.origin;
    (
        o.x + px / f64::from(pixels.width) * o.width,
        o.y + py / f64::from(pixels.height) * o.height,
    )
}

/// 类似 AutoHotkey 的 `PixelSearch`：每个通道与 `color` 相差不超过 `variation`（0 = 完全相同）
/// 的像素。`largest = false` 时按从上到下、从左到右返回**第一个**匹配像素的中心；
/// `largest = true` 时返回**最大一块相连区域**（4 邻接）的中心，适合拿去点击。
/// 结果是窗口比例坐标与匹配的像素数（第一个匹配时为 1）；少于 `min_pixels` 视为没找到。
pub fn find_color_in(
    pixels: &Pixels,
    color: [u8; 3],
    variation: u8,
    largest: bool,
    min_pixels: u32,
) -> Option<(f64, f64, u32)> {
    let (w, h) = (pixels.width as usize, pixels.height as usize);
    if w == 0 || h == 0 || pixels.rgba.len() < w * h * 4 {
        return None;
    }
    let hit = |i: usize| near(&pixels.rgba[i * 4..i * 4 + 3], color, variation);
    if !largest {
        let i = (0..w * h).find(|&i| hit(i))?;
        let (x, y) = to_window(pixels, (i % w) as f64 + 0.5, (i / w) as f64 + 0.5);
        return (min_pixels <= 1).then_some((x, y, 1));
    }
    let mut seen = vec![false; w * h];
    let mut best: Option<(u64, u64, u32)> = None;
    let mut stack = Vec::new();
    for start in 0..w * h {
        if seen[start] || !hit(start) {
            continue;
        }
        seen[start] = true;
        stack.push(start);
        let (mut sx, mut sy, mut n) = (0u64, 0u64, 0u32);
        while let Some(i) = stack.pop() {
            let (x, y) = (i % w, i / w);
            sx += x as u64;
            sy += y as u64;
            n += 1;
            let mut visit = |j: usize| {
                if !seen[j] && hit(j) {
                    seen[j] = true;
                    stack.push(j);
                }
            };
            if x > 0 {
                visit(i - 1);
            }
            if x + 1 < w {
                visit(i + 1);
            }
            if y > 0 {
                visit(i - w);
            }
            if y + 1 < h {
                visit(i + w);
            }
        }
        if best.map_or(true, |b| n > b.2) {
            best = Some((sx, sy, n));
        }
    }
    let (sx, sy, n) = best.filter(|b| b.2 >= min_pixels.max(1))?;
    let (x, y) = to_window(
        pixels,
        sx as f64 / f64::from(n) + 0.5,
        sy as f64 / f64::from(n) + 0.5,
    );
    Some((x, y, n))
}

/// 类似 AutoHotkey 的 `ImageSearch`：在 `haystack` 里找 `needle`，每个像素每个通道相差不超过
/// `variation`；`trans` 颜色的模板像素不参与比较（AHK 的 `*TransN`）。按从上到下、从左到右
/// 返回**第一个**匹配位置（窗口比例的左上角与中心）。模板按原尺寸比较，不缩放：游戏分辨率
/// 或界面缩放变了要重新截模板。`stop()` 每行检查一次，返回 true 时放弃。
pub fn find_image_in(
    haystack: &Pixels,
    needle: &Pixels,
    variation: u8,
    trans: Option<[u8; 3]>,
    stop: &dyn Fn() -> bool,
) -> Option<((f64, f64), (f64, f64))> {
    let (hw, hh) = (haystack.width as usize, haystack.height as usize);
    let (nw, nh) = (needle.width as usize, needle.height as usize);
    if nw == 0 || nh == 0 || nw > hw || nh > hh {
        return None;
    }
    if haystack.rgba.len() < hw * hh * 4 || needle.rgba.len() < nw * nh * 4 {
        return None;
    }
    // 参与比较的模板像素（去掉透明色与全透明像素），第一个用作快速排除。
    let points: Vec<(usize, usize, [u8; 3])> = (0..nw * nh)
        .filter_map(|i| {
            let p = &needle.rgba[i * 4..i * 4 + 4];
            let rgb = [p[0], p[1], p[2]];
            (p[3] != 0 && trans != Some(rgb)).then_some((i % nw, i / nw, rgb))
        })
        .collect();
    if points.is_empty() {
        return None;
    }
    for y in 0..=hh - nh {
        if stop() {
            return None;
        }
        for x in 0..=hw - nw {
            let matches = points.iter().all(|&(dx, dy, rgb)| {
                let i = ((y + dy) * hw + x + dx) * 4;
                near(&haystack.rgba[i..i + 3], rgb, variation)
            });
            if matches {
                let top_left = to_window(haystack, x as f64, y as f64);
                let center = to_window(
                    haystack,
                    x as f64 + nw as f64 / 2.0,
                    y as f64 + nh as f64 / 2.0,
                );
                return Some((top_left, center));
            }
        }
    }
    None
}

fn button_from(name: Option<String>) -> mlua::Result<MouseButton> {
    match name.as_deref().unwrap_or("left") {
        "left" => Ok(MouseButton::Left),
        "right" => Ok(MouseButton::Right),
        "middle" => Ok(MouseButton::Middle),
        other => Err(mlua::Error::runtime(format!(
            "鼠标键只能是 left / right / middle（收到 {other}）"
        ))),
    }
}

fn check_point(x: f64, y: f64) -> mlua::Result<()> {
    if x.is_finite() && y.is_finite() && (0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y) {
        Ok(())
    } else {
        Err(mlua::Error::runtime("坐标必须用窗口比例（0 到 1）表示"))
    }
}

fn opt<T: mlua::FromLua>(opts: &Option<Table>, key: &str) -> mlua::Result<Option<T>> {
    match opts {
        Some(t) => t.get::<Option<T>>(key),
        None => Ok(None),
    }
}

/// 一段要交给宿主的话：去掉首尾空白，不能为空、不能超过 `max` 字。
fn words(text: &str, max: usize, what: &str) -> mlua::Result<String> {
    let text = text.trim();
    if text.is_empty() || text.chars().count() > max {
        return Err(mlua::Error::runtime(format!("{what}为空或超过 {max} 字")));
    }
    Ok(text.to_string())
}

/// 白名单函数（kotone-core 的 `WINDOW_FUNCTIONS` / `PLAIN_FUNCTIONS` 给每个都归了类，测试保证不漏）。
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
            s.host_result(s.host.key(KeyAction::Press, &key))
        })?,
    )?;
    let s = state.clone();
    g.set(
        "key_down",
        lua.create_function(move |_, key: String| {
            check_key(&key)?;
            s.before_input()?;
            s.host_result(s.host.key(KeyAction::Down, &key))
        })?,
    )?;
    let s = state.clone();
    g.set(
        "key_up",
        lua.create_function(move |_, key: String| {
            check_key(&key)?;
            s.require_window()?;
            s.host_result(s.host.key(KeyAction::Up, &key))
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
            s.host_result(s.host.type_text(&text))
        })?,
    )?;

    let s = state.clone();
    g.set(
        "chat",
        lua.create_function(move |_, (text, opts): (String, Option<Table>)| {
            let text = text.trim().to_string();
            if text.is_empty() || text.chars().count() > MAX_CHAT_CHARS {
                return Err(mlua::Error::runtime(format!(
                    "要发的话为空或超过 {MAX_CHAT_CHARS} 字"
                )));
            }
            let channel: Option<String> = opt(&opts, "channel")?;
            if channel
                .as_deref()
                .is_some_and(|c| c != "team" && c != "all")
            {
                return Err(mlua::Error::runtime(
                    "channel 只能是 \"team\"（队伍）或 \"all\"（所有人）",
                ));
            }
            s.before_input()?;
            s.host_result(s.host.chat(&text, channel.as_deref()))
        })?,
    )?;

    let s = state.clone();
    g.set("wait", lua.create_function(move |_, ms: u64| s.sleep(ms))?)?;
    let s = state.clone();
    g.set(
        "now",
        lua.create_function(move |_, ()| Ok(s.host.now_ms().saturating_sub(s.started)))?,
    )?;
    // 没绑窗口的执行里，游戏永远“不在前台”（ADR 045 D4）。
    let s = state.clone();
    g.set(
        "game_focused",
        lua.create_function(move |_, ()| {
            Ok(s.host.require_window().is_ok() && s.host.game_focused())
        })?,
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
            let region = region_from(&r)?;
            s.require_window()?;
            s.host_result(s.host.brightness(region))
        })?,
    )?;
    let s = state.clone();
    g.set(
        "color_at",
        lua.create_function(move |_, (x, y): (f64, f64)| {
            check_point(x, y)?;
            s.require_window()?;
            let [r, g, b] = s.host_result(s.host.color_at(x, y))?;
            Ok((r, g, b))
        })?,
    )?;
    let s = state.clone();
    g.set(
        "ocr",
        lua.create_function(move |_, r: Table| {
            let region = region_from(&r)?;
            s.require_window()?;
            Ok(ScreenText(s.host_result(s.host.ocr(region))?))
        })?,
    )?;
    // find_text(region, text) → x, y（文字中心，窗口比例）或 nil：对区域做一次 OCR 再找。
    let s = state.clone();
    g.set(
        "find_text",
        lua.create_function(move |_, (r, needle): (Table, String)| {
            let region = region_from(&r)?;
            s.require_window()?;
            let text = ScreenText(s.host_result(s.host.ocr(region))?);
            Ok(match text.find(&needle)? {
                Some((x, y)) => (Some(x), Some(y)),
                None => (None, None),
            })
        })?,
    )?;
    // find_color(region, color, {variation=0, largest=false, min_pixels=1}) → x, y, 像素数 或 nil
    let s = state.clone();
    g.set(
        "find_color",
        lua.create_function(move |_, (r, color, opts): (Table, Value, Option<Table>)| {
            let region = region_from(&r)?;
            let color = color_from(&color)?;
            let variation = opt::<u8>(&opts, "variation")?.unwrap_or(0);
            let largest = opt::<bool>(&opts, "largest")?.unwrap_or(false);
            let min_pixels = opt::<u32>(&opts, "min_pixels")?.unwrap_or(1);
            s.require_window()?;
            let pixels = s.host_result(s.host.pixels(region))?;
            Ok(
                match find_color_in(&pixels, color, variation, largest, min_pixels) {
                    Some((x, y, n)) => (Some(x), Some(y), Some(n)),
                    None => (None, None, None),
                },
            )
        })?,
    )?;
    // find_image(region, image, {variation=0, trans=color}) → 中心 x, y, 左上 x, y 或 nil。
    // image 来自 image 类型的入参（args.<name>）。
    let s = state.clone();
    g.set(
        "find_image",
        lua.create_function(move |_, (r, image, opts): (Table, Table, Option<Table>)| {
            let region = region_from(&r)?;
            let id: String = image.get::<Option<String>>("image")?.ok_or_else(|| {
                mlua::Error::runtime("find_image 的图片要来自 image 类型的入参（args.<名字>）")
            })?;
            let variation = opt::<u8>(&opts, "variation")?.unwrap_or(0);
            let trans = match &opts {
                Some(t) => match t.get::<Value>("trans")? {
                    Value::Nil => None,
                    v => Some(color_from(&v)?),
                },
                None => None,
            };
            s.require_window()?;
            let needle = s.image(&id)?;
            let haystack = s.host_result(s.host.pixels(region))?;
            let cancel = s.cancel.clone();
            let found = find_image_in(&haystack, &needle, variation, trans, &move || {
                cancel.is_cancelled()
            });
            s.deadline_check()?;
            Ok(match found {
                Some(((lx, ly), (cx, cy))) => (Some(cx), Some(cy), Some(lx), Some(ly)),
                None => (None, None, None, None),
            })
        })?,
    )?;

    // 鼠标：每个动作都算一次输入（与按键共用前台检查、最小间隔和次数上限）。
    let s = state.clone();
    g.set(
        "click",
        lua.create_function(move |_, (x, y, opts): (f64, f64, Option<Table>)| {
            check_point(x, y)?;
            let button = button_from(opt(&opts, "button")?)?;
            let count = opt::<u8>(&opts, "count")?.unwrap_or(1);
            if !(1..=3).contains(&count) {
                return Err(mlua::Error::runtime("count 只能是 1 到 3"));
            }
            s.before_input()?;
            s.host_result(s.host.mouse(MouseAction::Click {
                x,
                y,
                button,
                count,
            }))
        })?,
    )?;
    let s = state.clone();
    g.set(
        "mouse_move",
        lua.create_function(move |_, (x, y): (f64, f64)| {
            check_point(x, y)?;
            s.before_input()?;
            s.host_result(s.host.mouse(MouseAction::Move { x, y }))
        })?,
    )?;
    let s = state.clone();
    g.set(
        "mouse_down",
        lua.create_function(move |_, button: Option<String>| {
            let button = button_from(button)?;
            s.before_input()?;
            s.host_result(s.host.mouse(MouseAction::Down(button)))
        })?,
    )?;
    let s = state.clone();
    g.set(
        "mouse_up",
        lua.create_function(move |_, button: Option<String>| {
            let button = button_from(button)?;
            s.require_window()?;
            s.host_result(s.host.mouse(MouseAction::Up(button)))
        })?,
    )?;
    let s = state.clone();
    g.set(
        "scroll",
        lua.create_function(move |_, clicks: i32| {
            if clicks == 0 || clicks.abs() > 20 {
                return Err(mlua::Error::runtime("滚轮格数要在 -20 到 20 之间且不为 0"));
            }
            s.before_input()?;
            s.host_result(s.host.mouse(MouseAction::Scroll(clicks)))
        })?,
    )?;

    // say(text, {toast = true})：她开口说，阻塞到说完；toast = true 时语音和弹幕同时出现。
    let s = state.clone();
    g.set(
        "say",
        lua.create_function(move |_, (text, opts): (String, Option<Table>)| {
            let text = words(&text, MAX_SAY_CHARS, "要说的话")?;
            let toast = opt::<bool>(&opts, "toast")?.unwrap_or(false);
            s.deadline_check()?;
            s.host_result(s.host.say(&text, toast))
        })?,
    )?;
    let s = state.clone();
    g.set(
        "toast",
        lua.create_function(move |_, text: String| {
            let text = words(&text, MAX_SAY_CHARS, "弹幕内容")?;
            s.host_result(s.host.toast(&text))
        })?,
    )?;
    // push(text)：Kotone 聊天里记一条、悬浮窗显示一行；不调用模型、不出声。
    let s = state.clone();
    g.set(
        "push",
        lua.create_function(move |_, text: String| {
            let text = words(&text, MAX_PUSH_CHARS, "推送内容")?;
            if text.contains(['\r', '\n']) {
                return Err(mlua::Error::runtime("推送只有一行，不能换行"));
            }
            s.before_push()?;
            let result = s.host_result(s.host.push(&text));
            *s.last_push.lock().unwrap() = Some(s.host.now_ms());
            result
        })?,
    )?;
    // ask(text) → true（交给她了）/ false（没交出去）：每次花一次 AI 调用。
    let s = state.clone();
    g.set(
        "ask",
        lua.create_function(move |_, text: String| {
            let text = words(&text, MAX_ASK_CHARS, "交给她的话")?;
            s.deadline_check()?;
            s.host_result(s.host.ask(&text))
        })?,
    )?;
    // tool(name, args) → 结果（只读的表）；工具自己说没成 → nil, 原因；调不了 → 程序出错结束（ADR 046 D1）。
    let s = state.clone();
    g.set(
        "tool",
        lua.create_function(move |lua, (name, args): (Value, Value)| {
            let name = match &name {
                Value::String(s) => s.to_str().ok().map(|s| s.to_string()),
                _ => None,
            }
            .filter(|n| is_tool_name(n))
            .ok_or_else(|| {
                mlua::Error::runtime(
                    "tool() 的第一个参数要直接写工具名，例如 tool(\"music\", { action = \"play\" })",
                )
            })?;
            let args = tool_args(&args)?;
            s.before_tool()?;
            s.deadline_check()?;
            let reply = s.host_result(s.host.tool(&name, &args));
            *s.last_tool.lock().unwrap() = Some(s.host.now_ms());
            let reply = reply?;
            // 停下了：正在等的那次记成已取消，不再往下跑
            s.deadline_check()?;
            let failed = |reason: &str| -> mlua::Result<(Value, Value)> {
                let reason: String = reason.trim().chars().take(MAX_TOOL_REASON_CHARS).collect();
                let reason = if reason.is_empty() { "没成".into() } else { reason };
                Ok((Value::Nil, Value::String(lua.create_string(reason)?)))
            };
            match reply {
                ToolReply::Failed(reason) => failed(&reason),
                ToolReply::Value(value) => match tool_result(lua, &value) {
                    Some(result) => Ok((result?, Value::Nil)),
                    None => failed(&format!(
                        "{name} 的结果太大（超过 {} KB）或嵌套太深（超过 {MAX_TOOL_RESULT_DEPTH} 层），程序拿不到",
                        MAX_TOOL_RESULT_BYTES / 1024
                    )),
                },
            }
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

/// 入参（`args`）最多嵌套几层。
const MAX_ARGS_DEPTH: usize = 4;

/// JSON 值 → 只读的 Luau 值（对象、数组递归转换为只读表）；比 `max_depth` 深就报错。
fn lua_value(
    lua: &Lua,
    value: &serde_json::Value,
    depth: usize,
    max_depth: usize,
) -> mlua::Result<Value> {
    if depth > max_depth {
        return Err(mlua::Error::runtime("入参嵌套过深"));
    }
    Ok(match value {
        serde_json::Value::Null => Value::Nil,
        serde_json::Value::Bool(b) => Value::Boolean(*b),
        serde_json::Value::Number(n) => Value::Number(n.as_f64().unwrap_or(0.0)),
        serde_json::Value::String(s) => Value::String(lua.create_string(s)?),
        serde_json::Value::Array(items) => {
            let t = lua.create_table()?;
            for (i, item) in items.iter().enumerate() {
                t.set(i + 1, lua_value(lua, item, depth + 1, max_depth)?)?;
            }
            t.set_readonly(true);
            Value::Table(t)
        }
        serde_json::Value::Object(map) => Value::Table(args_table(lua, map, depth + 1, max_depth)?),
    })
}

fn args_table(
    lua: &Lua,
    map: &serde_json::Map<String, serde_json::Value>,
    depth: usize,
    max_depth: usize,
) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    for (k, v) in map {
        t.set(k.as_str(), lua_value(lua, v, depth, max_depth)?)?;
    }
    t.set_readonly(true);
    Ok(t)
}

/// JSON 有几层表（对象、数组各算一层，文字、数字 0 层）。
fn json_depth(value: &serde_json::Value) -> usize {
    match value {
        serde_json::Value::Array(items) => 1 + items.iter().map(json_depth).max().unwrap_or(0),
        serde_json::Value::Object(map) => 1 + map.values().map(json_depth).max().unwrap_or(0),
        _ => 0,
    }
}

/// 工具结果 → 只读的 Luau 值（ADR 046 D1）。超过 [`MAX_TOOL_RESULT_BYTES`] 或 [`MAX_TOOL_RESULT_DEPTH`] 层返回
/// None（当没成）。做成了的结果总是真值：没有内容（null）给一张空表，`if not r` 只在没成时成立。
fn tool_result(lua: &Lua, value: &serde_json::Value) -> Option<mlua::Result<Value>> {
    let bytes = serde_json::to_vec(value).map_or(usize::MAX, |b| b.len());
    if bytes > MAX_TOOL_RESULT_BYTES || json_depth(value) > MAX_TOOL_RESULT_DEPTH {
        return None;
    }
    Some(match value {
        serde_json::Value::Null => lua.create_table().map(|t| {
            t.set_readonly(true);
            Value::Table(t)
        }),
        value => lua_value(lua, value, 0, MAX_TOOL_RESULT_DEPTH),
    })
}

/// `tool()` 的第二个参数 → JSON 对象（不写 = `{}`）。表里只能有文字、数字、真假和表；数组从 1 连续编号；
/// 转出来不超过 [`MAX_TOOL_ARGS_BYTES`]。
fn tool_args(value: &Value) -> mlua::Result<serde_json::Value> {
    let bad = || {
        mlua::Error::runtime(
            "tool() 的第二个参数要写成表，例如 { action = \"play\", query = \"晴天\" }",
        )
    };
    let args = match value {
        Value::Nil => serde_json::Value::Object(serde_json::Map::new()),
        Value::Table(_) => match json_from_lua(value, 1)? {
            args @ serde_json::Value::Object(_) => args,
            _ => return Err(bad()),
        },
        _ => return Err(bad()),
    };
    if serde_json::to_vec(&args).map_or(usize::MAX, |b| b.len()) > MAX_TOOL_ARGS_BYTES {
        return Err(mlua::Error::runtime(format!(
            "tool() 的参数超过 {} KB",
            MAX_TOOL_ARGS_BYTES / 1024
        )));
    }
    Ok(args)
}

/// 数字 → JSON：整数写成整数（Luau 的数都是小数，工具的参数常要整数）。
fn number_json(n: f64) -> mlua::Result<serde_json::Value> {
    if !n.is_finite() {
        return Err(mlua::Error::runtime("tool() 的参数里不能有 NaN 或无穷大"));
    }
    if n.fract() == 0.0 && n.abs() < 9.0e15 {
        return Ok(serde_json::Value::from(n as i64));
    }
    Ok(serde_json::Number::from_f64(n).map_or(serde_json::Value::Null, serde_json::Value::Number))
}

/// Luau 值 → JSON（`tool()` 的参数用）：空表是 `{}`；键都是 1..n 的表是数组；键都是名字的是对象。
fn json_from_lua(value: &Value, depth: usize) -> mlua::Result<serde_json::Value> {
    use serde_json::Value as Json;
    let fail = |why: &str| mlua::Error::runtime(format!("tool() 的参数{why}"));
    if matches!(value, Value::Table(_)) && depth > MAX_TOOL_RESULT_DEPTH {
        return Err(fail(&format!(
            "嵌套太深（最多 {MAX_TOOL_RESULT_DEPTH} 层）"
        )));
    }
    Ok(match value {
        Value::Nil => Json::Null,
        Value::Boolean(b) => Json::Bool(*b),
        Value::Integer(i) => Json::from(*i),
        Value::Number(n) => number_json(*n)?,
        Value::String(s) => Json::String(
            s.to_str()
                .map_err(|_| fail("里有不是 UTF-8 的文字"))?
                .to_string(),
        ),
        Value::Table(t) => {
            let mut items: Vec<(i64, Json)> = Vec::new();
            let mut fields = serde_json::Map::new();
            for pair in t.pairs::<Value, Value>() {
                let (key, item) = pair?;
                let item = json_from_lua(&item, depth + 1)?;
                match key {
                    Value::String(key) => {
                        let key = key.to_str().map_err(|_| fail("里有不是 UTF-8 的名字"))?;
                        fields.insert(key.to_string(), item);
                    }
                    Value::Integer(i) => items.push((i64::from(i), item)),
                    Value::Number(n) if n.fract() == 0.0 && n.abs() < 9.0e15 => {
                        items.push((n as i64, item))
                    }
                    _ => return Err(fail("里，表的键只能是名字或 1、2、3……")),
                }
            }
            if !items.is_empty() && !fields.is_empty() {
                return Err(fail("里，一张表不能既有名字又有 1、2、3……"));
            }
            if items.is_empty() {
                Json::Object(fields)
            } else {
                items.sort_by_key(|(i, _)| *i);
                if items
                    .iter()
                    .enumerate()
                    .any(|(n, (i, _))| *i != n as i64 + 1)
                {
                    return Err(fail("里的列表要从 1 开始连续编号"));
                }
                Json::Array(items.into_iter().map(|(_, item)| item).collect())
            }
        }
        _ => return Err(fail("里只能有文字、数字、真假和表")),
    })
}

/// 运行一段脚本直到结束、取消或触发上限（阻塞；调用方放到专用线程）。
pub fn run(source: &str, host: Arc<dyn ScriptHost>, limits: Limits, cancel: Cancel) -> RunReport {
    run_with_args(source, &serde_json::Map::new(), host, limits, cancel)
}

/// 同 [`run`]，并把 `args` 作为只读全局表交给程序。
pub fn run_with_args(
    source: &str,
    args: &serde_json::Map<String, serde_json::Value>,
    host: Arc<dyn ScriptHost>,
    limits: Limits,
    cancel: Cancel,
) -> RunReport {
    let state = State::new(host.clone(), limits.clone(), cancel.clone());
    let started = state.started;
    let result = (|| -> mlua::Result<()> {
        let libs = StdLib::STRING | StdLib::TABLE | StdLib::MATH | StdLib::UTF8;
        let lua = Lua::new_with(libs, LuaOptions::default())?;
        lua.set_memory_limit(limits.memory_bytes)?;
        install(&lua, state.clone())?;
        lua.globals()
            .set("args", args_table(&lua, args, 0, MAX_ARGS_DEPTH)?)?;
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
        lua.load(source).set_name("automation").exec()
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
        .set_name("automation")
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
