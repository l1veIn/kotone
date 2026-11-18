//! 琴音在游戏里“看屏幕、动鼠标”（ADR 019）：工具 `game.look` / `game.act` 的桌面实现。
//!
//! 纯逻辑（动作清单、缩放、编号、标签、批量规则）在 `kotone_tools::computer_use`；这里接上
//! 真实能力：绑定前台游戏窗口（共享的 [`GameSession`]）、截图、文字识别、`SendInput`、
//! 停止（操作时玩家按 Esc——钩子拦下不传给游戏、停止热键、语音“停下”）与气泡里的文字提示。
//! 玩家自己动键盘鼠标不会让她停（2026-11-17 用户的决定，ADR 019 实施记录）。
//!
//! - 只截游戏窗口客户区，不截桌面；纯色画面（独占全屏）报错；
//! - 编号与坐标只对最近一次 `game.look` 有效；窗口尺寸变了作废，只移动窗口不作废；
//! - 按游戏授权：游戏配置的“允许自动化输入”“让琴音操作”（`profile.rs`），哪些游戏能操作由玩家判断；
//! - 与自动化程序互斥：程序在跑时不操作，操作时程序不启动；
//! - 停止：停止自动化热键、语音“停下”、本轮停止都会取消正在进行的操作，并不再自动重试。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use kotone_core::profile;
use kotone_core::tools::{WindowIdentity, WindowInfo, WindowObserver};
use kotone_platform_windows::hotkey_ll;
use kotone_platform_windows::inject;
use kotone_platform_windows::window_tools::WindowsWindowObserver;
use kotone_tools::computer_use::actions::{self, Action, Button, ScrollDirection};
use kotone_tools::computer_use::batch::{self, ActHost, Halt, StopReason};
use kotone_tools::computer_use::observe::{self, Observation, TextBox};
use kotone_tools::computer_use::{annotate, elements_json, report_json};
use kotone_tools::image::{CropRect, RgbaImage};
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

use crate::game_session::{ratio_to_screen, GameSession};

/// 截图的 JPEG 质量。
const JPEG_QUALITY: u8 = 80;
/// 游戏不在前台时最多等多久（之后按 `not_foreground` 停下）。
const FOREGROUND_WAIT: Duration = Duration::from_secs(5);
/// 做完以后等游戏重绘一下再看。
const SETTLE_BEFORE_LOOK: Duration = Duration::from_millis(400);

/// 最近一次 look（编号与坐标只对它有效）。
struct Latest {
    observation: Observation,
    target: WindowIdentity,
    /// 截图时窗口客户区的尺寸（物理像素）。
    client: (u32, u32),
}

#[derive(Default)]
struct Inner {
    latest: Option<Latest>,
    next_id: u64,
    /// 正在进行的 `game.act` 的停止标记。
    active: Option<Arc<AtomicBool>>,
}

#[derive(Default)]
pub struct ComputerUse {
    inner: Mutex<Inner>,
}

fn state(app: &AppHandle) -> Result<tauri::State<'_, ComputerUse>, String> {
    app.try_state::<ComputerUse>()
        .ok_or_else(|| "computer use 尚未就绪".to_string())
}

/// 正在操作游戏（自动化程序据此不启动，避免两边同时按键）。
pub fn busy(app: &AppHandle) -> bool {
    app.try_state::<ComputerUse>()
        .is_some_and(|s| s.inner.lock().unwrap().active.is_some())
}

/// 最近一次 look 的游戏在配置里“让琴音操作”是一直允许（内置 Agent 据此不再询问）。
pub fn always_allowed(app: &AppHandle) -> bool {
    let Some(target) = app
        .try_state::<ComputerUse>()
        .and_then(|s| s.inner.lock().unwrap().latest.as_ref().map(|l| l.target))
    else {
        return false;
    };
    let Ok(window) = WindowsWindowObserver.inspect(&target) else {
        return false;
    };
    let process = window.process_name.unwrap_or_default();
    let game = profile::find_by_process(&profile::list(), &process);
    profile::computer_use_verdict(game.as_ref()) == profile::ComputerUseVerdict::Allowed
}

