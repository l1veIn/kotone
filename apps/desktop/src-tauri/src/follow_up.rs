//! 回答完接着听（ADR 029 §五）在通话（ADR 033）里的样子，和“说哪些话算结束”。
//!
//! ADR 033 起没有单独的“接着听 8 秒”：呼叫键 / 唤醒就是接通通话，通话里一直在听（[`crate::call`]）；
//! 她念完一句回答时，气泡留着这句、显示通话还剩多久挂断（[`after_reply`]）。不在通话里（设置里关了
//! 通话）就一问一答，答完不再听。
//!
//! - 打断计数（[`interrupted`] / [`epoch`]）：她念的时候被打断了，之后排着的旧话不再念。
//! - 说“没事了 / 不用了 / 退下 / 安静 / 挂了 / 拜拜 / 再见”（整句）结束（[`is_exit_phrase`]），
//!   照 xiaozhi-esp32-server（MIT）精确匹配退出词、orb（MIT）的“退下”。

use std::sync::atomic::{AtomicU64, Ordering};

use tauri::AppHandle;

/// 打断计数：玩家按呼叫键打断她时加一。开始念回答时记下，念完时变了就不接着听。
static INTERRUPTS: AtomicU64 = AtomicU64::new(0);

pub fn epoch() -> u64 {
    INTERRUPTS.load(Ordering::SeqCst)
}

/// 玩家打断了她（按呼叫键、开口打断）。
pub fn interrupted() {
    INTERRUPTS.fetch_add(1, Ordering::SeqCst);
}

/// 语音回合的回答念完了（`epoch`：开始念时的打断计数）。通话里：气泡留着这句（`shown`）、
/// 显示倒计时，接着听（一直在听，这里不用再开收音）。
pub async fn after_reply(app: &AppHandle, epoch: u64, shown: &str) {
    if self::epoch() != epoch {
        return;
    }
    crate::call::after_reply(app, shown);
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
