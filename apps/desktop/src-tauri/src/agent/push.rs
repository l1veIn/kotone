//! 推送（ADR 043 D3–D5）的桌面端：插件经 `Host::push` 发来一条（卡片 / 一行 / 语音），记进当前主会话的末尾，
//! **不叫模型**。以前叫自动卡片（ADR 039 D3）。开关和清单的核对在 `plugins.rs`（`DesktopHost::push`），这里管：
//!
//! - **新的还是更新**（`kotone_agent::push::decide`）：同一个 `插件:key` 内容一样什么都不做；变了就原地改那条（D5），
//!   位置不动；新的追加。上一次的内容记在内存里，Kotone 重启以后去当前对话里找。一次只处理一条推送（[`serial`]）。
//! - **发到哪段对话**：新的发到当前对话（是主会话的话）；没有就最近的主会话；一段都没有就开一段新的
//!   （[`target_conversation`]，和复盘那条同一个）。更新改在上一次那段对话里（那一条在哪就改哪）。
//! - **什么时候记进对话**：照“她正忙时插进来”的规矩（`when_free.rs`）：她空着就马上记；不空就排着，推送照样马上显示
//!   （界面事件 [`PUSH_EVENT`]），空下来再把推送和给她的那句一起追加。原地更新：那段对话不在跑就马上改，在跑就排着、
//!   这一轮结束再改；还排着的直接换掉。不插进正在跑的那一轮，也不叫她开口（ADR 039 D3）。
//! - **界面事件** [`PUSH_EVENT`]（聊天页、悬浮窗都听；形状见 [`emit_push`]）：新的一条、更新了、结束了（`Host::push_done`）。
//! - **悬浮窗**（ADR 043 D6，`floating/mod.rs`）：一行显示在气泡里；卡片推送、开着“推送时弹出小窗”（`agent.autoCards.popup`）
//!   就弹出小窗，带 `alert` 的更新再弹一次；`push_done` 收起那一行。点卡片上面那一行打开聊天、定位到这条（[`auto_card_open`]）。
//! - **语音**：说法轮着用（每条推送一个计数），照 `kotone_agent::push_voice` 的规矩排（同一时刻只说最要紧的、20 秒冷却、
//!   不打断、排了 5 秒过时），用她的声音说（`assistant::speak_push`）。更新时说法没变就不再说（结论没变不重复）。

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use kotone_agent::push::{self as pushes, Decision, Record};
use kotone_agent::push_voice::{Verdict, VoiceArbiter};
use kotone_plugin_api::{Push, PushOutcome, PushPriority, PushSpec};

use super::*;

/// 让主窗口定位到某条推送（界面 `chatController`）：`{conversationId, key}`。
pub const FOCUS_EVENT: &str = "kotone://auto-card-focus";
/// 推送的界面事件（聊天页、悬浮窗都听），形状见 [`emit_push`]。
pub const PUSH_EVENT: &str = "kotone://push";

/// 一次只处理一条推送：查上一次、记进对话、记下这一次是一件事（两条同 key 的同时来，后一条看得到前一条）。
/// 锁的先后：它在最外面（再“桌子”、再 `Inner`），别处不拿它。
fn serial() -> &'static Mutex<()> {
    static SERIAL: OnceLock<Mutex<()>> = OnceLock::new();
    SERIAL.get_or_init(Mutex::default)
}

/// 每条推送（`插件:trigger`）开口过几次：几个说法轮着用。
fn spoken_counts() -> &'static Mutex<HashMap<String, usize>> {
    static COUNTS: OnceLock<Mutex<HashMap<String, usize>>> = OnceLock::new();
    COUNTS.get_or_init(Mutex::default)
}

/// 语音推送的排队（全进程一份）。
fn arbiter() -> &'static Mutex<VoiceArbiter> {
    static ARBITER: OnceLock<Mutex<VoiceArbiter>> = OnceLock::new();
    ARBITER.get_or_init(Mutex::default)
}

