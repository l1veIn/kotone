use super::*;
use crate::reminders::DeliveryFuture;

#[derive(Default)]
struct RecordingDelivery(Mutex<Vec<String>>);
impl ReminderDelivery for RecordingDelivery {
    fn deliver<'a>(&'a self, reminder: &'a Reminder, _: CancellationToken) -> DeliveryFuture<'a> {
        Box::pin(async move {
            self.0.lock().unwrap().push(reminder.id.clone());
            Ok(())
        })
    }
}

/// 假执行器：记录启动与停止；可设为拒绝启动。
#[derive(Default)]
struct FakeLauncher {
    launched: Mutex<Vec<String>>,
    stopped: Mutex<Vec<(String, String)>>,
    refuse: AtomicBool,
}
impl ProgramLauncher for FakeLauncher {
    fn launch(&self, run: &Run) -> Result<(), String> {
        if self.refuse.load(Ordering::SeqCst) {
            return Err("游戏窗口已关闭".into());
        }
        self.launched.lock().unwrap().push(run.id.clone());
        Ok(())
    }
    fn stop(&self, run_id: &str, reason: &str) {
        self.stopped
            .lock()
            .unwrap()
            .push((run_id.into(), reason.into()));
    }
}

fn intent(delay_seconds: u64) -> ReminderIntent {
    ReminderIntent {
        delay_seconds,
        text: "喝水".into(),
        delivery: vec![DeliveryKind::Voice, DeliveryKind::Danmaku],
    }
}

fn program(name: &str) -> ProgramAction {
    ProgramAction {
        name: name.into(),
        source: r#"every(1000, function() key("Q") end)"#.into(),
        automation_id: None,
        args: Default::default(),
        target: Some(RunTarget {
            handle: 42,
            pid: 7,
            process_started_at: None,
            label: "游戏 (game.exe)".into(),
        }),
    }
}

fn open_with_launcher(
    path: PathBuf,
) -> (
    Arc<AutomationScheduler>,
    Arc<FakeLauncher>,
    Arc<Mutex<Vec<Run>>>,
) {
    let queue = AutomationScheduler::open(path, Arc::new(RecordingDelivery::default())).unwrap();
    let launcher = Arc::new(FakeLauncher::default());
    queue.set_launcher(launcher.clone());
    let finished = Arc::new(Mutex::new(Vec::new()));
    let sink = finished.clone();
    queue.set_on_finished(Arc::new(move |run: &Run| {
        sink.lock().unwrap().push(run.clone())
    }));
    (queue, launcher, finished)
}

#[tokio::test]
async fn persists_and_delivers_once_at_received_time_plus_delay() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("automation-runs.json");
    let delivery = Arc::new(RecordingDelivery::default());
    let queue = AutomationScheduler::open(path.clone(), delivery.clone()).unwrap();
    let base = now_ms();
    let added = queue.schedule_batch("session", base, &[intent(5)]).unwrap();
    assert_eq!(added[0].due_at_ms, base + 5_000);
    drop(queue);
    let queue = AutomationScheduler::open(path, delivery.clone()).unwrap();
    queue.run_due(base + 4_999).await.unwrap();
    assert!(delivery.0.lock().unwrap().is_empty());
    queue.run_due(base + 5_000).await.unwrap();
    queue.run_due(base + 6_000).await.unwrap();
    assert_eq!(delivery.0.lock().unwrap().len(), 1);
    assert_eq!(queue.list()[0].status, ReminderStatus::Delivered);
    assert_eq!(queue.runs()[0].status, RunStatus::Finished);
}

#[tokio::test]
async fn cancel_and_expired_restart_never_deliver() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("automation-runs.json");
    let delivery = Arc::new(RecordingDelivery::default());
    let queue = AutomationScheduler::open(path.clone(), delivery.clone()).unwrap();
    let now = now_ms();
    let expired = queue
        .schedule_batch("old", now - MISSED_GRACE_MS - 10_000, &[intent(1)])
        .unwrap();
    let cancelled = queue.schedule_batch("new", now, &[intent(1)]).unwrap();
    queue.cancel(&cancelled[0].id).unwrap();
    drop(queue);
    let queue = AutomationScheduler::open(path, delivery.clone()).unwrap();
    queue.run_due(now + 2_000).await.unwrap();
    assert!(delivery.0.lock().unwrap().is_empty());
    let status = |id: &str| queue.list().iter().find(|r| r.id == id).unwrap().status;
    assert_eq!(status(&expired[0].id), ReminderStatus::Missed);
    assert_eq!(status(&cancelled[0].id), ReminderStatus::Cancelled);
}

