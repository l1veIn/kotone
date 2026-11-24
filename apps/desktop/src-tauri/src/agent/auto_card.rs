//! 自动卡片（ADR 039 D3）的桌面端：插件经 `Host::auto_card` 发来一张卡片，放进当前主会话的末尾，**不叫模型**。
//!
//! - 发到哪段对话：当前对话（是主会话的话）；没有就最近的主会话；一段都没有就开一段新的（照 `append_notice_when_idle`）。
//! - 什么时候记进对话：琴音空着（[`main_is_quiet`]：她没在回答、没在说话、没有提问在等，玩家没在说话——ADR 029
//!   “做完主动说”用的同一个判断）就马上记；不空就先压着（`kotone_agent::auto_card::Desk`），卡片照样马上显示在
//!   聊天里（界面事件 `autoCard`），等空下来再把卡片和给她的那句一起追加到对话末尾。不插进正在跑的那一轮，也不叫
//!   她开口（ADR 039 D3）。
//! - 消息提示：开着就弹（`crate::message_popup`）；点它打开聊天、定位到这张卡片（[`auto_card_open`]）。

use std::sync::OnceLock;

use kotone_agent::auto_card::{self as cards, Decision, Desk};
use kotone_plugin_api::AutoCard;

use super::*;

/// 等琴音空下来多久检查一次。
const FLUSH_EVERY: std::time::Duration = std::time::Duration::from_millis(500);
/// 一直不空（玩家一直在和她说话）最多等多久：之后只等她不在回答（这时记进对话不会被覆盖），不再等她说完。
const QUIET_WAIT: std::time::Duration = std::time::Duration::from_secs(600);
/// 让主窗口定位到某张卡片（界面 `chatController`）。
pub const FOCUS_EVENT: &str = "kotone://auto-card-focus";

fn desk() -> &'static Mutex<Desk> {
    static DESK: OnceLock<Mutex<Desk>> = OnceLock::new();
    DESK.get_or_init(Default::default)
}

/// 插件发来一张自动卡片（`Host::auto_card`，只有内置插件的宿主会调到这里）。`plugin_name` 是清单里的名字。
pub fn post_auto_card(
    app: &AppHandle,
    plugin: &str,
    plugin_name: &str,
    card: AutoCard,
) -> Result<bool, String> {
    let settings = settings(app);
    if settings.ui.legacy_upgrade.uses_legacy_assistant() {
        // 0.1.x 升级上来、还没启用琴音：没有她的聊天
        return Ok(false);
    }
    let popup = match desk()
        .lock()
        .unwrap()
        .decide(&settings.agent.auto_cards, plugin, &card.key)
    {
        Decision::Off => {
            log::log(&format!(
                "auto card: {plugin} {} 没发（玩家关了自动卡片）",
                card.key
            ));
            return Ok(false);
        }
        Decision::Duplicate => return Ok(false),
        Decision::Post { popup } => popup,
    };
    let store = store(app)?;
    let conversation_id = target_conversation(&store)?;
    let at_ms = now_ms();
    let messages = vec![
        cards::card_message(plugin, plugin_name, &card, at_ms),
        cards::note_message(plugin_name, &card, at_ms),
    ];
    let shown = messages[0].clone();
    let now = main_is_quiet(app) && append(app, &conversation_id, &messages);
    if !now {
        // 她在忙：卡片马上显示，两条等这一轮结束再记进对话
        desk().lock().unwrap().hold(&conversation_id, messages);
        emit(
            app,
            &conversation_id,
            json!({"kind": "autoCard", "message": shown}),
        );
        schedule_flush(app);
    }
    log::log(&format!(
        "auto card: {plugin} 发来「{}」（{}，对话 {conversation_id}{}）",
        card.title,
        card.key,
        if now {
            ""
        } else {
            "，等她这一轮说完再记进对话"
        }
    ));
    if popup {
        crate::message_popup::show(
            app,
            crate::message_popup::Popup {
                plugin: plugin.to_string(),
                plugin_name: plugin_name.to_string(),
                name: companion_name(&settings),
                text: cards::popup_text(plugin_name, &card),
                conversation_id,
                key: card.key.trim().to_string(),
                revision: 0,
            },
        );
    }
    Ok(true)
}

