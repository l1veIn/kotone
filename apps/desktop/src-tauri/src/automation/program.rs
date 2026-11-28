use std::future::Future;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use kotone_agent::catalog::ToolEntry;
use kotone_core::automation::{program_failure, program_view, ProgramAction, Run};
use kotone_core::cancel::CancellationToken;
use kotone_core::input_lock::{InputGuard, InputUser};
use kotone_core::reminders::DeliveryKind;
use kotone_core::send_to_game::{ChannelPick, ChatRequest, SendToGame, Speaker};
use kotone_platform_windows::inject;
use kotone_script::{
    Cancel, KeyAction, MouseAction, MouseButton, Pixels, Region, ScriptHost, TextLine, ToolReply,
};
use tauri::AppHandle;

use super::{images_dir, Confirmed, ToolRules, MAX_IMAGE_SIDE};
use crate::game_session::GameSession;
use crate::ocr_engines::ocr_image;
use crate::tool_call::{self, By};

/// `say()` 最长多久（排队等她的输出队列也算在里面；以前播报循环的投递上限，ADR 045 D3）。超过就停下这句、记失败。
pub(super) const SAY_LIMIT: Duration = Duration::from_secs(240);
/// `say()` 被停下（或超时）以后，再等投递自己收尾（停语音、收弹幕）最多多久。
pub(super) const SAY_GRACE: Duration = Duration::from_secs(5);
/// 多久看一次脚本的取消标记（停下时把它桥接到投递的取消 token）。
const CANCEL_POLL: Duration = Duration::from_millis(50);

/// `tool()` 等一个工具最多多久（ADR 046 D1，脚本引擎打断不了阻塞中的宿主调用，由宿主照这个数等）。
pub(super) const TOOL_LIMIT: Duration = Duration::from_millis(kotone_script::TOOL_TIMEOUT_MS);

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
    /// 这次执行的程序：名字、来自哪个自动化、触发它的游戏事件（`ask()` 的护栏和附件用）。
    action: ProgramAction,
    /// 已经推送了几条（推送的 key 是 `<执行记录 id>-<第几条>`，每条都是新的）。
    pushes: AtomicU32,
    /// 这段程序里直接写着的工具名（`tools_called`，安排时查过；`tool()` 调的名字要在这上面）。
    tools: Vec<String>,
}

impl DesktopHost {
    pub(super) fn new(
        app: AppHandle,
        session: Option<GameSession>,
        cancel: Cancel,
        run: &Run,
    ) -> Self {
        let action = run.program().clone();
        Self {
            app,
            session,
            cancel,
            started: Instant::now(),
            run_id: run.id.clone(),
            tools: kotone_core::automation::library::tools_called(&action.source)
                .unwrap_or_default(),
            action,
            pushes: AtomicU32::new(0),
        }
    }