#[test]
fn invalid_batch_and_failed_write_leave_memory_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let queue = AutomationScheduler::open(
        dir.path().join("automation-runs.json"),
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
    let (queue, _, _) = open_with_launcher(dir.path().join("automation-runs.json"));
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
    // 程序与提醒共用 100 条上限
    assert!(queue
        .schedule_program("s", program("连按"), now_ms() + 60_000)
        .is_err());
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
        AutomationScheduler::open(dir.path().join("automation-runs.json"), delivery.clone())
            .unwrap();
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
    let path = dir.path().join("automation-runs.json");
    let queue =
        AutomationScheduler::open(path.clone(), Arc::new(RecordingDelivery::default())).unwrap();
    queue.schedule_batch("s", now_ms(), &[intent(1)]).unwrap();
    {
        let mut state = queue.state.lock().unwrap();
        state.stored.runs[0].status = RunStatus::Running;
        queue.persist(&state.stored).unwrap();
    }
    drop(queue);
    let recovered =
        AutomationScheduler::open(path, Arc::new(RecordingDelivery::default())).unwrap();
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
        AutomationScheduler::open(dir.path().join("automation-runs.json"), delivery.clone())
            .unwrap();
    let base = now_ms();
    let accepted = queue
        .schedule_batch("before-exit", base, &[intent(1)])
        .unwrap();
    queue.shutdown();
    assert!(queue
        .schedule_batch("late-ai-result", base, &[intent(1)])
        .is_err());
    assert!(queue
        .schedule_program("late", program("连按"), base + 60_000)
        .is_err());
    queue.run_due(base + 1_000).await.unwrap();
    assert!(delivery.0.lock().unwrap().is_empty());
    assert_eq!(queue.list(), accepted);
}

#[test]
fn an_immediate_program_runs_now_and_its_end_is_recorded_once() {
    let dir = tempfile::tempdir().unwrap();
    let (queue, launcher, finished) = open_with_launcher(dir.path().join("automation-runs.json"));
    let run = queue
        .schedule_program("tool", program("  每秒按 Q  "), 0)
        .unwrap();
    assert_eq!(run.status, RunStatus::Running);
    assert_eq!(run.title(), "每秒按 Q");
    assert_eq!(*launcher.launched.lock().unwrap(), vec![run.id.clone()]);
    let report = ProgramReport {
        outcome: "finished".into(),
        keys: 10,
        duration_ms: 10_000,
        log: vec![],
    };
    queue
        .finish_program(&run.id, RunStatus::Finished, None, Some(report.clone()))
        .unwrap();
    // 迟到的第二次回报被忽略
    queue
        .finish_program(&run.id, RunStatus::Failed, Some("x".into()), None)
        .unwrap();
    let stored = queue.get(&run.id).unwrap();
    assert_eq!(stored.status, RunStatus::Finished);
    assert_eq!(stored.report, Some(report));
    assert_eq!(finished.lock().unwrap().len(), 1);
    assert!(queue.list().is_empty(), "程序不出现在提醒视图里");
}

