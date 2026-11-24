//! 自动化的桌面执行器（docs/adr/013-automation-domain-model.md、演进计划 §12）。
//!
//! - 执行记录由 core 的 [`AutomationScheduler`] 管（存盘、延迟、取消、错过不补、名额）；本模块是
//!   它的 [`ProgramLauncher`]：到点（或立即）启动一段 Luau 程序，结束时回报 `finish_program`。
//! - 每个程序在自己的线程里由 `kotone_script::run` 执行；宿主 [`DesktopHost`] 把白名单函数接到
//!   真实能力：按键 / 打字走 `SendInput`（与“帮忙打字”同一条输入通道），看屏幕用 GDI 截游戏
//!   客户区，`ocr()` 用陪玩选定的 OCR 模型，`say` / `toast` 复用提醒的交付路径。
//! - **只对下指令时的游戏窗口生效**：安排执行时记下前台窗口（[`RunTarget`]）；到点时窗口已关闭
//!   就不启动；运行中按键、截图前先确认它在前台，不在就暂停等待；窗口关闭则结束。
//! - **按住的键一定会松开**：`key_down` 记账，程序结束（正常、出错、取消、超限）时全部 `key_up`。
//! - 停止自动化热键、托盘菜单与不带 id 的 `automation.stop` 停止全部程序（含等待中的程序，
//!   不含提醒）；应用退出只停止运行中的程序，等待中的留到下次启动（过期则记为错过）。

mod program;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use kotone_core::automation::library::{
    validate_args, Automation, AutomationBody, AutomationDraft, AutomationLibrary, CreatedBy,
    ParamKind,
};
use kotone_core::automation::{
    AutomationScheduler, ChatAction, ProgramAction, ProgramLauncher, ProgramReport, Run, RunAction,
    RunStatus, RunTarget, SpeakAction, MAX_RUNNING_PROGRAMS, MAX_SOURCE_CHARS,
};
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

/// 执行记录的对外形状（工具与事件）。`with_source = false` 时省略程序源码。
pub fn run_json(run: &Run, with_source: bool) -> Value {
    let mut value = serde_json::to_value(run).unwrap_or(Value::Null);
    if !with_source {
        if let Some(action) = value.get_mut("action").and_then(Value::as_object_mut) {
            action.remove("source");
        }
    }
    value
}

/// 等待中、运行中与最近的执行记录（含提醒）。
pub fn list(app: &AppHandle) -> Result<Value, String> {
    let runs = scheduler(app)?.runs();
    let running = runs
        .iter()
        .filter(|r| r.is_program() && r.status == RunStatus::Running)
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

/// 安排一段程序：目标是**现在**的前台窗口（游戏）；`due_at_ms` 不晚于现在就立即启动。
pub fn start(
    app: &AppHandle,
    session: &str,
    request: ProgramRequest<'_>,
    due_at_ms: u64,
) -> Result<Value, String> {
    let source = check_source(request.source)?;
    let (target, window) = foreground_target_info()?;
    // 按游戏配置的“允许自动化输入”（ADR 014 第 5 步）；到点执行时还会再查一次。
    crate::game_tools::input_allowed(window.process_name.as_deref())?;
    let label = target.label.clone();
    let run = scheduler(app)?.schedule_program(
        session,
        ProgramAction {
            name: request.name.to_string(),
            source: source.to_string(),
            automation_id: request.automation_id,
            args: request.args,
            target: Some(target),
        },
        due_at_ms,
    )?;
    Ok(scheduled_json(&run, Some(&label)))
}

/// 运行一个已保存的自动化：先按声明校验入参；程序以现在的前台窗口为目标，播报不需要窗口。
pub fn run_saved(
    app: &AppHandle,
    session: &str,
    id: &str,
    args: &serde_json::Map<String, Value>,
    due_at_ms: u64,
) -> Result<Value, String> {
    let automation = library(app)?
        .get(id)
        .ok_or_else(|| format!("找不到自动化 {id}"))?;
    let args = validate_args(
        &automation.params,
        &store_arg_images(&automation.params, args)?,
    )?;
    match &automation.action {
        AutomationBody::Program(program) => start(
            app,
            session,
            ProgramRequest {
                name: &automation.name,
                source: &program.source,
                automation_id: Some(automation.id.clone()),
                args,
            },
            due_at_ms,
        ),
        AutomationBody::Speak(speak) => {
            // `{参数名}` 换成这次的值；`delay_seconds` = 几秒后再说。
            let (text, delay_ms) = kotone_core::automation::library::render_speak(speak, &args);
            let speak = kotone_core::automation::SpeakAction {
                text,
                delivery: speak.delivery.clone(),
            };
            let run = scheduler(app)?.schedule_speak(session, speak, due_at_ms + delay_ms)?;
            Ok(scheduled_json(&run, None))
        }
        AutomationBody::Prompt(prompt) => {
            crate::agent::run_prompt(app, &automation.name, &prompt.text, None);
            Ok(json!({"status": "sent"}))
        }
    }
}

/// `elapsedSeconds` → `elapsed_seconds`（参数名只能是小写字母、数字、下划线）。
fn snake(name: &str) -> String {
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

/// 游戏事件触发（ADR 036 D6）：和事件 `data` 字段同名的参数自动填上（事件是函数的输入）；
/// 同一个自动化的程序还在跑时按 `mode` 处理；“让她做”连同事件一起交给她。
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
    if let AutomationBody::Prompt(prompt) = &automation.action {
        crate::agent::run_prompt(app, &automation.name, &prompt.text, Some(&fire.event));
        return;
    }
    let args: serde_json::Map<String, Value> = automation
        .params
        .iter()
        .filter_map(|p| {
            fire.event
                .data
                .iter()
                .find(|(k, _)| **k == p.name || snake(k) == p.name)
                .map(|(_, v)| (p.name.clone(), v.clone()))
        })
        .collect();
    let mode = automation.on.as_ref().map(|t| t.mode).unwrap_or_default();
    let running = open_program_runs(app, &id);
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
                        run_event_now(&app, &id, &args);
                    });
                return;
            }
        }
    }
    run_event_now(app, &id, &args);
}

