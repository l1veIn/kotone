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
use crate::inject::{CancelToken, FocusBackend, Injector, TargetWindow};
use crate::interaction::{EndTrigger, InteractionPolicy, PostFinalize};
use crate::profile::{self, GameProfile};
use crate::settings::Settings;
use crate::stt::{EngineRegistry, SessionConfig, SttEvent, SttSession};

/// finalize 超时（docs/development.md §6：finalize 设置 10s 超时）
const DEFAULT_FINALIZE_TIMEOUT: Duration = Duration::from_secs(10);
/// Success/Error toast 停留时长，之后自动回 Idle
const DEFAULT_TOAST_DWELL: Duration = Duration::from_millis(1500);
/// 发送前焦点恢复后的等待：给系统完成前台切换与目标窗口激活的时间
const DEFAULT_FOCUS_RESTORE_DELAY: Duration = Duration::from_millis(30);

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
    /// eval 录档句柄（evalRecording 开启时存在；取消时随会话丢弃不录）
    recorder: Option<crate::eval::SessionRecorder>,
}

struct Inner {
    state: OrchestratorState,
    gen: u64,
    active: Option<ActiveSession>,
    preview_text: Option<String>,
    /// Sending 状态的取消令牌
    send_cancel: Option<CancelToken>,
    /// begin 时记录的前台窗口 = 注入目标（发送前把焦点还给它）
    target_window: Option<TargetWindow>,
    /// history 草稿（history.mode != off 时 begin 创建；不放 ActiveSession——
    /// end() 会 take 走 active，而 Sending/Error 态的终态落账仍需要草稿）
    history: Option<Arc<Mutex<HistoryDraft>>>,
}

/// history 草稿：会话期间累计指标，终态（sent/cancelled/error）时落账一条记录
struct HistoryDraft {
    /// 与 eval 录档共用的 sessionId（begin 生成一次同时喂两边，可互查）
    session_id: String,
    started: std::time::Instant,
    engine_id: String,
    profile_id: Option<String>,
    /// 已喂入的采样数（落账时按 eval::SAMPLE_RATE 换算 ms）
    audio_samples: u64,
    first_partial_ms: Option<u64>,
    final_text: String,
    finalize_latency_ms: Option<u64>,
    /// error 终态已落账（Error 态保留文本供重试：重试成功仍写 sent；
    /// error 后的 cancel 是清理动作，不再双记 cancelled）
    reported: bool,
}

pub struct Orchestrator {
    /// Arc：schedule_idle 的延迟任务需要共享访问
    inner: Arc<Mutex<Inner>>,
    op: tokio::sync::Mutex<()>,
    settings: Arc<RwLock<Settings>>,
    engines: Arc<EngineRegistry>,
    audio: Arc<dyn AudioBackend>,
    injector: Arc<dyn Injector>,
    focus: Arc<dyn FocusBackend>,
    emitter: Arc<dyn Emitter>,
    /// 自引用（Weak）：VAD 判停时 pump 需要触发 end()（与热键结束同路径）
    me: RwLock<Option<std::sync::Weak<Orchestrator>>>,
    /// finalize 超时（测试可调小）
    pub finalize_timeout: Duration,
    /// Success/Error 停留时长（测试可设为 0）
    pub toast_dwell: Duration,
    /// 发送前焦点恢复后的等待（测试可设为 0）
    pub focus_restore_delay: Duration,
    /// eval 录档目录覆盖（None = ~/.kotone/eval/；测试指向临时目录）
    pub eval_dir: Option<std::path::PathBuf>,
    /// history 目录覆盖（None = ~/.kotone/history/；测试指向临时目录）
    pub history_dir: Option<std::path::PathBuf>,
    /// VAD 工厂（ADR-007，B3 静音判停；None = 未接入——VAD 策略下 begin 报清晰错误）。
    /// 构造后注入（壳/CLI 按 feature 与模型就绪情况接线）
    pub vad_factory: Option<crate::vad::VadFactory>,
}

