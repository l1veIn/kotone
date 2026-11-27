use std::future::Future;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use kotone_core::assistant::is_legacy_reminder_session;
use kotone_core::automation::{ProgramAction, Run};
use kotone_core::cancel::CancellationToken;
use kotone_core::input_lock::{InputGuard, InputUser};
use kotone_core::reminders::DeliveryKind;
use kotone_core::send_to_game::{ChannelPick, ChatRequest, SendToGame, Speaker};
use kotone_platform_windows::inject;
use kotone_script::{
    Cancel, KeyAction, MouseAction, MouseButton, Pixels, Region, ScriptHost, TextLine,
};
use tauri::AppHandle;

use super::{images_dir, MAX_IMAGE_SIDE};
use crate::game_session::GameSession;
use crate::ocr_engines::ocr_image;

/// `say()` 最长多久（排队等她的输出队列也算在里面；以前播报循环的投递上限，ADR 045 D3）。超过就停下这句、记失败。
pub(super) const SAY_LIMIT: Duration = Duration::from_secs(240);
/// `say()` 被停下（或超时）以后，再等投递自己收尾（停语音、收弹幕）最多多久。
pub(super) const SAY_GRACE: Duration = Duration::from_secs(5);
/// 多久看一次脚本的取消标记（停下时把它桥接到投递的取消 token）。
const CANCEL_POLL: Duration = Duration::from_millis(50);

/// 没绑游戏窗口时窗口函数的报错（ADR 045 D4 兜底：不进“游戏不在前台就暂停”的等待）。
const NO_WINDOW: &str = "这次运行没有绑定游戏窗口（安排时没看出程序要操作游戏）：\
    按键、打字、鼠标、看屏幕、发到游戏聊天都用不了";

/// 真实宿主：窗口绑定、截图、坐标与按住的键都在共享的 [`GameSession`] 里（ADR 019 决定 1）。
/// 没用到游戏窗口的执行（ADR 045 D4）没有会话：窗口函数一调用就报错（[`NO_WINDOW`]）。
pub(super) struct DesktopHost {
    app: AppHandle,
    /// 绑定的游戏窗口；None = 程序里没用到游戏窗口，不看前台、不绑窗口。
    session: Option<GameSession>,
    cancel: Cancel,
    started: Instant,
    /// 执行记录 id（推送的 key 用它）。
    run_id: String,
    /// 0.1 升级上来的提醒（执行记录的会话是 `legacy-reminder-…`）：用旧语音引擎说。
    legacy_voice: bool,
    /// 这次执行的程序：名字、来自哪个自动化、触发它的游戏事件（`ask()` 的护栏和附件用）。
    action: ProgramAction,
    /// 已经推送了几条（推送的 key 是 `<执行记录 id>-<第几条>`，每条都是新的）。
    pushes: AtomicU32,
}

impl DesktopHost {
    pub(super) fn new(
        app: AppHandle,
        session: Option<GameSession>,
        cancel: Cancel,
        run: &Run,
    ) -> Self {
        Self {
            app,
            session,
            cancel,
            started: Instant::now(),
            run_id: run.id.clone(),
            legacy_voice: is_legacy_reminder_session(&run.session_id),
            action: run.program().clone(),
            pushes: AtomicU32::new(0),
        }
    }

    /// 绑定的游戏窗口；没绑时报 [`NO_WINDOW`]。
    fn window(&self) -> Result<&GameSession, String> {
        self.session.as_ref().ok_or_else(|| NO_WINDOW.to_string())
    }

    /// 程序结束时松开按住的键和鼠标键（没绑窗口时什么都没按着）。
    pub(super) fn release_all(&self) {
        if let Some(session) = &self.session {
            session.release_all();
        }
    }

    /// 绑定的游戏窗口关掉了（程序因此被取消）。
    pub(super) fn window_gone(&self) -> bool {
        self.session.as_ref().is_some_and(GameSession::is_gone)
    }

    fn focused_now(&self) -> bool {
        self.session.as_ref().is_some_and(GameSession::focused_now)
    }

    /// 节流地检查游戏窗口是否还在；不在就取消脚本。没绑窗口的不查（窗口关了不影响它）。
    fn check_alive(&self) {
        if self.session.as_ref().is_some_and(|s| !s.check_alive()) {
            self.cancel.cancel();
        }
    }

