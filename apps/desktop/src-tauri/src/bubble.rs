//! 琴音的对话气泡（屏幕角落，默认右下角）：语音入口（唤醒、陪玩热键、追问）的状态与回答。
//!
//! 参考成熟桌宠（dsh-pet、VPet、AIRI）的做法：
//! - 只有一个气泡位，状态依次是 在听 → 听清了 → 在想 / 在用工具 → 在说 / 等你回答 → 收起；
//! - 念出来的话：说的时候挂住，说完再按字数停留几秒才收（迟到的“说完了”按序号丢弃）；
//! - 窗口透明、置顶、不抢焦点，鼠标整窗穿透（右下角常是游戏小地图，不能挡操作）；
//! - 独占全屏时不显示，只靠语音。
//!
//! 线程约定（吸取悬浮窗死锁的教训）：状态只在短临界区里改；窗口的显示、隐藏、摆位一律经
//! `run_on_main_thread` 在主线程按顺序执行，执行时读最新状态；不在持锁时等主线程，也不在
//! 窗口回调里发事件。

use std::sync::Mutex;
use std::time::Duration;

use kotone_core::assistant::{BubbleConfig, BubbleCorner};
use kotone_core::log;
use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager};

use crate::SharedState;

pub const EVENT: &str = "kotone://bubble";
const LABEL: &str = "companion";
/// 窗口的逻辑尺寸（气泡贴着靠角的一边，从角落往外长）。
const WINDOW_SIZE: (f64, f64) = (440.0, 260.0);
/// 离屏幕边缘的基础距离（逻辑像素）。
const MARGIN: f64 = 24.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    Hidden,
    /// 在听玩家说话。
    Listening,
    /// 说完了，正在识别。
    Recognizing,
    /// 在想 / 在用工具。
    Thinking,
    /// 在说（念出来的回答）。
    Speaking,
    /// 问了玩家一句，在等回答。
    Asking,
    /// 一句简短的收尾（例如“没听到，先不打扰啦”）。
    Note,
    Error,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub revision: u64,
    pub phase: Phase,
    /// 状态行（如“在想…”“正在看屏幕”）。
    pub label: String,
    /// 正文：琴音说的话、问的问题或出错原因；在想时是玩家刚说的话。
    pub text: String,
    /// 正在听到的话（实时识别）。
    pub heard: String,
    /// 追问：等回答的秒数与开始等的时间（毫秒）。
    pub ask_seconds: Option<u64>,
    pub ask_started_ms: Option<u64>,
    pub corner: BubbleCorner,
}

/// 纯状态（不碰窗口，便于测试）。
#[derive(Debug, Clone)]
pub struct Model {
    snap: Snapshot,
    /// 正在念：在想 / 工具状态不覆盖正在念的句子。
    holding: bool,
    /// 琴音的名字（空 = 默认）。
    name: String,
}

impl Default for Model {
    fn default() -> Self {
        Self {
            snap: Snapshot {
                revision: 0,
                phase: Phase::Hidden,
                label: String::new(),
                text: String::new(),
                heard: String::new(),
                ask_seconds: None,
                ask_started_ms: None,
                corner: BubbleCorner::default(),
            },
            holding: false,
            name: String::new(),
        }
    }
}

impl Model {
    pub fn snapshot(&self) -> &Snapshot {
        &self.snap
    }

    fn name(&self) -> String {
        if self.name.is_empty() {
            kotone_agent::workspace::DEFAULT_NAME.to_string()
        } else {
            self.name.clone()
        }
    }

    fn set(&mut self, phase: Phase, label: &str, text: &str) -> &Snapshot {
        self.snap.revision += 1;
        self.snap.phase = phase;
        self.snap.label = label.to_string();
        self.snap.text = text.to_string();
        if !matches!(phase, Phase::Asking) {
            self.snap.ask_seconds = None;
            self.snap.ask_started_ms = None;
        }
        &self.snap
    }

    pub fn listening(&mut self) -> &Snapshot {
        if self.snap.phase == Phase::Asking {
            return &self.snap;
        }
        self.holding = false;
        self.snap.heard.clear();
        self.set(Phase::Listening, "在听…", "")
    }

