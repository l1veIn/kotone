//! 自动化的桌面执行器（docs/adr/013-automation-domain-model.md、ADR 045、演进计划 §12）。
//!
//! - 自动化只剩程序（ADR 045 D1）：执行记录由 core 的 [`AutomationScheduler`] 管（存盘、延迟、取消、错过不补、
//!   名额）；本模块是它的 [`ProgramLauncher`]：到点（或立即）启动一段 Luau 程序，结束时回报 `finish_program`。
//!   提醒也是程序（“晚一会儿开始、只说一句”，D3）。
//! - 每个程序在自己的线程里由 `kotone_script::run` 执行；宿主 [`DesktopHost`] 把白名单函数接到
//!   真实能力：按键 / 打字走 `SendInput`（与“帮忙打字”同一条输入通道），看屏幕用 GDI 截游戏
//!   客户区，`ocr()` 用陪玩选定的 OCR 模型；`say` / `toast` 走她的输出队列，`push` 走推送那条路，
//!   `ask` 交给她（D2）。宿主不替程序加“成功了”的提示：要通知就在程序里写，不写就安静。
//! - **用不用游戏窗口看程序里写了什么**（D4，[`ProgramUses`]）。用到的只对下指令时的游戏窗口生效：安排执行时
//!   前台要是允许自动化输入的游戏、记下这个窗口（[`RunTarget`]）；到点时窗口已关闭、她在操作游戏就不启动；
//!   运行中按键、截图前先确认它在前台，不在就暂停等待；窗口关闭则结束；占名额（D5）、挡她操作游戏。
//!   没用到的不看前台、不绑窗口，不会因为窗口没了失败或被停，不挡她操作游戏；窗口函数在这种执行里一调用就报错。
//! - **按住的键一定会松开**：`key_down` 记账，程序结束（正常、出错、取消、超限）时全部 `key_up`。
//! - 停下（停止自动化热键、托盘、说“停下”、不带 id 的 `automation.stop`，D6）：运行中的全停；等待中的只取消
//!   绑了游戏窗口的，等待中的提醒留着。应用退出只停止运行中的程序，等待中的留到下次启动（过期则记为错过）。
//! - **程序里调工具**（ADR 046，[`ToolRules`]）：能调的范围、权限、插件开关和热键同一张表、同一份目录；要问的由玩家在
//!   「自动化」页确认一次（[`automation_confirm`]、“允许并保存”，只有 Kotone 窗口能写）。安排时查一遍：等你确认的、
//!   调不了的不运行（▶ 和热键说原因、她拿到错误、游戏事件和定时只记日志）；运行中每次 `tool()` 再判一次
//!   （[`program::DesktopHost`]）。对外的定义不带确认的原文，带每个工具现在的状态（`tools[]`、`needsConfirm`）。

mod program;

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use kotone_agent::catalog::{Catalog, ToolEntry};
use kotone_agent::permissions::{program_callable, program_decision, BindingDecision};
use kotone_core::automation::library::{
    self, validate_args, Automation, AutomationBody, AutomationDraft, AutomationLibrary, CreatedBy,
    ParamKind, ProgramUses,
};
use kotone_core::automation::{
    AutomationScheduler, ProgramAction, ProgramLauncher, ProgramReport, Run, RunStatus, RunTarget,
    MAX_RUNNING_PROGRAMS, MAX_SOURCE_CHARS,
};
use kotone_core::games::GameEvent;
use kotone_core::reminders::DeliveryKind;
use kotone_core::settings::Settings;
use kotone_core::tools::{WindowIdentity, WindowInfo, WindowObserver};
use kotone_platform_windows::window_tools::WindowsWindowObserver;
use kotone_script::{Cancel, Limits, Outcome, ScriptHost};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};

use crate::game_session::GameSession;
use program::DesktopHost;

/// 工具 `automation.runs` 最多返回的条数。
const MAX_LISTED_RUNS: usize = 50;

/// 运行中的程序（只在内存里；执行记录在调度器）。
struct Live {
    cancel: Cancel,
    end_reason: Option<String>,
    /// 绑了游戏窗口（[`program_running`] 只数这些：在等 `ask` 的、正在说一句的不挡她操作游戏）。
    bound: bool,
}

/// 有绑了游戏窗口的程序在跑。
fn any_bound(live: &HashMap<String, Live>) -> bool {
    live.values().any(|l| l.bound)
}

pub struct AutomationManager {
    live: Mutex<HashMap<String, Live>>,
    /// 已保存的自动化（`~/.kotone/automations.json`）。
    library: Result<Arc<AutomationLibrary>, String>,
}

impl AutomationManager {
    pub fn open() -> Self {
        let library =
            AutomationLibrary::open(kotone_core::settings::kotone_dir().join("automations.json"))
                .map(Arc::new);
        if let Err(e) = &library {
            kotone_core::log::log(&format!("automation library unavailable: {e}"));
        }
        Self {
            live: Mutex::new(HashMap::new()),
            library,
        }
    }
}

fn library(app: &AppHandle) -> Result<Arc<AutomationLibrary>, String> {
    app.state::<AutomationManager>().library.clone()
}

/// 旧用户只有在启用琴音后才安装内置程序；既有删除记录继续有效。本体自己的内置自动化，加上内置插件在清单
/// `[[automations]]` 里声明的（只有内置插件的清单有这个字段，第三方插件没有），按合并后的事件目录校验。
pub fn ensure_builtin(app: &AppHandle) {
    let from_plugins: Vec<_> = crate::plugins::registry()
        .iter()
        .flat_map(|p| p.manifest().automations.iter().cloned())
        .collect();
    let events = {
        let settings = app
            .state::<crate::SharedState>()
            .settings
            .read()
            .unwrap()
            .clone();
        crate::games::event_catalog(&settings)
    };
    if let Err(e) = library(app).and_then(|library| library.ensure_builtin(&from_plugins, &events))
    {
        kotone_core::log::log(&format!("automation: builtin not installed: {e}"));
    }
}

/// 调度器的程序执行端口。
pub struct Launcher {
    pub app: AppHandle,
}

impl ProgramLauncher for Launcher {
    fn launch(&self, run: &Run) -> Result<(), String> {
        launch(&self.app, run)
    }

    fn stop(&self, run_id: &str, reason: &str) {
        let manager = self.app.state::<AutomationManager>();
        let mut live = manager.live.lock().unwrap();
        if let Some(live) = live.get_mut(run_id) {
            live.cancel.cancel();
            live.end_reason.get_or_insert_with(|| reason.to_string());
            kotone_core::log::log(&format!("automation {run_id}: stop ({reason})"));
        }
    }
}

fn scheduler(app: &AppHandle) -> Result<Arc<AutomationScheduler>, String> {
    app.try_state::<crate::assistant::AssistantRuntime>()
        .ok_or("自动化调度器尚未就绪")?
        .scheduler
        .clone()
}

fn target_label(window: &WindowInfo) -> String {
    match &window.process_name {
        Some(process) => format!("{} ({process})", window.title),
        None => window.title.clone(),
    }
}

/// 模板图片目录（内容寻址：`<sha256>.png`）。
fn images_dir() -> std::path::PathBuf {
    kotone_core::settings::kotone_dir().join("automation-images")
}

/// 模板图片上限：边长与文件大小。
const MAX_IMAGE_SIDE: u32 = 512;
const MAX_IMAGE_BYTES: usize = 512 * 1024;

/// 把提交的 `{png: base64}` 存成内容寻址文件，换成 `{image: <sha256>}`；已是引用的原样返回。
fn store_image(value: &Value) -> Result<Value, String> {
    let Some(b64) = value.get("png").and_then(Value::as_str) else {
        return Ok(value.clone());
    };
    if value.as_object().is_some_and(|o| o.len() != 1) {
        return Err("图片只能写成 {png: base64}".into());
    }
    let bytes = kotone_tools::image::decode_base64(b64.trim())
        .map_err(|e| format!("图片不是合法的 base64：{e}"))?;
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err(format!("图片超过 {} KB", MAX_IMAGE_BYTES / 1024));
    }
    kotone_tools::image::decode_png(&bytes, MAX_IMAGE_SIDE)?;
    let id = kotone_tools::image::sha256_hex(&bytes);
    let path = images_dir().join(format!("{id}.png"));
    if !path.exists() {
        std::fs::create_dir_all(images_dir()).map_err(|e| format!("创建图片目录失败：{e}"))?;
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, &bytes).map_err(|e| format!("保存图片失败：{e}"))?;
        std::fs::rename(&tmp, &path).map_err(|e| format!("保存图片失败：{e}"))?;
    }
    Ok(json!({"image": id}))
}

