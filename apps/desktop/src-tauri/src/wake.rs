//! 语音唤醒（ADR 017）：常开一路麦克风，在本机检测唤醒词；命中后接一声，再用与陪玩热键
//! 相同的助手会话收指令（识别、悬浮窗、交给琴音都不变），只是由唤醒开始、说完即提交。
//!
//! - **一路麦克风**：唤醒线程用现有的 `CpalBackend` 开流并持续消费。唤醒后的那次会话
//!   不再另开一路，而是经 [`WakeAudioBackend`] 从这里转发（先补上唤醒词之后已经说的话），
//!   所以“琴音琴音帮我看看……”连着说也不丢开头。热键会话照旧自己开流（WASAPI 共享模式）。
//! - **互斥**：任何会话进行中（游戏语音输入、陪玩热键、预览、发送）都不判唤醒。
//! - **应答**：命中后看 300 ms，还在说话就不应答、直接收音；否则说一声“嗯？/我在”，
//!   说完再收音（不把应答录进去）。
//! - 只在本机处理；音频不落盘。设置 `assistant.wake.enabled` 与语音运行时同时开着才监听。

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use kotone_core::assistant::{WakeAck, WakeConfig};
use kotone_core::audio::{AudioBackend, AudioHandle};
use kotone_core::log;
use kotone_core::orchestrator::OrchestratorState;
use serde_json::json;
use tauri::{AppHandle, Emitter, Manager};

use crate::SharedState;

