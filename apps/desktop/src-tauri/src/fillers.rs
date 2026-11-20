//! 填充话预先合成（ADR 029 §四）：唤醒应答“嗯？/我在”和等待时说的“我看看/稍等哈……”。
//!
//! 照 xiaozhi-esp32-server（MIT，`core/handle/helloHandle.py`）：按音色缓存合成好的短句，启动时预热，
//! 用的时候从不现场合成。以前“先接一句”每次现合成，玩家本机实测 1.3–7.8 秒才出声，比回答还慢。
//!
//! - 用在线语音或 Kokoro 时：启动后、换声音 / 角色 / 引擎后在后台把这些短句合成进现有的语音缓存
//!   （`kotone_tts::cache`，按“引擎 + 声音 + 语速 + 文字”区分）；每个语音回合结束后再过一遍：
//!   命中缓存不花钱，还能让它们留在缓存里不被挤掉。
//! - 落盘（照 xiaozhi-esp32-server `core/utils/wakeup_word.py` 按音色存唤醒回复）：合成过的存成
//!   `<KOTONE_HOME>/cache/tts/<缓存键的哈希>.wav`，重启后直接读文件，不再请求在线语音、不再花钱。
//!   只存这里登记的短句（`kotone_tts::cache::disk`），模型的回答不落盘。
//! - 只播已经合成好的：没准备好就这一轮不说（宁可不说，也不比回答还晚）。系统语音不用合成，随时能说。
//! - 预先合成的费用不归任何对话（`kotone_core::usage::claim_speech` 没有登记）。

use std::sync::Mutex;

use kotone_core::log;
use tauri::AppHandle;

/// 等待时说的话（声网 `response_wait_ms` 1.5 s / 每轮最多一句；我们 1.2 s，见 `agent::FILLER_AFTER`）。
pub const WAITING: &[&str] = &["我看看", "稍等哈", "让我想想", "马上", "我查一下", "嗯……"];

/// 语音入口交给后台时这一步模型没说话：运行时马上念这句（ADR 029 第二期真机：DeepSeek 调用工具时
/// 常常不带正文，等它第二次调用再说要 4–5 秒）。
pub const HANDOFF: &str = "交给我，好了告诉你";

struct Warmed {
    /// 合成这些话时的“引擎 + 声音 + 语速”；变了就要重新合成。
    signature: String,
    ready: Vec<&'static str>,
    warming: bool,
}

static STATE: Mutex<Warmed> = Mutex::new(Warmed {
    signature: String::new(),
    ready: Vec::new(),
    warming: false,
});

fn phrases() -> impl Iterator<Item = &'static str> {
    crate::wake::ACKS
        .iter()
        .chain(WAITING.iter())
        .copied()
        .chain(std::iter::once(HANDOFF))
}

/// 落盘的目录：`<KOTONE_HOME>/cache/tts/`。
fn disk_dir() -> std::path::PathBuf {
    kotone_core::settings::kotone_dir()
        .join("cache")
        .join("tts")
}

/// 在后台把填充话合成进缓存（已经在合成就不重复）。启动、改设置、每个语音回合结束后调用。
/// 上次启动时合成过的从文件读，不花钱。
pub fn refresh(app: &AppHandle) {
    static DISK: std::sync::Once = std::sync::Once::new();
    DISK.call_once(|| {
        let texts: Vec<&str> = phrases().collect();
        kotone_tts::cache::disk::configure(disk_dir(), &texts);
    });
    let Some(signature) = crate::assistant::speech_signature(app) else {
        return;
    };
    let was_ready = {
        let mut state = STATE.lock().unwrap();
        if state.warming {
            return;
        }
        if state.signature != signature {
            state.signature = signature.clone();
            state.ready.clear();
        }
        state.warming = true;
        state.ready.len()
    };
    let app = app.clone();
    std::thread::spawn(move || {
        let started = std::time::Instant::now();
        let mut warmed = 0;
        for text in phrases() {
            if crate::assistant::speech_signature(&app).as_deref() != Some(signature.as_str()) {
                break;
            }
            match synthesize(&app, text) {
                Ok(()) => {
                    warmed += 1;
                    let mut state = STATE.lock().unwrap();
                    if state.signature == signature && !state.ready.contains(&text) {
                        state.ready.push(text);
                    }
                }
                Err(e) => log::log(&format!("fillers: “{text}”没能合成：{e}")),
            }
        }
        let total = phrases().count();
        // 启动后、换声音后第一次准备好时记一笔（从文件读的话是几毫秒）
        if warmed < total || was_ready < total {
            log::log(&format!(
                "fillers: {warmed}/{total} ready in {} ms",
                started.elapsed().as_millis()
            ));
        }
        STATE.lock().unwrap().warming = false;
    });
}

