//! 工具宿主：桌面端执行的内置工具，以及权限询问（界面卡片 / 语音追问）。

use super::*;

pub(super) struct DesktopBuiltins {
    pub(super) app: AppHandle,
    /// 语音入口的一轮：说的话走对话气泡。
    pub(super) voice: bool,
    /// 语音入口边生成边念 speak 的那一路（流式语音用不了时为 None，整段念）。
    pub(super) speech: Option<Arc<crate::voice_reply::VoiceTurn>>,
    /// 写进执行记录的发起方：`agent:<对话 id>`。
    pub(super) session: String,
    /// 玩家这一轮说的原话（交给后台时附上）。
    pub(super) player: PlayerWords,
    /// 这一轮读到过外面的东西没有（交给后台时继承，ADR 030）。
    pub(super) turn_taint: TurnTaint,
    /// 语音入口的主会话：这一步替她说过“交给我”没有（慢的工具转到后台时看，ADR 029 §一）。
    pub(super) handoff: Option<Arc<Mutex<kotone_agent::background::HandoffLine>>>,
}

impl DesktopBuiltins {
    fn conversation_id(&self) -> String {
        self.session.trim_start_matches("agent:").to_string()
    }
}

/// 真机测试用（只在调试版）：`KOTONE_TEST_SLOW_TOOL=web.search:5000` 让这个内置工具先等这么多毫秒，
/// 测“主会话里超过 3 秒没回来就转到后台”（ADR 029 §一；`scripts/manual-e2e/slow_tool_voice_real.py`）。
#[cfg(debug_assertions)]
async fn test_slow_tool(tool: &str) {
    let Ok(spec) = std::env::var("KOTONE_TEST_SLOW_TOOL") else {
        return;
    };
    let Some((name, ms)) = spec.split_once(':') else {
        return;
    };
    if let (true, Ok(ms)) = (name.trim() == tool, ms.trim().parse::<u64>()) {
        log::log(&format!("agent: test hook: {tool} waits {ms} ms first"));
        tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
    }
}

impl BuiltinHost for DesktopBuiltins {
    async fn builtin(&self, tool: &str, arguments: &Value) -> Result<Value, String> {
        #[cfg(debug_assertions)]
        test_slow_tool(tool).await;
        let caller = self.conversation_id();
        match tool {
            kotone_agent::catalog::TOOL_AGENT_START => {
                let inherited = self.turn_taint.lock().unwrap().clone();
                agent_start(
                    &self.app,
                    &caller,
                    &self.player,
                    inherited,
                    self.voice,
                    arguments,
                )
                .await
            }
            kotone_agent::catalog::TOOL_AGENT_FOLLOWUP => {
                let inherited = self.turn_taint.lock().unwrap().clone();
                agent_followup(
                    &self.app,
                    &caller,
                    &self.player,
                    inherited,
                    self.voice,
                    arguments,
                )
                .await
            }
            kotone_agent::catalog::TOOL_AGENT_LIST => agent_list(&self.app, &caller),
            kotone_agent::catalog::TOOL_AGENT_STOP => {
                let name = str_arg(arguments, "name");
                if !subagents_of(&self.app, &caller)
                    .iter()
                    .any(|a| a.name == name)
                {
                    return Err(format!("后台没有叫 {name} 的事（agent.list 看看）"));
                }
                Ok(json!({"stopped": stop_subagent(&self.app, &caller, name, "主会话停掉了它")}))
            }
            "connections.request_key" => {
                crate::key_request::request(&self.app, &caller, self.voice, arguments).await
            }
            "voice.listen.start" => crate::voice_listen::start(&self.app, &caller, arguments),
            "voice.listen.stop" => {
                Ok(json!({"stopped": crate::voice_listen::stop_owner(&self.app, &caller)}))
            }
            "web.search" => crate::web_tools::search(&self.app, &caller, arguments).await,
            kotone_agent::catalog::TOOL_SCREEN_AT_SPEECH => {
                crate::speech_screen::tool(&self.app).await
            }
            "web.fetch" => crate::web_tools::fetch(&self.app, &caller, arguments).await,
            other => Err(format!("{other} 在这里不可用")),
        }
    }

