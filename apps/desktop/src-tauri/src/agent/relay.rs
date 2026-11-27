//! 后台任务经主会话转达：做完的报告、要玩家确认的事、玩家最近一次怎么说的。

use super::*;

/// 后台要开口问玩家前，主会话要连续安静这么久。
const QUIET_BEFORE_ASKING: std::time::Duration = std::time::Duration::from_millis(1_500);

/// 等主会话连续安静 `span`（见 [`main_is_quiet`]）。
async fn wait_quiet(app: &AppHandle, span: std::time::Duration) {
    let mut since: Option<std::time::Instant> = None;
    loop {
        if main_is_quiet(app) {
            let start = *since.get_or_insert_with(std::time::Instant::now);
            if start.elapsed() >= span {
                return;
            }
        } else {
            since = None;
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
}

/// 现在能不能插进主会话说一句：她没在回答、玩家没在说话、没有提问在等回答。
/// 她正忙时插进来的（自动卡片、复盘、自动化的结果，`when_free.rs`）也用它判断什么时候记进对话。
pub(super) fn main_is_quiet(app: &AppHandle) -> bool {
    if is_running(app) || crate::voice_ask::is_pending(app) || crate::assistant::output_busy(app) {
        return false;
    }
    matches!(
        app.state::<crate::SharedState>().orchestrator.state(),
        kotone_core::orchestrator::OrchestratorState::Idle
            | kotone_core::orchestrator::OrchestratorState::Success
            | kotone_core::orchestrator::OrchestratorState::Error
    )
}

/// 取走这段对话还没交回的结果（同时标成已交回、存盘）。没有了就不再排队交回。
pub(super) fn take_unreported(app: &AppHandle, main: &str) -> Vec<Subagent> {
    let (Ok(rt), Ok(store)) = (runtime(app), store(app)) else {
        return Vec::new();
    };
    let taken = {
        let mut inner = rt.inner.lock().unwrap();
        let list = registry(&mut inner, &store, main);
        let taken = kotone_agent::subagents::take_unreported(list);
        if !taken.is_empty() {
            if let Err(e) = store.save_subagents(main, list) {
                log::log(&format!("agent: save subagents failed: {e}"));
            }
        } else {
            inner.reporting.remove(main);
        }
        taken
    };
    for info in &taken {
        emit_subagent(app, main, info);
    }
    taken
}

/// 交回主会话的那条消息（总结按数据附上：它可能转述了网页、屏幕上别人写的话）。
pub(super) fn report_message(reports: &[Subagent]) -> Message {
    use kotone_agent::subagents as sa;
    Message::User {
        text: sa::report_text(reports),
        attachments: reports
            .iter()
            .map(|r| {
                let (label, text) = sa::report_attachment(r);
                Attachment::Data { label, text }
            })
            .collect(),
        source: sa::REPORT_SOURCE.into(),
        at_ms: now_ms(),
        context: String::new(),
    }
}

/// 有结果没交回：等主会话空下来，插一条结果消息并唤醒它说一句（照 OpenCode `<background-result>`）。
/// 主会话已经不是当前对话时只记进它的记录，不调用模型。
pub(super) fn schedule_report(app: &AppHandle, main: &str) {
    let Ok(rt) = runtime(app) else { return };
    if !rt.inner.lock().unwrap().reporting.insert(main.to_string()) {
        return;
    }
    let app = app.clone();
    let main = main.to_string();
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            if !main_is_quiet(&app) {
                continue;
            }
            let reports = take_unreported(&app, &main);
            if reports.is_empty() {
                return;
            }
            let message = report_message(&reports);
            let current = store(&app).ok().and_then(|s| s.current());
            if current.as_deref() == Some(main.as_str()) {
                match send_message(&app, message) {
                    Ok(_) => log::log(&format!(
                        "agent: {} background result(s) handed back to {main}",
                        reports.len()
                    )),
                    Err(e) => log::log(&format!("agent: background result not delivered: {e}")),
                }
            } else {
                append_to(&app, &main, message);
            }
            // 交完了这一批：同一把锁下确认没有新的再退出（见 take_unreported）。
        }
    });
}

