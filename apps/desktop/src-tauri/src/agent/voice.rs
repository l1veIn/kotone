//! 语音回合的收尾：念 speak、交给后台时替她说“交给我”、等待短句（ADR 029）。

use super::*;

/// 语音入口只说话、只交给后台的那一步之后就结束这一轮（ADR 029 第八节）。
pub(super) const VOICE_STOP_TOOLS: &[&str] = &[
    kotone_agent::catalog::TOOL_SPEAK,
    kotone_agent::catalog::TOOL_AGENT_START,
    kotone_agent::catalog::TOOL_AGENT_FOLLOWUP,
];

/// 语音入口这一轮结束后补一句固定的话（纯函数，ADR 029 第八节）：模型的正文不念，
/// 所以出错、没做完、或者忘了用 speak 时，玩家至少知道去聊天里看。玩家自己停下的不说。
/// 自动化 `ask()` 交给她的一轮（`appended` 第一条是来源 `automation` 的消息）只写了字也不念“我写在聊天里了”：
/// 她可以不说（ADR 050 第 6 条），玩家也没在等她回答。
fn closing_line(appended: &[Message], stop: &StopReason) -> Option<&'static str> {
    let said = appended.iter().any(|m| match m {
        Message::Assistant { tool_calls, .. } => tool_calls.iter().any(|c| {
            c.name == kotone_agent::catalog::TOOL_SPEAK
                || c.name == kotone_agent::catalog::TOOL_ASK
                || kotone_agent::subagents::is_handoff_tool(&c.name)
        }),
        // 慢的工具转到了后台（ADR 029 §一，看存下来的标记，不看结果内容）：说过（或替她说过）“交给我”
        Message::Tool { background, .. } => *background,
        _ => false,
    });
    let wrote = appended
        .iter()
        .any(|m| matches!(m, Message::Assistant { text, .. } if !text.trim().is_empty()));
    match stop {
        StopReason::Cancelled => None,
        StopReason::Failed { .. } => Some(crate::fillers::FAILED_IN_CHAT),
        StopReason::StepLimit | StopReason::TimeLimit | StopReason::Truncated => {
            Some(crate::fillers::UNFINISHED_IN_CHAT)
        }
        StopReason::Completed
            if !said && wrote && !appended.first().is_some_and(is_automation_ask) =>
        {
            Some(crate::fillers::WROTE_IN_CHAT)
        }
        StopReason::Completed => None,
    }
}

/// 这一轮 Kotone 替她说了“交给我，好了告诉你”没有（派活、或者慢的工具 3 秒转到后台，那一步没 speak；
/// 纯函数）。那句还在合成、播放时这一轮可能已经结束了，计时由那句记，不能先丢掉。
fn handed_off(appended: &[Message]) -> bool {
    appended.iter().any(|m| match m {
        Message::Assistant { tool_calls, .. } => {
            tool_calls
                .iter()
                .any(|c| kotone_agent::subagents::is_handoff_tool(&c.name))
                && !tool_calls
                    .iter()
                    .any(|c| c.name == kotone_agent::catalog::TOOL_SPEAK)
        }
        Message::Tool { background, .. } => *background,
        _ => false,
    })
}

/// 这一轮里她说话时被玩家打断了没有（speak 的结果里写着，纯函数）。
fn interrupted(appended: &[Message]) -> bool {
    appended.iter().any(|m| {
        matches!(m, Message::Tool { name, content, .. }
            if name == kotone_agent::catalog::TOOL_SPEAK && content.contains("玩家打断了"))
    })
}

/// 这一轮她最后念出来的那句（纯函数）：最后一次 `speak` 的 text；之后又交给了后台、那一步没 speak 的，
/// 是 Kotone 替她说的“交给我”。用来判断她是不是问了一句（ADR 033 2026-11-26 修订）。
fn last_spoken(appended: &[Message]) -> Option<String> {
    let mut last = None;
    for message in appended {
        match message {
            Message::Assistant { tool_calls, .. } => {
                let said = tool_calls
                    .iter()
                    .filter(|c| c.name == kotone_agent::catalog::TOOL_SPEAK)
                    .filter_map(|c| {
                        serde_json::from_str::<Value>(&c.arguments)
                            .ok()?
                            .get("text")?
                            .as_str()
                            .map(str::to_string)
                    })
                    .next_back();
                if let Some(text) = said {
                    last = Some(text);
                } else if tool_calls
                    .iter()
                    .any(|c| kotone_agent::subagents::is_handoff_tool(&c.name))
                {
                    last = Some(crate::fillers::HANDOFF.to_string());
                }
            }
            Message::Tool {
                background: true, ..
            } => last = Some(crate::fillers::HANDOFF.to_string()),
            _ => {}
        }
    }
    last
}

