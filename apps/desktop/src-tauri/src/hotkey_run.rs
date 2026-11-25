//! 按下自己加的热键（ADR 040 第 3 期，D5）：热键表里不绑命令、写着“要做什么”的条目（[`kotone_core::hotkey_custom`]）。
//!
//! - 跟她说一句（快捷指令）：交给她并接通通话（[`crate::companion::say`]）。要语音识别：随识别登记，识别没在跑时
//!   只看不吞、按了气泡提示（第 2 期主会话定的第 2 条），所以经 `dispatch_runtime_hotkey`（运行时的入场票）到这里。
//! - 运行自动化：跑那个已保存的自动化（目标是按下时的前台窗口，不调 AI），执行记录的发起方是 `hotkey`。不用听：
//!   随应用登记，识别没在跑也照样按（第 3 期主会话定的 P4），在 `route_named_hotkey` 里直接到这里。
//! - 调工具、打开页面：第 3 期第 2 步（这一步表里列成“找不到”、不登记）。
//!
//! 出了事照快捷指令以前的做法说一句人话（原则 21）；不重试、不补发（ADR 032）。

use tauri::{AppHandle, Manager};

use kotone_core::hotkey_custom::{find_row, Action, ROW_PREFIX};
use kotone_core::settings::Settings;

fn settings(app: &AppHandle) -> Settings {
    app.state::<crate::SharedState>()
        .settings
        .read()
        .unwrap()
        .clone()
}

/// 登记 id 是不是自己加的那一条（`custom.<id>`）。
pub(crate) fn is_custom(id: &str) -> bool {
    id.starts_with(ROW_PREFIX)
}

/// 这一条要不要语音识别（跟她说一句要；运行自动化不要）。表里找不到的当不要（按下时说清楚）。
pub(crate) fn needs_listening(app: &AppHandle, id: &str) -> bool {
    let s = settings(app);
    find_row(&s.hotkeys, id).and_then(|index| s.hotkeys.bindings[index].action().map(|a| a.kind()))
        == Some("say")
}

/// 这一条叫什么（说给玩家听的话里用）：玩家起的名字，没起就用键。
fn name_of(label: &str, key: &str) -> String {
    match (label.trim(), key.trim()) {
        ("", "") => "自己加的热键".to_string(),
        ("", key) => format!("热键 {key}"),
        (label, _) => format!("「{label}」"),
    }
}

/// 按下了自己加的 `id` 那一条。
pub(crate) fn press(app: &AppHandle, id: &str) {
    let s = settings(app);
    let Some(index) = find_row(&s.hotkeys, id) else {
        kotone_core::log::log(&format!("hotkey {id}: 表里已经没有这一条了"));
        return;
    };
    let binding = &s.hotkeys.bindings[index];
    let name = name_of(binding.label(), binding.key());
    match binding.action() {
        Some(Action::Say(text)) => crate::companion::say(app, text.to_string()),
        Some(Action::Automation {
            id: automation,
            args,
        }) => {
            let args = args.cloned().unwrap_or_default();
            let result = crate::automation::run_saved(
                app,
                "hotkey",
                automation,
                &args,
                kotone_core::automation::now_ms(),
            );
            if let Err(error) = result {
                kotone_core::log::log(&format!(
                    "hotkey {id}: automation {automation} failed: {error}"
                ));
                crate::companion::tell(
                    app,
                    format!(
                        "{name}没能运行自动化：{error}。去「设置 → 热键」看看它绑的自动化还在不在"
                    ),
                );
            }
        }
        Some(other) => kotone_core::log::log(&format!(
            "hotkey {id}: 这个版本还按不了 {}（第 3 期第 2 步）",
            other.kind()
        )),
        None => kotone_core::log::log(&format!("hotkey {id}: 读不懂这一条")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_in_what_she_says() {
        assert_eq!(name_of("连发", "F8"), "「连发」");
        assert_eq!(name_of(" ", "F8"), "热键 F8");
        assert_eq!(name_of("", ""), "自己加的热键");
        assert!(
            is_custom("custom.q1") && !is_custom("kotone.call") && !is_custom("quick-message-1")
        );
    }
}
