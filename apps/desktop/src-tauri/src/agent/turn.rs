//! 跑一轮对话：收到消息、排队与插话、调用模型、事件转发、自动化结果交回对话。

use super::*;

/// 等待结果回到对话的执行记录最多记几条。
const MAX_RUN_OWNERS: usize = 200;

/// 一条要交给琴音的消息。
pub struct Outgoing {
    pub text: String,
    /// `chat` / `assistant-text`（聊天窗口，回复看文字）；`voice` / `quick-command` / `tool`
    /// （游戏内：快速模式，回复用语音 / 弹幕播出）。
    pub source: String,
    pub attachments: Vec<Attachment>,
}

/// 发一条消息：没有在跑就开始一轮；正在跑就作为插话。返回 `{status, conversationId}`。
pub fn send(app: &AppHandle, outgoing: Outgoing) -> Result<Value, String> {
    let text = outgoing.text.trim().to_string();
    if text.is_empty() {
        return Err("消息为空".into());
    }
    // 游戏里用嘴说的（呼叫键、唤醒、接着听）；聊天页语音按钮的话另算（它进聊天窗口入口）。
    let voice = matches!(outgoing.source.as_str(), "wake-voice" | "voice");
    let spoken = is_spoken_source(&outgoing.source);
    if spoken {
        crate::voice_timing::recognized();
    }
    let name = settings(app).agent.identity.name;
    // 通话里玩家在她说话时开口了（ADR 033 第三节，她的声音这时压低着）：这句识别出来了，
    // 不算打断（空的、只是“嗯”、是她自己的回声）就恢复音量、她接着说；算就停下她，这句照常往下走。
    if voice && crate::voice_reply::ducked_for().is_some() {
        if let Some(reason) = crate::call::not_an_interruption(&text) {
            crate::voice_reply::unduck(reason);
            kotone_core::usage::release_asr(None);
            crate::voice_timing::discard();
            // 识别成功后气泡会显示“在想”：收起（她接着说的下一句会再显示出来）
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(400)).await;
                crate::bubble::finished(&app);
            });
            return Ok(json!({"status": "ignored", "reason": reason}));
        }
        tauri::async_runtime::spawn_blocking(|| {
            if crate::voice_reply::barge_in() {
                log::log("agent: 玩家开口打断了她");
            }
        });
    }
    // 通话里说“退下 / 安静”这类（又不是“不用了”那样的回答）：先挂断，不当成问题的回答（见下面）。
    let exit = voice
        && crate::call::active()
        && crate::follow_up::is_exit_phrase(&text, &name)
        && crate::voice_ask::parse_confirmation(&text).is_none();
    // 琴音刚用语音问了一句、正在等回答：这句语音就是回答，交回那次提问。
    if voice && !exit && crate::voice_ask::deliver_answer(app, &text) {
        kotone_core::usage::release_asr(kotone_core::usage::current_turn());
        return Ok(json!({"status": "answered"}));
    }
    // 问完那几秒已经过了、确认还在等：这句只是“可以 / 不要”就交给它（rc3 真人测试：说了
    // “允许”还卡在确认上，只能去窗口里点）。
    if voice && super::answer_by_voice(app, &text) {
        kotone_core::usage::release_asr(kotone_core::usage::current_turn());
        crate::voice_timing::discard();
        if !is_running(app) {
            // 回答的是后台那件事：主会话没在跑，识别成功后显示的“在想”稍后收起。
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(400)).await;
                crate::bubble::finished(&app);
            });
        }
        return Ok(json!({"status": "answered"}));
    }
    // 语音说“停下”：和停止热键、托盘「停下」一样停下一切（正在进行的操作、这一轮、子会话、
    // 全部自动化，ADR 019 决定 7 / ADR 028），不当成插话排队。
    if voice && is_stop_command(&text, &name) && crate::stop::stop_everything(app, "语音“停下”")
    {
        kotone_core::usage::release_asr(kotone_core::usage::current_turn());
        crate::voice_timing::discard();
        return Ok(json!({"status": "stopped"}));
    }
    // 说“退下 / 安静 / 挂了 / 没事了 / 拜拜”：通话里任何时候都挂断——停掉她正在说的话和这一轮，
    // 交给后台的事不停（ADR 033 第二节，照 orb 的“退下”）；不在通话里只在她没在忙时安静结束
    // （ADR 029 §五；在忙时照常插话）。都不交给模型。
    let in_call = crate::call::active();
    if voice && (in_call || !is_running(app)) && crate::follow_up::is_exit_phrase(&text, &name) {
        kotone_core::usage::release_asr(None);
        crate::voice_timing::discard();
        if in_call {
            crate::call::end(app, "玩家说了退下");
            if is_running(app) {
                let _ = stop(app);
            }
            let app = app.clone();
            tauri::async_runtime::spawn_blocking(move || crate::assistant::stop_speech(app));
        }
        log::log("agent: 玩家说了结束的话，安静结束");
        // 识别成功后气泡会先显示“在想”：稍后收起。
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(400)).await;
            crate::bubble::finished(&app);
        });
        return Ok(json!({"status": "dismissed"}));
    }
    let at_ms = now_ms();
    let result = send_message(
        app,
        Message::User {
            text,
            attachments: outgoing.attachments,
            source: outgoing.source,
            at_ms,
            context: String::new(),
        },
    );
    if spoken {
        // 这句话的语音识别费用记到它进的那一轮（插话也是聊天里单独的一轮）。
        let owner = result.as_ref().ok().and_then(|v| {
            v["conversationId"]
                .as_str()
                .map(|id| kotone_core::usage::Owner {
                    conversation: id.to_string(),
                    turn: Some(at_ms),
                })
        });
        kotone_core::usage::release_asr(owner);
    }
    result
}