/// 插件发来一条推送（`Host::push`；只有内置插件的宿主、开关和清单都核对过、`push` 已经按清单截过时才到这里）。
/// `plugin_name` 是清单里的名字。
pub fn post_push(
    app: &AppHandle,
    plugin: &str,
    plugin_name: &str,
    spec: &PushSpec,
    push: Push,
) -> Result<PushOutcome, String> {
    let settings = settings(app);
    if settings.ui.legacy_upgrade.uses_legacy_assistant() {
        // 0.1.x 升级上来、还没启用琴音：没有她的聊天
        return Ok(PushOutcome::Off);
    }
    let _one = serial().lock().unwrap();
    let id = pushes::id(plugin, &push.key);
    let remembered = desk().lock().unwrap().record(&id).cloned();
    let previous = match remembered {
        Some(r) => Some(r),
        // 内存里没有（Kotone 重启过）：看当前对话里有没有
        None => looked_up(app, &id)?,
    };
    // 说法变了（或者是新的）才开口；没变照旧写上次说的那句
    let speak = !push.voice.is_empty() && previous.as_ref().map_or(true, |p| p.voice != push.voice);
    let said = if speak {
        let mut counts = spoken_counts().lock().unwrap();
        let n = counts.entry(format!("{plugin}:{}", spec.id)).or_default();
        let said = pushes::pick(&push.voice, *n).to_string();
        *n += 1;
        said
    } else {
        previous
            .as_ref()
            .map(|p| p.said.clone())
            .unwrap_or_default()
    };
    let record = pushes::record(plugin, plugin_name, spec, &push, &said);
    let hash = pushes::content_hash(&record);
    let posted = match pushes::decide(previous, hash) {
        Decision::Unchanged => return Ok(PushOutcome::Unchanged),
        Decision::Post => post_new(app, plugin_name, &push, record, &id)?,
        Decision::Update(previous) => update(app, plugin_name, &push, record, &id, previous)?,
    };
    desk().lock().unwrap().remember(
        id,
        Record {
            conversation: posted.conversation.clone(),
            at_ms: posted.at_ms,
            hash,
            voice: push.voice.clone(),
            said: said.clone(),
            legacy: false,
        },
    );
    let title = pushes::shown_title(push_record(&posted.notice)).to_string();
    // 悬浮窗（ADR 043 D6）：一行显示在气泡里；新卡片、带 alert 的更新开着“推送时弹出小窗”时弹出小窗
    crate::floating::pushed(
        app,
        crate::floating::Pushed {
            plugin,
            plugin_name,
            conversation_id: &posted.conversation,
            key: push.key.trim(),
            line: push.line.as_ref(),
            until_done: spec.until_done,
            card: push.card.is_some(),
            posted: posted.outcome == PushOutcome::Posted,
            alert: push.alert,
        },
    );
    if speak && !said.is_empty() {
        // 同一条推送的更新（说法变了）不受冷却：三选一刷新以后最好的那张变了，玩家在等新的判断
        speak_in_order(
            app,
            spec.priority,
            said,
            posted.outcome == PushOutcome::Updated,
        );
    }
    log::log(&format!(
        "push: {plugin} {}「{title}」（{}，对话 {}{}）",
        match posted.outcome {
            PushOutcome::Updated => "更新了",
            _ => "推送",
        },
        push.key.trim(),
        posted.conversation,
        if posted.held {
            "，等她这一轮说完再记进对话"
        } else {
            ""
        }
    ));
    Ok(posted.outcome)
}

/// 记进（或排进）对话以后的样子。
struct Pushed {
    outcome: PushOutcome,
    conversation: String,
    at_ms: u64,
    notice: Message,
    /// 还排着（她在忙、那段对话在跑）。
    held: bool,
}

fn push_record(notice: &Message) -> &Value {
    static EMPTY: Value = Value::Null;
    pushes::push_of(notice).unwrap_or(&EMPTY)
}

