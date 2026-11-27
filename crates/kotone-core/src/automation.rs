//! 自动化调度器（docs/adr/013-automation-domain-model.md、ADR 045）：一次执行 = 一条 [`Run`]，动作只有
//! “程序”（沙盒 Luau，由宿主运行）。提醒是“晚一会儿开始、只说一句”的程序（[`reminder_source`]）。
//! 开始之前归调度器管（存盘、重启恢复、取消、错过不补），开始之后归宿主：经 [`ProgramLauncher`] 交给
//! 宿主，结束时宿主调用 [`AutomationScheduler::finish_program`]。
//!
//! 语义（与云函数相反之处）：最多执行一次、绝不自动重试；到期超过 [`MISSED_GRACE_MS`] 就标为
//! 错过；崩溃时“运行中”的记录恢复为失败；到期时绑了游戏窗口的名额已满则这次失败，不排队。
//! core 不执行模型代码，也不依赖脚本引擎。

use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, RwLock,
    },
    time::Duration,
};

use serde_json::{Map, Value};

use crate::assistant::LEGACY_REMINDER_SESSION_PREFIX;
use crate::cancel::CancellationToken;
use crate::games::GameEvent;
use crate::reminders::{
    validate_content, DeliveryKind, LegacyReminderStore, Reminder, ReminderIntent, ReminderStatus,
    MISSED_GRACE_MS,
};

pub use crate::reminders::now_ms;

/// 同时运行、绑了游戏窗口的执行上限（ADR 045 D5）；不用窗口的（提醒、只说话的）不占名额。
pub const MAX_RUNNING_PROGRAMS: usize = 3;
/// 程序源码上限（字符）。
pub const MAX_SOURCE_CHARS: usize = 8_000;
/// 延迟上限：7 天。
pub const MAX_DELAY_SECONDS: u64 = 604_800;
const MAX_PENDING: usize = 100;
const MAX_BATCH: usize = 8;
const MAX_HISTORY: usize = 1_000;
pub(crate) const MAX_NAME_CHARS: usize = 40;
const MAX_ARGS_BYTES: usize = 4_096;
/// 触发它的游戏事件随执行记录保存（`ask()` 附上）；太大的只留信封、去掉 `data`，执行记录文件不会撑爆。
const MAX_EVENT_BYTES: usize = 8_192;
const MAX_LOG_LINES: usize = 50;
const MAX_MESSAGE_CHARS: usize = 1_000;
const STORE_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RunStatus {
    Pending,
    Running,
    Finished,
    Failed,
    Cancelled,
    Missed,
}

impl RunStatus {
    pub fn is_open(self) -> bool {
        matches!(self, RunStatus::Pending | RunStatus::Running)
    }
}

/// 旧写法（rc.13 及以前）的“播报”：只为读旧定义、旧执行记录，读进来就转成程序（ADR 045 D1、D3），不再写。
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SpeakAction {
    pub text: String,
    pub delivery: Vec<DeliveryKind>,
}

/// 提醒的三段固定程序（ADR 045 D3）：只要语音、只要弹幕、两者都要。她的提醒工具、0.1 升级上来的旧助手、
/// 0.1 提醒导入、旧执行记录里的播报都生成这三段之一，`args.text` 是要说的话；[`Run::as_reminder`] 按它认提醒。
const REMIND_VOICE: &str = "say(args.text)";
const REMIND_DANMAKU: &str = "toast(args.text)";
const REMIND_BOTH: &str = "say(args.text, { toast = true })";

/// 按回答方式选提醒的程序（空的当语音）。
pub fn reminder_source(delivery: &[DeliveryKind]) -> &'static str {
    match (
        delivery.contains(&DeliveryKind::Voice),
        delivery.contains(&DeliveryKind::Danmaku),
    ) {
        (true, true) => REMIND_BOTH,
        (false, true) => REMIND_DANMAKU,
        _ => REMIND_VOICE,
    }
}

/// 反过来：是提醒的三段程序之一就给出回答方式。
fn reminder_delivery(source: &str) -> Option<Vec<DeliveryKind>> {
    match source.trim() {
        REMIND_VOICE => Some(vec![DeliveryKind::Voice]),
        REMIND_DANMAKU => Some(vec![DeliveryKind::Danmaku]),
        REMIND_BOTH => Some(vec![DeliveryKind::Voice, DeliveryKind::Danmaku]),
        _ => None,
    }
}

/// 一条提醒的程序（不绑窗口；名字取正文前几个字）。
fn reminder_action(text: &str, delivery: &[DeliveryKind]) -> ProgramAction {
    let text = text.trim();
    let mut args = Map::new();
    args.insert("text".into(), Value::from(text));
    ProgramAction {
        name: text.chars().take(MAX_NAME_CHARS).collect(),
        source: reminder_source(delivery).into(),
        automation_id: None,
        args,
        target: None,
        event: None,
    }
}