    pub fn heard(&mut self, text: &str) -> Option<&Snapshot> {
        if !matches!(
            self.snap.phase,
            Phase::Listening | Phase::Recognizing | Phase::Asking
        ) {
            return None;
        }
        self.snap.revision += 1;
        self.snap.heard = text.to_string();
        Some(&self.snap)
    }

    pub fn recognizing(&mut self) -> Option<&Snapshot> {
        (self.snap.phase == Phase::Listening).then(|| self.set(Phase::Recognizing, "听清了…", ""))
    }

    /// 在想 / 在用工具。正在念或在等回答时不打断。
    pub fn thinking(&mut self, label: &str) -> Option<&Snapshot> {
        if self.holding || self.snap.phase == Phase::Asking {
            return None;
        }
        let said = self.snap.heard.clone();
        Some(self.set(Phase::Thinking, label, &said))
    }

    /// 正在操作游戏（`game.act`）：状态行“正在操作”，正文是这一步在做什么。
    /// 正在念或在等回答时不打断。
    pub fn acting(&mut self, what: &str) -> Option<&Snapshot> {
        if self.holding || self.snap.phase == Phase::Asking {
            return None;
        }
        Some(self.set(Phase::Thinking, ACTING_LABEL, what))
    }

    /// 显示一条状态（状态行 + 正文），正在念或在等回答时不打断。
    pub fn acting_like(&mut self, label: &str, what: &str) -> Option<&Snapshot> {
        if self.holding || self.snap.phase == Phase::Asking {
            return None;
        }
        Some(self.set(Phase::Thinking, label, what))
    }

    /// 还显示着这条状态就收起。
    pub fn label_done(&mut self, label: &str) -> Option<&Snapshot> {
        (self.snap.phase == Phase::Thinking && self.snap.label == label)
            .then(|| self.set(Phase::Hidden, "", ""))
    }

    /// 操作结束：还显示着“正在操作”就收起（语音入口接着会显示“在想…”）。
    pub fn acting_done(&mut self) -> Option<&Snapshot> {
        (self.snap.phase == Phase::Thinking && self.snap.label == ACTING_LABEL)
            .then(|| self.set(Phase::Hidden, "", ""))
    }

    /// 气泡上显示的名字（设置里改过名字就用新名字）。
    pub fn set_name(&mut self, name: &str) {
        self.name = if name.trim().is_empty() {
            kotone_agent::workspace::DEFAULT_NAME.to_string()
        } else {
            name.trim().to_string()
        };
    }

    pub fn speaking(&mut self, text: &str) -> &Snapshot {
        self.holding = true;
        let name = self.name();
        self.set(Phase::Speaking, &name, text)
    }

    /// 这句念完了：返回应停留多久后收起（序号对不上 = 已经有新内容，不收）。
    pub fn spoken(&mut self, revision: u64) -> Option<Duration> {
        if self.snap.revision != revision {
            return None;
        }
        self.holding = false;
        Some(dwell(&self.snap.text))
    }

    pub fn asking(&mut self, question: &str) -> &Snapshot {
        self.holding = false;
        self.snap.heard.clear();
        self.set(Phase::Asking, "等你回答", question)
    }

    pub fn ask_listening(&mut self, seconds: u64, now_ms: u64) -> Option<&Snapshot> {
        if self.snap.phase != Phase::Asking {
            return None;
        }
        self.snap.revision += 1;
        self.snap.ask_seconds = Some(seconds);
        self.snap.ask_started_ms = Some(now_ms);
        Some(&self.snap)
    }

    /// 追问结束（拿到回答或没回答），回到“在想”。
    pub fn ask_done(&mut self) -> Option<&Snapshot> {
        if self.snap.phase != Phase::Asking {
            return None;
        }
        Some(self.set(Phase::Thinking, "在想…", ""))
    }

    pub fn note(&mut self, text: &str) -> &Snapshot {
        self.holding = false;
        let name = self.name();
        self.set(Phase::Note, &name, text)
    }

    pub fn error(&mut self, text: &str) -> &Snapshot {
        self.holding = false;
        self.set(Phase::Error, "出错了", text)
    }