/// 唤醒后看多久来判断“是不是连着在说”。
const CONTINUE_PROBE: Duration = Duration::from_millis(300);
/// 一次唤醒会话结束后，多久内不再判唤醒（避免会话尾音再次触发）。
const COOLDOWN: Duration = Duration::from_millis(1_200);
/// 转发给会话的队列上限（50 ms 一块 → 约 6 秒）。
const HANDOFF_CAPACITY: usize = 120;
/// 估计环境底噪用最近多少块（50 ms 一块）。
const NOISE_WINDOW: usize = 40;
const ACKS: &[&str] = &["嗯？", "我在"];

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase", tag = "state")]
pub enum WakeState {
    /// 设置里没开。
    Off,
    /// 开了，但语音识别没在运行（唤醒依赖语音识别）。
    WaitingRuntime,
    /// 开了，但唤醒模型还没下载。
    ModelMissing,
    Listening,
    Error {
        message: String,
    },
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WakeStatus {
    #[serde(flatten)]
    pub state: WakeState,
    pub model_ready: bool,
    pub downloading: bool,
    /// 本次启动以来唤醒了几次（诊断误唤醒用）。
    pub detections: u64,
}

struct Worker {
    stop: Arc<AtomicBool>,
    config: WakeConfig,
    device: String,
    /// 正在用的关键词行（当前角色的唤醒词）。
    keywords: String,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

#[derive(Default)]
pub struct WakeRuntime {
    worker: Mutex<Option<Worker>>,
    state: Mutex<Option<WakeState>>,
    downloading: AtomicBool,
    detections: std::sync::atomic::AtomicU64,
    /// 唤醒线程交给下一次会话的音频（由 [`WakeAudioBackend::start`] 取走）。
    handoff: Arc<Mutex<Option<Receiver<Vec<f32>>>>>,
}

impl WakeRuntime {
    pub fn handoff(&self) -> Arc<Mutex<Option<Receiver<Vec<f32>>>>> {
        self.handoff.clone()
    }
}

/// 录音会话用的音频后端：有唤醒交接时用唤醒线程转来的音频，否则照常开麦克风。
pub struct WakeAudioBackend {
    pub inner: Arc<dyn AudioBackend>,
    pub handoff: Arc<Mutex<Option<Receiver<Vec<f32>>>>>,
}

impl AudioBackend for WakeAudioBackend {
    fn start(&self, device_id: &str) -> Result<AudioHandle, String> {
        let Some(rx) = self.handoff.lock().unwrap().take() else {
            return self.inner.start(device_id);
        };
        let (pcm_tx, pcm_rx) = tokio::sync::mpsc::unbounded_channel::<Vec<f32>>();
        let (level_tx, level_rx) = tokio::sync::mpsc::unbounded_channel::<f32>();
        let (_err_tx, err_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let thread = std::thread::Builder::new()
            .name("kotone-wake-handoff".into())
            .spawn(move || {
                // 停止（会话结束）或唤醒线程关掉发送端时结束；结束即关闭 pcm 通道。
                let _err_tx = _err_tx;
                let mut forwarded = 0usize;
                loop {
                    if stop_rx.try_recv().is_ok() {
                        break;
                    }
                    match rx.recv_timeout(Duration::from_millis(100)) {
                        Ok(chunk) => {
                            forwarded += 1;
                            let _ = level_tx.send(rms(&chunk));
                            if pcm_tx.send(chunk).is_err() {
                                break;
                            }
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => continue,
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                }
                log::log(&format!("wake: session audio forwarded {forwarded} chunks"));
            })
            .map_err(|e| format!("无法启动唤醒音频转发线程：{e}"))?;
        Ok(AudioHandle::with_thread(
            pcm_rx, level_rx, err_rx, stop_tx, thread,
        ))
    }
}

fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
}

/// 交给识别前垫多少块静音。
const LEAD_SILENCE_CHUNKS: usize = 6;
/// 开始唤醒会话最多等多久。
const SESSION_START_LIMIT: Duration = Duration::from_secs(10);

/// 应答期间连续多少块（50 ms 一块）明显有声算玩家开口。
const BARGE_IN_CHUNKS: usize = 3;
/// 应答期间最多缓存多久的音频（块数）。
const ACK_BUFFER: usize = 60;
/// 找到话头后再往前留多少块（轻声开头的字不被截掉）。
const PRE_ROLL_CHUNKS: usize = 6;
/// 连续多少块安静算“话还没开始”。
const QUIET_GAP_CHUNKS: usize = 4;

/// 确认玩家开口后，在缓冲里往回找这段话的开头：最后一段连续安静之后，再往前留一点
/// （纯函数）。找不到安静段就从缓冲开头算。
fn speech_start(levels: &[f32], noise_floor: f32) -> usize {
    // 话头的声音常常比话中间轻：用一半的阈值判断“安静”
    let quiet = speech_threshold(noise_floor) * 0.5;
    let mut run = 0usize;
    for i in (0..levels.len()).rev() {
        if levels[i] <= quiet {
            run += 1;
            if run >= QUIET_GAP_CHUNKS {
                let first_sound = i + run;
                return first_sound.saturating_sub(PRE_ROLL_CHUNKS);
            }
        } else {
            run = 0;
        }
    }
    0
}

fn speech_threshold(noise_floor: f32) -> f32 {
    (noise_floor * 2.5).max(0.012)
}

/// 底噪之上多少算“还在说话”（纯函数，便于测试）。
fn still_speaking(probe: &[f32], noise_floor: f32) -> bool {
    let threshold = speech_threshold(noise_floor);
    let loud = probe.iter().filter(|&&r| r > threshold).count();
    // 300 ms 里至少一半的块明显高于底噪
    !probe.is_empty() && loud * 2 >= probe.len()
}

fn median(values: &VecDeque<f32>) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    let mut v: Vec<f32> = values.iter().copied().collect();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    v[v.len() / 2]
}

fn runtime(app: &AppHandle) -> Option<tauri::State<'_, WakeRuntime>> {
    app.try_state::<WakeRuntime>()
}

fn set_state(app: &AppHandle, state: WakeState) {
    if let Some(rt) = runtime(app) {
        let changed = rt.state.lock().unwrap().replace(state.clone()) != Some(state);
        if changed {
            let _ = app.emit("kotone://wake-status", status(app));
        }
    }
}

pub fn status(app: &AppHandle) -> WakeStatus {
    let rt = runtime(app);
    let enabled = app
        .state::<SharedState>()
        .settings
        .read()
        .unwrap()
        .assistant
        .wake
        .enabled;
    let state = rt
        .as_ref()
        .and_then(|rt| rt.state.lock().unwrap().clone())
        .unwrap_or(if enabled {
            WakeState::WaitingRuntime
        } else {
            WakeState::Off
        });
    WakeStatus {
        state,
        model_ready: kotone_stt::wake::kws_model_ready(),
        downloading: rt
            .as_ref()
            .is_some_and(|rt| rt.downloading.load(Ordering::SeqCst)),
        detections: rt
            .as_ref()
            .map(|rt| rt.detections.load(Ordering::SeqCst))
            .unwrap_or(0),
    }
}

fn voice_running(app: &AppHandle) -> bool {
    app.try_state::<crate::runtime::RuntimeManager>()
        .is_some_and(|rt| rt.phase() == kotone_core::runtime::RuntimePhase::Running)
}

/// 按设置与语音运行时状态启停唤醒监听。设置保存、语音识别启停后调用；幂等。
pub fn sync(app: &AppHandle) {
    let Some(rt) = runtime(app) else { return };
    let settings = app.state::<SharedState>().settings.read().unwrap().clone();
    let config = settings.assistant.wake.clone();
    let device = settings.audio_device_id.clone();
    let mut worker = rt.worker.lock().unwrap();
    let wanted = config.enabled && voice_running(app) && kotone_stt::wake::kws_model_ready();
    // 唤醒词跟着当前角色（换角色、改唤醒词都要重新开始监听）。
    let (keywords, keyword_label, keyword_warning) = crate::roles::wake_keywords(app);
    if let Some(w) = worker.as_ref() {
        if wanted && w.config == config && w.device == device && w.keywords == keywords {
            return;
        }
    }
    if let Some(warning) = &keyword_warning {
        log::log(&format!("wake: {warning}"));
    }
    // 先停旧的，再按需开新的。旧线程在后台退出（它可能正等着开始一次会话），
    // 调用方不必等它；两路共享模式的采集短暂重叠没有问题。
    if let Some(old) = worker.take() {
        old.stop.store(true, Ordering::SeqCst);
        std::thread::spawn(move || drop(old));
    }
    rt.handoff.lock().unwrap().take();
    if !config.enabled {
        drop(worker);
        set_state(app, WakeState::Off);
        return;
    }
    if !kotone_stt::wake::kws_model_ready() {
        drop(worker);
        set_state(app, WakeState::ModelMissing);
        return;
    }
    if !voice_running(app) {
        drop(worker);
        set_state(app, WakeState::WaitingRuntime);
        return;
    }
    let stop = Arc::new(AtomicBool::new(false));
    let thread = {
        let app = app.clone();
        let stop = stop.clone();
        let config = config.clone();
        let device = device.clone();
        let keywords = keywords.clone();
        std::thread::Builder::new()
            .name("kotone-wake".into())
            .spawn(move || {
                if let Err(error) = listen(&app, &config, &keywords, &keyword_label, &device, &stop)
                {
                    log::log(&format!("wake: stopped with error: {error}"));
                    set_state(&app, WakeState::Error { message: error });
                }
            })
    };
    match thread {
        Ok(thread) => {
            *worker = Some(Worker {
                stop,
                config,
                device,
                keywords,
                thread: Some(thread),
            });
        }
        Err(e) => {
            drop(worker);
            set_state(
                app,
                WakeState::Error {
                    message: format!("无法启动唤醒线程：{e}"),
                },
            );
        }
    }
}

/// 停止监听（应用退出）。
pub fn shutdown(app: &AppHandle) {
    if let Some(rt) = runtime(app) {
        drop(rt.worker.lock().unwrap().take());
        rt.handoff.lock().unwrap().take();
    }
}

enum Phase {
    Listening,
    /// 命中后观察 300 ms：收集这段音频，判断是否连着在说。
    Probing {
        until: Instant,
        chunks: Vec<Vec<f32>>,
    },
    /// 正在说应答；说完开始收音。期间也在听：玩家开口就停掉应答，从开口处开始收音。
    Acking {
        done: Receiver<()>,
        buffer: VecDeque<Vec<f32>>,
        loud: usize,
        floor: f32,
    },
    /// 正在开始唤醒会话（在后台进行）：这期间照常读麦克风，音频继续交给即将开始的会话。
    /// 不在这个线程上等：采集端只能缓存约 2 秒，等久了会报“处理不过来”并让唤醒停掉。
    Starting {
        tx: SyncSender<Vec<f32>>,
        done: Receiver<Result<(), String>>,
        started: Instant,
        prefilled: usize,
    },
    /// 音频转给唤醒会话；会话结束（接收端关闭）后回到监听。
    Handoff {
        tx: SyncSender<Vec<f32>>,
    },
    Cooldown {
        until: Instant,
    },
}

#[cfg(feature = "engine-sherpa")]
fn listen(
    app: &AppHandle,
    config: &WakeConfig,
    keywords: &str,
    phrase: &str,
    device: &str,
    stop: &AtomicBool,
) -> Result<(), String> {
    let sensitivity = serde_json::to_value(config.sensitivity)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default();
    let mut detector = kotone_stt::wake::WakeDetector::new(
        &kotone_stt::wake::kws_model_dir(),
        keywords,
        &sensitivity,
    )?;
    let backend = kotone_platform_windows::audio::CpalBackend;
    let mut handle = backend.start(device)?;
    let mut pcm = handle.pcm_rx.take().ok_or("麦克风通道不可用")?;
    let mut errors = handle.error_rx.take();
    log::log(&format!(
        "wake: listening (phrase={phrase}, sensitivity={sensitivity}, device={device})"
    ));
    set_state(app, WakeState::Listening);
    if config.ack == WakeAck::Voice {
        // 应答先合成好：在线语音现场合成要一两秒，玩家会觉得她反应慢。
        let prefetch_app = app.clone();
        std::thread::spawn(move || crate::assistant::prefetch(&prefetch_app, ACKS));
    }
    let orchestrator = app.state::<SharedState>().orchestrator.clone();
    let rt = runtime(app).ok_or("唤醒状态不可用")?;
    let handoff = rt.handoff();
    let mut noise: VecDeque<f32> = VecDeque::with_capacity(NOISE_WINDOW);
    let mut phase = Phase::Listening;
    let mut beat = Instant::now();
    let mut chunks_seen = 0u64;
    let mut ack_index = 0usize;
    while !stop.load(Ordering::SeqCst) {
        if let Some(rx) = errors.as_mut() {
            if let Ok(message) = rx.try_recv() {
                return Err(format!("麦克风出错：{message}"));
            }
        }
        // 50 ms 一块；没有数据时短暂等待再检查停止标记
        let chunk = match pcm.try_recv() {
            Ok(chunk) => chunk,
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                std::thread::sleep(Duration::from_millis(10));
                continue;
            }
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                return Err("麦克风已关闭".into());
            }
        };
        let level = rms(&chunk);
        chunks_seen += 1;
        if std::env::var_os("KOTONE_WAKE_DEBUG").is_some()
            && beat.elapsed() >= Duration::from_secs(5)
        {
            beat = Instant::now();
            let name = match &phase {
                Phase::Listening => "listening",
                Phase::Probing { .. } => "probing",
                Phase::Acking { .. } => "acking",
                Phase::Starting { .. } => "starting",
                Phase::Handoff { .. } => "handoff",
                Phase::Cooldown { .. } => "cooldown",
            };
            log::log(&format!(
                "wake debug: phase={name} orchestrator={:?} chunks={chunks_seen} level={level:.4}",
                orchestrator.state()
            ));
        }
        phase = match phase {
            Phase::Listening => {
                if noise.len() == NOISE_WINDOW {
                    noise.pop_front();
                }
                noise.push_back(level);
                if !matches!(
                    orchestrator.state(),
                    OrchestratorState::Idle | OrchestratorState::Success | OrchestratorState::Error
                ) {
                    // 有会话在进行：不判唤醒，也不让会话里的话残留在检测器里
                    detector.reset();
                    Phase::Listening
                } else if let Some(keyword) = detector.accept(&chunk) {
                    let n = rt.detections.fetch_add(1, Ordering::SeqCst) + 1;
                    log::log(&format!("wake: detected {keyword} (#{n})"));
                    let _ = app.emit("kotone://wake", json!({"keyword": keyword}));
                    crate::bubble::woke(app);
                    Phase::Probing {
                        until: Instant::now() + CONTINUE_PROBE,
                        chunks: Vec::new(),
                    }
                } else {
                    Phase::Listening
                }
            }
            Phase::Probing { until, mut chunks } => {
                chunks.push(chunk);
                if Instant::now() < until {
                    Phase::Probing { until, chunks }
                } else {
                    let levels: Vec<f32> = chunks.iter().map(|c| rms(c)).collect();
                    let continuing = still_speaking(&levels, median(&noise));
                    if continuing || config.ack != WakeAck::Voice {
                        begin_session(app, &handoff, chunks, config)
                    } else {
                        let text = ACKS[ack_index % ACKS.len()].to_string();
                        log::log(&format!("wake: answering {text}"));
                        ack_index += 1;
                        let (done_tx, done_rx) = mpsc::channel();
                        let ack_app = app.clone();
                        std::thread::spawn(move || {
                            match crate::assistant::speak_now(&ack_app, &text) {
                                Ok(()) => {}
                                Err(e) if e.contains("取消") => {
                                    log::log("wake: answer stopped because the player spoke")
                                }
                                Err(e) => log::log(&format!("wake: answer failed: {e}")),
                            }
                            let _ = done_tx.send(());
                        });
                        Phase::Acking {
                            done: done_rx,
                            // 判断窗口里的音频也留着：玩家停顿很短就接着说时，开头在这里
                            buffer: chunks.into_iter().collect(),
                            // 窗口末尾已经开口的块也算进去
                            loud: levels
                                .iter()
                                .rev()
                                .take_while(|&&r| r > speech_threshold(median(&noise)))
                                .count(),
                            floor: median(&noise),
                        }
                    }
                }
            }
            Phase::Acking {
                done,
                mut buffer,
                loud,
                floor,
            } => {
                let loud = if level > speech_threshold(floor) {
                    loud + 1
                } else {
                    0
                };
                if buffer.len() == ACK_BUFFER {
                    buffer.pop_front();
                }
                buffer.push_back(chunk);
                if loud >= BARGE_IN_CHUNKS {
                    // 玩家没等应答说完就开口了：停掉应答，从开口前一点开始收音
                    log::log("wake: player spoke during the answer, listening now");
                    // 停播可能要等语音服务最多 2 秒：不在这个线程上等，免得漏收音频
                    let silence_app = app.clone();
                    std::thread::spawn(move || crate::assistant::silence(&silence_app));
                    let levels: Vec<f32> = buffer.iter().map(|c| rms(c)).collect();
                    let start = speech_start(&levels, floor);
                    let prefill: Vec<Vec<f32>> = buffer.drain(start..).collect();
                    begin_session(app, &handoff, prefill, config)
                } else {
                    match done.try_recv() {
                        // 应答说完：从现在开始收音，应答本身不进会话
                        Ok(()) | Err(TryRecvError::Disconnected) => {
                            let last = buffer.pop_back().into_iter().collect();
                            begin_session(app, &handoff, last, config)
                        }
                        Err(TryRecvError::Empty) => Phase::Acking {
                            done,
                            buffer,
                            loud,
                            floor,
                        },
                    }
                }
            }
            Phase::Starting {
                tx,
                done,
                started,
                prefilled,
            } => {
                // 会话还没开始也照收：它开始后会把这些一起读走
                let _ = tx.try_send(chunk);
                match done.try_recv() {
                    Ok(result) => session_started(&handoff, result, tx, started, prefilled),
                    Err(TryRecvError::Disconnected) => session_started(
                        &handoff,
                        Err("开始收音的任务意外结束".into()),
                        tx,
                        started,
                        prefilled,
                    ),
                    Err(TryRecvError::Empty) if started.elapsed() >= SESSION_START_LIMIT => {
                        session_started(
                            &handoff,
                            Err(format!(
                                "开始收音超过 {} 秒没有完成",
                                SESSION_START_LIMIT.as_secs()
                            )),
                            tx,
                            started,
                            prefilled,
                        )
                    }
                    Err(TryRecvError::Empty) => Phase::Starting {
                        tx,
                        done,
                        started,
                        prefilled,
                    },
                }
            }
            Phase::Handoff { tx } => match tx.try_send(chunk) {
                Ok(()) | Err(TrySendError::Full(_)) => Phase::Handoff { tx },
                Err(TrySendError::Disconnected(_)) => {
                    detector.reset();
                    Phase::Cooldown {
                        until: Instant::now() + COOLDOWN,
                    }
                }
            },
            Phase::Cooldown { until } => {
                if Instant::now() >= until {
                    detector.reset();
                    Phase::Listening
                } else {
                    Phase::Cooldown { until }
                }
            }
        };
    }
    drop(handle);
    Ok(())
}

