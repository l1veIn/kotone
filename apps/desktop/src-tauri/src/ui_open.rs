//! `ui.open` 不打断玩家（共享资源评估第二节第 6 条，2026-11-25；ADR 039 实施记录“ui.open 不打断玩家”）。
//!
//! 页面地址的检查和发事件照旧是 [`crate::plugins::open_page`]；打开在哪、要不要等玩家点由界面定
//! （`apps/desktop/src/lib/stores/uiOpen.ts`：玩家在聊天页、右侧栏空着或显示的就是这个插件时直接显示，
//! 别的时候放进右侧栏“卡片”等玩家点；同一个页面反复打开只算一次）。界面处理完调 [`ui_open_reply`] 交回结果，
//! 这里等它（最多 [`WAIT`]），把“已经显示 / 放在右侧栏等玩家点”写进工具结果，调用方（她、外部 Agent）照着说。
//!
//! 谁调的不分：她和外部 Agent 走同一条路、同一条规矩。外部 Agent 调工具 Kotone 本来就不问（ADR 014 第 5 节：
//! 权限由它们自己的运行框架管，所有工具都这样），不打断玩家以后它最多在右侧栏放一张卡片。

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use serde_json::Value;
use tauri::{AppHandle, Manager};
use tokio::sync::oneshot;

/// 等界面回话最多这么久（界面收到事件马上就回；窗口还没加载完时不回）。
const WAIT: Duration = Duration::from_secs(2);

/// 等界面回话的调用：页面地址 → 等着的（同一个地址同时开几次，都拿同一个结果）。
#[derive(Default)]
pub struct UiOpenReplies {
    pending: Mutex<HashMap<String, Vec<oneshot::Sender<String>>>>,
}

impl UiOpenReplies {
    fn wait(&self, uri: &str) -> oneshot::Receiver<String> {
        let (tx, rx) = oneshot::channel();
        self.pending
            .lock()
            .unwrap()
            .entry(uri.to_string())
            .or_default()
            .push(tx);
        rx
    }

    /// 界面处理完一个地址：交给所有在等它的。没人等（例如超时以后才回）就算了。
    fn answer(&self, uri: &str, outcome: &str) {
        let waiting = self.pending.lock().unwrap().remove(uri).unwrap_or_default();
        for tx in waiting {
            let _ = tx.send(outcome.to_string());
        }
    }

    /// 不等了（检查没过、超时）：把已经关掉的拿掉，别的照旧等。
    fn forget(&self, uri: &str) {
        let mut pending = self.pending.lock().unwrap();
        if let Some(list) = pending.get_mut(uri) {
            list.retain(|tx| !tx.is_closed());
            if list.is_empty() {
                pending.remove(uri);
            }
        }
    }
}

/// `ui.open`：检查地址、让界面打开（[`crate::plugins::open_page`]），等界面说它怎么处理的。
pub async fn open(app: &AppHandle, args: &Value) -> Result<Value, String> {
    let uri = args
        .get("page")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("")
        .to_string();
    let replies = app.try_state::<UiOpenReplies>();
    // 先登记再发事件：界面回得比这边快也接得住
    let mut rx = replies.as_ref().map(|r| r.wait(&uri));
    let result = crate::plugins::open_page(app, args);
    let outcome = match (&result, rx.as_mut()) {
        (Ok(_), Some(rx)) => tokio::time::timeout(WAIT, rx)
            .await
            .ok()
            .and_then(Result::ok),
        _ => None,
    };
    drop(rx);
    if let Some(r) = &replies {
        r.forget(&uri);
    }
    result.map(|value| finish(value, outcome.as_deref()))
}

/// 把界面的结果写进工具结果：`opened` 是“已经显示了”，`status` 是 `shown` / `waiting` / `sent`（界面没回话），
/// `note` 是给调用方照着说的话。
fn finish(mut value: Value, outcome: Option<&str>) -> Value {
    let (opened, status, note) = match outcome {
        Some("shown") => (
            true,
            "shown",
            "已经在 Kotone 窗口里显示了（插件的页面在聊天页的右侧栏）；窗口不会自己跳到前面，玩家在游戏里时切回 Kotone 才看得到",
        ),
        Some("waiting") => (
            false,
            "waiting",
            "玩家正在看别的，没有切过去：放在聊天页右侧栏的“卡片”里，窗口里也提示了，玩家点“打开”才打开。可以告诉玩家去那里看；同一个页面不用再开",
        ),
        _ => (
            false,
            "sent",
            "交给 Kotone 窗口了，但窗口没回话（可能还没加载完），不知道玩家看没看到",
        ),
    };
    if let Some(map) = value.as_object_mut() {
        map.insert("opened".into(), Value::Bool(opened));
        map.insert("status".into(), Value::String(status.into()));
        map.insert("note".into(), Value::String(note.into()));
    }
    value
}

/// 界面处理完 `kotone://ui-open`：`outcome` 是 `shown` / `waiting`，认不出的地址是 null。
#[tauri::command]
pub fn ui_open_reply(app: AppHandle, uri: String, outcome: Option<String>) {
    if let Some(replies) = app.try_state::<UiOpenReplies>() {
        replies.answer(uri.trim(), outcome.as_deref().unwrap_or(""));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn opened() -> Value {
        json!({"opened": true, "uri": "ui://kotone/lol/match", "title": "这局", "note": "旧的说法"})
    }

    /// 工具结果让调用方知道“已经显示”还是“放在右侧栏等玩家点”（共享资源评估第二节第 6 条）；
    /// 界面没回话不说“打开了”。字段名不改（单向门）：`opened`、`uri`、`title` 照旧，多一个 `status`。
    #[test]
    fn the_result_says_shown_or_waiting() {
        let shown = finish(opened(), Some("shown"));
        assert_eq!(shown["opened"], true);
        assert_eq!(shown["status"], "shown");
        assert_eq!(shown["uri"], "ui://kotone/lol/match");
        assert_eq!(shown["title"], "这局");
        let waiting = finish(opened(), Some("waiting"));
        assert_eq!(waiting["opened"], false);
        assert_eq!(waiting["status"], "waiting");
        let note = waiting["note"].as_str().unwrap();
        assert!(
            note.contains("右侧栏") && note.contains("点") && note.contains("不用再开"),
            "{note}"
        );
        for silent in [None, Some(""), Some("nope")] {
            let sent = finish(opened(), silent);
            assert_eq!(sent["opened"], false, "{silent:?}");
            assert_eq!(sent["status"], "sent", "{silent:?}");
        }
    }

    /// 界面的回话交给所有在等这个地址的；别的地址不受影响；没人等的回话丢掉；不等了的拿掉。
    #[tokio::test]
    async fn replies_go_to_whoever_waits_for_that_page() {
        let replies = UiOpenReplies::default();
        let a1 = replies.wait("ui://kotone/lol/match");
        let a2 = replies.wait("ui://kotone/lol/match");
        let mut b = replies.wait("ui://kotone/lol/tier");
        replies.answer("ui://kotone/lol/match", "waiting");
        assert_eq!(a1.await.unwrap(), "waiting");
        assert_eq!(a2.await.unwrap(), "waiting");
        assert!(b.try_recv().is_err(), "别的地址还在等");
        replies.answer("ui://kotone/settings/sound", "shown");
        drop(b);
        replies.forget("ui://kotone/lol/tier");
        assert!(replies.pending.lock().unwrap().is_empty());
    }
}
