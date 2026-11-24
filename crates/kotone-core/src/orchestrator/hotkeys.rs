use super::*;

impl Orchestrator {
    // ---------- 热键入口（hotkey 模块调用；ADR-006 单键语义表 + 策略路由） ----------

    /// 进行中的会话使用开始时的策略；热更新不偷换一次按住/松开配对的语义。
    pub(super) fn policy(&self) -> InteractionPolicy {
        let inner = self.inner.lock().unwrap();
        if let Some(policy) = inner
            .session_policy
            .filter(|_| inner.state != OrchestratorState::Idle)
        {
            return policy;
        }
        let purpose = inner.purpose;
        drop(inner);
        let settings = self.settings.read().unwrap();
        Self::configured_policy(&settings, purpose)
    }

    pub(super) fn configured_policy(
        settings: &Settings,
        purpose: SessionPurpose,
    ) -> InteractionPolicy {
        match purpose {
            SessionPurpose::Dictation => InteractionPolicy::from_settings(settings),
            SessionPurpose::Assistant => {
                InteractionPolicy::from_preset(settings.assistant.interaction_mode.into())
            }
        }
    }

    pub(super) fn hotkey_session_snapshot(&self) -> (OrchestratorState, SessionPurpose, u64, bool) {
        let inner = self.inner.lock().unwrap();
        (
            inner.state,
            inner.purpose,
            inner.gen,
            inner
                .process_case_id
                .as_deref()
                .is_some_and(is_legacy_reminder_session),
        )
    }

    fn is_legacy_assistant_hotkey(&self, purpose: SessionPurpose) -> bool {
        purpose == SessionPurpose::Assistant
            && self
                .settings
                .read()
                .unwrap()
                .ui
                .legacy_upgrade
                .has_legacy_reminders()
    }

    /// toggle 触发键（A2 点按事件）按状态路由：
    /// Idle 开始 / Listening 结束（B2；B3 接入后兜底强制结束；solo 连续模式 = 停止会话，
    /// 不发送在途段）/ Preview 确认发送（C2 的主交互，全程不碰鼠标不抢焦点）/ 其余状态中止。
    pub async fn on_hotkey_toggle(&self) {
        self.on_hotkey_toggle_with_ticket(self.hotkey_input_ticket())
            .await;
    }

    pub async fn on_hotkey_toggle_with_ticket(&self, ticket: u64) {
        self.on_purpose_hotkey_toggle(SessionPurpose::Dictation, ticket)
            .await;
    }

    pub(super) async fn on_purpose_hotkey_toggle(&self, purpose: SessionPurpose, ticket: u64) {
        if !self.input_ticket_is_current(Some(ticket)) {
            return;
        }
        // 另一用途的热键只能取消旧会话，不结束它、确认它或把结果交给另一条管线。
        let (state, active_purpose, gen, legacy_reminder) = self.hotkey_session_snapshot();
        if self.inner.lock().unwrap().assistant_voice_owner
            && !matches!(
                state,
                OrchestratorState::Idle | OrchestratorState::Success | OrchestratorState::Error
            )
        {
            return;
        }
        if state != OrchestratorState::Idle
            && (active_purpose != purpose
                || legacy_reminder != self.is_legacy_assistant_hotkey(purpose))
        {
            self.cancel_inner(Some(ticket), Some(gen)).await;
            return;
        }
        match state {
            OrchestratorState::Idle => {
                let _ = self
                    .begin_inner(false, purpose, Some(gen), Some(ticket))
                    .await;
            }
            OrchestratorState::Listening => {
                // B1（松手结束）策略下不会产生 toggle 事件；B2/B3 均为按下结束
                let policy = self.policy();
                if self.continuous_session() {
                    // solo：再点按 = 停止会话（发送完成不停机由 schedule_idle 负责；
                    // 这里丢弃在途段——用户的意图是「停」，不是「把这句发出去」）
                    self.cancel_inner(Some(ticket), Some(gen)).await;
                } else if policy.end != EndTrigger::HotkeyRelease {
                    let _ = self.end_inner(Some(gen), Some(ticket)).await;
                }
            }
            OrchestratorState::Preview => {
                let _ = self.confirm_send_inner(Some(ticket), Some(gen)).await;
            }
            // Success / 无文本 Error 的 toast 驻留期：点按 = 立即开始下一句
            // （先 cancel 清理 toast 再 begin，「第一次按没反应」修复）。
            // 带文本的 Error（发送失败待重试）保持只取消，不吞掉重试入口。
            OrchestratorState::Success => {
                self.cancel_inner(Some(ticket), Some(gen)).await;
                let _ = self
                    .begin_inner(false, purpose, Some(gen + 1), Some(ticket))
                    .await;
            }
            OrchestratorState::Error if self.inner.lock().unwrap().pending_text.is_none() => {
                self.cancel_inner(Some(ticket), Some(gen)).await;
                let _ = self
                    .begin_inner(false, purpose, Some(gen + 1), Some(ticket))
                    .await;
            }
            _ => self.cancel_inner(Some(ticket), Some(gen)).await,
        }
    }

