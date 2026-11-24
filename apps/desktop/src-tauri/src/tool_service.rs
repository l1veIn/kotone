//! 可选的本机 Agent 工具服务的桌面生命周期（docs/tool-service.md）。
//!
//! - 服务对象**常驻**（内置 Agent 在进程内调用同一份工具实现，ADR 014 §3）；
//!   `toolService.enabled` 只决定是否对外监听 HTTP 并发布描述文件，只在启动时读取，
//!   默认关闭，修改后需重启桌面端。
//! - 与 ASR 运行时互相独立：不借用 start/stop_runtime，运行时 Stopped 时照常服务；
//!   `system.status` 只读取当前相位用于展示。
//! - 服务从不显示、移动或聚焦窗口；CLI 直连本服务，不会再次启动 kotone.exe。
//! - `RunEvent::Exit` 时同步删除本实例的描述文件并停止服务。日志只记录端口与实例 ID，不记录凭证。
//! - OCR：统一模型清单里的每个本地 PP-OCRv5 包各是一个后端，读取 `<模型根>/ocr/<模型 ID>`
//!   （设置页或 `kotone-cli ocr install --model <id>` 安装，每次调用重新校验，安装后无需重启）；
//!   DeepSeek 在线后端在 OCR 清单里绑定连接（旧的 `toolService.ocr.deepseekConnectionId`
//!   仍然有效）后才就绪，API key 来自系统凭据库。本地与在线互不降级。
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
use kotone_tools::{discovery, RunningToolService, ServiceDeps, ServiceLimits, ToolService};

use crate::runtime::RuntimeManager;

#[derive(Default)]
struct Slot {
    /// 常驻的服务（内置 Agent 与对外监听共用）。
    service: Option<Arc<ToolService>>,
    maintenance: Option<tokio::task::JoinHandle<()>>,
    /// 对外 HTTP 监听（`toolService.enabled`）。
    running: Option<RunningToolService>,
    /// 应用已进入退出流程：此后才完成启动的服务立即关闭，不留下描述文件。
    closed: bool,
}

#[derive(Default)]
pub struct ToolServiceState(Mutex<Slot>);

/// 在 setup 共享状态就绪后调用：创建常驻服务；`enabled` 时再对外监听。
pub fn start(app: &AppHandle, enabled: bool) {
    app.manage(ToolServiceState::default());
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
        let service = ToolService::new(deps);
        let maintenance = service.spawn_maintenance();
        service.publish_event(
            "app.started",
            json!({"instanceId": service.instance_id(), "pid": std::process::id()}),
        );
        // 后台做一次 OCR 后端就绪检查（模型哈希、凭据读取），不阻塞启动与异步线程
        let refresh = service.clone();
        tauri::async_runtime::spawn_blocking(move || {
            refresh.refresh_ocr_readiness();
        });
        let Some(state) = handle.try_state::<ToolServiceState>() else {
            service.close();
            maintenance.abort();
            return;
        };
        {
            let mut slot = state.0.lock().unwrap();
            if slot.closed {
                service.close();
                maintenance.abort();
                return;
            }
            slot.service = Some(service.clone());
            slot.maintenance = Some(maintenance);
        }
        log::log(&format!(
            "tool service: resident instance={}",
            service.instance_id()
        ));
        if !enabled {
            log::log("tool service: not listening (toolService.enabled=false)");
            return;
        }
        let dir = discovery::default_dir();
        match RunningToolService::serve(service, Arc::new(WindowsToolHost), &dir).await {
            Ok(mut running) => {
                log::log(&format!(
                    "tool service: listening on {} instance={}",
                    running.local_addr(),
                    running.instance_id()
                ));
                let mut slot = state.0.lock().unwrap();
                if slot.closed {
                    running.shutdown_now();
                } else {
                    slot.running = Some(running);
                }
            }
            Err(error) => log::log(&format!("tool service: failed to listen: {error}")),
        }
    });
}