/// 停止正在进行的操作（停止热键、语音“停下”、本轮停止）。返回是否有操作被停下。
pub fn stop_all(app: &AppHandle, reason: &str) -> bool {
    let Some(s) = app.try_state::<ComputerUse>() else {
        return false;
    };
    let active = s.inner.lock().unwrap().active.clone();
    match active {
        Some(flag) => {
            flag.store(true, Ordering::SeqCst);
            kotone_core::log::log(&format!("computer use: stop ({reason})"));
            true
        }
        None => false,
    }
}

fn foreground_game() -> Result<WindowInfo, String> {
    let window = WindowsWindowObserver
        .foreground_window()
        .map_err(|e| format!("找不到前台窗口：{e}"))?
        .ok_or("当前没有前台窗口：先用 game.focus 切到游戏")?;
    if window.identity.pid == std::process::id() {
        return Err("前台是 Kotone 自己的窗口：先用 game.focus 切到游戏".into());
    }
    Ok(window)
}

/// OCR 结果 → 文字框（窗口比例）。识别不可用时返回原因，截图照常给出。
fn recognize(app: &AppHandle, image: &RgbaImage) -> (Vec<TextBox>, Option<String>) {
    match crate::automation::ocr_image(app, image) {
        Ok(lines) => (
            lines
                .into_iter()
                .filter_map(|(text, rect)| rect.map(|ratio| TextBox { text, ratio }))
                .collect(),
            None,
        ),
        Err(e) => (Vec::new(), Some(e)),
    }
}

fn crop_ratio(image: &RgbaImage, [x, y, w, h]: [f64; 4]) -> RgbaImage {
    let (iw, ih) = (f64::from(image.width()), f64::from(image.height()));
    let x0 = ((x * iw).floor() as u32).min(image.width() - 1);
    let y0 = ((y * ih).floor() as u32).min(image.height() - 1);
    let cw = (((x + w) * iw).ceil() as u32).clamp(x0 + 1, image.width()) - x0;
    let ch = (((y + h) * ih).ceil() as u32).clamp(y0 + 1, image.height()) - y0;
    image.crop(CropRect {
        x: x0,
        y: y0,
        width: cw,
        height: ch,
    })
}

