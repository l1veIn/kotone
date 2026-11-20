//! 回答完接着听（ADR 029 §五）：语音入口的回答念完后，不用再按呼叫键、不用再喊唤醒词，
//! 直接接着说就是这段对话的下一句；几秒没开口就安静结束。
//!
//! 照成熟做法：xiaozhi-esp32-server（MIT）`auto` 模式念完约 0.24 s 重新开麦（`sendAudioHandle.py`）、
//! 精确匹配“退出 / 关闭”结束（`exit_commands`）；Pipecat 唤醒后 10 s 内不用再喊
//! （`WakePhraseUserTurnStartStrategy`）；Home Assistant 回答以问号结尾才再听一次。
//! 半双工：她念完才开麦，所以不会录到自己的声音。
//!
//! - 收音复用 `voice_ask` 那条路（`begin_assistant_wake`：说完即提交，一直不开口就安静取消）；
//! - 气泡复用追问的倒计时（[`crate::bubble::follow_up`]）；
//! - 玩家按呼叫键打断她（[`interrupted`]）、说“停下”、有别的会话在进行时不接着听；
//! - 说“没事了 / 不用了 / 退下 / 拜拜 / 再见”（整句）安静结束，不交给模型（[`is_exit_phrase`]）。

use std::sync::atomic::{AtomicU64, Ordering};

use kotone_core::log;
use kotone_core::orchestrator::OrchestratorState;
use tauri::{AppHandle, Manager};

use crate::SharedState;

/// 打断计数：玩家按呼叫键打断她时加一。开始念回答时记下，念完时变了就不接着听。
static INTERRUPTS: AtomicU64 = AtomicU64::new(0);

pub fn epoch() -> u64 {
    INTERRUPTS.load(Ordering::SeqCst)
}

/// 玩家打断了她（按呼叫键）。
pub fn interrupted() {
    INTERRUPTS.fetch_add(1, Ordering::SeqCst);
}

/// 语音回合的回答念完了（`epoch`：开始念时的打断计数）：条件都满足就接着听。
/// `shown`：气泡里留着的那句回答。
pub async fn after_reply(app: &AppHandle, epoch: u64, shown: &str) {
    let (seconds, legacy) = {
        let settings = app.state::<SharedState>().settings.read().unwrap().clone();
        (
            settings.assistant.follow_up_seconds,
            settings.ui.legacy_upgrade.uses_legacy_assistant(),
        )
    };
    if seconds == 0 || legacy {
        return;
    }
    // 这一轮刚结束时对话循环还要收个尾：稍等它空下来（玩家又插了话、接着跑下一轮就不听了）。
    for _ in 0..30 {
        if !crate::agent::is_running(app) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    if let Some(reason) = blocked(app, epoch) {
        log::log(&format!("follow-up: not listening ({reason})"));
        return;
    }
    let orchestrator = app.state::<SharedState>().orchestrator.clone();
    crate::bubble::follow_up(app, u64::from(seconds), shown);
    match orchestrator.begin_assistant_wake(seconds * 1000).await {
        Ok(()) => log::log(&format!("follow-up: listening for {seconds}s")),
        Err(e) => {
            log::log(&format!("follow-up: could not listen: {e}"));
            crate::bubble::follow_up_done(app);
        }
    }
}

/// 现在不该接着听的原因（None = 可以）。
fn blocked(app: &AppHandle, epoch: u64) -> Option<&'static str> {
    if self::epoch() != epoch {
        return Some("interrupted");
    }
    if crate::agent::is_running(app) {
        return Some("agent busy");
    }
    if crate::voice_ask::is_pending(app) {
        return Some("asking");
    }
    let state = app.state::<SharedState>().orchestrator.state();
    if !matches!(
        state,
        OrchestratorState::Idle | OrchestratorState::Success | OrchestratorState::Error
    ) {
        return Some("another session");
    }
    let phase = app.state::<crate::runtime::RuntimeManager>().phase();
    if phase != kotone_core::runtime::RuntimePhase::Running {
        return Some("speech recognition not running");
    }
    None
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
        "没事" | "没事儿" | "不用" | "退下" | "拜拜" | "再见" | "bye" | "byebye"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

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
        ] {
            assert!(is_exit_phrase(t, "大肥鱼"), "{t}");
        }
        for t in [
            "没事了帮我看看地图",
            "不用了，换个英雄",
            "再见到他告诉我",
            "拜托帮我查一下",
            "你好",
            "",
        ] {
            assert!(!is_exit_phrase(t, "大肥鱼"), "{t}");
        }
    }
}
