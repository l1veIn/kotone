// 业务模块空壳 —— 只定义类型与最小占位，保证 cargo check 通过。
// 职责划分见 docs/development.md §5.1，业务逻辑由后续开发填充。
mod audio;
mod eval;
mod hotkey;
mod inject;
mod model;
mod orchestrator;
mod profile;
mod settings;
mod stt;
mod tray;

/// 冒烟测试命令：前端可 invoke("ping") 验证 IPC 通路
#[tauri::command]
fn ping() -> &'static str {
    "pong"
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .setup(|app| {
            tray::setup_tray(app.handle())?;
            // TODO: hotkey / orchestrator / settings 初始化（Phase 1）
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![ping])
        .run(tauri::generate_context!())
        .expect("error while running Kotone application");
}
