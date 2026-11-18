//! 绑定一个游戏窗口后的共享底层（ADR 019 决定 1）：窗口身份、前台检查与等待、只截客户区
//! （纯色画面判独占全屏）、窗口比例 → 屏幕物理像素、按住的键 / 鼠标键记账与收尾松开。
//!
//! 自动化程序（`automation.rs` 的 `DesktopHost`）与一步一步操作（`computer_use.rs` 的
//! `game.act`）共用这一份，不各写一套。

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Instant;

use kotone_core::tools::{
    PhysicalRect, WindowCapturer, WindowIdentity, WindowInfo, WindowObserver,
};
use kotone_platform_windows::inject;
use kotone_platform_windows::window_tools::{GdiForegroundCapturer, WindowsWindowObserver};
use kotone_tools::image::RgbaImage;

/// 截图像素上限（4K）。
pub const MAX_CAPTURE_PIXELS: u64 = 3840 * 2160;

/// 窗口还在不在：最多每隔这么久真正查一次。
const ALIVE_INTERVAL_MS: u64 = 500;

pub struct GameSession {
    pub target: WindowIdentity,
    started: Instant,
    held_keys: Mutex<BTreeSet<String>>,
    held_buttons: Mutex<Vec<inject::MouseButton>>,
    /// 游戏窗口已关闭或身份变化。
    gone: AtomicBool,
    last_inspect: AtomicU64,
}

impl GameSession {
    pub fn new(target: WindowIdentity) -> Self {
        Self {
            target,
            started: Instant::now(),
            held_keys: Mutex::new(BTreeSet::new()),
            held_buttons: Mutex::new(Vec::new()),
            gone: AtomicBool::new(false),
            last_inspect: AtomicU64::new(0),
        }
    }

    pub fn elapsed_ms(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }

    /// 游戏窗口此刻在前台。
    pub fn focused_now(&self) -> bool {
        matches!(
            WindowsWindowObserver.foreground_window(),
            Ok(Some(w)) if w.identity.handle == self.target.handle && w.identity.pid == self.target.pid
        )
    }

    /// 节流地确认窗口还在；不在了返回 false（之后 [`is_gone`](Self::is_gone) 也为 true）。
    pub fn check_alive(&self) -> bool {
        if self.is_gone() {
            return false;
        }
        let now = self.elapsed_ms();
        if now.saturating_sub(self.last_inspect.load(Ordering::Relaxed)) < ALIVE_INTERVAL_MS {
            return true;
        }
        self.last_inspect.store(now, Ordering::Relaxed);
        if WindowsWindowObserver.inspect(&self.target).is_err() {
            self.gone.store(true, Ordering::SeqCst);
            return false;
        }
        true
    }

    pub fn is_gone(&self) -> bool {
        self.gone.load(Ordering::SeqCst)
    }

    /// 窗口当前信息（标题、客户区……）。
    pub fn inspect(&self) -> Result<WindowInfo, String> {
        WindowsWindowObserver
            .inspect(&self.target)
            .map_err(|e| format!("游戏窗口不可用：{e}"))
    }

    /// 客户区的屏幕物理像素位置（最小化时报错）。
    pub fn client_rect(&self) -> Result<PhysicalRect, String> {
        let rect = self
            .inspect()?
            .client_rect
            .ok_or("拿不到游戏窗口的客户区位置")?;
        if rect.width == 0 || rect.height == 0 {
            return Err("游戏窗口客户区为空（可能最小化了）".into());
        }
        Ok(rect)
    }

    /// 截游戏客户区（调用方先确认它在前台）。纯色画面按独占全屏报错。
    pub fn capture(&self) -> Result<RgbaImage, String> {
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
        RgbaImage::new(image.width, image.height, image.rgba).map_err(|e| e.to_string())
    }

    /// 窗口比例（0–1）→ 屏幕物理像素（按当前客户区位置）。
    pub fn screen_point(&self, x: f64, y: f64) -> Result<(i32, i32), String> {
        Ok(ratio_to_screen(&self.client_rect()?, x, y))
    }

    /// 按下一个键并记账（收尾时由 [`release_all`](Self::release_all) 松开）。
    pub fn key_down(&self, key: &str) -> Result<(), String> {
        inject::key_down(key).map_err(|e| e.message)?;
        self.held_keys.lock().unwrap().insert(key.to_string());
        Ok(())
    }

    pub fn key_up(&self, key: &str) -> Result<(), String> {
        self.held_keys.lock().unwrap().remove(key);
        inject::key_up(key).map_err(|e| e.message)
    }

    pub fn mouse_down(&self, button: inject::MouseButton) -> Result<(), String> {
        inject::mouse_button(button, true).map_err(|e| e.message)?;
        self.held_buttons.lock().unwrap().push(button);
        Ok(())
    }

    pub fn mouse_up(&self, button: inject::MouseButton) -> Result<(), String> {
        self.held_buttons.lock().unwrap().retain(|b| *b != button);
        inject::mouse_button(button, false).map_err(|e| e.message)
    }

    /// 松开所有按住的键与鼠标键。
    pub fn release_all(&self) {
        let held: Vec<String> = std::mem::take(&mut *self.held_keys.lock().unwrap())
            .into_iter()
            .collect();
        for key in held.iter().rev() {
            let _ = inject::key_up(key);
        }
        for button in std::mem::take(&mut *self.held_buttons.lock().unwrap()) {
            let _ = inject::mouse_button(button, false);
        }
    }
}

/// 窗口比例 → 客户区内的屏幕物理像素（不越出客户区）。
pub fn ratio_to_screen(rect: &PhysicalRect, x: f64, y: f64) -> (i32, i32) {
    let w = rect.width.max(1) as i32;
    let h = rect.height.max(1) as i32;
    let px = rect.x + ((x.clamp(0.0, 1.0) * f64::from(w)) as i32).min(w - 1);
    let py = rect.y + ((y.clamp(0.0, 1.0) * f64::from(h)) as i32).min(h - 1);
    (px, py)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ratio_maps_inside_the_client_area() {
        let rect = PhysicalRect {
            x: 100,
            y: 50,
            width: 800,
            height: 600,
        };
        assert_eq!(ratio_to_screen(&rect, 0.0, 0.0), (100, 50));
        assert_eq!(ratio_to_screen(&rect, 0.5, 0.5), (500, 350));
        assert_eq!(ratio_to_screen(&rect, 1.0, 1.0), (899, 649));
        assert_eq!(ratio_to_screen(&rect, -3.0, 9.0), (100, 649));
    }
}
