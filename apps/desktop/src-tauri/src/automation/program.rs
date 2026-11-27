use std::time::{Duration, Instant};

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

/// 真实宿主：窗口绑定、截图、坐标与按住的键都在共享的 [`GameSession`] 里（ADR 019 决定 1）。
pub(super) struct DesktopHost {
    pub(super) app: AppHandle,
    pub(super) session: GameSession,
    pub(super) cancel: Cancel,
}

impl DesktopHost {
    fn focused_now(&self) -> bool {
        self.session.focused_now()
    }

    /// 节流地检查游戏窗口是否还在；不在就取消脚本。
    fn check_alive(&self) {
        if !self.session.check_alive() {
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
        self.session.capture()
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
        self.session.screen_point(x, y)
    }

    fn deliver(&self, text: &str, kinds: Vec<DeliveryKind>, wait: bool) -> Result<(), String> {
        let app = self.app.clone();
        let text = text.to_string();
        let task = async move {
            crate::assistant::deliver_text(&app, text, kinds)
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
        self.session.elapsed_ms()
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
            KeyAction::Press => {
                let _input = self.input()?;
                inject::key_down_up(key).map_err(|e| e.message)
            }
            KeyAction::Down => {
                let _input = self.input()?;
                self.session.key_down(key)
            }
            KeyAction::Up => self.session.key_up(key),
        }
    }

    fn type_text(&self, text: &str) -> Result<(), String> {
        let _input = self.input()?;
        inject::send_unicode(text).map_err(|e| e.message)
    }

    /// 经本体的“发到游戏”入口（ADR 041 D1）：游戏配置的聊天键和频道、等修饰键、排队都在入口里。
    fn chat(&self, text: &str, channel: Option<&str>) -> Result<(), String> {
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
            MouseAction::Down(button) => self.session.mouse_down(platform_button(button)),
            MouseAction::Up(button) => self.session.mouse_up(platform_button(button)),
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

    // TODO(ADR 045 B): `say()` 外面包 240 秒超时；停下时桥接到投递的取消 token、等壳收尾（最多 5 秒）；
    // 0.1 的提醒用旧语音引擎（legacy_voice）。
    fn say(&self, text: &str, toast: bool) -> Result<(), String> {
        let kinds = if toast {
            vec![DeliveryKind::Voice, DeliveryKind::Danmaku]
        } else {
            vec![DeliveryKind::Voice]
        };
        self.deliver(text, kinds, true)
    }

    fn toast(&self, text: &str) -> Result<(), String> {
        self.deliver(text, vec![DeliveryKind::Danmaku], false)
    }

    // TODO(ADR 045 B): 接插件推送同一条路（`post_automation_push`），附一条不叫模型的说明。
    fn push(&self, _text: &str) -> Result<(), String> {
        Err("push() 还没接上".into())
    }

    // TODO(ADR 045 B): 先 `ProgramAction::check_ask`（临时程序、事件填进来的值），按事件触发的再
    // `Triggers::take_ask`，然后等她空下来（最多 2 分钟）交给主会话、附上触发它的事件。
    fn ask(&self, _text: &str) -> Result<bool, String> {
        Err("ask() 还没接上".into())
    }

    // TODO(ADR 045 B): 窗口懒绑定：没绑游戏窗口的执行（`target = None`）这里返回 Err。现在所有执行都绑了窗口，
    // 用默认实现（Ok）。
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