/// 把一条消息交给主会话（当前对话）：没有在跑就开始一轮；正在跑就作为插话。
pub fn send_message(app: &AppHandle, message: Message) -> Result<Value, String> {
    let rt = runtime(app)?;
    let store = store(app)?;
    {
        let inner = rt.inner.lock().unwrap();
        if let Some(running) = &inner.running {
            running.inbox.steer(message);
            return Ok(json!({"status": "steered", "conversationId": running.conversation_id}));
        }
    }
    // 子会话的记录只能看：玩家选着它发消息时开一段新对话（子会话由父会话驱动，ADR 022）。
    let conversation = match store.current() {
        Some(id) => store
            .load(&id)?
            .filter(|c| c.parent.is_none())
            .unwrap_or_else(|| store.create()),
        None => store.create(),
    };
    let cancel = CancellationToken::default();
    let inbox = Arc::new(Inbox::default());
    {
        let mut inner = rt.inner.lock().unwrap();
        if let Some(running) = &inner.running {
            running.inbox.steer(message);
            return Ok(json!({"status": "steered", "conversationId": running.conversation_id}));
        }
        inner.running = Some(Running {
            conversation_id: conversation.id.clone(),
            cancel: cancel.clone(),
            inbox: inbox.clone(),
        });
    }
    store.set_current(Some(&conversation.id))?;
    let id = conversation.id.clone();
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        drive(app, conversation, message, cancel, inbox).await;
    });
    Ok(json!({"status": "started", "conversationId": id}))
}