/// 程序要作用的游戏窗口：在**下指令时**记录，到期时窗口已关闭则失败，不改投当时的前台窗口。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunTarget {
    pub handle: u64,
    pub pid: u32,
    #[serde(default)]
    pub process_started_at: Option<u64>,
    /// 展示用：“标题 (进程名)”。
    pub label: String,
}

/// 程序：一段 Luau 源码（快照）与调用时固定的入参。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgramAction {
    pub name: String,
    pub source: String,
    /// 来自哪个已保存的自动化；临时程序为 None。
    #[serde(default)]
    pub automation_id: Option<String>,
    #[serde(default)]
    pub args: Map<String, Value>,
    /// 绑定的游戏窗口：程序里用到游戏窗口（[`library::ProgramUses`]）才有；None = 不看前台、不占名额（ADR 045 D4）。
    #[serde(default)]
    pub target: Option<RunTarget>,
    /// 触发它的游戏事件（按事件触发的才有，ADR 045 D8）：`ask()` 交给她时附上；对外只露 `type` 和 `game`。
    /// rc.13 读到这个字段会忽略。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<GameEvent>,
}

impl ProgramAction {
    fn validate(&self) -> Result<(), String> {
        let chars = self.source.trim().chars().count();
        if chars == 0 || chars > MAX_SOURCE_CHARS {
            return Err(format!("自动化内容为空或超过 {MAX_SOURCE_CHARS} 字"));
        }
        if self.name.chars().count() > MAX_NAME_CHARS {
            return Err(format!("自动化名字最多 {MAX_NAME_CHARS} 字"));
        }
        let args = serde_json::to_vec(&self.args).map_err(|e| e.to_string())?;
        if args.len() > MAX_ARGS_BYTES {
            return Err("自动化入参过大".into());
        }
        Ok(())
    }

    /// 从触发事件填进入参的文字值：（参数名，值）。填法和 [`library::event_args`] 一样（参数名和事件 `data`
    /// 字段同名，或 camelCase 字段对 snake_case 参数）。
    pub fn event_filled_texts(&self) -> Vec<(&str, &str)> {
        let Some(event) = &self.event else {
            return Vec::new();
        };
        self.args
            .iter()
            .filter_map(|(name, value)| {
                let text = value.as_str().filter(|s| !s.trim().is_empty())?;
                let from_event = event
                    .data
                    .iter()
                    .any(|(k, v)| (k == name || library::snake(k) == *name) && v == value);
                from_event.then_some((name.as_str(), text))
            })
            .collect()
    }