/// 组装 OCR 后端：统一模型清单里的全部本地包（目录随启动时的 settings.models.dir）与
/// DeepSeek 在线后端（每次识别时按当前设置解析连接）。与陪玩流程共用同一批实例。
fn ocr_backends(app: &AppHandle) -> Vec<Arc<dyn OcrBackend>> {
    let (Some(engines), Some(shared)) = (
        app.try_state::<crate::ocr_engines::OcrEngines>(),
        app.try_state::<crate::SharedState>(),
    ) else {
        return Vec::new();
    };
    let settings = shared.settings.read().unwrap().clone();
    engines.all(&settings)
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
            // ADR 028：在线识别没有“启动 / 停止”，停了玩家在界面上也找不到地方再打开。
            let remote = self
                .app
                .try_state::<crate::SharedState>()
                .is_some_and(|state| {
                    crate::runtime::asr_is_remote(&state.settings.read().unwrap())
                });
            if remote {
                return Err("现在用的是在线语音识别，打开就能用，不需要停止".into());
            }
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

    /// 名字叫插件的工具，本体用 ToolSpec 登记的（`game_chat.send`，ADR 039 D2）也从这里列：
    /// 都是宿主执行、经 `invoke` 调。
    fn plugin_tools(&self) -> Vec<Value> {
        let Some(state) = self.app.try_state::<crate::SharedState>() else {
            return kotone_agent::body::service_specs();
        };
        let settings = state.settings.read().unwrap().clone();
        hosted_tool_specs(&settings)
    }

    fn invoke(&self, tool: &str, args: Value) -> HostInvokeFuture<'_> {
        let tool = tool.to_string();
        Box::pin(async move { crate::tool_config::invoke(&self.app, &tool, args).await })
    }
}

/// 工具服务里由宿主执行、用 ToolSpec 登记的工具（调用经 `invoke` 到 `tool_config`）：本体的
/// （ADR 039 D2 的 `game_chat.send`，不看插件开关）在前，再加开着的插件的（ADR 038）。
pub(crate) fn hosted_tool_specs(settings: &kotone_core::settings::Settings) -> Vec<Value> {
    let mut specs = kotone_agent::body::service_specs();
    specs.extend(crate::plugins::service_specs(settings));
    specs
}

/// 常驻的服务（启动早期尚未就绪、或应用正在退出时为 None）。
pub fn service(app: &AppHandle) -> Option<Arc<ToolService>> {
    let state = app.try_state::<ToolServiceState>()?;
    let slot = state.0.lock().unwrap();
    slot.service.clone()
}

/// 发布事件到工具服务事件总线（服务未运行时为空操作）。
pub fn publish(app: &AppHandle, kind: &str, data: Value) {
    if let Some(service) = service(app) {
        service.publish_event(kind, data);
    }
}

/// 把用户经助手入口说/写的话转交外部 Agent（`user.request`）。没有对外监听时报错
/// （外部 Agent 连不上，请求只会过期）。
pub fn submit_user_request(app: &AppHandle, text: &str, source: &str) -> Result<Value, String> {
    let listening = {
        let Some(state) = app.try_state::<ToolServiceState>() else {
            return Err(not_listening());
        };
        let slot = state.0.lock().unwrap();
        slot.running
            .as_ref()
            .map(|running| running.service().clone())
    };
    match listening {
        Some(service) => service.submit_user_request(text, source),
        None => Err(not_listening()),
    }
}

fn not_listening() -> String {
    "转交 Agent 需要开启工具服务（toolService.enabled）并重启 Kotone".into()
}

/// `RunEvent::Exit`：删除本实例描述文件、停止接受连接、丢弃内存产物。
pub fn shutdown(app: &AppHandle) {
    let Some(state) = app.try_state::<ToolServiceState>() else {
        return;
    };
    let (running, service, maintenance) = {
        let mut slot = state.0.lock().unwrap();
        slot.closed = true;
        (
            slot.running.take(),
            slot.service.take(),
            slot.maintenance.take(),
        )
    };
    // 先关监听（同时关闭服务、删除描述文件），再兜底关闭常驻服务与维护任务。
    if let Some(mut running) = running {
        running.shutdown_now();
    }
    if let Some(service) = service {
        service.close();
    }
    if let Some(maintenance) = maintenance {
        maintenance.abort();
    }
    log::log("tool service: stopped");
}
