//! 语音输入运行时与会话命令：状态、启动 / 停止、确认发送、取消、模拟发送。

use super::*;

/// 全量运行时状态（相位 + restartNeeded + 当前引擎/模型/模式）
#[tauri::command]
pub(crate) fn get_runtime_status(
    app: AppHandle,
    state: tauri::State<SharedState>,
) -> RuntimeStatus {
    let rt = app.state::<RuntimeManager>();
    let settings = state.settings.read().unwrap();
    rt.status(&settings, &state.engines, None)
}

/// 启动：warmup 引擎 → 注册热键 → 显示悬浮窗（进度经 kotone://runtime 推送）。
/// Running + restartNeeded 时等价于 stop + start（重启语义）。
#[tauri::command]
pub(crate) async fn start_runtime(app: AppHandle) -> Result<RuntimeStatus, String> {
    runtime::start(&app).await
}

/// 停止：取消会话 → 注销热键 → 隐藏悬浮窗 → 卸载引擎。Stopped 时幂等。
#[tauri::command]
pub(crate) async fn stop_runtime(app: AppHandle) -> Result<RuntimeStatus, String> {
    runtime::stop(&app).await
}

// ---------- 模型目录管理 ----------

/// Preview 状态下确认发送（ADR-006：预览只读，始终发送预览文本）
#[tauri::command]
pub(crate) async fn confirm_send(state: tauri::State<'_, SharedState>) -> Result<(), String> {
    state.orchestrator.confirm_send().await
}

/// 取消当前会话（悬浮窗取消按钮 / 调试）
#[tauri::command]
pub(crate) async fn cancel_session(state: tauri::State<'_, SharedState>) -> Result<(), String> {
    state.orchestrator.cancel().await;
    Ok(())
}

/// 手动触发发送（调试/记事本测试用）：走真实 WindowsInjector 时序
/// （openChatKey → Unicode/剪贴板 → sendKey，直发当前前台窗口）。
///
/// P0（用户反馈「SendInput 被 360 拦截后卡死」）：此前是同步命令，在主线程
/// 直接调 SendInput——被安全软件钩住挂起时整个主进程（UI/托盘/IPC）冻结。
/// 改为 async + spawn_blocking + 10s 超时：挂起只阻塞工作线程，超时后返回
/// 清晰错误，UI 保持响应。
#[tauri::command]
pub(crate) async fn simulate_send(
    state: tauri::State<'_, SharedState>,
    text: String,
    profile_id: Option<String>,
) -> Result<(), InjectError> {
    let profile = profile_id
        .as_deref()
        .and_then(profile::get)
        .unwrap_or_else(GameProfile::builtin_generic);
    let injector = state.injector.clone();
    let task = tauri::async_runtime::spawn_blocking(move || {
        injector.send(&text, &profile, CancelToken::default())
    });
    const SEND_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
    match tokio::time::timeout(SEND_TIMEOUT, task).await {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => Err(InjectError::new("发送线程异常".to_string())),
        Err(_) => Err(InjectError::new(
            "发送超时（10s）：模拟输入未在限时内完成，通常是被安全软件或游戏反作弊拦截".to_string(),
        )),
    }
}