/// 一轮接一轮地跑，直到没有待处理的消息。
async fn drive(
    app: AppHandle,
    mut conversation: Conversation,
    first: Message,
    cancel: CancellationToken,
    inbox: Arc<Inbox>,
) {
    let mut next = Some(first);
    loop {
        while let Some(user) = next.take() {
            let entry = entry_for(&app, &conversation.id, &user);
            let report = is_report(&user);
            if !report && is_player_message(&user) {
                remember_entry(&app, &conversation.id, entry);
                // 后台还有没交回的结果（主会话一直没空、或者 Kotone 重启过）：放在玩家这句话前面一起交给模型。
                inject_pending_reports(&app, &mut conversation);
            }
            let start = conversation.messages.len();
            inbox.take_delivered();
            let owner = turn_owner(&conversation.id, &user);
            let voice_turn = follows_up(&user);
            // 游戏内入口：回答边生成边念（ADR 029 §六）；用不了时为 None，答完整段念。
            let voice = if entry == Entry::Call {
                crate::voice_reply::VoiceTurn::new(&app, &conversation.id, owner.clone())
            } else {
                None
            };
            // 后台的结果交回时不接“我看看”（玩家没在等）。
            let filler =
                (entry == Entry::Call && !report).then(|| progress_filler(&app, &cancel, owner));
            let stop = turn(
                &app,
                &mut conversation,
                user,
                entry,
                &cancel,
                &inbox,
                None,
                voice.as_ref(),
            )
            .await;
            if let Some(filler) = filler {
                filler.cancel();
            }
            if let Some(voice) = &voice {
                voice.close();
            }
            // 聊天窗口发起的一轮里，玩家在游戏里插了话（热键、唤醒）：回答也要播出来，
            // 否则人在游戏里听不到。
            let delivered = inbox.take_delivered();
            let game_steered = delivered.iter().any(|m| entry_of(m) == Entry::Call);
            if entry == Entry::Call || game_steered {
                let follow = (voice_turn || delivered.iter().any(follows_up))
                    && stop == StopReason::Completed;
                finish_voice_turn(
                    &app,
                    &conversation.messages[start.min(conversation.messages.len())..],
                    &stop,
                    follow,
                );
            }
            if stop == StopReason::Cancelled || cancel.is_cancelled() {
                inbox.clear();
                break;
            }
            next = inbox.next_follow_up();
        }
        // 在同一把锁下确认没有新消息再收尾（send 也在这把锁下插话），不会丢消息。
        let Ok(rt) = runtime(&app) else { return };
        let mut inner = rt.inner.lock().unwrap();
        if !cancel.is_cancelled() {
            if let Some(message) = inbox.next_follow_up() {
                drop(inner);
                next = Some(message);
                continue;
            }
        }
        if inner
            .running
            .as_ref()
            .is_some_and(|r| r.conversation_id == conversation.id)
        {
            inner.running = None;
        }
        drop(inner);
        break;
    }
    emit(&app, &conversation.id, json!({"kind": "idle"}));
}

/// 把还没交回的后台结果记进对话（不单独调用模型，玩家这句话的这一轮一起看）。
fn inject_pending_reports(app: &AppHandle, conversation: &mut Conversation) {
    let reports = take_unreported(app, &conversation.id);
    if reports.is_empty() {
        return;
    }
    let message = report_message(&reports);
    conversation.messages.push(message.clone());
    if let Ok(store) = store(app) {
        if let Err(e) = store.save(conversation) {
            log::log(&format!("agent: save failed: {e}"));
        }
    }
    emit(
        app,
        &conversation.id,
        event_json(&AgentEvent::Appended { message }),
    );
}

