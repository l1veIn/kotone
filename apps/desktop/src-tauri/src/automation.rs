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

use kotone_core::automation::library::{
    validate_args, Automation, AutomationBody, AutomationDraft, AutomationLibrary, CreatedBy,
    ParamKind,
};
use kotone_core::automation::{
    AutomationScheduler, ChatAction, ProgramAction, ProgramLauncher, ProgramReport, Run, RunAction,
    RunStatus, RunTarget, SpeakAction, MAX_RUNNING_PROGRAMS, MAX_SOURCE_CHARS,
};
use kotone_core::ocr::{OcrCancel, OcrInput, OcrParams};
use kotone_core::reminders::DeliveryKind;
use kotone_core::tools::{WindowCapturer, WindowIdentity, WindowInfo, WindowObserver};
use kotone_platform_windows::inject;
use kotone_platform_windows::window_tools::{GdiForegroundCapturer, WindowsWindowObserver};
use kotone_script::{
    Cancel, KeyAction, Limits, MouseAction, MouseButton, Outcome, Pixels, Region, ScriptHost,
    TextLine,
};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};

const MAX_CAPTURE_PIXELS: u64 = 3840 * 2160;
/// 工具 `automation.runs` 最多返回的条数。
const MAX_LISTED_RUNS: usize = 50;

/// 运行中的程序（只在内存里；执行记录在调度器）。
struct Live {
    cancel: Cancel,
    end_reason: Option<String>,
}

pub struct AutomationManager {
    live: Mutex<HashMap<String, Live>>,
    /// 已保存的自动化（`~/.kotone/automations.json`）。
    library: Result<Arc<AutomationLibrary>, String>,
}

impl AutomationManager {
    pub fn open() -> Self {
        let library =
            AutomationLibrary::open(kotone_core::settings::kotone_dir().join("automations.json"))
                .map(Arc::new);
        if let Err(e) = &library {
            kotone_core::log::log(&format!("automation library unavailable: {e}"));
        }
        Self {
            live: Mutex::new(HashMap::new()),
            library,
        }
    }
}

fn library(app: &AppHandle) -> Result<Arc<AutomationLibrary>, String> {
    app.state::<AutomationManager>().library.clone()
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
    /// 按住的鼠标键（结束时一起松开）。
    held_buttons: Mutex<Vec<MouseButton>>,
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
        Self::crop_with_origin(image, r).0
    }

    /// 按窗口比例裁剪；同时返回实际裁到的区域（按像素取整后，窗口比例）。
    fn crop_with_origin(
        image: &kotone_tools::image::RgbaImage,
        r: Region,
    ) -> (kotone_tools::image::RgbaImage, Region) {
        let (w, h) = (image.width(), image.height());
        let x = ((r.x * f64::from(w)).floor() as u32).min(w - 1);
        let y = ((r.y * f64::from(h)).floor() as u32).min(h - 1);
        let cw = ((r.width * f64::from(w)).ceil() as u32).clamp(1, w - x);
        let ch = ((r.height * f64::from(h)).ceil() as u32).clamp(1, h - y);
        let crop = image.crop(kotone_tools::image::CropRect {
            x,
            y,
            width: cw,
            height: ch,
        });
        let origin = Region {
            x: f64::from(x) / f64::from(w),
            y: f64::from(y) / f64::from(h),
            width: f64::from(cw) / f64::from(w),
            height: f64::from(ch) / f64::from(h),
        };
        (crop, origin)
    }

    /// 窗口比例 → 屏幕物理像素（按当前客户区位置）。
    fn screen_point(&self, x: f64, y: f64) -> Result<(i32, i32), String> {
        let info = WindowsWindowObserver
            .inspect(&self.target)
            .map_err(|e| format!("游戏窗口不可用：{e}"))?;
        let rect = info.client_rect.ok_or("拿不到游戏窗口的客户区位置")?;
        if rect.width == 0 || rect.height == 0 {
            return Err("游戏窗口客户区为空（可能最小化了）".into());
        }
        let px = rect.x + ((x * f64::from(rect.width)) as i32).min(rect.width as i32 - 1);
        let py = rect.y + ((y * f64::from(rect.height)) as i32).min(rect.height as i32 - 1);
        Ok((px, py))
    }

    fn release_all(&self) {
        let held: Vec<String> = std::mem::take(&mut *self.held.lock().unwrap())
            .into_iter()
            .collect();
        for key in held.iter().rev() {
            let _ = inject::key_up(key);
        }
        for button in std::mem::take(&mut *self.held_buttons.lock().unwrap()) {
            let _ = inject::mouse_button(platform_button(button), false);
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

fn platform_button(button: MouseButton) -> inject::MouseButton {
    match button {
        MouseButton::Left => inject::MouseButton::Left,
        MouseButton::Right => inject::MouseButton::Right,
        MouseButton::Middle => inject::MouseButton::Middle,
    }
}

/// 模板图片目录（内容寻址：`<sha256>.png`）。
fn images_dir() -> std::path::PathBuf {
    kotone_core::settings::kotone_dir().join("automation-images")
}

/// 模板图片上限：边长与文件大小。
const MAX_IMAGE_SIDE: u32 = 512;
const MAX_IMAGE_BYTES: usize = 512 * 1024;

/// 把提交的 `{png: base64}` 存成内容寻址文件，换成 `{image: <sha256>}`；已是引用的原样返回。
fn store_image(value: &Value) -> Result<Value, String> {
    let Some(b64) = value.get("png").and_then(Value::as_str) else {
        return Ok(value.clone());
    };
    if value.as_object().is_some_and(|o| o.len() != 1) {
        return Err("图片只能写成 {png: base64}".into());
    }
    let bytes = kotone_tools::image::decode_base64(b64.trim())
        .map_err(|e| format!("图片不是合法的 base64：{e}"))?;
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err(format!("图片超过 {} KB", MAX_IMAGE_BYTES / 1024));
    }
    kotone_tools::image::decode_png(&bytes, MAX_IMAGE_SIDE)?;
    let id = kotone_tools::image::sha256_hex(&bytes);
    let path = images_dir().join(format!("{id}.png"));
    if !path.exists() {
        std::fs::create_dir_all(images_dir()).map_err(|e| format!("创建图片目录失败：{e}"))?;
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, &bytes).map_err(|e| format!("保存图片失败：{e}"))?;
        std::fs::rename(&tmp, &path).map_err(|e| format!("保存图片失败：{e}"))?;
    }
    Ok(json!({"image": id}))
}