/// 草稿里 image 参数的缺省值、或运行入参里的图片：`{png}` → 引用。
fn store_draft_images(draft: &mut AutomationDraft) -> Result<(), String> {
    for spec in &mut draft.params {
        if spec.kind == ParamKind::Image {
            if let Some(default) = &spec.default {
                spec.default = Some(store_image(default)?);
            }
        }
    }
    Ok(())
}

fn store_arg_images(
    params: &[kotone_core::automation::library::ParamSpec],
    args: &serde_json::Map<String, Value>,
) -> Result<serde_json::Map<String, Value>, String> {
    let mut out = args.clone();
    for spec in params.iter().filter(|p| p.kind == ParamKind::Image) {
        if let Some(value) = args.get(&spec.name) {
            out.insert(spec.name.clone(), store_image(value)?);
        }
    }
    Ok(out)
}

/// 执行记录的对外形状（工具、界面与事件）。`with_source = false` 时省略程序源码。触发它的游戏事件对外只露
/// `type` 和 `game`（ADR 045 D8：整份事件只在 `ask()` 交给她时附上）。
pub fn run_json(run: &Run, with_source: bool) -> Value {
    let mut value = serde_json::to_value(run).unwrap_or(Value::Null);
    if let Some(action) = value.get_mut("action").and_then(Value::as_object_mut) {
        if !with_source {
            action.remove("source");
        }
        if let Some(event) = action.get_mut("event") {
            *event = json!({"type": event["type"], "game": event["game"]});
        }
    }
    value
}

/// 等待中、运行中与最近的执行记录（含提醒）。
pub fn list(app: &AppHandle) -> Result<Value, String> {
    let runs = scheduler(app)?.runs();
    let running = runs
        .iter()
        .filter(|r| r.bound_to_window() && r.status == RunStatus::Running)
        .count();
    let pending = runs
        .iter()
        .filter(|r| r.status == RunStatus::Pending)
        .count();
    Ok(json!({
        "running": running,
        "pending": pending,
        "maxRunning": MAX_RUNNING_PROGRAMS,
        "runs": runs.iter().take(MAX_LISTED_RUNS).map(|r| run_json(r, true)).collect::<Vec<_>>(),
    }))
}

/// 当前前台窗口作为目标（不能是 Kotone 自己）。
pub(crate) fn foreground_target_info() -> Result<(RunTarget, WindowInfo), String> {
    let window = WindowsWindowObserver
        .foreground_window()
        .map_err(|e| format!("找不到前台窗口：{e}"))?
        .ok_or("当前没有前台窗口：请先切到游戏再启动自动化")?;
    if window.identity.pid == std::process::id() {
        return Err("前台是 Kotone 自己的窗口：请先切到游戏再启动自动化".into());
    }
    let target = RunTarget {
        handle: window.identity.handle,
        pid: window.identity.pid,
        process_started_at: window.identity.process_started_at,
        label: target_label(&window),
    };
    Ok((target, window))
}

/// 要安排的一段程序（临时写的，或来自已保存的自动化）。
pub struct ProgramRequest<'a> {
    pub name: &'a str,
    pub source: &'a str,
    pub automation_id: Option<String>,
    pub args: serde_json::Map<String, Value>,
}

fn check_source(source: &str) -> Result<&str, String> {
    let source = source.trim();
    if source.is_empty() || source.chars().count() > MAX_SOURCE_CHARS {
        return Err(format!("自动化内容为空或超过 {MAX_SOURCE_CHARS} 字"));
    }
    kotone_script::check_syntax(source).map_err(|e| format!("自动化语法错误：{e}"))?;
    Ok(source)
}

// ---------- 程序里调的工具（ADR 046 D1–D3、D9） ----------

/// 确认：工具 → 确认时它的副作用（[`Automation::confirmed`]）。
type Confirmed = BTreeMap<String, Vec<String>>;

/// 程序里调的一个工具现在怎么样。界面上“会调用：…”那一行、安排时的检查、每次 `tool()` 用同一个判断（[`ToolRules`]）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ToolState {
    /// 能调。
    Ready,
    /// 要玩家在「自动化」页点“允许”（保存的自动化）：这个工具会做什么（权限的原因）。
    Confirm(String),
    /// 插件或它要的数据开关关着：说要打开哪个。
    Off(String),
    /// 调不了：不在程序能调的范围（D2）、权限里禁止、只读模式、找不到、临时程序遇到要确认的。
    Blocked(String),
}

impl ToolState {
    /// 给界面的 `state`。
    fn word(&self) -> &'static str {
        match self {
            ToolState::Ready => "ready",
            ToolState::Confirm(_) => "confirm",
            ToolState::Off(_) => "off",
            ToolState::Blocked(_) => "blocked",
        }
    }

    /// 给界面的 `reason`（能调的是空的）。
    fn reason(&self) -> &str {
        match self {
            ToolState::Ready => "",
            ToolState::Confirm(r) | ToolState::Off(r) | ToolState::Blocked(r) => r,
        }
    }

    /// 不运行 / 调不了时交给发起方的一句（带上是哪个工具）。
    fn refusal(&self, tool: &str) -> String {
        match self {
            ToolState::Ready => String::new(),
            ToolState::Confirm(reason) => {
                format!("要先在「自动化」页点“允许”，才能调用 {tool}（{reason}）")
            }
            ToolState::Off(reason) | ToolState::Blocked(reason) if reason.contains(tool) => {
                reason.clone()
            }
            ToolState::Off(reason) | ToolState::Blocked(reason) => format!("{tool}：{reason}"),
        }
    }
}

/// 判断程序里调的工具用的那一刻的规则：玩家的权限（含整体模式）、插件和数据开关、能绑的工具目录（和热键同一份，
/// [`crate::hotkey_bind::catalog`]）。一次判断建一个（只读内存）。
pub(crate) struct ToolRules {
    settings: Settings,
    catalog: Catalog,
}

impl ToolRules {
    pub(crate) fn of(settings: Settings) -> Self {
        let catalog = crate::hotkey_bind::catalog(&settings);
        Self { settings, catalog }
    }

    pub(crate) fn now(app: &AppHandle) -> Self {
        Self::of(
            app.state::<crate::SharedState>()
                .settings
                .read()
                .unwrap()
                .clone(),
        )
    }

    /// 这个工具现在能不能调：能调交回它的条目，不能调交回为什么（[`ToolState`]，不会是 `Ready`）。
    /// `confirmed`：保存的自动化是 `Some(它的确认)`，临时程序是 None（只能调不用确认的，D3）。
    pub(crate) fn entry(
        &self,
        tool: &str,
        confirmed: Option<&Confirmed>,
    ) -> Result<&ToolEntry, ToolState> {
        let Some(entry) = self.catalog.get(tool) else {
            return Err(self.missing(tool));
        };
        program_callable(entry).map_err(ToolState::Blocked)?;
        let policy = &self.settings.agent.permissions;
        let seen: Option<Vec<&str>> = confirmed
            .and_then(|map| map.get(tool))
            .map(|effects| effects.iter().map(String::as_str).collect());
        match program_decision(policy, entry, seen.as_deref(), confirmed.is_some()) {
            BindingDecision::Run => Ok(entry),
            BindingDecision::Confirm { reason } => Err(ToolState::Confirm(reason)),
            // 临时程序遇到要确认的：program_decision 已经说清楚了（先存起来让玩家允许）
            BindingDecision::Refused { reason }
                if confirmed.is_none()
                    && matches!(
                        program_decision(policy, entry, None, true),
                        BindingDecision::Confirm { .. }
                    ) =>
            {
                Err(ToolState::Blocked(reason))
            }
            BindingDecision::Refused { reason } => {
                Err(ToolState::Blocked(format!("{reason}：去她的「权限」里改")))
            }
        }
    }

