//! 陪玩脚本的桌面执行器（docs/product-evolution-game-tools.md §12）。
//!
//! - 每个脚本在自己的线程里由 `kotone_script::run` 执行；宿主 [`DesktopHost`] 把白名单函数接到
//!   真实能力：按键 / 打字走 `SendInput`（与“帮忙打字”同一条输入通道），看屏幕用 GDI 截游戏
//!   客户区，`ocr()` 用陪玩选定的 OCR 模型，`say` / `toast` 复用提醒的交付路径。
//! - **只对启动时的游戏窗口生效**：启动脚本时记下前台窗口；按键、截图前先确认它在前台，不在
//!   就暂停等待；窗口关闭则结束脚本。
//! - **按住的键一定会松开**：`key_down` 记账，脚本结束（正常、出错、取消、超限）时全部 `key_up`。
//! - 同时最多运行 [`MAX_RUNNING`] 个脚本；停止脚本热键、托盘菜单、工具 `script.stop` 与应用退出
//!   都会立即取消。运行状态经 `kotone://scripts` 推给界面，结束经工具事件 `script.finished` 发布。

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use kotone_core::ocr::{OcrCancel, OcrInput, OcrParams};
use kotone_core::reminders::DeliveryKind;
use kotone_core::tools::{WindowCapturer, WindowIdentity, WindowInfo, WindowObserver};
use kotone_platform_windows::inject;
use kotone_platform_windows::window_tools::{GdiForegroundCapturer, WindowsWindowObserver};
use kotone_script::{Cancel, KeyAction, Limits, Outcome, Region, RunReport, ScriptHost};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};

pub const MAX_RUNNING: usize = 3;
const MAX_CAPTURE_PIXELS: u64 = 3840 * 2160;
const MAX_SOURCE_CHARS: usize = 8_000;

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// 一次运行的记录（界面与工具展示用；不含脚本以外的用户数据）。
#[derive(Clone)]
struct Run {
    id: String,
    name: String,
    source: String,
    target: String,
    started_at_ms: u64,
    cancel: Cancel,
    report: Option<RunReport>,
    end_reason: Option<String>,
}

impl Run {
    fn to_json(&self) -> Value {
        let (status, message) = match &self.report {
            None => ("running", None),
            Some(report) => match &report.outcome {
                Outcome::Finished => ("finished", None),
                Outcome::Cancelled => ("stopped", self.end_reason.clone()),
                Outcome::LimitReached(m) => ("limit", Some(m.clone())),
                Outcome::Error(m) => ("error", Some(m.clone())),
            },
        };
        json!({
            "id": self.id,
            "name": self.name,
            "target": self.target,
            "status": status,
            "message": message,
            "startedAtUnixMs": self.started_at_ms,
            "keys": self.report.as_ref().map(|r| r.keys),
            "durationMs": self.report.as_ref().map(|r| r.duration_ms),
            "log": self.report.as_ref().map(|r| r.log.clone()).unwrap_or_default(),
            "source": self.source,
        })
    }
}

#[derive(Default)]
pub struct ScriptManager {
    runs: Mutex<Vec<Run>>,
}

impl ScriptManager {
    fn running(&self) -> usize {
        self.runs
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.report.is_none())
            .count()
    }
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
                return Err("脚本已停止".into());
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

fn emit(app: &AppHandle) {
    let _ = app.emit("kotone://scripts", list(app));
}

/// 当前与最近的运行记录（新的在前，最多 20 条）。
pub fn list(app: &AppHandle) -> Value {
    let manager = app.state::<ScriptManager>();
    let runs = manager.runs.lock().unwrap();
    json!({
        "running": runs.iter().filter(|r| r.report.is_none()).count(),
        "maxRunning": MAX_RUNNING,
        "runs": runs.iter().rev().take(20).map(Run::to_json).collect::<Vec<_>>(),
    })
}

