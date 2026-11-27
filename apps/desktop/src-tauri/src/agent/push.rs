//! 推送（ADR 043 D3–D5）的桌面端：插件经 `Host::push` 发来一条（卡片 / 一行 / 语音），记进当前主会话的末尾，
//! **不叫模型**。以前叫自动卡片（ADR 039 D3）。开关和清单的核对在 `plugins.rs`（`DesktopHost::push`），这里管：
//!
//! - **新的还是更新**（`kotone_agent::push::decide`）：同一个 `插件:key` 内容一样什么都不做；变了就原地改那条（D5），
//!   位置不动、哪里都不滚过去；**带 `alert` 的更新挪到那段对话的最后**（D5 修订，2026-11-27，[`move_to_bottom`]：
//!   原地改在最早那条的位置，悬浮窗弹出来定位到它就跳回开局附近）；新的追加。上一次的内容记在内存里，Kotone 重启以后去
//!   当前对话里找。一次只处理一条推送（[`serial`]）。
//! - **发到哪段对话**：新的发到当前对话（是主会话的话）；没有就最近的主会话；一段都没有就开一段新的
//!   （[`target_conversation`]，和复盘那条同一个）。更新改在（挪到）上一次那段对话里（那一条在哪段对话就是哪段）。
//! - **什么时候记进对话**：照“她正忙时插进来”的规矩（`when_free.rs`）：她空着就马上记；不空就排着，推送照样马上显示
//!   （界面事件 [`PUSH_EVENT`]），空下来再把推送和给她的那句一起追加。原地更新、挪到最后：那段对话不在跑就马上改，在跑
//!   就排着、这一轮结束再改；还排着的直接换掉。不插进正在跑的那一轮，也不叫她开口（ADR 039 D3）。
//! - **界面事件** [`PUSH_EVENT`]（聊天页、悬浮窗都听；形状见 [`emit_push`]）：新的一条、原地更新了、挪到最后了、
//!   结束了（`Host::push_done`）。
//! - **悬浮窗**（ADR 043 D6，`floating/mod.rs`）：一行显示在气泡里；卡片推送、开着“推送时弹出小窗”（`agent.autoCards.popup`）
//!   就弹出小窗，带 `alert` 的更新再弹一次；`push_done` 收起那一行。点卡片上面那一行打开聊天、定位到这条（[`auto_card_open`]）。
//! - **语音**：说法轮着用（每条推送一个计数），照 `kotone_agent::push_voice` 的规矩排（同一时刻只说最要紧的、20 秒冷却、
//!   不打断、排了 5 秒过时），用她的声音说（`assistant::speak_push`）。更新时说法没变就不再说（结论没变不重复）。
//! - **自动化也走这条路**（ADR 045 D2，程序里的 `push(text)`，[`post_automation_push`]）：不经清单核对，宿主照插件推送
//!   的样子合成一条只有一行的推送；给她的那句（不叫模型）说是哪个自动化推的。

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use kotone_agent::push::{self as pushes, Decision, Record};
use kotone_agent::push_voice::{Verdict, VoiceArbiter};
use kotone_plugin_api::{Push, PushCell, PushForm, PushLine, PushOutcome, PushPriority, PushSpec};

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

/// 推送是谁发的：给她的那句（`note`）说法不一样。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Sender {
    Plugin,
    Automation,
}

/// 自动化推送在聊天记录里的插件 id（界面按它显示本地化的“自动化”，ADR 045 D2）。
pub const AUTOMATION_PUSH_PLUGIN: &str = "automation";
/// 自动化推送记录里的 `pluginName`（旧版本、悬浮窗没有本地化文案时看到的字）。
pub const AUTOMATION_PUSH_NAME: &str = "自动化";
/// 临时程序（没保存成自动化）推送的 `trigger`。
const ADHOC_TRIGGER: &str = "adhoc";