    /// 目录里没有：分清插件关着、数据开关关着、第三方插件没连上、根本没有（和热键同一套说法）。
    fn missing(&self, tool: &str) -> ToolState {
        let page_only = crate::plugins::owner(tool)
            .and_then(|p| p.tools().into_iter().find(|t| t.name == tool))
            .is_some_and(|spec| !spec.for_model());
        if page_only {
            return ToolState::Blocked(format!("{tool} 只给插件的页面用，程序里调不了"));
        }
        if let Some(reason) = crate::hotkey_bind::sources_off(&self.settings, tool) {
            return ToolState::Off(reason);
        }
        match crate::hotkey_bind::owner_of(&self.settings, tool) {
            Some((name, false)) => ToolState::Off(format!(
                "「{name}」插件关着（在左侧栏的「插件」里可以打开）"
            )),
            Some((name, true)) => ToolState::Blocked(format!(
                "「{name}」现在没有 {tool} 这个工具（第三方插件还没连上，或者更新以后去掉了）"
            )),
            None => ToolState::Blocked(format!(
                "找不到工具 {tool}（插件没装或被删了，或者名字写错了）"
            )),
        }
    }

    pub(crate) fn state(&self, tool: &str, confirmed: Option<&Confirmed>) -> ToolState {
        self.entry(tool, confirmed)
            .map_or_else(|state| state, |_| ToolState::Ready)
    }

    /// 显示名：工具说明的第一句（和「设置 → 热键」的“加一个”同一个；插件关着时照插件登记的说明），没有就用名字。
    fn title(&self, tool: &str) -> String {
        let description = self
            .catalog
            .get(tool)
            .map(|e| e.description.clone())
            .or_else(|| {
                crate::plugins::owner(tool)
                    .and_then(|p| p.tools().into_iter().find(|t| t.name == tool))
                    .map(|t| t.description)
            })
            .unwrap_or_default();
        let title = crate::hotkey_bind::title_of(&description);
        if title.is_empty() {
            tool.to_string()
        } else {
            title
        }
    }

    /// 界面上“会调用：…”那一行（D9）：程序里每个工具（按出现的先后）现在怎么样。
    pub(crate) fn lines(&self, automation: &Automation) -> Vec<ToolLine> {
        self.lines_of(automation.action.source(), &automation.confirmed)
            .unwrap_or_default()
    }

    /// 一段程序（保存的自动化的确认是 `confirmed`）里每个工具现在怎么样；工具名不是直接写的返回原因。
    fn lines_of(&self, source: &str, confirmed: &Confirmed) -> Result<Vec<ToolLine>, String> {
        Ok(library::tools_called(source)?
            .into_iter()
            .map(|name| ToolLine {
                title: self.title(&name),
                state: self.state(&name, Some(confirmed)),
                name,
            })
            .collect())
    }

    /// 安排之前查一遍（D1、D3）：工具名要直接写（临时程序也是）；等你确认的、调不了的都不运行（免得只做一半：
    /// 话说了、歌没放），原因交给发起方。
    pub(crate) fn check(&self, source: &str, confirmed: Option<&Confirmed>) -> Result<(), String> {
        let problems: Vec<String> = library::tools_called(source)?
            .iter()
            .filter_map(|tool| {
                self.entry(tool, confirmed)
                    .err()
                    .map(|state| state.refusal(tool))
            })
            .collect();
        if problems.is_empty() {
            Ok(())
        } else {
            Err(problems.join("；"))
        }
    }

    /// 玩家点“允许”时记下的（D3）：程序里现在要确认的工具 → 它这时的副作用。不用确认的、调不了的不记。
    pub(crate) fn to_confirm(&self, source: &str) -> Confirmed {
        let policy = &self.settings.agent.permissions;
        library::tools_called(source)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|tool| {
                let entry = self.catalog.get(&tool)?;
                matches!(
                    program_decision(policy, entry, None, true),
                    BindingDecision::Confirm { .. }
                )
                .then(|| (tool, entry.side_effects.clone()))
            })
            .collect()
    }
}

/// 程序里调的一个工具在界面上的一行（D9）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ToolLine {
    pub name: String,
    pub title: String,
    pub state: ToolState,
}

impl ToolLine {
    fn json(&self) -> Value {
        json!({
            "name": self.name,
            "title": self.title,
            "state": self.state.word(),
            "reason": self.state.reason(),
        })
    }
}

fn scheduled_json(run: &Run, target: Option<&str>) -> Value {
    json!({
        "status": if run.status == RunStatus::Running { "started" } else { "scheduled" },
        "id": run.id,
        "target": target,
        "dueAtUnixMs": run.due_at_ms,
    })
}

/// 安排一段程序；`due_at_ms` 不晚于现在就立即启动。程序里用到游戏窗口（[`ProgramUses`]，ADR 045 D4）时目标是
/// **现在**的前台窗口（要是允许自动化输入的游戏）；没用到的不看前台、不绑窗口（返回的 `target` 为 null）。
///
/// 临时程序（她和外部 Agent 的 `automation.run {source}`）只能调不用确认的工具（ADR 046 D3）。
pub fn start(
    app: &AppHandle,
    session: &str,
    request: ProgramRequest<'_>,
    due_at_ms: u64,
) -> Result<Value, String> {
    schedule(app, session, request, None, None, due_at_ms)
}

/// [`start`]，带上保存的自动化对这份程序的确认（`confirmed`，临时程序是 None）和触发它的游戏事件（按事件触发的执行：
/// 随执行记录保存，`ask()` 交给她时附上，ADR 045 D8）。程序里调的工具先查一遍（[`ToolRules::check`]）：等你确认的、
/// 调不了的不安排，原因交给发起方。
fn schedule(
    app: &AppHandle,
    session: &str,
    request: ProgramRequest<'_>,
    confirmed: Option<&Confirmed>,
    event: Option<GameEvent>,
    due_at_ms: u64,
) -> Result<Value, String> {
    let source = check_source(request.source)?;
    if !library::tools_called(source)?.is_empty() {
        ToolRules::now(app).check(source, confirmed)?;
    }
    let target = if ProgramUses::of(source).window {
        let (target, window) = foreground_target_info()?;
        // 按游戏配置的“允许自动化输入”（ADR 014 第 5 步）；到点执行时还会再查一次。
        crate::game_tools::input_allowed(window.process_name.as_deref())?;
        Some(target)
    } else {
        None
    };
    let label = target.as_ref().map(|t| t.label.clone());
    let run = scheduler(app)?.schedule_program(
        session,
        ProgramAction {
            name: request.name.to_string(),
            source: source.to_string(),
            automation_id: request.automation_id,
            args: request.args,
            target,
            event,
        },
        due_at_ms,
    )?;
    Ok(scheduled_json(&run, label.as_deref()))
}

/// 运行一个已保存的自动化：校验入参 → 算延迟（声明了 `delay_seconds` 的晚这么多秒开始，和 `due_at_ms` 相加，
/// 合计不超过 7 天由调度器查，ADR 045 D7）→ 按程序里写了什么决定绑不绑窗口（[`start`]）。
pub fn run_saved(
    app: &AppHandle,
    session: &str,
    id: &str,
    args: &serde_json::Map<String, Value>,
    due_at_ms: u64,
) -> Result<Value, String> {
    run_saved_on(app, session, id, args, None, due_at_ms)
}

fn run_saved_on(
    app: &AppHandle,
    session: &str,
    id: &str,
    args: &serde_json::Map<String, Value>,
    event: Option<GameEvent>,
    due_at_ms: u64,
) -> Result<Value, String> {
    let automation = library(app)?
        .get(id)
        .ok_or_else(|| format!("找不到自动化 {id}"))?;
    let args = validate_args(
        &automation.params,
        &store_arg_images(&automation.params, args)?,
    )?;
    let due = due_at_ms.saturating_add(library::delay_ms(&automation.params, &args));
    schedule(
        app,
        session,
        ProgramRequest {
            name: &automation.name,
            source: automation.action.source(),
            automation_id: Some(automation.id.clone()),
            args,
        },
        Some(&automation.confirmed),
        event,
        due,
    )
}

/// ▶ 要不要先倒数（让玩家切到游戏）：只有用到游戏窗口的才要（ADR 045 D9）。
fn needs_countdown(automation: &Automation) -> bool {
    automation.action.uses().window
}

/// 游戏事件触发时 `mode`（single / restart / queued）管不管这个自动化：只管用到游戏窗口的；不用窗口的每次触发都跑
/// （和以前的播报一样，ADR 045 D8）。
fn mode_applies(automation: &Automation) -> bool {
    automation.action.uses().window
}

