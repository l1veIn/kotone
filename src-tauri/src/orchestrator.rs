//! orchestrator：唯一状态所有者。串联 hotkey → audio → stt → inject，
//! partial 转发、取消与超时（docs/development.md §4、§4.1、§5.1）
//!
//! 状态迁移全部在 Rust 侧完成，UI 只渲染 emit 的状态事件。
//!
//! 并发模型：
//! - `inner`（std Mutex）只存状态与句柄，**绝不跨 await 持有**；
//! - `op`（tokio Mutex）串行化 begin/end/cancel/confirm 的状态切换临界区；
//! - `gen` 代际计数：每次 begin/cancel 自增，async 空隙后（finalize、发送）校验 gen，
//!   期间被取消则丢弃过期结果，保证「任意状态 Esc 取消 → Idle」不错乱。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use serde_json::json;
use tokio::sync::{mpsc, oneshot};

use crate::audio::{AudioBackend, AudioHandle};
use crate::inject::{CancelToken, Injector};
use crate::profile::{self, GameProfile};
use crate::settings::Settings;
use crate::stt::{EngineRegistry, SessionConfig, SttEvent, SttSession};

/// finalize 超时（docs/development.md §6：finalize 设置 10s 超时）
const DEFAULT_FINALIZE_TIMEOUT: Duration = Duration::from_secs(10);
/// Success/Error toast 停留时长，之后自动回 Idle
const DEFAULT_TOAST_DWELL: Duration = Duration::from_millis(1500);

/// 核心状态机（§4.1）
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OrchestratorState {
    Idle,
    Listening,
    Transcribing,
    Preview,
    Sending,
    Success,
    Error,
}

/// Rust → UI 事件出口（生产实现发 Tauri 事件，测试用 Vec 收集）
pub trait Emitter: Send + Sync {
    fn emit(&self, event: &str, payload: serde_json::Value);
}

/// 进行中的会话句柄
struct ActiveSession {
    stop_tx: Option<oneshot::Sender<()>>,
    session_rx: Option<oneshot::Receiver<Box<dyn SttSession>>>,
    /// pump 线程据此决定收尾时 session.cancel() 还是交还 session
    cancelled_flag: Arc<AtomicBool>,
    /// 录音采集 guard：drop 即停止采集线程
    guard: Option<AudioHandle>,
    pump: tokio::task::JoinHandle<()>,
    level_task: tokio::task::JoinHandle<()>,
}

struct Inner {
    state: OrchestratorState,
    gen: u64,
    active: Option<ActiveSession>,
    preview_text: Option<String>,
    /// Sending 状态的取消令牌
    send_cancel: Option<CancelToken>,
}

pub struct Orchestrator {
    /// Arc：schedule_idle 的延迟任务需要共享访问
    inner: Arc<Mutex<Inner>>,
    op: tokio::sync::Mutex<()>,
    settings: Arc<RwLock<Settings>>,
    engines: Arc<EngineRegistry>,
    audio: Arc<dyn AudioBackend>,
    injector: Arc<dyn Injector>,
    emitter: Arc<dyn Emitter>,
    /// finalize 超时（测试可调小）
    pub finalize_timeout: Duration,
    /// Success/Error 停留时长（测试可设为 0）
    pub toast_dwell: Duration,
}