    /// 截游戏客户区；游戏不在前台时先等它回来（与按键一样暂停）。
    fn capture(&self) -> Result<kotone_tools::image::RgbaImage, String> {
        let session = self.window()?;
        while !session.focused_now() {
            if !self.sleep(200, &self.cancel.clone()) {
                return Err("自动化已停止".into());
            }
        }
        session.capture()
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

    /// 一把输入锁（ADR 041 D4）：每一下按键、打字、点击各拿一次，不和发到游戏的字、`game.act` 交错。
    /// 松开（`key_up` / `mouse_up` / 结束时全部松开）不拿锁，永远不会因为排队卡住一个按着的键。
    fn input(&self) -> Result<InputGuard, String> {
        crate::game_chat::input_lock(&self.app)
            .acquire_for(InputUser::Program)
            .map_err(|busy| busy.to_string())
    }

    fn screen_point(&self, x: f64, y: f64) -> Result<(i32, i32), String> {
        self.window()?.screen_point(x, y)
    }
}

/// 一句 `say()`（ADR 045 D3，以前播报循环保证的那几条）：投递拿到一个取消 token；脚本被停下（`cancel`）时
/// 桥接过去，再等投递自己收尾（停语音、收弹幕）最多 `grace`；超过 `limit` 也停下，报错（记失败）。
/// 停下时返回的错误由脚本引擎记成“已取消”（它看得到 `cancel`）。
pub(super) async fn say_bounded<F>(
    cancel: &Cancel,
    limit: Duration,
    grace: Duration,
    deliver: impl FnOnce(CancellationToken) -> F,
) -> Result<(), String>
where
    F: Future<Output = Result<(), String>>,
{
    let token = CancellationToken::default();
    let delivery = deliver(token.clone());
    tokio::pin!(delivery);
    let stopped = async {
        while !cancel.is_cancelled() {
            tokio::time::sleep(CANCEL_POLL).await;
        }
    };
    let interrupted = tokio::select! {
        biased;
        _ = stopped => "已停下，这句没有说完".to_string(),
        result = tokio::time::timeout(limit, &mut delivery) => match result {
            Ok(result) => return result,
            Err(_) => format!("说一句超过 {} 秒还没说完，停了", limit.as_secs()),
        },
    };
    token.cancel();
    // 不在收到取消的同一轮直接丢弃投递：它要继续跑才能停掉已经发出的语音、收起弹幕。收尾有上限，
    // 投递自己另有 Drop 守卫兜底。
    let _ = tokio::time::timeout(grace, &mut delivery).await;
    Err(interrupted)
}

fn platform_button(button: MouseButton) -> inject::MouseButton {
    match button {
        MouseButton::Left => inject::MouseButton::Left,
        MouseButton::Right => inject::MouseButton::Right,
        MouseButton::Middle => inject::MouseButton::Middle,
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
        let session = self.window()?;
        match action {
            KeyAction::Press => {
                let _input = self.input()?;
                inject::key_down_up(key).map_err(|e| e.message)
            }
            KeyAction::Down => {
                let _input = self.input()?;
                session.key_down(key)
            }
            KeyAction::Up => session.key_up(key),
        }
    }

    fn type_text(&self, text: &str) -> Result<(), String> {
        self.window()?;
        let _input = self.input()?;
        inject::send_unicode(text).map_err(|e| e.message)
    }

    /// 经本体的“发到游戏”入口（ADR 041 D1）：游戏配置的聊天键和频道、等修饰键、排队都在入口里。
    fn chat(&self, text: &str, channel: Option<&str>) -> Result<(), String> {
        self.window()?;
        let mut request = ChatRequest::new(text, Speaker::Automation);
        request.channel = channel.map(|id| ChannelPick {
            id: id.to_string(),
            for_game: None,
        });
        let outcome = crate::game_chat::entry(&self.app)
            .send(request)
            .map_err(|failure| failure.to_string())?;
        match outcome.stopped {
            None => Ok(()),
            Some(stopped) => Err(format!(
                "发了 {} 行，后面 {} 行没发：{stopped}",
                outcome.sent_lines,
                outcome.lines.len() - outcome.sent_lines
            )),
        }
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
        let session = self.window()?;
        let _input = match action {
            MouseAction::Up(_) => None,
            _ => Some(self.input()?),
        };
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
            MouseAction::Down(button) => session.mouse_down(platform_button(button)),
            MouseAction::Up(button) => session.mouse_up(platform_button(button)),
            MouseAction::Scroll(clicks) => inject::mouse_wheel(clicks).map_err(err),
        }
    }