#[cfg(not(feature = "engine-sherpa"))]
fn listen(
    _app: &AppHandle,
    _config: &WakeConfig,
    _keywords: &str,
    _phrase: &str,
    _device: &str,
    _stop: &AtomicBool,
) -> Result<(), String> {
    Err("这个版本没有编译语音唤醒".into())
}

/// 把已收集的音频交给新的唤醒会话并开始它；失败就回到冷却。
#[cfg(feature = "engine-sherpa")]
fn begin_session(
    app: &AppHandle,
    handoff: &Arc<Mutex<Option<Receiver<Vec<f32>>>>>,
    prefill: Vec<Vec<f32>>,
    config: &WakeConfig,
) -> Phase {
    let (tx, rx) = mpsc::sync_channel::<Vec<f32>>(HANDOFF_CAPACITY);
    let prefilled = prefill.len();
    // 先垫一小段静音：流式识别第一个字更稳（话头紧贴开头时会把“我”认成“或”）
    for _ in 0..LEAD_SILENCE_CHUNKS {
        let _ = tx.try_send(vec![0.0; kotone_core::audio::CHUNK_SAMPLES]);
    }
    for chunk in prefill {
        let _ = tx.try_send(chunk);
    }
    log::log(&format!(
        "wake: starting a session (prefill {prefilled} chunks)"
    ));
    *handoff.lock().unwrap() = Some(rx);
    let orchestrator = app.state::<SharedState>().orchestrator.clone();
    let timeout = config.no_speech_timeout_ms;
    let (done_tx, done) = mpsc::channel();
    tauri::async_runtime::spawn(async move {
        let _ = done_tx.send(orchestrator.begin_assistant_wake(timeout).await);
    });
    Phase::Starting {
        tx,
        done,
        started: Instant::now(),
        prefilled,
    }
}

