//! 语音多轮：琴音用语音问一句，然后不用唤醒词直接听玩家回答几秒（像车机、智能音箱的追问）。
//!
//! - `reply.ask` 工具与“语音确认权限”共用这里。
//! - 问题用当前语音说完才开始收音（不把自己的声音录进去）；收音走与唤醒相同的助手会话
//!   （`begin_assistant_wake`：说完即提交，一直不开口就安静结束）。
//! - 识别结果照常经助手入口交给琴音；[`deliver_answer`] 在那里拦下，交回正在等的这次提问，
//!   而不是变成插话或新的一轮。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use kotone_core::log;
use kotone_core::orchestrator::OrchestratorState;
use tauri::{AppHandle, Manager};
use tokio::sync::oneshot;

use crate::SharedState;

/// 等不到回答时最多等多久（玩家开口后说得很长、识别慢的上限）。
const HARD_LIMIT: Duration = Duration::from_secs(60);

#[derive(Default)]
pub struct VoiceAsk {
    pending: Mutex<Option<(u64, oneshot::Sender<String>)>>,
    next: AtomicU64,
}

/// 有提问在等回答时，把这句语音交给它；返回是否交出去了。
pub fn deliver_answer(app: &AppHandle, text: &str) -> bool {
    let Some(state) = app.try_state::<VoiceAsk>() else {
        return false;
    };
    let pending = state.pending.lock().unwrap().take();
    match pending {
        Some((_, tx)) => tx.send(text.to_string()).is_ok(),
        None => false,
    }
}

/// 提问结束（任何方式）时气泡回到“在想”。
struct AskBubbleGuard(AppHandle);

impl Drop for AskBubbleGuard {
    fn drop(&mut self) {
        crate::bubble::ask_done(&self.0);
    }
}

/// 提问结束（拿到回答、超时、或这一轮被停止）时清掉登记，晚到的回答不会被误交。
struct PendingGuard<'a> {
    state: &'a VoiceAsk,
    id: u64,
}

impl Drop for PendingGuard<'_> {
    fn drop(&mut self) {
        let mut pending = self.state.pending.lock().unwrap();
        if pending.as_ref().is_some_and(|(id, _)| *id == self.id) {
            *pending = None;
        }
    }
}

fn session_active(state: OrchestratorState) -> bool {
    matches!(
        state,
        OrchestratorState::Listening
            | OrchestratorState::Transcribing
            | OrchestratorState::Processing
            | OrchestratorState::Preview
            | OrchestratorState::Sending
    )
}

/// 说出问题，再听玩家回答最多 `wait_seconds` 秒（开口后会等他说完）。
/// `Ok(None)` = 没回答；`Err` = 问不出去或没法收音（例如语音识别没在运行）。
pub async fn ask(
    app: &AppHandle,
    question: &str,
    wait_seconds: u64,
) -> Result<Option<String>, String> {
    let state = app.try_state::<VoiceAsk>().ok_or("语音提问不可用")?;
    crate::bubble::asking(app, question);
    let _bubble = AskBubbleGuard(app.clone());
    // 1. 说出问题（说完才返回）
    let speak_app = app.clone();
    let text = question.to_string();
    tauri::async_runtime::spawn_blocking(move || crate::assistant::speak_now(&speak_app, &text))
        .await
        .map_err(|_| "播报问题失败".to_string())?
        .map_err(|e| format!("没能把问题说出来：{e}"))?;
    // 2. 登记等待（新的提问顶掉旧的）
    let (tx, mut rx) = oneshot::channel();
    let id = state.next.fetch_add(1, Ordering::SeqCst) + 1;
    *state.pending.lock().unwrap() = Some((id, tx));
    let _guard = PendingGuard { state: &state, id };
    // 3. 不用唤醒词，直接开始收音
    let wait = wait_seconds.clamp(3, 15);
    let orchestrator = app.state::<SharedState>().orchestrator.clone();
    orchestrator
        .begin_assistant_wake((wait * 1000) as u32)
        .await
        .map_err(|e| format!("没法开始听你回答：{e}"))?;
    log::log(&format!("voice ask: listening for {wait}s"));
    crate::bubble::ask_listening(app, wait);
    // 4. 等回答：开口后等他说完；一直没开口、过了等待时间就算没回答
    let started = Instant::now();
    let window = Duration::from_secs(wait) + Duration::from_millis(800);
    loop {
        tokio::select! {
            answer = &mut rx => {
                return Ok(answer.ok().map(|a| a.trim().to_string()).filter(|a| !a.is_empty()));
            }
            _ = tokio::time::sleep(Duration::from_millis(200)) => {}
        }
        let elapsed = started.elapsed();
        if elapsed >= HARD_LIMIT || (elapsed >= window && !session_active(orchestrator.state())) {
            log::log("voice ask: no answer");
            return Ok(None);
        }
    }
}