/// 程序里的 `ask()`（以前的“让她做”）最多等主会话空下来多久（玩家一直在和她说话就算了：事件已经过去了）。
const PROMPT_WAIT: std::time::Duration = std::time::Duration::from_secs(120);

/// `ask()` 等过 [`PROMPT_WAIT`] 没交出去时，留在聊天里给玩家看的那一句（不发给模型）。
fn prompt_skipped_notice(name: &str) -> Message {
    Message::Notice {
        text: format!(
            "自动化「{name}」要她做的事没做：她一直在忙，等了 {} 分钟，已经过时了",
            PROMPT_WAIT.as_secs() / 60
        ),
        at_ms: now_ms(),
        compaction: None,
        usage: None,
        recap: None,
        card: None,
        push: None,
    }
}

/// `ask()` 交给她的那条消息。来源是 `automation`：界面当系统消息显示；算游戏里的入口（开口说）；不是玩家亲口说的，
/// 这一轮不能交给后台（ADR 030 第 2 节）。触发它的游戏事件按数据附上（里面可能有别人起的名字；话里不能带，
/// `ProgramAction::check_ask` 查过了）。
fn ask_message(name: &str, text: &str, event: Option<&kotone_core::games::GameEvent>) -> Message {
    let why = match event {
        Some(e) => format!(
            "因为游戏事件 {} 触发，请照下面这句做；触发它的事件见附件",
            e.kind
        ),
        None => "被运行，请照下面这句做".to_string(),
    };
    Message::User {
        text: format!("（系统消息，不是玩家说的话：自动化「{name}」{why}。）\n{text}"),
        attachments: event
            .map(|e| Attachment::Data {
                label: "游戏事件".into(),
                text: serde_json::to_string(e).unwrap_or_default(),
            })
            .into_iter()
            .collect(),
        source: "automation".into(),
        at_ms: now_ms(),
        context: String::new(),
    }
}

/// 程序里的 `ask()`（ADR 045 D2，以前的“让她做”，ADR 036 D8）：等主会话空下来，放一条系统消息（[`ask_message`]）进
/// 当前对话，让她跑一轮。阻塞到交出去为止：交出去返回 true；等过 [`PROMPT_WAIT`] 还不空就不交了（事件已经过去了），
/// 在聊天里留一句（照 `when_free.rs` 排着，不悄悄丢；共享资源评估第二节第 9 条），返回 false。等的时候程序被停下
/// （`stopped`）就什么都不发，返回错误（脚本引擎记成“已取消”）。0.1 旧助手没有她：报错。
/// 两道护栏和每局 3 次由调用方（`DesktopHost::ask`）先查。
pub async fn ask_from_automation(
    app: &AppHandle,
    name: &str,
    text: &str,
    event: Option<&kotone_core::games::GameEvent>,
    stopped: impl Fn() -> bool,
) -> Result<bool, String> {
    store(app)?;
    let deadline = std::time::Instant::now() + PROMPT_WAIT;
    loop {
        if stopped() {
            return Err("已停下，没有交给她".into());
        }
        if main_is_quiet(app) {
            break;
        }
        if std::time::Instant::now() >= deadline {
            log::log(&format!(
                "automation {name}: 她一直没空，这次 ask() 没交给她"
            ));
            post_notice(app, prompt_skipped_notice(name));
            return Ok(false);
        }
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    }
    send_message(app, ask_message(name, text, event))?;
    log::log(&format!("automation {name}: 交给她了"));
    Ok(true)
}

/// 只把一条消息记进某段对话（不调用模型）；那段对话恰好在跑就作为插话交给它，她正忙就排着（`when_free.rs`）。
fn append_to(app: &AppHandle, id: &str, message: Message) {
    match post(app, id, &[message], IfBusy::Steer) {
        Ok(Posted::Gone) => log::log(&format!("agent: background result not saved: {id} 没了")),
        Ok(_) => {}
        Err(e) => log::log(&format!("agent: background result not saved: {e}")),
    }
}

