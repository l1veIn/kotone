//! 一轮对话开始前要定的事：从哪个入口来、限制、思考强度、每轮附注、说话那一刻的屏幕、“停下”。

use super::*;

/// 语音入口的上限（ADR 014 §4：先给常用工具、限制步数，玩家在等）。语音入口也能查找工具、
/// 用 reply.ask 追问玩家（等回答要时间），所以比最初的 6 步 / 60 秒宽一些。
pub(super) fn game_limits() -> AgentLimits {
    AgentLimits {
        max_steps: 10,
        max_duration: std::time::Duration::from_secs(120),
        ..chat_limits()
    }
}

/// 聊天窗口的上限；两个入口一旦用上 game.look / game.act（一边看一边操作游戏），
/// 本轮放宽到 30 步、5 分钟（ADR 019 决定 6）。
pub(super) fn chat_limits() -> AgentLimits {
    AgentLimits {
        extend_on: kotone_agent::catalog::COMPUTER_USE_TOOLS
            .iter()
            .map(|t| t.to_string())
            .collect(),
        extended_steps: 30,
        extended_duration: std::time::Duration::from_secs(300),
        compact_at_tokens: kotone_core::agent::DEFAULT_COMPACT_AT_TOKENS,
        ..AgentLimits::default()
    }
}

/// 消息来源 → 入口：聊天窗口里打的字是聊天，其余（热键语音、快捷指令、工具）都在游戏里。
pub(super) fn entry_of(message: &Message) -> Entry {
    match message {
        Message::User { source, .. } => entry_of_source(source),
        _ => Entry::Game,
    }
}

pub(super) fn entry_of_source(source: &str) -> Entry {
    if source == "chat" || source == "assistant-text" {
        Entry::Chat
    } else {
        Entry::Game
    }
}

/// 玩家用嘴说的（经过语音识别）：唤醒、呼叫键、聊天页的语音按钮。
pub(super) fn is_spoken_source(source: &str) -> bool {
    matches!(source, "wake-voice" | "voice" | "assistant-text")
}

/// 语音入口的一轮：回答念完后接着听（ADR 029 §五）。快捷指令也算（玩家在游戏里、按键叫她）。
pub(super) fn follows_up(message: &Message) -> bool {
    matches!(message, Message::User { source, .. }
        if matches!(source.as_str(), "wake-voice" | "voice" | "quick-command"))
}

/// 这一轮从哪个入口来：后台交回的结果跟随玩家最近一次说话的方式（用嘴说的就开口说一句，打字的就写一句）。
pub(super) fn entry_for(app: &AppHandle, conversation: &str, user: &Message) -> Entry {
    if is_report(user) {
        return last_entry(app, conversation).unwrap_or(Entry::Chat);
    }
    entry_of(user)
}

pub(super) fn is_report(message: &Message) -> bool {
    matches!(message, Message::User { source, .. } if source == kotone_agent::subagents::REPORT_SOURCE)
}

pub(super) fn is_player_message(message: &Message) -> bool {
    matches!(message, Message::User { source, .. } if kotone_agent::subagents::is_player_source(source))
}

/// 交给模型的已保存自动化：没被隐藏、模型能调用（没有必填图片），最多 30 个。
pub(super) fn offered_automations(
    app: &AppHandle,
    hidden: &[String],
) -> Vec<kotone_core::automation::library::Automation> {
    crate::automation::saved_list(app)
        .into_iter()
        .filter(|a| !hidden.contains(&a.id))
        .filter(|a| saved_entry(a).is_some())
        .take(MAX_SAVED_TOOLS)
        .collect()
}

/// 思考强度（DeepSeek）：语音入口的主会话默认不思考（ADR 029 §一：玩家本机实测开着思考
/// “你能做什么”模型要 19.6 秒；Pipecat、xiaozhi 对 DeepSeek 也默认关掉），玩家在设置里明确选了
/// 才按他选的；子会话和聊天窗口照旧（空 = 服务商默认）。
pub(super) fn effort_for(entry: Entry, child: bool, game: &str, chat: &str) -> Option<String> {
    match entry {
        Entry::Game if !child && game.trim().is_empty() => Some("none".into()),
        Entry::Game => valid_effort(game),
        Entry::Chat => valid_effort(chat),
    }
}

/// 这句话的费用归属：这段对话、这句话开始的那一轮。
pub(super) fn turn_owner(conversation: &str, user: &Message) -> Option<kotone_core::usage::Owner> {
    match user {
        Message::User { at_ms, .. } => Some(kotone_core::usage::Owner {
            conversation: conversation.to_string(),
            turn: Some(*at_ms),
        }),
        _ => None,
    }
}

