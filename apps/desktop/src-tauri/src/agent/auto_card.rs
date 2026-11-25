//! 自动卡片（ADR 039 D3）的桌面端：插件经 `Host::auto_card` 发来一张卡片，放进当前主会话的末尾，**不叫模型**。
//!
//! - 发到哪段对话：当前对话（是主会话的话）；没有就最近的主会话；一段都没有就开一段新的（[`target_conversation`]，
//!   和复盘那条同一个）。
//! - 什么时候记进对话：照“她正忙时插进来”的那一条规矩（`when_free.rs`，复盘、自动化的结果也用它）：她空着
//!   （[`main_is_quiet`]：她没在回答、没在说话、没有提问在等，玩家没在说话——ADR 029“做完主动说”用的同一个判断）
//!   就马上记；不空就排着，卡片照样马上显示在聊天里（界面事件 `autoCard`），等空下来再把卡片和给她的那句一起追加到
//!   对话末尾。不插进正在跑的那一轮，也不叫她开口（ADR 039 D3）。排着的存盘，Kotone 退出了下次启动接着记。
//! - 去重：贴出去（记进对话或排着）了才算发过；Kotone 重启以后看对话里有没有同插件同键的卡片。
//! - 消息提示：开着就弹（`crate::message_popup`）；点它打开聊天、定位到这张卡片（[`auto_card_open`]）。

use kotone_agent::auto_card::{self as cards, Decision};
use kotone_plugin_api::AutoCard;

use super::*;

/// 让主窗口定位到某张卡片（界面 `chatController`）。
pub const FOCUS_EVENT: &str = "kotone://auto-card-focus";

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
    // 贴出去（记进对话或排着）了才算发过：这之前出错、或者对话里已经有了，都把占下的键放掉
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
    let now = match post(app, &conversation_id, &messages, IfBusy::Wait) {
        Ok(Posted::Saved) => true,
        Ok(Posted::Held | Posted::Steered) => false,
        Ok(Posted::Gone) => {
            desk().lock().unwrap().forget(plugin, &card.key);
            return Err("那段对话刚被删了".into());
        }
        Err(e) => {
            desk().lock().unwrap().forget(plugin, &card.key);
            return Err(e);
        }
    };
    if !now {
        // 她在忙（或者刚才没存成）：卡片马上显示，两条等这一轮结束再记进对话
        emit(
            app,
            &conversation_id,
            json!({"kind": "autoCard", "message": messages[0]}),
        );
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

/// 某段对话里排着、还没记进对话的卡片（界面重新读对话时照样显示）。
#[tauri::command]
pub fn agent_held_cards(id: String) -> Vec<Message> {
    held_cards(&id)
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
