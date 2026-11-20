//! 诊断命令：ping、导出 / 清空诊断、前端错误日志、资源占用。

use super::*;

/// 冒烟测试命令：前端可 invoke("ping") 验证 IPC 通路
#[tauri::command]
pub(crate) fn ping() -> &'static str {
    "pong"
}

/// 导出不含识别文本、音频和热词的诊断 ZIP。`window`：24h / 7d / all。
#[tauri::command]
pub(crate) fn export_diagnostics(
    app: AppHandle,
    path: String,
    window: Option<String>,
) -> Result<diagnostics::DiagnosticExportResult, String> {
    let window = diagnostics::DiagnosticWindow::parse(window.as_deref().unwrap_or("all"));
    diagnostics::export(&app, std::path::Path::new(&path), window)
}

/// 清除本地流程事件与 kotone.log。`stopRecording=true` 时同时关闭后续记录。
#[tauri::command]
pub(crate) fn clear_diagnostics(app: AppHandle, stop_recording: bool) -> Result<(), String> {
    diagnostics::clear(&app, stop_recording)
}

/// 前端全局异常与更新器错误落入持久日志。消息先做主目录脱敏并限制长度。
#[tauri::command]
pub(crate) fn log_frontend_error(context: String, message: String) {
    let context: String = context
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
        .take(64)
        .collect();
    let message = diagnostics::redact_home(&message.replace(['\r', '\n'], " "));
    log::log(&format!("frontend error [{context}]: {message}"));
}

// ---------- 设置与配置（§5.3） ----------

/// sysinfo System 常驻进程内存：CPU 百分比依赖两次 refresh 的间隔采样，
/// 复用同一实例才能保证前端每次轮询拿到增量而非 0。
pub(crate) struct ResourceMonitor(pub(crate) Mutex<sysinfo::System>);

/// 当前进程资源占用（前端每 2s 轮询一次）
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceUsage {
    /// CPU 占用百分比（保留 1 位小数）。sysinfo 语义：自上次 refresh 以来
    /// 的增量采样，首次调用可能为 0；前端 2s 轮询正好提供采样间隔。
    pub cpu_percent: f32,
    /// 常驻内存（字节）
    pub memory_bytes: u64,
}

#[tauri::command]
pub(crate) fn get_resource_usage(monitor: tauri::State<ResourceMonitor>) -> ResourceUsage {
    let pid = sysinfo::Pid::from_u32(std::process::id());
    let mut sys = monitor.0.lock().unwrap();
    // sysinfo 0.30 无按 pid 过滤的 refresh（ProcessesToUpdate 是 0.32+ 的 API），
    // 全量刷新后只取本进程；2s 轮询频率下开销可接受。
    sys.refresh_processes();
    let (cpu_percent, memory_bytes) = sys
        .process(pid)
        .map(|p| ((p.cpu_usage() * 10.0).round() / 10.0, p.memory()))
        .unwrap_or((0.0, 0));
    ResourceUsage {
        cpu_percent,
        memory_bytes,
    }
}

// ---------- 会话控制 / 调试 ----------