    fn ocr(&self, region: Region) -> Result<Vec<TextLine>, String> {
        let (crop, origin) = Self::crop_with_origin(&self.capture()?, region);
        // 识别框是裁剪图里的比例，换回窗口比例。
        Ok(ocr_image(&self.app, &crop)?
            .into_iter()
            .map(|(text, rect)| TextLine {
                rect: rect.map(|[x, y, w, h]| Region {
                    x: origin.x + x * origin.width,
                    y: origin.y + y * origin.height,
                    width: w * origin.width,
                    height: h * origin.height,
                }),
                text,
            })
            .collect())
    }

    /// 她开口说（ADR 045 D2、D3）：排进她的输出队列（一次只说一条），麦克风开着先等、语音失败改弹幕（在投递里）；
    /// `toast = true` 时一次投递语音 + 弹幕（同时出现）。最多 [`SAY_LIMIT`]；停下就跟着停，等投递收尾最多
    /// [`SAY_GRACE`]。0.1 的提醒用旧语音引擎。
    fn say(&self, text: &str, toast: bool) -> Result<(), String> {
        let kinds = if toast {
            vec![DeliveryKind::Voice, DeliveryKind::Danmaku]
        } else {
            vec![DeliveryKind::Voice]
        };
        let app = self.app.clone();
        let text = text.to_string();
        let legacy_voice = self.legacy_voice;
        tauri::async_runtime::block_on(say_bounded(
            &self.cancel,
            SAY_LIMIT,
            SAY_GRACE,
            move |token| async move {
                crate::assistant::deliver_text_with(&app, text, kinds, token, legacy_voice)
                    .await
                    .map(|_| ())
            },
        ))
    }