    /// 这一轮结束且没有要念的话：还在“在想”就收起。
    pub fn finished(&mut self) -> Option<&Snapshot> {
        (matches!(self.snap.phase, Phase::Thinking | Phase::Recognizing) && !self.holding)
            .then(|| self.set(Phase::Hidden, "", ""))
    }

    pub fn hide(&mut self) -> &Snapshot {
        self.holding = false;
        self.snap.heard.clear();
        self.set(Phase::Hidden, "", "")
    }

    /// 只在序号没变时收起（中间有新内容就不收）。
    pub fn hide_if(&mut self, revision: u64) -> Option<&Snapshot> {
        (self.snap.revision == revision && self.snap.phase != Phase::Hidden).then(|| self.hide())
    }
}

/// 有子会话在听队友时的状态行（ADR 021）。
pub const LISTEN_LABEL: &str = "正在听队友";

/// 操作游戏时气泡的状态行（ADR 019：“正在操作：打开背包…”，并提示怎么停）。
pub const ACTING_LABEL: &str = "正在操作 · 按 Esc 停止";

/// 念完后停留多久：3 秒 + 每 10 个字 1 秒，最多 10 秒（参考 VPet / dsh-pet）。
pub fn dwell(text: &str) -> Duration {
    let chars = text.chars().count() as u64;
    Duration::from_millis((3_000 + chars * 100).min(10_000))
}

/// 工具名 → 气泡状态行。
pub fn tool_label(name: &str) -> &'static str {
    match name.split('.').next().unwrap_or(name) {
        "companion" | "screen" | "window" | "ocr" | "chat" | "image" => "正在看屏幕",
        "automation" | "saved" => "正在执行自动化",
        "game" => "正在处理游戏窗口",
        "tools" => "在找能用的工具",
        "settings" | "hotkeys" | "profiles" | "audio" | "connections" | "autostart" => "正在改设置",
        "models" | "speech" | "wake" | "stt" => "正在处理模型",
        "reminder" => "正在设提醒",
        "game_chat" => "正在打字",
        _ => "在想…",
    }
}

// ---------- 桌面端胶水 ----------

#[derive(Default)]
pub struct BubbleRuntime {
    model: Mutex<Model>,
}

fn config(app: &AppHandle) -> Option<BubbleConfig> {
    app.try_state::<SharedState>()
        .map(|s| s.settings.read().unwrap().assistant.bubble.clone())
}

/// 气泡开着吗（关掉 = 回到旧的悬浮窗 + 弹幕）。
pub fn enabled(app: &AppHandle) -> bool {
    config(app).is_some_and(|c| c.enabled)
}

/// 这次录音会话归气泡管吗：琴音会话、来自热键 / 唤醒（设置窗口语音按钮的会话在聊天窗口里显示）。
pub fn owns_session(app: &AppHandle, payload: &Value) -> bool {
    enabled(app)
        && payload.pointer("/payload/purpose").and_then(Value::as_str) == Some("assistant")
        && payload
            .pointer("/payload/inputSource")
            .and_then(Value::as_str)
            != Some("button")
}

/// 改状态并推送；返回改后的序号。
fn update(app: &AppHandle, change: impl FnOnce(&mut Model) -> Option<Snapshot>) -> Option<u64> {
    let rt = app.try_state::<BubbleRuntime>()?;
    let snapshot = {
        let name = app
            .try_state::<SharedState>()
            .map(|state| state.settings.read().unwrap().agent.identity.name.clone())
            .unwrap_or_default();
        let mut model = rt.model.lock().unwrap();
        model.set_name(&name);
        let mut snap = change(&mut model)?;
        snap.corner = config(app).map(|c| c.corner).unwrap_or_default();
        snap
    };
    let revision = snapshot.revision;
    let _ = app.emit(EVENT, &snapshot);
    apply_window(app);
    Some(revision)
}

fn schedule_hide(app: &AppHandle, revision: u64, after: Duration) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(after).await;
        update(&app, |m| m.hide_if(revision).cloned());
    });
}

pub fn snapshot(app: &AppHandle) -> Option<Snapshot> {
    let rt = app.try_state::<BubbleRuntime>()?;
    let mut snap = rt.model.lock().unwrap().snapshot().clone();
    snap.corner = config(app).map(|c| c.corner).unwrap_or_default();
    Some(snap)
}