/// 这个自动化还没结束的程序执行。
fn open_program_runs(app: &AppHandle, id: &str) -> Vec<String> {
    let Ok(scheduler) = scheduler(app) else {
        return Vec::new();
    };
    scheduler
        .runs()
        .into_iter()
        .filter(|r| r.status.is_open())
        .filter(|r| {
            matches!(&r.action, kotone_core::automation::RunAction::Program(p)
                if p.automation_id.as_deref() == Some(id))
        })
        .map(|r| r.id)
        .collect()
}

/// 游戏事件触发（ADR 036 D6、ADR 045 D8）：和事件 `data` 字段同名的参数自动填上（事件是函数的输入，
/// [`library::event_args`]）；事件随执行记录保存（程序里的 `ask()` 连同事件一起交给她）。用到游戏窗口的程序还在跑时
/// 按 `mode` 处理；不用窗口的每次触发都跑。
pub fn run_on_event(app: &AppHandle, fire: &kotone_core::games::Fire) {
    use kotone_core::games::RunMode;
    let Some(automation) = library(app).ok().and_then(|l| l.get(&fire.automation)) else {
        return;
    };
    let id = automation.id.clone();
    kotone_core::log::log(&format!(
        "automation {id}: 游戏事件 {} 触发（{}）",
        fire.event.kind, fire.event.game
    ));
    let args = library::event_args(&automation.params, &fire.event);
    let event = fire.event.clone();
    let mode = automation.on.as_ref().map(|t| t.mode).unwrap_or_default();
    let running = if mode_applies(&automation) {
        open_program_runs(app, &id)
    } else {
        Vec::new()
    };
    if !running.is_empty() {
        match mode {
            RunMode::Single => {
                kotone_core::log::log(&format!("automation {id}: 还在跑，这次不管（single）"));
                return;
            }
            RunMode::Restart => {
                if let Ok(scheduler) = scheduler(app) {
                    for run in &running {
                        let _ = scheduler.cancel_with_reason(run, "游戏事件重新触发");
                    }
                }
            }
            RunMode::Queued => {
                // 跑完一个再跑下一个（最多等 30 分钟，和单次程序的上限一样）
                let app = app.clone();
                let _ = std::thread::Builder::new()
                    .name("kotone-automation-queued".into())
                    .spawn(move || {
                        let deadline = std::time::Instant::now() + Duration::from_secs(1800);
                        while !open_program_runs(&app, &id).is_empty() {
                            if std::time::Instant::now() >= deadline {
                                return;
                            }
                            std::thread::sleep(Duration::from_secs(1));
                        }
                        run_event_now(&app, &id, &args, event);
                    });
                return;
            }
        }
    }
    run_event_now(app, &id, &args, event);
}

fn run_event_now(
    app: &AppHandle,
    id: &str,
    args: &serde_json::Map<String, Value>,
    event: GameEvent,
) {
    let now = kotone_core::automation::now_ms();
    if let Err(e) = run_saved_on(app, "event", id, args, Some(event), now) {
        kotone_core::log::log(&format!("automation {id}: 游戏事件触发的运行失败：{e}"));
    }
}

/// 定时运行的自动化（类似 crontab）：每 20 秒看一次上次检查以来有没有到点的；
/// Kotone 没开时错过的不补（与调度器“错过不补”一致）。
pub fn start_schedules(app: &AppHandle) {
    let app = app.clone();
    let _ = std::thread::Builder::new()
        .name("kotone-automation-schedule".into())
        .spawn(move || {
            let mut last = kotone_core::automation::now_ms();
            loop {
                std::thread::sleep(Duration::from_secs(20));
                let now = kotone_core::automation::now_ms();
                if app
                    .state::<crate::SharedState>()
                    .settings
                    .read()
                    .unwrap()
                    .ui
                    .legacy_upgrade
                    .voice_input_only()
                {
                    last = now;
                    continue;
                }
                for automation in saved_list(&app) {
                    let Some(schedule) = automation.schedule.as_ref().filter(|s| s.enabled) else {
                        continue;
                    };
                    let Ok(cron) = kotone_core::automation::cron::Cron::parse(&schedule.cron)
                    else {
                        continue;
                    };
                    if !kotone_core::automation::cron::due_between(&cron, last, now) {
                        continue;
                    }
                    kotone_core::log::log(&format!(
                        "automation {}: scheduled run ({})",
                        automation.id, schedule.cron
                    ));
                    if let Err(e) = run_saved(&app, "schedule", &automation.id, &schedule.args, now)
                    {
                        kotone_core::log::log(&format!(
                            "automation {}: scheduled run failed: {e}",
                            automation.id
                        ));
                    }
                }
                last = now;
            }
        });
}

/// 她的提醒工具：“晚一会儿开始、只说一句”的程序（core 的 `schedule_reminder`，ADR 045 D3），不绑游戏窗口：
/// Kotone 自己在前台时也能安排，到点时游戏关了照样提醒。`delivery` 为空时用助手的默认方式。
pub fn schedule_reminder(
    app: &AppHandle,
    session: &str,
    text: String,
    delivery: Vec<DeliveryKind>,
    delay_seconds: u64,
) -> Result<Value, String> {
    let delivery = if delivery.is_empty() {
        app.state::<crate::SharedState>()
            .settings
            .read()
            .unwrap()
            .assistant
            .default_delivery
            .clone()
    } else {
        delivery
    };
    let due = kotone_core::automation::now_ms() + delay_seconds.saturating_mul(1_000);
    let run = scheduler(app)?.schedule_reminder(session, &text, &delivery, due)?;
    Ok(scheduled_json(&run, None))
}

/// 已保存的自动化（按名字排序；库不可用时为空）。
pub fn saved_list(app: &AppHandle) -> Vec<Automation> {
    library(app).map(|l| l.list()).unwrap_or_default()
}

/// 定义的对外形状；`with_source = false`（列表）时省略程序源码。后端按源码算好的 `usesWindow`（用到游戏窗口：
/// ▶ 先倒数、要切到游戏）和 `asks`（会叫她，每次花一次 AI 调用）一起给（ADR 045 D9）。确认的原文（`confirmed`）不往外给
/// （ADR 046：外部 Agent 原样抄回 `automation.save` 会被拒），每个工具现在怎么样见 [`describe`] 加的 `tools[]`。
pub fn automation_json(automation: &Automation, with_source: bool) -> Value {
    let uses = automation.action.uses();
    let mut value = serde_json::to_value(automation).unwrap_or(Value::Null);
    if let Some(object) = value.as_object_mut() {
        if !with_source {
            if let Some(action) = object.get_mut("action").and_then(Value::as_object_mut) {
                action.remove("source");
            }
        }
        object.remove("confirmed");
        object.insert("usesWindow".into(), json!(uses.window));
        object.insert("asks".into(), json!(uses.asks));
    }
    value
}

/// [`automation_json`] 加上程序里调的工具（ADR 046 D9）：`tools: [{name, title, state, reason}]`（`state` 是
/// `ready` / `confirm` / `off` / `blocked`，按程序里出现的先后），`needsConfirm`（有要玩家在「自动化」页点“允许”的）。
/// 界面、工具 `automation.get` / `list` / `save` 都是这个形状。
fn describe(automation: &Automation, with_source: bool, rules: &ToolRules) -> Value {
    let mut value = automation_json(automation, with_source);
    if let Some(object) = value.as_object_mut() {
        add_tools(object, &rules.lines(automation));
    }
    value
}

/// `tools[]` 和 `needsConfirm`（[`describe`]、编辑器的 [`automation_tools`] 同一个形状）。
fn add_tools(object: &mut serde_json::Map<String, Value>, lines: &[ToolLine]) {
    object.insert(
        "tools".into(),
        json!(lines.iter().map(ToolLine::json).collect::<Vec<_>>()),
    );
    object.insert(
        "needsConfirm".into(),
        json!(lines
            .iter()
            .any(|l| matches!(l.state, ToolState::Confirm(_)))),
    );
}

