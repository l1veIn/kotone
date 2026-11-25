//! 系统命令：管理员权限、热键环境与录键、开机启动、提示音试听、打开链接和文件夹。
//! 热键冲突和「设置 → 热键」那张表在 `hotkey_table_cmd.rs`（ADR 040）。

use super::*;

#[tauri::command]
pub(crate) fn get_elevation_status(
    state: tauri::State<SharedState>,
) -> compatibility::ElevationStatus {
    compatibility::elevation_status(&state)
}

/// 热键注册状态：设置页热键分区展示「注册失败，可能被其他程序/其他 Kotone 实例占用」
#[tauri::command]
pub(crate) fn get_hotkey_status(app: AppHandle) -> HotkeyStatus {
    app.state::<HotkeyManager>().status()
}

/// 主动检测低级键盘钩子与 SendInput 环境；不要求先开始录入或启动运行时。
#[tauri::command]
pub(crate) async fn check_input_environment(
    app: AppHandle,
) -> Result<InputEnvironmentCheck, String> {
    let worker_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        worker_app
            .state::<HotkeyManager>()
            .check_input_environment()
    })
    .await
    .map_err(|e| format!("输入环境自检任务失败：{e}"))
}

/// 开始热键捕获（设置页「点击录入」）：结果经 `kotone://hotkey-capture` 事件推送
#[tauri::command]
pub(crate) async fn start_hotkey_capture(app: AppHandle) -> Result<(), String> {
    let worker_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        worker_app
            .state::<HotkeyManager>()
            .start_capture(worker_app.clone())
    })
    .await
    .map_err(|e| format!("启动热键自检任务失败：{e}"))?
}

/// 取消进行中的热键捕获
#[tauri::command]
pub(crate) fn cancel_hotkey_capture(app: AppHandle) {
    app.state::<HotkeyManager>().cancel_capture();
}

/// 一键管理员重启：ShellExecuteExW "runas" 拉起新进程后退出当前进程。
/// 用户在 UAC 弹窗点「否」会返回错误，当前进程继续运行。
#[tauri::command]
pub(crate) fn restart_as_admin(app: AppHandle) -> Result<(), String> {
    elevation::restart_as_admin()?;
    app.exit(0);
    Ok(())
}

// ---------- 开机自启 ----------

/// 读取「开机自动启动」系统真实状态（HKCU Run 键 Kotone 值是否存在）。
/// 用户在任务管理器「启动」里禁用后这里返回 false，设置页据此与系统同步。
#[tauri::command]
pub(crate) fn get_autostart_enabled() -> bool {
    autostart::autostart_enabled()
}

/// 设置「开机自动启动」：写入 / 删除 HKCU Run 键（无参数启动；
/// 值名与 NSIS 卸载清理共用，卸载时自动移除）。
#[tauri::command]
pub(crate) fn set_autostart_enabled(enabled: bool) -> Result<(), String> {
    autostart::set_autostart(enabled)
}

// ---------- 音效提示 ----------

/// 试听内置提示音（设置页「音效」tab）：kind = record/send；soundId 缺失时按事件默认。
#[tauri::command]
pub(crate) fn preview_sfx(kind: String, sound_id: Option<String>) {
    let is_record = kind != "send";
    let id = sound_id
        .as_deref()
        .and_then(playback::SfxId::from_id)
        .unwrap_or_else(|| playback::SfxId::default_for(is_record));
    playback::play_sfx(id, 0.8);
}

// ---------- 模型 / 评测 ----------

/// 在系统默认浏览器中打开外部链接（关于页 GitHub 等）。
/// webview 内 target=_blank 不会调起系统浏览器；rundll32 方案零新增依赖，
/// 白名单只允许 http/https，避免被当成任意协议执行入口。
#[tauri::command]
pub(crate) fn open_external(url: String) -> Result<(), String> {
    if !url.starts_with("https://") && !url.starts_with("http://") {
        return Err("仅支持打开 http/https 链接".into());
    }
    std::process::Command::new("rundll32")
        .args(["url.dll,FileProtocolHandler", &url])
        .spawn()
        .map_err(|e| format!("打开浏览器失败：{e}"))?;
    Ok(())
}

#[cfg(windows)]
pub(crate) fn open_in_file_manager(dir: &std::path::Path) -> Result<(), String> {
    std::process::Command::new("explorer.exe")
        .arg(dir)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("打开目录失败：{e}"))
}

#[cfg(not(windows))]
fn open_in_file_manager(dir: &std::path::Path) -> Result<(), String> {
    std::process::Command::new("xdg-open")
        .arg(dir)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("打开目录失败：{e}"))
}

// ---------- 识别历史（core history 薄转发；参考 CLI log list/clear，§8.1 历史面板） ----------
