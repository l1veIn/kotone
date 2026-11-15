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
//! - 应用级工具（`app.*`、`runtime.*`、`agent.reply`、`assistant.submit`）经 [`DesktopHost`]
//!   调用桌面能力；助手入口在 `assistant.route = agent` 时经 [`submit_user_request`] 把原文
//!   转交 Agent；运行时相位变化经 [`publish`] 发布到事件总线。
//! - 配置类工具（设置、模型、连接、profile、热键、系统、历史、诊断）经 `invoke` 交给
//!   `tool_config`，与设置页调用同一批命令函数。

use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

use kotone_core::log;
use kotone_core::ocr::OcrBackend;
use kotone_platform_windows::tool_host::WindowsToolHost;
use kotone_platform_windows::window_tools::{GdiForegroundCapturer, WindowsWindowObserver};
use kotone_tools::artifacts::SystemClock;
use kotone_tools::host::{AppHost, HostFuture, HostInvokeFuture, ReplyOutput};
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
            host: Arc::new(DesktopHost {
                app: handle.clone(),
            }),
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

/// 桌面宿主：把应用级工具接到真实的运行时、退出、交付与助手入口。
struct DesktopHost {
    app: AppHandle,
}

impl AppHost for DesktopHost {
    fn app_info(&self) -> Value {
        let background = self
            .app
            .try_state::<crate::StartupOptions>()
            .map(|o| o.background)
            .unwrap_or(false);
        let route = self
            .app
            .try_state::<crate::SharedState>()
            .map(|s| s.settings.read().unwrap().assistant.route)
            .unwrap_or_default();
        json!({
            "host": "desktop",
            "launch": if background { "background" } else { "normal" },
            "assistantRoute": route,
            "kotoneHome": kotone_core::settings::kotone_dir().display().to_string(),
        })
    }

    fn runtime_status(&self) -> Value {
        let (Some(state), Some(rt)) = (
            self.app.try_state::<crate::SharedState>(),
            self.app.try_state::<RuntimeManager>(),
        ) else {
            return json!({"phase": "unknown"});
        };
        let settings = state.settings.read().unwrap();
        serde_json::to_value(rt.status(&settings, &state.engines, None)).unwrap_or(Value::Null)
    }

    fn runtime_start(&self) -> HostFuture<'_, Value> {
        Box::pin(async move {
            let status = crate::runtime::start(&self.app).await?;
            serde_json::to_value(status).map_err(|e| e.to_string())
        })
    }

    fn runtime_stop(&self) -> HostFuture<'_, Value> {
        Box::pin(async move {
            let status = crate::runtime::stop(&self.app).await?;
            serde_json::to_value(status).map_err(|e| e.to_string())
        })
    }

    fn request_quit(&self) -> Result<(), String> {
        let app = self.app.clone();
        // 先让本次响应送出，再走与托盘「退出」相同的路径（RunEvent::Exit 关闭工具服务）
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            log::log("tool service: app.quit requested");
            app.exit(0);
        });
        Ok(())
    }

    fn deliver(&self, text: String, outputs: Vec<ReplyOutput>) -> HostFuture<'_, Value> {
        Box::pin(async move {
            let kinds = outputs
                .into_iter()
                .map(|o| match o {
                    ReplyOutput::Speech => kotone_core::reminders::DeliveryKind::Voice,
                    ReplyOutput::Danmaku => kotone_core::reminders::DeliveryKind::Danmaku,
                })
                .collect();
            let delivered = crate::assistant::deliver_text(&self.app, text, kinds).await?;
            let names: Vec<&str> = delivered
                .iter()
                .map(|k| match k {
                    kotone_core::reminders::DeliveryKind::Voice => "speech",
                    kotone_core::reminders::DeliveryKind::Danmaku => "danmaku",
                })
                .collect();
            Ok(json!({"delivered": names, "errors": []}))
        })
    }

    fn assistant_submit(&self, text: String) -> HostFuture<'_, Value> {
        Box::pin(async move {
            let plan = crate::assistant::submit_text(&self.app, text, "assistant-tool").await?;
            serde_json::to_value(plan).map_err(|e| e.to_string())
        })
    }

    fn supports_invoke(&self) -> bool {
        true
    }

    fn invoke(&self, tool: &str, args: Value) -> HostInvokeFuture<'_> {
        let tool = tool.to_string();
        Box::pin(async move { crate::tool_config::invoke(&self.app, &tool, args).await })
    }
}

fn with_service<T>(
    app: &AppHandle,
    f: impl FnOnce(&Arc<kotone_tools::ToolService>) -> T,
) -> Option<T> {
    let state = app.try_state::<ToolServiceState>()?;
    let slot = state.0.lock().unwrap();
    slot.running.as_ref().map(|running| f(running.service()))
}

/// 发布事件到工具服务事件总线（服务未运行时为空操作）。
pub fn publish(app: &AppHandle, kind: &str, data: Value) {
    let _ = with_service(app, |service| service.publish_event(kind, data));
}

/// 把用户经助手入口说/写的话转交 Agent（`user.request`）。服务未运行时报错。
pub fn submit_user_request(app: &AppHandle, text: &str, source: &str) -> Result<Value, String> {
    with_service(app, |service| service.submit_user_request(text, source)).unwrap_or_else(|| {
        Err("转交 Agent 需要开启工具服务（toolService.enabled）并重启 Kotone".into())
    })
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
