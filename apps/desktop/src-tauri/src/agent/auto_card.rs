//! 自动卡片（ADR 039 D3）的桌面端：插件经 `Host::auto_card` 发来一张卡片，放进当前主会话的末尾，**不叫模型**。
//!
//! - 发到哪段对话：当前对话（是主会话的话）；没有就最近的主会话；一段都没有就开一段新的（照 `append_notice_when_idle`）。
//! - 什么时候记进对话：琴音空着（[`main_is_quiet`]：她没在回答、没在说话、没有提问在等，玩家没在说话——ADR 029
//!   “做完主动说”用的同一个判断）就马上记；不空就先压着（`kotone_agent::auto_card::Desk`），卡片照样马上显示在
//!   聊天里（界面事件 `autoCard`），等空下来再把卡片和给她的那句一起追加到对话末尾。不插进正在跑的那一轮，也不叫
//!   她开口（ADR 039 D3）。压着的存一份在 `agent/auto-cards-held.json`：这期间 Kotone 退出了，下次启动接着记
//!   （[`resume_held_cards`]）。记进对话一直出错就试几次后放弃（`cards::MAX_FAILURES`），只记一次日志。
//! - 记进对话都走 [`append_to_conversation`]（和 `send_message` 认领主会话同一把锁），不会被正要开始的一轮盖掉。
//! - 去重：贴出去（记进对话或压着）了才算发过；Kotone 重启以后看对话里有没有同插件同键的卡片。
//! - 消息提示：开着就弹（`crate::message_popup`）；点它打开聊天、定位到这张卡片（[`auto_card_open`]）。

use std::sync::OnceLock;

use kotone_agent::auto_card::{self as cards, Decision, Desk, Held};
use kotone_plugin_api::AutoCard;

use super::*;

/// 等琴音空下来多久检查一次。
const FLUSH_EVERY: std::time::Duration = std::time::Duration::from_millis(500);
/// 一直不空（玩家一直在和她说话）最多等多久：之后只等她不在回答（这时记进对话不会被覆盖），不再等她说完。
const QUIET_WAIT: std::time::Duration = std::time::Duration::from_secs(600);
/// 让主窗口定位到某张卡片（界面 `chatController`）。
pub const FOCUS_EVENT: &str = "kotone://auto-card-focus";

/// 压着的卡片存在哪（只有压着时有这个文件）。
fn held_path() -> std::path::PathBuf {
    kotone_core::settings::kotone_dir()
        .join("agent")
        .join("auto-cards-held.json")
}

/// 第一次用时接上盘上存着的压着的卡片（上次退出时还没记进对话的）。
fn desk() -> &'static Mutex<Desk> {
    static DESK: OnceLock<Mutex<Desk>> = OnceLock::new();
    DESK.get_or_init(|| {
        let mut desk = Desk::default();
        if let Ok(bytes) = std::fs::read(held_path()) {
            match serde_json::from_slice::<Vec<Held>>(&bytes) {
                Ok(held) => desk.restore(held),
                Err(e) => log::log(&format!("auto card: 压着的卡片读不了，不要了：{e}")),
            }
        }
        Mutex::new(desk)
    })
}

/// 把压着的写到盘上（空了就删掉文件）。调用方拿着 `desk()` 的锁。
fn save_held(desk: &Desk) {
    let path = held_path();
    let result = if desk.held().is_empty() {
        match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
            _ => Ok(()),
        }
    } else {
        serde_json::to_vec(desk.held())
            .map_err(|e| e.to_string())
            .and_then(|bytes| {
                let tmp = path.with_extension("json.tmp");
                std::fs::write(&tmp, bytes)
                    .and_then(|_| std::fs::rename(&tmp, &path))
                    .map_err(|e| e.to_string())
            })
    };
    if let Err(e) = result {
        log::log(&format!("auto card: 压着的卡片存不了：{e}"));
    }
}

/// 启动时：上次退出时还压着的卡片，等琴音空下来接着记进对话。
pub fn resume_held_cards(app: &AppHandle) {
    if desk().lock().unwrap().has_held() {
        log::log("auto card: 上次还压着卡片，空下来接着记进对话");
        schedule_flush(app);
    }
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
    // 贴出去（记进对话或压着）了才算发过：这之前出错、或者对话里已经有了，都把占下的键放掉
    let target = (|| {
        let store = store(app)?;
        let id = target_conversation(&store)?;
        let posted = store
            .load(&id)?
            .is_some_and(|c| cards::posted_in(&c.messages, plugin, &card.key));
        Ok::<_, String>((id, posted))
    })();
    let conversation_id = match target {
        Ok((_, true)) => {
            // Kotone 重启过（去重表空了），插件又发了同一局的同一张：对话里已经有了
            log::log(&format!(
                "auto card: {plugin} {} 对话里已经有了，不再发",
                card.key
            ));
            return Ok(false);
        }
        Ok((id, false)) => id,
        Err(e) => {
            desk().lock().unwrap().forget(plugin, &card.key);
            return Err(e);
        }
    };
    let at_ms = now_ms();
    let messages = vec![
        cards::card_message(plugin, plugin_name, &card, at_ms),
        cards::note_message(plugin_name, &card, at_ms),
    ];
    let shown = messages[0].clone();
    let now = main_is_quiet(app)
        && match append_to_conversation(app, &conversation_id, &messages, IfBusy::Wait) {
            Ok(Appended::Saved) => true,
            Ok(Appended::Gone) => {
                desk().lock().unwrap().forget(plugin, &card.key);
                return Err("那段对话刚被删了".into());
            }
            Ok(Appended::Busy | Appended::Steered) => false,
            Err(e) => {
                log::log(&format!("auto card: 记进对话失败，压着等下次：{e}"));
                false
            }
        };
    if !now {
        // 她在忙（或者刚才没存成）：卡片马上显示，两条等这一轮结束再记进对话
        {
            let mut desk = desk().lock().unwrap();
            desk.hold(&conversation_id, messages);
            save_held(&desk);
        }
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

/// 把压着的都记进对话；她又开始跑了就放回去下次再记（保持先后）；记进对话出错的数着次数，到了上限就放弃、记一次
/// 日志。没有压着的了返回 true。
fn flush(app: &AppHandle) -> bool {
    let held = desk().lock().unwrap().take();
    let mut back = Vec::new();
    let mut rest = held.into_iter();
    while let Some(h) = rest.next() {
        match append_to_conversation(app, &h.conversation, &h.messages, IfBusy::Wait) {
            Ok(Appended::Saved) => log::log(&format!(
                "auto card: 记进对话 {}（她这一轮说完了）",
                h.conversation
            )),
            Ok(Appended::Gone) => log::log(&format!(
                "auto card: 对话 {} 没了，压着的卡片不记了",
                h.conversation
            )),
            Ok(Appended::Busy | Appended::Steered) => {
                back.push(h);
                back.extend(rest.by_ref());
            }
            Err(e) => {
                let conversation = h.conversation.clone();
                match Desk::failed(h) {
                    Some(h) => {
                        back.push(h);
                        back.extend(rest.by_ref());
                    }
                    None => log::log(&format!(
                        "auto card: 记进对话 {conversation} 连着失败 {} 次，这张不记了：{e}",
                        cards::MAX_FAILURES
                    )),
                }
            }
        }
    }
    let mut desk = desk().lock().unwrap();
    desk.put_back(back);
    save_held(&desk);
    !desk.has_held()
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