impl Orchestrator {
    pub fn new(
        settings: Arc<RwLock<Settings>>,
        engines: Arc<EngineRegistry>,
        audio: Arc<dyn AudioBackend>,
        injector: Arc<dyn Injector>,
        emitter: Arc<dyn Emitter>,
    ) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                state: OrchestratorState::Idle,
                gen: 0,
                active: None,
                preview_text: None,
                send_cancel: None,
            })),
            op: tokio::sync::Mutex::new(()),
            settings,
            engines,
            audio,
            injector,
            emitter,
            finalize_timeout: DEFAULT_FINALIZE_TIMEOUT,
            toast_dwell: DEFAULT_TOAST_DWELL,
        }
    }

    pub fn state(&self) -> OrchestratorState {
        self.inner.lock().unwrap().state
    }

    // ---------- 热键入口（hotkey 模块调用） ----------

    /// toggle 模式：按一下开始、再按结束；转写/发送中再按 = 中止（§4 设计要点 4）
    pub async fn on_hotkey_toggle(&self) {
        match self.state() {
            OrchestratorState::Idle => {
                let _ = self.begin().await;
            }
            OrchestratorState::Listening => {
                let _ = self.end().await;
            }
            _ => self.cancel().await,
        }
    }

    /// hold 模式：按下开始、松开结束
    pub async fn on_hotkey_hold(&self, pressed: bool) {
        if pressed {
            let _ = self.begin().await;
        } else if self.state() == OrchestratorState::Listening {
            let _ = self.end().await;
        }
    }

    // ---------- 状态机操作 ----------

    /// 开始一次「按下到松手」的会话：建 STT session → 开录音 → Listening
    pub async fn begin(&self) -> Result<(), String> {
        let _op = self.op.lock().await;
        {
            let inner = self.inner.lock().unwrap();
            if inner.state != OrchestratorState::Idle {
                return Err(format!("当前状态 {:?} 不能开始新会话", inner.state));
            }
        }
        match self.begin_locked() {
            Ok(()) => Ok(()),
            Err(e) => {
                // 开始失败（如引擎未就绪）：Error toast 提示后自动回 Idle
                self.toast_error(&e);
                Err(e)
            }
        }
    }

    fn begin_locked(&self) -> Result<(), String> {
        let settings = self.settings.read().unwrap().clone();
        let engine_id = settings.stt_engine.clone();
        let engine = self
            .engines
            .get(&engine_id)
            .ok_or_else(|| format!("未注册的 STT 引擎: {engine_id}"))?;
        if !engine.is_ready() {
            return Err(format!(
                "引擎「{}」未就绪（模型未下载或未实现），联调可切换到 mock-stream",
                engine.display_name()
            ));
        }

        let active_profile = settings
            .active_profile_id
            .as_deref()
            .and_then(profile::get)
            .unwrap_or_else(GameProfile::builtin_generic);
        let cfg = SessionConfig {
            language: settings.language.clone(),
            hotwords: active_profile.hotwords.clone(),
            options: settings
                .engine_options
                .get(&engine_id)
                .cloned()
                .unwrap_or(serde_json::Value::Null),
        };

        let (stt_tx, stt_rx) = mpsc::unbounded_channel::<SttEvent>();
        let session = engine.start_session(&cfg, stt_tx)?;

        // 开录音失败要有清晰错误；已建的 session 取消掉
        let mut handle = match self.audio.start(&settings.audio_device_id) {
            Ok(h) => h,
            Err(e) => {
                let mut session = session;
                session.cancel();
                return Err(e);
            }
        };
        let pcm_rx = handle.pcm_rx.take().expect("audio handle pcm channel");
        let level_rx = handle.level_rx.take().expect("audio handle level channel");

        let (stop_tx, stop_rx) = oneshot::channel::<()>();
        let (session_tx, session_rx) = oneshot::channel::<Box<dyn SttSession>>();
        let cancelled_flag = Arc::new(AtomicBool::new(false));

        // PCM pump：录音 → session.push_audio；STT partial → kotone://partial
        let emitter = self.emitter.clone();
        let pump_flag = cancelled_flag.clone();
        let pump = tokio::spawn(async move {
            let mut session = session;
            let mut stop_rx = stop_rx;
            let mut pcm_rx = pcm_rx;
            let mut stt_rx = stt_rx;
            loop {
                tokio::select! {
                    biased;
                    _ = &mut stop_rx => break,
                    chunk = pcm_rx.recv() => {
                        match chunk {
                            Some(c) => {
                                if session.push_audio(&c).is_err() { break; }
                            }
                            None => break, // 采集结束
                        }
                    }
                    ev = stt_rx.recv() => {
                        match ev {
                            Some(SttEvent::Partial { text }) => {
                                emitter.emit("kotone://partial", json!({ "text": text }));
                            }
                            // Final 由 finalize 返回值承载，这里不重复上屏
                            Some(SttEvent::Final { .. }) | None => {}
                        }
                    }
                }
            }
            if pump_flag.load(Ordering::SeqCst) {
                session.cancel();
            }
            // 取消路径下接收端已 drop，send 失败则 session 随之释放
            let _ = session_tx.send(session);
        });

        // RMS 电平 → kotone://level（驱动波形，~50ms 一条）
        let emitter = self.emitter.clone();
        let level_task = tokio::spawn(async move {
            let mut level_rx = level_rx;
            while let Some(rms) = level_rx.recv().await {
                emitter.emit("kotone://level", json!({ "rms": rms }));
            }
        });

        let mut inner = self.inner.lock().unwrap();
        inner.gen += 1;
        inner.state = OrchestratorState::Listening;
        inner.preview_text = None;
        inner.active = Some(ActiveSession {
            stop_tx: Some(stop_tx),
            session_rx: Some(session_rx),
            cancelled_flag,
            guard: Some(handle),
            pump,
            level_task,
        });
        drop(inner);
        self.emit_state(OrchestratorState::Listening, None);
        Ok(())
    }

    /// 结束会话：finalize → autoSend 分流（§6 发送时序上半段）
    pub async fn end(&self) -> Result<(), String> {
        let (gen, mut active) = {
            let _op = self.op.lock().await;
            let mut inner = self.inner.lock().unwrap();
            if inner.state != OrchestratorState::Listening {
                return Err(format!("当前状态 {:?} 不能结束会话", inner.state));
            }
            inner.gen += 1;
            let active = inner
                .active
                .take()
                .ok_or_else(|| "会话句柄缺失（内部错误）".to_string())?;
            inner.state = OrchestratorState::Transcribing;
            let gen = inner.gen;
            drop(inner);
            self.emit_state(OrchestratorState::Transcribing, None);
            (gen, active)
        };

        // 停 pump 并取回 session（不持锁，允许 cancel 插队，gen 校验兜底）
        if let Some(tx) = active.stop_tx.take() {
            let _ = tx.send(());
        }
        let session = match active.session_rx.take() {
            Some(rx) => match rx.await {
                Ok(s) => s,
                Err(_) => {
                    self.fail(gen, "录音线程异常结束", None);
                    return Err("录音线程异常结束".into());
                }
            },
            None => {
                self.fail(gen, "会话句柄缺失（内部错误）", None);
                return Err("会话句柄缺失".into());
            }
        };
        // 停止采集线程与辅助任务
        drop(active.guard.take());
        active.pump.abort();
        active.level_task.abort();

        // finalize（同步引擎可能耗时，spawn_blocking + 10s 超时）
        let finalize = tokio::task::spawn_blocking(move || session.finalize());
        let result = match tokio::time::timeout(self.finalize_timeout, finalize).await {
            Ok(Ok(Ok(t))) => Ok(t),
            Ok(Ok(Err(e))) => Err(e),
            Ok(Err(_)) => Err("转写线程异常".to_string()),
            Err(_) => Err(format!("转写超时（{}s）", self.finalize_timeout.as_secs())),
        };

        // 期间被取消 → 丢弃过期结果
        if self.inner.lock().unwrap().gen != gen {
            return Ok(());
        }

        match result {
            Ok(t) => {
                // 最终文本上屏（替换 partial）
                self.emitter
                    .emit("kotone://partial", json!({ "text": t.text }));
                // TODO(eval 子代理)：eval_recording 开启时在此录档（wav + partial 时间线 + 指标）
                let auto_send = self.settings.read().unwrap().auto_send;
                if auto_send {
                    self.do_send(t.text, gen).await;
                } else {
                    let _op = self.op.lock().await;
                    let mut inner = self.inner.lock().unwrap();
                    if inner.gen != gen {
                        return Ok(());
                    }
                    inner.preview_text = Some(t.text.clone());
                    inner.state = OrchestratorState::Preview;
                    drop(inner);
                    self.emit_state(
                        OrchestratorState::Preview,
                        Some(json!({ "text": t.text })),
                    );
                }
            }
            Err(e) => {
                self.fail(gen, &e, None);
                return Err(e);
            }
        }
        Ok(())
    }

    /// Preview 状态下用户确认/编辑后发送；Error 状态（payload 带文本）也接受，
    /// 重新进入 Sending 实现重试（docs/development.md §4.1「Error 保留文本可重试」）
    pub async fn confirm_send(&self, text: Option<String>) -> Result<(), String> {
        let (gen, final_text) = {
            let _op = self.op.lock().await;
            let inner = self.inner.lock().unwrap();
            match inner.state {
                OrchestratorState::Preview | OrchestratorState::Error => {}
                s => return Err(format!("当前状态 {s:?} 不能确认发送")),
            }
            let t = text
                .filter(|s| !s.trim().is_empty())
                .or_else(|| inner.preview_text.clone())
                .ok_or_else(|| "无待发送文本".to_string())?;
            (inner.gen, t)
        };
        self.do_send(final_text, gen).await;
        Ok(())
    }

    /// 任意状态取消：回到 Idle（session cancel；发送中置取消令牌）
    pub async fn cancel(&self) {
        let _op = self.op.lock().await;
        let mut inner = self.inner.lock().unwrap();
        if inner.state == OrchestratorState::Idle {
            return;
        }
        inner.gen += 1;
        let mut active = inner.active.take();
        inner.preview_text = None;
        if let Some(token) = inner.send_cancel.take() {
            token.cancel();
        }
        inner.state = OrchestratorState::Idle;
        drop(inner);

        if let Some(a) = active.as_mut() {
            a.cancelled_flag.store(true, Ordering::SeqCst);
            if let Some(tx) = a.stop_tx.take() {
                let _ = tx.send(());
            }
            drop(a.guard.take());
            a.pump.abort();
            a.level_task.abort();
            // session_rx 随 ActiveSession drop：pump 收尾时 session.cancel() 后发送失败即释放
        }
        self.emit_state(OrchestratorState::Idle, None);
    }

    // ---------- 内部 ----------

    /// Sending → Success/Error（§6 发送时序下半段；inject 实现负责按键细节）
    async fn do_send(&self, text: String, gen: u64) {
        let (profile, token) = {
            let _op = self.op.lock().await;
            let mut inner = self.inner.lock().unwrap();
            if inner.gen != gen {
                return;
            }
            inner.state = OrchestratorState::Sending;
            let token = CancelToken::default();
            inner.send_cancel = Some(token.clone());
            drop(inner);
            self.emit_state(OrchestratorState::Sending, Some(json!({ "text": text })));
            let settings = self.settings.read().unwrap().clone();
            let profile = settings
                .active_profile_id
                .as_deref()
                .and_then(profile::get)
                .unwrap_or_else(GameProfile::builtin_generic);
            (profile, token)
        };

        let injector = self.injector.clone();
        let send_text = text.clone();
        let result =
            tokio::task::spawn_blocking(move || injector.send(&send_text, &profile, token)).await;

        let _op = self.op.lock().await;
        let mut inner = self.inner.lock().unwrap();
        if inner.gen != gen {
            return;
        }
        inner.send_cancel = None;
        match result {
            Ok(Ok(())) => {
                inner.preview_text = None;
                inner.state = OrchestratorState::Success;
                drop(inner);
                self.emit_state(OrchestratorState::Success, Some(json!({ "text": text })));
            }
            Ok(Err(e)) => {
                // Error 保留文本（preview_text 承载），前端/confirm_send 可重试（§4.1）；
                // needsElevation 透传 UIPI 提权信号（§10 R-1）
                inner.preview_text = Some(text.clone());
                inner.state = OrchestratorState::Error;
                drop(inner);
                self.emit_state(
                    OrchestratorState::Error,
                    Some(json!({ "message": e.message, "needsElevation": e.needs_elevation, "text": text })),
                );
            }
            Err(_) => {
                inner.preview_text = Some(text.clone());
                inner.state = OrchestratorState::Error;
                drop(inner);
                self.emit_state(
                    OrchestratorState::Error,
                    Some(json!({ "message": "发送线程异常", "text": text })),
                );
            }
        }
        drop(_op);
        self.schedule_idle(gen);
    }

    /// 开始失败等场景：Error toast → 自动回 Idle（无文本，不可重试）
    fn toast_error(&self, message: &str) {
        let gen = {
            let mut inner = self.inner.lock().unwrap();
            inner.preview_text = None;
            inner.state = OrchestratorState::Error;
            inner.gen
        };
        self.emit_state(
            OrchestratorState::Error,
            Some(json!({ "message": message })),
        );
        self.schedule_idle(gen);
    }

    /// 转写失败：Error toast（保留文本可重试时经 fail 的 text 参数携带）
    fn fail(&self, gen: u64, message: &str, text: Option<String>) {
        let mut inner = self.inner.lock().unwrap();
        if inner.gen != gen {
            return;
        }
        inner.preview_text = text.clone();
        inner.state = OrchestratorState::Error;
        drop(inner);
        self.emit_state(
            OrchestratorState::Error,
            Some(json!({ "message": message, "text": text })),
        );
        self.schedule_idle(gen);
    }

    /// toast_dwell 后自动回 Idle（期间有新会话/取消则不动作）。
    /// 带文本的 Error 不自动回 Idle：保留待重试文本，等用户重试或取消（§4.1）。
    fn schedule_idle(&self, gen: u64) {
        let dwell = self.toast_dwell;
        let inner = self.inner.clone();
        let emitter = self.emitter.clone();
        tokio::spawn(async move {
            tokio::time::sleep(dwell).await;
            let mut g = inner.lock().unwrap();
            let should_idle = g.gen == gen
                && (g.state == OrchestratorState::Success
                    || (g.state == OrchestratorState::Error && g.preview_text.is_none()));
            if should_idle {
                g.state = OrchestratorState::Idle;
                drop(g);
                emitter.emit(
                    "kotone://state",
                    json!({ "state": OrchestratorState::Idle, "payload": null }),
                );
            }
        });
    }

    fn emit_state(&self, state: OrchestratorState, payload: Option<serde_json::Value>) {
        self.emitter.emit(
            "kotone://state",
            json!({ "state": state, "payload": payload }),
        );
    }
}

