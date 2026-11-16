//! 自动化的桌面执行器（docs/adr/013-automation-domain-model.md、演进计划 §12）。
//!
//! - 执行记录由 core 的 [`AutomationScheduler`] 管（存盘、延迟、取消、错过不补、名额）；本模块是
//!   它的 [`ProgramLauncher`]：到点（或立即）启动一段 Luau 程序，结束时回报 `finish_program`。
//! - 每个程序在自己的线程里由 `kotone_script::run` 执行；宿主 [`DesktopHost`] 把白名单函数接到
//!   真实能力：按键 / 打字走 `SendInput`（与“帮忙打字”同一条输入通道），看屏幕用 GDI 截游戏
//!   客户区，`ocr()` 用陪玩选定的 OCR 模型，`say` / `toast` 复用提醒的交付路径。
//! - **只对下指令时的游戏窗口生效**：安排执行时记下前台窗口（[`RunTarget`]）；到点时窗口已关闭
//!   就不启动；运行中按键、截图前先确认它在前台，不在就暂停等待；窗口关闭则结束。
//! - **按住的键一定会松开**：`key_down` 记账，程序结束（正常、出错、取消、超限）时全部 `key_up`。
//! - 停止自动化热键、托盘菜单与不带 id 的 `automation.stop` 停止全部程序（含等待中的程序，
//!   不含提醒）；应用退出只停止运行中的程序，等待中的留到下次启动（过期则记为错过）。

use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use kotone_core::automation::{
    AutomationScheduler, ProgramAction, ProgramLauncher, ProgramReport, Run, RunAction, RunStatus,
    RunTarget, MAX_RUNNING_PROGRAMS, MAX_SOURCE_CHARS,
};
use kotone_core::ocr::{OcrCancel, OcrInput, OcrParams};
use kotone_core::reminders::DeliveryKind;
use kotone_core::tools::{WindowCapturer, WindowIdentity, WindowInfo, WindowObserver};
use kotone_platform_windows::inject;
use kotone_platform_windows::window_tools::{GdiForegroundCapturer, WindowsWindowObserver};
use kotone_script::{Cancel, KeyAction, Limits, Outcome, Region, ScriptHost};
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

const MAX_CAPTURE_PIXELS: u64 = 3840 * 2160;
/// 工具 `automation.runs` 最多返回的条数。
const MAX_LISTED_RUNS: usize = 50;

/// 运行中的程序（只在内存里；执行记录在调度器）。
struct Live {
    cancel: Cancel,
    end_reason: Option<String>,
}

#[derive(Default)]
pub struct AutomationManager {
    live: Mutex<HashMap<String, Live>>,
}

/// 调度器的程序执行端口。
pub struct Launcher {
    pub app: AppHandle,
}

impl ProgramLauncher for Launcher {
    fn launch(&self, run: &Run) -> Result<(), String> {
        launch(&self.app, run)
    }

    fn stop(&self, run_id: &str, reason: &str) {
        let manager = self.app.state::<AutomationManager>();
        let mut live = manager.live.lock().unwrap();
        if let Some(live) = live.get_mut(run_id) {
            live.cancel.cancel();
            live.end_reason.get_or_insert_with(|| reason.to_string());
            kotone_core::log::log(&format!("automation {run_id}: stop ({reason})"));
        }
    }
}

fn scheduler(app: &AppHandle) -> Result<Arc<AutomationScheduler>, String> {
    app.try_state::<crate::assistant::AssistantRuntime>()
        .ok_or("自动化调度器尚未就绪")?
        .scheduler
        .clone()
}

fn target_label(window: &WindowInfo) -> String {
    match &window.process_name {
        Some(process) => format!("{} ({process})", window.title),
        None => window.title.clone(),
    }
}

/// 真实宿主。
struct DesktopHost {
    app: AppHandle,
    target: WindowIdentity,
    started: Instant,
    held: Mutex<BTreeSet<String>>,
    /// 游戏窗口已关闭或身份变化：脚本按取消结束。
    gone: AtomicBool,
    last_inspect: AtomicU64,
    cancel: Cancel,
}

