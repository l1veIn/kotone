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
fn main_is_quiet(app: &AppHandle) -> bool {
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

/// 只把一条消息记进某段对话（不调用模型）。
fn append_to(app: &AppHandle, id: &str, message: Message) {
    let Ok(store) = store(app) else { return };
    match store.load(id) {
        Ok(Some(mut conversation)) => {
            conversation.messages.push(message.clone());
            if let Err(e) = store.save(&mut conversation) {
                log::log(&format!("agent: save failed: {e}"));
            }
            emit(app, id, event_json(&AgentEvent::Appended { message }));
        }
        Ok(None) => {}
        Err(e) => log::log(&format!("agent: load failed: {e}")),
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
}