fn run_event_now(app: &AppHandle, id: &str, args: &serde_json::Map<String, Value>) {
    if let Err(e) = run_saved(app, "event", id, args, kotone_core::automation::now_ms()) {
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

/// 内置 Agent 的提醒：延迟播报，进同一个调度器（ADR 013“延迟 + 播报”）。
/// `delivery` 为空时用助手的默认方式。
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
    let run = scheduler(app)?.schedule_speak(session, SpeakAction { text, delivery }, due)?;
    Ok(scheduled_json(&run, None))
}

/// 内置 Agent 的“发到游戏聊天”：记一条执行记录（ADR 014“已确认”：替玩家发了什么要看得到）。
pub fn record_chat(
    app: &AppHandle,
    session: &str,
    action: ChatAction,
    result: Result<(), String>,
) -> Result<Run, String> {
    scheduler(app)?.record_chat(session, action, result)
}

/// 已保存的自动化（按名字排序；库不可用时为空）。
pub fn saved_list(app: &AppHandle) -> Vec<Automation> {
    library(app).map(|l| l.list()).unwrap_or_default()
}

/// 定义的对外形状；`with_source = false`（列表）时省略程序源码。
pub fn automation_json(automation: &Automation, with_source: bool) -> Value {
    let mut value = serde_json::to_value(automation).unwrap_or(Value::Null);
    if !with_source {
        if let Some(action) = value.get_mut("action").and_then(Value::as_object_mut) {
            action.remove("source");
        }
    }
    value
}

/// 保存（新建或按 id 覆盖）；程序先检查语法。游戏事件触发按合并后的事件目录校验（插件登记的也认得）；
/// 触发里的事现在不会发生时（例如英雄联盟插件关着）照常保存，结果里多一项 `triggerNotes` 说清楚（原则 21）。
pub fn save(app: &AppHandle, mut draft: AutomationDraft, by: CreatedBy) -> Result<Value, String> {
    if let AutomationBody::Program(program) = &mut draft.action {
        program.source = check_source(&program.source)?.to_string();
    }
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

/// 有自动化程序正在运行（`game.act` 据此不与它同时按键，ADR 019“输入锁”）。
pub fn program_running(app: &AppHandle) -> bool {
    app.try_state::<AutomationManager>()
        .is_some_and(|m| !m.live.lock().unwrap().is_empty())
}

/// 调度器到点（或立即）调用：确认目标窗口还在，然后在新线程里运行程序。
fn launch(app: &AppHandle, run: &Run) -> Result<(), String> {
    let RunAction::Program(program) = &run.action else {
        return Err("不是程序".into());
    };
    if crate::computer_use::busy(app) {
        return Err("正在替你操作游戏，自动化没有启动（不能同时按键）".into());
    }
    let target = program.target.as_ref().ok_or("没有记录目标窗口")?;
    let identity = WindowIdentity {
        handle: target.handle,
        pid: target.pid,
        process_started_at: target.process_started_at,
    };
    let window = WindowsWindowObserver
        .inspect(&identity)
        .map_err(|_| format!("游戏窗口已关闭（{}），自动化没有启动", target.label))?;
    crate::game_tools::input_allowed(window.process_name.as_deref())?;
    let id = run.id.clone();
    let cancel = Cancel::default();
    {
        let manager = app.state::<AutomationManager>();
        manager.live.lock().unwrap().insert(
            id.clone(),
            Live {
                cancel: cancel.clone(),
                end_reason: None,
            },
        );
    }
    let host = Arc::new(DesktopHost {
        app: app.clone(),
        session: GameSession::new(identity),
        cancel: cancel.clone(),
    });
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
            host.session.release_all();
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
            let gone = host.session.is_gone();
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

/// 取消 / 停止一条执行（`id`，提醒也可以）；`id = None` 停止全部程序（含等待中的，不含提醒）。
/// 返回涉及的条数。
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

#[tauri::command]
pub fn automation_saved_list(app: AppHandle) -> Result<Value, String> {
    list_saved(&app)
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

/// 设置页“运行”：点按钮时前台是 Kotone 自己，所以程序类先倒数 `switch_seconds` 秒
/// （1–30，让用户切到游戏），再以那时的前台窗口为目标运行；入参先校验，错了立即返回。
/// 倒数后的失败（例如前台仍是 Kotone）用语音 / 弹幕告诉用户。播报类立即运行。
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
    if matches!(
        automation.action,
        AutomationBody::Speak(_) | AutomationBody::Prompt(_)
    ) {
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