/// 合成一句进缓存；语音队列满（正在说话）时稍等再试。
fn synthesize(app: &AppHandle, text: &str) -> Result<(), String> {
    let mut last = String::new();
    for _ in 0..5 {
        match crate::assistant::synthesize_cached(app, text) {
            Ok(()) => return Ok(()),
            Err(e) if e.contains("队列已满") => {
                last = e;
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
            Err(e) => return Err(e),
        }
    }
    Err(last)
}

/// 现在能马上说的等待话：合成好的（系统语音不用合成，都能说）。
pub fn ready_waiting(app: &AppHandle) -> Vec<&'static str> {
    match crate::assistant::speech_signature(app) {
        None => WAITING.to_vec(),
        Some(signature) => {
            let state = STATE.lock().unwrap();
            if state.signature != signature {
                return Vec::new();
            }
            WAITING
                .iter()
                .copied()
                .filter(|t| state.ready.contains(t))
                .collect()
        }
    }
}

/// “交给我，好了告诉你”现在能马上说吗（合成好了；系统语音随时能说）。
pub fn handoff_ready(app: &AppHandle) -> bool {
    match crate::assistant::speech_signature(app) {
        None => true,
        Some(signature) => {
            let state = STATE.lock().unwrap();
            state.signature == signature && state.ready.contains(&HANDOFF)
        }
    }
}

/// 随机挑一句，不和上一句重复（只剩一句时除外）。纯函数。
pub fn pick(ready: &[&'static str], last: Option<&str>, seed: u64) -> Option<&'static str> {
    let fresh: Vec<&'static str> = ready.iter().copied().filter(|t| Some(*t) != last).collect();
    let pool = if fresh.is_empty() { ready } else { &fresh[..] };
    (!pool.is_empty()).then(|| pool[(seed % pool.len() as u64) as usize])
}

/// 随机数种子（不需要密码学随机）。
pub fn seed() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64 ^ d.as_secs())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_a_ready_phrase_and_avoids_repeating_the_last_one() {
        assert_eq!(pick(&[], None, 3), None);
        assert_eq!(pick(&["马上"], Some("马上"), 7), Some("马上"));
        for seed in 0..20 {
            let chosen = pick(&["我看看", "稍等哈", "马上"], Some("稍等哈"), seed).unwrap();
            assert_ne!(chosen, "稍等哈");
        }
        let all: std::collections::HashSet<_> = (0..20)
            .map(|seed| pick(WAITING, None, seed).unwrap())
            .collect();
        assert_eq!(all.len(), WAITING.len());
    }

    #[test]
    fn the_phrases_are_short_and_never_promise_a_result() {
        assert!(phrases().any(|t| t == HANDOFF), "交给后台那句也预先合成");
        for &text in crate::wake::ACKS.iter().chain(WAITING.iter()) {
            assert!(text.chars().count() <= 4, "{text}");
            for promise in ["好了", "找到", "没问题", "可以"] {
                assert!(!text.contains(promise), "{text}");
            }
        }
    }
}