/// 琴音现在叫什么（消息提示上那一行）。
fn companion_name(settings: &kotone_core::settings::Settings) -> String {
    let name = settings.agent.identity.name.trim();
    if name.is_empty() {
        kotone_agent::workspace::DEFAULT_NAME.to_string()
    } else {
        name.to_string()
    }
}

/// 发到哪段对话：当前的主会话；没有就最近的主会话；一段都没有就开一段新的。
fn target_conversation(store: &ConversationStore) -> Result<String, String> {
    if let Some(id) = store.current() {
        if store.load(&id)?.is_some_and(|c| c.parent.is_none()) {
            return Ok(id);
        }
    }
    if let Some(summary) = store.list().into_iter().find(|c| c.parent.is_none()) {
        return Ok(summary.id);
    }
    let mut conversation = store.create();
    store.save(&mut conversation)?;
    store.set_current(Some(&conversation.id))?;
    Ok(conversation.id)
}

/// 追加到对话末尾并通知界面。主会话正在跑一轮时不写（那一轮存盘时会把这里写的盖掉），返回 false。
fn append(app: &AppHandle, id: &str, messages: &[Message]) -> bool {
    let (Ok(rt), Ok(store)) = (runtime(app), store(app)) else {
        return false;
    };
    // 和 send_message 同一把锁：确认没在跑再写
    let inner = rt.inner.lock().unwrap();
    if inner.running.is_some() {
        return false;
    }
    let mut conversation = match store.load(id) {
        Ok(Some(c)) => c,
        Ok(None) => return true, // 那段对话没了（被删了）：不记了
        Err(e) => {
            log::log(&format!("auto card: load failed: {e}"));
            return false;
        }
    };
    conversation.messages.extend(messages.iter().cloned());
    if let Err(e) = store.save(&mut conversation) {
        log::log(&format!("auto card: save failed: {e}"));
        return false;
    }
    drop(inner);
    for message in messages {
        emit(
            app,
            id,
            event_json(&AgentEvent::Appended {
                message: message.clone(),
            }),
        );
    }
    true
}

/// 压着的卡片：等琴音空下来再记进对话（全进程一个循环）。
fn schedule_flush(app: &AppHandle) {
    static FLUSHING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if FLUSHING.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let started = std::time::Instant::now();
        loop {
            tokio::time::sleep(FLUSH_EVERY).await;
            let ready = main_is_quiet(&app) || started.elapsed() >= QUIET_WAIT;
            if ready && flush(&app) {
                FLUSHING.store(false, std::sync::atomic::Ordering::SeqCst);
                // 交还以后又压进来的（同一把锁外，极少）：再排一次
                if desk().lock().unwrap().has_held() {
                    schedule_flush(&app);
                }
                return;
            }
        }
    });
}

/// 把压着的都记进对话；她又开始跑了就放回去下次再记。全记完返回 true。
fn flush(app: &AppHandle) -> bool {
    let held = desk().lock().unwrap().take();
    let mut rest = held.into_iter();
    for h in rest.by_ref() {
        if !append(app, &h.conversation, &h.messages) {
            let mut d = desk().lock().unwrap();
            d.hold(&h.conversation, h.messages);
            for later in rest {
                d.hold(&later.conversation, later.messages);
            }
            return false;
        }
        log::log(&format!(
            "auto card: 记进对话 {}（她这一轮说完了）",
            h.conversation
        ));
    }
    true
}

/// 某段对话里压着、还没记进对话的卡片（界面重新读对话时照样显示）。
#[tauri::command]
pub fn agent_held_cards(id: String) -> Vec<Message> {
    desk().lock().unwrap().held_cards(&id)
}

/// 玩家点了消息提示：打开 Kotone 的聊天页，定位到那张卡片。这是玩家自己的动作，照“点托盘打开窗口”把主窗口
/// 拿到前面；她不在回答时顺便把那段对话切成当前对话（在回答时不切，和对话列表的规矩一样）。
#[tauri::command]
pub fn auto_card_open(app: AppHandle, conversation_id: String, key: String) -> Result<(), String> {
    crate::message_popup::hide(&app);
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.show();
        let _ = win.unminimize();
        let _ = win.set_focus();
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
