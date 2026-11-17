//! 琴音在游戏里“看屏幕、动鼠标”（computer use，ADR 019）的纯逻辑部分，不依赖 Windows：
//!
//! - [`actions`]：`game.act` 的动作清单与参数校验（照抄 Anthropic computer use）；
//! - [`observe`]：`game.look` 的截图缩放、编号清单与坐标换算（Anthropic 文档、UFO² / OmniParser）；
//! - [`annotate`]：在截图上画框与编号（OmniParser 的标签样式）、缩放、JPEG 编码；
//! - [`batch`]：批量执行规则（dsh-computer-use-win `batch`）。
//!
//! 截图、键鼠、窗口检查由桌面端实现 [`batch::ActHost`]（`apps/desktop/src-tauri/src/computer_use.rs`）。

pub mod actions;
pub mod annotate;
pub mod batch;
pub mod observe;

#[cfg(test)]
mod tests;

use serde_json::{json, Value};

/// 给模型的元素清单 `[{id, text, box}]`。
pub fn elements_json(elements: &[observe::Element]) -> Value {
    Value::Array(
        elements
            .iter()
            .map(|e| json!({"id": e.id, "text": e.text, "box": e.bbox}))
            .collect(),
    )
}

/// `game.act` 的结果正文（不含新截图）。
pub fn report_json(report: &batch::ActReport) -> Value {
    let results: Vec<Value> = report
        .steps
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let mut v = json!({"index": i + 1, "action": s.action, "status": s.status.as_str()});
            if let Some(message) = &s.message {
                v["message"] = json!(message);
            }
            v
        })
        .collect();
    let mut out = json!({
        "sent": report.sent(),
        "total": report.steps.len(),
        "results": results,
        "note": "sent 只表示动作已经发出；有没有生效请看新截图判断",
    });
    if let Some(stop) = report.stop {
        out["stopReason"] = json!(stop.code());
        out["hint"] = json!(stop.hint());
    }
    out
}