    /// 弹幕，不阻塞（排在她的输出队列里）。弹不出来（例如独占全屏）只记日志，不算程序出错（ADR 045“代价”）。
    fn toast(&self, text: &str) -> Result<(), String> {
        let app = self.app.clone();
        let text = text.to_string();
        let run_id = self.run_id.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(e) =
                crate::assistant::deliver_text(&app, text, vec![DeliveryKind::Danmaku]).await
            {
                kotone_core::log::log(&format!("automation {run_id}: 弹幕没能显示：{e}"));
            }
        });
        Ok(())
    }

    /// 推送（ADR 045 D2）：和插件推送同一条路，Kotone 聊天里记一条、悬浮窗显示一行，不调用模型、不出声；
    /// 附一条不叫模型的说明，她之后在对话里看得到。每条的 key 都是新的（`<执行记录 id>-<第几条>`）。
    fn push(&self, text: &str) -> Result<(), String> {
        let n = self.pushes.fetch_add(1, Ordering::SeqCst) + 1;
        crate::agent::post_automation_push(
            &self.app,
            self.action.automation_id.as_deref(),
            &self.action.name,
            &format!("{}-{n}", self.run_id),
            text,
        )
    }

    /// 交给她（ADR 045 D2，以前的“让她做”）：先过 core 的两道护栏（只在保存的自动化里能用、话里不能带从游戏
    /// 事件填进来的值）；按游戏事件触发的每个自动化每局最多叫她 3 次（超了不交、返回 false）；然后等她空下来
    /// （最多 2 分钟，期间停下就什么都不发）交给主会话，附上触发它的事件。
    fn ask(&self, text: &str) -> Result<bool, String> {
        self.action.check_ask(text)?;
        let automation = self.action.automation_id.as_deref().unwrap_or_default();
        if let Some(event) = &self.action.event {
            if !crate::games::take_ask(automation, &event.game) {
                kotone_core::log::log(&format!(
                    "automation {automation}: 这局已经叫过她 {} 次，这次不交给她",
                    kotone_core::games::ASKS_PER_MATCH
                ));
                return Ok(false);
            }
        }
        let cancel = self.cancel.clone();
        tauri::async_runtime::block_on(crate::agent::ask_from_automation(
            &self.app,
            &self.action.name,
            text,
            self.action.event.as_ref(),
            move || cancel.is_cancelled(),
        ))
    }

    /// 窗口懒绑定（ADR 045 D4）：没绑游戏窗口的执行，窗口函数一调用就报错，不进暂停等待。
    fn require_window(&self) -> Result<(), String> {
        self.window().map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    #[test]
    fn luma_of_black_white_and_red() {
        assert_eq!(mean_luma(&[0, 0, 0, 255, 0, 0, 0, 255]), 0.0);
        assert!((mean_luma(&[255, 255, 255, 255]) - 255.0).abs() < 1e-9);
        assert!((mean_luma(&[255, 0, 0, 255]) - 76.245).abs() < 1e-6);
        assert_eq!(mean_luma(&[]), 0.0);
    }

    const LIMIT: Duration = Duration::from_millis(400);
    const GRACE: Duration = Duration::from_millis(300);

    /// 假的投递：说 `speak` 这么久；取消 token 以后过 `cleanup` 收尾（停语音、收弹幕），记下收没收尾。
    async fn fake_delivery(
        token: CancellationToken,
        speak: Duration,
        cleanup: Duration,
        cleaned: Arc<AtomicBool>,
    ) -> Result<(), String> {
        tokio::select! {
            _ = token.cancelled() => {
                tokio::time::sleep(cleanup).await;
                cleaned.store(true, Ordering::SeqCst);
                Err("提醒已取消".into())
            }
            _ = tokio::time::sleep(speak) => Ok(()),
        }
    }

    /// 说完了就是说完了；投递自己的失败原样交回（语音失败改弹幕也失败时，记失败）。
    #[tokio::test]
    async fn say_returns_what_the_delivery_returns() {
        let cancel = Cancel::default();
        let cleaned = Arc::new(AtomicBool::new(false));
        let c = cleaned.clone();
        let done = say_bounded(&cancel, LIMIT, GRACE, |token| {
            fake_delivery(token, Duration::from_millis(20), Duration::ZERO, c)
        })
        .await;
        assert_eq!(done, Ok(()));
        assert!(!cleaned.load(Ordering::SeqCst));
        let failed = say_bounded(&cancel, LIMIT, GRACE, |_| async {
            Err::<(), String>("弹幕窗口不可用".into())
        })
        .await;
        assert_eq!(failed.unwrap_err(), "弹幕窗口不可用");
    }

    /// 测试清单 13：说到一半被停下，取消桥接到投递，等它收尾（语音停、弹幕收）再返回；返回错误（脚本引擎看得到
    /// 已取消，记成 Cancelled）。收尾很慢也只等 `grace`。
    #[tokio::test]
    async fn stopping_a_say_stops_the_delivery_and_waits_for_it_to_clean_up() {
        let cancel = Cancel::default();
        let cleaned = Arc::new(AtomicBool::new(false));
        let stopper = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(60)).await;
            stopper.cancel();
        });
        let started = Instant::now();
        let c = cleaned.clone();
        let result = say_bounded(&cancel, LIMIT, GRACE, |token| {
            fake_delivery(token, Duration::from_secs(30), Duration::from_millis(40), c)
        })
        .await;
        assert!(result.unwrap_err().contains("停下"));
        assert!(cleaned.load(Ordering::SeqCst), "等投递收尾了");
        assert!(started.elapsed() < LIMIT, "停下就跟着停，不等到上限");

        // 收尾一直不结束：最多等 grace
        let cancel = Cancel::default();
        cancel.cancel();
        let started = Instant::now();
        let result = say_bounded(&cancel, LIMIT, GRACE, |_| {
            std::future::pending::<Result<(), String>>()
        })
        .await;
        assert!(result.is_err());
        assert!(started.elapsed() >= GRACE && started.elapsed() < GRACE + LIMIT);
    }

    /// 测试清单 13：一句话说太久（排队、麦克风一直开着、语音卡住）到上限就停下这句（投递也收尾），记失败。
    #[tokio::test]
    async fn a_say_that_takes_too_long_is_stopped_and_fails() {
        let cancel = Cancel::default();
        let cleaned = Arc::new(AtomicBool::new(false));
        let c = cleaned.clone();
        let started = Instant::now();
        let result = say_bounded(&cancel, LIMIT, GRACE, |token| {
            fake_delivery(token, Duration::from_secs(30), Duration::ZERO, c)
        })
        .await;
        let error = result.unwrap_err();
        assert!(error.contains("还没说完"), "{error}");
        assert!(cleaned.load(Ordering::SeqCst), "超时也让投递收尾");
        assert!(!cancel.is_cancelled(), "不是停下：脚本引擎记成失败");
        assert!(started.elapsed() >= LIMIT);
        assert_eq!(SAY_LIMIT, Duration::from_secs(240));
        assert_eq!(SAY_GRACE, Duration::from_secs(5));
    }
}