/// 开始会话的结果（或超时）出来以后转到哪个阶段。
#[cfg(feature = "engine-sherpa")]
fn session_started(
    handoff: &Arc<Mutex<Option<Receiver<Vec<f32>>>>>,
    result: Result<(), String>,
    tx: SyncSender<Vec<f32>>,
    started: Instant,
    prefilled: usize,
) -> Phase {
    log::log(&format!(
        "wake: session start took {} ms (prefill {prefilled} chunks)",
        started.elapsed().as_millis()
    ));
    match result {
        Ok(()) if handoff.lock().unwrap().is_none() => Phase::Handoff { tx },
        Ok(()) => {
            // 会话没有用到唤醒音频（不应发生）：收回，正常监听
            handoff.lock().unwrap().take();
            Phase::Cooldown {
                until: Instant::now() + COOLDOWN,
            }
        }
        Err(error) => {
            handoff.lock().unwrap().take();
            log::log(&format!("wake: session did not start: {error}"));
            Phase::Cooldown {
                until: Instant::now() + COOLDOWN,
            }
        }
    }
}

/// 下载唤醒模型（约 33 MB 整包，只解出约 5.5 MB）。进度经 `kotone://wake-model` 推送。
#[tauri::command]
pub async fn wake_download_model(app: AppHandle) -> Result<WakeStatus, String> {
    download_model(&app).await?;
    Ok(status(&app))
}