    /// 插件的工具（ADR 038）：交给插件执行。
    async fn plugin(
        &self,
        tool: &str,
        arguments: &Value,
    ) -> Result<kotone_plugin_api::ToolResult, String> {
        crate::plugins::call(&self.app, tool, arguments, crate::plugins::Caller::Her).await
    }

    /// 本体用 ToolSpec 登记的工具（ADR 039 D2：`game_chat.send`）：执行记录记在这段对话下；
    /// 截止时间用入口自己的默认值（ADR 041）。
    async fn body_tool(
        &self,
        tool: &str,
        arguments: &Value,
    ) -> Result<kotone_plugin_api::ToolResult, String> {
        crate::game_chat::body_tool(
            &self.app,
            tool,
            arguments,
            &self.session,
            kotone_core::send_to_game::Speaker::Companion,
            None,
        )
        .await
    }

    async fn exec(
        &self,
        _call_id: &str,
        tool: &str,
        arguments: &Value,
        confirm: bool,
        cancel: &CancellationToken,
    ) -> ToolOutcome {
        // 浏览器（ADR 031）和执行命令、读写文件（ADR 030）都只给后台，都经这里。
        if kotone_agent::browser::is_browser_tool(tool) {
            return browser_tool(
                &self.app,
                &self.conversation_id(),
                tool,
                arguments,
                confirm,
                cancel,
            )
            .await;
        }
        exec_tool(
            &self.app,
            &self.conversation_id(),
            tool,
            arguments,
            confirm,
            cancel,
        )
        .await
    }

    fn preapproved(&self, tool: &str, _arguments: &Value) -> bool {
        tool == "game.act" && crate::computer_use::always_allowed(&self.app)
    }

    /// 主会话里一个工具 3 秒没回来（ADR 029 §一）：登记成一件后台的事（卡片），语音回合这一步没说话就
    /// 替她说“交给我，好了告诉你”。
    fn move_to_background(
        &self,
        moved: &kotone_agent::background::Moved,
    ) -> Result<String, String> {
        let inherited = self.turn_taint.lock().unwrap().clone();
        let name = move_tool(
            &self.app,
            &self.conversation_id(),
            &self.player,
            inherited,
            moved,
        )?;
        if self
            .handoff
            .as_ref()
            .is_some_and(|line| line.lock().unwrap().on_moved())
        {
            say_handoff(&self.app);
        }
        Ok(name)
    }

    fn background_finished(
        &self,
        name: &str,
        moved: &kotone_agent::background::Moved,
        outcome: ToolOutcome,
    ) {
        finish_moved(&self.app, &self.conversation_id(), name, moved, outcome);
    }

    async fn ask(&self, question: String, wait_seconds: u64) -> Result<Value, String> {
        LAST_SAY_MS.store(now_ms(), std::sync::atomic::Ordering::SeqCst);
        match crate::voice_ask::ask(&self.app, &question, wait_seconds).await? {
            Some(answer) => Ok(json!({"answered": true, "answer": answer})),
            None => Ok(json!({"answered": false, "note": "玩家没有回答"})),
        }
    }

    async fn speak(&self, call_id: &str, text: String) -> Result<Value, String> {
        LAST_SAY_MS.store(now_ms(), std::sync::atomic::Ordering::SeqCst);
        // 合成费用记到这一轮。
        let owner = kotone_core::usage::current_turn()
            .filter(|t| t.conversation == self.conversation_id())
            .unwrap_or(kotone_core::usage::Owner {
                conversation: self.conversation_id(),
                turn: None,
            });
        // 已经边生成边念了：补上最后的字，等念完（或被打断）。
        if let Some(reply) = self.speech.as_ref().and_then(|v| v.take(call_id, &text)) {
            return crate::voice_reply::spoken_result(&reply.finish(&text).await);
        }
        // 整段念（流式语音用不了，或别的服务商不流式给参数）。
        match deliver_voice_reply(&self.app, text, Some(owner), "speak").await {
            Ok(_) => crate::voice_reply::spoken_result(&crate::voice_reply::Spoken::Done),
            Err(e) if e.contains("打断") => crate::voice_reply::spoken_result(
                &crate::voice_reply::Spoken::Interrupted(String::new()),
            ),
            Err(e) => Err(e),
        }
    }

    async fn danmaku(&self, text: String) -> Result<Value, String> {
        crate::assistant::deliver_text(&self.app, text, vec![DeliveryKind::Danmaku]).await?;
        Ok(json!({"shown": true}))
    }