/// 把玩家对“可以吗？”的回答理解成同意 / 拒绝；听不出来时 `None`（纯函数）。
pub fn parse_confirmation(text: &str) -> Option<bool> {
    let t: String = text
        .chars()
        .filter(|c| !c.is_whitespace() && !"，。！？、,.!?~～…".contains(*c))
        .collect::<String>()
        .to_lowercase();
    if t.is_empty() {
        return None;
    }
    const NO: &[&str] = &[
        "不要",
        "不用",
        "不行",
        "不可以",
        "不好",
        "不了",
        "不需要",
        "别",
        "算了",
        "取消",
        "拒绝",
        "停",
        "no",
        "不",
    ];
    const YES: &[&str] = &[
        "可以",
        "好",
        "行",
        "是",
        "对",
        "嗯",
        "确定",
        "同意",
        "允许",
        "没问题",
        "继续",
        "要",
        "下载",
        "ok",
        "yes",
        "当然",
        "来吧",
        "开始",
    ];
    if NO
        .iter()
        .any(|w| t.starts_with(w) || (w.chars().count() > 1 && t.contains(w)))
    {
        return Some(false);
    }
    // 犹豫、反问：不当成同意（退回窗口里确认）
    const UNSURE: &[&str] = &[
        "想想",
        "等等",
        "等一下",
        "再说",
        "不知道",
        "考虑",
        "什么",
        "吗",
    ];
    if UNSURE.iter().any(|w| t.contains(w)) {
        return None;
    }
    if YES.iter().any(|w| t.contains(w)) {
        return Some(true);
    }
    None
}

/// 权限确认要念出来的话：去掉括号里的补充说明（念出来别扭）。
pub fn confirmation_question(reason: &str) -> String {
    let mut out = String::new();
    let mut depth = 0;
    for c in reason.chars() {
        match c {
            '（' | '(' => depth += 1,
            '）' | ')' => depth = (depth - 1).max(0),
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    format!("这一步{}，可以吗？", out.trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confirmations_are_understood() {
        for yes in [
            "好",
            "好的",
            "可以。",
            "行啊",
            "嗯嗯",
            "确定",
            "下载吧",
            "OK",
            "要的",
        ] {
            assert_eq!(parse_confirmation(yes), Some(true), "{yes}");
        }
        for no in [
            "不要",
            "不用了",
            "算了",
            "别下了",
            "不行",
            "不",
            "取消吧",
            "不好",
        ] {
            assert_eq!(parse_confirmation(no), Some(false), "{no}");
        }
        for unclear in ["", "嗯……我想想", "你说什么", "等一下", "要下多大的吗"]
        {
            assert_eq!(parse_confirmation(unclear), None, "{unclear}");
        }
    }

    #[test]
    fn spoken_question_drops_parentheses() {
        assert_eq!(
            confirmation_question("会下载模型（可能很大）"),
            "这一步会下载模型，可以吗？"
        );
        assert_eq!(
            confirmation_question("会修改设置"),
            "这一步会修改设置，可以吗？"
        );
    }

    #[test]
    fn only_running_sessions_keep_the_wait_open() {
        assert!(session_active(OrchestratorState::Listening));
        assert!(session_active(OrchestratorState::Processing));
        assert!(!session_active(OrchestratorState::Idle));
        assert!(!session_active(OrchestratorState::Success));
    }
}