/// 看一眼：截图、识别、编号、记下 observation，返回给模型的结果（含 JPEG）。
pub fn look(app: &AppHandle, zoom: Option<[f64; 4]>) -> Result<Value, String> {
    let window = foreground_game()?;
    let session = GameSession::new(window.identity);
    let capture = session.capture()?;
    let client = (capture.width(), capture.height());
    let state = state(app)?;
    // zoom 用上一张完整截图的坐标系（同一个窗口、同样大小）。
    let base = if zoom.is_some() {
        let inner = state.inner.lock().unwrap();
        let latest = inner
            .latest
            .as_ref()
            .filter(|l| l.target == window.identity)
            .ok_or("zoom 之前要先完整地 game.look 一次")?;
        if latest.client != client {
            return Err("游戏窗口尺寸变了：先不带 zoom 重新 game.look".into());
        }
        Some((
            latest.observation.image_width,
            latest.observation.image_height,
        ))
    } else {
        None
    };
    let (image, elements, coordinate_size, ocr_note) = match (zoom, base) {
        (Some(zoom), Some((full_w, full_h))) => {
            let region = observe::zoom_region(full_w, full_h, zoom)?;
            let crop = crop_ratio(&capture, region);
            let scale = observe::scale_for(
                crop.width(),
                crop.height(),
                observe::DEFAULT_LONG_EDGE,
                observe::DEFAULT_MAX_PIXELS,
            );
            let shown = annotate::resize(&crop, scale.width, scale.height);
            let (lines, note) = recognize(app, &crop);
            // 识别框：裁剪图比例 → 整窗比例（元素用整张截图的坐标系）。
            let [rx, ry, rw, rh] = region;
            let full_lines: Vec<TextBox> = lines
                .iter()
                .map(|l| TextBox {
                    text: l.text.clone(),
                    ratio: [
                        rx + l.ratio[0] * rw,
                        ry + l.ratio[1] * rh,
                        l.ratio[2] * rw,
                        l.ratio[3] * rh,
                    ],
                })
                .collect();
            let elements = observe::elements_from_text(&full_lines, full_w, full_h);
            // 画在放大图上：整张截图像素 → 放大图像素。
            let (sx, sy) = (
                f64::from(shown.width()) / (rw * f64::from(full_w)),
                f64::from(shown.height()) / (rh * f64::from(full_h)),
            );
            let (ox, oy) = (rx * f64::from(full_w), ry * f64::from(full_h));
            let boxes: Vec<(u32, [u32; 4])> = elements
                .iter()
                .map(|e| {
                    let map = |v: u32, o: f64, s: f64, max: u32| {
                        (((f64::from(v) - o) * s).max(0.0) as u32).min(max)
                    };
                    (
                        e.id,
                        [
                            map(e.bbox[0], ox, sx, shown.width() - 1),
                            map(e.bbox[1], oy, sy, shown.height() - 1),
                            map(e.bbox[2], ox, sx, shown.width()),
                            map(e.bbox[3], oy, sy, shown.height()),
                        ],
                    )
                })
                .collect();
            (
                annotate::annotate(&shown, &boxes),
                elements,
                (full_w, full_h),
                note,
            )
        }
        _ => {
            let scale = observe::scale_for(
                client.0,
                client.1,
                observe::DEFAULT_LONG_EDGE,
                observe::DEFAULT_MAX_PIXELS,
            );
            let shown = annotate::resize(&capture, scale.width, scale.height);
            let (lines, note) = recognize(app, &capture);
            let elements = observe::elements_from_text(&lines, scale.width, scale.height);
            let boxes: Vec<(u32, [u32; 4])> = elements.iter().map(|e| (e.id, e.bbox)).collect();
            (
                annotate::annotate(&shown, &boxes),
                elements,
                (scale.width, scale.height),
                note,
            )
        }
    };
    let jpeg = annotate::encode_jpeg(&image, JPEG_QUALITY)?;
    let id = {
        let mut inner = state.inner.lock().unwrap();
        inner.next_id += 1;
        let id = format!("look-{}", inner.next_id);
        inner.latest = Some(Latest {
            observation: Observation {
                id: id.clone(),
                client_width: client.0,
                client_height: client.1,
                image_width: coordinate_size.0,
                image_height: coordinate_size.1,
                elements: elements.clone(),
            },
            target: window.identity,
            client,
        });
        id
    };
    let mut out = json!({
        "observationId": id,
        "screenshot": {"width": coordinate_size.0, "height": coordinate_size.1},
        "window": {
            "title": window.title,
            "process": window.process_name,
            "width": client.0,
            "height": client.1,
        },
        "elements": elements_json(&elements),
        "image": {
            "mimeType": "image/jpeg",
            "base64": kotone_tools::image::encode_base64(&jpeg),
            "width": image.width(),
            "height": image.height(),
        },
    });
    if let Some(zoom) = zoom {
        out["zoom"] = json!(zoom);
    }
    if let Some(note) = ocr_note {
        out["ocr"] = json!(format!("没有文字编号：{note}"));
    }
    Ok(out)
}

/// 停止标记：工具调用被丢弃（本轮停止、超时）时一并停下。
struct ActGuard {
    app: AppHandle,
    flag: Arc<AtomicBool>,
}

impl Drop for ActGuard {
    fn drop(&mut self) {
        self.flag.store(true, Ordering::SeqCst);
        hotkey_ll::arm_esc_stop(false);
        if let Some(s) = self.app.try_state::<ComputerUse>() {
            let mut inner = s.inner.lock().unwrap();
            if inner
                .active
                .as_ref()
                .is_some_and(|a| Arc::ptr_eq(a, &self.flag))
            {
                inner.active = None;
            }
        }
    }
}

/// 检查通过后的操作上下文：observation、目标窗口、客户区尺寸、停止标记。
type Prepared = (Observation, WindowIdentity, (u32, u32), Arc<AtomicBool>);