pub async fn download_model(app: &AppHandle) -> Result<(), String> {
    let rt = runtime(app).ok_or("唤醒状态不可用")?;
    if rt.downloading.swap(true, Ordering::SeqCst) {
        return Err("唤醒模型正在下载".into());
    }
    let _ = app.emit("kotone://wake-status", status(app));
    let progress_app = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        // 进度最多每秒推送一次（与模型下载相同），完成时一定推送最后一次
        let last = Mutex::new(None::<Instant>);
        kotone_stt::wake::download_kws_model(&|done, total| {
            let mut last = last.lock().unwrap();
            let finished = total.is_some_and(|t| done >= t);
            if !finished && last.is_some_and(|t| t.elapsed() < Duration::from_secs(1)) {
                return;
            }
            *last = Some(Instant::now());
            let progress = json!({"downloadedBytes": done, "totalBytes": total});
            crate::tool_service::publish(&progress_app, "wake.model.progress", progress.clone());
            let _ = progress_app.emit("kotone://wake-model", progress);
        })
    })
    .await
    .map_err(|e| e.to_string())
    .and_then(|r| r);
    rt.downloading.store(false, Ordering::SeqCst);
    sync(app);
    let _ = app.emit("kotone://wake-status", status(app));
    result
}

#[tauri::command]
pub fn wake_status(app: AppHandle) -> WakeStatus {
    status(&app)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn continuing_speech_needs_most_of_the_probe_above_noise() {
        // 安静环境：底噪 0.002，说话 0.05
        assert!(still_speaking(&[0.05, 0.06, 0.04, 0.05, 0.05, 0.03], 0.002));
        assert!(!still_speaking(
            &[0.002, 0.003, 0.002, 0.004, 0.002, 0.002],
            0.002
        ));
        // 只有尾音一两块：不算连着说
        assert!(!still_speaking(
            &[0.05, 0.02, 0.003, 0.002, 0.002, 0.002],
            0.002
        ));
        // 嘈杂环境：底噪高，阈值跟着抬高
        assert!(!still_speaking(&[0.05, 0.05, 0.05, 0.05, 0.05, 0.05], 0.03));
        assert!(!still_speaking(&[], 0.0));
    }

    #[test]
    fn speech_start_backs_up_to_the_last_quiet_gap() {
        let q = 0.001; // 安静
        let s = 0.05; // 说话
        let soft = 0.008; // 轻声开头（低于说话阈值、高于一半阈值）
                          // 应答期间：安静 10 块，然后轻声开头 2 块，再正常说话 3 块；话头在下标 10
        let mut levels = vec![q; 10];
        levels.extend([soft, soft, s, s, s]);
        assert_eq!(speech_start(&levels, 0.0), 10 - PRE_ROLL_CHUNKS);
        // 一直有声（没有安静段）：从头开始
        assert_eq!(speech_start(&[s, s, soft, s], 0.0), 0);
        // 话头之后的短暂停顿（不足 4 块）不算新的话头
        let mut levels = vec![q; 10];
        levels.extend([s, q, q, s, s]);
        assert_eq!(speech_start(&levels, 0.0), 10 - PRE_ROLL_CHUNKS);
    }

    #[test]
    fn median_of_levels() {
        let v: VecDeque<f32> = [0.3, 0.1, 0.2].into_iter().collect();
        assert_eq!(median(&v), 0.2);
        assert_eq!(median(&VecDeque::new()), 0.0);
    }
}
