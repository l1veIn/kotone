//! 她答完以后还听不听（ADR 029 §五“接着听”，ADR 033 通话，ADR 033 2026-11-26 修订），和“说哪些话算结束”。
//!
//! - 通话里一直在听（[`crate::call`]）：她念完一句回答时，气泡留着这句、显示通话还剩多久挂断。
//! - 说一句就走、录音笔（缺省）：她说完就安静；只有她这次最后一句以问号结尾（[`ends_with_question`]），
//!   才再听一次 `assistant.followUpSeconds` 秒（玩家原话：“只有结尾是问号才自动聆听好一点”）。
//! - 通话被挂断了（退下、呼叫键、没人说话）：她问了也不再听。
//!
//! 怎么办由纯函数 [`after_reply_action`] 定，[`after_reply`] 照着做。
//!
//! - 打断计数（[`interrupted`] / [`epoch`]）：她念的时候被打断了，之后排着的旧话不再念。
//! - 说“没事了 / 不用了 / 退下 / 安静 / 挂了 / 拜拜 / 再见”（整句）结束（[`is_exit_phrase`]），
//!   照 xiaozhi-esp32-server（MIT）精确匹配退出词、orb（MIT）的“退下”。

use std::sync::atomic::{AtomicU64, Ordering};

use kotone_core::assistant::CallMode;
use kotone_core::log;
use kotone_core::orchestrator::OrchestratorState;
use tauri::{AppHandle, Manager};

use crate::SharedState;

/// 打断计数：玩家按呼叫键打断她时加一。开始念回答时记下，念完时变了就不接着听。
static INTERRUPTS: AtomicU64 = AtomicU64::new(0);

pub fn epoch() -> u64 {
    INTERRUPTS.load(Ordering::SeqCst)
}

/// 玩家打断了她（按呼叫键、开口打断）。
pub fn interrupted() {
    INTERRUPTS.fetch_add(1, Ordering::SeqCst);
}

/// 她答完以后怎么办。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AfterReply {
    /// 通话还开着：一直在听，气泡显示通话还剩多久挂断。
    KeepCall,
    /// 她问了一句：再听这么多秒。
    ListenOnce(u32),
    /// 安静，不再听。
    Quiet,
}

/// 她答完以后怎么办（纯函数）。`last_spoken`：她这一轮最后念出来的那句。
pub fn after_reply_action(
    in_call: bool,
    mode: CallMode,
    last_spoken: &str,
    follow_up_seconds: u32,
) -> AfterReply {
    if in_call {
        return AfterReply::KeepCall;
    }
    // 选的是通话、可通话已经挂断了（玩家说了退下、按了呼叫键、没人说话）：不再听
    if mode == CallMode::Call || follow_up_seconds == 0 || !ends_with_question(last_spoken) {
        return AfterReply::Quiet;
    }
    AfterReply::ListenOnce(follow_up_seconds)
}

/// 这句话是不是问句：去掉末尾的空白、引号、括号、波浪号、感叹号和表情以后，最后一个字是 `？` 或 `?`（纯函数）。
pub fn ends_with_question(text: &str) -> bool {
    text.trim_end_matches(|c: char| c.is_whitespace() || trails_a_question(c))
        .ends_with(['？', '?'])
}

/// 问号后面常跟着、不改变“这是问句”的字符。
fn trails_a_question(c: char) -> bool {
    matches!(
        c,
        '"' | '\''
            | '“'
            | '”'
            | '‘'
            | '’'
            | '」'
            | '』'
            | '）'
            | ')'
            | '】'
            | '》'
            | '~'
            | '～'
            | '！'
            | '!'
    ) || matches!(
        u32::from(c),
        // 表情、符号、变体选择符、连接符（👀 ☺️ 👍🏻 这类）
        0x1F000..=0x1FAFF | 0x2600..=0x27BF | 0x2B00..=0x2BFF | 0xFE00..=0xFE0F | 0x200D | 0x20E3
    )
}

/// 语音回合的回答念完了（`epoch`：开始念时的打断计数；`shown`：通话里气泡留着的那句；`last_spoken`：
/// 她这一轮最后念出来的那句）。通话里：气泡显示倒计时（一直在听，这里不用再开收音）；说一句就走 / 录音笔里
/// 她问了一句：再听一次。
pub async fn after_reply(app: &AppHandle, epoch: u64, shown: &str, last_spoken: &str) {
    if self::epoch() != epoch {
        return;
    }
    let follow_up_seconds = app
        .state::<SharedState>()
        .settings
        .read()
        .unwrap()
        .assistant
        .follow_up_seconds;
    match after_reply_action(
        crate::call::active(),
        crate::call::mode(app),
        last_spoken,
        follow_up_seconds,
    ) {
        AfterReply::KeepCall => crate::call::after_reply(app, shown),
        AfterReply::ListenOnce(seconds) => listen_once(app, seconds, last_spoken).await,
        AfterReply::Quiet => {}
    }
}