/// 编辑器里“会调用：…”那一行（ADR 046 D9）：照编辑器里现在这份程序（还没存）算，`{tools, needsConfirm, error}`，存的
/// 按钮照 `needsConfirm` 写“保存”还是“允许并保存”。`confirmed` 是正在改的那个自动化存着的确认：程序和存着的一样才算数
/// （存的时候程序一变，确认就作废）。工具名不是直接写的：`tools` 为空，`error` 说清楚（这样存不进来）。
fn preview(source: &str, stored: Option<&Automation>, rules: &ToolRules) -> Value {
    let confirmed = stored
        .filter(|a| a.action.source().trim() == source.trim())
        .map(|a| a.confirmed.clone())
        .unwrap_or_default();
    let (lines, error) = match rules.lines_of(source, &confirmed) {
        Ok(lines) => (lines, None),
        Err(error) => (Vec::new(), Some(error)),
    };
    let mut object = serde_json::Map::new();
    add_tools(&mut object, &lines);
    object.insert("error".into(), json!(error));
    Value::Object(object)
}

/// 保存（新建或按 id 覆盖）；程序先检查语法。游戏事件触发按合并后的事件目录校验（插件登记的也认得）；
/// 触发里的事现在不会发生时（例如英雄联盟插件关着）照常保存，结果里多一项 `triggerNotes` 说清楚（原则 21）；
/// 开了一局一次或冷却、程序又读了事件填的参数时也在 `triggerNotes` 里提醒一句（[`library::trigger_hints`]，不拦）。
/// 程序里调了要确认的工具时，结果的 `needsConfirm` 是 true（她存的：告诉玩家去「自动化」页点“允许”，ADR 046 D9）。
pub fn save(app: &AppHandle, draft: AutomationDraft, by: CreatedBy) -> Result<Value, String> {
    let (saved, notes) = store(app, draft, by)?;
    Ok(saved_json(&saved, &notes, &ToolRules::now(app)))
}

/// 保存结果的形状：[`describe`]（带源码），有关于触发的提醒时加 `triggerNotes`（现在不会发生的事、条件写在程序里
/// 会白白用掉次数和冷却）。
fn saved_json(saved: &Automation, notes: &[String], rules: &ToolRules) -> Value {
    let mut value = describe(saved, true, rules);
    if let (false, Some(object)) = (notes.is_empty(), value.as_object_mut()) {
        object.insert("triggerNotes".into(), json!(notes));
    }
    value
}

/// 存下草稿（[`save`] 和「自动化」页的保存共用）：返回存好的定义和关于触发的提醒（[`saved_json`] 的 `triggerNotes`）。
fn store(
    app: &AppHandle,
    mut draft: AutomationDraft,
    by: CreatedBy,
) -> Result<(Automation, Vec<String>), String> {
    let AutomationBody::Program(program) = &mut draft.action;
    program.source = check_source(&program.source)?.to_string();
    store_draft_images(&mut draft)?;
    let events = {
        let settings = app
            .state::<crate::SharedState>()
            .settings
            .read()
            .unwrap()
            .clone();
        crate::games::event_catalog(&settings)
    };
    let saved = library(app)?.save(draft, by, &events)?;
    let _ = app.emit("kotone://automations-updated", ());
    crate::tool_service::publish(
        app,
        "automation.saved",
        json!({"id": saved.id, "name": saved.name}),
    );
    let mut notes = saved
        .on
        .as_ref()
        .map(|on| events.notes(on))
        .unwrap_or_default();
    notes.extend(library::trigger_hints(&saved, &events));
    Ok((saved, notes))
}

pub fn get(app: &AppHandle, id: &str) -> Result<Value, String> {
    let rules = ToolRules::now(app);
    library(app)?
        .get(id)
        .map(|a| describe(&a, true, &rules))
        .ok_or_else(|| format!("找不到自动化 {id}"))
}

pub fn list_saved(app: &AppHandle) -> Result<Value, String> {
    let list = library(app)?.list();
    let now = kotone_core::automation::now_ms();
    let rules = ToolRules::now(app);
    Ok(json!({
        "automations": list.iter().map(|a| {
            let mut value = describe(a, false, &rules);
            // 定时的下一次（界面显示“下次 08:00”）
            let next = a.schedule.as_ref().filter(|s| s.enabled)
                .and_then(|s| kotone_core::automation::cron::Cron::parse(&s.cron).ok())
                .and_then(|cron| kotone_core::automation::cron::next_run_ms(&cron, now));
            if let (Some(next), Some(object)) = (next, value.as_object_mut()) {
                object.insert("nextRunAtMs".into(), json!(next));
            }
            value
        }).collect::<Vec<_>>(),
        "max": kotone_core::automation::library::MAX_AUTOMATIONS,
    }))
}

pub fn delete(app: &AppHandle, id: &str) -> Result<bool, String> {
    let deleted = library(app)?.delete(id)?;
    if deleted {
        let _ = app.emit("kotone://automations-updated", ());
        crate::tool_service::publish(app, "automation.deleted", json!({"id": id}));
    }
    Ok(deleted)
}

/// 记下这段程序可以调现在要确认的工具（[`ToolRules::to_confirm`]；核对的是 `automation` 这份程序，期间被改了就不认）。
/// 没有要确认的什么都不写。
fn confirm_tools(
    app: &AppHandle,
    automation: &Automation,
    rules: &ToolRules,
) -> Result<Automation, String> {
    let map = rules.to_confirm(automation.action.source());
    if map.is_empty() {
        return Ok(automation.clone());
    }
    let confirmed = library(app)?.confirm(&automation.id, automation.action.source(), map)?;
    let _ = app.emit("kotone://automations-updated", ());
    kotone_core::log::log(&format!(
        "automation {}: 玩家允许了 {}",
        confirmed.id,
        confirmed
            .confirmed
            .keys()
            .cloned()
            .collect::<Vec<_>>()
            .join("、")
    ));
    Ok(confirmed)
}

/// 第三方插件的配置变了、删掉了（`remote_plugins::forget_rules`）：自动化上对它的工具的确认也作废（ADR 046 D3，
/// 和热键一样）。`prefix` 带点（`demo.`）。
pub fn forget_confirmations(app: &AppHandle, prefix: &str) {
    let Some(manager) = app.try_state::<AutomationManager>() else {
        return;
    };
    let Ok(library) = &manager.library else {
        return;
    };
    match library.forget_confirmations(prefix) {
        Ok(true) => {
            kotone_core::log::log(&format!("automation: 清掉了对 {prefix}* 的确认"));
            let _ = app.emit("kotone://automations-updated", ());
        }
        Ok(false) => {}
        Err(error) => {
            kotone_core::log::log(&format!("automation: 对 {prefix}* 的确认没清掉：{error}"))
        }
    }
}

/// 有绑了游戏窗口的自动化程序正在运行（`game.act` 据此不与它同时按键，ADR 019“输入锁”）。不用窗口的（正在说一句、
/// 在等 `ask` 交给她的）不算：它们不按键，不该挡她操作游戏（ADR 045 D4）。
pub fn program_running(app: &AppHandle) -> bool {
    app.try_state::<AutomationManager>()
        .is_some_and(|m| any_bound(&m.live.lock().unwrap()))
}

/// 绑了窗口的执行到点时：她没在操作游戏、窗口还在、这个游戏允许自动化输入。返回窗口会话。
fn bind_window(app: &AppHandle, target: &RunTarget) -> Result<GameSession, String> {
    if crate::computer_use::busy(app) {
        return Err("正在替你操作游戏，自动化没有启动（不能同时按键）".into());
    }
    let identity = WindowIdentity {
        handle: target.handle,
        pid: target.pid,
        process_started_at: target.process_started_at,
    };
    let window = WindowsWindowObserver
        .inspect(&identity)
        .map_err(|_| format!("游戏窗口已关闭（{}），自动化没有启动", target.label))?;
    crate::game_tools::input_allowed(window.process_name.as_deref())?;
    Ok(GameSession::new(identity))
}