impl DesktopHost {
    fn focused_now(&self) -> bool {
        matches!(
            WindowsWindowObserver.foreground_window(),
            Ok(Some(w)) if w.identity.handle == self.target.handle && w.identity.pid == self.target.pid
        )
    }

    /// 每 500 ms 检查一次游戏窗口是否还在；不在就取消脚本。
    fn check_alive(&self) {
        let now = self.now_ms();
        if now.saturating_sub(self.last_inspect.load(Ordering::Relaxed)) < 500 {
            return;
        }
        self.last_inspect.store(now, Ordering::Relaxed);
        if WindowsWindowObserver.inspect(&self.target).is_err() {
            self.gone.store(true, Ordering::SeqCst);
            self.cancel.cancel();
        }
    }

    /// 截游戏客户区；游戏不在前台时先等它回来（与按键一样暂停）。
    fn capture(&self) -> Result<kotone_tools::image::RgbaImage, String> {
        while !self.focused_now() {
            if !self.sleep(200, &self.cancel.clone()) {
                return Err("自动化已停止".into());
            }
        }
        let image = GdiForegroundCapturer
            .capture_foreground_client(&self.target, MAX_CAPTURE_PIXELS)
            .map_err(|e| format!("截图失败：{e}"))?;
        if image
            .warnings
            .iter()
            .any(|w| w.code == kotone_platform_windows::window_tools::WARNING_UNIFORM_PIXELS)
        {
            return Err("截到的是纯色画面，游戏可能是独占全屏：请改成无边框窗口".into());
        }
        kotone_tools::image::RgbaImage::new(image.width, image.height, image.rgba)
            .map_err(|e| e.to_string())
    }

    fn crop(image: &kotone_tools::image::RgbaImage, r: Region) -> kotone_tools::image::RgbaImage {
        let (w, h) = (image.width(), image.height());
        let x = ((r.x * f64::from(w)).floor() as u32).min(w - 1);
        let y = ((r.y * f64::from(h)).floor() as u32).min(h - 1);
        let cw = ((r.width * f64::from(w)).ceil() as u32).clamp(1, w - x);
        let ch = ((r.height * f64::from(h)).ceil() as u32).clamp(1, h - y);
        image.crop(kotone_tools::image::CropRect {
            x,
            y,
            width: cw,
            height: ch,
        })
    }

    fn release_all(&self) {
        let held: Vec<String> = std::mem::take(&mut *self.held.lock().unwrap())
            .into_iter()
            .collect();
        for key in held.iter().rev() {
            let _ = inject::key_up(key);
        }
    }

    fn deliver(&self, text: &str, kind: DeliveryKind, wait: bool) -> Result<(), String> {
        let app = self.app.clone();
        let text = text.to_string();
        let task = async move {
            crate::assistant::deliver_text(&app, text, vec![kind])
                .await
                .map(|_| ())
        };
        if wait {
            tauri::async_runtime::block_on(task)
        } else {
            tauri::async_runtime::spawn(task);
            Ok(())
        }
    }
}

/// 平均亮度（Rec.601 luma）。
pub fn mean_luma(pixels: &[u8]) -> f64 {
    let n = pixels.len() / 4;
    if n == 0 {
        return 0.0;
    }
    let sum: f64 = pixels
        .chunks_exact(4)
        .map(|p| 0.299 * f64::from(p[0]) + 0.587 * f64::from(p[1]) + 0.114 * f64::from(p[2]))
        .sum();
    sum / n as f64
}

impl ScriptHost for DesktopHost {
    fn now_ms(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }

    fn sleep(&self, ms: u64, cancel: &Cancel) -> bool {
        let deadline = Instant::now() + Duration::from_millis(ms);
        loop {
            if cancel.is_cancelled() {
                return false;
            }
            self.check_alive();
            if cancel.is_cancelled() {
                return false;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return true;
            }
            std::thread::sleep(left.min(Duration::from_millis(25)));
        }
    }

    fn game_focused(&self) -> bool {
        self.focused_now()
    }