/// 录音状态（`kotone://state`）。只处理归气泡管的会话。
pub fn on_state(app: &AppHandle, payload: &Value) {
    let state = payload.get("state").and_then(Value::as_str).unwrap_or("");
    if state != "idle" && !owns_session(app, payload) {
        return;
    }
    match state {
        "listening" => {
            update(app, |m| Some(m.listening().clone()));
        }
        "transcribing" | "processing" => {
            update(app, |m| m.recognizing().cloned());
        }
        "success" => {
            update(app, |m| m.thinking("在想…").cloned());
        }
        "error" => {
            let message = payload
                .pointer("/payload/message")
                .and_then(Value::as_str)
                .unwrap_or("出了点问题")
                .to_string();
            if let Some(rev) = update(app, |m| Some(m.error(&message).clone())) {
                schedule_hide(app, rev, Duration::from_secs(6));
            }
        }
        "idle" => {
            // 会话被取消（Esc、热键取消）：还在听就收起；交给琴音以后的状态由琴音那边管。
            update(app, |m| {
                matches!(m.snapshot().phase, Phase::Listening | Phase::Recognizing)
                    .then(|| m.hide().clone())
            });
        }
        _ => {}
    }
}

/// 唤醒词刚命中：先亮出“在听…”（应答说完、收音开始前就出现）。10 秒内没有任何后续就收起。
pub fn woke(app: &AppHandle) {
    if !enabled(app) {
        return;
    }
    if let Some(rev) = update(app, |m| Some(m.listening().clone())) {
        schedule_hide(app, rev, Duration::from_secs(10));
    }
}

pub fn on_partial(app: &AppHandle, text: &str) {
    update(app, |m| m.heard(text).cloned());
}

/// 唤醒后一直没开口。
pub fn on_no_speech(app: &AppHandle) {
    if let Some(rev) = update(app, |m| {
        matches!(m.snapshot().phase, Phase::Listening).then(|| m.note("没听到，先不打扰啦").clone())
    }) {
        schedule_hide(app, rev, Duration::from_millis(1_500));
    }
}

pub fn thinking(app: &AppHandle, label: &str) {
    if enabled(app) {
        update(app, |m| m.thinking(label).cloned());
    }
}

/// 正在听某个程序（ADR 021）：气泡角落提示“正在听队友”。None = 没人在听了，收起提示。
pub fn listening_to(app: &AppHandle, app_name: Option<&str>) {
    match app_name {
        Some(name) => {
            update(app, |m| {
                m.acting_like(LISTEN_LABEL, &format!("在听 {name} 里的声音"))
                    .cloned()
            });
        }
        None => {
            update(app, |m| m.label_done(LISTEN_LABEL).cloned());
        }
    }
}

/// 正在操作游戏：不论从哪个入口发起、气泡开没开都显示（玩家要看得见琴音在动键鼠、知道按 Esc 能停）。
pub fn acting(app: &AppHandle, what: &str) {
    update(app, |m| m.acting(what).cloned());
}

pub fn acting_done(app: &AppHandle) {
    update(app, |m| m.acting_done().cloned());
}

pub fn asking(app: &AppHandle, question: &str) {
    if enabled(app) {
        update(app, |m| Some(m.asking(question).clone()));
    }
}

pub fn ask_listening(app: &AppHandle, seconds: u64) {
    update(app, |m| m.ask_listening(seconds, now_ms()).cloned());
}

pub fn ask_done(app: &AppHandle) {
    update(app, |m| m.ask_done().cloned());
}

/// 这一轮结束了：没有要念的话就收起。
pub fn finished(app: &AppHandle) {
    update(app, |m| m.finished().cloned());
}

/// 念一句话：气泡显示这句，念完再停一会儿才收。`speak` 返回时这句已经念完（或失败）。
pub async fn say<F, Fut>(app: &AppHandle, text: &str, speak: F)
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let revision = update(app, |m| Some(m.speaking(text).clone()));
    speak().await;
    if let Some(revision) = revision {
        let after = app
            .try_state::<BubbleRuntime>()
            .and_then(|rt| rt.model.lock().unwrap().spoken(revision));
        if let Some(after) = after {
            schedule_hide(app, revision, after);
        }
    }
}