/// 后台要玩家确认：经主会话问（玩家最近是用嘴说的就开口问，同时卡片上也能点；打字的就等卡片上点）。
pub(super) async fn subagent_ask(
    app: &AppHandle,
    spec: &ChildSpec,
    child_id: &str,
    question: String,
    cancel: &CancellationToken,
) -> Answer {
    let (tx, rx) = oneshot::channel();
    {
        let Ok(rt) = runtime(app) else {
            return Answer::Deny;
        };
        let mut inner = rt.inner.lock().unwrap();
        let Some(child) = inner.children.get_mut(child_id) else {
            return Answer::Deny;
        };
        child.ask = Some(tx);
    }
    update_subagent(app, &spec.parent, &spec.name, |a| {
        a.status = SubagentStatus::Waiting;
        a.question = Some(question.clone());
    });
    log::log(&format!(
        "agent: background task {} asks the player",
        spec.name
    ));
    let _guard = SubagentAskGuard {
        app: app.clone(),
        spec: spec.clone(),
        child_id: child_id.to_string(),
    };
    let mut rx = rx;
    if last_entry(app, &spec.parent) == Some(Entry::Call) {
        // 问完那几秒没听清，之后用嘴说“可以 / 不要”也算（[`super::answer_by_voice`]）。
        add_spoken_ask(app, SpokenAsk::Child(child_id.to_string()));
        let voice = async {
            // 主会话安静了一会儿再问：她刚说完“交给我”时接着听马上就要开始，抢在那几十毫秒里
            // 问会因为“正在录音”说不出来（2026-11-20 真机）。说不出来就等下一次安静再问。
            for attempt in 0..3 {
                wait_quiet(app, QUIET_BEFORE_ASKING).await;
                match crate::voice_ask::ask(app, &question, 8).await {
                    Ok(Some(text)) => return crate::voice_ask::parse_confirmation(&text),
                    Ok(None) => return None,
                    Err(e) => log::log(&format!(
                        "agent: 后台的确认没能用语音问（第 {} 次）：{e}",
                        attempt + 1
                    )),
                }
            }
            None
        };
        let decided = tokio::select! {
            biased;
            _ = cancel.cancelled() => Some(Answer::Deny),
            answer = &mut rx => Some(answer.unwrap_or(Answer::Deny)),
            heard = voice => heard.map(|ok| if ok { Answer::AllowOnce } else { Answer::Deny }),
        };
        if let Some(answer) = decided {
            return answer;
        }
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            let _ = crate::assistant::deliver_text(
                &app,
                crate::voice_ask::NOT_HEARD.into(),
                vec![DeliveryKind::Voice],
            )
            .await;
        });
    }
    tokio::select! {
        biased;
        _ = cancel.cancelled() => Answer::Deny,
        answer = rx => answer.unwrap_or(Answer::Deny),
    }
}

/// 确认结束（任何方式）：清掉登记，状态回到“在做”。
struct SubagentAskGuard {
    app: AppHandle,
    spec: ChildSpec,
    child_id: String,
}

impl Drop for SubagentAskGuard {
    fn drop(&mut self) {
        remove_spoken_ask(&self.app, &SpokenAsk::Child(self.child_id.clone()));
        if let Ok(rt) = runtime(&self.app) {
            if let Some(child) = rt.inner.lock().unwrap().children.get_mut(&self.child_id) {
                child.ask = None;
            }
        }
        update_subagent(&self.app, &self.spec.parent, &self.spec.name, |a| {
            if a.status == SubagentStatus::Waiting {
                a.status = SubagentStatus::Running;
            }
            a.question = None;
        });
    }
}

/// 玩家最近一次在这段对话里是怎么说的（语音 / 打字）。
pub(super) fn last_entry(app: &AppHandle, main: &str) -> Option<Entry> {
    runtime(app)
        .ok()?
        .inner
        .lock()
        .unwrap()
        .last_entry
        .get(main)
        .copied()
}