/// 内存里没有这条（Kotone 重启过）：去当前对话里找上一次那条（新的推送、旧的自动卡片都认得）。
fn looked_up(app: &AppHandle, id: &str) -> Result<Option<Record>, String> {
    let store = store(app)?;
    let target = target_conversation(&store)?;
    Ok(store
        .load(&target)?
        .and_then(|c| pushes::find(&c.messages, id).and_then(|m| Record::of(&target, m))))
}

/// 新的一条：推送那条 + 给她的那句（有 note 才有），追加在当前主会话末尾。
fn post_new(
    app: &AppHandle,
    plugin_name: &str,
    push: &Push,
    record: Value,
    id: &str,
) -> Result<Pushed, String> {
    let store = store(app)?;
    let conversation = target_conversation(&store)?;
    let at_ms = now_ms();
    let title = pushes::shown_title(&record).to_string();
    let notice = pushes::notice(plugin_name, record, at_ms);
    let mut messages = vec![notice.clone()];
    if !push.note.is_empty() {
        messages.push(pushes::note_message(
            plugin_name,
            &title,
            &push.note,
            false,
            at_ms,
        ));
    }
    let held = match post(app, &conversation, &messages, IfBusy::Wait)? {
        Posted::Saved => false,
        Posted::Held | Posted::Steered => true,
        Posted::Gone => {
            desk().lock().unwrap().forget(id);
            return Err("那段对话刚被删了".into());
        }
    };
    emit_push(app, "posted", &conversation, &notice, held, false);
    Ok(Pushed {
        outcome: PushOutcome::Posted,
        conversation,
        at_ms,
        notice,
        held,
    })
}

/// 同一个 key、内容变了：原地改上一次那条（D5）。带了 note 再追加一条给她的；那段对话没了就当新的发。
fn update(
    app: &AppHandle,
    plugin_name: &str,
    push: &Push,
    mut record: Value,
    id: &str,
    previous: Record,
) -> Result<Pushed, String> {
    let now = now_ms();
    record["updatedAtMs"] = json!(now);
    let title = pushes::shown_title(&record).to_string();
    let notice = pushes::notice(plugin_name, record.clone(), previous.at_ms);
    let conversation = previous.conversation;
    let held = match replace(app, &conversation, &notice)? {
        Posted::Saved | Posted::Steered => false,
        Posted::Held => true,
        Posted::Gone => {
            // 上一次那段对话被删了：当成新的一条发到当前对话
            desk().lock().unwrap().forget(id);
            record.as_object_mut().map(|r| r.remove("updatedAtMs"));
            return post_new(app, plugin_name, push, record, id);
        }
    };
    emit_push(app, "updated", &conversation, &notice, held, push.alert);
    if !push.note.is_empty() {
        let note = pushes::note_message(plugin_name, &title, &push.note, true, now);
        if let Err(e) = post(app, &conversation, &[note], IfBusy::Wait) {
            log::log(&format!("push: 更新带的那句没记进对话：{e}"));
        }
    }
    Ok(Pushed {
        outcome: PushOutcome::Updated,
        conversation,
        at_ms: previous.at_ms,
        notice,
        held,
    })
}

/// 推送结束了（`Host::push_done`，例如三选一的卡片从屏幕上消失）：悬浮窗收起那一行，聊天里的记录留着。
pub fn push_done(app: &AppHandle, plugin: &str, trigger: &str, key: &str) {
    crate::floating::done(app, plugin, key);
    let _ = app.emit(
        PUSH_EVENT,
        json!({"kind": "done", "plugin": plugin, "trigger": trigger, "key": key.trim()}),
    );
}

