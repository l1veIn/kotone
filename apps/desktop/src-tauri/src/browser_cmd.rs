//! 内置浏览器的界面命令（ADR 031，聊天页右侧栏；接口约定见 ADR 031 实现情况）。

use serde_json::Value;
use tauri::AppHandle;

use crate::browser::{self, Bounds, BrowserState};

#[tauri::command]
pub fn browser_state(app: AppHandle) -> BrowserState {
    browser::state(&app)
}

/// 右侧栏里浏览器区域的位置（逻辑像素，相对窗口内容区左上角）；`visible: false` 只隐藏、不销毁。
#[tauri::command]
pub async fn browser_set_bounds(
    app: AppHandle,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    visible: bool,
) -> Result<(), String> {
    browser::set_bounds(
        &app,
        Bounds {
            x,
            y,
            width,
            height,
            visible,
        },
    )
    .await;
    Ok(())
}

/// 右侧栏上的按钮：`reload` / `back` / `close`（销毁视图） / `play` / `pause` / `next`。
#[tauri::command]
pub async fn browser_control(app: AppHandle, action: String) -> Result<Value, String> {
    browser::control(&app, &action).await
}

/// 玩家在“交给你”的横幅上点了“好了”。
#[tauri::command]
pub fn browser_handover_done(app: AppHandle) {
    browser::handover_done(&app);
}

/// 设置里“清除浏览器登录”。
#[tauri::command]
pub async fn browser_clear_data(app: AppHandle) -> Result<(), String> {
    browser::clear_data(&app).await
}

/// 只在调试版里：不经过模型，直接打开网址（同样的网址检查）、读页面，或者按编号点击 / 输入、
/// 调用 `music`，或者 `eval` 执行一段脚本、`audio` 问页面有没有在出声（真机脚本排查和对照用；发布版拒绝）。
#[tauri::command]
pub async fn browser_probe(
    app: AppHandle,
    action: String,
    arg: Option<Value>,
) -> Result<Value, String> {
    if !cfg!(debug_assertions) {
        return Err("只在调试版里可用".into());
    }
    let arg = arg.unwrap_or(Value::Null);
    match action.as_str() {
        "open" => {
            let url = kotone_agent::browser::check_url(arg.as_str().unwrap_or(""))?;
            browser::open(&app, &url).await
        }
        "snapshot" => browser::snapshot(&app, arg.as_bool().unwrap_or(false))
            .await
            .map(|s| s.value),
        "click" => browser::click(&app, arg.as_u64().unwrap_or(0)).await,
        "type" => {
            browser::type_text(
                &app,
                arg["ref"].as_u64().unwrap_or(0),
                arg["text"].as_str().unwrap_or(""),
                arg["submit"].as_bool().unwrap_or(false),
            )
            .await
        }
        "music" => browser::music(&app, arg.as_str().unwrap_or("now")).await,
        "eval" => browser::probe_eval(&app, arg.as_str().unwrap_or("null")).await,
        "audio" => browser::probe_audio(&app).await,
        other => Err(format!("不认识的操作：{other}")),
    }
}