/// 程序里的 `push(text)`（ADR 045 D2）：和插件推送同一条路，Kotone 聊天里记一条、悬浮窗显示一行，不调用模型、不出声；
/// 附一条不叫模型的说明（[`automation_note`]），她之后在对话里看得到。她正忙时先排着，界面照样马上显示。
/// 合成的样子见 [`automation_push`]；`key` 每条都是新的（`<执行记录 id>-<第几条>`）。
/// 0.1.x 升级上来、还没启用琴音时没有她的聊天：报错。
pub fn post_automation_push(
    app: &AppHandle,
    automation_id: Option<&str>,
    name: &str,
    key: &str,
    text: &str,
) -> Result<(), String> {
    let (spec, push) = automation_push(automation_id, name, key, text);
    match post_from(
        app,
        Sender::Automation,
        AUTOMATION_PUSH_PLUGIN,
        AUTOMATION_PUSH_NAME,
        &spec,
        push,
    )? {
        PushOutcome::Off => {
            Err("还没启用琴音，没有聊天，push() 用不了：先在聊天页添加一个 DeepSeek Key".into())
        }
        _ => Ok(()),
    }
}

/// 自动化推送的样子：插件 `automation`，清单里那一条的 id = 自动化 id（临时程序是 `adhoc`）、名字 = 自动化的名字，
/// 只有一行（显示几秒就收起），一格就是推的那句（最多 40 字，[`PushCell::ONE_CELL_MAX_CHARS`]）；给她的那句也是它。
fn automation_push(
    automation_id: Option<&str>,
    name: &str,
    key: &str,
    text: &str,
) -> (PushSpec, Push) {
    let id = automation_id.unwrap_or(ADHOC_TRIGGER).to_string();
    let title = match name.trim() {
        "" => AUTOMATION_PUSH_NAME.to_string(),
        name => name.to_string(),
    };
    let spec = PushSpec {
        id: id.clone(),
        title,
        when: String::new(),
        what: String::new(),
        forms: vec![PushForm::Line],
        until_done: false,
        priority: PushPriority::Normal,
        enabled_by_default: true,
        needs: Vec::new(),
    };
    let push = Push {
        trigger: id,
        key: key.to_string(),
        line: Some(PushLine {
            cells: vec![PushCell {
                text: text.to_string(),
                tone: None,
            }],
        }),
        note: text.to_string(),
        ..Push::default()
    }
    .shaped(&spec);
    (spec, push)
}

/// 自动化推送给她的那句（不叫模型；她之后在对话里看得到）：说法照插件推送（“系统消息，不是玩家说的话”），推的字按数据
/// 附上（可能有从游戏事件填进来的、别人起的名字）。
fn automation_note(name: &str, text: &str, at_ms: u64) -> Message {
    Message::User {
        text: format!(
            "（系统消息，不是玩家说的话：自动化「{name}」推送了一条，玩家在聊天里看得到；内容见附件。）"
        ),
        attachments: vec![Attachment::Data {
            label: format!("{AUTOMATION_PUSH_NAME} · {name}"),
            text: text.to_string(),
        }],
        source: pushes::SOURCE.into(),
        at_ms,
        context: String::new(),
    }
}

/// 给她的那句（推送带了 `note` 才有）。
fn note_for(
    from: Sender,
    plugin_name: &str,
    title: &str,
    note: &str,
    updated: bool,
    at_ms: u64,
) -> Message {
    match from {
        Sender::Plugin => pushes::note_message(plugin_name, title, note, updated, at_ms),
        Sender::Automation => automation_note(title, note, at_ms),
    }
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
    post_from(app, Sender::Plugin, plugin, plugin_name, spec, push)
}

