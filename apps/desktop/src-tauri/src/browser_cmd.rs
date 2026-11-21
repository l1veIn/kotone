//! 内置浏览器的界面命令（ADR 031，聊天页右侧栏：多标签页、地址栏、前进后退、插件；接口约定见 ADR 031 实现情况）。

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

/// 右侧栏上的按钮（对当前标签页）：`reload` / `back` / `forward` / `stop` / `close`（关掉当前标签页）；
/// `play` / `pause` / `next` 对正在放歌的标签页。
#[tauri::command]
pub async fn browser_control(app: AppHandle, action: String) -> Result<Value, String> {
    browser::control(&app, &action).await
}

/// 地址栏回车：原样交过来，后端判断是网址还是搜索词（搜索用必应）；在当前标签页打开（没有就新开一个）。
/// 玩家自己打开的网站记进站点授权。
#[tauri::command]
pub async fn browser_navigate(app: AppHandle, url: String) -> Result<Value, String> {
    browser::navigate(&app, &url).await
}

/// 新标签页（给了 `url` 就马上打开，同地址栏规则），切过去。最多 8 个。
#[tauri::command]
pub async fn browser_tab_new(app: AppHandle, url: Option<String>) -> Result<Value, String> {
    browser::tab_new(&app, url.as_deref()).await
}

// 动网页视图的命令都是 async：同步命令在主线程的 IPC 回调里跑，在那里动视图会自锁（2026-11-20 的教训）。
#[tauri::command]
pub async fn browser_tab_close(app: AppHandle, id: String) -> Result<(), String> {
    browser::tab_close(&app, &id)
}

#[tauri::command]
pub async fn browser_tab_select(app: AppHandle, id: String) -> Result<(), String> {
    browser::tab_select(&app, &id)
}

/// 浏览器插件（已解压的 Chrome / Edge 插件文件夹）。
#[tauri::command]
pub fn browser_extensions(app: AppHandle) -> Vec<browser::ExtensionInfo> {
    browser::extensions(&app)
}

#[tauri::command]
pub async fn browser_extension_add(
    app: AppHandle,
    path: String,
) -> Result<browser::ExtensionInfo, String> {
    browser::extension_add(&app, &path).await
}

#[tauri::command]
pub async fn browser_extension_remove(app: AppHandle, id: String) -> Result<(), String> {
    browser::extension_remove(&app, &id).await
}

#[tauri::command]
pub async fn browser_extension_toggle(
    app: AppHandle,
    id: String,
    enabled: bool,
) -> Result<browser::ExtensionInfo, String> {
    browser::extension_toggle(&app, &id, enabled).await
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

/// 只在调试版里：不经过模型，直接打开网址（同样的网址检查）、读页面，或者按编号点击 / 输入
/// （`mouse`：经浏览器的输入管线点，像玩家亲手点的）、
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
        "mouse" => browser::probe_mouse(&app, arg.as_u64().unwrap_or(0)).await,
        "focus" => browser::probe_focus(&app),
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
