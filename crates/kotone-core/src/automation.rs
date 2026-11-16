//! 自动化调度器（docs/adr/013-automation-domain-model.md）：一次执行 = 一条 [`Run`]，动作是
//! “播报”（原提醒）或“程序”（沙盒 Luau，由宿主运行）。开始之前归调度器管（存盘、重启恢复、
//! 取消、错过不补），开始之后归执行方：播报经 [`ReminderDelivery`] 投递，程序经
//! [`ProgramLauncher`] 交给宿主，结束时宿主调用 [`AutomationScheduler::finish_program`]。
//!
//! 语义（与云函数相反之处）：最多执行一次、绝不自动重试；到期超过 [`MISSED_GRACE_MS`] 就标为
//! 错过；崩溃时“运行中”的记录恢复为失败；到期时程序名额已满则这次失败，不排队。
//! core 不执行模型代码，也不依赖脚本引擎。

use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, RwLock,
    },
    time::{Duration, Instant},
};

use crate::cancel::CancellationToken;
use crate::reminders::{
    validate_content, DeliveryKind, LegacyReminderStore, Reminder, ReminderDelivery,
    ReminderIntent, ReminderStatus, MISSED_GRACE_MS,
};

pub use crate::reminders::now_ms;

/// 同时运行的程序上限（播报不占名额）。
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
const MAX_LOG_LINES: usize = 50;
const MAX_MESSAGE_CHARS: usize = 1_000;
const STORE_VERSION: u32 = 1;
// A desktop delivery may wait for a preview (60s), microphone (30s), speech
// (60s), then its visual fallback (60s). The timeout must cover valid settings.
const DELIVERY_TIMEOUT: Duration = Duration::from_secs(240);

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