impl Orchestrator {
    /// 构造（返回 Self 便于调用方先调整 pub 字段：超时/录档目录/vad_factory）。
    /// **必须接 `.into_arc()` 收尾**：VAD 判停靠 Weak 自引用触发 end()，
    /// 不 into_arc 的实例在 one-shot 模式下不会自动判停。
    pub fn new(
        settings: Arc<RwLock<Settings>>,
        engines: Arc<EngineRegistry>,
        audio: Arc<dyn AudioBackend>,
        injector: Arc<dyn Injector>,
        focus: Arc<dyn FocusBackend>,
        emitter: Arc<dyn Emitter>,
    ) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                state: OrchestratorState::Idle,
                gen: 0,
                active: None,
                preview_text: None,
                send_cancel: None,
                target_window: None,
                history: None,
            })),
            op: tokio::sync::Mutex::new(()),
            settings,
            engines,
            audio,
            injector,
            focus,
            emitter,
            me: RwLock::new(None),
            finalize_timeout: DEFAULT_FINALIZE_TIMEOUT,
            toast_dwell: DEFAULT_TOAST_DWELL,
            focus_restore_delay: DEFAULT_FOCUS_RESTORE_DELAY,
            eval_dir: None,
            history_dir: None,
            vad_factory: None,
        }
    }

    /// 包装为 Arc 并接线 Weak 自引用（VAD 判停 → end() 的回调通道）
    pub fn into_arc(self) -> Arc<Self> {
        let orch = Arc::new(self);
        *orch.me.write().unwrap() = Some(Arc::downgrade(&orch));
        orch
    }

    pub fn state(&self) -> OrchestratorState {
        self.inner.lock().unwrap().state
    }

    /// settings 句柄（集成测试与壳需要改写引擎选择等运行时配置）
    pub fn settings(&self) -> &Arc<RwLock<Settings>> {
        &self.settings
    }

    // ---------- 热键入口（hotkey 模块调用；ADR-006 单键语义表 + 策略路由） ----------

    /// 当前配置推导的交互策略（每次事件现场组装：settings 热更新即生效，无需失效处理）
    fn policy(&self) -> InteractionPolicy {
        InteractionPolicy::from_settings(&self.settings.read().unwrap())
    }

    /// toggle 触发键（A2 点按事件）按状态路由：
    /// Idle 开始 / Listening 结束（B2；B3 接入后兜底强制结束；solo 连续模式 = 停止会话，
    /// 不发送在途段）/ Preview 确认发送（C2 的主交互，全程不碰鼠标不抢焦点）/ 其余状态中止。
    pub async fn on_hotkey_toggle(&self) {
        match self.state() {
            OrchestratorState::Idle => {
                let _ = self.begin().await;
            }
            OrchestratorState::Listening => {
                // B1（松手结束）策略下不会产生 toggle 事件；B2/B3 均为按下结束
                let policy = self.policy();
                if policy.continuous {
                    // solo：再点按 = 停止会话（发送完成不停机由 schedule_idle 负责；
                    // 这里丢弃在途段——用户的意图是「停」，不是「把这句发出去」）
                    self.cancel().await;
                } else if policy.end != EndTrigger::HotkeyRelease {
                    let _ = self.end().await;
                }
            }
            OrchestratorState::Preview => {
                let _ = self.confirm_send().await;
            }
            // Success / 无文本 Error 的 toast 驻留期：点按 = 立即开始下一句
            // （先 cancel 清理 toast 再 begin，「第一次按没反应」修复）。
            // 带文本的 Error（发送失败待重试）保持只取消，不吞掉重试入口。
            OrchestratorState::Success => {
                self.cancel().await;
                let _ = self.begin().await;
            }
            OrchestratorState::Error
                if self.inner.lock().unwrap().preview_text.is_none() =>
            {
                self.cancel().await;
                let _ = self.begin().await;
            }
            _ => self.cancel().await,
        }
    }

    /// hold 触发键（A1）：按下开始。Idle 直接 begin；Success / 无文本 Error 的
    /// toast 驻留期内按下 = 立即开始下一句（先 cancel 再 begin，「第一次按没反应」
    /// 修复）；其余非 Idle 态按下忽略（Preview / 带文本 Error 保留确认与重试入口，
    /// 不让 begin 失败的 Error toast 冲掉预览文本）。松开在 Listening 态结束（B1）。
    pub async fn on_hotkey_hold(&self, pressed: bool) {
        if pressed {
            match self.state() {
                OrchestratorState::Idle => {
                    let _ = self.begin().await;
                }
                OrchestratorState::Success => {
                    self.cancel().await;
                    let _ = self.begin().await;
                }
                OrchestratorState::Error
                    if self.inner.lock().unwrap().preview_text.is_none() =>
                {
                    self.cancel().await;
                    let _ = self.begin().await;
                }
                _ => {}
            }
        } else if self.policy().end == EndTrigger::HotkeyRelease
            && self.state() == OrchestratorState::Listening
        {
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
                "引擎「{}」未就绪（模型未下载），请到「设置 → 高级」下载模型",
                engine.display_name()
            ));
        }

        let active_profile = settings
            .active_profile_id
            .as_deref()
            .and_then(profile::get)
            .unwrap_or_else(GameProfile::builtin_generic);
        let cfg = build_session_config(&settings, &engine_id, &active_profile);

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

        // eval 录档：evalRecording 开启时创建录档句柄，pump 边录边喂；
        // finalize 成功后落盘，取消则随会话句柄整体丢弃（docs/adr/005）。
        // history 开启时 sessionId 生成一次同时喂给录档与历史草稿，两边可互查；
        // history.mode == off 零开销：不建草稿、不生成 id、录档自生成 id。
        let history_on = settings.history.mode != crate::history::HistoryMode::Off;
        let session_id = history_on.then(crate::eval::new_session_id);
        let recorder = settings.eval_recording.then(|| {
            let dir = self.eval_dir.clone().unwrap_or_else(crate::eval::eval_dir);
            match &session_id {
                Some(id) => crate::eval::SessionRecorder::new_with_id(dir, &engine_id, id.clone()),
                None => crate::eval::SessionRecorder::new_in(dir, &engine_id),
            }
        });
        let history_draft = session_id.map(|sid| {
            Arc::new(Mutex::new(HistoryDraft {
                session_id: sid,
                started: std::time::Instant::now(),
                engine_id: engine_id.clone(),
                profile_id: settings.active_profile_id.clone(),
                audio_samples: 0,
                first_partial_ms: None,
                final_text: String::new(),
                finalize_latency_ms: None,
                reported: false,
            }))
        });

        // B3（VAD 静音判停，ADR-007）：策略要求 VAD 时每会话建一个实例。
        // 未接入工厂 / 模型未就绪 → begin 直接失败（清晰错误，不开半截会话）
        let use_vad = self.policy().end == EndTrigger::VadSilence;
        let vad_state = if use_vad {
            let factory = self.vad_factory.clone().ok_or_else(|| {
                "one-shot / solo 模式需要 VAD，但当前构建未接入（vad-silero feature 未编译）"
                    .to_string()
            })?;
            let vad = factory()?;
            let silence_ms = settings
                .vad_silence_ms
                .clamp(*crate::vad::SILENCE_MS_RANGE.start(), *crate::vad::SILENCE_MS_RANGE.end());
            Some((
                vad,
                crate::vad::FrameSplitter::new(),
                crate::vad::SilenceStopTracker::new(silence_ms, crate::vad::MIN_SESSION_MS),
            ))
        } else {
            None
        };

        // PCM pump：录音 → session.push_audio（VAD 策略下并行喂 VAD）；
        // STT partial → kotone://partial；VAD 判停 → 触发 end()（与热键结束同路径）
        let emitter = self.emitter.clone();
        let pump_flag = cancelled_flag.clone();
        let pump_recorder = recorder.clone();
        let pump_history = history_draft.clone();
        let me_weak = self.me.read().unwrap().clone();
        let pump = tokio::spawn(async move {
            let mut session = session;
            let mut stop_rx = stop_rx;
            let mut pcm_rx = pcm_rx;
            let mut stt_rx = stt_rx;
            let mut vad_state = vad_state;
            // 松手收尾：end() 先停采集再发停止信号，pump 收到信号后把通道里
            // 已缓冲未消费的 PCM 全部灌进 session（松手丢字 P0：采音侧排空）；
            // 取消（pump_flag）则立即停，音频丢弃
            loop {
                tokio::select! {
                    biased;
                    r = &mut stop_rx => {
                        let _ = r;
                        if !pump_flag.load(Ordering::SeqCst) {
                            // 非阻塞排空：此时采集线程已被 join（end 先 drop
                            // guard），通道里剩下的就是全部缓冲
                            while let Ok(c) = pcm_rx.try_recv() {
                                if let Some(rec) = &pump_recorder {
                                    rec.push_pcm(&c);
                                }
                                if let Some(h) = &pump_history {
                                    h.lock().unwrap().audio_samples += c.len() as u64;
                                }
                                if session.push_audio(&c).is_err() { break; }
                            }
                        }
                        break;
                    }
                    chunk = pcm_rx.recv() => {
                        match chunk {
                            Some(c) => {
                                if let Some(rec) = &pump_recorder {
                                    rec.push_pcm(&c);
                                }
                                if let Some(h) = &pump_history {
                                    h.lock().unwrap().audio_samples += c.len() as u64;
                                }
                                if session.push_audio(&c).is_err() { break; }
                                // VAD 帧判定与 STT 并行（同一 PCM 流）；
                                // 推理失败只禁用判停（热键强制结束兜底），不断会话
                                if let Some((vad, splitter, tracker)) = &mut vad_state {
                                    let mut decision = crate::vad::StopDecision::Continue;
                                    let mut failed = None;
                                    for frame in splitter.push(&c) {
                                        match vad.push_frame(&frame) {
                                            Ok(speech) => {
                                                decision = tracker.push(speech);
                                                if decision == crate::vad::StopDecision::Stop {
                                                    break;
                                                }
                                            }
                                            Err(e) => {
                                                failed = Some(e);
                                                break;
                                            }
                                        }
                                    }
                                    if let Some(e) = failed {
                                        crate::log::log(&format!(
                                            "VAD 推理失败，本次会话禁用静音判停（热键仍可结束）: {e}"
                                        ));
                                        vad_state = None;
                                    } else if decision == crate::vad::StopDecision::Stop {
                                        emitter.emit(
                                            "kotone://vad-stop",
                                            json!({ "elapsedMs": tracker.elapsed_ms() }),
                                        );
                                        if let Some(me) =
                                            me_weak.as_ref().and_then(|w| w.upgrade())
                                        {
                                            tokio::spawn(async move {
                                                let _ = me.end().await;
                                            });
                                        }
                                        // 立刻交出 session：end() 正在等 session_rx
                                        break;
                                    }
                                }
                            }
                            None => break, // 采集结束
                        }
                    }
                    ev = stt_rx.recv() => {
                        match ev {
                            Some(SttEvent::Partial { text }) => {
                                if let Some(rec) = &pump_recorder {
                                    rec.push_partial(&text);
                                }
                                if let Some(h) = &pump_history {
                                    let mut g = h.lock().unwrap();
                                    if g.first_partial_ms.is_none() {
                                        g.first_partial_ms =
                                            Some(g.started.elapsed().as_millis() as u64);
                                    }
                                }
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
        // 目标窗口记忆：用户按下热键说话前所在的前台窗口 = 注入目标，
        // 发送前（do_send）会把焦点还给它，避免 preview 交互抢焦点导致注入打错窗口
        inner.target_window = self.focus.foreground_window();
        inner.history = history_draft;
        inner.active = Some(ActiveSession {
            stop_tx: Some(stop_tx),
            session_rx: Some(session_rx),
            cancelled_flag,
            guard: Some(handle),
            pump,
            level_task,
            recorder,
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

        // 停 pump 并取回 session（不持锁，允许 cancel 插队，gen 校验兜底）。
        // 顺序关键（松手丢字 P0）：先 drop guard 停采集——采集线程 join 后 pcm
        // 通道不再有新数据，pump 收到停止信号才能一次性排空通道里已缓冲的 PCM
        drop(active.guard.take());
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
        // 停止辅助任务
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
                // 空转录「无事发生」：不发送、不进预览、不落 history/eval 录档、
                // 不发 toast，状态直接回 Idle（空文本还会触发注入器敲两个回车，
                // openChatKey/sendKey 都是 Enter 时表现为莫名换行）
                if t.text.trim().is_empty() {
                    // 录档句柄直接 drop（不落盘）；history 草稿清掉不写记录
                    drop(active.recorder.take());
                    let continuous = self.policy().continuous;
                    // 块作用域收窄 MutexGuard（绝不跨 await 持锁）
                    {
                        let _op = self.op.lock().await;
                        let mut inner = self.inner.lock().unwrap();
                        if inner.gen != gen {
                            return Ok(());
                        }
                        inner.history = None;
                        inner.state = OrchestratorState::Idle;
                    }
                    self.emit_state(OrchestratorState::Idle, None);
                    // solo 连续模式：立即开下一段（同 schedule_idle 的 continuous 处理；
                    // begin 失败走自身 toast_error 收尾）
                    if continuous {
                        let _ = self.begin().await;
                    }
                    return Ok(());
                }
                // 最终文本上屏（替换 partial）
                self.emitter
                    .emit("kotone://partial", json!({ "text": t.text }));
                // history 草稿补齐 finalize 产物（终态落账时写入记录）
                if let Some(h) = &self.inner.lock().unwrap().history {
                    let mut g = h.lock().unwrap();
                    g.final_text = t.text.clone();
                    g.finalize_latency_ms = Some(t.latency_ms as u64);
                }
                // eval 录档落盘（wav + partial 时间线 + 指标）；失败静默记日志不阻断流程
                if let Some(rec) = active.recorder.take() {
                    match rec.finish(&t.text, t.latency_ms as u64) {
                        Ok(saved) => {
                            crate::log::log(&format!("eval 录档完成: {}", saved.session_id))
                        }
                        Err(e) => crate::log::log(&format!("eval 录档失败（忽略）: {e}")),
                    }
                }
                // PostFinalize 策略（ADR-006）：C1 直接发送 / C2 预览确认
                match self.policy().post {
                    PostFinalize::SendDirect => {
                        self.do_send(t.text, gen).await;
                    }
                    PostFinalize::PreviewConfirm => {
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
            }
            Err(e) => {
                self.fail(gen, &e, None);
                return Err(e);
            }
        }
        Ok(())
    }

    /// Preview 状态下用户确认发送；Error 状态（payload 带文本）也接受，
    /// 重新进入 Sending 实现重试（docs/development.md §4.1「Error 保留文本可重试」）。
    /// ADR-006 预览只读化：不再接受编辑文本，一律发送已定的 preview_text。
    pub async fn confirm_send(&self) -> Result<(), String> {
        let (gen, final_text) = {
            let _op = self.op.lock().await;
            let inner = self.inner.lock().unwrap();
            match inner.state {
                OrchestratorState::Preview | OrchestratorState::Error => {}
                s => return Err(format!("当前状态 {s:?} 不能确认发送")),
            }
            let t = inner
                .preview_text
                .clone()
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
        // error 已落账过的草稿跳过（Error 后的 Esc 是清理动作，不双记 cancelled）
        self.write_history(crate::history::HistoryOutcome::Cancelled, None);
    }

    // ---------- 内部 ----------

    /// Sending → Success/Error（§6 发送时序下半段；inject 实现负责按键细节）
    async fn do_send(&self, text: String, gen: u64) {
        let (profile, token, target) = {
            let _op = self.op.lock().await;
            let mut inner = self.inner.lock().unwrap();
            if inner.gen != gen {
                return;
            }
            inner.state = OrchestratorState::Sending;
            let token = CancelToken::default();
            inner.send_cancel = Some(token.clone());
            let target = inner.target_window;
            drop(inner);
            self.emit_state(OrchestratorState::Sending, Some(json!({ "text": text })));
            let settings = self.settings.read().unwrap().clone();
            let profile = settings
                .active_profile_id
                .as_deref()
                .and_then(profile::get)
                .unwrap_or_else(GameProfile::builtin_generic);
            (profile, token, target)
        };

        // 焦点恢复：preview 交互（点击悬浮条/热键确认）可能已把焦点带离目标窗口，
        // 先把焦点还给 begin 时记录的注入目标，再交由注入器注入。
        // 恢复失败（窗口已关闭）不致命：前台守卫已移除，仍直发当前前台窗口。
        if let Some(t) = target {
            let focus = self.focus.clone();
            let restored = tokio::task::spawn_blocking(move || focus.restore(t))
                .await
                .unwrap_or(false);
            if restored && self.focus_restore_delay > Duration::ZERO {
                tokio::time::sleep(self.focus_restore_delay).await;
            }
        }

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
        // history 终态（锁外落账，见下方 write_history 调用；三个分支各自赋值一次）
        let history_write: (crate::history::HistoryOutcome, Option<String>);
        match result {
            Ok(Ok(())) => {
                inner.preview_text = None;
                inner.state = OrchestratorState::Success;
                drop(inner);
                self.emit_state(OrchestratorState::Success, Some(json!({ "text": text })));
                history_write = (crate::history::HistoryOutcome::Sent, None);
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
                history_write = (crate::history::HistoryOutcome::Error, Some(e.message));
            }
            Err(_) => {
                inner.preview_text = Some(text.clone());
                inner.state = OrchestratorState::Error;
                drop(inner);
                self.emit_state(
                    OrchestratorState::Error,
                    Some(json!({ "message": "发送线程异常", "text": text })),
                );
                history_write = (
                    crate::history::HistoryOutcome::Error,
                    Some("发送线程异常".to_string()),
                );
            }
        }
        drop(_op);
        self.write_history(history_write.0, history_write.1);
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
        self.write_history(crate::history::HistoryOutcome::Error, Some(message.to_string()));
        self.schedule_idle(gen);
    }

    /// history 终态落账：取草稿快照 → 组记录 → 追加到 history.jsonl。
    /// - mode=off / 无草稿（begin 失败等）→ 直接返回（off 零开销在 begin 已保证，这里兜底热更新）；
    /// - error：落账但保留草稿（Error 态可重试，重试成功同 sessionId 再写一条 sent，
    ///   两条记录是刻意的「失败→重试成功」叙事）；
    /// - cancelled：草稿已被 error 落账过则跳过（Error 后的 Esc 是清理动作，不双记）；
    /// - sent / cancelled：落账并终结草稿；
    /// - IO 失败静默记日志，绝不炸主流程。
    fn write_history(&self, outcome: crate::history::HistoryOutcome, error: Option<String>) {
        let draft = match self.inner.lock().unwrap().history.clone() {
            Some(d) => d,
            None => return,
        };
        let cfg = self.settings.read().unwrap().history.clone();
        if cfg.mode == crate::history::HistoryMode::Off {
            return;
        }
        let mut record = {
            let mut g = draft.lock().unwrap();
            if outcome == crate::history::HistoryOutcome::Cancelled && g.reported {
                return;
            }
            if outcome != crate::history::HistoryOutcome::Cancelled {
                g.reported = true;
            }
            crate::history::HistoryRecord {
                session_id: g.session_id.clone(),
                ts: crate::eval::utc_now_iso(),
                engine_id: g.engine_id.clone(),
                profile_id: g.profile_id.clone(),
                final_text: g.final_text.clone(),
                audio_ms: g.audio_samples * 1000 / crate::eval::SAMPLE_RATE as u64,
                first_partial_ms: g.first_partial_ms,
                finalize_latency_ms: g.finalize_latency_ms,
                outcome,
                error,
                audio_file: None,
            }
        };
        let dir = self
            .history_dir
            .clone()
            .unwrap_or_else(crate::history::history_dir);
        if cfg.include_audio {
            // 音频来自 eval 录档（cancel 路径录档随会话丢弃，无 wav 可复制 → None）
            let eval_dir = self.eval_dir.clone().unwrap_or_else(crate::eval::eval_dir);
            record.audio_file =
                crate::history::copy_audio_in(&dir, &eval_dir, &record.session_id);
        }
        if let Err(e) = crate::history::append_in(&dir, &record, &cfg) {
            crate::log::log(&format!("history 落账失败（忽略）: {e}"));
        }
        if outcome != crate::history::HistoryOutcome::Error {
            self.inner.lock().unwrap().history = None;
        }
    }

    /// toast_dwell 后自动回 Idle（期间有新会话/取消则不动作）。
    /// 带文本的 Error 不自动回 Idle：保留待重试文本，等用户重试或取消（§4.1）。
    /// solo 连续模式：发送成功（Success）不停机——置回 Idle 后立即 begin 下一段，
    /// 状态经 Success 回到 Listening 而非 stopped；begin 失败走 Error toast 自行收尾。
    fn schedule_idle(&self, gen: u64) {
        let dwell = self.toast_dwell;
        let inner = self.inner.clone();
        let emitter = self.emitter.clone();
        let settings = self.settings.clone();
        let me = self.me.read().unwrap().clone();
        tokio::spawn(async move {
            tokio::time::sleep(dwell).await;
            let continuous =
                InteractionPolicy::from_settings(&settings.read().unwrap()).continuous;
            // 块作用域收窄 MutexGuard（绝不跨 await 持锁）
            let resume = {
                let mut g = inner.lock().unwrap();
                let success = g.state == OrchestratorState::Success;
                let should_idle = g.gen == gen
                    && (success
                        || (g.state == OrchestratorState::Error && g.preview_text.is_none()));
                if should_idle {
                    g.state = OrchestratorState::Idle;
                    Some(success)
                } else {
                    None
                }
            };
            let success = match resume {
                Some(s) => s,
                None => return,
            };
            if continuous && success {
                // solo：立即开下一段（每个 VAD 切分段 = 完整会话周期，
                // 采音/会话/录档/history 全部按段独立，甩尾修复逻辑段内生效）
                if let Some(orch) = me.as_ref().and_then(|w| w.upgrade()) {
                    let _ = orch.begin().await;
                    // begin 成功已 emit Listening；失败走 toast_error → 自行回 Idle
                    return;
                }
            }
            emitter.emit(
                "kotone://state",
                json!({ "state": OrchestratorState::Idle, "payload": null }),
            );
        });
    }

    fn emit_state(&self, state: OrchestratorState, payload: Option<serde_json::Value>) {
        self.emitter.emit(
            "kotone://state",
            json!({ "state": state, "payload": payload }),
        );
    }
}

/// 会话配置组装（纯函数，可单测）：profile 热词 → SessionConfig.hotwords，
/// 引擎专有配置取 engineOptions[engine_id]（缺省 Null）
fn build_session_config(
    settings: &Settings,
    engine_id: &str,
    active_profile: &GameProfile,
) -> SessionConfig {
    SessionConfig {
        language: settings.language.clone(),
        hotwords: active_profile.hotwords.clone(),
        options: settings
            .engine_options
            .get(engine_id)
            .cloned()
            .unwrap_or(serde_json::Value::Null),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 热词端到端链路的 core 段：profile.hotwords 原样进 SessionConfig
    /// （stt 段见 online_transducer 的 format_hotwords 测试）
    #[test]
    fn session_config_carries_profile_hotwords() {
        let settings = Settings::default();
        let profile = GameProfile::builtin_lol();
        let cfg = build_session_config(&settings, "sherpa-onnx-x-asr-zh-en", &profile);
        assert_eq!(cfg.hotwords, profile.hotwords);
        assert!(cfg.hotwords.contains(&"打野".to_string()));
        assert_eq!(cfg.language, "zh");
    }

    #[test]
    fn session_config_options_fallback_null() {
        let mut settings = Settings::default();
        settings.engine_options["sherpa-onnx-x-asr-zh-en"]["provider"] =
            serde_json::json!("cpu");
        let generic = GameProfile::builtin_generic();
        let cfg = build_session_config(&settings, "sherpa-onnx-x-asr-zh-en", &generic);
        assert_eq!(cfg.options["provider"], "cpu");
        assert!(cfg.hotwords.is_empty(), "generic 无热词");
        // 未配置的引擎 → Null
        let cfg2 = build_session_config(&settings, "mock-stream", &generic);
        assert!(cfg2.options.is_null());
    }
}