#[test]
fn a_refused_or_over_capacity_immediate_start_leaves_no_record() {
    let dir = tempfile::tempdir().unwrap();
    let (queue, launcher, _) = open_with_launcher(dir.path().join("automation-runs.json"));
    for i in 0..MAX_RUNNING_PROGRAMS {
        queue
            .schedule_program("tool", program(&format!("p{i}")), 0)
            .unwrap();
    }
    let busy = queue
        .schedule_program("tool", program("第四个"), 0)
        .unwrap_err();
    assert!(busy.contains("最多同时运行"), "{busy}");
    let first = queue.runs()[0].id.clone();
    queue
        .finish_program(&first, RunStatus::Finished, None, None)
        .unwrap();
    launcher.refuse.store(true, Ordering::SeqCst);
    let refused = queue
        .schedule_program("tool", program("第四个"), 0)
        .unwrap_err();
    assert!(refused.contains("窗口已关闭"));
    assert_eq!(queue.runs().len(), MAX_RUNNING_PROGRAMS);
    // 宿主没接上时不能立即启动
    let bare = AutomationScheduler::open(
        dir.path().join("other.json"),
        Arc::new(RecordingDelivery::default()),
    )
    .unwrap();
    assert!(bare.schedule_program("tool", program("p"), 0).is_err());
    assert!(bare
        .schedule_program("tool", program("p"), now_ms() + 600_000_000_000)
        .is_err());
}

#[test]
fn a_delayed_program_starts_at_its_time_and_never_late() {
    let dir = tempfile::tempdir().unwrap();
    let (queue, launcher, finished) = open_with_launcher(dir.path().join("automation-runs.json"));
    let base = now_ms();
    let on_time = queue
        .schedule_program("companion", program("五分钟后按 Q"), base + 300_000)
        .unwrap();
    let late = queue
        .schedule_program("companion", program("晚到的"), base + 310_000)
        .unwrap();
    assert_eq!(on_time.status, RunStatus::Pending);
    queue.dispatch_programs(base + 299_999).unwrap();
    assert!(launcher.launched.lock().unwrap().is_empty());
    queue.dispatch_programs(base + 300_000).unwrap();
    assert_eq!(*launcher.launched.lock().unwrap(), vec![on_time.id.clone()]);
    // 第二条到期已过去超过 120 秒：错过，不补运行
    queue
        .dispatch_programs(base + 310_000 + MISSED_GRACE_MS + 1)
        .unwrap();
    assert_eq!(launcher.launched.lock().unwrap().len(), 1);
    assert_eq!(queue.get(&late.id).unwrap().status, RunStatus::Missed);
    assert_eq!(finished.lock().unwrap()[0].id, late.id);
}

#[test]
fn a_due_program_fails_instead_of_queueing_when_all_slots_are_busy() {
    let dir = tempfile::tempdir().unwrap();
    let (queue, launcher, _) = open_with_launcher(dir.path().join("automation-runs.json"));
    let base = now_ms();
    let delayed = queue
        .schedule_program("tool", program("稍后"), base + 1_000)
        .unwrap();
    for i in 0..MAX_RUNNING_PROGRAMS {
        queue
            .schedule_program("tool", program(&format!("p{i}")), 0)
            .unwrap();
    }
    queue.dispatch_programs(base + 1_000).unwrap();
    let run = queue.get(&delayed.id).unwrap();
    assert_eq!(run.status, RunStatus::Failed);
    assert!(run.message.unwrap().contains("没有启动"));
    assert_eq!(
        launcher.launched.lock().unwrap().len(),
        MAX_RUNNING_PROGRAMS
    );
    // 到点时宿主拒绝启动（窗口已关闭）：失败并记下原因
    let refused = queue
        .schedule_program("tool", program("窗口没了"), base + 2_000)
        .unwrap();
    for run in queue.runs() {
        if run.status == RunStatus::Running {
            queue
                .finish_program(&run.id, RunStatus::Finished, None, None)
                .unwrap();
        }
    }
    launcher.refuse.store(true, Ordering::SeqCst);
    queue.dispatch_programs(base + 2_000).unwrap();
    let run = queue.get(&refused.id).unwrap();
    assert_eq!(run.status, RunStatus::Failed);
    assert_eq!(run.message.as_deref(), Some("游戏窗口已关闭"));
}