/// 草稿里 image 参数的缺省值、或运行入参里的图片：`{png}` → 引用。
fn store_draft_images(draft: &mut AutomationDraft) -> Result<(), String> {
    for spec in &mut draft.params {
        if spec.kind == ParamKind::Image {
            if let Some(default) = &spec.default {
                spec.default = Some(store_image(default)?);
            }
        }
    }
    Ok(())
}

fn store_arg_images(
    params: &[kotone_core::automation::library::ParamSpec],
    args: &serde_json::Map<String, Value>,
) -> Result<serde_json::Map<String, Value>, String> {
    let mut out = args.clone();
    for spec in params.iter().filter(|p| p.kind == ParamKind::Image) {
        if let Some(value) = args.get(&spec.name) {
            out.insert(spec.name.clone(), store_image(value)?);
        }
    }
    Ok(out)
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

    fn pixels(&self, region: Region) -> Result<Pixels, String> {
        let (crop, origin) = Self::crop_with_origin(&self.capture()?, region);
        Ok(Pixels {
            width: crop.width(),
            height: crop.height(),
            rgba: crop.pixels().to_vec(),
            origin,
        })
    }

    fn image(&self, id: &str) -> Result<Pixels, String> {
        if !kotone_core::automation::library::is_image_id(id) {
            return Err("图片编号不合法".into());
        }
        let bytes = std::fs::read(images_dir().join(format!("{id}.png")))
            .map_err(|e| format!("找不到模板图片（{e}）"))?;
        let image = kotone_tools::image::decode_png(&bytes, MAX_IMAGE_SIDE)?;
        Ok(Pixels {
            width: image.width(),
            height: image.height(),
            rgba: image.pixels().to_vec(),
            origin: Region {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            },
        })
    }

    fn mouse(&self, action: MouseAction) -> Result<(), String> {
        let err = |e: kotone_core::inject::InjectError| e.message;
        match action {
            MouseAction::Move { x, y } => {
                let (px, py) = self.screen_point(x, y)?;
                inject::mouse_move_to(px, py).map_err(err)
            }
            MouseAction::Click {
                x,
                y,
                button,
                count,
            } => {
                let (px, py) = self.screen_point(x, y)?;
                inject::mouse_move_to(px, py).map_err(err)?;
                inject::mouse_click(platform_button(button), count).map_err(err)
            }
            MouseAction::Down(button) => {
                inject::mouse_button(platform_button(button), true).map_err(err)?;
                self.held_buttons.lock().unwrap().push(button);
                Ok(())
            }
            MouseAction::Up(button) => {
                self.held_buttons.lock().unwrap().retain(|b| *b != button);
                inject::mouse_button(platform_button(button), false).map_err(err)
            }
            MouseAction::Scroll(clicks) => inject::mouse_wheel(clicks).map_err(err),
        }
    }

    fn ocr(&self, region: Region) -> Result<Vec<TextLine>, String> {
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
        let (crop, origin) = Self::crop_with_origin(&self.capture()?, region);
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
        // 识别框是裁剪图里的像素坐标，换回窗口比例。
        let (cw, ch) = (f64::from(crop.width()), f64::from(crop.height()));
        Ok(output
            .observations
            .into_iter()
            .map(|o| TextLine {
                rect: o.bbox.map(|b| Region {
                    x: origin.x + b.x / cw * origin.width,
                    y: origin.y + b.y / ch * origin.height,
                    width: b.width / cw * origin.width,
                    height: b.height / ch * origin.height,
                }),
                text: o.text,
            })
            .collect())
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
pub(crate) fn foreground_target() -> Result<RunTarget, String> {
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

/// 要安排的一段程序（临时写的，或来自已保存的自动化）。
pub struct ProgramRequest<'a> {
    pub name: &'a str,
    pub source: &'a str,
    pub automation_id: Option<String>,
    pub args: serde_json::Map<String, Value>,
}

fn check_source(source: &str) -> Result<&str, String> {
    let source = source.trim();
    if source.is_empty() || source.chars().count() > MAX_SOURCE_CHARS {
        return Err(format!("自动化内容为空或超过 {MAX_SOURCE_CHARS} 字"));
    }
    kotone_script::check_syntax(source).map_err(|e| format!("自动化语法错误：{e}"))?;
    Ok(source)
}

fn scheduled_json(run: &Run, target: Option<&str>) -> Value {
    json!({
        "status": if run.status == RunStatus::Running { "started" } else { "scheduled" },
        "id": run.id,
        "target": target,
        "dueAtUnixMs": run.due_at_ms,
    })
}

/// 安排一段程序：目标是**现在**的前台窗口（游戏）；`due_at_ms` 不晚于现在就立即启动。
pub fn start(
    app: &AppHandle,
    session: &str,
    request: ProgramRequest<'_>,
    due_at_ms: u64,
) -> Result<Value, String> {
    let source = check_source(request.source)?;
    let target = foreground_target()?;
    let label = target.label.clone();
    let run = scheduler(app)?.schedule_program(
        session,
        ProgramAction {
            name: request.name.to_string(),
            source: source.to_string(),
            automation_id: request.automation_id,
            args: request.args,
            target: Some(target),
        },
        due_at_ms,
    )?;
    Ok(scheduled_json(&run, Some(&label)))
}

/// 运行一个已保存的自动化：先按声明校验入参；程序以现在的前台窗口为目标，播报不需要窗口。
pub fn run_saved(
    app: &AppHandle,
    session: &str,
    id: &str,
    args: &serde_json::Map<String, Value>,
    due_at_ms: u64,
) -> Result<Value, String> {
    let automation = library(app)?
        .get(id)
        .ok_or_else(|| format!("找不到自动化 {id}"))?;
    let args = validate_args(
        &automation.params,
        &store_arg_images(&automation.params, args)?,
    )?;
    match &automation.action {
        AutomationBody::Program(program) => start(
            app,
            session,
            ProgramRequest {
                name: &automation.name,
                source: &program.source,
                automation_id: Some(automation.id.clone()),
                args,
            },
            due_at_ms,
        ),
        AutomationBody::Speak(speak) => {
            let run = scheduler(app)?.schedule_speak(session, speak.clone(), due_at_ms)?;
            Ok(scheduled_json(&run, None))
        }
    }
}

/// 内置 Agent 的提醒：延迟播报，进同一个调度器（ADR 013“延迟 + 播报”）。
/// `delivery` 为空时用助手的默认方式。
pub fn schedule_reminder(
    app: &AppHandle,
    session: &str,
    text: String,
    delivery: Vec<DeliveryKind>,
    delay_seconds: u64,
) -> Result<Value, String> {
    let delivery = if delivery.is_empty() {
        app.state::<crate::SharedState>()
            .settings
            .read()
            .unwrap()
            .assistant
            .default_delivery
            .clone()
    } else {
        delivery
    };
    let due = kotone_core::automation::now_ms() + delay_seconds.saturating_mul(1_000);
    let run = scheduler(app)?.schedule_speak(session, SpeakAction { text, delivery }, due)?;
    Ok(scheduled_json(&run, None))
}

/// 内置 Agent 的“发到游戏聊天”：记一条执行记录（ADR 014“已确认”：替玩家发了什么要看得到）。
pub fn record_chat(
    app: &AppHandle,
    session: &str,
    action: ChatAction,
    result: Result<(), String>,
) -> Result<Run, String> {
    scheduler(app)?.record_chat(session, action, result)
}

/// 已保存的自动化（按名字排序；库不可用时为空）。
pub fn saved_list(app: &AppHandle) -> Vec<Automation> {
    library(app).map(|l| l.list()).unwrap_or_default()
}

/// 定义的对外形状；`with_source = false`（列表）时省略程序源码。
pub fn automation_json(automation: &Automation, with_source: bool) -> Value {
    let mut value = serde_json::to_value(automation).unwrap_or(Value::Null);
    if !with_source {
        if let Some(action) = value.get_mut("action").and_then(Value::as_object_mut) {
            action.remove("source");
        }
    }
    value
}

/// 保存（新建或按 id 覆盖）；程序先检查语法。
pub fn save(app: &AppHandle, mut draft: AutomationDraft, by: CreatedBy) -> Result<Value, String> {
    if let AutomationBody::Program(program) = &mut draft.action {
        program.source = check_source(&program.source)?.to_string();
    }
    store_draft_images(&mut draft)?;
    let saved = library(app)?.save(draft, by)?;
    let _ = app.emit("kotone://automations-updated", ());
    crate::tool_service::publish(
        app,
        "automation.saved",
        json!({"id": saved.id, "name": saved.name}),
    );
    Ok(automation_json(&saved, true))
}

pub fn get(app: &AppHandle, id: &str) -> Result<Value, String> {
    library(app)?
        .get(id)
        .map(|a| automation_json(&a, true))
        .ok_or_else(|| format!("找不到自动化 {id}"))
}

pub fn list_saved(app: &AppHandle) -> Result<Value, String> {
    let list = library(app)?.list();
    Ok(json!({
        "automations": list.iter().map(|a| automation_json(a, false)).collect::<Vec<_>>(),
        "max": kotone_core::automation::library::MAX_AUTOMATIONS,
    }))
}

pub fn delete(app: &AppHandle, id: &str) -> Result<bool, String> {
    let deleted = library(app)?.delete(id)?;
    if deleted {
        let _ = app.emit("kotone://automations-updated", ());
        crate::tool_service::publish(app, "automation.deleted", json!({"id": id}));
    }
    Ok(deleted)
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
        held_buttons: Mutex::new(Vec::new()),
        gone: AtomicBool::new(false),
        last_inspect: AtomicU64::new(0),
        cancel: cancel.clone(),
    });
    let thread_app = app.clone();
    let thread_id = id.clone();
    let code = program.source.clone();
    let args = program.args.clone();
    let spawned = std::thread::Builder::new()
        .name(format!("kotone-automation-{id}"))
        .spawn(move || {
            let report = kotone_script::run_with_args(
                &code,
                &args,
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
        crate::agent::on_run_finished(&handle, run);
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
pub fn automation_saved_list(app: AppHandle) -> Result<Value, String> {
    list_saved(&app)
}

#[tauri::command]
pub fn automation_get(app: AppHandle, id: String) -> Result<Value, String> {
    get(&app, &id)
}

/// 设置页保存（创建者记为用户）。
#[tauri::command]
pub fn automation_save(app: AppHandle, draft: AutomationDraft) -> Result<Value, String> {
    save(&app, draft, CreatedBy::User)
}

#[tauri::command]
pub fn automation_delete(app: AppHandle, id: String) -> Result<bool, String> {
    delete(&app, &id)
}

/// 设置页“运行”：点按钮时前台是 Kotone 自己，所以程序类先倒数 `switch_seconds` 秒
/// （1–30，让用户切到游戏），再以那时的前台窗口为目标运行；入参先校验，错了立即返回。
/// 倒数后的失败（例如前台仍是 Kotone）用语音 / 弹幕告诉用户。播报类立即运行。
#[tauri::command]
pub fn automation_run_saved(
    app: AppHandle,
    id: String,
    args: serde_json::Map<String, Value>,
    switch_seconds: u64,
) -> Result<Value, String> {
    let automation = library(&app)?
        .get(&id)
        .ok_or_else(|| format!("找不到自动化 {id}"))?;
    let stored = store_arg_images(&automation.params, &args)?;
    validate_args(&automation.params, &stored)?;
    if matches!(automation.action, AutomationBody::Speak(_)) {
        return run_saved(&app, "ui", &id, &stored, kotone_core::automation::now_ms());
    }
    let seconds = switch_seconds.clamp(1, 30);
    std::thread::Builder::new()
        .name("kotone-automation-countdown".into())
        .spawn(move || {
            std::thread::sleep(Duration::from_secs(seconds));
            if let Err(error) =
                run_saved(&app, "ui", &id, &stored, kotone_core::automation::now_ms())
            {
                kotone_core::log::log(&format!("automation {id}: ui run failed: {error}"));
                let app2 = app.clone();
                tauri::async_runtime::spawn(async move {
                    let _ = crate::assistant::deliver_text(
                        &app2,
                        format!("自动化没能运行：{error}"),
                        Vec::new(),
                    )
                    .await;
                });
            }
        })
        .map_err(|e| format!("无法开始倒数：{e}"))?;
    Ok(json!({"status": "countdown", "seconds": seconds}))
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
