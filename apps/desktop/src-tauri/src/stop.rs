//! 「停下」（ADR 028 的核心概念之一）：停止琴音正在做的事和所有自动化。
//!
//! 四个入口都走这里：停止热键（`automationStopHotkey`）、托盘「停下」、说“停下”、
//! 琴音操作游戏时按 Esc（Esc 由 computer_use 自己拦截，结束后同样停下这一轮）。
//! 语音输入（悬浮窗那条链路）不受影响：它没有“正在做的事”，按 Esc 取消的是录音本身。

use tauri::AppHandle;

/// 停下一切：全部自动化、琴音这一轮（含正在操作游戏）、交给后台的事和它们的监听、正在念的话。
/// 返回是否真的停下了什么（都没在跑时为 false）。
pub fn stop_everything(app: &AppHandle, reason: &str) -> bool {
    let automations = crate::automation::stop(app, None, reason);
    let acting = crate::computer_use::stop_all(app, reason);
    // 交给后台的事（同传等）和它们名下的监听一起停（ADR 029 第二期）。
    let children = crate::agent::stop_all_children(app, reason);
    // 结束这一轮（不再自动重试，ADR 019）；0.1.x 用户还没启用琴音时没有这一轮。
    let turn = crate::agent::stop(app).unwrap_or(false);
    // 正在念的回答、弹幕一起收掉。
    crate::assistant::stop_speech(app.clone());
    kotone_core::log::log(&format!(
        "停下（{reason}）：自动化 {automations} 个，操作 {acting}，后台 {children} 件，这一轮 {turn}"
    ));
    automations > 0 || acting || children > 0 || turn
}