/// 开始一次操作前的检查：observation 是最新的、游戏允许、没有别的输入在进行。
fn prepare(app: &AppHandle, observation_id: &str) -> Result<Prepared, String> {
    let state = state(app)?;
    let (observation, target, client) = {
        let inner = state.inner.lock().unwrap();
        let latest = inner
            .latest
            .as_ref()
            .ok_or("还没有看过画面：先 game.look")?;
        if latest.observation.id != observation_id {
            return Err(format!(
                "observationId {observation_id} 不是最近一次 game.look（最近是 {}）：先 game.look 再操作",
                latest.observation.id
            ));
        }
        (latest.observation.clone(), latest.target, latest.client)
    };
    let window = WindowsWindowObserver
        .inspect(&target)
        .map_err(|_| "游戏窗口已经关闭".to_string())?;
    let process = window.process_name.clone().unwrap_or_default();
    let game = profile::find_by_process(&profile::list(), &process);
    if let profile::ComputerUseVerdict::Denied(reason) =
        profile::computer_use_verdict(game.as_ref())
    {
        return Err(format!("没有操作：{reason}"));
    }
    if crate::automation::program_running(app) {
        return Err("有自动化程序正在运行，不能同时操作：先等它结束或停止它".into());
    }
    let flag = Arc::new(AtomicBool::new(false));
    {
        let mut inner = state.inner.lock().unwrap();
        if inner.active.is_some() {
            return Err("上一次操作还没结束".into());
        }
        inner.active = Some(flag.clone());
    }
    Ok((observation, target, client, flag))
}

/// 真正按键、动鼠标的一方。
struct DesktopActHost {
    session: GameSession,
    client: (u32, u32),
    stop: Arc<AtomicBool>,
}

impl DesktopActHost {
    fn interrupted(&self) -> Option<StopReason> {
        (self.stop.load(Ordering::SeqCst) || hotkey_ll::esc_stop_pressed())
            .then_some(StopReason::Cancelled)
    }

    fn halt_if_interrupted(&self) -> Result<(), Halt> {
        self.interrupted().map_or(Ok(()), |r| Err(Halt::Stop(r)))
    }

    fn input(result: Result<(), kotone_core::inject::InjectError>) -> Result<(), Halt> {
        result.map_err(|e| Halt::Fail(e.message))
    }
}

fn platform_button(button: Button) -> inject::MouseButton {
    match button {
        Button::Left => inject::MouseButton::Left,
        Button::Right => inject::MouseButton::Right,
        Button::Middle => inject::MouseButton::Middle,
    }
}

impl ActHost for DesktopActHost {
    fn key_known(&self, keys: &str) -> bool {
        inject::key_spec_valid(keys)
    }

    fn checkpoint(&self) -> Result<(), Halt> {
        self.halt_if_interrupted()?;
        if !self.session.check_alive() {
            return Err(Halt::Stop(StopReason::WindowClosed));
        }
        let deadline = Instant::now() + FOREGROUND_WAIT;
        while !self.session.focused_now() {
            if Instant::now() >= deadline {
                return Err(Halt::Stop(StopReason::NotForeground));
            }
            std::thread::sleep(Duration::from_millis(50));
            self.halt_if_interrupted()?;
        }
        let rect = self
            .session
            .client_rect()
            .map_err(|_| Halt::Stop(StopReason::WindowClosed))?;
        if (rect.width, rect.height) != self.client {
            return Err(Halt::Stop(StopReason::WindowChanged));
        }
        Ok(())
    }

    fn move_to(&self, x: f64, y: f64) -> Result<(), Halt> {
        self.halt_if_interrupted()?;
        let rect = self
            .session
            .client_rect()
            .map_err(|_| Halt::Stop(StopReason::WindowClosed))?;
        if (rect.width, rect.height) != self.client {
            return Err(Halt::Stop(StopReason::WindowChanged));
        }
        let (px, py) = ratio_to_screen(&rect, x, y);
        if kotone_platform_windows::game_window::process_at(px, py)
            .is_some_and(|pid| pid != self.session.target.pid)
        {
            return Err(Halt::Stop(StopReason::PointCovered));
        }
        Self::input(inject::mouse_move_to(px, py))
    }

    fn button(&self, button: Button, down: bool) -> Result<(), Halt> {
        let button = platform_button(button);
        if down {
            self.halt_if_interrupted()?;
            self.session.mouse_down(button).map_err(Halt::Fail)
        } else {
            self.session.mouse_up(button).map_err(Halt::Fail)
        }
    }

    fn click(&self, button: Button, count: u8) -> Result<(), Halt> {
        self.halt_if_interrupted()?;
        Self::input(inject::mouse_click(platform_button(button), count))
    }

    fn wheel(&self, direction: ScrollDirection, amount: u32) -> Result<(), Halt> {
        self.halt_if_interrupted()?;
        let n = amount as i32;
        Self::input(match direction {
            ScrollDirection::Up => inject::mouse_wheel(n),
            ScrollDirection::Down => inject::mouse_wheel(-n),
            ScrollDirection::Right => inject::mouse_hwheel(n),
            ScrollDirection::Left => inject::mouse_hwheel(-n),
        })
    }