#[test]
fn stop_all_stops_programs_but_keeps_pending_reminders() {
    let dir = tempfile::tempdir().unwrap();
    let (queue, launcher, finished) = open_with_launcher(dir.path().join("automation-runs.json"));
    let base = now_ms();
    let running = queue.schedule_program("tool", program("在跑"), 0).unwrap();
    let pending = queue
        .schedule_program("tool", program("等着"), base + 60_000)
        .unwrap();
    let reminder = queue.schedule_batch("s", base, &[intent(60)]).unwrap();
    assert_eq!(queue.stop_programs("停止自动化热键").unwrap(), 2);
    assert_eq!(
        *launcher.stopped.lock().unwrap(),
        vec![(running.id.clone(), "停止自动化热键".to_string())]
    );
    let pending = queue.get(&pending.id).unwrap();
    assert_eq!(pending.status, RunStatus::Cancelled);
    assert_eq!(pending.message.as_deref(), Some("停止自动化热键"));
    assert_eq!(queue.list()[0].id, reminder[0].id);
    assert_eq!(queue.list()[0].status, ReminderStatus::Pending);
    assert_eq!(finished.lock().unwrap().len(), 1);
    // 运行中的程序由宿主回报结束
    assert_eq!(queue.get(&running.id).unwrap().status, RunStatus::Running);
    assert!(queue.cancel_with_reason(&running.id, "工具").unwrap());
    assert_eq!(launcher.stopped.lock().unwrap().len(), 2);
    assert!(!queue.cancel("run-999").unwrap());
}

#[test]
fn programs_running_or_overdue_at_restart_are_never_rerun() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("automation-runs.json");
    let (queue, _, _) = open_with_launcher(path.clone());
    let running = queue.schedule_program("tool", program("在跑"), 0).unwrap();
    let overdue = queue
        .schedule_program("tool", program("过期"), now_ms() + 1_000)
        .unwrap();
    {
        let mut state = queue.state.lock().unwrap();
        let run = state
            .stored
            .runs
            .iter_mut()
            .find(|r| r.id == overdue.id)
            .unwrap();
        run.due_at_ms -= MISSED_GRACE_MS + 10_000;
        queue.persist(&state.stored).unwrap();
    }
    drop(queue);
    let (queue, launcher, _) = open_with_launcher(path);
    queue.dispatch_programs(now_ms()).unwrap();
    assert!(launcher.launched.lock().unwrap().is_empty());
    let running = queue.get(&running.id).unwrap();
    assert_eq!(running.status, RunStatus::Failed);
    assert!(running.message.unwrap().contains("不会自动重新运行"));
    assert_eq!(queue.get(&overdue.id).unwrap().status, RunStatus::Missed);
}

#[test]
fn pending_legacy_reminders_are_imported_once_and_the_old_file_is_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let legacy = dir.path().join("reminders.json");
    let now = now_ms();
    let reminder = |id: &str, due: u64, status: &str| {
        serde_json::json!({
            "id": id, "sessionId": "assistant-1", "text": "买装备",
            "delivery": ["voice"], "createdAtMs": now - 1_000, "dueAtMs": due,
            "status": status, "error": null,
        })
    };
    let old = serde_json::json!({
        "version": 1, "nextId": 4,
        "reminders": [
            reminder("reminder-1", now + 300_000, "pending"),
            reminder("reminder-2", now - MISSED_GRACE_MS - 1_000, "pending"),
            reminder("reminder-3", now - 1_000, "delivered"),
        ],
    });
    let bytes = serde_json::to_vec(&old).unwrap();
    std::fs::write(&legacy, &bytes).unwrap();
    let path = dir.path().join("automation-runs.json");
    let (queue, _, _) = open_with_launcher(path.clone());
    assert_eq!(queue.import_legacy_reminders(&legacy).unwrap(), 1);
    assert_eq!(queue.list().len(), 1);
    assert_eq!(queue.list()[0].id, "reminder-1");
    assert_eq!(queue.list()[0].text, "买装备");
    // 再导入一次（含重启后）不会重复
    assert_eq!(queue.import_legacy_reminders(&legacy).unwrap(), 0);
    drop(queue);
    let (queue, _, _) = open_with_launcher(path);
    assert_eq!(queue.import_legacy_reminders(&legacy).unwrap(), 0);
    assert_eq!(queue.list().len(), 1);
    assert_eq!(std::fs::read(&legacy).unwrap(), bytes);
    // 新编号与旧编号不冲突
    let added = queue.schedule_batch("s", now, &[intent(1)]).unwrap();
    assert!(added[0].id.starts_with("run-"));
    // 没有旧文件也算导入完成
    let (fresh, _, _) = open_with_launcher(dir.path().join("fresh.json"));
    assert_eq!(
        fresh
            .import_legacy_reminders(&dir.path().join("missing.json"))
            .unwrap(),
        0
    );
    assert!(fresh.state.lock().unwrap().stored.imported_reminders);
}