/// 播报：今天的提醒。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeakAction {
    pub text: String,
    pub delivery: Vec<DeliveryKind>,
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
    pub args: serde_json::Map<String, serde_json::Value>,
    #[serde(default)]
    pub target: Option<RunTarget>,
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
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RunAction {
    Speak(SpeakAction),
    Program(ProgramAction),
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

impl Run {
    pub fn is_program(&self) -> bool {
        matches!(self.action, RunAction::Program(_))
    }

    /// 展示名：播报取正文，程序取名字。
    pub fn title(&self) -> &str {
        match &self.action {
            RunAction::Speak(s) => &s.text,
            RunAction::Program(p) => &p.name,
        }
    }

    /// 提醒界面与助手接口沿用的视图（只对播报有意义）。
    pub fn as_reminder(&self) -> Option<Reminder> {
        let RunAction::Speak(speak) = &self.action else {
            return None;
        };
        Some(Reminder {
            id: self.id.clone(),
            session_id: self.session_id.clone(),
            text: speak.text.clone(),
            delivery: speak.delivery.clone(),
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
        match &self.action {
            RunAction::Speak(s) => validate_content(&s.text, &s.delivery),
            RunAction::Program(p) => p.validate(),
        }
    }
}

/// 程序执行端口（宿主实现）。`launch` 必须很快返回（另起线程运行），程序结束时宿主调用
/// [`AutomationScheduler::finish_program`]；调用调度器时不得持有宿主自己的锁。
pub trait ProgramLauncher: Send + Sync {
    fn launch(&self, run: &Run) -> Result<(), String>;
    /// 请求停止一个运行中的程序；结束仍经 `finish_program` 回报。
    fn stop(&self, run_id: &str, reason: &str);
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredRuns {
    version: u32,
    next_id: u64,
    /// `reminders.json` 里未到期的提醒是否已导入（只导入一次，旧文件不改）。
    #[serde(default)]
    imported_reminders: bool,
    runs: Vec<Run>,
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

    fn running_programs(&self) -> usize {
        self.runs
            .iter()
            .filter(|r| r.is_program() && r.status == RunStatus::Running)
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
    /// 正在投递的播报。
    active: Option<(String, CancellationToken)>,
}

type RunObserver = Arc<dyn Fn(&Run) + Send + Sync>;

pub struct AutomationScheduler {
    path: PathBuf,
    state: Mutex<State>,
    delivery: Arc<dyn ReminderDelivery>,
    launcher: RwLock<Option<Arc<dyn ProgramLauncher>>>,
    on_changed: RwLock<Option<Arc<dyn Fn() + Send + Sync>>>,
    on_finished: RwLock<Option<RunObserver>>,
    speak_changed: tokio::sync::Notify,
    program_changed: tokio::sync::Notify,
    delivering: tokio::sync::Mutex<()>,
    shutdown: CancellationToken,
    started: AtomicBool,
}

fn clip(message: impl Into<String>) -> String {
    message.into().chars().take(MAX_MESSAGE_CHARS).collect()
}

impl AutomationScheduler {
    pub fn open(path: PathBuf, delivery: Arc<dyn ReminderDelivery>) -> Result<Arc<Self>, String> {
        let mut stored = match std::fs::read(&path) {
            Ok(bytes) => {
                if bytes.len() > 8 * 1024 * 1024 {
                    return Err("自动化记录文件超过大小上限".into());
                }
                let stored: StoredRuns = serde_json::from_slice(&bytes)
                    .map_err(|e| format!("自动化记录无法读取，已保留原文件：{e}"))?;
                if stored.version != STORE_VERSION {
                    return Err("自动化记录版本暂不支持，已保留原文件".into());
                }
                if stored.runs.len() > MAX_HISTORY {
                    return Err("自动化记录超过上限".into());
                }
                for run in &stored.runs {
                    run.validate()?;
                }
                stored
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => StoredRuns::default(),
            Err(e) => return Err(format!("读取自动化记录失败：{e}")),
        };
        let now = now_ms();
        let mut recovered = false;
        for run in &mut stored.runs {
            if run.status == RunStatus::Running {
                run.status = RunStatus::Failed;
                run.message = Some(if run.is_program() {
                    "Kotone 退出时自动化仍在运行，已停止，不会自动重新运行".into()
                } else {
                    "上次投递中应用退出，完成情况未知，未自动重播".into()
                });
                run.finished_at_ms.get_or_insert(now);
                recovered = true;
            } else if run.status == RunStatus::Pending
                && now.saturating_sub(run.due_at_ms) > MISSED_GRACE_MS
            {
                run.status = RunStatus::Missed;
                run.message = Some(if run.is_program() {
                    "Kotone 关闭或休眠期间已错过启动时间，未补运行".into()
                } else {
                    "应用关闭或休眠期间已错过提醒，未补播".into()
                });
                recovered = true;
            }
        }
        let scheduler = Arc::new(Self {
            path,
            state: Mutex::new(State {
                stored,
                active: None,
            }),
            delivery,
            launcher: RwLock::new(None),
            on_changed: RwLock::new(None),
            on_finished: RwLock::new(None),
            speak_changed: tokio::sync::Notify::new(),
            program_changed: tokio::sync::Notify::new(),
            delivering: tokio::sync::Mutex::new(()),
            shutdown: CancellationToken::default(),
            started: AtomicBool::new(false),
        });
        if recovered {
            scheduler.persist(&scheduler.state.lock().unwrap().stored)?;
        }
        Ok(scheduler)
    }

    /// 第一次启动时把旧 `reminders.json` 里尚未到期的提醒导入为播报执行；旧文件不改不删。
    /// 返回导入条数。
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
        let mut next = state.stored.clone();
        let mut imported = 0;
        for reminder in legacy_runs {
            if reminder.status != ReminderStatus::Pending
                || now.saturating_sub(reminder.due_at_ms) > MISSED_GRACE_MS
                || validate_content(&reminder.text, &reminder.delivery).is_err()
                || next.runs.iter().any(|r| r.id == reminder.id)
                || next.open_count() >= MAX_PENDING
            {
                continue;
            }
            next.runs.push(Run {
                id: reminder.id,
                session_id: reminder.session_id,
                action: RunAction::Speak(SpeakAction {
                    text: reminder.text,
                    delivery: reminder.delivery,
                }),
                created_at_ms: reminder.created_at_ms,
                due_at_ms: reminder.due_at_ms,
                status: RunStatus::Pending,
                message: None,
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
        self.speak_changed.notify_one();
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

    /// 提醒视图：只含播报，按到期时间排序。
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

    /// 运行中的程序数。
    pub fn running_programs(&self) -> usize {
        self.state.lock().unwrap().stored.running_programs()
    }

    /// 整批验证、整批落盘；调用方可在 CancellationToken::run_if_active 中调用。
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
                action: RunAction::Speak(SpeakAction {
                    text: intent.text.trim().into(),
                    delivery: intent.delivery.clone(),
                }),
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

    /// 安排一次播报（运行已保存的播报类自动化）。`due_at_ms` 不晚于现在就尽快投递。
    pub fn schedule_speak(
        &self,
        session_id: &str,
        mut action: SpeakAction,
        due_at_ms: u64,
    ) -> Result<Run, String> {
        action.text = action.text.trim().to_string();
        validate_content(&action.text, &action.delivery)?;
        let now = now_ms();
        if due_at_ms > now.saturating_add(MAX_DELAY_SECONDS * 1_000) {
            return Err("最多只能安排在 7 天之内".into());
        }
        let run = self
            .shutdown
            .run_if_active(|| -> Result<Run, String> {
                let mut state = self.state.lock().unwrap();
                if state.stored.open_count() >= MAX_PENDING {
                    return Err("等待中与运行中的自动化最多 100 条，请先取消一些".into());
                }
                let mut next = state.stored.clone();
                let run = Run {
                    id: next.allocate_id()?,
                    session_id: session_id.into(),
                    action: RunAction::Speak(action),
                    created_at_ms: now,
                    due_at_ms: due_at_ms.max(now),
                    status: RunStatus::Pending,
                    message: None,
                    started_at_ms: None,
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
        self.notify_changed();
        Ok(run)
    }

    /// 安排一次程序执行。`due_at_ms` 不晚于现在就立即启动：名额已满、执行器不可用或宿主拒绝
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
                if immediate && state.stored.running_programs() >= MAX_RUNNING_PROGRAMS {
                    return Err(format!(
                        "最多同时运行 {MAX_RUNNING_PROGRAMS} 个自动化，请先停止一个"
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

    /// 程序结束（宿主回报）。只接受“运行中”的程序记录。
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
                .find(|r| r.id == id && r.is_program() && r.status == RunStatus::Running)
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
        if run.is_program() && run.status == RunStatus::Running {
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
        record.message = record.is_program().then(|| clip(reason));
        record.finished_at_ms = Some(now_ms());
        let cancelled = record.clone();
        self.persist(&next)?;
        state.stored = next;
        let active = state
            .active
            .as_ref()
            .filter(|(active_id, _)| active_id == id)
            .map(|(_, token)| token.clone());
        drop(state);
        if let Some(token) = active {
            token.cancel();
        }
        self.notify_changed();
        self.notify_finished(&[cancelled]);
        Ok(true)
    }

    /// 停止全部运行中的程序并取消全部等待中的程序；等待中的播报（提醒）不受影响。
    /// 返回涉及的条数。停止热键、托盘与不带 id 的 `automation.stop` 用它。
    pub fn stop_programs(&self, reason: &str) -> Result<usize, String> {
        let (cancelled, running) = {
            let mut state = self.state.lock().unwrap();
            let mut next = state.stored.clone();
            let now = now_ms();
            let mut cancelled = vec![];
            let mut running = vec![];
            for run in next.runs.iter_mut().filter(|r| r.is_program()) {
                match run.status {
                    RunStatus::Pending => {
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
        let speak = self.clone();
        let speak = tokio::spawn(async move {
            loop {
                if speak.shutdown.is_cancelled() {
                    return;
                }
                if let Err(error) = speak.run_due(now_ms()).await {
                    crate::log::log(&format!("automation scheduler (speak): {error}"));
                }
                tokio::select! {
                    _ = speak.shutdown.cancelled() => return,
                    _ = speak.speak_changed.notified() => {},
                    _ = tokio::time::sleep(Duration::from_secs(1)) => {},
                }
            }
        });
        // 程序到点要准时，不能排在一条长播报后面：单独一个循环。
        let programs = self.clone();
        let programs = tokio::spawn(async move {
            loop {
                if programs.shutdown.is_cancelled() {
                    return;
                }
                if let Err(error) = programs.dispatch_programs(now_ms()) {
                    crate::log::log(&format!("automation scheduler (program): {error}"));
                }
                tokio::select! {
                    _ = programs.shutdown.cancelled() => return,
                    _ = programs.program_changed.notified() => {},
                    _ = tokio::time::sleep(Duration::from_millis(250)) => {},
                }
            }
        });
        tokio::spawn(async move {
            let _ = tokio::join!(speak, programs);
        })
    }

    pub fn shutdown(&self) {
        self.shutdown.cancel();
        if let Some((_, token)) = &self.state.lock().unwrap().active {
            token.cancel();
        }
    }

    /// 启动到点的程序（显式时钟，便于测试）。错过的标为错过；名额已满的这次失败。
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
                    .filter(|(_, r)| {
                        r.is_program() && r.status == RunStatus::Pending && r.due_at_ms <= at_ms
                    })
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
                } else if running >= MAX_RUNNING_PROGRAMS {
                    run.status = RunStatus::Failed;
                    run.message = Some(format!(
                        "到点时已有 {MAX_RUNNING_PROGRAMS} 个自动化在运行，这次没有启动"
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

    /// 投递到点的播报（显式时钟，便于确定性测试）；并发调用仍只允许一条投递管道。
    pub async fn run_due(&self, at_ms: u64) -> Result<(), String> {
        let _delivery = self.delivering.lock().await;
        let started = Instant::now();
        loop {
            if self.shutdown.is_cancelled() {
                return Ok(());
            }
            let claim = {
                let current_ms = at_ms.saturating_add(started.elapsed().as_millis() as u64);
                let mut state = self.state.lock().unwrap();
                let mut next = state.stored.clone();
                let Some(index) = next
                    .runs
                    .iter()
                    .enumerate()
                    .filter(|(_, r)| {
                        !r.is_program()
                            && r.status == RunStatus::Pending
                            && r.due_at_ms <= current_ms
                    })
                    .min_by_key(|(_, r)| r.due_at_ms)
                    .map(|(i, _)| i)
                else {
                    return Ok(());
                };
                let run = &mut next.runs[index];
                let missed = current_ms.saturating_sub(run.due_at_ms) > MISSED_GRACE_MS;
                if missed {
                    run.status = RunStatus::Missed;
                    run.message = Some("已错过提醒，未补播".into());
                } else {
                    run.status = RunStatus::Running;
                    run.started_at_ms = Some(now_ms());
                }
                let run = run.clone();
                self.persist(&next)?;
                state.stored = next;
                let token = CancellationToken::default();
                if !missed {
                    state.active = Some((run.id.clone(), token.clone()));
                }
                (run, token, missed)
            };
            self.notify_changed();
            let (run, token, missed) = claim;
            if missed {
                self.notify_finished(&[run]);
                continue;
            }
            let reminder = run.as_reminder().expect("speak run");
            if self.shutdown.is_cancelled() {
                token.cancel();
            }
            let result = if token.is_cancelled() {
                Err("提醒已取消".to_string())
            } else {
                let delivery = self.delivery.deliver(&reminder, token.clone());
                tokio::pin!(delivery);
                let interrupted = tokio::select! {
                    biased;
                    _ = token.cancelled() => "提醒已取消",
                    result = tokio::time::timeout(DELIVERY_TIMEOUT, &mut delivery) => {
                        match result {
                            Ok(result) => {
                                self.finish_delivery(&run.id, result)?;
                                continue;
                            }
                            Err(_) => "提醒投递超时",
                        }
                    },
                };
                token.cancel();
                // 不在收到取消的同一轮直接丢弃 delivery：壳需要继续 poll 才能
                // 清理已发出的语音/弹幕。收尾仍有界，壳另以 Drop guard 作兜底。
                let _ = tokio::time::timeout(Duration::from_secs(5), &mut delivery).await;
                Err(interrupted.to_string())
            };
            self.finish_delivery(&run.id, result)?;
        }
    }

    fn finish_delivery(&self, id: &str, result: Result<(), String>) -> Result<(), String> {
        let finished = {
            let mut state = self.state.lock().unwrap();
            state.active = None;
            let mut next = state.stored.clone();
            match next
                .runs
                .iter_mut()
                .find(|r| r.id == id && r.status == RunStatus::Running)
            {
                Some(record) => {
                    record.status = if result.is_ok() {
                        RunStatus::Finished
                    } else {
                        RunStatus::Failed
                    };
                    record.message = result.err().map(clip);
                    record.finished_at_ms = Some(now_ms());
                    let record = record.clone();
                    self.persist(&next)?;
                    state.stored = next;
                    Some(record)
                }
                None => None,
            }
        };
        self.notify_changed();
        if let Some(run) = finished {
            self.notify_finished(&[run]);
        }
        Ok(())
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

pub mod library;

#[cfg(test)]
mod tests;