    fn key(&self, action: KeyAction, key: &str) -> Result<(), String> {
        match action {
            KeyAction::Press => inject::key_down_up(key).map_err(|e| e.message),
            KeyAction::Down => {
                inject::key_down(key).map_err(|e| e.message)?;
                self.held.lock().unwrap().insert(key.to_string());
                Ok(())
            }
            KeyAction::Up => {
                self.held.lock().unwrap().remove(key);
                inject::key_up(key).map_err(|e| e.message)
            }
        }
    }

    fn type_text(&self, text: &str) -> Result<(), String> {
        inject::send_unicode(text).map_err(|e| e.message)
    }

    fn brightness(&self, region: Region) -> Result<f64, String> {
        let image = self.capture()?;
        Ok(mean_luma(Self::crop(&image, region).pixels()))
    }

    fn color_at(&self, x: f64, y: f64) -> Result<[u8; 3], String> {
        let image = self.capture()?;
        let px = ((x * f64::from(image.width())) as u32).min(image.width() - 1);
        let py = ((y * f64::from(image.height())) as u32).min(image.height() - 1);
        let [r, g, b, _] = image.pixel(px, py).ok_or("坐标超出画面")?;
        Ok([r, g, b])
    }

    fn ocr(&self, region: Region) -> Result<Vec<String>, String> {
        let settings = self
            .app
            .state::<crate::SharedState>()
            .settings
            .read()
            .unwrap()
            .clone();
        let (_, backend) = crate::companion::ocr_backend(&self.app, &settings)?;
        let info = backend.info();
        if !info.ready {
            return Err(format!(
                "文字识别模型还不能用：{}",
                info.reason.unwrap_or(info.state)
            ));
        }
        let crop = Self::crop(&self.capture()?, region);
        let png = if backend.kind() == kotone_core::ocr::OcrBackendKind::Online {
            kotone_tools::image::encode_png(&crop).map_err(|e| e.to_string())?
        } else {
            Vec::new()
        };
        let output = backend
            .recognize(
                &OcrInput {
                    width: crop.width(),
                    height: crop.height(),
                    rgba: crop.pixels(),
                    png: &png,
                },
                &OcrParams {
                    language: info.default_language,
                    options: Value::Null,
                },
                &OcrCancel::new(),
                Instant::now() + Duration::from_secs(20),
            )
            .map_err(|e| format!("文字识别失败：{e}"))?;
        Ok(output.observations.into_iter().map(|o| o.text).collect())
    }

    fn say(&self, text: &str) -> Result<(), String> {
        self.deliver(text, DeliveryKind::Voice, true)
    }

    fn toast(&self, text: &str) -> Result<(), String> {
        self.deliver(text, DeliveryKind::Danmaku, false)
    }
}

/// 执行记录的对外形状（工具与事件）。`with_source = false` 时省略程序源码。
pub fn run_json(run: &Run, with_source: bool) -> Value {
    let mut value = serde_json::to_value(run).unwrap_or(Value::Null);
    if !with_source {
        if let Some(action) = value.get_mut("action").and_then(Value::as_object_mut) {
            action.remove("source");
        }
    }
    value
}

/// 等待中、运行中与最近的执行记录（含提醒）。
pub fn list(app: &AppHandle) -> Result<Value, String> {
    let runs = scheduler(app)?.runs();
    let running = runs
        .iter()
        .filter(|r| r.is_program() && r.status == RunStatus::Running)
        .count();
    let pending = runs
        .iter()
        .filter(|r| r.status == RunStatus::Pending)
        .count();
    Ok(json!({
        "running": running,
        "pending": pending,
        "maxRunning": MAX_RUNNING_PROGRAMS,
        "runs": runs.iter().take(MAX_LISTED_RUNS).map(|r| run_json(r, true)).collect::<Vec<_>>(),
    }))
}

/// 当前前台窗口作为目标（不能是 Kotone 自己）。
fn foreground_target() -> Result<RunTarget, String> {
    let window = WindowsWindowObserver
        .foreground_window()
        .map_err(|e| format!("找不到前台窗口：{e}"))?
        .ok_or("当前没有前台窗口：请先切到游戏再启动自动化")?;
    if window.identity.pid == std::process::id() {
        return Err("前台是 Kotone 自己的窗口：请先切到游戏再启动自动化".into());
    }
    Ok(RunTarget {
        handle: window.identity.handle,
        pid: window.identity.pid,
        process_started_at: window.identity.process_started_at,
        label: target_label(&window),
    })
}