/// 调度器到点（或立即）调用：绑了窗口的先确认窗口还在（[`bind_window`]），不绑的直接开始；然后在新线程里运行程序。
fn launch(app: &AppHandle, run: &Run) -> Result<(), String> {
    let program = run.program();
    let session = match &program.target {
        Some(target) => Some(bind_window(app, target)?),
        None => None,
    };
    let id = run.id.clone();
    let cancel = Cancel::default();
    {
        let manager = app.state::<AutomationManager>();
        manager.live.lock().unwrap().insert(
            id.clone(),
            Live {
                cancel: cancel.clone(),
                end_reason: None,
                bound: session.is_some(),
            },
        );
    }
    let host = Arc::new(DesktopHost::new(app.clone(), session, cancel.clone(), run));
    let thread_app = app.clone();
    let thread_id = id.clone();
    let code = program.source.clone();
    let args = program.args.clone();
    let spawned = std::thread::Builder::new()
        .name(format!("kotone-automation-{id}"))
        .spawn(move || {
            let report = kotone_script::run_with_args(
                &code,
                &args,
                host.clone() as Arc<dyn ScriptHost>,
                Limits::default(),
                cancel,
            );
            host.release_all();
            kotone_core::log::log(&format!(
                "automation {thread_id}: {:?} keys={} {}ms",
                report.outcome, report.keys, report.duration_ms
            ));
            // 读停止原因时只短暂持有自己的锁，回报调度器时不持有（ProgramLauncher 的约定）；
            // 记录写完才从 live 移除，应用退出时据此等待“键已松开、记录已写”。
            let end_reason = thread_app
                .state::<AutomationManager>()
                .live
                .lock()
                .unwrap()
                .get(&thread_id)
                .and_then(|live| live.end_reason.clone());
            let gone = host.window_gone();
            let (status, message, outcome) = match &report.outcome {
                Outcome::Finished => (RunStatus::Finished, None, "finished"),
                Outcome::Cancelled => (
                    RunStatus::Cancelled,
                    end_reason.or_else(|| gone.then(|| "游戏窗口已关闭".to_string())),
                    "stopped",
                ),
                Outcome::LimitReached(m) => (RunStatus::Failed, Some(m.clone()), "limit"),
                Outcome::Error(m) => (RunStatus::Failed, Some(m.clone()), "error"),
            };
            let summary = ProgramReport {
                outcome: outcome.into(),
                keys: u32::try_from(report.keys).unwrap_or(u32::MAX),
                duration_ms: report.duration_ms,
                log: report.log,
            };
            if let Ok(scheduler) = scheduler(&thread_app) {
                if let Err(e) = scheduler.finish_program(&thread_id, status, message, Some(summary))
                {
                    kotone_core::log::log(&format!("automation {thread_id}: record failed: {e}"));
                }
            }
            thread_app
                .state::<AutomationManager>()
                .live
                .lock()
                .unwrap()
                .remove(&thread_id);
        });
    if let Err(e) = spawned {
        app.state::<AutomationManager>()
            .live
            .lock()
            .unwrap()
            .remove(&id);
        return Err(format!("无法启动自动化线程：{e}"));
    }
    Ok(())
}

/// 取消 / 停止一条执行（`id`，提醒也可以）；`id = None` 是停下（ADR 045 D6）：运行中的全停，等待中的只取消
/// 绑了游戏窗口的（等待中的提醒留着，单条在「自动化」页取消）。返回涉及的条数。
pub fn stop(app: &AppHandle, id: Option<&str>, reason: &str) -> usize {
    let Ok(scheduler) = scheduler(app) else {
        return 0;
    };
    let result = match id {
        Some(id) => scheduler.cancel_with_reason(id, reason).map(usize::from),
        None => scheduler.stop_programs(reason),
    };
    match result {
        Ok(n) => {
            if n > 0 {
                kotone_core::log::log(&format!("automation: stopped {n} ({reason})"));
            }
            n
        }
        Err(e) => {
            kotone_core::log::log(&format!("automation: stop failed: {e}"));
            0
        }
    }
}

/// 接到调度器上：程序执行端口与结束事件。
pub fn attach(app: &AppHandle) {
    let Ok(scheduler) = scheduler(app) else {
        return;
    };
    let handle = app.clone();
    scheduler.set_on_finished(Arc::new(move |run: &Run| {
        crate::tool_service::publish(&handle, "automation.finished", run_json(run, false));
        crate::agent::on_run_finished(&handle, run);
    }));
    scheduler.set_launcher(Arc::new(Launcher { app: app.clone() }));
}

/// 应用退出等程序线程收尾（松开按住的键、写结束记录）的最长时间。
const SHUTDOWN_WAIT: Duration = Duration::from_millis(1_500);

/// 应用退出：只停止运行中的程序，等待中的留给下次启动（过期则记为错过）。
/// 进程在本函数返回后就会结束，所以要等各线程松开按住的键、写完记录（有上限）。
pub fn shutdown(app: &AppHandle) {
    let Some(manager) = app.try_state::<AutomationManager>() else {
        return;
    };
    let count = {
        let mut live = manager.live.lock().unwrap();
        for run in live.values_mut() {
            run.cancel.cancel();
            run.end_reason
                .get_or_insert_with(|| "Kotone 退出".to_string());
        }
        live.len()
    };
    if count == 0 {
        return;
    }
    let deadline = Instant::now() + SHUTDOWN_WAIT;
    while !manager.live.lock().unwrap().is_empty() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    let left = manager.live.lock().unwrap().len();
    kotone_core::log::log(&format!(
        "automation: stopped {count} at exit, {left} did not finish in time"
    ));
}

#[tauri::command]
pub fn automation_runs(app: AppHandle) -> Result<Value, String> {
    list(&app)
}

/// 设置页的列表：和工具 `automation.list` 一样，另加新建时选的模板（说一句 / 推送一条 / 让她做 / 空白程序，
/// ADR 045 D9；界面按 id 取文案）。
#[tauri::command]
pub fn automation_saved_list(app: AppHandle) -> Result<Value, String> {
    let mut value = list_saved(&app)?;
    value["templates"] = json!(library::templates());
    Ok(value)
}

#[tauri::command]
pub fn automation_get(app: AppHandle, id: String) -> Result<Value, String> {
    get(&app, &id)
}

/// 「自动化」页保存（创建者记为用户），返回存好的定义（[`describe`] 的形状，带 `tools[]`、`needsConfirm`）。
/// `confirm` = 玩家点了“允许并保存”（ADR 046 D3、D9）：存好以后照刚存的这份程序记下要确认的工具（[`confirm_tools`]）；
/// 不传当 false。改了程序的，存的时候就要再点一次“允许并保存”（存的时候确认已经作废了）。
#[tauri::command]
pub fn automation_save(
    app: AppHandle,
    draft: AutomationDraft,
    confirm: Option<bool>,
) -> Result<Value, String> {
    let (saved, notes) = store(&app, draft, CreatedBy::User)?;
    let rules = ToolRules::now(&app);
    let saved = if confirm.unwrap_or(false) {
        confirm_tools(&app, &saved, &rules)
            .map_err(|error| format!("已经存了，但没允许成：{error}"))?
    } else {
        saved
    };
    Ok(saved_json(&saved, &notes, &rules))
}

/// 「自动化」页列表上点“允许”（ADR 046 D3）：`source` 是玩家看到的那份程序，和现在存的不一样（这期间被她、外部 Agent
/// 或另一个窗口改了）就不认。返回确认后的定义（带源码）。
///
/// **确认只有这条路和“允许并保存”（[`automation_save`] 带 `confirm`）能写**：和热键的 `confirm_hotkey` 一样是 Kotone
/// 窗口的命令，不进工具服务（她和外部 Agent 调不到）；插件的页面在回环代理的另一个源里，没有 Kotone 的命令可调；
/// `automation.save` 的草稿带 `confirmed` 直接拒。
#[tauri::command]
pub fn automation_confirm(app: AppHandle, id: String, source: String) -> Result<Value, String> {
    let automation = library(&app)?
        .get(&id)
        .ok_or_else(|| format!("找不到自动化 {id}"))?;
    if automation.action.source().trim() != source.trim() {
        return Err("程序刚被改过，请重新看一遍".into());
    }
    let rules = ToolRules::now(&app);
    let confirmed = confirm_tools(&app, &automation, &rules)?;
    Ok(describe(&confirmed, true, &rules))
}

/// 编辑器里程序下面那一行（[`preview`]）：`source` 是编辑器里现在的程序，`id` 是正在改的那个自动化（新建的不传）。
#[tauri::command]
pub fn automation_tools(app: AppHandle, source: String, id: Option<String>) -> Value {
    let stored = id.and_then(|id| library(&app).ok()?.get(&id));
    preview(&source, stored.as_ref(), &ToolRules::now(&app))
}

