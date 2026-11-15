//! 可选的本机 Agent 工具服务的桌面生命周期（docs/tool-service.md）。
//!
//! - 只在启动时读取 `toolService.enabled`；默认关闭，修改后需重启桌面端。
//! - 与 ASR 运行时互相独立：不借用 start/stop_runtime，运行时 Stopped 时照常服务；
//!   `system.status` 只读取当前相位用于展示。
//! - 服务从不显示、移动或聚焦窗口；CLI 直连本服务，不会再次启动 kotone.exe。
//! - `RunEvent::Exit` 时同步删除本实例的描述文件并停止服务。日志只记录端口与实例 ID，不记录凭证。
//! - OCR（M2）：本地韩文 PP-OCRv5 后端读取 `<模型根>/ocr/ppocrv5-korean-mobile`（由
//!   `kotone-cli ocr install` 安装，每次调用重新校验，安装后无需重启）；DeepSeek 在线后端
//!   只有在 `toolService.ocr.deepseekConnectionId` 指定连接时才就绪，连接与 API key 复用
//!   连接目录与系统凭据库（桌面内存中的设置，修改后需重启桌面端）。两者互不降级。

use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Manager};

use kotone_core::log;
use kotone_core::ocr::OcrBackend;
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
            ocr_backends: ocr_backends(&handle),
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

/// 组装 OCR 后端：本地（模型目录随 settings.models.dir）与在线（按设置选择连接）。
fn ocr_backends(app: &AppHandle) -> Vec<Arc<dyn OcrBackend>> {
    let models_root = app
        .try_state::<crate::SharedState>()
        .map(|shared| kotone_stt::model::models_dir_from(&shared.settings.read().unwrap()))
        .unwrap_or_else(kotone_stt::model::models_dir);
    let local = kotone_ocr::LocalKoreanOcr::new(kotone_ocr::local_model_dir(&models_root));
    let source_app = app.clone();
    let source: kotone_ocr::deepseek::ConnectionSource = Arc::new(move || {
        let Some(shared) = source_app.try_state::<crate::SharedState>() else {
            return Err("应用状态尚未就绪".into());
        };
        let id = shared
            .settings
            .read()
            .unwrap()
            .tool_service
            .ocr
            .deepseek_connection_id
            .clone();
        match id {
            None => Ok(None),
            Some(id) => shared.connections.resolve(&id).map(Some),
        }
    });
    vec![
        Arc::new(local),
        Arc::new(kotone_ocr::DeepSeekOcr::new(source)),
    ]
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