/// 安排一段程序：目标是**现在**的前台窗口（游戏）；`due_at_ms` 不晚于现在就立即启动。
pub fn start(
    app: &AppHandle,
    session: &str,
    name: &str,
    source: &str,
    due_at_ms: u64,
) -> Result<Value, String> {
    let source = source.trim();
    if source.is_empty() || source.chars().count() > MAX_SOURCE_CHARS {
        return Err(format!("自动化内容为空或超过 {MAX_SOURCE_CHARS} 字"));
    }
    kotone_script::check_syntax(source).map_err(|e| format!("自动化语法错误：{e}"))?;
    let target = foreground_target()?;
    let label = target.label.clone();
    let run = scheduler(app)?.schedule_program(
        session,
        ProgramAction {
            name: name.to_string(),
            source: source.to_string(),
            automation_id: None,
            args: Default::default(),
            target: Some(target),
        },
        due_at_ms,
    )?;
    Ok(json!({
        "status": if run.status == RunStatus::Running { "started" } else { "scheduled" },
        "id": run.id,
        "target": label,
        "dueAtUnixMs": run.due_at_ms,
    }))
}

/// 调度器到点（或立即）调用：确认目标窗口还在，然后在新线程里运行程序。
fn launch(app: &AppHandle, run: &Run) -> Result<(), String> {
    let RunAction::Program(program) = &run.action else {
        return Err("不是程序".into());
    };
    let target = program.target.as_ref().ok_or("没有记录目标窗口")?;
    let identity = WindowIdentity {
        handle: target.handle,
        pid: target.pid,
        process_started_at: target.process_started_at,
    };
    if WindowsWindowObserver.inspect(&identity).is_err() {
        return Err(format!(
            "游戏窗口已关闭（{}），自动化没有启动",
            target.label
        ));
    }
    let id = run.id.clone();
    let cancel = Cancel::default();
    {
        let manager = app.state::<AutomationManager>();
        manager.live.lock().unwrap().insert(
            id.clone(),
            Live {
                cancel: cancel.clone(),
                end_reason: None,
            },
        );
    }
    let host = Arc::new(DesktopHost {
        app: app.clone(),
        target: identity,
        started: Instant::now(),
        held: Mutex::new(BTreeSet::new()),
        gone: AtomicBool::new(false),
        last_inspect: AtomicU64::new(0),
        cancel: cancel.clone(),
    });
    let thread_app = app.clone();
    let thread_id = id.clone();
    let code = program.source.clone();
    let spawned = std::thread::Builder::new()
        .name(format!("kotone-automation-{id}"))
        .spawn(move || {
            let report = kotone_script::run(
                &code,
                host.clone() as Arc<dyn ScriptHost>,
                Limits::default(),
                cancel,
            );
            host.release_all();
            kotone_core::log::log(&format!(
                "automation {thread_id}: {:?} keys={} {}ms",
                report.outcome, report.keys, report.duration_ms
            ));
            // 读停止原因时只短暂持有自己的锁，回报调度器时不持有（ProgramLauncher 的约定）；
            // 记录写完才从 live 移除，应用退出时据此等待“键已松开、记录已写”。
            let end_reason = thread_app
                .state::<AutomationManager>()
                .live
                .lock()
                .unwrap()
                .get(&thread_id)
                .and_then(|live| live.end_reason.clone());
            let gone = host.gone.load(Ordering::SeqCst);
            let (status, message, outcome) = match &report.outcome {
                Outcome::Finished => (RunStatus::Finished, None, "finished"),
                Outcome::Cancelled => (
                    RunStatus::Cancelled,
                    end_reason.or_else(|| gone.then(|| "游戏窗口已关闭".to_string())),
                    "stopped",
                ),
                Outcome::LimitReached(m) => (RunStatus::Failed, Some(m.clone()), "limit"),
                Outcome::Error(m) => (RunStatus::Failed, Some(m.clone()), "error"),
            };
            let summary = ProgramReport {
                outcome: outcome.into(),
                keys: u32::try_from(report.keys).unwrap_or(u32::MAX),
                duration_ms: report.duration_ms,
                log: report.log,
            };
            if let Ok(scheduler) = scheduler(&thread_app) {
                if let Err(e) = scheduler.finish_program(&thread_id, status, message, Some(summary))
                {
                    kotone_core::log::log(&format!("automation {thread_id}: record failed: {e}"));
                }
            }
            thread_app
                .state::<AutomationManager>()
                .live
                .lock()
                .unwrap()
                .remove(&thread_id);
        });
    if let Err(e) = spawned {
        app.state::<AutomationManager>()
            .live
            .lock()
            .unwrap()
            .remove(&id);
        return Err(format!("无法启动自动化线程：{e}"));
    }
    Ok(())
}