#[tauri::command]
pub fn automation_delete(app: AppHandle, id: String) -> Result<bool, String> {
    delete(&app, &id)
}

/// 设置页“运行”（▶）：点按钮时前台是 Kotone 自己，所以用到游戏窗口的先倒数 `switch_seconds` 秒
/// （1–30，让用户切到游戏），再以那时的前台窗口为目标运行（返回 `{status: "countdown", seconds}`）；入参先校验，
/// 错了立即返回。倒数后的失败（例如前台仍是 Kotone）用语音 / 弹幕告诉用户。
/// 不用游戏窗口的直接排上（ADR 045 D9），返回 `{status: "started" | "scheduled", id, target: null, dueAtUnixMs}`。
#[tauri::command]
pub fn automation_run_saved(
    app: AppHandle,
    id: String,
    args: serde_json::Map<String, Value>,
    switch_seconds: u64,
) -> Result<Value, String> {
    let automation = library(&app)?
        .get(&id)
        .ok_or_else(|| format!("找不到自动化 {id}"))?;
    let stored = store_arg_images(&automation.params, &args)?;
    validate_args(&automation.params, &stored)?;
    if !needs_countdown(&automation) {
        return run_saved(&app, "ui", &id, &stored, kotone_core::automation::now_ms());
    }
    // 等你确认的、调不了的不用倒数就说（ADR 046 D3）；倒数完安排时还会再查一次
    ToolRules::now(&app).check(automation.action.source(), Some(&automation.confirmed))?;
    let seconds = switch_seconds.clamp(1, 30);
    std::thread::Builder::new()
        .name("kotone-automation-countdown".into())
        .spawn(move || {
            std::thread::sleep(Duration::from_secs(seconds));
            if let Err(error) =
                run_saved(&app, "ui", &id, &stored, kotone_core::automation::now_ms())
            {
                kotone_core::log::log(&format!("automation {id}: ui run failed: {error}"));
                let app2 = app.clone();
                tauri::async_runtime::spawn(async move {
                    let _ = crate::assistant::deliver_text(
                        &app2,
                        format!("自动化没能运行：{error}"),
                        Vec::new(),
                    )
                    .await;
                });
            }
        })
        .map_err(|e| format!("无法开始倒数：{e}"))?;
    Ok(json!({"status": "countdown", "seconds": seconds}))
}

#[tauri::command]
pub fn automation_stop(app: AppHandle, id: Option<String>) -> usize {
    stop(&app, id.as_deref(), "用户停止")
}

#[cfg(test)]
mod tests {
    use super::*;
    use kotone_core::automation::library::{BUILTIN_LOL_SCOUT_ID, BUILTIN_REMIND_ID};

    fn builtin(id: &str) -> Automation {
        AutomationLibrary::builtin_programs_preview()
            .into_iter()
            .find(|a| a.id == id)
            .unwrap()
    }

    /// 执行记录对外只露触发事件的 `type` 和 `game`（ADR 045 D8）：事件里别人起的名字只在 `ask()` 交给她时附上；
    /// 不绑窗口的 `target` 是 null。
    #[test]
    fn run_json_shows_only_the_type_and_game_of_the_event() {
        let run: Run = serde_json::from_value(json!({
            "id": "run-7", "sessionId": "event", "createdAtMs": 1, "dueAtMs": 1, "status": "running",
            "action": {
                "kind": "program", "name": "拿龙", "source": "say(\"拿龙了\")",
                "automationId": "a-1", "args": {"killer": "某人"},
                "event": {"id": "e-1", "type": "lol.dragon", "game": "lol", "match": "m-1", "time": 5,
                          "source": "lol/live-client", "data": {"killer": "某人"}},
            },
        }))
        .unwrap();
        let value = run_json(&run, false);
        let action = &value["action"];
        assert_eq!(
            action["event"],
            json!({"type": "lol.dragon", "game": "lol"})
        );
        assert!(action.get("source").is_none());
        assert_eq!(action["target"], Value::Null);
        assert_eq!(action["automationId"], "a-1");
        assert!(run_json(&run, true)["action"]["source"].is_string());
        // 没有事件的（手动、热键、定时、提醒）不多出一项
        let mut manual = run.clone();
        let kotone_core::automation::RunAction::Program(program) = &mut manual.action;
        program.event = None;
        assert!(run_json(&manual, true)["action"].get("event").is_none());
    }

