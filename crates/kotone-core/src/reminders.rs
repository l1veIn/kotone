//! 本地提醒队列：受限计划原子落盘、到期投递、取消与启动恢复。
//! 调度与识别运行时独立；系统调用/语音实现由壳注入，不执行模型提供的代码。

use std::{
    future::Future,
    io::Write,
    path::PathBuf,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, RwLock,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use crate::cancel::CancellationToken;

pub const MISSED_GRACE_MS: u64 = 120_000;
const MAX_PENDING: usize = 100;
const MAX_BATCH: usize = 8;
const MAX_HISTORY: usize = 1_000;
// A desktop delivery may wait for a preview (60s), microphone (30s), speech
// (60s), then its visual fallback (60s). The timeout must cover valid settings.
const DELIVERY_TIMEOUT: Duration = Duration::from_secs(240);

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeliveryKind {
    Voice,
    Danmaku,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReminderIntent {
    pub delay_seconds: u64,
    pub text: String,
    pub delivery: Vec<DeliveryKind>,
}

impl ReminderIntent {
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=604_800).contains(&self.delay_seconds) {
            return Err("提醒时间必须在 1 秒到 7 天之间".into());
        }
        validate_content(&self.text, &self.delivery)
    }
}

fn validate_content(text: &str, delivery: &[DeliveryKind]) -> Result<(), String> {
    if !(1..=300).contains(&text.trim().chars().count()) {
        return Err("提醒内容必须为 1 到 300 个字符".into());
    }
    if delivery.is_empty()
        || delivery.len() > 2
        || (delivery.len() == 2 && delivery[0] == delivery[1])
    {
        return Err("提醒方式需为语音、弹幕或两者，不能重复".into());
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReminderStatus {
    Pending,
    Delivering,
    Delivered,
    Failed,
    Cancelled,
    Missed,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Reminder {
    pub id: String,
    pub session_id: String,
    pub text: String,
    pub delivery: Vec<DeliveryKind>,
    pub created_at_ms: u64,
    pub due_at_ms: u64,
    pub status: ReminderStatus,
    pub error: Option<String>,
}

pub type DeliveryFuture<'a> = Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>>;

pub trait ReminderDelivery: Send + Sync {
    /// 投递操作必须尊重取消；返回表示该投递已完成，不能悄悄丢给无人管理的任务。
    fn deliver<'a>(
        &'a self,
        reminder: &'a Reminder,
        cancel: CancellationToken,
    ) -> DeliveryFuture<'a>;
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredReminders {
    version: u32,
    next_id: u64,
    reminders: Vec<Reminder>,
}

impl Default for StoredReminders {
    fn default() -> Self {
        Self {
            version: 1,
            next_id: 1,
            reminders: vec![],
        }
    }
}

struct State {
    stored: StoredReminders,
    active: Option<(String, CancellationToken)>,
}

pub struct ReminderScheduler {
    path: PathBuf,
    state: Mutex<State>,
    delivery: Arc<dyn ReminderDelivery>,
    on_changed: RwLock<Option<Arc<dyn Fn() + Send + Sync>>>,
    changed: tokio::sync::Notify,
    delivering: tokio::sync::Mutex<()>,
    shutdown: CancellationToken,
    started: AtomicBool,
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

impl ReminderScheduler {
    pub fn open(path: PathBuf, delivery: Arc<dyn ReminderDelivery>) -> Result<Arc<Self>, String> {
        let mut stored = match std::fs::read(&path) {
            Ok(bytes) => {
                if bytes.len() > 4 * 1024 * 1024 {
                    return Err("提醒文件超过大小上限".into());
                }
                let stored: StoredReminders = serde_json::from_slice(&bytes)
                    .map_err(|e| format!("提醒文件无法读取，已保留原文件：{e}"))?;
                if stored.version != 1 {
                    return Err("提醒文件版本暂不支持，已保留原文件".into());
                }
                if stored.reminders.len() > MAX_HISTORY {
                    return Err("提醒记录超过上限".into());
                }
                for reminder in &stored.reminders {
                    validate_content(&reminder.text, &reminder.delivery)?;
                }
                stored
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => StoredReminders::default(),
            Err(e) => return Err(format!("读取提醒失败：{e}")),
        };
        let now = now_ms();
        let mut recovered = false;
        for reminder in &mut stored.reminders {
            if reminder.status == ReminderStatus::Delivering {
                reminder.status = ReminderStatus::Failed;
                reminder.error = Some("上次投递中应用退出，完成情况未知，未自动重播".into());
                recovered = true;
            } else if reminder.status == ReminderStatus::Pending
                && now.saturating_sub(reminder.due_at_ms) > MISSED_GRACE_MS
            {
                reminder.status = ReminderStatus::Missed;
                reminder.error = Some("应用关闭或休眠期间已错过提醒，未补播".into());
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
            on_changed: RwLock::new(None),
            changed: tokio::sync::Notify::new(),
            delivering: tokio::sync::Mutex::new(()),
            shutdown: CancellationToken::default(),
            started: AtomicBool::new(false),
        });
        if recovered {
            scheduler.persist(&scheduler.state.lock().unwrap().stored)?;
        }
        Ok(scheduler)
    }

    pub fn set_on_changed(&self, observer: Arc<dyn Fn() + Send + Sync>) {
        *self.on_changed.write().unwrap() = Some(observer);
    }

    fn notify_changed(&self) {
        self.changed.notify_one();
        let observer = self.on_changed.read().unwrap().clone();
        if let Some(observer) = observer {
            observer();
        }
    }

    pub fn list(&self) -> Vec<Reminder> {
        let mut reminders = self.state.lock().unwrap().stored.reminders.clone();
        reminders.sort_by_key(|r| (r.due_at_ms, r.id.clone()));
        reminders
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
        let pending = state
            .stored
            .reminders
            .iter()
            .filter(|r| {
                matches!(
                    r.status,
                    ReminderStatus::Pending | ReminderStatus::Delivering
                )
            })
            .count();
        if pending + intents.len() > MAX_PENDING {
            return Err("待执行提醒最多保留 100 条，请先取消部分提醒".into());
        }
        let mut next = state.stored.clone();
        let created = now_ms();
        let mut added = Vec::with_capacity(intents.len());
        for intent in intents {
            let due = received_at_ms
                .checked_add(intent.delay_seconds * 1_000)
                .ok_or("提醒时间超出范围")?;
            let id = format!("reminder-{}", next.next_id);
            next.next_id = next.next_id.checked_add(1).ok_or("提醒编号超出范围")?;
            let reminder = Reminder {
                id,
                session_id: session_id.into(),
                text: intent.text.trim().into(),
                delivery: intent.delivery.clone(),
                created_at_ms: created,
                due_at_ms: due,
                status: ReminderStatus::Pending,
                error: None,
            };
            next.reminders.push(reminder.clone());
            added.push(reminder);
        }
        while next.reminders.len() > MAX_HISTORY {
            let index = next
                .reminders
                .iter()
                .position(|r| {
                    !matches!(
                        r.status,
                        ReminderStatus::Pending | ReminderStatus::Delivering
                    )
                })
                .ok_or("提醒记录已满")?;
            next.reminders.remove(index);
        }
        self.persist(&next)?;
        state.stored = next;
        Ok(added)
    }

    pub fn cancel(&self, id: &str) -> Result<bool, String> {
        let mut state = self.state.lock().unwrap();
        let mut next = state.stored.clone();
        let Some(reminder) = next.reminders.iter_mut().find(|r| {
            r.id == id
                && matches!(
                    r.status,
                    ReminderStatus::Pending | ReminderStatus::Delivering
                )
        }) else {
            return Ok(false);
        };
        reminder.status = ReminderStatus::Cancelled;
        reminder.error = None;
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
        Ok(true)
    }

    /// 与 ASR runtime 无关，由应用启动一次；停止识别不应停止已经接受的提醒。
    pub fn start(self: &Arc<Self>) -> tokio::task::JoinHandle<()> {
        let scheduler = self.clone();
        let already_started = self.started.swap(true, Ordering::SeqCst);
        tokio::spawn(async move {
            if already_started {
                return;
            }
            loop {
                if scheduler.shutdown.is_cancelled() {
                    return;
                }
                if let Err(error) = scheduler.run_due(now_ms()).await {
                    crate::log::log(&format!("reminder scheduler: {error}"));
                }
                tokio::select! {
                    _ = scheduler.shutdown.cancelled() => return,
                    _ = scheduler.changed.notified() => {},
                    _ = tokio::time::sleep(Duration::from_secs(1)) => {},
                }
            }
        })
    }

    pub fn shutdown(&self) {
        self.shutdown.cancel();
        if let Some((_, token)) = &self.state.lock().unwrap().active {
            token.cancel();
        }
    }

    /// 使用显式时钟处理一轮，便于确定性测试；并发调用仍只允许一条投递管道。
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
                    .reminders
                    .iter()
                    .enumerate()
                    .filter(|(_, r)| {
                        r.status == ReminderStatus::Pending && r.due_at_ms <= current_ms
                    })
                    .min_by_key(|(_, r)| r.due_at_ms)
                    .map(|(i, _)| i)
                else {
                    return Ok(());
                };
                let reminder = &mut next.reminders[index];
                let missed = current_ms.saturating_sub(reminder.due_at_ms) > MISSED_GRACE_MS;
                reminder.status = if missed {
                    ReminderStatus::Missed
                } else {
                    ReminderStatus::Delivering
                };
                reminder.error = missed.then(|| "已错过提醒，未补播".into());
                let reminder = reminder.clone();
                self.persist(&next)?;
                state.stored = next;
                let token = CancellationToken::default();
                if !missed {
                    state.active = Some((reminder.id.clone(), token.clone()));
                }
                (reminder, token, missed)
            };
            self.notify_changed();
            let (reminder, token, missed) = claim;
            if missed {
                continue;
            }
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
                                self.finish_delivery(&reminder.id, result)?;
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
            self.finish_delivery(&reminder.id, result)?;
        }
    }

    fn finish_delivery(&self, id: &str, result: Result<(), String>) -> Result<(), String> {
        {
            let mut state = self.state.lock().unwrap();
            state.active = None;
            let mut next = state.stored.clone();
            if let Some(record) = next
                .reminders
                .iter_mut()
                .find(|r| r.id == id && r.status == ReminderStatus::Delivering)
            {
                record.status = if result.is_ok() {
                    ReminderStatus::Delivered
                } else {
                    ReminderStatus::Failed
                };
                record.error = result.err().map(|e| e.chars().take(1_000).collect());
                self.persist(&next)?;
                state.stored = next;
            }
        }
        self.notify_changed();
        Ok(())
    }

    fn persist(&self, stored: &StoredReminders) -> Result<(), String> {
        if let Some(parent) = self.path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent).map_err(|e| format!("创建提醒目录失败：{e}"))?;
        }
        let tmp = self.path.with_extension("tmp");
        let bytes = serde_json::to_vec(stored).map_err(|e| e.to_string())?;
        let mut file = std::fs::File::create(&tmp).map_err(|e| format!("写入提醒失败：{e}"))?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|e| format!("写入提醒失败：{e}"))?;
        drop(file);
        std::fs::rename(&tmp, &self.path).map_err(|e| format!("保存提醒失败：{e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct RecordingDelivery(Mutex<Vec<String>>);
    impl ReminderDelivery for RecordingDelivery {
        fn deliver<'a>(
            &'a self,
            reminder: &'a Reminder,
            _: CancellationToken,
        ) -> DeliveryFuture<'a> {
            Box::pin(async move {
                self.0.lock().unwrap().push(reminder.id.clone());
                Ok(())
            })
        }
    }
    fn intent(delay_seconds: u64) -> ReminderIntent {
        ReminderIntent {
            delay_seconds,
            text: "喝水".into(),
            delivery: vec![DeliveryKind::Voice, DeliveryKind::Danmaku],
        }
    }

    #[tokio::test]
    async fn persists_and_delivers_once_at_received_time_plus_delay() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("reminders.json");
        let delivery = Arc::new(RecordingDelivery::default());
        let queue = ReminderScheduler::open(path.clone(), delivery.clone()).unwrap();
        let base = now_ms();
        let added = queue.schedule_batch("session", base, &[intent(5)]).unwrap();
        assert_eq!(added[0].due_at_ms, base + 5_000);
        drop(queue);
        let queue = ReminderScheduler::open(path, delivery.clone()).unwrap();
        queue.run_due(base + 4_999).await.unwrap();
        assert!(delivery.0.lock().unwrap().is_empty());
        queue.run_due(base + 5_000).await.unwrap();
        queue.run_due(base + 6_000).await.unwrap();
        assert_eq!(delivery.0.lock().unwrap().len(), 1);
        assert_eq!(queue.list()[0].status, ReminderStatus::Delivered);
    }

    #[tokio::test]
    async fn cancel_and_expired_restart_never_deliver() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("reminders.json");
        let delivery = Arc::new(RecordingDelivery::default());
        let queue = ReminderScheduler::open(path.clone(), delivery.clone()).unwrap();
        let now = now_ms();
        let expired = queue
            .schedule_batch("old", now - MISSED_GRACE_MS - 10_000, &[intent(1)])
            .unwrap();
        let cancelled = queue.schedule_batch("new", now, &[intent(1)]).unwrap();
        queue.cancel(&cancelled[0].id).unwrap();
        drop(queue);
        let queue = ReminderScheduler::open(path, delivery.clone()).unwrap();
        queue.run_due(now + 2_000).await.unwrap();
        assert!(delivery.0.lock().unwrap().is_empty());
        assert_eq!(
            queue
                .list()
                .iter()
                .find(|r| r.id == expired[0].id)
                .unwrap()
                .status,
            ReminderStatus::Missed
        );
        assert_eq!(
            queue
                .list()
                .iter()
                .find(|r| r.id == cancelled[0].id)
                .unwrap()
                .status,
            ReminderStatus::Cancelled
        );
    }

    #[test]
    fn invalid_batch_and_failed_write_leave_memory_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let queue = ReminderScheduler::open(
            dir.path().join("reminders.json"),
            Arc::new(RecordingDelivery::default()),
        )
        .unwrap();
        assert!(queue
            .schedule_batch("s", now_ms(), &[intent(1), intent(0)])
            .is_err());
        assert!(queue.list().is_empty());
        std::fs::create_dir(&queue.path).unwrap();
        assert!(queue.schedule_batch("s", now_ms(), &[intent(1)]).is_err());
        assert!(queue.list().is_empty());
    }

    #[test]
    fn queue_limits_and_cancelled_commit_are_enforced() {
        let dir = tempfile::tempdir().unwrap();
        let queue = ReminderScheduler::open(
            dir.path().join("reminders.json"),
            Arc::new(RecordingDelivery::default()),
        )
        .unwrap();
        assert!(queue
            .schedule_batch("s", now_ms(), &vec![intent(1); 9])
            .is_err());
        for _ in 0..12 {
            queue
                .schedule_batch("s", now_ms(), &vec![intent(1); 8])
                .unwrap();
        }
        queue
            .schedule_batch("s", now_ms(), &vec![intent(1); 4])
            .unwrap();
        assert!(queue.schedule_batch("s", now_ms(), &[intent(1)]).is_err());
        let token = CancellationToken::default();
        token.cancel();
        assert!(token
            .run_if_active(|| queue.schedule_batch("s", now_ms(), &[intent(1)]))
            .is_none());
    }

    struct CleanupDelivery {
        entered: tokio::sync::Notify,
        cleaned: AtomicBool,
    }

    impl ReminderDelivery for CleanupDelivery {
        fn deliver<'a>(&'a self, _: &'a Reminder, cancel: CancellationToken) -> DeliveryFuture<'a> {
            Box::pin(async move {
                self.entered.notify_one();
                cancel.cancelled().await;
                // Models the desktop cancellation branch stopping speech/hiding an overlay.
                self.cleaned.store(true, Ordering::SeqCst);
                Err("cancelled".into())
            })
        }
    }

    #[tokio::test]
    async fn cancellation_polls_delivery_cleanup_and_preserves_cancelled_status() {
        let dir = tempfile::tempdir().unwrap();
        let delivery = Arc::new(CleanupDelivery {
            entered: tokio::sync::Notify::new(),
            cleaned: AtomicBool::new(false),
        });
        let queue =
            ReminderScheduler::open(dir.path().join("reminders.json"), delivery.clone()).unwrap();
        let base = now_ms();
        let added = queue.schedule_batch("s", base, &[intent(1)]).unwrap();
        let worker_queue = queue.clone();
        let worker = tokio::spawn(async move { worker_queue.run_due(base + 1_000).await });
        delivery.entered.notified().await;
        queue.cancel(&added[0].id).unwrap();
        tokio::time::timeout(Duration::from_secs(1), worker)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(delivery.cleaned.load(Ordering::SeqCst));
        assert_eq!(queue.list()[0].status, ReminderStatus::Cancelled);
    }

    #[test]
    fn interrupted_delivery_is_not_replayed_after_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("reminders.json");
        let queue =
            ReminderScheduler::open(path.clone(), Arc::new(RecordingDelivery::default())).unwrap();
        queue.schedule_batch("s", now_ms(), &[intent(1)]).unwrap();
        {
            let mut state = queue.state.lock().unwrap();
            state.stored.reminders[0].status = ReminderStatus::Delivering;
            queue.persist(&state.stored).unwrap();
        }
        drop(queue);
        let recovered =
            ReminderScheduler::open(path, Arc::new(RecordingDelivery::default())).unwrap();
        assert_eq!(recovered.list()[0].status, ReminderStatus::Failed);
        assert!(recovered.list()[0]
            .error
            .as_ref()
            .unwrap()
            .contains("未自动重播"));
    }

    #[tokio::test]
    async fn shutdown_closes_admission_and_preserves_existing_pending_reminders() {
        let dir = tempfile::tempdir().unwrap();
        let delivery = Arc::new(RecordingDelivery::default());
        let queue =
            ReminderScheduler::open(dir.path().join("reminders.json"), delivery.clone()).unwrap();
        let base = now_ms();
        let accepted = queue
            .schedule_batch("before-exit", base, &[intent(1)])
            .unwrap();
        queue.shutdown();
        assert!(queue
            .schedule_batch("late-ai-result", base, &[intent(1)])
            .is_err());
        queue.run_due(base + 1_000).await.unwrap();
        assert!(delivery.0.lock().unwrap().is_empty());
        assert_eq!(queue.list(), accepted);
    }
}