    /// hold 触发键（A1）：按下开始。Idle 直接 begin；Success / 无文本 Error 的
    /// toast 驻留期内按下 = 立即开始下一句（先 cancel 再 begin，「第一次按没反应」
    /// 修复）；带文本的 Error（发送失败待重试）按下 = 重试发送——hold 模式没有
    /// 独立的确认键，悬浮窗重试按钮要动鼠标，不修复的话用户只能重启 runtime
    /// （0.1.5 用户反馈：注入失败后按热键全部无响应）；Preview 保留确认入口。
    /// 松开在 Listening 态结束（B1）。
    pub async fn on_hotkey_hold(&self, pressed: bool) {
        self.on_hotkey_hold_with_ticket(pressed, self.hotkey_input_ticket())
            .await;
    }

    pub async fn on_hotkey_hold_with_ticket(&self, pressed: bool, ticket: u64) {
        self.on_purpose_hotkey_hold(SessionPurpose::Dictation, pressed, ticket)
            .await;
    }

    pub(super) async fn on_purpose_hotkey_hold(
        &self,
        purpose: SessionPurpose,
        pressed: bool,
        ticket: u64,
    ) {
        let _input = self.hold_input.lock().await;
        if !self.input_ticket_is_current(Some(ticket)) {
            return;
        }
        let (state, active_purpose, gen, legacy_reminder) = self.hotkey_session_snapshot();
        if self.inner.lock().unwrap().assistant_voice_owner
            && !matches!(
                state,
                OrchestratorState::Idle | OrchestratorState::Success | OrchestratorState::Error
            )
        {
            return;
        }
        if state != OrchestratorState::Idle
            && (active_purpose != purpose
                || legacy_reminder != self.is_legacy_assistant_hotkey(purpose))
        {
            if pressed {
                self.cancel_inner(Some(ticket), Some(gen)).await;
            }
            return;
        }
        if pressed {
            // 宽限期内再次按下 = 收回「结束」，当前这句继续说（对讲机直觉）
            {
                let mut inner = self.inner.lock().unwrap();
                if inner.gen != gen || !self.input_ticket_is_current(Some(ticket)) {
                    return;
                }
                if let Some(task) = inner.release_grace_task.take() {
                    task.abort();
                }
            }
            match state {
                OrchestratorState::Idle => {
                    let _ = self
                        .begin_inner(false, purpose, Some(gen), Some(ticket))
                        .await;
                }
                OrchestratorState::Success => {
                    self.cancel_inner(Some(ticket), Some(gen)).await;
                    let _ = self
                        .begin_inner(false, purpose, Some(gen + 1), Some(ticket))
                        .await;
                }
                OrchestratorState::Error => {
                    if self.inner.lock().unwrap().pending_text.is_some() {
                        // 带文本 Error = 发送失败待重试：按下即重发（confirm_send
                        // 从 Error 态重新进入 Sending）
                        let _ = self.confirm_send_inner(Some(ticket), Some(gen)).await;
                    } else {
                        self.cancel_inner(Some(ticket), Some(gen)).await;
                        let _ = self
                            .begin_inner(false, purpose, Some(gen + 1), Some(ticket))
                            .await;
                    }
                }
                _ => {}
            }
        } else if self.policy().end == EndTrigger::HotkeyRelease
            && self.state() == OrchestratorState::Listening
        {
            // 松手 ≠ 立刻结束：进入宽限期继续收音，倒计时到点才 finalize
            // （句尾音素与传输链路上的在途音频不再被切掉，见 DEFAULT_RELEASE_GRACE）
            self.schedule_release_end(gen, ticket);
        }
    }

    /// 松手宽限期编排：延迟 release_grace 后结束会话；期间 gen/状态变化则作废。
    /// 宽限期内 pump 照常把 PCM 灌进 session——多录的是环境尾音，
    /// 与录音笔模式人类反应延迟天然产生的尾音等效。
    pub(super) fn schedule_release_end(&self, gen: u64, ticket: u64) {
        // 防御：理论上一句只松手一次，重复松手（双通道事件）以前一个倒计时为准
        let mut inner = self.inner.lock().unwrap();
        if inner.gen != gen
            || inner.state != OrchestratorState::Listening
            || !self.input_ticket_is_current(Some(ticket))
            || inner.release_grace_task.is_some()
        {
            return;
        }
        let grace = self.release_grace;
        let me_weak = self.me.read().unwrap().clone();
        let handle = tokio::spawn(async move {
            tokio::time::sleep(grace).await;
            let Some(me) = me_weak.as_ref().and_then(|w| w.upgrade()) else {
                return;
            };
            // 保留任务句柄直到原子进入 Transcribing，等待 op 期间重按仍可 abort。
            let _ = me.end_inner(Some(gen), Some(ticket)).await;
        });
        inner.release_grace_task = Some(handle);
    }