    /// 界面要的 `usesWindow` / `asks` 由后端按源码算好（ADR 045 D9）；模板是 4 个，界面按 id 取文案。
    #[test]
    fn automations_say_whether_they_use_the_game_window_and_ask_her() {
        let remind = automation_json(&builtin(BUILTIN_REMIND_ID), false);
        assert_eq!(
            (remind["usesWindow"].clone(), remind["asks"].clone()),
            (json!(false), json!(false))
        );
        assert!(remind["action"].get("source").is_none(), "列表不带源码");
        let scout = automation_json(&builtin(BUILTIN_LOL_SCOUT_ID), true);
        assert_eq!(
            (scout["usesWindow"].clone(), scout["asks"].clone()),
            (json!(false), json!(true))
        );
        assert!(scout["action"]["source"]
            .as_str()
            .unwrap()
            .starts_with("ask("));
        let keys = automation_json(&builtin("builtin-key-repeat"), false);
        assert_eq!(
            (keys["usesWindow"].clone(), keys["asks"].clone()),
            (json!(true), json!(false))
        );

        let templates = json!(library::templates());
        let ids: Vec<&str> = templates
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["say", "push", "ask", "blank", "onedragon-push"]);
        for t in templates.as_array().unwrap() {
            for key in ["name", "description", "params", "source"] {
                assert!(t.get(key).is_some(), "{key}");
            }
        }
    }

    /// 测试清单 22、23：▶ 只对用到游戏窗口的先倒数；游戏事件触发的 `mode` 只管用到游戏窗口的，不用窗口的
    /// （提醒、只说话的、叫她的）每次触发都跑。
    #[test]
    fn only_programs_that_use_the_game_window_count_down_and_follow_the_mode() {
        for id in kotone_core::automation::library::BUILTIN_PROGRAM_IDS {
            let a = builtin(id);
            assert!(needs_countdown(&a) && mode_applies(&a), "{id}");
        }
        for id in [BUILTIN_REMIND_ID, BUILTIN_LOL_SCOUT_ID] {
            let a = builtin(id);
            assert!(!needs_countdown(&a) && !mode_applies(&a), "{id}");
        }
    }

    /// 测试清单 17：正在跑的提醒、在等 `ask` 的程序不挡她操作游戏（`game.act`）；绑了窗口的才挡。
    #[test]
    fn only_programs_bound_to_the_game_window_block_her_from_acting() {
        let live = |bound: bool| Live {
            cancel: Cancel::default(),
            end_reason: None,
            bound,
        };
        let mut running = HashMap::new();
        assert!(!any_bound(&running));
        running.insert("run-1".to_string(), live(false));
        running.insert("run-2".to_string(), live(false));
        assert!(!any_bound(&running), "提醒、在等 ask 的不挡");
        running.insert("run-3".to_string(), live(true));
        assert!(any_bound(&running));
    }

    // ---------- ADR 046：程序里调工具 ----------

    /// 存进一个临时的库（事件目录照默认设置合并）。本体的代码里不写插件登记的事件名（kotone-plugin-api 的
    /// tests/event_boundaries.rs），要事件触发的测试用假想的 demo 游戏：目录里多登记一种 `demo.death`。
    pub(super) fn saved(draft: Value) -> (tempfile::TempDir, AutomationLibrary, Automation) {
        let dir = tempfile::tempdir().unwrap();
        let library = AutomationLibrary::open(dir.path().join("automations.json")).unwrap();
        let mut events = crate::games::event_catalog(&Settings::default());
        events
            .add(kotone_core::games::CatalogEntry {
                spec: kotone_core::games::EventSpec {
                    kind: "demo.death".into(),
                    label: "有英雄阵亡".into(),
                    data: BTreeMap::new(),
                    needs: Vec::new(),
                    private: Vec::new(),
                },
                adapter: "demo".into(),
                plugin: Some(("demo".into(), "示例".into())),
                games: vec!["demo".into()],
                off: None,
            })
            .unwrap();
        let draft: AutomationDraft = serde_json::from_value(draft).unwrap();
        let automation = library.save(draft, CreatedBy::Companion, &events).unwrap();
        (dir, library, automation)
    }

    fn program(source: &str) -> Value {
        json!({"name": "调工具", "action": {"kind": "program", "source": source}})
    }

    fn states(rules: &ToolRules, automation: &Automation) -> Vec<(String, &'static str)> {
        rules
            .lines(automation)
            .into_iter()
            .map(|l| (l.name, l.state.word()))
            .collect()
    }

    /// 测试清单 10、13、D9：界面上“会调用：…”每个工具现在怎么样——放歌默认能调；改设置要确认（`needsConfirm`）；
    /// 结果不可信又只读的、往游戏里打字的、找不到的调不了。对外的形状不带确认的原文。
    #[test]
    fn each_tool_a_program_calls_shows_whether_it_is_ready() {
        let (_dir, library, automation) = saved(program(
            "tool('music', { action = 'play' }) tool('settings.update', { patch = {} })\n\
             tool('lol.scout') tool('game_chat.send', { text = 'gg' }) tool('nobody.thing')\n\
             tool('ocr.recognize') tool('companion.read_chat')",
        ));
        let rules = ToolRules::of(Settings::default());
        assert_eq!(
            states(&rules, &automation),
            [
                ("music".to_string(), "ready"),
                ("settings.update".to_string(), "confirm"),
                ("lol.scout".to_string(), "blocked"),
                ("game_chat.send".to_string(), "blocked"),
                ("nobody.thing".to_string(), "blocked"),
                ("ocr.recognize".to_string(), "blocked"),
                ("companion.read_chat".to_string(), "blocked"),
            ]
        );
        let value = describe(&automation, true, &rules);
        assert_eq!(value["needsConfirm"], true);
        let tools = value["tools"].as_array().unwrap();
        assert!(tools[0]["title"].as_str().unwrap().starts_with("放歌"));
        assert_eq!(tools[0]["reason"], "");
        assert_eq!(tools[1]["reason"], "会修改设置");
        assert!(tools[2]["reason"].as_str().unwrap().contains("别人写的字"));
        assert!(tools[3]["reason"].as_str().unwrap().contains("chat()"));
        assert!(tools[4]["reason"].as_str().unwrap().contains("找不到"));
        assert_eq!(tools[4]["title"], "nobody.thing", "没有显示名就用名字");
        for key in ["name", "title", "state", "reason"] {
            assert!(tools[0].get(key).is_some(), "{key}");
        }

        // 玩家点“允许”：只记要确认的那个，记的是它这时的副作用；之后就能调了，对外也看不到 confirmed
        let map = rules.to_confirm(automation.action.source());
        assert_eq!(
            map.keys().collect::<Vec<_>>(),
            ["settings.update"],
            "不用确认的、调不了的不记"
        );
        let confirmed = library
            .confirm(&automation.id, automation.action.source(), map)
            .unwrap();
        assert_eq!(states(&rules, &confirmed)[1].1, "ready");
        let value = describe(&confirmed, false, &rules);
        assert_eq!(value["needsConfirm"], false);
        assert!(value.get("confirmed").is_none(), "确认的原文不往外给");
        assert!(value["action"].get("source").is_none());
        assert!(automation_json(&confirmed, true).get("confirmed").is_none());
    }

    /// D3：插件关着、数据开关关着是 `off`（说要打开哪个）；只读模式、权限里禁止是 `blocked`（说去「权限」里改）。
    #[test]
    fn switched_off_plugins_and_permissions_say_what_to_change() {
        let (_dir, _library, automation) = saved(program(
            "tool('lol.match') tool('music', { action = 'play' })",
        ));
        let line = |settings: Settings, index: usize| {
            ToolRules::of(settings).lines(&automation)[index].clone()
        };
        let mut settings = Settings::default();
        assert_eq!(
            line(settings.clone(), 0).state,
            ToolState::Ready,
            "这一局的数据是可信的"
        );
        settings
            .plugins
            .entry("lol".into())
            .or_default()
            .sources
            .insert("client".into(), false);
        assert!(matches!(
            line(settings.clone(), 0).state,
            ToolState::Off(r) if r.contains("“读客户端数据”")
        ));
        settings.plugins.get_mut("lol").unwrap().enabled = Some(false);
        let off = line(settings.clone(), 0);
        assert!(matches!(&off.state, ToolState::Off(r) if r.contains("插件关着")));
        assert_eq!(off.state.word(), "off");
        assert_ne!(off.title, "lol.match", "插件关着也有显示名");

        let mut settings = Settings::default();
        settings.agent.permissions.mode = kotone_core::agent::Mode::ReadOnly;
        assert!(matches!(
            line(settings.clone(), 1).state,
            ToolState::Blocked(r) if r.contains("只读模式") && r.contains("「权限」")
        ));
        let mut settings = Settings::default();
        settings
            .agent
            .permissions
            .rules
            .insert("music".into(), kotone_core::agent::Rule::Deny);
        assert!(matches!(
            line(settings, 1).state,
            ToolState::Blocked(r) if r.contains("禁止")
        ));
    }

    /// 测试清单 11、13：安排之前查一遍——等你确认的不运行（说去「自动化」页点允许）；临时程序只能调允许的；工具名
    /// 不是直接写的、调不了的不运行，原因里带着是哪个工具。
    #[test]
    fn nothing_runs_while_a_tool_waits_for_confirmation_or_cannot_be_called() {
        let rules = ToolRules::of(Settings::default());
        let none = Confirmed::new();
        let asks = "tool('settings.update', { patch = {} })";
        let error = rules.check(asks, Some(&none)).unwrap_err();
        assert!(
            error.contains("「自动化」页") && error.contains("settings.update"),
            "{error}"
        );
        let confirmed: Confirmed = rules.to_confirm(asks);
        assert!(rules.check(asks, Some(&confirmed)).is_ok());
        // 临时程序（她和外部 Agent 的 automation.run {source}）：要确认的就是不能调，确认了也不算
        let temp = rules.check(asks, None).unwrap_err();
        assert!(temp.contains("临时程序"), "{temp}");
        assert!(rules
            .check("tool('music', { action = 'play' })", None)
            .is_ok());
        assert!(rules
            .check("tool(args.name)", None)
            .unwrap_err()
            .contains("直接写工具名"));
        assert!(rules
            .check(
                "say('好') tool('game_chat.send', { text = 'gg' })",
                Some(&none)
            )
            .unwrap_err()
            .contains("game_chat.send"));
        // 工具后来多了副作用：要重新确认
        let mut stale = confirmed.clone();
        stale.insert("settings.update".into(), Vec::new());
        assert!(rules.check(asks, Some(&stale)).is_err());
        assert!(rules.check("say('只说一句')", None).is_ok());
    }

    /// D3、D9：编辑器照还没存的程序算“会调用：…”——程序没改，存着的确认算数（直接“保存”）；改了程序，要再点一次
    /// “允许并保存”；新建的、工具名不是直接写的说清楚。只给插件页面用的工具调不了。
    #[test]
    fn the_editor_previews_the_program_being_written() {
        let source = "tool('settings.update', { patch = {} })";
        let (_dir, library, automation) = saved(program(source));
        let rules = ToolRules::of(Settings::default());
        let confirmed = library
            .confirm(&automation.id, source, rules.to_confirm(source))
            .unwrap();
        let same = preview(&format!("{source}\n"), Some(&confirmed), &rules);
        assert_eq!(
            (same["needsConfirm"].clone(), same["error"].clone()),
            (json!(false), Value::Null)
        );
        assert_eq!(same["tools"][0]["state"], "ready");
        let edited = preview(&format!("{source}\nsay('好')"), Some(&confirmed), &rules);
        assert_eq!(edited["needsConfirm"], true, "改了程序要重新允许");
        assert_eq!(preview(source, None, &rules)["needsConfirm"], true);
        let bad = preview("tool(args.name)", None, &rules);
        assert!(bad["error"].as_str().unwrap().contains("直接写工具名"));
        assert_eq!(bad["tools"], json!([]));
        let page = preview("tool('music.state')", None, &rules);
        assert_eq!(page["tools"][0]["state"], "blocked");
        assert!(page["tools"][0]["reason"]
            .as_str()
            .unwrap()
            .contains("页面"));
    }
}