/// 取消 / 停止一条执行（`id`，提醒也可以）；`id = None` 停止全部程序（含等待中的，不含提醒）。
/// 返回涉及的条数。
pub fn stop(app: &AppHandle, id: Option<&str>, reason: &str) -> usize {
    let Ok(scheduler) = scheduler(app) else {
        return 0;
    };
    let result = match id {
        Some(id) => scheduler.cancel_with_reason(id, reason).map(usize::from),
        None => scheduler.stop_programs(reason),
    };
    match result {
        Ok(n) => {
            if n > 0 {
                kotone_core::log::log(&format!("automation: stopped {n} ({reason})"));
            }
            n
        }
        Err(e) => {
            kotone_core::log::log(&format!("automation: stop failed: {e}"));
            0
        }
    }
}

/// 接到调度器上：程序执行端口与结束事件。
pub fn attach(app: &AppHandle) {
    let Ok(scheduler) = scheduler(app) else {
        return;
    };
    let handle = app.clone();
    scheduler.set_on_finished(Arc::new(move |run: &Run| {
        crate::tool_service::publish(&handle, "automation.finished", run_json(run, false));
    }));
    scheduler.set_launcher(Arc::new(Launcher { app: app.clone() }));
}

/// 应用退出等程序线程收尾（松开按住的键、写结束记录）的最长时间。
const SHUTDOWN_WAIT: Duration = Duration::from_millis(1_500);

/// 应用退出：只停止运行中的程序，等待中的留给下次启动（过期则记为错过）。
/// 进程在本函数返回后就会结束，所以要等各线程松开按住的键、写完记录（有上限）。
pub fn shutdown(app: &AppHandle) {
    let Some(manager) = app.try_state::<AutomationManager>() else {
        return;
    };
    let count = {
        let mut live = manager.live.lock().unwrap();
        for run in live.values_mut() {
            run.cancel.cancel();
            run.end_reason
                .get_or_insert_with(|| "Kotone 退出".to_string());
        }
        live.len()
    };
    if count == 0 {
        return;
    }
    let deadline = Instant::now() + SHUTDOWN_WAIT;
    while !manager.live.lock().unwrap().is_empty() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    let left = manager.live.lock().unwrap().len();
    kotone_core::log::log(&format!(
        "automation: stopped {count} at exit, {left} did not finish in time"
    ));
}

#[tauri::command]
pub fn automation_runs(app: AppHandle) -> Result<Value, String> {
    list(&app)
}

#[tauri::command]
pub fn automation_stop(app: AppHandle, id: Option<String>) -> usize {
    stop(&app, id.as_deref(), "用户停止")
}

#[cfg(test)]
mod tests {
    use super::mean_luma;

    #[test]
    fn luma_of_black_white_and_red() {
        assert_eq!(mean_luma(&[0, 0, 0, 255, 0, 0, 0, 255]), 0.0);
        assert!((mean_luma(&[255, 255, 255, 255]) - 255.0).abs() < 1e-9);
        assert!((mean_luma(&[255, 0, 0, 255]) - 76.245).abs() < 1e-6);
        assert_eq!(mean_luma(&[]), 0.0);
    }
}