/// 开始前就失败（没有连接、工具服务未就绪）：把用户的话和原因记进对话。
fn fail_early(
    app: &AppHandle,
    store: &ConversationStore,
    conversation: &mut Conversation,
    user: Message,
    message: String,
) -> StopReason {
    let stop = StopReason::Failed { message };
    for m in [
        user,
        Message::Notice {
            text: stop.notice().unwrap_or_default(),
            at_ms: now_ms(),
            compaction: None,
            usage: None,
            recap: None,
        },
    ] {
        conversation.messages.push(m.clone());
        emit(
            app,
            &conversation.id,
            event_json(&AgentEvent::Appended { message: m }),
        );
    }
    if let Err(e) = store.save(conversation) {
        log::log(&format!("agent: save failed: {e}"));
    }
    emit(
        app,
        &conversation.id,
        event_json(&AgentEvent::Finished { stop: stop.clone() }),
    );
    stop
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn turn(
    app: &AppHandle,
    conversation: &mut Conversation,
    user: Message,
    entry: Entry,
    cancel: &CancellationToken,
    inbox: &Arc<Inbox>,
    child: Option<&ChildSpec>,
    voice: Option<&Arc<crate::voice_reply::VoiceTurn>>,
) -> StopReason {
    let Ok(store) = store(app) else {
        return StopReason::Failed {
            message: "对话存储不可用".into(),
        };
    };
    let settings = settings(app);
    let mut connection = match crate::companion::connection_ready(app, &settings) {
        Ok(c) => c,
        Err(e) => return fail_early(app, &store, conversation, user, e),
    };
    // 后台可以换一个模型（OpenClaw `subagents.model`；空 = 和琴音同一个）。
    if child.is_some() && !settings.agent.subagents.model.trim().is_empty() {
        connection.model = settings.agent.subagents.model.trim().to_string();
    }
    let Some(service) = crate::tool_service::service(app) else {
        return fail_early(
            app,
            &store,
            conversation,
            user,
            "工具服务尚未就绪，请稍后再试".into(),
        );
    };
    let specs = service.capabilities()["tools"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let automations = offered_automations(app, &settings.agent.hidden_automations);
    let catalog = her_catalog(
        &settings,
        &specs,
        &automations,
        &crate::plugins::offers(app),
    );
    // 主会话只拿马上出结果的一小套，后台几乎全给（ADR 029 第二期）；一段对话里清单不变。
    let offered: Vec<String> = catalog
        .entries()
        .iter()
        .filter(|e| match child {
            Some(_) => kotone_agent::catalog::subagent_offers(e),
            None => kotone_agent::catalog::main_offers(e),
        })
        .map(|e| e.name.clone())
        .collect();
    // “你能做什么”按真实能做的事回答（ADR 028）；只给和玩家对话的主会话。
    let capabilities = if child.is_none() {
        capability_overview(&catalog)
    } else {
        String::new()
    };
    // 开着的插件给主会话的几条“工作方式”（ADR 038）
    let plugin_rules = catalog.plugin_rules();
    let session = match runtime(app) {
        Ok(rt) => rt
            .inner
            .lock()
            .unwrap()
            .sessions
            .entry(conversation.id.clone())
            .or_default()
            .clone(),
        Err(_) => Arc::new(Session::default()),
    };
    let mut user = user;
    if entry == Entry::Call && child.is_none() {
        attach_speech_screen(app, &mut user).await;
    }
    let provider = connection.provider.clone();
    let model_id = connection.model.clone();
    let ws = workspace();
    // 玩家这一轮说的原话（只有它能交给后台，ADR 030 第 2 节）：玩家的话进对话时记下。
    let player: PlayerWords = Arc::default();
    // 这一轮从玩家这句话开始读到过外面的东西没有（交给后台时继承，ADR 030 第 3 节）。
    let turn_taint: TurnTaint = Arc::default();
    let port = KotonePort::new(
        service,
        catalog,
        settings.agent.permissions.clone(),
        session,
        DesktopBuiltins {
            voice: entry == Entry::Call && child.is_none(),
            speech: voice.cloned(),
            app: app.clone(),
            session: format!("agent:{}", conversation.id),
            player: player.clone(),
            turn_taint: turn_taint.clone(),
        },
        DesktopAsker {
            app: app.clone(),
            conversation_id: conversation.id.clone(),
            entry,
            child: child.cloned(),
        },
    )
    .with_scope(if entry == Entry::Call && child.is_none() {
        Scope::Game
    } else {
        Scope::Full
    })
    .with_workspace(ws.clone())
    .with_tool_filter(offered);
    let profile = crate::companion::active_profile(&settings);
    let session_state = runtime(app).ok().and_then(|rt| {
        rt.inner
            .lock()
            .unwrap()
            .sessions
            .get(&conversation.id)
            .cloned()
    });
    let snapshot = match &session_state {
        Some(session) => session
            .snapshot
            .get_or_init(|| ws.snapshot(&settings.agent.identity.name, Some(&profile.id)))
            .clone(),
        None => ws.snapshot(&settings.agent.identity.name, Some(&profile.id)),
    };
    // 系统提示在这段对话第一次用时生成、之后不变（照 DeepSeek Harness，前缀缓存才命中）；
    // 每轮会变的（入口、当前游戏、时间）写进这条消息前面的附注。
    let system = {
        let make = || match child {
            Some(spec) => child_prompt(
                &snapshot.name,
                &spec.label,
                &snapshot.skills,
                settings.browser.enabled,
            ),
            None => system_prompt(&PromptContext {
                automations: &automations,
                workspace: Some(&snapshot),
                capabilities: &capabilities,
                plugin_rules: &plugin_rules,
            }),
        };
        match &session_state {
            Some(session) => session.system.get_or_init(make).clone(),
            None => make(),
        }
    };
    if let Message::User { context, .. } = &mut user {
        if context.is_empty() {
            *context = match child {
                None => main_turn_context(
                    app,
                    &settings,
                    entry,
                    &profile,
                    &snapshot,
                    session_state.as_deref(),
                    &ws,
                ),
                Some(_) => child_turn_context(
                    &kotone_core::agent::now_label(),
                    profile.display_name.trim(),
                    &profile.id,
                ),
            };
        }
    }
    let mut model = AgentModel::for_connection(connection);
    // DeepSeek 官方最佳实践（ADR 026）：内容审核、思考强度、会话标题。
    let title_cell: Arc<Mutex<Option<String>>> = Arc::default();
    if let Some(deepseek) = model.deepseek() {
        deepseek.safety_check = settings.agent.safety_check;
        deepseek.reasoning_effort = effort_for(
            entry,
            child.is_some(),
            &settings.agent.game_effort,
            &settings.agent.chat_effort,
        );
        if child.is_none() && conversation.auto_title.is_none() {
            let cell = title_cell.clone();
            deepseek.title = Some(kotone_ai::agent::deepseek::TitleRequest::new(move |t| {
                *cell.lock().unwrap() = Some(t);
            }));
        }
    }
    let limits = match child {
        Some(_) => subagent_limits(settings.agent.compact_at_tokens),
        // 语音入口：只说话（或只交给后台）的一步之后就结束，不再调用模型（ADR 029 第八节）。
        None if entry == Entry::Call => AgentLimits {
            compact_at_tokens: settings.agent.compact_at_tokens,
            stop_after: VOICE_STOP_TOOLS.iter().map(|t| t.to_string()).collect(),
            ..game_limits()
        },
        None => AgentLimits {
            compact_at_tokens: settings.agent.compact_at_tokens,
            ..chat_limits()
        },
    };
    let conversation_id = conversation.id.clone();
    let shared = Mutex::new(conversation.clone());
    let tool_names: Mutex<HashMap<String, String>> = Mutex::default();
    // 语音入口的主会话：计时、工具前的一句马上念（ADR 029 §一、§七）。
    let voice_main = entry == Entry::Call && child.is_none();
    // 后台的费用记到派它出去的那段对话、那一轮（玩家在聊天里看得到这件事花了多少）。
    let record_usage = |kind, usage: &kotone_ai::agent::TokenUsage| match child {
        Some(spec) => {
            crate::usage::record_model_for(kind, &provider, &model_id, &spec.owner, usage)
        }
        None => crate::usage::record_model(kind, &provider, &model_id, &conversation_id, usage),
    };
    let on_event = |event: AgentEvent| {
        if voice_main
            && matches!(
                event,
                AgentEvent::MessageChunk { .. }
                    | AgentEvent::ThoughtChunk { .. }
                    | AgentEvent::ToolCallChunk { .. }
                    | AgentEvent::ToolCall { .. }
                    | AgentEvent::Appended {
                        message: Message::Assistant { .. }
                    }
            )
        {
            crate::voice_timing::first_token();
        }
        // 语音入口：只念 speak 的字，模型一边写参数一边念（ADR 029 第六、八节）。
        if let Some(voice) = voice {
            match &event {
                AgentEvent::StepStarted { .. } => voice.on_step(),
                AgentEvent::ToolCallChunk {
                    index,
                    call_id,
                    name,
                    chunk,
                } => {
                    if voice.on_args(*index, call_id, name, chunk) {
                        // 开口了：这一轮不再接填充话
                        LAST_SAY_MS.store(now_ms(), std::sync::atomic::Ordering::SeqCst);
                    }
                }
                AgentEvent::ToolCall { call } => voice.on_call(&call.id),
                AgentEvent::Discarded { .. } => voice.on_discard(),
                _ => {}
            }
        }
        // 语音入口交给后台、这一步没说话：Kotone 替她说“交给我，好了告诉你”（不等模型）。
        if voice_main {
            if let AgentEvent::Appended {
                message: Message::Assistant { tool_calls, .. },
            } = &event
            {
                if handoff_without_speech(tool_calls) {
                    say_handoff(app);
                }
            }
        }
        match &event {
            // 玩家的话（含插话）进了对话：这之后的读屏、搜索费用记到这一轮；记下原话（能交给后台）
            // 和他是怎么说的（后台的结果、确认照它开口或写字）。
            AgentEvent::Appended {
                message:
                    Message::User {
                        at_ms,
                        source,
                        text,
                        attachments,
                        ..
                    },
            } if child.is_none() => {
                kotone_core::usage::set_turn(Some(kotone_core::usage::Owner {
                    conversation: conversation_id.clone(),
                    turn: Some(*at_ms),
                }));
                if kotone_agent::subagents::is_player_source(source) {
                    *player.lock().unwrap() = Some((text.clone(), *at_ms));
                    remember_entry(app, &conversation_id, entry_of_source(source));
                    // 玩家新说了一句：读没读过外面的东西从这里重新算（附带的截图、数据算外面的；
                    // Kotone 自己写的“开口时的前台窗口”说明不算，否则用嘴派的活每次都要先问）。
                    *turn_taint.lock().unwrap() = attachments
                        .iter()
                        .any(|a| !crate::speech_screen::is_turn_note(a))
                        .then(|| "玩家这句话附带了截图或数据".to_string());
                }
            }
            // ADR 030：读到了外面的东西（不可信的工具结果）。主会话记在这一轮（交给后台时继承），
            // 后台记在它自己的登记里。执行工具的结果不按“不可信”算，看 kotone-exec 给的 taints。
            AgentEvent::Appended {
                message:
                    Message::Tool {
                        name,
                        untrusted: true,
                        ..
                    },
            } if !kotone_agent::exec::is_exec_tool(name) => {
                let why = format!("读了 {name} 的结果（屏幕、聊天、网页这类别人写的内容）");
                match child {
                    None => {
                        turn_taint.lock().unwrap().get_or_insert(why);
                    }
                    Some(spec) => mark_subagent_taint(app, spec, &why),
                }
            }
            // 后台收到的别人说的话（同传听到的一句）或附带的数据。
            AgentEvent::Appended {
                message:
                    Message::User {
                        source,
                        attachments,
                        ..
                    },
            } if child.is_some() && (source == "heard" || !attachments.is_empty()) => {
                if let Some(spec) = child {
                    mark_subagent_taint(app, spec, "收到了别人说的话或附带的数据");
                }
            }
            // 后台每调用一次模型，卡片上的“第 n 步”加一。
            AgentEvent::StepStarted { .. } => {
                if let Some(spec) = child {
                    update_subagent(app, &spec.parent, &spec.name, |a| a.steps += 1);
                }
            }
            _ => {}
        }
        match &event {
            AgentEvent::Appended { message } => {
                let mut c = shared.lock().unwrap();
                c.messages.push(message.clone());
                if let Some(title) = title_cell.lock().unwrap().take() {
                    log::log(&format!("agent: conversation {conversation_id} titled"));
                    c.auto_title = Some(title);
                }
                if let Err(e) = store.save(&mut c) {
                    log::log(&format!("agent: save failed: {e}"));
                }
                if let Message::Assistant {
                    usage: Some(usage), ..
                } = message
                {
                    record_usage(kotone_core::usage::UsageKind::Chat, usage);
                }
            }
            AgentEvent::Compacted { index, message } => {
                let mut c = shared.lock().unwrap();
                let at = (*index).min(c.messages.len());
                c.messages.insert(at, message.clone());
                if let Err(e) = store.save(&mut c) {
                    log::log(&format!("agent: save failed: {e}"));
                }
                if let Message::Notice {
                    usage: Some(usage), ..
                } = message
                {
                    record_usage(kotone_core::usage::UsageKind::Compaction, usage);
                }
                log::log(&format!(
                    "agent: compacted conversation {conversation_id} at {at}"
                ));
            }
            AgentEvent::Discarded { usage } => {
                log::log("agent: empty reply discarded, asking again");
                if let Some(usage) = usage {
                    record_usage(kotone_core::usage::UsageKind::Chat, usage);
                }
            }
            AgentEvent::StepStarted { .. } if entry == Entry::Call => {
                crate::bubble::thinking(app, "在想…");
            }
            AgentEvent::ToolCall { call } => {
                tool_names
                    .lock()
                    .unwrap()
                    .insert(call.id.clone(), call.name.clone());
                let narrated = [
                    kotone_agent::catalog::TOOL_SPEAK,
                    kotone_agent::catalog::TOOL_ASK,
                ];
                if entry == Entry::Call && !narrated.contains(&call.name.as_str()) {
                    crate::bubble::thinking(app, crate::bubble::tool_label(&call.name));
                }
            }
            AgentEvent::ToolCallUpdate {
                call_id,
                status: ToolStatus::Completed,
                content: Some(content),
            } => {
                let name = tool_names.lock().unwrap().get(call_id).cloned();
                if name.is_some_and(|n| n == "automation.run" || n.starts_with(SAVED_PREFIX)) {
                    remember_run(app, content, &conversation_id);
                }
            }
            _ => {}
        }
        // 工具参数的片段只给念 speak 用，不发给界面。
        if !matches!(event, AgentEvent::ToolCallChunk { .. }) {
            emit(app, &conversation_id, event_json(&event));
        }
    };
    let mut history = conversation.messages.clone();
    let stop = run_turn(
        &model,
        &port,
        &mut history,
        Turn {
            system: &system,
            user,
            limits: &limits,
            cancel,
            inbox: Some(inbox),
        },
        &on_event,
    )
    .await;
    if child.is_none() {
        kotone_core::usage::set_turn(None);
    }
    *conversation = shared.into_inner().unwrap();
    stop
}

/// 陪玩热键 / 快捷指令 / 工具入口交给琴音（`agent.inGame`）。立即返回，结果在后台播出。
pub fn submit_from_assistant(
    app: &AppHandle,
    text: String,
    source: &str,
) -> Result<String, String> {
    let result = send(
        app,
        Outgoing {
            text,
            source: source.to_string(),
            attachments: Vec::new(),
        },
    )?;
    Ok(match result["status"].as_str() {
        Some("steered") => "正在处理，已把这句话交给她".into(),
        Some("answered") => "已回答".into(),
        _ => "已交给她".into(),
    })
}

/// `automation.run` / `saved.*` 的结果里有执行记录 id：记下它属于哪段对话。
fn remember_run(app: &AppHandle, content: &str, conversation_id: &str) {
    let Some(id) = serde_json::from_str::<Value>(content)
        .ok()
        .and_then(|v| v.get("id").and_then(Value::as_str).map(str::to_string))
    else {
        return;
    };
    if let Ok(rt) = runtime(app) {
        let mut inner = rt.inner.lock().unwrap();
        if inner.run_owners.len() >= MAX_RUN_OWNERS {
            inner.run_owners.clear();
        }
        inner.run_owners.insert(id, conversation_id.to_string());
    }
}

/// 调度器的结束回调：由对话发起的执行，把结果放回那段对话。
pub fn on_run_finished(app: &AppHandle, run: &Run) {
    let Ok(rt) = runtime(app) else { return };
    let Some(conversation_id) = rt.inner.lock().unwrap().run_owners.remove(&run.id) else {
        return;
    };
    let status = match run.status {
        RunStatus::Finished => "已完成",
        RunStatus::Failed => "失败",
        RunStatus::Cancelled => "已停止",
        RunStatus::Missed => "错过了",
        RunStatus::Pending | RunStatus::Running => return,
    };
    let message = Message::User {
        text: format!(
            "（系统消息，不是玩家说的话：自动化「{}」{status}。执行记录见附件；需要的话告诉玩家结果。）",
            run.title()
        ),
        attachments: vec![Attachment::Data {
            label: "执行记录".into(),
            text: crate::automation::run_json(run, false).to_string(),
        }],
        source: "automation".into(),
        at_ms: now_ms(),
        context: String::new(),
    };
    {
        let inner = rt.inner.lock().unwrap();
        // 主会话或后台的那件事还在做：作为插话交给它。
        let running = inner
            .running
            .as_ref()
            .filter(|r| r.conversation_id == conversation_id)
            .or_else(|| {
                inner
                    .children
                    .get(&conversation_id)
                    .and_then(|c| c.running.as_ref())
            });
        if let Some(running) = running {
            running.inbox.steer(message);
            return;
        }
    }
    // 没在跑：只记进对话，不自动调用模型（不花钱）；玩家下次说话时模型能看到。
    let Ok(store) = store(app) else { return };
    match store.load(&conversation_id) {
        Ok(Some(mut conversation)) => {
            conversation.messages.push(message.clone());
            if let Err(e) = store.save(&mut conversation) {
                log::log(&format!("agent: save failed: {e}"));
            }
            emit(
                app,
                &conversation_id,
                event_json(&AgentEvent::Appended { message }),
            );
        }
        Ok(None) => {}
        Err(e) => log::log(&format!("agent: load failed: {e}")),
    }
}

// ---------- 内置动作与询问 ----------