/// 显示一句提示（不念），按字数停留后收起。
pub fn note(app: &AppHandle, text: &str) {
    if let Some(rev) = update(app, |m| Some(m.note(text).clone())) {
        schedule_hide(app, rev, dwell(text));
    }
}

/// 设置页“试一下”：显示一句示例，几秒后收起。
pub fn preview(app: &AppHandle) {
    if let Some(rev) = update(app, |m| {
        Some(
            m.speaking("我在这儿～游戏里叫我的时候，我会在这个角落说话。")
                .clone(),
        )
    }) {
        if let Some(rt) = app.try_state::<BubbleRuntime>() {
            rt.model.lock().unwrap().spoken(rev);
        }
        schedule_hide(app, rev, Duration::from_secs(5));
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default()
}

/// 按最新状态显示 / 隐藏 / 摆位。在主线程上按顺序执行。
fn apply_window(app: &AppHandle) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        let Some(window) = handle.get_webview_window(LABEL) else {
            return;
        };
        let Some(snap) = snapshot(&handle) else {
            return;
        };
        if snap.phase == Phase::Hidden {
            crate::hide_window(&window);
            return;
        }
        if crate::fullscreen::is_exclusive_fullscreen_active() == Some(true) {
            // 独占全屏时透明窗口会让游戏最小化或显示不出来：只靠语音。
            crate::hide_window(&window);
            return;
        }
        let config = config(&handle).unwrap_or_default();
        if let Err(e) = place(&window, &config) {
            log::log(&format!("bubble: place failed: {e}"));
        }
        let _ = window.set_ignore_cursor_events(true);
        crate::show_window_no_focus(&window);
    });
}

/// 摆到前台窗口所在显示器的工作区角落（避开任务栏），DPI 按那块屏幕换算。
fn place(window: &tauri::WebviewWindow, config: &BubbleConfig) -> Result<(), String> {
    let (left, top, right, bottom) = foreground_work_area().ok_or("无法确定显示器")?;
    let scale = window
        .available_monitors()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|m| {
            let p = m.position();
            let s = m.size();
            let cx = (left + right) / 2;
            let cy = (top + bottom) / 2;
            cx >= p.x && cx < p.x + s.width as i32 && cy >= p.y && cy < p.y + s.height as i32
        })
        .map(|m| m.scale_factor())
        .unwrap_or(1.0);
    let width = (WINDOW_SIZE.0 * scale).round() as i32;
    let height = (WINDOW_SIZE.1 * scale).round() as i32;
    let margin = (MARGIN * scale).round() as i32;
    let offset = (config.offset_y as f64 * scale).round() as i32;
    let x = match config.corner {
        BubbleCorner::BottomRight | BubbleCorner::TopRight => right - width - margin,
        BubbleCorner::BottomLeft | BubbleCorner::TopLeft => left + margin,
    };
    let y = match config.corner {
        BubbleCorner::BottomRight | BubbleCorner::BottomLeft => bottom - height - margin - offset,
        BubbleCorner::TopRight | BubbleCorner::TopLeft => top + margin + offset,
    };
    window
        .set_size(tauri::PhysicalSize::new(width as u32, height as u32))
        .map_err(|e| e.to_string())?;
    window
        .set_position(tauri::PhysicalPosition::new(x, y))
        .map_err(|e| e.to_string())
}

#[cfg(windows)]
fn foreground_work_area() -> Option<(i32, i32, i32, i32)> {
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTOPRIMARY,
    };
    use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    let ok = unsafe {
        GetMonitorInfoW(
            MonitorFromWindow(GetForegroundWindow(), MONITOR_DEFAULTTOPRIMARY),
            &mut info,
        )
    };
    ok.as_bool().then(|| {
        let r = info.rcWork;
        (r.left, r.top, r.right, r.bottom)
    })
}

#[cfg(not(windows))]
fn foreground_work_area() -> Option<(i32, i32, i32, i32)> {
    None
}

#[tauri::command]
pub fn bubble_snapshot(app: AppHandle) -> Option<Snapshot> {
    snapshot(&app)
}

