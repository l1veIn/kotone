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
}

impl DesktopBuiltins {
    fn conversation_id(&self) -> String {
        self.session.trim_start_matches("agent:").to_string()
    }
}

impl BuiltinHost for DesktopBuiltins {
    async fn builtin(&self, tool: &str, arguments: &Value) -> Result<Value, String> {
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
            // Kotone 浏览器里在放的歌（ADR 031，主会话马上做）
            kotone_agent::browser::TOOL_MUSIC => {
                let action = str_arg(arguments, "action");
                crate::browser::music(&self.app, action).await
            }
            other => Err(format!("{other} 在这里不可用")),
        }
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

    async fn send_to_game_chat(&self, text: String, send: Option<bool>) -> Result<Value, String> {
        // 前台是 Kotone 自己（例如玩家正在聊天窗口里）时不能打字，否则字会打进 Kotone。
        let (_, window) = crate::automation::foreground_target_info()
            .map_err(|_| "前台不是游戏窗口：请先切到游戏，再让我替你发言".to_string())?;
        let settings = settings(&self.app);
        // 前台也得是这个游戏：不能把字和回车打进桌面、文件夹或别的程序（2026-11-18 真机测试里
        // 玩家说“跟队友说…”，她选了打字，而前台是桌面）。
        let game = crate::companion::active_profile(&settings);
        if let Some(reason) = foreground_refusal(&game, window.process_name.as_deref()) {
            return Err(reason);
        }
        let send = send.unwrap_or(settings.assistant.companion.auto_send);
        let profile = crate::companion::active_profile(&settings).id;
        let result = crate::companion::type_text(&self.app, &text, send).await;
        let record = crate::automation::record_chat(
            &self.app,
            &self.session,
            ChatAction {
                text,
                sent: send && result.is_ok(),
                profile,
            },
            result.as_ref().map(|_| ()).map_err(Clone::clone),
        );
        if let Err(e) = &record {
            log::log(&format!("agent: chat record failed: {e}"));
        }
        let mut value = result?;
        if let Ok(run) = record {
            value["runId"] = json!(run.id);
        }
        Ok(value)
    }
}

/// 前台不是这个游戏时不打字的原因（None = 可以打）。
fn foreground_refusal(
    game: &kotone_core::profile::GameProfile,
    process: Option<&str>,
) -> Option<String> {
    let process = process.unwrap_or("");
    if process.eq_ignore_ascii_case("explorer.exe") || process.is_empty() {
        return Some("前台是桌面或文件夹，不是游戏：请先切到游戏，再让我替你发言".into());
    }
    if !kotone_core::profile::matches_process(game, process) {
        return Some(format!(
            "前台是 {process}，不是「{}」：请先切到游戏，再让我替你发言",
            game.display_name
        ));
    }
    None
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn game_chat_is_only_typed_into_the_game() {
        let lol = kotone_core::profile::GameProfile::builtin_lol();
        assert!(foreground_refusal(&lol, Some("explorer.exe")).is_some());
        assert!(foreground_refusal(&lol, None).is_some());
        assert!(foreground_refusal(&lol, Some("notepad.exe")).is_some());
        let game = lol.process_names[0].clone();
        assert!(foreground_refusal(&lol, Some(&game)).is_none());
        // 没写进程名的通用配置：除了桌面，前台是什么就算什么
        let generic = kotone_core::profile::GameProfile::builtin_generic();
        assert!(foreground_refusal(&generic, Some("SomeGame.exe")).is_none());
        assert!(foreground_refusal(&generic, Some("Explorer.EXE")).is_some());
    }
}