/// 在当前前台窗口（游戏）上启动脚本，立即返回运行 ID。
pub fn start(app: &AppHandle, name: &str, source: &str) -> Result<Value, String> {
    let source = source.trim();
    if source.is_empty() || source.chars().count() > MAX_SOURCE_CHARS {
        return Err(format!("脚本为空或超过 {MAX_SOURCE_CHARS} 字"));
    }
    kotone_script::check_syntax(source).map_err(|e| format!("脚本语法错误：{e}"))?;
    let manager = app.state::<ScriptManager>();
    if manager.running() >= MAX_RUNNING {
        return Err(format!("最多同时运行 {MAX_RUNNING} 个脚本，请先停止一个"));
    }
    let window = WindowsWindowObserver
        .foreground_window()
        .map_err(|e| format!("找不到前台窗口：{e}"))?
        .ok_or("当前没有前台窗口：请先切到游戏再启动脚本")?;
    let own_pid = std::process::id();
    if window.identity.pid == own_pid {
        return Err("前台是 Kotone 自己的窗口：请先切到游戏再启动脚本".into());
    }
    let id = kotone_tools::ids::new_id("scr");
    let cancel = Cancel::default();
    let run = Run {
        id: id.clone(),
        name: name.trim().chars().take(40).collect(),
        source: source.to_string(),
        target: target_label(&window),
        started_at_ms: unix_ms(),
        cancel: cancel.clone(),
        report: None,
        end_reason: None,
    };
    {
        let mut runs = manager.runs.lock().unwrap();
        runs.push(run);
        while runs.len() > 40 {
            match runs.iter().position(|r| r.report.is_some()) {
                Some(i) => {
                    runs.remove(i);
                }
                None => break,
            }
        }
    }
    let host = Arc::new(DesktopHost {
        app: app.clone(),
        target: window.identity,
        started: Instant::now(),
        held: Mutex::new(BTreeSet::new()),
        gone: AtomicBool::new(false),
        last_inspect: AtomicU64::new(0),
        cancel: cancel.clone(),
    });
    let thread_app = app.clone();
    let thread_id = id.clone();
    let code = source.to_string();
    std::thread::Builder::new()
        .name(format!("kotone-script-{id}"))
        .spawn(move || {
            let report = kotone_script::run(
                &code,
                host.clone() as Arc<dyn ScriptHost>,
                Limits::default(),
                cancel,
            );
            host.release_all();
            let reason = host
                .gone
                .load(Ordering::SeqCst)
                .then(|| "游戏窗口已关闭".to_string());
            kotone_core::log::log(&format!(
                "script {thread_id}: {:?} keys={} {}ms",
                report.outcome, report.keys, report.duration_ms
            ));
            let finished = {
                let manager = thread_app.state::<ScriptManager>();
                let mut runs = manager.runs.lock().unwrap();
                runs.iter_mut().find(|r| r.id == thread_id).map(|run| {
                    run.report = Some(report);
                    if run.end_reason.is_none() {
                        run.end_reason = reason;
                    }
                    run.to_json()
                })
            };
            if let Some(mut finished) = finished {
                if let Some(obj) = finished.as_object_mut() {
                    obj.remove("source");
                }
                crate::tool_service::publish(&thread_app, "script.finished", finished);
            }
            emit(&thread_app);
        })
        .map_err(|e| format!("无法启动脚本线程：{e}"))?;
    emit(app);
    Ok(json!({"status": "started", "id": id, "target": target_label(&window)}))
}

/// 停止一个脚本；`id = None` 停止全部。返回被停止的个数。
pub fn stop(app: &AppHandle, id: Option<&str>, reason: &str) -> usize {
    let Some(manager) = app.try_state::<ScriptManager>() else {
        return 0;
    };
    let mut stopped = 0;
    for run in manager.runs.lock().unwrap().iter_mut() {
        if run.report.is_none() && id.map_or(true, |id| run.id == id) {
            run.cancel.cancel();
            run.end_reason.get_or_insert_with(|| reason.to_string());
            stopped += 1;
        }
    }
    if stopped > 0 {
        kotone_core::log::log(&format!("scripts: stopped {stopped} ({reason})"));
    }
    stopped
}

/// 应用退出：停止全部脚本（各脚本线程随后松开按住的键）。
pub fn shutdown(app: &AppHandle) {
    stop(app, None, "Kotone 退出");
}

#[tauri::command]
pub fn script_list(app: AppHandle) -> Value {
    list(&app)
}

#[tauri::command]
pub fn script_stop(app: AppHandle, id: Option<String>) -> usize {
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
