use super::capture::SessionOrigin;
use super::*;
use crate::hotkey_options::{auto_release_ends, VoiceKeyMode};
use crate::interaction::InteractionMode;

impl Orchestrator {
    /// 说话键带了自己的方式（ADR 054：热键表那一条的 `mode`，或第一条跟着的全局方式）：按下 / 松开各来一次。
    /// - 自动：按下照说一句就走开始，按着的时候静音判停不算数；松开时按了 `hold_ms` 以上 = 按住说话，照对讲机的
    ///   松手宽限结束；短于它什么都不做，接着照说一句就走等说完。
    /// - 按住说话：照对讲机。其余三种只看按下，照那个预设。
    ///
    /// `ticket` 是宿主给的入场票（命令行没有，现场取一张）。
    pub async fn on_talk_key(
        &self,
        mode: VoiceKeyMode,
        pressed: bool,
        ticket: Option<u64>,
        hold_ms: u32,
    ) {
        let ticket = ticket.unwrap_or_else(|| self.hotkey_input_ticket());
        let purpose = SessionPurpose::Dictation;
        match mode {
            VoiceKeyMode::Auto if pressed => {
                let at = std::time::Instant::now();
                let _input = self.hold_input.lock().await;
                *self.key_press.lock().unwrap() = None;
                let (_, _, before) = self.hotkey_session_snapshot();
                self.on_purpose_hotkey_toggle_preset(
                    purpose,
                    ticket,
                    Some(InteractionMode::OneShot),
                )
                .await;
                self.remember_press(before, purpose, at);
            }
            VoiceKeyMode::Auto => {
                let _input = self.hold_input.lock().await;
                if let Some(gen) = self.take_long_press(ticket, Some(hold_ms), purpose) {
                    // 和对讲机一样留一小段松手宽限，句尾不被切掉
                    self.schedule_release_end(gen, ticket);
                }
            }
            VoiceKeyMode::Hold => {
                self.on_purpose_hotkey_hold_preset(
                    purpose,
                    pressed,
                    ticket,
                    Some(InteractionMode::PushToTalk),
                )
                .await
            }
            _ if !pressed => {}
            other => {
                self.on_purpose_hotkey_toggle_preset(purpose, ticket, other.to_interaction())
                    .await
            }
        }
    }

    /// 这一下按下开始了新会话（代际变了、正在听、是这个用途）：记下来，按着的时候静音判停不算数。
    pub(super) fn remember_press(
        &self,
        before_gen: u64,
        purpose: SessionPurpose,
        at: std::time::Instant,
    ) {
        let (state, active, gen) = self.hotkey_session_snapshot();
        if gen != before_gen && state == OrchestratorState::Listening && active == purpose {
            self.key_held.store(true, Ordering::SeqCst);
            *self.key_press.lock().unwrap() = Some((gen, at));
        }
    }

    /// 松开：不再“按着”。这一下按下开始的会话还在听，并且（自动时）按了 `hold_ms` 以上，返回它的代际，由调用方结束。
    /// `hold_ms` 为 None 是按住说话（松开就结束）。
    pub(super) fn take_long_press(
        &self,
        ticket: u64,
        hold_ms: Option<u32>,
        purpose: SessionPurpose,
    ) -> Option<u64> {
        self.key_held.store(false, Ordering::SeqCst);
        let (gen, at) = self.key_press.lock().unwrap().take()?;
        if !self.input_ticket_is_current(Some(ticket)) {
            return None;
        }
        if hold_ms.is_some_and(|ms| !auto_release_ends(at.elapsed(), ms)) {
            return None;
        }
        let (state, active, current) = self.hotkey_session_snapshot();
        (current == gen && state == OrchestratorState::Listening && active == purpose)
            .then_some(gen)
    }

    // ---------- 热键入口（hotkey 模块调用；ADR-006 单键语义表 + 策略路由） ----------

    /// 进行中的会话使用开始时的策略；热更新不偷换一次按住/松开配对的语义。
    pub fn policy(&self) -> InteractionPolicy {
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
            // 她的会话由呼叫开始（说一句就走 / 录音笔，见 `begin_admitted_with`），不跟着任何热键设置
            SessionPurpose::Assistant => {
                InteractionPolicy::from_preset(crate::interaction::InteractionMode::OneShot)
            }
        }
    }

    pub(super) fn hotkey_session_snapshot(&self) -> (OrchestratorState, SessionPurpose, u64) {
        let inner = self.inner.lock().unwrap();
        (inner.state, inner.purpose, inner.gen)
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
        self.on_purpose_hotkey_toggle_preset(purpose, ticket, None)
            .await;
    }

    /// 同上；`preset` = 这个键带了自己的方式（ADR 054），开始的会话照它，不看 `interactionMode`。
    pub(super) async fn on_purpose_hotkey_toggle_preset(
        &self,
        purpose: SessionPurpose,
        ticket: u64,
        preset: Option<InteractionMode>,
    ) {
        if !self.input_ticket_is_current(Some(ticket)) {
            return;
        }
        let origin = preset.map_or(SessionOrigin::Hotkey, SessionOrigin::HotkeyPreset);
        // 另一用途的热键只能取消旧会话，不结束它、确认它或把结果交给另一条管线。
        let (state, active_purpose, gen) = self.hotkey_session_snapshot();
        if state != OrchestratorState::Idle && active_purpose != purpose {
            self.cancel_inner(Some(ticket), Some(gen)).await;
            return;
        }
        match state {
            OrchestratorState::Idle => {
                let _ = self
                    .begin_inner_from(false, purpose, Some(gen), Some(ticket), origin)
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
                    .begin_inner_from(false, purpose, Some(gen + 1), Some(ticket), origin)
                    .await;
            }
            OrchestratorState::Error if self.inner.lock().unwrap().pending_text.is_none() => {
                self.cancel_inner(Some(ticket), Some(gen)).await;
                let _ = self
                    .begin_inner_from(false, purpose, Some(gen + 1), Some(ticket), origin)
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
        self.on_purpose_hotkey_hold_preset(purpose, pressed, ticket, None)
            .await;
    }

    /// 同上；`preset` 同 [`Self::on_purpose_hotkey_toggle_preset`]。
    pub(super) async fn on_purpose_hotkey_hold_preset(
        &self,
        purpose: SessionPurpose,
        pressed: bool,
        ticket: u64,
        preset: Option<InteractionMode>,
    ) {
        let _input = self.hold_input.lock().await;
        if !self.input_ticket_is_current(Some(ticket)) {
            return;
        }
        let origin = preset.map_or(SessionOrigin::Hotkey, SessionOrigin::HotkeyPreset);
        let (state, active_purpose, gen) = self.hotkey_session_snapshot();
        if state != OrchestratorState::Idle && active_purpose != purpose {
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
                        .begin_inner_from(false, purpose, Some(gen), Some(ticket), origin)
                        .await;
                }
                OrchestratorState::Success => {
                    self.cancel_inner(Some(ticket), Some(gen)).await;
                    let _ = self
                        .begin_inner_from(false, purpose, Some(gen + 1), Some(ticket), origin)
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
                            .begin_inner_from(false, purpose, Some(gen + 1), Some(ticket), origin)
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
}