fn post_from(
    app: &AppHandle,
    from: Sender,
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
        // 自动化的 key 每条都是新的（带执行记录 id）：不用去对话里找
        None if from == Sender::Automation => None,
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
    let posted = match pushes::decide(previous, hash, push.alert) {
        Decision::Unchanged => return Ok(PushOutcome::Unchanged),
        Decision::Post => post_new(app, from, plugin_name, &push, record, &id)?,
        Decision::Update(previous) => update(app, from, plugin_name, &push, record, &id, previous)?,
        Decision::Move(previous) => {
            move_to_bottom(app, from, plugin_name, &push, record, &id, previous)?
        }
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
            PushOutcome::Updated if posted.moved => "更新了（挪到最后）",
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
    /// 带提醒的更新：挪到了那段对话的最后。
    moved: bool,
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
    from: Sender,
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
        messages.push(note_for(
            from,
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
        moved: false,
    })
}

/// 同一个 key、内容变了、不带 alert：原地改上一次那条（D5），位置不动。带了 note 再追加一条给她的；那段对话没了就当新的发。
fn update(
    app: &AppHandle,
    from: Sender,
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
            return post_new(app, from, plugin_name, push, record, id);
        }
    };
    emit_push(app, "updated", &conversation, &notice, held, push.alert);
    if !push.note.is_empty() {
        let note = note_for(from, plugin_name, &title, &push.note, true, now);
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
        moved: false,
    })
}

