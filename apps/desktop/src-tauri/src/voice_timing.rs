//! 语音回合每一段花了多久（ADR 029 §七）：说完（静音判停）→ 识别出字 → 模型第一个字 → 第一次出声，
//! 每个语音回合在日志里写一行 `voice-timing:`，填充话和回答分开记。
//!
//! 照 LiveKit Agents 的 `metrics`（EOU 延迟、LLM TTFT、TTS TTFB 分开记）与 Pipecat 的
//! `UserBotLatencyLogObserver`（说完到出声）。语音回合一次只有一个，所以只记“当前这一轮”。
//!
//! “出声”：回答是整段合成好（已进缓存）、交给播放的时刻；前面还在播填充话时要等它播完，
//! 所以另外记填充话播完的时刻，出声取两者较晚的。

use std::sync::Mutex;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Timing {
    /// 静音判停、开始识别。
    pub speech_end: Option<u64>,
    /// 识别出的话交给琴音。
    pub text: Option<u64>,
    /// 模型的第一个字（或第一个工具调用）。
    pub first_token: Option<u64>,
    /// 填充话出声的时刻与内容；播完的时刻。
    pub filler: Option<(u64, String)>,
    pub filler_end: Option<u64>,
    /// 她第一次出声（模型说的：工具前的一句或回答）。
    pub sound: Option<u64>,
    pub sound_kind: &'static str,
}

static CURRENT: Mutex<Option<Timing>> = Mutex::new(None);

fn now() -> u64 {
    kotone_core::reminders::now_ms()
}

/// 助手会话静音判停（编排器进入“识别中”）：开始记新的一轮。
pub fn speech_ended() {
    *CURRENT.lock().unwrap() = Some(Timing {
        speech_end: Some(now()),
        ..Timing::default()
    });
}

fn update(change: impl FnOnce(&mut Timing)) {
    if let Some(t) = CURRENT.lock().unwrap().as_mut() {
        change(t);
    }
}

/// 识别出的话交给了琴音（语音来源）。
pub fn recognized() {
    let at = now();
    update(|t| {
        t.text.get_or_insert(at);
    });
}

/// 语音回合里模型的第一个字 / 工具调用。
pub fn first_token() {
    let at = now();
    update(|t| {
        if t.text.is_some() {
            t.first_token.get_or_insert(at);
        }
    });
}

/// 填充话开始播。
pub fn filler_started(text: &str) {
    let at = now();
    update(|t| {
        if t.filler.is_none() {
            t.filler = Some((at, text.to_string()));
        }
    });
}

/// 填充话播完。
pub fn filler_ended() {
    let at = now();
    update(|t| {
        if t.filler.is_some() {
            t.filler_end.get_or_insert(at);
        }
    });
}

/// 模型说的第一句话可以开始播了（`kind`：`answer` / `preamble` / `say` / `ask`）：写一行日志，这一轮记完。
pub fn sound(kind: &'static str) {
    let at = now();
    let done = {
        let mut current = CURRENT.lock().unwrap();
        match current.as_mut() {
            Some(t) if t.text.is_some() && t.sound.is_none() => {
                t.sound = Some(at);
                t.sound_kind = kind;
                current.take()
            }
            _ => None,
        }
    };
    if let Some(t) = done {
        kotone_core::log::log(&format(&t));
    }
}

/// 这一轮没有出声就结束了（被停下、没有要念的）：丢掉。
pub fn discard() {
    *CURRENT.lock().unwrap() = None;
}

/// 日志里的一行（纯函数）。缺的段写 `-`。
pub fn format(t: &Timing) -> String {
    let span = |from: Option<u64>, to: Option<u64>| match (from, to) {
        (Some(a), Some(b)) => format!("{}ms", b.saturating_sub(a)),
        _ => "-".into(),
    };
    // 填充话还在播时，回答要等它播完才出声。
    let sound = match (t.sound, t.filler_end) {
        (Some(s), Some(f)) => Some(s.max(f)),
        (s, _) => s,
    };
    let mut line = format!(
        "voice-timing: vad→text={} text→token={} token→sound={} vad→sound={} first={}",
        span(t.speech_end, t.text),
        span(t.text, t.first_token),
        span(t.first_token, sound),
        span(t.speech_end, sound),
        if t.sound_kind.is_empty() {
            "-"
        } else {
            t.sound_kind
        },
    );
    match &t.filler {
        Some((at, text)) => line.push_str(&format!(
            " filler=+{}ms({text})",
            at.saturating_sub(t.speech_end.unwrap_or(*at))
        )),
        None => line.push_str(" filler=none"),
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_line_has_every_stage_and_the_filler_separately() {
        let t = Timing {
            speech_end: Some(1_000),
            text: Some(1_060),
            first_token: Some(2_360),
            filler: Some((2_200, "我看看".into())),
            filler_end: Some(2_900),
            sound: Some(2_700),
            sound_kind: "answer",
        };
        // 回答 2.7 s 就合成好了，但要等填充话 2.9 s 播完
        assert_eq!(
            format(&t),
            "voice-timing: vad→text=60ms text→token=1300ms token→sound=540ms vad→sound=1900ms first=answer filler=+1200ms(我看看)"
        );
        let bare = Timing {
            speech_end: Some(0),
            text: Some(40),
            ..Timing::default()
        };
        assert_eq!(
            format(&bare),
            "voice-timing: vad→text=40ms text→token=- token→sound=- vad→sound=- first=- filler=none"
        );
    }

    #[test]
    fn a_turn_is_logged_once_at_its_first_sound() {
        speech_ended();
        // 还没识别出字时的“出声”（例如唤醒应答）不算
        sound("answer");
        assert!(CURRENT.lock().unwrap().is_some());
        recognized();
        first_token();
        filler_started("稍等哈");
        filler_started("我看看");
        filler_ended();
        assert_eq!(
            CURRENT
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .filler
                .as_ref()
                .unwrap()
                .1,
            "稍等哈"
        );
        sound("preamble");
        assert!(CURRENT.lock().unwrap().is_none());
        // 记完以后的事件不再开新的一轮
        recognized();
        sound("answer");
        assert!(CURRENT.lock().unwrap().is_none());
    }
}