#[tauri::command]
pub fn bubble_preview(app: AppHandle) {
    preview(&app);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_voice_turn_goes_listen_think_speak_then_hides_after_the_dwell() {
        let mut m = Model::default();
        assert_eq!(m.listening().phase, Phase::Listening);
        assert_eq!(m.heard("帮我看看").unwrap().heard, "帮我看看");
        assert_eq!(m.recognizing().unwrap().phase, Phase::Recognizing);
        let s = m.thinking("在想…").unwrap();
        assert_eq!((s.phase, s.text.as_str()), (Phase::Thinking, "帮我看看"));
        assert_eq!(m.thinking("正在看屏幕").unwrap().label, "正在看屏幕");
        let rev = m.speaking("队友说去下路").revision;
        // 念的时候，工具状态不覆盖这句
        assert!(m.thinking("正在执行自动化").is_none());
        assert_eq!(m.snapshot().phase, Phase::Speaking);
        let dwell = m.spoken(rev).unwrap();
        assert!(dwell >= Duration::from_secs(3));
        assert!(m.hide_if(rev).is_some());
        assert_eq!(m.snapshot().phase, Phase::Hidden);
    }

    #[test]
    fn late_events_do_not_hide_newer_content() {
        let mut m = Model::default();
        let old = m.speaking("第一句").revision;
        m.speaking("第二句");
        assert!(m.spoken(old).is_none(), "迟到的“念完了”不收新句子");
        assert!(m.hide_if(old).is_none());
        assert_eq!(m.snapshot().text, "第二句");
    }

    #[test]
    fn asking_keeps_its_question_until_answered() {
        let mut m = Model::default();
        m.asking("要我帮你下载吗？");
        assert!(m.ask_listening(8, 1_000).is_some());
        assert_eq!(m.snapshot().ask_seconds, Some(8));
        // 追问期间的录音状态和工具状态都不顶掉问题
        assert_eq!(m.listening().phase, Phase::Asking);
        assert!(m.thinking("在想…").is_none());
        assert_eq!(m.heard("可以").unwrap().heard, "可以");
        assert_eq!(m.ask_done().unwrap().phase, Phase::Thinking);
        assert_eq!(m.snapshot().ask_seconds, None);
    }

    #[test]
    fn acting_shows_the_step_and_does_not_cut_speech() {
        let mut m = Model::default();
        let snap = m.acting("按 i（1/2）").unwrap().clone();
        assert_eq!(
            (snap.phase, snap.label.as_str()),
            (Phase::Thinking, ACTING_LABEL)
        );
        assert_eq!(snap.text, "按 i（1/2）");
        assert_eq!(m.acting_done().unwrap().phase, Phase::Hidden);
        // 别的“在想”不被 acting_done 收起；正在念时不打断
        m.thinking("在想…");
        assert!(m.acting_done().is_none());
        m.speaking("好的");
        assert!(m.acting("点击").is_none());
    }

    #[test]
    fn a_turn_without_speech_hides_when_finished() {
        let mut m = Model::default();
        m.listening();
        m.thinking("在想…");
        assert_eq!(m.finished().unwrap().phase, Phase::Hidden);
        // 正在念时“结束”不收，等念完的停留
        m.speaking("好了");
        assert!(m.finished().is_none());
    }

    #[test]
    fn the_bubble_shows_the_chosen_name() {
        let mut m = Model::default();
        assert_eq!(m.speaking("嗨").label, "琴音");
        m.set_name(" 小雨 ");
        assert_eq!(m.note("嗨").label, "小雨");
        m.set_name("");
        assert_eq!(m.note("嗨").label, "琴音");
    }

    #[test]
    fn dwell_grows_with_length_and_is_capped() {
        assert_eq!(dwell("好"), Duration::from_millis(3_100));
        assert_eq!(dwell(&"字".repeat(500)), Duration::from_secs(10));
    }

    #[test]
    fn tool_labels() {
        assert_eq!(tool_label("companion.read_chat"), "正在看屏幕");
        assert_eq!(tool_label("settings.update"), "正在改设置");
        assert_eq!(tool_label("saved.auto-1"), "正在执行自动化");
        assert_eq!(tool_label("models.download"), "正在处理模型");
        assert_eq!(tool_label("something.else"), "在想…");
    }
}