    async fn remind(
        &self,
        delay_seconds: u64,
        text: String,
        delivery: Vec<DeliveryKind>,
    ) -> Result<Value, String> {
        crate::automation::schedule_reminder(
            &self.app,
            &self.session,
            text,
            delivery,
            delay_seconds,
        )
    }
}

pub(super) struct DesktopAsker {
    pub(super) app: AppHandle,
    pub(super) conversation_id: String,
    pub(super) entry: Entry,
    /// 后台的一件事：不自己弹窗，经主会话问玩家（ADR 029 第三节）。
    pub(super) child: Option<ChildSpec>,
}

impl Asker for DesktopAsker {
    async fn ask(&self, request: PermissionRequest, cancel: &CancellationToken) -> Answer {
        if let Some(spec) = &self.child {
            let question = kotone_agent::subagents::question(&spec.label, &request.reason);
            return subagent_ask(&self.app, spec, &self.conversation_id, question, cancel).await;
        }
        let (tx, rx) = oneshot::channel();
        let ask_id = {
            let Ok(rt) = runtime(&self.app) else {
                return Answer::Deny;
            };
            let mut inner = rt.inner.lock().unwrap();
            inner.next_ask += 1;
            let id = format!("ask-{}", inner.next_ask);
            inner.asks.insert(id.clone(), tx);
            id
        };
        let reason = request.reason.clone();
        let _ = self.app.emit(
            ASK_EVENT,
            json!({"askId": ask_id, "conversationId": self.conversation_id, "request": request}),
        );
        // 这一轮被停止或超时时，循环会直接丢掉这个 future：由守卫在任何结束方式下
        // 清掉登记并通知界面收起按钮。
        let mut guard = AskGuard {
            app: self.app.clone(),
            ask_id,
            conversation_id: self.conversation_id.clone(),
            answer: Answer::Deny,
        };
        let mut rx = rx;
        if self.entry == Entry::Call {
            // 语音入口：先用语音问，听玩家说“好 / 不要”；没听清时之后用嘴说“可以 / 不要”也算
            // （[`super::answer_by_voice`]），窗口按钮一直可用。
            add_spoken_ask(&self.app, SpokenAsk::Main(guard.ask_id.clone()));
            let question = crate::voice_ask::confirmation_question(&reason);
            let voice = async {
                match crate::voice_ask::ask(&self.app, &question, 6).await {
                    Ok(Some(text)) => crate::voice_ask::parse_confirmation(&text),
                    Ok(None) => None,
                    Err(e) => {
                        log::log(&format!("agent: 语音确认没能进行：{e}"));
                        None
                    }
                }
            };
            let decided = tokio::select! {
                biased;
                _ = cancel.cancelled() => Some(Answer::Deny),
                answer = &mut rx => Some(answer.unwrap_or(Answer::Deny)),
                heard = voice => heard.map(|ok| if ok { Answer::AllowOnce } else { Answer::Deny }),
            };
            if let Some(answer) = decided {
                guard.answer = answer;
                return guard.answer;
            }
            let app = self.app.clone();
            tauri::async_runtime::spawn(async move {
                let _ = crate::assistant::deliver_text(
                    &app,
                    crate::voice_ask::NOT_HEARD.into(),
                    vec![DeliveryKind::Voice],
                )
                .await;
            });
        }
        guard.answer = tokio::select! {
            biased;
            _ = cancel.cancelled() => Answer::Deny,
            answer = rx => answer.unwrap_or(Answer::Deny),
        };
        guard.answer
    }
}

struct AskGuard {
    app: AppHandle,
    ask_id: String,
    conversation_id: String,
    answer: Answer,
}

impl Drop for AskGuard {
    fn drop(&mut self) {
        if let Ok(rt) = runtime(&self.app) {
            let mut inner = rt.inner.lock().unwrap();
            inner.asks.remove(&self.ask_id);
            let spoken = SpokenAsk::Main(self.ask_id.clone());
            inner.spoken_asks.retain(|a| *a != spoken);
        }
        let _ = self.app.emit(
            ASK_RESOLVED_EVENT,
            json!({"askId": self.ask_id, "conversationId": self.conversation_id, "answer": self.answer}),
        );
    }
}