    /// 保存的自动化对这次跑的这份程序的确认（每次 `tool()` 现读：玩家可能刚允许；定义上的程序要是这期间被改了，
    /// 那上面的确认不算这份程序的）；临时程序是 None（只能调不用确认的）。
    fn confirmed_now(&self) -> Option<Confirmed> {
        let id = self.action.automation_id.as_deref()?;
        let current = super::library(&self.app).ok().and_then(|l| l.get(id));
        Some(
            current
                .filter(|a| a.action.source().trim() == self.action.source.trim())
                .map(|a| a.confirmed)
                .unwrap_or_default(),
        )
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

/// 等一件事（`tool()` 调的工具）的结果。
#[derive(Debug, PartialEq)]
pub(super) enum Waited<T> {
    Done(T),
    /// 超过上限还没回来。
    TimedOut,
    /// 程序被停下了。
    Stopped,
}

/// 最多等 `limit`；脚本被停下（`cancel`）就不等了（ADR 046 D1：停下就跟着停）。不等的那件事直接丢掉，不收尾：
/// 工具调用没有要停的语音、要收的弹幕。
pub(super) async fn wait_bounded<T>(
    cancel: &Cancel,
    limit: Duration,
    work: impl Future<Output = T>,
) -> Waited<T> {
    let stopped = async {
        while !cancel.is_cancelled() {
            tokio::time::sleep(CANCEL_POLL).await;
        }
    };
    tokio::select! {
        biased;
        _ = stopped => Waited::Stopped,
        result = tokio::time::timeout(limit, work) => match result {
            Ok(value) => Waited::Done(value),
            Err(_) => Waited::TimedOut,
        },
    }
}

/// 程序里一次 `tool()` 能不能调（ADR 046 D1、D3）：名字要在这段程序的单子上（安排时按源码列的；`_G["to" .. "ol"]` 这类
/// 绕过记号器的写法到这里就报错），再照这一刻的权限、插件开关判一次（规则可能刚改过）。能调交回这个工具的条目；
/// 调不了返回原因（程序出错结束，执行记录写原因）。`confirmed`：保存的自动化是它对这份程序的确认，临时程序 None。
pub(super) fn program_tool(
    called: &[String],
    rules: &ToolRules,
    name: &str,
    confirmed: Option<&Confirmed>,
) -> Result<ToolEntry, String> {
    if !called.iter().any(|c| c == name) {
        return Err(format!(
            "这段程序里没有直接写 tool(\"{name}\")，调不了：tool() 的第一个参数要直接写工具名"
        ));
    }
    rules
        .entry(name, confirmed)
        .cloned()
        .map_err(|state| state.refusal(name))
}

/// 工具回来以后交给程序的（ADR 046 D1、D4）：做成了给 [`program_view`]（不可信的只有 `{status}`）；工具自己说没成、超时
/// 是 `nil, 原因`（不可信的只有“没成”，完整原因交给 `log`）；停下了返回错误（脚本引擎看得到停下，记成已取消）。
pub(super) fn tool_reply(
    name: &str,
    untrusted: bool,
    waited: Waited<Result<tool_call::Reply, String>>,
    log: impl FnOnce(&str),
) -> Result<ToolReply, String> {
    let reason = match waited {
        Waited::Done(Ok(reply)) => {
            return Ok(ToolReply::Value(program_view(
                reply.for_program(),
                untrusted,
            )))
        }
        Waited::Done(Err(reason)) => reason,
        Waited::TimedOut => format!("{name} 超过 {} 秒没回来", TOOL_LIMIT.as_secs().max(1)),
        Waited::Stopped => return Err(format!("已停下，不等 {name} 了")),
    };
    log(&reason);
    Ok(ToolReply::Failed(program_failure(&reason, untrusted)))
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
        tauri::async_runtime::block_on(say_bounded(
            &self.cancel,
            SAY_LIMIT,
            SAY_GRACE,
            move |token| async move {
                crate::assistant::deliver_text_with(&app, text, kinds, token)
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

    /// 调一个工具（ADR 046 D1）：先照这一刻的规则判能不能调（[`program_tool`]），再经和热键同一份的路调（[`tool_call`]），
    /// 最多等 [`TOOL_LIMIT`]、停下就跟着停（[`wait_bounded`]），结果按“数据不是指令”挑好再交给程序（[`tool_reply`]）。
    fn tool(&self, name: &str, args: &serde_json::Value) -> Result<ToolReply, String> {
        let rules = ToolRules::now(&self.app);
        let entry = program_tool(&self.tools, &rules, name, self.confirmed_now().as_ref())?;
        let untrusted = entry.untrusted;
        let app = self.app.clone();
        let args = args.clone();
        let waited =
            tauri::async_runtime::block_on(wait_bounded(&self.cancel, TOOL_LIMIT, async move {
                tool_call::call(&app, &entry, args, By::Program).await
            }));
        tool_reply(name, untrusted, waited, |reason| {
            kotone_core::log::log(&format!(
                "automation {}: {name} 没成：{reason}",
                self.run_id
            ))
        })
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

    // ---------- ADR 046：程序里调工具 ----------

    use crate::automation::ToolState;
    use kotone_core::automation::library::{event_args, tools_called, validate_args};
    use kotone_core::games::{trigger_matches, Armed, GameEvent, Triggers};
    use kotone_core::settings::Settings;
    use kotone_script::{Limits, Outcome};
    use serde_json::{json, Value};
    use std::sync::Mutex;

    /// 测试清单 14：工具回来了就是回来了；超过上限不等了；停下就跟着停（不等到上限），已经停下了就不去调。
    #[tokio::test]
    async fn a_tool_call_waits_at_most_the_limit_and_stops_with_the_program() {
        let cancel = Cancel::default();
        assert_eq!(
            wait_bounded(&cancel, LIMIT, async { 7 }).await,
            Waited::Done(7)
        );
        let started = Instant::now();
        assert_eq!(
            wait_bounded(&cancel, LIMIT, std::future::pending::<()>()).await,
            Waited::TimedOut
        );
        assert!(started.elapsed() >= LIMIT);
        let stopper = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(60)).await;
            stopper.cancel();
        });
        let started = Instant::now();
        assert_eq!(
            wait_bounded(&cancel, LIMIT, std::future::pending::<()>()).await,
            Waited::Stopped
        );
        assert!(started.elapsed() < LIMIT, "停下就跟着停");
        let called = Arc::new(AtomicBool::new(false));
        let c = called.clone();
        let waited = wait_bounded(&cancel, LIMIT, async move {
            c.store(true, Ordering::SeqCst);
        })
        .await;
        assert_eq!(waited, Waited::Stopped);
        assert!(!called.load(Ordering::SeqCst), "已经停下了就不去调");
        assert_eq!(TOOL_LIMIT, Duration::from_secs(30));
    }

    /// 测试清单 13、14：做成了——可信的原样给（有结构化结果用它），不可信的只有 status；工具自己说没成、超时——
    /// `nil, 原因`（不可信的只有“没成”，完整原因进日志）；停下了——报错（脚本引擎记成已取消）。
    #[test]
    fn what_the_program_gets_back_from_a_tool() {
        let done = |content: Value, structured: Option<Value>| {
            Waited::Done(Ok(tool_call::Reply {
                content,
                structured,
            }))
        };
        let quiet = |_: &str| {};
        let this_match = tool_reply(
            "lol.match",
            false,
            done(
                json!("这局 12:30"),
                Some(json!({"status": "ok", "gameTime": 750})),
            ),
            quiet,
        );
        assert_eq!(
            this_match,
            Ok(ToolReply::Value(json!({"status": "ok", "gameTime": 750})))
        );
        let music = tool_reply(
            "music",
            true,
            done(
                json!({"status": "playing", "playing": "某首歌 - 某人"}),
                None,
            ),
            quiet,
        );
        assert_eq!(music, Ok(ToolReply::Value(json!({"status": "playing"}))));
        let mut logged = String::new();
        let failed = tool_reply(
            "music",
            true,
            Waited::Done(Err("歌单「某人起的名字」没有版权".into())),
            |reason| logged = reason.to_string(),
        );
        assert_eq!(failed, Ok(ToolReply::Failed("没成".into())));
        assert!(logged.contains("某人起的名字"), "完整原因只进日志");
        assert_eq!(
            tool_reply(
                "settings.get",
                false,
                Waited::Done(Err("读不到设置".into())),
                quiet
            ),
            Ok(ToolReply::Failed("读不到设置".into()))
        );
        let Ok(ToolReply::Failed(timeout)) =
            tool_reply("settings.get", false, Waited::TimedOut, quiet)
        else {
            panic!("超时是没成");
        };
        assert!(timeout.contains("30 秒"), "{timeout}");
        assert_eq!(
            tool_reply("music", true, Waited::TimedOut, quiet),
            Ok(ToolReply::Failed("没成".into()))
        );
        assert!(tool_reply("music", true, Waited::Stopped, |_: &str| panic!(
            "停下不算没成"
        ))
        .is_err());
    }

    /// 测试清单 8、12、13：名字不在这段程序的单子上调不了；屏幕上的字、读聊天、往游戏里打字、自动化自己的工具都调不了；
    /// 放歌能调；要确认的，保存的自动化确认过才能调，临时程序不能；只读模式下放歌被拒。
    #[test]
    fn which_tools_a_running_program_may_call() {
        let rules = ToolRules::of(Settings::default());
        let called: Vec<String> = [
            "music",
            "ocr.recognize",
            "companion.read_chat",
            "game_chat.send",
            "lol.shout",
            "automation.run",
            "settings.update",
        ]
        .map(String::from)
        .to_vec();
        let none = Confirmed::new();
        assert_eq!(
            program_tool(&called, &rules, "music", None).unwrap().name,
            "music"
        );
        assert!(program_tool(&called, &rules, "lol.match", None)
            .unwrap_err()
            .contains("直接写"));
        for name in [
            "ocr.recognize",
            "companion.read_chat",
            "game_chat.send",
            "lol.shout",
            "automation.run",
        ] {
            let error = program_tool(&called, &rules, name, Some(&none)).unwrap_err();
            assert!(error.contains(name), "{name}: {error}");
        }
        assert!(
            program_tool(&called, &rules, "settings.update", Some(&none))
                .unwrap_err()
                .contains("「自动化」页")
        );
        let confirmed = rules.to_confirm("tool('settings.update', { patch = {} })");
        assert!(program_tool(&called, &rules, "settings.update", Some(&confirmed)).is_ok());
        assert!(program_tool(&called, &rules, "settings.update", None)
            .unwrap_err()
            .contains("临时程序"));
        let mut settings = Settings::default();
        settings.agent.permissions.mode = kotone_core::agent::Mode::ReadOnly;
        assert!(
            program_tool(&called, &ToolRules::of(settings), "music", Some(&none))
                .unwrap_err()
                .contains("只读模式")
        );
    }

    /// 假宿主：`tool()` 走和真宿主同一个判断和整理（[`program_tool`]、[`tool_reply`]），工具本身是假的——记下调了什么，
    /// 回 `reply`（当成插件给她的那份内容）。说话、弹幕、推送记下来；没绑游戏窗口。
    struct FakeHost {
        rules: ToolRules,
        called: Vec<String>,
        confirmed: Option<Confirmed>,
        reply: Value,
        calls: Mutex<Vec<(String, Value)>>,
        shown: Mutex<Vec<String>>,
        started: Instant,
    }

    impl FakeHost {
        fn new(source: &str, confirmed: Option<Confirmed>, reply: Value) -> Self {
            Self {
                rules: ToolRules::of(Settings::default()),
                called: tools_called(source).unwrap(),
                confirmed,
                reply,
                calls: Mutex::default(),
                shown: Mutex::default(),
                started: Instant::now(),
            }
        }

        fn show(&self, text: &str) -> Result<(), String> {
            self.shown.lock().unwrap().push(text.to_string());
            Ok(())
        }
    }

    impl ScriptHost for FakeHost {
        fn now_ms(&self) -> u64 {
            self.started.elapsed().as_millis() as u64
        }
        fn sleep(&self, ms: u64, cancel: &Cancel) -> bool {
            std::thread::sleep(Duration::from_millis(ms));
            !cancel.is_cancelled()
        }
        fn game_focused(&self) -> bool {
            false
        }
        fn key(&self, _: KeyAction, _: &str) -> Result<(), String> {
            Err(NO_WINDOW.into())
        }
        fn type_text(&self, _: &str) -> Result<(), String> {
            Err(NO_WINDOW.into())
        }
        fn chat(&self, _: &str, _: Option<&str>) -> Result<(), String> {
            Err(NO_WINDOW.into())
        }
        fn brightness(&self, _: Region) -> Result<f64, String> {
            Err(NO_WINDOW.into())
        }
        fn color_at(&self, _: f64, _: f64) -> Result<[u8; 3], String> {
            Err(NO_WINDOW.into())
        }
        fn pixels(&self, _: Region) -> Result<Pixels, String> {
            Err(NO_WINDOW.into())
        }
        fn ocr(&self, _: Region) -> Result<Vec<TextLine>, String> {
            Err(NO_WINDOW.into())
        }
        fn image(&self, _: &str) -> Result<Pixels, String> {
            Err(NO_WINDOW.into())
        }
        fn mouse(&self, _: MouseAction) -> Result<(), String> {
            Err(NO_WINDOW.into())
        }
        fn say(&self, text: &str, _: bool) -> Result<(), String> {
            self.show(text)
        }
        fn toast(&self, text: &str) -> Result<(), String> {
            self.show(text)
        }
        fn push(&self, text: &str) -> Result<(), String> {
            self.show(text)
        }
        fn ask(&self, _: &str) -> Result<bool, String> {
            Ok(false)
        }
        fn require_window(&self) -> Result<(), String> {
            Err(NO_WINDOW.into())
        }
        fn tool(&self, name: &str, args: &Value) -> Result<ToolReply, String> {
            let entry = program_tool(&self.called, &self.rules, name, self.confirmed.as_ref())?;
            self.calls
                .lock()
                .unwrap()
                .push((name.to_string(), args.clone()));
            let reply = tool_call::Reply {
                content: self.reply.clone(),
                structured: None,
            };
            tool_reply(name, entry.untrusted, Waited::Done(Ok(reply)), |_| {})
        }
    }

    fn run_on(
        source: &str,
        args: &serde_json::Map<String, Value>,
        host: FakeHost,
    ) -> (kotone_script::RunReport, Arc<FakeHost>) {
        let host = Arc::new(host);
        let report = kotone_script::run_with_args(
            source,
            args,
            host.clone() as Arc<dyn ScriptHost>,
            Limits::default(),
            Cancel::default(),
        );
        (report, host)
    }

    /// 验收场景（ADR 046 末尾）：“队友都倒了放首歌”。本体的代码里不写插件登记的事件名，英雄联盟的“有英雄阵亡”换成
    /// 假想的 demo 游戏的 `demo.death`（字段照英雄联盟插件登记的，见 `automation::tests::saved`）。存得进来；不用游戏窗口、
    /// 不会叫她；`music` 默认允许、不用确认。最后一个队友倒下（我还活着）那一件触发它，对面死了、还剩两个队友、我死了
    /// 都不触发，60 秒里不再触发。程序调到 `music`（`action = "play"`，`query` 是参数的缺省值），拿到的只有 `{status}`：
    /// 没登录时弹一句，放上了什么都不说。
    #[test]
    fn acceptance_play_a_song_when_the_last_ally_falls() {
        let (_dir, _library, automation) = crate::automation::tests::saved(json!({
            "name": "队友都倒了放首歌",
            "params": [{"name": "song", "label": "放什么", "type": "text", "maxLength": 100, "default": "Legends Never Die"}],
            "action": {"kind": "program", "source": "local r = tool(\"music\", { action = \"play\", query = args.song })\nif not r then\n  log(\"没放成\")\nelseif r.status == \"need_login\" then\n  toast(\"网易云音乐还没登录，没放成\")\nend"},
            "on": {"event": ["demo.death"], "game": "demo", "match": {"team": "ally", "meAlive": true, "alliesAlive": 1}, "cooldownSeconds": 60}
        }));
        let source = automation.action.source();
        let uses = automation.action.uses();
        assert!(!uses.window && !uses.asks, "不用游戏窗口、不会叫她");
        let rules = ToolRules::of(Settings::default());
        let lines = rules.lines(&automation);
        assert_eq!(
            lines
                .iter()
                .map(|l| (l.name.as_str(), l.state.clone()))
                .collect::<Vec<_>>(),
            [("music", ToolState::Ready)],
            "放歌默认允许，不用确认"
        );
        assert!(rules.check(source, Some(&automation.confirmed)).is_ok());

        let death = |data: Value| GameEvent {
            id: "e-1".into(),
            kind: "demo.death".into(),
            game: "demo".into(),
            match_id: "m-1".into(),
            time: 0,
            game_time: Some(900.0),
            source: "demo/live".into(),
            data: data.as_object().unwrap().clone(),
        };
        let fields = |team: &str, me: bool, allies: u32, me_alive: bool| {
            json!({"team": team, "champion": "Ahri", "me": me, "killer": "Zed", "respawnSeconds": 30,
                   "alliesAlive": allies, "enemiesAlive": 4, "meAlive": me_alive})
        };
        let trigger = automation.on.as_ref().unwrap();
        for (data, why) in [
            (fields("enemy", false, 1, true), "对面死了"),
            (fields("ally", false, 2, true), "还剩两个队友"),
            (fields("ally", true, 0, false), "我死了"),
        ] {
            assert!(!trigger_matches(trigger, &death(data)), "{why}");
        }
        let last_ally = death(fields("ally", false, 1, true));
        let armed = [Armed {
            id: &automation.id,
            trigger,
        }];
        let mut triggers = Triggers::default();
        let fires = triggers.on_event(&armed, &last_ally, 1_000);
        assert_eq!(fires.len(), 1);
        assert!(
            triggers.on_event(&armed, &last_ally, 30_000).is_empty(),
            "60 秒冷却"
        );

        let args = validate_args(
            &automation.params,
            &event_args(&automation.params, &fires[0].event),
        )
        .unwrap();
        let confirmed = Some(automation.confirmed.clone());
        let (report, host) = run_on(
            source,
            &args,
            FakeHost::new(
                source,
                confirmed.clone(),
                json!({"status": "need_login", "text": "要先登录网易云音乐"}),
            ),
        );
        assert_eq!(report.outcome, Outcome::Finished, "{report:?}");
        assert_eq!(
            *host.calls.lock().unwrap(),
            [(
                "music".to_string(),
                json!({"action": "play", "query": "Legends Never Die"})
            )]
        );
        assert_eq!(*host.shown.lock().unwrap(), ["网易云音乐还没登录，没放成"]);

        let (report, host) = run_on(
            source,
            &args,
            FakeHost::new(
                source,
                confirmed,
                json!({"status": "playing", "playing": "Legends Never Die - Against The Current"}),
            ),
        );
        assert_eq!(report.outcome, Outcome::Finished, "{report:?}");
        assert!(host.shown.lock().unwrap().is_empty(), "放上了就安静");
    }

    /// 测试清单 13、D4：不可信的工具程序只拿得到 status（歌名拿不到）；绕过记号器拼出来的名字（`_G["to" .. "ol"]`）
    /// 安排时看不出来，一调就让程序出错结束。
    #[test]
    fn programs_see_only_the_status_and_cannot_sneak_in_a_tool_name() {
        let source = "local r = tool('music', { action = 'now' })\nlog(tostring(r.status))\nlog(tostring(r.playing))";
        let (report, _) = run_on(
            source,
            &Default::default(),
            FakeHost::new(
                source,
                None,
                json!({"status": "playing", "playing": "某首歌 - 某人"}),
            ),
        );
        assert_eq!(report.outcome, Outcome::Finished, "{report:?}");
        assert_eq!(report.log, ["playing", "nil"]);

        let sneaky = "local f = _G['to' .. 'ol']\nf('music', { action = 'play' })";
        assert_eq!(tools_called(sneaky).unwrap(), Vec::<String>::new());
        let (report, host) = run_on(
            sneaky,
            &Default::default(),
            FakeHost::new(sneaky, None, json!({"status": "playing"})),
        );
        assert!(
            matches!(&report.outcome, Outcome::Error(e) if e.contains("直接写")),
            "{report:?}"
        );
        assert!(host.calls.lock().unwrap().is_empty());
    }
}