    /// 收回宽限期（再次按下/取消/停止 runtime 时调用；无待结束任务则无事发生）
    pub(super) fn abort_release_grace(&self) {
        if let Some(t) = self.inner.lock().unwrap().release_grace_task.take() {
            t.abort();
        }
    }

    // ---------- 频道切换（ADR-008：sticky 频道态，句间动作） ----------

    /// 频道切换键：按声明顺序循环当前激活 profile 的频道，悬浮窗提示新频道。
    /// sticky：切换后保持到下次切换；频道在发送时刻读取，因此 toggle/one-shot/
    /// solo 模式会话进行中按下自然作用于当前这句（hold 模式该组合物理上按不出）。
    /// 单频道 profile 无操作。任意状态都接受（频道与会话状态正交）。
    pub async fn on_cycle_channel(&self) {
        let settings = self.settings.read().unwrap().clone();
        let profile = settings
            .active_profile_id
            .as_deref()
            .and_then(profile::get)
            .unwrap_or_else(GameProfile::builtin_generic);
        let current = { self.inner.lock().unwrap().channel.clone() };
        // profile 不符（切了游戏适配）的残留视为「位于默认频道」
        let current_id = current
            .filter(|(pid, _)| *pid == profile.id)
            .map(|(_, cid)| cid);
        let Some(next_id) = profile.next_channel_id(current_id.as_deref()) else {
            return; // 单频道：切换键无操作
        };
        let strategy = profile.channel_strategy(Some(&next_id));
        {
            self.inner.lock().unwrap().channel = Some((profile.id.clone(), next_id.clone()));
        }
        self.emitter.emit(
            "kotone://channel",
            json!({
                "channelId": strategy.id,
                "displayName": strategy.display_name,
                "isDefault": strategy.is_default,
            }),
        );
        self.emit_process(
            "channel_switched",
            json!({ "channelId": strategy.id, "profileId": profile.id }),
        );
    }

    // ---------- 文字处理热键（开关 / 切换流程） ----------

    /// 文字处理启用开关热键：切换 `postProcessing.enabled` 并持久化，悬浮窗显示状态。
    /// 与设置页改配置一致，下一句识别按快照生效；任意状态都可切换（与会话正交）。
    pub async fn on_post_processing_toggle(&self) {
        let Some(repo) = self.settings_repository.clone() else {
            crate::log::log("post-processing toggle: 未注入配置仓库，忽略");
            return;
        };
        let next = match repo.update(|settings| {
            settings.post_processing.enabled = !settings.post_processing.enabled;
            Ok(())
        }) {
            Ok(next) => next,
            Err(e) => {
                crate::log::log(&format!("post-processing toggle 保存失败: {e}"));
                return;
            }
        };
        self.emit_pp_status(
            "toggle",
            next.post_processing.enabled,
            next.post_processing.active_pipeline_id,
        );
    }

    /// 切换文字处理流程热键：按声明顺序把 `active_pipeline_id` 前进到下一个流程并
    /// 持久化；不足两条流程时无操作。悬浮窗显示切换到的新流程名。
    pub async fn on_post_processing_cycle(&self) {
        let Some(repo) = self.settings_repository.clone() else {
            crate::log::log("post-processing cycle: 未注入配置仓库，忽略");
            return;
        };
        if repo.snapshot().post_processing.pipelines.len() < 2 {
            // 单流程甚至无流程：切换键无操作（记日志便于排障）
            crate::log::log("post-processing cycle: 流程不足两条，忽略");
            return;
        }
        let next = match repo.update(|settings| {
            let pipelines = &settings.post_processing.pipelines;
            let active = settings.post_processing.active_pipeline_id.clone();
            let cur = pipelines.iter().position(|p| p.id == active).unwrap_or(0);
            let next_id = pipelines[(cur + 1) % pipelines.len()].id.clone();
            settings.post_processing.active_pipeline_id = next_id;
            Ok(())
        }) {
            Ok(next) => next,
            Err(e) => {
                crate::log::log(&format!("post-processing cycle 保存失败: {e}"));
                return;
            }
        };
        self.emit_pp_status(
            "cycle",
            next.post_processing.enabled,
            next.post_processing.active_pipeline_id,
        );
    }

    /// 向悬浮窗广播一次文字处理状态变化（开灯 / 切换流程；`action` 供前端区分文案）。
    fn emit_pp_status(&self, action: &str, enabled: bool, active_pipeline_id: String) {
        let settings = self.settings.read().unwrap().clone();
        let display_name = settings
            .post_processing
            .active_pipeline()
            .map(|p| p.display_name.clone())
            .unwrap_or_default();
        self.emitter.emit(
            "kotone://pp-status",
            json!({
                "action": action,
                "enabled": enabled,
                "pipelineId": active_pipeline_id,
                "displayName": display_name,
            }),
        );
    }
}