/// 主会话这一轮的附注：入口、现在几点、当前游戏；游戏变了（或第一次出现）再附上玩家写的游戏说明，
/// 系统提示里的游戏记忆不是这个游戏的就附上这个游戏的记忆。
pub(super) fn main_turn_context(
    app: &AppHandle,
    settings: &kotone_core::settings::Settings,
    entry: Entry,
    profile: &kotone_core::profile::GameProfile,
    snapshot: &kotone_agent::workspace::Snapshot,
    session: Option<&kotone_agent::port::Session>,
    ws: &Workspace,
) -> String {
    let voice_chat = settings.voice_chat.enabled;
    let game = profile.display_name.trim().to_string();
    let changed = match session {
        Some(session) => {
            let mut last = session.last_game.lock().unwrap();
            let changed = last.as_deref() != Some(profile.id.as_str());
            *last = Some(profile.id.clone());
            changed
        }
        None => true,
    };
    let snapshot_game = snapshot.game.as_ref().map(|(id, _)| id.as_str());
    let other_memory = (changed && snapshot_game != Some(profile.id.as_str()))
        .then(|| ws.game_memory(&profile.id))
        .flatten()
        .filter(|m| !m.trim().is_empty());
    // 系统提示或附注里已经有这个游戏的记忆时，不再附旧的“陪玩说明”。
    let has_memory = snapshot_game == Some(profile.id.as_str()) || other_memory.is_some();
    let notes = (changed && !has_memory)
        .then_some(profile.companion_notes.as_str())
        .filter(|n| !n.trim().is_empty());
    let now = kotone_core::agent::now_label();
    // 聊天窗口里才写“怎么叫她”（回答“你能做什么”、给下一步建议用）；语音入口玩家已经在叫她了。
    let hotkey = display_hotkey(settings.call_hotkey());
    let wake_phrase = (entry == Entry::Chat && settings.assistant.wake.enabled)
        .then(|| crate::roles::wake_keywords(app).1);
    let calling = (entry == Entry::Chat).then(|| Calling {
        hotkey: &hotkey,
        wake_phrase: wake_phrase.as_deref(),
        asr: asr_state(app, settings),
    });
    turn_context(&TurnContext {
        entry,
        game: &game,
        game_id: &profile.id,
        voice_chat,
        game_notes: notes,
        game_memory: other_memory.as_deref(),
        now: &now,
        calling,
    })
}

/// 呼叫键给玩家看的写法（`Backquote` → `~`）。
fn display_hotkey(combo: &str) -> String {
    combo.trim().replace("Backquote", "~")
}

/// 语音识别现在的情况：在线识别打开就绪；本地识别要启动（ADR 028“启动”只属于本地识别）。
fn asr_state(app: &AppHandle, settings: &kotone_core::settings::Settings) -> AsrState {
    use kotone_core::runtime::RuntimePhase;
    let model = kotone_stt::model::active_model_from(settings, &settings.stt_engine);
    let online = kotone_stt::model::is_remote_model(&model);
    let phase = app
        .try_state::<crate::runtime::RuntimeManager>()
        .map(|rt| rt.phase())
        .unwrap_or(RuntimePhase::Stopped);
    match (online, phase) {
        (true, RuntimePhase::Running) => AsrState::OnlineReady,
        (true, RuntimePhase::Starting) => AsrState::OnlinePreparing,
        (true, _) => AsrState::OnlineNotReady,
        (false, RuntimePhase::Running) => AsrState::LocalStarted,
        (false, RuntimePhase::Starting) => AsrState::LocalPreparing,
        (false, _) => AsrState::LocalNotStarted,
    }
}

/// 语音入口：附上玩家开口时的前台窗口（标题、程序名），告诉模型要看屏幕就调用 `screen.at_speech`
/// （ADR 029 §二）。以前这里每轮先截图 + 在线文字识别，玩家本机实测固定多等 12 秒。
pub(super) async fn attach_speech_screen(app: &AppHandle, user: &mut Message) {
    let Message::User {
        attachments,
        source,
        at_ms,
        ..
    } = user
    else {
        return;
    };
    if !attachments.is_empty() {
        return;
    }
    let text = match source.as_str() {
        "wake-voice" | "voice" => {
            let shot = crate::speech_screen::latest(app).await;
            crate::speech_screen::turn_note(shot.as_deref(), *at_ms)
        }
        // 快捷指令没有收音，也就没有开口时的截图。
        "quick-command" => crate::speech_screen::turn_note(None, *at_ms),
        _ => return,
    };
    attachments.push(Attachment::Data {
        label: crate::speech_screen::NOTE_LABEL.into(),
        text,
    });
}