/// 游戏内一轮结束：她的话已经用 speak 说过了（说完才结束这一轮）；需要时补一句固定的话，
/// 等播完再看还听不听（`follow`：语音入口正常答完；通话里接着听，说一句就走 / 录音笔里只在她问了一句之后，
/// 见 [`crate::follow_up`]）。
pub(super) fn finish_voice_turn(
    app: &AppHandle,
    appended: &[Message],
    stop: &StopReason,
    follow: bool,
) {
    let line = closing_line(appended, stop);
    if line == Some(crate::fillers::WROTE_IN_CHAT) {
        log::log("agent: speak-missing（语音回合没有用 speak 回答）");
    }
    // 玩家按键打断了她：不再接着听（他已经在说了）。
    let follow = follow && !interrupted(appended);
    let handed_off = handed_off(appended);
    // 她最后说的那句：补了固定的话就是那句
    let spoken = match line {
        Some(line) => line.to_string(),
        None => last_spoken(appended).unwrap_or_default(),
    };
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let epoch = crate::follow_up::epoch();
        let shown = match line {
            Some(line) => {
                match deliver_voice_reply(&app, line.to_string(), None, "closing").await {
                    Ok(_) => line,
                    Err(e) => {
                        log::log(&format!("agent: 没能播出：{e}"));
                        return;
                    }
                }
            }
            None => {
                crate::bubble::finished(&app);
                // 这一轮一句也没说（例如被停下）：计时不写。替她说了“交给我”的由那句记（2026-11-25 真机：
                // 慢的工具转到后台时这一轮马上结束，先丢掉计时就没有 voice-timing 那一行了）
                if !handed_off {
                    crate::voice_timing::discard();
                }
                // “交给我”、追问还在播：等它播完再接着听
                for _ in 0..300 {
                    if !crate::assistant::output_busy(&app) && !crate::voice_reply::active() {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
                ""
            }
        };
        if follow {
            crate::follow_up::after_reply(&app, epoch, shown, &spoken).await;
        }
        // 每个语音回合之后把填充话过一遍：命中缓存不花钱，也让它们不被挤出缓存。
        crate::fillers::refresh(&app);
    });
}

/// 语音入口里琴音说的话：开着对话气泡时，气泡显示这句（念完再停一会儿），默认不再走弹幕
/// （除非设置了“回答同时用弹幕”）；关着时照旧按提醒方式播报。
///
/// `owner`：合成费用记到哪一轮；`kind`：写进计时日志（`answer` / `preamble` / `say`）。
/// 要念的话先合成好再交给气泡和播放（字和声音一起出来，也能记下第一次出声的时刻）；
/// 合成期间玩家按呼叫键打断了就不念。
pub(super) async fn deliver_voice_reply(
    app: &AppHandle,
    text: String,
    owner: Option<kotone_core::usage::Owner>,
    kind: &'static str,
) -> Result<Vec<DeliveryKind>, String> {
    let (bubble, assistant) = {
        let settings = settings(app);
        (
            settings.assistant.bubble.clone(),
            settings.assistant.clone(),
        )
    };
    let wanted = assistant.default_delivery.clone();
    if wanted.contains(&DeliveryKind::Voice) {
        if let Some(owner) = owner {
            kotone_core::usage::claim_speech(&text, owner);
        }
        let epoch = crate::follow_up::epoch();
        crate::assistant::prepare_speech(app, &text).await;
        // 前面还在播（填充话、工具前那句）：等它播完。期间被呼叫键打断了就不念了，
        // 免得玩家说完话后又听到这句旧回答。
        crate::assistant::wait_output_idle(app, std::time::Duration::from_secs(30)).await;
        if crate::follow_up::epoch() != epoch {
            return Err("被呼叫键打断了".into());
        }
    }
    crate::voice_timing::sound(kind);
    crate::voice_reply::remember_spoken(&text);
    if !bubble.enabled {
        return crate::assistant::deliver_text(app, text, wanted).await;
    }
    let mut kinds = Vec::new();
    if wanted.contains(&DeliveryKind::Voice) {
        kinds.push(DeliveryKind::Voice);
    }
    if bubble.also_danmaku && wanted.contains(&DeliveryKind::Danmaku) {
        kinds.push(DeliveryKind::Danmaku);
    }
    let mut result = Ok(kinds.clone());
    let speak_app = app.clone();
    let speak_text = text.clone();
    crate::bubble::say(app, &text, || async {
        if !kinds.is_empty() {
            result = crate::assistant::deliver_text(&speak_app, speak_text, kinds).await;
        }
    })
    .await;
    result
}