/// 推送的界面事件 [`PUSH_EVENT`]（聊天页、悬浮窗都听）：
///
/// ```text
/// { kind: "posted" | "updated", conversationId, plugin, trigger, key,
///   held,      // 还排着没记进对话（她在忙）；界面照样马上显示
///   alert,     // 带 alert 的更新：悬浮窗把卡片再弹一次
///   message }  // 整条 Notice（role = notice，push 字段见 kotone_agent::push::record）
/// { kind: "done", plugin, trigger, key }        // Host::push_done
/// ```
///
/// `posted`：这个 key 第一次推（记进对话时还会照常来一条 `kotone://agent-event` 的 `appended`，同一条，界面按标识去重）。
/// `updated`：原地换掉同一个标识（`plugin:key`）的那条，位置不动。
fn emit_push(
    app: &AppHandle,
    kind: &str,
    conversation: &str,
    notice: &Message,
    held: bool,
    alert: bool,
) {
    let record = push_record(notice);
    let _ = app.emit(
        PUSH_EVENT,
        json!({
            "kind": kind,
            "conversationId": conversation,
            "plugin": record["plugin"],
            "trigger": record["trigger"],
            "key": record["key"],
            "held": held,
            "alert": alert,
            "message": notice,
        }),
    );
}

/// 推送的语音照规矩排（`kotone_agent::push_voice`），轮到了用她的声音说；过时了、被更要紧的换掉了就不说（只记日志：
/// 一行和聊天里的记录照旧）。
fn speak_in_order(app: &AppHandle, priority: PushPriority, text: String, update: bool) {
    let arrived = Instant::now();
    let offered = {
        let mut arbiter = arbiter().lock().unwrap();
        if update {
            arbiter.offer_update(priority, arrived)
        } else {
            arbiter.offer(priority, arrived)
        }
    };
    let ticket = match offered {
        Ok(ticket) => ticket,
        Err(why) => {
            log::log(&format!("push: 这句不说（{why}）：{text}"));
            return;
        }
    };
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_millis(50)).await;
            match arbiter().lock().unwrap().poll(ticket, Instant::now()) {
                Verdict::Wait => continue,
                Verdict::Speak => break,
                Verdict::Drop(why) => {
                    log::log(&format!("push: 这句不说（{why}）：{text}"));
                    return;
                }
            }
        }
        let spoken =
            crate::assistant::speak_push(&app, &text, VoiceArbiter::deadline(arrived)).await;
        match &spoken {
            Ok(true) => {}
            Ok(false) => log::log(&format!(
                "push: 这句不说（{}）：{text}",
                kotone_agent::push_voice::TOO_LATE
            )),
            Err(e) => log::log(&format!("push: 这句没说出来：{e}")),
        }
        arbiter()
            .lock()
            .unwrap()
            .finished(ticket, matches!(spoken, Ok(true)));
    });
}

/// 某段对话里排着、还没记进对话的推送（新的、排着的更新；界面重新读对话时照样显示：同一个标识的原地换掉，没有的放在
/// 最后面）。命令名是只有自动卡片时起的，不改。
#[tauri::command]
pub fn agent_held_cards(id: String) -> Vec<Message> {
    held_pushes(&id)
}

/// 玩家在悬浮窗里点了卡片上面那一行（或“打开 Kotone”）：打开 Kotone 的聊天页，定位到那条推送。这是玩家自己的动作，
/// 照“点托盘打开窗口”把主窗口拿到前面；她不在回答时顺便把那段对话切成当前对话（在回答时不切，和对话列表的规矩一样）。
/// 悬浮窗的小窗和几行收起（推送都在聊天里了）。命令名是只有自动卡片时起的，不改。
#[tauri::command]
pub fn auto_card_open(app: AppHandle, conversation_id: String, key: String) -> Result<(), String> {
    crate::floating::opened(&app);
    if let Some(win) = app.get_webview_window("main") {
        crate::show_and_focus(&win);
    }
    if !is_running(&app) {
        let store = store(&app)?;
        if store.load(&conversation_id)?.is_some() {
            store.set_current(Some(&conversation_id))?;
        }
    }
    let _ = app.emit_to(
        "main",
        FOCUS_EVENT,
        json!({"conversationId": conversation_id, "key": key}),
    );
    Ok(())
}
