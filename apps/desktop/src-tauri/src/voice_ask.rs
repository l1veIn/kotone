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

/// 问完那几秒没听到回答时说的话。确认还在等：之后用嘴说“可以 / 不要”也算
/// （[`crate::agent::answer_by_voice`]），窗口里的按钮也一直能点。
pub const NOT_HEARD: &str = "没听清。说“可以”或“不要”就行。";

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

/// 有提问正在等回答吗（这时不“回答完接着听”，见 [`crate::follow_up`]）。
pub fn is_pending(app: &AppHandle) -> bool {
    app.try_state::<VoiceAsk>()
        .is_some_and(|state| state.pending.lock().unwrap().is_some())
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
    // 1. 说出问题（说完才返回）。合成费用记到正在跑的那一轮。
    if let Some(owner) = kotone_core::usage::current_turn() {
        kotone_core::usage::claim_speech(question, owner);
    }
    // 工具前那句（“我问一下”）还在边生成边念：念完再问，两句不叠在一起。
    crate::voice_reply::wait_idle(Duration::from_secs(15)).await;
    crate::voice_timing::sound("ask");
    let speak_app = app.clone();
    let text = question.to_string();
    let spoken = tauri::async_runtime::spawn_blocking(move || {
        crate::assistant::speak_now(&speak_app, &text)
    })
    .await
    .map_err(|_| "播报问题失败".to_string())?;
    match spoken {
        Ok(()) => {}
        // 通话里玩家没等她问完就开口了（问题被停掉、或正在收他的话）：直接听他这句（ADR 033）
        Err(e) if crate::call::active() && (e.contains("取消") || e.contains("正在录音")) => {
            log::log("voice ask: the player spoke before the question finished");
        }
        Err(e) => return Err(format!("没能把问题说出来：{e}")),
    }
    // 2. 登记等待（新的提问顶掉旧的）
    let (tx, mut rx) = oneshot::channel();
    let id = state.next.fetch_add(1, Ordering::SeqCst) + 1;
    *state.pending.lock().unwrap() = Some((id, tx));
    let _guard = PendingGuard { state: &state, id };
    // 3. 不用唤醒词，直接开始收音（通话里本来就一直在听，下一句就是回答，ADR 033）
    let wait = wait_seconds.clamp(3, 15);
    let orchestrator = app.state::<SharedState>().orchestrator.clone();
    if !crate::call::active() {
        orchestrator
            .begin_assistant_wake((wait * 1000) as u32)
            .await
            .map_err(|e| format!("没法开始听你回答：{e}"))?;
    }
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

/// 整句只是“可以 / 允许 / 不要”这类回答吗（去掉开头叫的名字、语气词和标点）；不是就 `None`。
///
/// 用在问完那几秒之后：确认还在等、玩家又开口时，只有这样的整句才当回答，别的话照常交给她——
/// 不能用 [`parse_confirmation`]，它按“含有”判断，“对面打野在下路”会被当成“对”。
pub fn bare_confirmation(text: &str, name: &str) -> Option<bool> {
    const YES: &[&str] = &[
        "没问题",
        "可以",
        "允许",
        "同意",
        "确定",
        "继续",
        "好的",
        "行",
        "好",
        "是",
        "对",
        "嗯",
        "要",
        "ok",
        "yes",
        "来吧",
        "当然",
    ];
    const NO: &[&str] = &[
        "不可以",
        "不需要",
        "不允许",
        "不同意",
        "不要",
        "不用",
        "不行",
        "不了",
        "算了",
        "取消",
        "拒绝",
        "别",
        "no",
        "不",
    ];
    const FILLER: &[char] = &['吧', '啊', '呀', '了', '的', '哈', '嘛', '呗', '哦', '喔'];
    let core: String = text
        .chars()
        .filter(|c| {
            !c.is_whitespace() && !c.is_ascii_punctuation() && !"，。！？、…~～".contains(*c)
        })
        .collect::<String>()
        .to_lowercase();
    let name = name.trim().to_lowercase();
    let mut rest = if !name.is_empty() && core.starts_with(&name) {
        &core[name.len()..]
    } else {
        core.trim_start_matches("琴音")
    };
    // 整句由“是”的词（或整句由“不”的词）和语气词拼成才算；两种混着说（“好，不要”）听不出来。
    let mut answer = None;
    while !rest.is_empty() {
        if let Some(c) = rest.chars().next().filter(|c| FILLER.contains(c)) {
            rest = &rest[c.len_utf8()..];
            continue;
        }
        // 开头是哪个词（多长）
        let matched = |words: &[&str]| -> Option<usize> {
            words
                .iter()
                .find(|w| rest.starts_with(**w))
                .map(|w| w.len())
        };
        let (len, yes) = match (matched(NO), matched(YES)) {
            (Some(len), _) => (len, false),
            (None, Some(len)) => (len, true),
            (None, None) => return None,
        };
        if answer.is_some_and(|a| a != yes) {
            return None;
        }
        answer = Some(yes);
        rest = &rest[len..];
    }
    answer
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
    fn later_answers_must_be_bare_confirmations() {
        for yes in [
            "允许",
            "允许。",
            "可以",
            "可以可以",
            "好的",
            "行吧",
            "嗯嗯",
            "琴音，可以",
            "大肥鱼 允许吧",
            "OK",
            "没问题啊",
            "好，允许",
        ] {
            assert_eq!(bare_confirmation(yes, "大肥鱼"), Some(true), "{yes}");
        }
        for no in [
            "不要",
            "不用了",
            "不行",
            "算了吧",
            "别",
            "不允许",
            "取消",
            "琴音不要",
        ] {
            assert_eq!(bare_confirmation(no, "大肥鱼"), Some(false), "{no}");
        }
        // 平常说话里带着“对 / 是 / 好 / 要”不能当成回答（2026-11-22 修：之前只在问完那几秒里听）
        for other in [
            "",
            "对面打野在下路",
            "是谁在说话",
            "好像不太对",
            "要不要先回城",
            "不知道",
            "可以吗",
            "好，不要",
            "允许它下载那个文件，然后把音量调小",
        ] {
            assert_eq!(bare_confirmation(other, "大肥鱼"), None, "{other}");
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
