//! 可选的本机 Agent 工具服务的桌面生命周期（docs/tool-service.md）。
//!
//! - 只在启动时读取 `toolService.enabled`；默认关闭，修改后需重启桌面端。
//! - 与 ASR 运行时互相独立：不借用 start/stop_runtime，运行时 Stopped 时照常服务；
//!   `system.status` 只读取当前相位用于展示。
//! - 服务从不显示、移动或聚焦窗口；CLI 直连本服务，不会再次启动 kotone.exe。
//! - `RunEvent::Exit` 时同步删除本实例的描述文件并停止服务。日志只记录端口与实例 ID，不记录凭证。

use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Manager};

use kotone_core::log;
use kotone_platform_windows::tool_host::WindowsToolHost;
use kotone_platform_windows::window_tools::{GdiForegroundCapturer, WindowsWindowObserver};
use kotone_tools::artifacts::SystemClock;
use kotone_tools::{discovery, RunningToolService, ServiceDeps, ServiceLimits};

use crate::runtime::RuntimeManager;

#[derive(Default)]
struct Slot {
    running: Option<RunningToolService>,
    /// 应用已进入退出流程：此后才完成启动的服务立即关闭，不留下描述文件。
    closed: bool,
}

#[derive(Default)]
pub struct ToolServiceState(Mutex<Slot>);

/// 在 setup 共享状态就绪后调用。未开启时只登记空状态。
pub fn start(app: &AppHandle, enabled: bool) {
    app.manage(ToolServiceState::default());
    if !enabled {
        log::log("tool service: disabled (toolService.enabled=false)");
        return;
    }
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        let phase_app = handle.clone();
        let deps = ServiceDeps {
            instance_id: kotone_tools::ids::new_id("inst"),
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            observer: Arc::new(WindowsWindowObserver),
            capturer: Arc::new(GdiForegroundCapturer),
            asr_phase: Arc::new(move || {
                phase_app
                    .try_state::<RuntimeManager>()
                    .map(|runtime| runtime.phase().as_str().to_string())
                    .unwrap_or_else(|| "unknown".into())
            }),
            limits: ServiceLimits::default(),
            clock: Arc::new(SystemClock),
        };
        let dir = discovery::default_dir();
        match RunningToolService::start(deps, Arc::new(WindowsToolHost), &dir).await {
            Ok(mut running) => {
                log::log(&format!(
                    "tool service: listening on {} instance={}",
                    running.local_addr(),
                    running.instance_id()
                ));
                let Some(state) = handle.try_state::<ToolServiceState>() else {
                    running.shutdown_now();
                    return;
                };
                let mut slot = state.0.lock().unwrap();
                if slot.closed {
                    running.shutdown_now();
                } else {
                    slot.running = Some(running);
                }
            }
            Err(error) => log::log(&format!("tool service: failed to start: {error}")),
        }
    });
}

/// `RunEvent::Exit`：删除本实例描述文件、停止接受连接、丢弃内存产物。
pub fn shutdown(app: &AppHandle) {
    let Some(state) = app.try_state::<ToolServiceState>() else {
        return;
    };
    let running = {
        let mut slot = state.0.lock().unwrap();
        slot.closed = true;
        slot.running.take()
    };
    if let Some(mut running) = running {
        running.shutdown_now();
        log::log("tool service: stopped");
    }
}