    /// `ask()` 的两道护栏（ADR 045 D2）。宿主（桌面端 `DesktopHost::ask`）把话交给她**之前**调用，报错就原样
    /// 交回脚本（脚本引擎只查长度）：
    /// - 只在保存的自动化里能用：临时程序（`automation_id = None`）没人看过，不能花 AI 调用、不能指使她；
    /// - 话里不能带从游戏事件填进来的入参值（别人起的名字不能进指令的位置）。
    pub fn check_ask(&self, text: &str) -> Result<(), String> {
        if self.automation_id.is_none() {
            return Err("ask() 只能在保存的自动化里用：先把这段程序保存成自动化".into());
        }
        if let Some((name, _)) = self
            .event_filled_texts()
            .into_iter()
            .find(|(_, value)| text.contains(value))
        {
            return Err(format!(
                "ask() 的话里带了游戏事件填进来的参数 {name}：别人起的名字不能进交给她的话里，\
                 触发它的事件会一起交给她"
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RunAction {
    Program(ProgramAction),
}

/// 旧执行记录里的动作（只为读，[`AutomationScheduler::open`] 读到就换掉并存盘）：播报换成提醒的程序；
/// rc.13 及以前替玩家发到游戏聊天的一句话（`kind: "chat"`，那不是自动化，2026-11-27 玩家）去掉。
#[derive(serde::Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum StoredAction {
    Speak(SpeakAction),
    Program(Box<ProgramAction>),
    Chat(serde::de::IgnoredAny),
}

/// 程序执行的结果摘要。
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgramReport {
    /// `finished` / `stopped` / `limit` / `error`
    pub outcome: String,
    pub keys: u32,
    pub duration_ms: u64,
    #[serde(default)]
    pub log: Vec<String>,
}

/// 一次执行（执行记录）。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Run {
    pub id: String,
    /// 谁发起的（助手会话、陪玩、工具等）。
    pub session_id: String,
    pub action: RunAction,
    pub created_at_ms: u64,
    pub due_at_ms: u64,
    pub status: RunStatus,
    /// 失败、停止或错过的原因。
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub started_at_ms: Option<u64>,
    #[serde(default)]
    pub finished_at_ms: Option<u64>,
    #[serde(default)]
    pub report: Option<ProgramReport>,
}

/// 存盘时的执行记录：和 [`Run`] 一样，只是动作还可能是旧写法。
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredRun {
    id: String,
    session_id: String,
    action: StoredAction,
    created_at_ms: u64,
    due_at_ms: u64,
    status: RunStatus,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    started_at_ms: Option<u64>,
    #[serde(default)]
    finished_at_ms: Option<u64>,
    #[serde(default)]
    report: Option<ProgramReport>,
}

impl StoredRun {
    /// 换成现在的样子；旧的发到游戏聊天去掉（None）。第二项：是不是换过。
    fn upgrade(self) -> (Option<Run>, bool) {
        let (action, changed) = match self.action {
            StoredAction::Program(program) => (*program, false),
            StoredAction::Speak(speak) => (reminder_action(&speak.text, &speak.delivery), true),
            StoredAction::Chat(_) => return (None, true),
        };
        let run = Run {
            id: self.id,
            session_id: self.session_id,
            action: RunAction::Program(action),
            created_at_ms: self.created_at_ms,
            due_at_ms: self.due_at_ms,
            status: self.status,
            message: self.message,
            started_at_ms: self.started_at_ms,
            finished_at_ms: self.finished_at_ms,
            report: self.report,
        };
        (Some(run), changed)
    }
}

impl Run {
    pub fn program(&self) -> &ProgramAction {
        let RunAction::Program(program) = &self.action;
        program
    }

    /// 绑了游戏窗口（占名额、停下时连等待中的一起取消，ADR 045 D5、D6）。
    pub fn bound_to_window(&self) -> bool {
        self.program().target.is_some()
    }

    /// 展示名：提醒取正文，别的取名字。
    pub fn title(&self) -> &str {
        let program = self.program();
        match self.reminder_text() {
            Some(text) => text,
            None => &program.name,
        }
    }

    /// 提醒形状的程序（三段固定源码之一，`args.text` 是文字）的正文和回答方式。
    fn reminder_parts(&self) -> Option<(&str, Vec<DeliveryKind>)> {
        let program = self.program();
        let delivery = reminder_delivery(&program.source)?;
        let text = program.args.get("text")?.as_str()?;
        Some((text, delivery))
    }

    fn reminder_text(&self) -> Option<&str> {
        self.reminder_parts().map(|(text, _)| text)
    }

    /// 提醒界面与旧助手接口沿用的视图（只对提醒形状的程序有意义）。
    pub fn as_reminder(&self) -> Option<Reminder> {
        let (text, delivery) = self.reminder_parts()?;
        Some(Reminder {
            id: self.id.clone(),
            session_id: self.session_id.clone(),
            text: text.to_string(),
            delivery,
            created_at_ms: self.created_at_ms,
            due_at_ms: self.due_at_ms,
            status: match self.status {
                RunStatus::Pending => ReminderStatus::Pending,
                RunStatus::Running => ReminderStatus::Delivering,
                RunStatus::Finished => ReminderStatus::Delivered,
                RunStatus::Failed => ReminderStatus::Failed,
                RunStatus::Cancelled => ReminderStatus::Cancelled,
                RunStatus::Missed => ReminderStatus::Missed,
            },
            error: self.message.clone(),
        })
    }

    fn validate(&self) -> Result<(), String> {
        self.program().validate()
    }
}

/// 程序执行端口（宿主实现）。`launch` 必须很快返回（另起线程运行），程序结束时宿主调用
/// [`AutomationScheduler::finish_program`]；调用调度器时不得持有宿主自己的锁。
pub trait ProgramLauncher: Send + Sync {
    fn launch(&self, run: &Run) -> Result<(), String>;
    /// 请求停止一个运行中的程序；结束仍经 `finish_program` 回报。
    fn stop(&self, run_id: &str, reason: &str);
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct StoredRuns {
    version: u32,
    next_id: u64,
    /// 旧 `reminders.json` 是否已导入（只导入一次，旧文件不改）。
    imported_reminders: bool,
    runs: Vec<Run>,
}

/// 存盘文件（读）：执行记录可能还是旧写法。
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredRunsFile {
    version: u32,
    next_id: u64,
    #[serde(default)]
    imported_reminders: bool,
    runs: Vec<StoredRun>,
}

impl Default for StoredRuns {
    fn default() -> Self {
        Self {
            version: STORE_VERSION,
            next_id: 1,
            imported_reminders: false,
            runs: vec![],
        }
    }
}

impl StoredRuns {
    fn allocate_id(&mut self) -> Result<String, String> {
        let id = format!("run-{}", self.next_id);
        self.next_id = self.next_id.checked_add(1).ok_or("执行编号超出范围")?;
        Ok(id)
    }

    fn open_count(&self) -> usize {
        self.runs.iter().filter(|r| r.status.is_open()).count()
    }

    /// 运行中、绑了游戏窗口的执行（名额只数这些）。
    fn running_programs(&self) -> usize {
        self.runs
            .iter()
            .filter(|r| r.bound_to_window() && r.status == RunStatus::Running)
            .count()
    }

    fn trim_history(&mut self) -> Result<(), String> {
        while self.runs.len() > MAX_HISTORY {
            let index = self
                .runs
                .iter()
                .position(|r| !r.status.is_open())
                .ok_or("执行记录已满")?;
            self.runs.remove(index);
        }
        Ok(())
    }
}

struct State {
    stored: StoredRuns,
}

type RunObserver = Arc<dyn Fn(&Run) + Send + Sync>;

pub struct AutomationScheduler {
    path: PathBuf,
    state: Mutex<State>,
    launcher: RwLock<Option<Arc<dyn ProgramLauncher>>>,
    on_changed: RwLock<Option<Arc<dyn Fn() + Send + Sync>>>,
    on_finished: RwLock<Option<RunObserver>>,
    program_changed: tokio::sync::Notify,
    shutdown: CancellationToken,
    started: AtomicBool,
}

fn clip(message: impl Into<String>) -> String {
    message.into().chars().take(MAX_MESSAGE_CHARS).collect()
}

impl AutomationScheduler {
    /// 打开执行记录。旧写法（播报、发到游戏聊天）在这里换掉并存盘（ADR 045 D3）：播报全部换成提醒的程序
    /// （不绑窗口，到期时间不变；已结束的作为历史），发到游戏聊天的去掉。
    pub fn open(path: PathBuf) -> Result<Arc<Self>, String> {
        let mut upgraded = false;
        let mut stored = match std::fs::read(&path) {
            Ok(bytes) => {
                if bytes.len() > 8 * 1024 * 1024 {
                    return Err("自动化记录文件超过大小上限".into());
                }
                let file: StoredRunsFile = serde_json::from_slice(&bytes)
                    .map_err(|e| format!("自动化记录无法读取，已保留原文件：{e}"))?;
                if file.version != STORE_VERSION {
                    return Err("自动化记录版本暂不支持，已保留原文件".into());
                }
                let mut runs = Vec::with_capacity(file.runs.len());
                for run in file.runs {
                    let (run, changed) = run.upgrade();
                    upgraded |= changed;
                    runs.extend(run);
                }
                if runs.len() > MAX_HISTORY {
                    return Err("自动化记录超过上限".into());
                }
                for run in &runs {
                    run.validate()?;
                }
                StoredRuns {
                    version: file.version,
                    next_id: file.next_id,
                    imported_reminders: file.imported_reminders,
                    runs,
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => StoredRuns::default(),
            Err(e) => return Err(format!("读取自动化记录失败：{e}")),
        };
        let now = now_ms();
        let mut recovered = upgraded;
        for run in &mut stored.runs {
            if run.status == RunStatus::Running {
                run.status = RunStatus::Failed;
                run.message = Some("Kotone 退出时自动化仍在运行，已停止，不会自动重新运行".into());
                run.finished_at_ms.get_or_insert(now);
                recovered = true;
            } else if run.status == RunStatus::Pending
                && now.saturating_sub(run.due_at_ms) > MISSED_GRACE_MS
            {
                run.status = RunStatus::Missed;
                run.message = Some("Kotone 关闭或休眠期间已错过启动时间，未补运行".into());
                recovered = true;
            }
        }
        let scheduler = Arc::new(Self {
            path,
            state: Mutex::new(State { stored }),
            launcher: RwLock::new(None),
            on_changed: RwLock::new(None),
            on_finished: RwLock::new(None),
            program_changed: tokio::sync::Notify::new(),
            shutdown: CancellationToken::default(),
            started: AtomicBool::new(false),
        });
        if recovered {
            scheduler.persist(&scheduler.state.lock().unwrap().stored)?;
        }
        Ok(scheduler)
    }

    /// 第一次启动时导入旧 `reminders.json` 的提醒与历史；旧文件不改不删。
    /// 已结束或超过原有宽限的提醒只作为历史显示。返回导入条数。
    pub fn import_legacy_reminders(&self, legacy: &Path) -> Result<usize, String> {
        if self.state.lock().unwrap().stored.imported_reminders {
            return Ok(0);
        }
        let legacy_runs: Vec<Reminder> = match std::fs::read(legacy) {
            Ok(bytes) => {
                if bytes.len() > 4 * 1024 * 1024 {
                    return Err("旧提醒文件超过大小上限，未导入".into());
                }
                let store: LegacyReminderStore = serde_json::from_slice(&bytes)
                    .map_err(|e| format!("旧提醒文件无法读取，未导入：{e}"))?;
                if store.version != 1 {
                    return Err("旧提醒文件版本不支持，未导入".into());
                }
                store.reminders
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => vec![],
            Err(e) => return Err(format!("读取旧提醒文件失败：{e}")),
        };
        let now = now_ms();
        let mut state = self.state.lock().unwrap();
        if state.stored.imported_reminders {
            return Ok(0);
        }
        let mut next = state.stored.clone();
        let mut imported = 0;
        for reminder in legacy_runs {
            if validate_content(&reminder.text, &reminder.delivery).is_err()
                || next.runs.iter().any(|r| r.id == reminder.id)
            {
                continue;
            }
            let (status, message) = match reminder.status {
                ReminderStatus::Pending
                    if now.saturating_sub(reminder.due_at_ms) > MISSED_GRACE_MS =>
                {
                    (
                        RunStatus::Missed,
                        Some("应用关闭或休眠期间已错过提醒，未补播".into()),
                    )
                }
                ReminderStatus::Pending => (RunStatus::Pending, reminder.error),
                ReminderStatus::Delivering => (
                    RunStatus::Failed,
                    Some("上次投递中应用退出，完成情况未知，未自动重播".into()),
                ),
                ReminderStatus::Delivered => (RunStatus::Finished, reminder.error),
                ReminderStatus::Failed => (RunStatus::Failed, reminder.error),
                ReminderStatus::Cancelled => (RunStatus::Cancelled, reminder.error),
                ReminderStatus::Missed => (RunStatus::Missed, reminder.error),
            };
            if status == RunStatus::Pending && next.open_count() >= MAX_PENDING {
                return Err("待执行提醒总数已满，未导入旧提醒。请先取消部分提醒再重启".into());
            }
            next.runs.push(Run {
                id: reminder.id,
                session_id: format!("{LEGACY_REMINDER_SESSION_PREFIX}{}", reminder.session_id),
                action: RunAction::Program(reminder_action(&reminder.text, &reminder.delivery)),
                created_at_ms: reminder.created_at_ms,
                due_at_ms: reminder.due_at_ms,
                status,
                message,
                started_at_ms: None,
                finished_at_ms: None,
                report: None,
            });
            imported += 1;
        }
        next.imported_reminders = true;
        next.trim_history()?;
        self.persist(&next)?;
        state.stored = next;
        drop(state);
        if imported > 0 {
            self.notify_changed();
        }
        Ok(imported)
    }

    pub fn set_on_changed(&self, observer: Arc<dyn Fn() + Send + Sync>) {
        *self.on_changed.write().unwrap() = Some(observer);
    }

    /// 每条执行进入终态（完成 / 失败 / 取消 / 错过）时调用一次。
    pub fn set_on_finished(&self, observer: RunObserver) {
        *self.on_finished.write().unwrap() = Some(observer);
    }

    pub fn set_launcher(&self, launcher: Arc<dyn ProgramLauncher>) {
        *self.launcher.write().unwrap() = Some(launcher);
        self.program_changed.notify_one();
    }

    fn launcher(&self) -> Option<Arc<dyn ProgramLauncher>> {
        self.launcher.read().unwrap().clone()
    }

    fn notify_changed(&self) {
        self.program_changed.notify_one();
        let observer = self.on_changed.read().unwrap().clone();
        if let Some(observer) = observer {
            observer();
        }
    }

    fn notify_finished(&self, runs: &[Run]) {
        let observer = self.on_finished.read().unwrap().clone();
        if let Some(observer) = observer {
            for run in runs {
                observer(run);
            }
        }
    }

    /// 提醒视图：只含提醒形状的程序，按到期时间排序。
    pub fn list(&self) -> Vec<Reminder> {
        let mut reminders: Vec<Reminder> = self
            .state
            .lock()
            .unwrap()
            .stored
            .runs
            .iter()
            .filter_map(Run::as_reminder)
            .collect();
        reminders.sort_by_key(|r| (r.due_at_ms, r.id.clone()));
        reminders
    }

    /// 全部执行记录：未结束的在前（按到期时间），其后是已结束的（新的在前）。
    pub fn runs(&self) -> Vec<Run> {
        let mut runs = self.state.lock().unwrap().stored.runs.clone();
        runs.sort_by(|a, b| match (a.status.is_open(), b.status.is_open()) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            (true, true) => (a.due_at_ms, &a.id).cmp(&(b.due_at_ms, &b.id)),
            (false, false) => (b.finished_at_ms.unwrap_or(b.due_at_ms), &b.id)
                .cmp(&(a.finished_at_ms.unwrap_or(a.due_at_ms), &a.id)),
        });
        runs
    }

    pub fn get(&self, id: &str) -> Option<Run> {
        self.state
            .lock()
            .unwrap()
            .stored
            .runs
            .iter()
            .find(|r| r.id == id)
            .cloned()
    }

    /// 运行中、绑了游戏窗口的执行数（名额只数这些）。
    pub fn running_programs(&self) -> usize {
        self.state.lock().unwrap().stored.running_programs()
    }

    /// 整批验证、整批落盘；调用方可在 CancellationToken::run_if_active 中调用。
    /// 每条都是一段提醒的程序（不绑窗口），返回的仍是提醒视图。
    pub fn schedule_batch(
        &self,
        session_id: &str,
        received_at_ms: u64,
        intents: &[ReminderIntent],
    ) -> Result<Vec<Reminder>, String> {
        let added = self
            .shutdown
            .run_if_active(|| self.schedule_active_batch(session_id, received_at_ms, intents))
            .ok_or_else(|| "提醒队列已关闭".to_string())??;
        if !added.is_empty() {
            self.notify_changed();
        }
        Ok(added)
    }

    fn schedule_active_batch(
        &self,
        session_id: &str,
        received_at_ms: u64,
        intents: &[ReminderIntent],
    ) -> Result<Vec<Reminder>, String> {
        if intents.is_empty() {
            return Ok(vec![]);
        }
        if intents.len() > MAX_BATCH {
            return Err("一次最多创建 8 条提醒".into());
        }
        for intent in intents {
            intent.validate()?;
        }
        let mut state = self.state.lock().unwrap();
        if state.stored.open_count() + intents.len() > MAX_PENDING {
            return Err("待执行提醒最多保留 100 条，请先取消部分提醒".into());
        }
        let mut next = state.stored.clone();
        let created = now_ms();
        let mut added = Vec::with_capacity(intents.len());
        for intent in intents {
            let due = received_at_ms
                .checked_add(intent.delay_seconds * 1_000)
                .ok_or("提醒时间超出范围")?;
            let run = Run {
                id: next.allocate_id()?,
                session_id: session_id.into(),
                action: RunAction::Program(reminder_action(&intent.text, &intent.delivery)),
                created_at_ms: created,
                due_at_ms: due,
                status: RunStatus::Pending,
                message: None,
                started_at_ms: None,
                finished_at_ms: None,
                report: None,
            };
            added.extend(run.as_reminder());
            next.runs.push(run);
        }
        next.trim_history()?;
        self.persist(&next)?;
        state.stored = next;
        Ok(added)
    }

    /// 安排一条提醒（“晚一会儿开始、只说一句”的程序，不绑窗口，ADR 045 D3）。`due_at_ms` 不晚于现在就
    /// 立即启动（同 [`AutomationScheduler::schedule_program`]）。
    pub fn schedule_reminder(
        &self,
        session_id: &str,
        text: &str,
        delivery: &[DeliveryKind],
        due_at_ms: u64,
    ) -> Result<Run, String> {
        validate_content(text, delivery)?;
        self.schedule_program(session_id, reminder_action(text, delivery), due_at_ms)
    }

    /// 安排一次程序执行。`due_at_ms` 不晚于现在就立即启动：绑窗口的名额已满、执行器不可用或宿主拒绝
    /// 启动时返回错误且**不留记录**；晚于现在则存为等待中。
    pub fn schedule_program(
        &self,
        session_id: &str,
        mut action: ProgramAction,
        due_at_ms: u64,
    ) -> Result<Run, String> {
        action.name = action.name.trim().chars().take(MAX_NAME_CHARS).collect();
        action.source = action.source.trim().to_string();
        action.validate()?;
        if let Some(event) = &mut action.event {
            let size = serde_json::to_vec(&*event).map_or(usize::MAX, |b| b.len());
            if size > MAX_EVENT_BYTES {
                event.data.clear();
            }
        }
        let now = now_ms();
        if due_at_ms > now.saturating_add(MAX_DELAY_SECONDS * 1_000) {
            return Err("自动化最多只能安排在 7 天之内".into());
        }
        let immediate = due_at_ms <= now;
        let launcher = if immediate {
            Some(self.launcher().ok_or("自动化执行器不可用")?)
        } else {
            None
        };
        let run = self
            .shutdown
            .run_if_active(|| -> Result<Run, String> {
                let mut state = self.state.lock().unwrap();
                if state.stored.open_count() >= MAX_PENDING {
                    return Err("等待中与运行中的自动化最多 100 条，请先取消一些".into());
                }
                if immediate
                    && action.target.is_some()
                    && state.stored.running_programs() >= MAX_RUNNING_PROGRAMS
                {
                    return Err(format!(
                        "最多同时运行 {MAX_RUNNING_PROGRAMS} 个操作游戏的自动化，请先停止一个"
                    ));
                }
                let mut next = state.stored.clone();
                let run = Run {
                    id: next.allocate_id()?,
                    session_id: session_id.into(),
                    action: RunAction::Program(action),
                    created_at_ms: now,
                    due_at_ms: if immediate { now } else { due_at_ms },
                    status: if immediate {
                        RunStatus::Running
                    } else {
                        RunStatus::Pending
                    },
                    message: None,
                    started_at_ms: immediate.then_some(now),
                    finished_at_ms: None,
                    report: None,
                };
                next.runs.push(run.clone());
                next.trim_history()?;
                self.persist(&next)?;
                state.stored = next;
                Ok(run)
            })
            .ok_or_else(|| "自动化调度已关闭".to_string())??;
        if let Some(launcher) = launcher {
            if let Err(error) = launcher.launch(&run) {
                self.remove(&run.id)?;
                return Err(error);
            }
        }
        self.notify_changed();
        Ok(run)
    }

    fn remove(&self, id: &str) -> Result<(), String> {
        let mut state = self.state.lock().unwrap();
        let mut next = state.stored.clone();
        next.runs.retain(|r| r.id != id);
        self.persist(&next)?;
        state.stored = next;
        Ok(())
    }

    /// 程序结束（宿主回报）。只接受“运行中”的记录。
    pub fn finish_program(
        &self,
        id: &str,
        status: RunStatus,
        message: Option<String>,
        report: Option<ProgramReport>,
    ) -> Result<(), String> {
        if status.is_open() {
            return Err("结束状态不能是等待中或运行中".into());
        }
        let finished = {
            let mut state = self.state.lock().unwrap();
            let mut next = state.stored.clone();
            let Some(run) = next
                .runs
                .iter_mut()
                .find(|r| r.id == id && r.status == RunStatus::Running)
            else {
                return Ok(());
            };
            run.status = status;
            run.message = message.map(clip);
            run.finished_at_ms = Some(now_ms());
            run.report = report.map(|mut r| {
                r.log.truncate(MAX_LOG_LINES);
                r
            });
            let run = run.clone();
            self.persist(&next)?;
            state.stored = next;
            run
        };
        self.notify_changed();
        self.notify_finished(&[finished]);
        Ok(())
    }

    /// 取消等待中的执行，或停止运行中的执行。返回是否找到。
    pub fn cancel(&self, id: &str) -> Result<bool, String> {
        self.cancel_with_reason(id, "用户取消")
    }

    pub fn cancel_with_reason(&self, id: &str, reason: &str) -> Result<bool, String> {
        let mut state = self.state.lock().unwrap();
        let Some(run) = state
            .stored
            .runs
            .iter()
            .find(|r| r.id == id && r.status.is_open())
            .cloned()
        else {
            return Ok(false);
        };
        if run.status == RunStatus::Running {
            drop(state);
            match self.launcher() {
                Some(launcher) => launcher.stop(id, reason),
                None => self.finish_program(id, RunStatus::Cancelled, Some(reason.into()), None)?,
            }
            return Ok(true);
        }
        let mut next = state.stored.clone();
        let record = next.runs.iter_mut().find(|r| r.id == id).unwrap();
        record.status = RunStatus::Cancelled;
        record.message = Some(clip(reason));
        record.finished_at_ms = Some(now_ms());
        let cancelled = record.clone();
        self.persist(&next)?;
        state.stored = next;
        drop(state);
        self.notify_changed();
        self.notify_finished(&[cancelled]);
        Ok(true)
    }

    /// 停下（ADR 045 D6，行为和以前一样）：停掉全部运行中的执行；取消等待中、**绑了游戏窗口**的执行；
    /// 等待中的提醒（不绑窗口的）留着，单条在「自动化」页取消。“停下”不只是停止热键：玩家说“停下”打断她、
    /// 按 Esc 停她操作游戏也走这里，不能顺带悄悄取消“5 分钟后提醒我买眼”。返回涉及的条数。
    pub fn stop_programs(&self, reason: &str) -> Result<usize, String> {
        let (cancelled, running) = {
            let mut state = self.state.lock().unwrap();
            let mut next = state.stored.clone();
            let now = now_ms();
            let mut cancelled = vec![];
            let mut running = vec![];
            for run in next.runs.iter_mut() {
                match run.status {
                    RunStatus::Pending if run.bound_to_window() => {
                        run.status = RunStatus::Cancelled;
                        run.message = Some(clip(reason));
                        run.finished_at_ms = Some(now);
                        cancelled.push(run.clone());
                    }
                    RunStatus::Running => running.push(run.id.clone()),
                    _ => {}
                }
            }
            if !cancelled.is_empty() {
                self.persist(&next)?;
                state.stored = next;
            }
            (cancelled, running)
        };
        if let Some(launcher) = self.launcher() {
            for id in &running {
                launcher.stop(id, reason);
            }
        }
        if !cancelled.is_empty() {
            self.notify_changed();
            self.notify_finished(&cancelled);
        }
        Ok(cancelled.len() + running.len())
    }

    /// 应用启动一次；与 ASR runtime 无关，停止识别不停止已经接受的执行。
    pub fn start(self: &Arc<Self>) -> tokio::task::JoinHandle<()> {
        let already_started = self.started.swap(true, Ordering::SeqCst);
        if already_started {
            return tokio::spawn(async {});
        }
        let programs = self.clone();
        tokio::spawn(async move {
            loop {
                if programs.shutdown.is_cancelled() {
                    return;
                }
                if let Err(error) = programs.dispatch_programs(now_ms()) {
                    crate::log::log(&format!("automation scheduler: {error}"));
                }
                tokio::select! {
                    _ = programs.shutdown.cancelled() => return,
                    _ = programs.program_changed.notified() => {},
                    _ = tokio::time::sleep(Duration::from_millis(250)) => {},
                }
            }
        })
    }

    pub fn shutdown(&self) {
        self.shutdown.cancel();
    }

    /// 启动到点的程序（显式时钟，便于测试）。错过的标为错过；绑窗口的名额已满的这次失败
    /// （不绑窗口的照样启动）。
    pub fn dispatch_programs(&self, at_ms: u64) -> Result<(), String> {
        loop {
            if self.shutdown.is_cancelled() {
                return Ok(());
            }
            let Some(launcher) = self.launcher() else {
                return Ok(());
            };
            let (run, launch) = {
                let mut state = self.state.lock().unwrap();
                let mut next = state.stored.clone();
                let running = next.running_programs();
                let Some(index) = next
                    .runs
                    .iter()
                    .enumerate()
                    .filter(|(_, r)| r.status == RunStatus::Pending && r.due_at_ms <= at_ms)
                    .min_by_key(|(_, r)| r.due_at_ms)
                    .map(|(i, _)| i)
                else {
                    return Ok(());
                };
                let run = &mut next.runs[index];
                let launch = if at_ms.saturating_sub(run.due_at_ms) > MISSED_GRACE_MS {
                    run.status = RunStatus::Missed;
                    run.message = Some("已错过启动时间，未补运行".into());
                    false
                } else if run.bound_to_window() && running >= MAX_RUNNING_PROGRAMS {
                    run.status = RunStatus::Failed;
                    run.message = Some(format!(
                        "到点时已有 {MAX_RUNNING_PROGRAMS} 个操作游戏的自动化在运行，这次没有启动"
                    ));
                    run.finished_at_ms = Some(now_ms());
                    false
                } else {
                    run.status = RunStatus::Running;
                    run.started_at_ms = Some(now_ms());
                    true
                };
                let run = run.clone();
                self.persist(&next)?;
                state.stored = next;
                (run, launch)
            };
            self.notify_changed();
            if !launch {
                self.notify_finished(&[run]);
                continue;
            }
            if let Err(error) = launcher.launch(&run) {
                self.finish_program(&run.id, RunStatus::Failed, Some(error), None)?;
            }
        }
    }

    fn persist(&self, stored: &StoredRuns) -> Result<(), String> {
        let bytes = serde_json::to_vec(stored).map_err(|e| e.to_string())?;
        write_atomic(&self.path, &bytes, "自动化记录")
    }
}

/// 先写临时文件再改名，避免写到一半时崩溃留下半个文件。
pub(crate) fn write_atomic(path: &Path, bytes: &[u8], what: &str) -> Result<(), String> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建{what}目录失败：{e}"))?;
    }
    let tmp = path.with_extension("tmp");
    let mut file = std::fs::File::create(&tmp).map_err(|e| format!("写入{what}失败：{e}"))?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| format!("写入{what}失败：{e}"))?;
    drop(file);
    std::fs::rename(&tmp, path).map_err(|e| format!("保存{what}失败：{e}"))
}

pub mod cron;
pub mod library;

#[cfg(test)]
mod tests;