    fn type_text(&self, text: &str) -> Result<(), Halt> {
        self.halt_if_interrupted()?;
        Self::input(inject::send_unicode(text))
    }

    fn key_press(&self, keys: &str) -> Result<(), Halt> {
        self.halt_if_interrupted()?;
        Self::input(inject::key_down_up(keys))
    }

    fn key_down(&self, keys: &str) -> Result<(), Halt> {
        self.halt_if_interrupted()?;
        self.session.key_down(keys).map_err(Halt::Fail)
    }

    fn key_up(&self, keys: &str) -> Result<(), Halt> {
        self.session.key_up(keys).map_err(Halt::Fail)
    }

    fn move_by(&self, dx: i32, dy: i32) -> Result<(), Halt> {
        self.halt_if_interrupted()?;
        Self::input(inject::mouse_move_by(dx, dy))
    }

    fn sleep(&self, ms: u64) -> Result<(), Halt> {
        let deadline = Instant::now() + Duration::from_millis(ms);
        loop {
            self.halt_if_interrupted()?;
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Ok(());
            }
            std::thread::sleep(left.min(Duration::from_millis(10)));
        }
    }
}

/// 一次操作：校验 → 执行 → 松开 → 再看一眼。阻塞执行，调用方放在阻塞线程里。
pub fn act(
    app: &AppHandle,
    observation_id: &str,
    raw_actions: &Value,
    flag_out: &Mutex<Option<Arc<AtomicBool>>>,
) -> Result<Value, String> {
    let plan = actions::parse_plan(raw_actions)?;
    let (observation, target, client, flag) = prepare(app, observation_id)?;
    *flag_out.lock().unwrap() = Some(flag.clone());
    let _guard = ActGuard {
        app: app.clone(),
        flag: flag.clone(),
    };
    let host = DesktopActHost {
        session: GameSession::new(target),
        client,
        stop: flag.clone(),
    };
    // 操作期间玩家按 Esc 就停（钩子拦下这个 Esc，不传给游戏）。
    hotkey_ll::arm_esc_stop(true);
    batch::validate(&plan, &observation, &host)?;
    let total = plan.len();
    let on_step = |i: usize, action: &Action| {
        crate::bubble::acting(app, &format!("{}（{}/{total}）", action.describe(), i + 1));
    };
    let report = batch::execute(&plan, &observation, &host, &on_step);
    let esc = hotkey_ll::esc_stop_pressed();
    hotkey_ll::arm_esc_stop(false);
    host.session.release_all();
    crate::bubble::acting_done(app);
    kotone_core::log::log(&format!(
        "computer use: act {}/{} sent, stop={:?}",
        report.sent(),
        total,
        report.stop.map(StopReason::code)
    ));
    let mut out = report_json(&report);
    if !hotkey_ll::hooks_running() {
        out["warning"] = json!("键鼠钩子没有运行：按 Esc 停不下来，只能说“停下”");
    }
    // 结尾再看一眼（dsh-computer-use-win）；玩家让停了就不看。
    let look_again = !matches!(
        report.stop,
        Some(StopReason::Cancelled | StopReason::WindowClosed)
    );
    drop(_guard);
    if esc {
        // 按 Esc = 停止整件事：这一轮也结束，琴音不再接着做（与停止热键一致）。
        kotone_core::log::log("computer use: Esc pressed, stopping the turn");
        let _ = crate::agent::stop(app);
        return Ok(out);
    }
    if look_again {
        std::thread::sleep(SETTLE_BEFORE_LOOK);
        match look(app, None) {
            Ok(after) => out["look"] = after,
            Err(e) => out["look"] = json!({"error": e}),
        }
    }
    Ok(out)
}

/// 工具调用被丢弃时（本轮停止、超时）停下进行中的操作。
pub struct CallGuard {
    pub flag: Arc<Mutex<Option<Arc<AtomicBool>>>>,
}

impl Drop for CallGuard {
    fn drop(&mut self) {
        if let Some(flag) = self.flag.lock().unwrap().as_ref() {
            flag.store(true, Ordering::SeqCst);
        }
    }
}
