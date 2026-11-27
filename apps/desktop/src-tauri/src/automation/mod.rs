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

mod program;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

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

/// 旧用户只有在启用琴音后才安装内置程序；既有删除记录继续有效。
pub fn ensure_builtin(app: &AppHandle) {
    if let Err(e) = library(app).and_then(|library| library.ensure_builtin()) {
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
pub fn start(
    app: &AppHandle,
    session: &str,
    request: ProgramRequest<'_>,
    due_at_ms: u64,
) -> Result<Value, String> {
    schedule(app, session, request, None, due_at_ms)
}

/// [`start`]，带上触发它的游戏事件（按事件触发的执行：随执行记录保存，`ask()` 交给她时附上，ADR 045 D8）。
fn schedule(
    app: &AppHandle,
    session: &str,
    request: ProgramRequest<'_>,
    event: Option<GameEvent>,
    due_at_ms: u64,
) -> Result<Value, String> {
    let source = check_source(request.source)?;
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
                    .uses_legacy_assistant()
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
/// ▶ 先倒数、要切到游戏）和 `asks`（会叫她，每次花一次 AI 调用）一起给（ADR 045 D9）。
pub fn automation_json(automation: &Automation, with_source: bool) -> Value {
    let uses = automation.action.uses();
    let mut value = serde_json::to_value(automation).unwrap_or(Value::Null);
    if let Some(object) = value.as_object_mut() {
        if !with_source {
            if let Some(action) = object.get_mut("action").and_then(Value::as_object_mut) {
                action.remove("source");
            }
        }
        object.insert("usesWindow".into(), json!(uses.window));
        object.insert("asks".into(), json!(uses.asks));
    }
    value
}

/// 保存（新建或按 id 覆盖）；程序先检查语法。游戏事件触发按合并后的事件目录校验（插件登记的也认得）；
/// 触发里的事现在不会发生时（例如英雄联盟插件关着）照常保存，结果里多一项 `triggerNotes` 说清楚（原则 21）。
pub fn save(app: &AppHandle, mut draft: AutomationDraft, by: CreatedBy) -> Result<Value, String> {
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
    let mut value = automation_json(&saved, true);
    let notes = saved
        .on
        .as_ref()
        .map(|on| events.notes(on))
        .unwrap_or_default();
    if let (false, Some(object)) = (notes.is_empty(), value.as_object_mut()) {
        object.insert("triggerNotes".into(), json!(notes));
    }
    Ok(value)
}

pub fn get(app: &AppHandle, id: &str) -> Result<Value, String> {
    library(app)?
        .get(id)
        .map(|a| automation_json(&a, true))
        .ok_or_else(|| format!("找不到自动化 {id}"))
}

pub fn list_saved(app: &AppHandle) -> Result<Value, String> {
    let list = library(app)?.list();
    let now = kotone_core::automation::now_ms();
    Ok(json!({
        "automations": list.iter().map(|a| {
            let mut value = automation_json(a, false);
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

/// 设置页保存（创建者记为用户）。
#[tauri::command]
pub fn automation_save(app: AppHandle, draft: AutomationDraft) -> Result<Value, String> {
    save(&app, draft, CreatedBy::User)
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
        assert_eq!(ids, ["say", "push", "ask", "blank"]);
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
}