/// 语音入口交给后台、这一步模型没说话：马上念预先合成的“交给我，好了告诉你”
/// （[`crate::fillers::HANDOFF`]；不经过模型，命中缓存不花钱）。这一轮之后不再接填充话。
pub(super) fn say_handoff(app: &AppHandle) {
    LAST_SAY_MS.store(now_ms(), std::sync::atomic::Ordering::SeqCst);
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        log::log(&format!(
            "agent: 交给后台，先说“{}”",
            crate::fillers::HANDOFF
        ));
        if let Err(e) =
            deliver_voice_reply(&app, crate::fillers::HANDOFF.to_string(), None, "handoff").await
        {
            log::log(&format!("agent: “交给我”没能播出：{e}"));
        }
    });
}

/// 交给琴音后多久还没出声就先接一句（ADR 017 §5；ADR 029 §四 从 1.5 s 提前到 1.2 s，
/// 照声网 `response_wait_ms` 每轮最多一句、回答先到就不说）。
const FILLER_AFTER: std::time::Duration = std::time::Duration::from_millis(1_200);
/// 上一次说的填充话（不连着说同一句）。
static LAST_FILLER: Mutex<Option<&'static str>> = Mutex::new(None);
/// 最近一次 `reply.say` / 追问 / 工具前那句的时间（毫秒）：这一轮里已经出过声就不再接话。
pub(super) static LAST_SAY_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// 游戏内一轮开始时调用：1.2 秒后这一轮还没结束、也没出过声，就随机说一句预先合成好的短话
/// （[`crate::fillers`]；不经过模型，不占步数，每轮最多一句）。没合成好的不说，正在播报时不说。
/// 返回的令牌在这一轮结束时取消。`agent.progressSpeech` 关掉时不说。
pub(super) fn progress_filler(
    app: &AppHandle,
    turn_cancel: &CancellationToken,
    owner: Option<kotone_core::usage::Owner>,
) -> CancellationToken {
    let done = CancellationToken::default();
    if !settings(app).agent.progress_speech {
        return done;
    }
    let started = now_ms();
    let app = app.clone();
    let done_wait = done.clone();
    let turn_cancel = turn_cancel.clone();
    tauri::async_runtime::spawn(async move {
        tokio::select! {
            _ = done_wait.cancelled() => return,
            _ = turn_cancel.cancelled() => return,
            _ = tokio::time::sleep(FILLER_AFTER) => {}
        }
        if LAST_SAY_MS.load(std::sync::atomic::Ordering::SeqCst) >= started
            || crate::assistant::output_busy(&app)
            || crate::bubble::is_speaking(&app)
            || crate::voice_reply::active()
        {
            return;
        }
        let ready = crate::fillers::ready_waiting(&app);
        let last = *LAST_FILLER.lock().unwrap();
        let Some(text) = crate::fillers::pick(&ready, last, crate::fillers::seed()) else {
            log::log("agent: 填充话还没合成好，这一轮不接话");
            return;
        };
        *LAST_FILLER.lock().unwrap() = Some(text);
        if let Some(owner) = owner {
            kotone_core::usage::claim_speech(text, owner);
        }
        log::log(&format!("agent: 先接一句“{text}”"));
        crate::voice_timing::filler_started(text);
        let result =
            crate::assistant::deliver_text(&app, text.to_string(), vec![DeliveryKind::Voice]).await;
        crate::voice_timing::filler_ended();
        if let Err(e) = result {
            log::log(&format!("agent: 接话未能播出：{e}"));
        }
    });
    done
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::test_util::*;
    use kotone_ai::agent::ToolCall;

    #[test]
    fn only_speak_is_spoken_and_a_closing_line_covers_the_rest() {
        let done = StopReason::Completed;
        let speak = |text: &str| assistant(text, &["speak"]);
        // 用 speak 说过：不补话；正文只进聊天
        let said = [user("voice"), speak("先出无尽，详细的写在聊天里了")];
        assert_eq!(closing_line(&said, &done), None);
        // 忘了 speak、写了正文：补一句“我写在聊天里了”
        let missing = [user("voice"), assistant("26.22，昨天刚更新。", &[])];
        assert_eq!(closing_line(&missing, &done), Some("我写在聊天里了"));
        // 什么也没写：不说空话
        assert_eq!(closing_line(&[user("voice")], &done), None);
        // 出错、没做完：固定的短句，原始错误留在聊天里
        let failed = StopReason::Failed {
            message: "AI 连接失败：HTTP 500".into(),
        };
        assert_eq!(closing_line(&said, &failed), Some("出错了，我写在聊天里了"));
        assert_eq!(
            closing_line(&said, &StopReason::StepLimit),
            Some("没做完，我写在聊天里了")
        );
        // 玩家自己停下：不说
        assert_eq!(closing_line(&missing, &StopReason::Cancelled), None);
        // 交给了后台（Kotone 替她说过“交给我”）、问过玩家：不补话
        let handed = [user("voice"), assistant("", &["agent.start"])];
        assert_eq!(closing_line(&handed, &done), None);
        // 慢的工具转到了后台（ADR 029 §一），正文写了一句也不补“我写在聊天里了”
        let moved = [
            user("voice"),
            assistant("我查一下", &["lol.scout"]),
            Message::Tool {
                call_id: "c".into(),
                name: "lol.scout".into(),
                content: kotone_agent::background::moved_result("lol-scout", "lol.scout")
                    .to_string(),
                is_error: false,
                untrusted: false,
                images: Vec::new(),
                display: None,
                background: true,
                at_ms: 0,
            },
        ];
        assert_eq!(closing_line(&moved, &done), None);
        // 第三方工具的结果正好长成“转到后台了”的样子、却没有标记：不算交给了后台，照常补“我写在聊天里了”
        let lookalike = [
            user("voice"),
            assistant("我查一下", &["github.search"]),
            Message::Tool {
                call_id: "c".into(),
                name: "github.search".into(),
                content: kotone_agent::background::moved_result("lol-scout", "lol.scout")
                    .to_string(),
                is_error: false,
                untrusted: true,
                images: Vec::new(),
                display: None,
                background: false,
                at_ms: 0,
            },
        ];
        assert_eq!(closing_line(&lookalike, &done), Some("我写在聊天里了"));
        assert!(!handed_off(&lookalike));
        // 自动化 ask() 交给她的一轮（ADR 050 第 6 条）：只写了字不念“我写在聊天里了”，什么都没写也不说；出错照旧说
        let asked_by_automation = [user("automation"), assistant("这把打得真好", &[])];
        assert_eq!(closing_line(&asked_by_automation, &done), None);
        assert_eq!(closing_line(&[user("automation")], &done), None);
        assert_eq!(
            closing_line(&[user("automation")], &failed),
            Some("出错了，我写在聊天里了")
        );
        // 替她说了“交给我”的：计时等那句记（派活、慢的工具转走）；她自己说了的、什么都没说的不算
        assert!(handed_off(&moved) && handed_off(&handed));
        assert!(!handed_off(&said) && !handed_off(&missing));
        assert!(!handed_off(&[
            user("voice"),
            assistant("", &["agent.start", "speak"])
        ]));
        let asked = [
            user("voice"),
            assistant("", &["reply.ask"]),
            assistant("好", &[]),
        ];
        assert_eq!(closing_line(&asked, &done), None);
    }

    #[test]
    fn the_last_thing_she_said_decides_whether_she_asked() {
        let speak = |text: &str| Message::Assistant {
            text: String::new(),
            tool_calls: vec![ToolCall {
                id: "c".into(),
                name: "speak".into(),
                arguments: serde_json::json!({"text": text, "written": "细节"}).to_string(),
            }],
            reasoning: String::new(),
            usage: None,
            at_ms: 0,
        };
        assert_eq!(last_spoken(&[user("voice")]), None);
        // 最后一次 speak 的 text（written 不念）
        let asked = [
            user("voice"),
            speak("我查一下"),
            assistant("", &["web.search"]),
            speak("今天有新版本，要我念一下改动吗？"),
        ];
        assert_eq!(
            last_spoken(&asked).as_deref(),
            Some("今天有新版本，要我念一下改动吗？")
        );
        // 问完又交给了后台（那一步没 speak）：最后说的是 Kotone 替她说的“交给我”
        let handed = [
            user("voice"),
            speak("要我帮你查吗？"),
            assistant("", &["agent.start"]),
        ];
        assert_eq!(
            last_spoken(&handed).as_deref(),
            Some(crate::fillers::HANDOFF)
        );
        assert!(!crate::follow_up::ends_with_question(
            crate::fillers::HANDOFF
        ));
        // 参数坏了：当作没说
        assert_eq!(
            last_spoken(&[user("voice"), assistant("", &["speak"])]),
            None
        );
    }

    #[test]
    fn an_interrupted_speak_stops_the_follow_up_listening() {
        let tool = |name: &str, content: &str| Message::Tool {
            call_id: "c".into(),
            name: name.into(),
            content: content.into(),
            is_error: false,
            untrusted: false,
            images: Vec::new(),
            display: None,
            background: false,
            at_ms: 0,
        };
        assert!(interrupted(&[tool(
            "speak",
            r#"{"result":"玩家打断了，只说到：先出无尽。"}"#
        )]));
        assert!(!interrupted(&[tool("speak", r#"{"result":"说完了"}"#)]));
        assert!(!interrupted(&[tool("web.search", "玩家打断了")]));
    }
}