/// 设置里的思考强度：只认 DeepSeek 接受的几个值，其余（含空）用服务商默认。
fn valid_effort(effort: &str) -> Option<String> {
    matches!(effort, "none" | "low" | "high" | "max").then(|| effort.to_string())
}

/// 玩家说的是不是“停下”（整句只有停止的意思，去掉标点、语气词和开头叫的名字：当前角色名或琴音）。
pub(super) fn is_stop_command(text: &str, name: &str) -> bool {
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
        .trim_end_matches(['吧', '啊', '呀', '了', '吗'])
        .to_lowercase();
    matches!(
        core.as_str(),
        "停" | "停下" | "停下来" | "停止" | "停一下" | "快停" | "别动" | "住手" | "不要动" | "stop"
    )
}

// ---------- 对话管理 ----------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::test_util::*;

    #[test]
    fn chat_window_text_is_chat_everything_else_is_in_game() {
        assert_eq!(entry_of(&user("chat")), Entry::Chat);
        assert_eq!(entry_of(&user("assistant-text")), Entry::Chat);
        for source in ["voice", "quick-command", "tool", "automation", ""] {
            assert_eq!(entry_of(&user(source)), Entry::Game, "{source}");
        }
    }

    #[test]
    fn voice_turns_do_not_think_unless_the_player_chose_to() {
        // 语音入口的主会话：空 = 不思考（ADR 029）
        assert_eq!(
            effort_for(Entry::Game, false, "", "").as_deref(),
            Some("none")
        );
        assert_eq!(
            effort_for(Entry::Game, false, " ", "high").as_deref(),
            Some("none")
        );
        // 玩家明确选了就照办
        assert_eq!(
            effort_for(Entry::Game, false, "high", "").as_deref(),
            Some("high")
        );
        assert_eq!(
            effort_for(Entry::Game, false, "none", "").as_deref(),
            Some("none")
        );
        // 子会话、聊天窗口：空 = 服务商默认（不变）
        assert_eq!(effort_for(Entry::Game, true, "", ""), None);
        assert_eq!(effort_for(Entry::Chat, false, "", ""), None);
        assert_eq!(
            effort_for(Entry::Chat, false, "", "low").as_deref(),
            Some("low")
        );
        assert_eq!(effort_for(Entry::Chat, false, "", "bogus"), None);
    }

    #[test]
    fn spoken_sources_and_follow_up_turns() {
        for source in ["wake-voice", "voice", "assistant-text"] {
            assert!(is_spoken_source(source), "{source}");
        }
        for source in ["chat", "quick-command", "tool", "automation"] {
            assert!(!is_spoken_source(source), "{source}");
        }
        // 呼叫键、唤醒、快捷指令（含接着听本身）答完接着听；聊天窗口、系统消息不接着听
        for source in ["wake-voice", "voice", "quick-command"] {
            assert!(follows_up(&user(source)), "{source}");
        }
        for source in ["chat", "assistant-text", "tool", "automation", "heard"] {
            assert!(!follows_up(&user(source)), "{source}");
        }
        let owner = turn_owner("c1", &user("voice")).unwrap();
        assert_eq!((owner.conversation.as_str(), owner.turn), ("c1", Some(0)));
        assert!(turn_owner("c1", &assistant("", &[])).is_none());
    }

    #[test]
    fn stop_commands_are_recognized_only_alone() {
        for t in [
            "停下",
            "停下！",
            "琴音，停下吧",
            "大肥鱼，停下",
            "别动了",
            "Stop.",
            "停一下",
            "住手",
        ] {
            assert!(is_stop_command(t, "大肥鱼"), "{t}");
        }
        for t in ["停下来帮我看看地图", "不要停", "帮我打开背包", "停车场在哪"]
        {
            assert!(!is_stop_command(t, "大肥鱼"), "{t}");
        }
    }

    #[test]
    fn computer_use_widens_both_entries_to_thirty_steps_and_five_minutes() {
        for limits in [game_limits(), chat_limits()] {
            assert!(limits.extend_on.iter().any(|t| t == "game.act"));
            assert_eq!(limits.extended_steps, 30);
            assert_eq!(limits.extended_duration.as_secs(), 300);
        }
    }

    #[test]
    fn game_limits_are_tighter_than_chat() {
        let game = game_limits();
        let chat = chat_limits();
        assert!(game.max_steps < chat.max_steps);
        assert!(game.max_duration < chat.max_duration);
    }
}