pub(super) fn remember_entry(app: &AppHandle, main: &str, entry: Entry) {
    if let Ok(rt) = runtime(app) {
        rt.inner
            .lock()
            .unwrap()
            .last_entry
            .insert(main.to_string(), entry);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::test_util::*;

    /// 后台交回的结果、自动化结果都不是玩家说的话（不能派活、不记成玩家的说话方式）；
    /// 结果交回那一轮的入口跟随玩家最近一次怎么说的（这里只测分类，入口的查表在运行时里）。
    #[test]
    fn background_results_are_not_the_players_words() {
        let report = user(kotone_agent::subagents::REPORT_SOURCE);
        assert!(is_report(&report));
        assert!(!is_player_message(&report));
        for source in [
            "chat",
            "assistant-text",
            "voice",
            "wake-voice",
            "quick-command",
        ] {
            assert!(is_player_message(&user(source)), "{source}");
            assert!(!is_report(&user(source)));
        }
        for source in ["tool", "automation", "heard", "agent-task:3"] {
            assert!(!is_player_message(&user(source)), "{source}");
        }
        assert!(is_task("agent-task:3") && !is_task("chat"));
        assert_eq!(entry_of_source("chat"), Entry::Chat);
        assert_eq!(entry_of_source("voice"), Entry::Call);
        // 交回主会话的消息：说明在正文，总结作为数据附件（可能转述了网页、屏幕上别人写的话）。
        let message = report_message(&[Subagent {
            name: "lol-patch".into(),
            label: "查版本".into(),
            conversation_id: "c-x".into(),
            status: SubagentStatus::Done,
            summary: Some("26.22：三条更新".into()),
            started_at_ms: 0,
            updated_at_ms: 0,
            steps: 3,
            question: None,
            reported: false,
            turn: None,
            taint: None,
            taint_approved: false,
            tool: None,
        }]);
        let Message::User {
            text,
            attachments,
            source,
            ..
        } = message
        else {
            panic!()
        };
        assert_eq!(source, "agent-report");
        assert!(text.contains("<agent-report name=\"lol-patch\""));
        assert!(
            matches!(&attachments[..], [Attachment::Data { text, .. }] if text.contains("26.22"))
        );
    }

    /// 测试清单 20：`ask()` 交给她的那条：来源 automation（不是玩家的话，这一轮不能交给后台）、话在正文、触发它的
    /// 事件整份按数据附上；手动运行的没有附件。
    #[test]
    fn an_ask_goes_to_her_as_an_automation_message_with_the_event_attached() {
        let event: kotone_core::games::GameEvent = serde_json::from_value(json!({
            "id": "e-1", "type": "game.phase_changed", "game": "lol", "match": "m-1", "time": 5,
            "source": "lol/client", "data": {"to": "loading", "summoner": "某人"},
        }))
        .unwrap();
        let Message::User {
            text,
            attachments,
            source,
            ..
        } = ask_message("突破口", "看看对面谁是突破口", Some(&event))
        else {
            panic!()
        };
        assert_eq!(source, "automation");
        assert!(!is_player_message(&user(&source)));
        assert_eq!(
            text,
            "（系统消息，不是玩家说的话：自动化「突破口」因为游戏事件 game.phase_changed 触发，请照下面这句做；\
             触发它的事件见附件。）\n看看对面谁是突破口"
        );
        assert!(
            matches!(&attachments[..], [Attachment::Data { label, text }] if label == "游戏事件" && text.contains("某人"))
        );
        let Message::User {
            text, attachments, ..
        } = ask_message("突破口", "看看", None)
        else {
            panic!()
        };
        assert!(text.contains("「突破口」被运行，请照下面这句做"), "{text}");
        assert!(attachments.is_empty());
    }

    /// `ask()` 过时没交出去：留给玩家看的是一条说明（`Notice` 不发给模型，她不会照着晚做），说清是哪个自动化、为什么。
    #[test]
    fn a_skipped_prompt_leaves_a_line_for_the_player() {
        let Message::Notice {
            text, card, recap, ..
        } = prompt_skipped_notice("进加载播报突破口")
        else {
            panic!("要是 Notice")
        };
        assert_eq!(
            text,
            "自动化「进加载播报突破口」要她做的事没做：她一直在忙，等了 2 分钟，已经过时了"
        );
        assert!(card.is_none() && recap.is_none());
    }
}