/// 同一个 key、内容变了、带 alert（插件要提醒玩家，例如又阵亡了一次）：挪到上一次那段对话的最后（D5 修订）——拿掉原来
/// 那条和跟着它的那句，用新内容、新时间追加在末尾，同一个 `插件:key` 仍然只有一条；带了 note 就是这次的那句跟在后面，
/// 没带就把原来那句挪过来（`kotone_agent::push::apply_move`）。于是聊天页、悬浮窗看到它都在最新的位置，不跳回原处。
/// 那段对话没了就当新的发。
fn move_to_bottom(
    app: &AppHandle,
    from: Sender,
    plugin_name: &str,
    push: &Push,
    mut record: Value,
    id: &str,
    previous: Record,
) -> Result<Pushed, String> {
    let now = now_ms();
    record["updatedAtMs"] = json!(now);
    let title = pushes::shown_title(&record).to_string();
    let notice = pushes::notice(plugin_name, record.clone(), now);
    let mut moved = vec![notice.clone()];
    if !push.note.is_empty() {
        moved.push(note_for(from, plugin_name, &title, &push.note, true, now));
    }
    let conversation = previous.conversation;
    let held = match move_to_end(app, &conversation, &moved)? {
        Posted::Saved | Posted::Steered => false,
        Posted::Held => true,
        Posted::Gone => {
            // 上一次那段对话被删了：当成新的一条发到当前对话
            desk().lock().unwrap().forget(id);
            record.as_object_mut().map(|r| r.remove("updatedAtMs"));
            return post_new(app, from, plugin_name, push, record, id);
        }
    };
    emit_push(app, "moved", &conversation, &notice, held, true);
    Ok(Pushed {
        outcome: PushOutcome::Updated,
        conversation,
        at_ms: now,
        notice,
        held,
        moved: true,
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
/// { kind: "posted" | "updated" | "moved", conversationId, plugin, trigger, key,
///   held,      // 还排着没记进对话（她在忙）；界面照样马上显示
///   alert,     // 带 alert 的更新（moved 总是 true）：悬浮窗把卡片再弹一次
///   message }  // 整条 Notice（role = notice，push 字段见 kotone_agent::push::record）
/// { kind: "done", plugin, trigger, key }        // Host::push_done
/// ```
///
/// `posted`：这个 key 第一次推（记进对话时还会照常来一条 `kotone://agent-event` 的 `appended`，同一条，界面按标识去重）。
/// `updated`：原地换掉同一个标识（`plugin:key`）的那条，位置不动、时间不变，哪里都不滚过去。
/// `moved`：带提醒的更新（D5 修订）：从原处拿掉同一个标识的那条，这条放在最后（`message` 的时间是挪的那一刻）。
/// 记进对话以后不再来 `appended`（和 `updated` 一样）。
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
/// 开局战绩展开到英雄联盟插件的「这局」。悬浮窗的小窗和几行一起收起。
#[tauri::command]
pub fn auto_card_open(
    app: AppHandle,
    conversation_id: String,
    key: String,
    open_match: Option<bool>,
) -> Result<(), String> {
    crate::floating::opened(&app);
    if let Some(win) = app.get_webview_window("main") {
        crate::show_and_focus(&win);
    }
    if open_match == Some(true) {
        return app
            .emit_to(
                "main",
                crate::plugins::UI_OPEN_EVENT,
                json!({"uri": kotone_plugin_lol::pages::PAGE_MATCH, "plugin": "lol", "byPlayer": true, "standalone": true}),
            )
            .map_err(|error| error.to_string());
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试清单 19：自动化的 `push()` 在聊天里的记录和插件推送同一个形状（界面照 `PushRecord` 画、悬浮窗显示一行）：
    /// 插件 `automation`、trigger = 自动化 id（临时程序 `adhoc`）、title = 自动化名字、只有一行、key 是这次给的。
    #[test]
    fn an_automation_push_is_a_one_line_push_from_the_automation_plugin() {
        let (spec, push) = automation_push(Some("a-1"), "提醒买眼", "run-7-2", " 记得买真眼 ");
        let record = pushes::record(
            AUTOMATION_PUSH_PLUGIN,
            AUTOMATION_PUSH_NAME,
            &spec,
            &push,
            "",
        );
        assert_eq!(
            record,
            json!({
                "plugin": "automation", "pluginName": "自动化", "trigger": "a-1", "title": "提醒买眼",
                "key": "run-7-2", "line": {"cells": [{"text": "记得买真眼"}]},
            })
        );
        assert!(!spec.until_done, "一行显示几秒就收起");
        assert!(
            push.voice.is_empty() && push.card.is_none(),
            "不出声、没有卡片"
        );
        assert_eq!(
            pushes::id(AUTOMATION_PUSH_PLUGIN, &push.key),
            "automation:run-7-2"
        );

        // 临时程序、没名字的
        let (spec, push) = automation_push(None, " ", "run-8-1", "x");
        assert_eq!(
            (spec.id.as_str(), push.trigger.as_str()),
            ("adhoc", "adhoc")
        );
        assert_eq!(spec.title, "自动化");
        // 一行最多 40 字（脚本引擎已经挡过，这里再截一次）
        let (_, push) = automation_push(None, "n", "k", &"长".repeat(60));
        let cell = &push.line.unwrap().cells[0].text;
        assert_eq!(cell.chars().count(), PushCell::ONE_CELL_MAX_CHARS);
    }

    /// 给她的那句（不叫模型、界面不显示）：说是哪个自动化推的，推的字按数据附上（可能有别人起的名字）。
    #[test]
    fn the_note_for_her_says_which_automation_pushed_and_attaches_the_text() {
        let Message::User {
            text,
            attachments,
            source,
            ..
        } = note_for(
            Sender::Automation,
            AUTOMATION_PUSH_NAME,
            "提醒买眼",
            "某人拿了龙",
            false,
            1,
        )
        else {
            panic!("要是给她的用户消息")
        };
        assert_eq!(source, pushes::SOURCE);
        assert!(
            text.starts_with("（系统消息，不是玩家说的话：自动化「提醒买眼」推送了一条"),
            "{text}"
        );
        assert!(!text.contains("某人"), "推的字不进正文");
        assert!(!text.contains("插件"), "自动化不是插件");
        assert!(matches!(
            &attachments[..],
            [Attachment::Data { label, text }] if label == "自动化 · 提醒买眼" && text == "某人拿了龙"
        ));
        // 插件的照旧
        let Message::User { text, .. } =
            note_for(Sender::Plugin, "英雄联盟", "开局战绩", "x", false, 1)
        else {
            panic!()
        };
        assert!(
            text.contains("英雄联盟插件发来一条推送「开局战绩」"),
            "{text}"
        );
    }
}