/// 她问了一句：不用再按呼叫键，直接听玩家回答（说完即交给她；一直没开口就安静收起）。
async fn listen_once(app: &AppHandle, seconds: u32, question: &str) {
    let orchestrator = app.state::<SharedState>().orchestrator.clone();
    // 玩家已经又按了呼叫键在说（或者别的会话在进行）、有提问在等回答（它自己会听）：不另开
    if !matches!(
        orchestrator.state(),
        OrchestratorState::Idle | OrchestratorState::Success | OrchestratorState::Error
    ) || crate::voice_ask::is_pending(app)
    {
        return;
    }
    log::log(&format!(
        "follow-up: she asked a question, listening {seconds}s more"
    ));
    crate::bubble::question_follow_up(app, u64::from(seconds), question);
    if let Err(e) = orchestrator.begin_assistant_wake(seconds * 1_000).await {
        log::log(&format!("follow-up: could not listen again: {e}"));
        crate::bubble::follow_up_done(app);
    }
}

/// 玩家说的是不是“没事了 / 不用了 / 退下 / 拜拜 / 再见”（整句只有结束的意思；去掉标点、语气词和
/// 开头叫的名字）。写法照 [`crate::agent`] 的 `is_stop_command`。
pub fn is_exit_phrase(text: &str, name: &str) -> bool {
    let core: String = text
        .chars()
        .filter(|c| !c.is_ascii_punctuation() && !"，。！？、…~～ 　".contains(*c))
        .collect();
    let name = name.trim();
    let core = if !name.is_empty() && core.starts_with(name) {
        &core[name.len()..]
    } else {
        core.trim_start_matches("琴音")
    };
    let core = core
        .trim_start_matches(['好', '嗯', '那', '哦'])
        .trim_end_matches(['了', '吧', '啊', '呀', '啦', '哈', '哦', '喽'])
        .to_lowercase();
    matches!(
        core.as_str(),
        "没事"
            | "没事儿"
            | "不用"
            | "退下"
            | "你退下"
            | "你先退下"
            | "安静"
            | "安静点"
            | "安静点儿"
            | "安静一点"
            | "闭嘴"
            | "挂"
            | "挂断"
            | "挂电话"
            | "先这样"
            | "拜拜"
            | "再见"
            | "bye"
            | "byebye"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_question_is_what_ends_with_a_question_mark() {
        for t in [
            "要我帮你看看对面打野吗？",
            "要吗?",
            "要我放首歌吗？ ",
            "你说的是“中路”吗？”",
            "真的要开吗？！",
            "要不要我提醒你？～",
            "要我继续吗？👀",
            "好不好？☺️",
            "好吗?\"  ",
            "‘要吗？’",
        ] {
            assert!(ends_with_question(t), "{t}");
        }
        for t in [
            "",
            "   ",
            "好的，已经放了。",
            "这是什么？我查到了：无尽之刃。",
            "你问我能做什么？很多。",
            "下次再说吧！",
            "？是什么意思",
            "我在想…",
        ] {
            assert!(!ends_with_question(t), "{t}");
        }
    }

    #[test]
    fn she_listens_again_only_after_a_question_unless_in_a_call() {
        use AfterReply::*;
        // 说一句就走、录音笔：说完就安静；问句才再听 followUpSeconds 秒
        for mode in [CallMode::OneShot, CallMode::Dictation] {
            assert_eq!(after_reply_action(false, mode, "好的，放了。", 8), Quiet);
            assert_eq!(
                after_reply_action(false, mode, "要我顺便放下一首吗？", 8),
                ListenOnce(8)
            );
            // 什么也没说（被停下、只交给了后台）
            assert_eq!(after_reply_action(false, mode, "", 8), Quiet);
            // followUpSeconds 0：问了也不再听
            assert_eq!(after_reply_action(false, mode, "要吗？", 0), Quiet);
        }
        // 通话：开着就一直听（不管是不是问句）；挂断了（退下 / 呼叫键 / 没人说话）就不再听
        assert_eq!(
            after_reply_action(true, CallMode::Call, "好的。", 8),
            KeepCall
        );
        assert_eq!(
            after_reply_action(true, CallMode::Call, "要吗？", 8),
            KeepCall
        );
        assert_eq!(
            after_reply_action(false, CallMode::Call, "要吗？", 8),
            Quiet
        );
        // 通话中途在设置里换成了说一句就走：这一句答完照旧（挂断由通话自己收尾）
        assert_eq!(
            after_reply_action(true, CallMode::OneShot, "好的。", 8),
            KeepCall
        );
    }

    #[test]
    fn exit_phrases_end_quietly_only_when_alone() {
        for t in [
            "没事了",
            "没事了。",
            "嗯，不用了",
            "不用了吧",
            "退下",
            "退下吧！",
            "拜拜",
            "拜拜啦~",
            "再见",
            "琴音，再见",
            "大肥鱼 没事了",
            "好，没事了",
            "Bye.",
            "安静",
            "安静点！",
            "琴音，安静一点",
            "挂了吧",
            "挂断",
            "你退下吧",
            "闭嘴",
            "先这样吧",
        ] {
            assert!(is_exit_phrase(t, "大肥鱼"), "{t}");
        }
        for t in [
            "没事了帮我看看地图",
            "不用了，换个英雄",
            "再见到他告诉我",
            "拜托帮我查一下",
            "你好",
            "安静地等一下",
            "挂机了",
            "",
        ] {
            assert!(!is_exit_phrase(t, "大肥鱼"), "{t}");
        }
    }
}