// ---------- 测试：mock audio / mock inject / vec emitter 全链路状态迁移 ----------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::AudioHandle;
    use crate::inject::InjectError;
    use tokio::sync::mpsc;

    /// 10 倍速推送假音频的 mock 采集后端（每 5ms 推一个 50ms chunk）
    struct MockAudioBackend;

    impl AudioBackend for MockAudioBackend {
        fn start(&self, _device_id: &str) -> Result<AudioHandle, String> {
            let (pcm_tx, pcm_rx) = mpsc::unbounded_channel::<Vec<f32>>();
            let (level_tx, level_rx) = mpsc::unbounded_channel::<f32>();
            tokio::spawn(async move {
                loop {
                    if pcm_tx.send(vec![0.1f32; 800]).is_err() {
                        break;
                    }
                    if level_tx.send(0.1f32).is_err() {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            });
            Ok(AudioHandle::detached(pcm_rx, level_rx))
        }
    }

    /// 记录发送文本的 mock 注入器
    struct RecordingInjector {
        sent: Arc<Mutex<Vec<String>>>,
    }

    impl Injector for RecordingInjector {
        fn send(
            &self,
            text: &str,
            _profile: &GameProfile,
            _cancel: CancelToken,
        ) -> Result<(), InjectError> {
            self.sent.lock().unwrap().push(text.to_string());
            Ok(())
        }
    }

    /// 第一次发送失败、之后成功的注入器（验证 Error 保留文本可重试，§4.1）
    struct FlakyInjector {
        attempts: Arc<std::sync::atomic::AtomicUsize>,
        sent: Arc<Mutex<Vec<String>>>,
    }

    impl Injector for FlakyInjector {
        fn send(
            &self,
            text: &str,
            _profile: &GameProfile,
            _cancel: CancelToken,
        ) -> Result<(), InjectError> {
            let n = self.attempts.fetch_add(1, Ordering::SeqCst);
            if n == 0 {
                return Err(InjectError::new("游戏不在前台：目标进程未处于前台"));
            }
            self.sent.lock().unwrap().push(text.to_string());
            Ok(())
        }
    }

    /// Vec 收集事件的 mock emitter
    #[derive(Default)]
    struct VecEmitter {
        events: Mutex<Vec<(String, serde_json::Value)>>,
    }

    impl VecEmitter {
        fn state_sequence(&self) -> Vec<String> {
            self.events
                .lock()
                .unwrap()
                .iter()
                .filter(|(e, _)| e == "kotone://state")
                .filter_map(|(_, p)| p.get("state")?.as_str().map(String::from))
                .collect()
        }
        fn partials(&self) -> Vec<String> {
            self.events
                .lock()
                .unwrap()
                .iter()
                .filter(|(e, _)| e == "kotone://partial")
                .filter_map(|(_, p)| p.get("text")?.as_str().map(String::from))
                .collect()
        }
        fn levels(&self) -> usize {
            self.events
                .lock()
                .unwrap()
                .iter()
                .filter(|(e, _)| e == "kotone://level")
                .count()
        }
    }

    impl Emitter for VecEmitter {
        fn emit(&self, event: &str, payload: serde_json::Value) {
            self.events
                .lock()
                .unwrap()
                .push((event.to_string(), payload));
        }
    }

    fn make_orchestrator(
        auto_send: bool,
    ) -> (Arc<Orchestrator>, Arc<VecEmitter>, Arc<Mutex<Vec<String>>>) {
        let sent = Arc::new(Mutex::new(Vec::new()));
        let injector: Arc<dyn Injector> = Arc::new(RecordingInjector { sent: sent.clone() });
        let (orch, emitter) = make_orchestrator_with(auto_send, injector);
        (orch, emitter, sent)
    }

    fn make_orchestrator_with(
        auto_send: bool,
        injector: Arc<dyn Injector>,
    ) -> (Arc<Orchestrator>, Arc<VecEmitter>) {
        let mut settings = Settings::default();
        settings.stt_engine = "mock-stream".into();
        settings.auto_send = auto_send;
        settings.active_profile_id = None; // 测试不依赖真实 ~/.kotone
        let settings = Arc::new(RwLock::new(settings));
        let engines = Arc::new(EngineRegistry::new());
        let emitter = Arc::new(VecEmitter::default());
        let mut orch = Orchestrator::new(
            settings,
            engines,
            Arc::new(MockAudioBackend),
            injector,
            emitter.clone(),
        );
        orch.toast_dwell = Duration::from_millis(10);
        orch.finalize_timeout = Duration::from_secs(2);
        (Arc::new(orch), emitter)
    }

    #[tokio::test]
    async fn preview_flow_full_transitions() {
        let (orch, emitter, sent) = make_orchestrator(false);

        orch.begin().await.unwrap();
        assert_eq!(orch.state(), OrchestratorState::Listening);

        // 等 partial（Windows 定时器粒度约 15.6ms，留足余量：500ms ≈ 30+ chunk > 8000 采样阈值）
        tokio::time::sleep(Duration::from_millis(500)).await;
        let partials = emitter.partials();
        assert!(
            partials.contains(&"对面".to_string()),
            "partials: {partials:?}, all events: {:?}",
            emitter.events.lock().unwrap()
        );
        assert!(emitter.levels() > 0, "录音期间应有 RMS 电平事件");

        orch.end().await.unwrap();
        assert_eq!(orch.state(), OrchestratorState::Preview);
        // 最终文本上屏
        assert!(emitter
            .partials()
            .contains(&"对面打野在下路".to_string()));

        orch.confirm_send(None).await.unwrap();
        assert_eq!(orch.state(), OrchestratorState::Success);
        assert_eq!(
            sent.lock().unwrap().as_slice(),
            &["对面打野在下路".to_string()]
        );

        tokio::time::sleep(Duration::from_millis(40)).await;
        assert_eq!(orch.state(), OrchestratorState::Idle);

        let seq = emitter.state_sequence();
        for expected in [
            "listening",
            "transcribing",
            "preview",
            "sending",
            "success",
            "idle",
        ] {
            assert!(seq.contains(&expected.to_string()), "缺状态 {expected}: {seq:?}");
        }
    }

    #[tokio::test]
    async fn auto_send_flow_skips_preview() {
        let (orch, emitter, sent) = make_orchestrator(true);

        orch.begin().await.unwrap();
        tokio::time::sleep(Duration::from_millis(60)).await;
        orch.end().await.unwrap();

        assert_eq!(orch.state(), OrchestratorState::Success);
        assert_eq!(
            sent.lock().unwrap().as_slice(),
            &["对面打野在下路".to_string()]
        );
        let seq = emitter.state_sequence();
        assert!(!seq.contains(&"preview".to_string()), "autoSend 不应进 Preview");
        assert!(seq.contains(&"sending".to_string()));
    }

    #[tokio::test]
    async fn confirm_send_with_edited_text() {
        let (orch, _emitter, sent) = make_orchestrator(false);
        orch.begin().await.unwrap();
        tokio::time::sleep(Duration::from_millis(30)).await;
        orch.end().await.unwrap();
        assert_eq!(orch.state(), OrchestratorState::Preview);

        orch.confirm_send(Some("编辑后的文本".into())).await.unwrap();
        assert_eq!(
            sent.lock().unwrap().as_slice(),
            &["编辑后的文本".to_string()]
        );
    }

    #[tokio::test]
    async fn cancel_during_listening_returns_to_idle() {
        let (orch, emitter, sent) = make_orchestrator(false);
        orch.begin().await.unwrap();
        tokio::time::sleep(Duration::from_millis(30)).await;

        orch.cancel().await;
        assert_eq!(orch.state(), OrchestratorState::Idle);
        assert!(emitter.state_sequence().contains(&"idle".to_string()));

        // 取消后不能再 end；也不应有任何发送
        assert!(orch.end().await.is_err());
        assert!(sent.lock().unwrap().is_empty());

        // 取消后可重新开始新会话
        orch.begin().await.unwrap();
        assert_eq!(orch.state(), OrchestratorState::Listening);
        orch.cancel().await;
    }

    #[tokio::test]
    async fn double_begin_rejected() {
        let (orch, _e, _s) = make_orchestrator(false);
        orch.begin().await.unwrap();
        let second = orch.begin().await;
        assert!(second.is_err(), "Listening 状态不允许再次 begin");
        orch.cancel().await;
    }

    #[tokio::test]
    async fn begin_with_unready_engine_toasts_error() {
        let (orch, emitter, _s) = {
            let (o, e, s) = make_orchestrator(false);
            o.settings.write().unwrap().stt_engine = "whisper-cpp-sidecar".into();
            (o, e, s)
        };
        let r = orch.begin().await;
        assert!(r.is_err(), "未就绪引擎应开始失败");
        // Error toast 已发出，dwell 后自动回 Idle
        tokio::time::sleep(Duration::from_millis(40)).await;
        assert_eq!(orch.state(), OrchestratorState::Idle);
        assert!(emitter.state_sequence().contains(&"error".to_string()));
    }

    /// §4.1：Error 保留文本可重试——confirm_send 在 Error 状态重新进入 Sending
    #[tokio::test]
    async fn error_state_retains_text_and_confirm_send_retries() {
        let sent = Arc::new(Mutex::new(Vec::new()));
        let injector: Arc<dyn Injector> = Arc::new(FlakyInjector {
            attempts: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            sent: sent.clone(),
        });
        let (orch, emitter) = make_orchestrator_with(false, injector);

        orch.begin().await.unwrap();
        tokio::time::sleep(Duration::from_millis(60)).await;
        orch.end().await.unwrap();
        assert_eq!(orch.state(), OrchestratorState::Preview);

        // 第一次确认：注入失败 → Error；dwell 后仍停在 Error（文本保留，可重试）
        orch.confirm_send(None).await.unwrap();
        assert_eq!(orch.state(), OrchestratorState::Error);
        tokio::time::sleep(Duration::from_millis(50)).await; // > toast_dwell(10ms)
        assert_eq!(
            orch.state(),
            OrchestratorState::Error,
            "带文本的 Error 不应自动回 Idle，需保留给用户重试"
        );

        // 重试：Error 状态 confirm_send 重新进入 Sending 并成功
        orch.confirm_send(None).await.unwrap();
        assert_eq!(orch.state(), OrchestratorState::Success);
        assert_eq!(
            sent.lock().unwrap().as_slice(),
            &["对面打野在下路".to_string()]
        );
        let seq = emitter.state_sequence();
        assert_eq!(
            seq.iter().filter(|s| *s == "sending").count(),
            2,
            "重试应第二次进入 Sending: {seq:?}"
        );

        // Success 仍按 toast 节奏自动回 Idle
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(orch.state(), OrchestratorState::Idle);
    }

    /// Error 状态下可带编辑后文本重试
    #[tokio::test]
    async fn error_state_retry_with_edited_text() {
        let sent = Arc::new(Mutex::new(Vec::new()));
        let injector: Arc<dyn Injector> = Arc::new(FlakyInjector {
            attempts: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            sent: sent.clone(),
        });
        let (orch, _emitter) = make_orchestrator_with(false, injector);

        orch.begin().await.unwrap();
        tokio::time::sleep(Duration::from_millis(60)).await;
        orch.end().await.unwrap();
        orch.confirm_send(None).await.unwrap();
        assert_eq!(orch.state(), OrchestratorState::Error);

        orch.confirm_send(Some("编辑后重发".into())).await.unwrap();
        assert_eq!(orch.state(), OrchestratorState::Success);
        assert_eq!(
            sent.lock().unwrap().as_slice(),
            &["编辑后重发".to_string()]
        );
    }

    /// 无文本的 Error（如引擎未就绪）不可重试，且仍自动回 Idle
    #[tokio::test]
    async fn error_without_text_rejects_retry_and_auto_idles() {
        let (orch, emitter, _s) = {
            let (o, e, s) = make_orchestrator(false);
            o.settings.write().unwrap().stt_engine = "whisper-cpp-sidecar".into();
            (o, e, s)
        };
        let _ = orch.begin().await;
        assert_eq!(orch.state(), OrchestratorState::Error);
        // 无待发送文本：confirm_send 拒绝
        assert!(orch.confirm_send(None).await.is_err());
        // dwell 后自动回 Idle
        tokio::time::sleep(Duration::from_millis(40)).await;
        assert_eq!(orch.state(), OrchestratorState::Idle);
        assert!(emitter.state_sequence().contains(&"error".to_string()));
    }

    /// Error 状态取消：清空保留文本并回 Idle，之后不可再重试
    #[tokio::test]
    async fn cancel_from_error_clears_retry_text() {
        let sent = Arc::new(Mutex::new(Vec::new()));
        let injector: Arc<dyn Injector> = Arc::new(FlakyInjector {
            attempts: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            sent: sent.clone(),
        });
        let (orch, _emitter) = make_orchestrator_with(false, injector);

        orch.begin().await.unwrap();
        tokio::time::sleep(Duration::from_millis(60)).await;
        orch.end().await.unwrap();
        orch.confirm_send(None).await.unwrap();
        assert_eq!(orch.state(), OrchestratorState::Error);

        orch.cancel().await;
        assert_eq!(orch.state(), OrchestratorState::Idle);
        assert!(orch.confirm_send(None).await.is_err(), "取消后不可再重试");
    }
}