#[test]
fn run_list_puts_open_runs_first_and_store_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("automation-runs.json");
    let (queue, _, _) = open_with_launcher(path.clone());
    let base = now_ms();
    let done = queue
        .schedule_program("tool", program("完成的"), 0)
        .unwrap();
    queue
        .finish_program(&done.id, RunStatus::Finished, None, None)
        .unwrap();
    let later = queue
        .schedule_program("tool", program("晚"), base + 120_000)
        .unwrap();
    let sooner = queue.schedule_batch("s", base, &[intent(30)]).unwrap();
    let order: Vec<String> = queue.runs().into_iter().map(|r| r.id).collect();
    assert_eq!(order, vec![sooner[0].id.clone(), later.id.clone(), done.id]);
    let json: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(json["version"], 1);
    assert_eq!(json["runs"][1]["action"]["kind"], "program");
    assert_eq!(
        json["runs"][1]["action"]["target"]["label"],
        "游戏 (game.exe)"
    );
    drop(queue);
    let (queue, _, _) = open_with_launcher(path);
    assert_eq!(queue.runs().len(), 3);
}

#[tokio::test]
async fn a_saved_spoken_automation_is_delivered_like_a_reminder() {
    let dir = tempfile::tempdir().unwrap();
    let delivery = Arc::new(RecordingDelivery::default());
    let queue =
        AutomationScheduler::open(dir.path().join("automation-runs.json"), delivery.clone())
            .unwrap();
    let now = now_ms();
    let speak = |text: &str| SpeakAction {
        text: text.into(),
        delivery: vec![DeliveryKind::Danmaku],
    };
    let run = queue.schedule_speak("tool", speak(" 买装备 "), 0).unwrap();
    assert_eq!(run.status, RunStatus::Pending);
    assert!(run.due_at_ms >= now);
    assert_eq!(run.title(), "买装备");
    queue.run_due(now_ms()).await.unwrap();
    assert_eq!(*delivery.0.lock().unwrap(), vec![run.id.clone()]);
    assert!(queue.schedule_speak("tool", speak(""), 0).is_err());
    assert!(queue
        .schedule_speak("tool", speak("x"), now + 8 * 24 * 3_600_000)
        .is_err());
}

#[test]
fn chat_records_are_kept_finished_and_never_scheduled() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("automation-runs.json");
    let (queue, launcher, finished) = open_with_launcher(path.clone());
    let chat = |text: &str| ChatAction {
        text: text.into(),
        sent: true,
        profile: "lol".into(),
    };
    let ok = queue
        .record_chat("agent:c-1", chat(" 我去下路 "), Ok(()))
        .unwrap();
    assert_eq!(ok.status, RunStatus::Finished);
    assert_eq!(ok.title(), "我去下路");
    let failed = queue
        .record_chat("agent:c-1", chat("gg"), Err("打字超时".into()))
        .unwrap();
    assert_eq!(failed.status, RunStatus::Failed);
    assert_eq!(failed.message.as_deref(), Some("打字超时"));
    assert!(queue.record_chat("agent:c-1", chat("  "), Ok(())).is_err());
    assert!(queue
        .record_chat("agent:c-1", chat(&"字".repeat(201)), Ok(()))
        .is_err());
    // 不启动程序、不触发结束回调、不占“进行中”。
    assert!(launcher.launched.lock().unwrap().is_empty());
    assert!(finished.lock().unwrap().is_empty());
    assert_eq!(queue.running_programs(), 0);
    // 重启后还在。
    drop(queue);
    let reopened = AutomationScheduler::open(path, Arc::new(RecordingDelivery::default())).unwrap();
    let runs = reopened.runs();
    assert_eq!(runs.len(), 2);
    assert!(runs
        .iter()
        .any(|r| matches!(&r.action, RunAction::Chat(c) if c.text == "我去下路" && c.sent)));
}
