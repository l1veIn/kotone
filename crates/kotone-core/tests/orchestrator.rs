//! Orchestrator 全链路状态迁移集成测试：mock audio / mock inject / vec emitter。
//!
//! 注意：必须放 tests/（集成测试）而非 src 内 #[cfg(test)] 单元测试——
//! kotone-core 的 dev-dependency kotone-stt 反向依赖 kotone-core；
//! 单元测试会把 kotone-core 源码重编译为独立 test crate，
//! 与 kotone-stt 链接的 kotone-core rlib 是两个编译单元，
//! EngineRegistry 等类型互不兼容（E0308）。集成测试与 kotone-stt
//! 链接同一个 rlib，类型一致。

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use kotone_core::audio::{AudioBackend, AudioHandle};
use kotone_core::inject::{CancelToken, FocusBackend, InjectError, Injector, TargetWindow};
use kotone_core::orchestrator::{Emitter, Orchestrator, OrchestratorState};
use kotone_core::profile::GameProfile;
use kotone_core::settings::Settings;
use kotone_core::stt::EngineRegistry;
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

/// mock 焦点后端：记录 begin 捕获与发送前恢复调用；restore_ok 控制恢复成败
struct MockFocusBackend {
    /// 操作流水（"capture" / "restore:<hwnd>"），与 LoggingInjector 共享以断言顺序
    log: Arc<Mutex<Vec<String>>>,
    /// 模拟的前台窗口
    foreground: TargetWindow,
    restore_ok: bool,
}

impl MockFocusBackend {
    fn new(log: Arc<Mutex<Vec<String>>>, foreground: usize, restore_ok: bool) -> Self {
        Self {
            log,
            foreground: TargetWindow(foreground),
            restore_ok,
        }
    }
}

impl FocusBackend for MockFocusBackend {
    fn foreground_window(&self) -> Option<TargetWindow> {
        self.log.lock().unwrap().push("capture".into());
        Some(self.foreground)
    }
    fn restore(&self, target: TargetWindow) -> bool {
        self.log
            .lock()
            .unwrap()
            .push(format!("restore:{}", target.0));
        self.restore_ok
    }
}

/// 发送时同步记录操作流水的注入器（验证「先恢复焦点、后注入」顺序）
struct LoggingInjector {
    log: Arc<Mutex<Vec<String>>>,
}

impl Injector for LoggingInjector {
    fn send(
        &self,
        text: &str,
        _profile: &GameProfile,
        _cancel: CancelToken,
    ) -> Result<(), InjectError> {
        self.log.lock().unwrap().push(format!("send:{text}"));
        Ok(())
    }
}

/// finalize 返回固定文本的 stub 引擎（测空转录等边界；mock-stream 的文本是钉死的）
struct StubFinalEngine {
    final_text: &'static str,
}

impl kotone_core::stt::SttEngine for StubFinalEngine {
    fn id(&self) -> &'static str {
        "stub-final"
    }
    fn display_name(&self) -> &str {
        "Stub 固定文本引擎"
    }
    fn capabilities(&self) -> kotone_core::stt::EngineCapabilities {
        kotone_core::stt::EngineCapabilities {
            streaming: true,
            hotwords: false,
            gpu: false,
            offline: true,
            languages: vec!["zh".into()],
        }
    }
    fn is_ready(&self) -> bool {
        true
    }
    fn start_session(
        &self,
        _cfg: &kotone_core::stt::SessionConfig,
        _events: mpsc::UnboundedSender<kotone_core::stt::SttEvent>,
    ) -> Result<Box<dyn kotone_core::stt::SttSession>, String> {
        Ok(Box::new(StubFinalSession {
            final_text: self.final_text,
            cancelled: false,
        }))
    }
}

struct StubFinalSession {
    final_text: &'static str,
    cancelled: bool,
}

impl kotone_core::stt::SttSession for StubFinalSession {
    fn push_audio(&mut self, _pcm: &[f32]) -> Result<(), String> {
        if self.cancelled {
            return Err("会话已取消".into());
        }
        Ok(())
    }
    fn finalize(self: Box<Self>) -> Result<kotone_core::stt::Transcript, String> {
        if self.cancelled {
            return Err("会话已取消".into());
        }
        Ok(kotone_core::stt::Transcript {
            text: self.final_text.into(),
            latency_ms: 1,
        })
    }
    fn cancel(&mut self) {
        self.cancelled = true;
    }
}

/// 恒未就绪的占位引擎：模拟「引擎未就绪」路径（不能再用 sherpa 占位——
/// CLI 默认 feature 带 sherpa 后，workspace 构建下 sherpa 在真机是就绪的）
struct NeverReadyEngine;
impl kotone_core::stt::SttEngine for NeverReadyEngine {
    fn id(&self) -> &'static str {
        "never-ready"
    }
    fn display_name(&self) -> &str {
        "恒未就绪占位引擎"
    }
    fn capabilities(&self) -> kotone_core::stt::EngineCapabilities {
        kotone_core::stt::EngineCapabilities {
            streaming: false,
            hotwords: false,
            gpu: false,
            offline: true,
            languages: vec![],
        }
    }
    fn is_ready(&self) -> bool {
        false
    }
    fn start_session(
        &self,
        _cfg: &kotone_core::stt::SessionConfig,
        _events: mpsc::UnboundedSender<kotone_core::stt::SttEvent>,
    ) -> Result<Box<dyn kotone_core::stt::SttSession>, String> {
        unreachable!("恒未就绪引擎不会 start_session")
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
    let focus: Arc<dyn FocusBackend> = Arc::new(MockFocusBackend::new(
        Arc::new(Mutex::new(Vec::new())),
        42,
        true,
    ));
    make_orchestrator_full(auto_send, injector, focus)
}

fn make_orchestrator_full(
    auto_send: bool,
    injector: Arc<dyn Injector>,
    focus: Arc<dyn FocusBackend>,
) -> (Arc<Orchestrator>, Arc<VecEmitter>) {
    make_orchestrator_tuned(auto_send, injector, focus, |_| {})
}

/// 同 make_orchestrator_full，但允许在 into_arc 前调整超时等字段（测试用）
fn make_orchestrator_tuned(
    auto_send: bool,
    injector: Arc<dyn Injector>,
    focus: Arc<dyn FocusBackend>,
    tune: impl FnOnce(&mut Orchestrator),
) -> (Arc<Orchestrator>, Arc<VecEmitter>) {
    let mut settings = Settings::default();
    // 0.1.5 起默认交互模式为「对讲机」（hold 直发）；本测试族沿用旧的
    // toggle + autoSend 推导行为，显式置 None 走自定义兼容路径
    settings.interaction_mode = None;
    settings.stt_engine = "mock-stream".into();
    settings.auto_send = auto_send;
    settings.active_profile_id = None; // 测试不依赖真实 ~/.kotone
    settings.eval_recording = false; // 默认不录档（需要时测试自行开启并覆盖 eval_dir）
    settings.history.mode = kotone_core::history::HistoryMode::Off; // 默认不记历史（需要时测试自行开启并覆盖 history_dir）
    let settings = Arc::new(RwLock::new(settings));
    let mut registry = EngineRegistry::new();
    // 内置引擎（mock-stream 等）由 kotone-stt 注入（dev-dependency）
    kotone_stt::register_builtin(&mut registry);
    registry.register(Box::new(NeverReadyEngine));
    let engines = Arc::new(registry);
    let emitter = Arc::new(VecEmitter::default());
    let mut orch = Orchestrator::new(
        settings,
        engines,
        Arc::new(MockAudioBackend),
        injector,
        focus,
        emitter.clone(),
    );
    orch.toast_dwell = Duration::from_millis(10);
    orch.finalize_timeout = Duration::from_secs(2);
    orch.focus_restore_delay = Duration::ZERO;
    orch.release_grace = Duration::from_millis(10); // 测试用小宽限期
    tune(&mut orch);
    (orch.into_arc(), emitter)
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
    assert!(emitter.partials().contains(&"对面打野在下路".to_string()));

    orch.confirm_send().await.unwrap();
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
        assert!(
            seq.contains(&expected.to_string()),
            "缺状态 {expected}: {seq:?}"
        );
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
    assert!(
        !seq.contains(&"preview".to_string()),
        "autoSend 不应进 Preview"
    );
    assert!(seq.contains(&"sending".to_string()));
}

/// ADR-006 预览只读化：confirm_send 无文本参数，一律发送 preview_text
#[tokio::test]
async fn confirm_send_uses_preview_text() {
    let (orch, _emitter, sent) = make_orchestrator(false);
    orch.begin().await.unwrap();
    tokio::time::sleep(Duration::from_millis(30)).await;
    orch.end().await.unwrap();
    assert_eq!(orch.state(), OrchestratorState::Preview);

    orch.confirm_send().await.unwrap();
    assert_eq!(
        sent.lock().unwrap().as_slice(),
        &["对面打野在下路".to_string()]
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
        // NeverReadyEngine 恒未就绪（X-ASR 真机已就绪，不能再用它模拟）
        o.settings().write().unwrap().stt_engine = "never-ready".into();
        (o, e, s)
    };
    let r = orch.begin().await;
    assert!(r.is_err(), "未就绪引擎应开始失败");
    // Error toast 已发出，dwell 后自动回 Idle
    tokio::time::sleep(Duration::from_millis(40)).await;
    assert_eq!(orch.state(), OrchestratorState::Idle);
    assert!(emitter.state_sequence().contains(&"error".to_string()));
}

/// 发送挂起模拟（SendInput 被安全软件钩住不返回）：首次调用阻塞 hang_ms，
/// 之后立即成功（验证 inject_timeout 超时兜底回 Error 态 + 重试可恢复，P0）。
struct HangingInjector {
    hang_ms: u64,
    hung: Arc<std::sync::atomic::AtomicBool>,
}

impl Injector for HangingInjector {
    fn send(
        &self,
        _text: &str,
        _profile: &GameProfile,
        _cancel: CancelToken,
    ) -> Result<(), InjectError> {
        if self.hung.swap(false, Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(self.hang_ms));
        }
        Ok(())
    }
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
    orch.confirm_send().await.unwrap();
    assert_eq!(orch.state(), OrchestratorState::Error);
    tokio::time::sleep(Duration::from_millis(50)).await; // > toast_dwell(10ms)
    assert_eq!(
        orch.state(),
        OrchestratorState::Error,
        "带文本的 Error 不应自动回 Idle，需保留给用户重试"
    );

    // 重试：Error 状态 confirm_send 重新进入 Sending 并成功
    orch.confirm_send().await.unwrap();
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

/// P0：SendInput 挂起（被安全软件钩住不返回）时，inject_timeout 兜底回 Error 态。
/// 状态机不能永久卡在 Sending；Error 保留文本，后续可重试。
#[tokio::test]
async fn inject_timeout_falls_back_to_error_state_and_retry() {
    let injector: Arc<dyn Injector> = Arc::new(HangingInjector {
        hang_ms: 300,
        hung: Arc::new(std::sync::atomic::AtomicBool::new(true)),
    });
    let focus: Arc<dyn FocusBackend> =
        Arc::new(MockFocusBackend::new(Arc::new(Mutex::new(Vec::new())), 42, true));
    let (orch, emitter) = make_orchestrator_tuned(false, injector, focus, |o| {
        o.inject_timeout = Duration::from_millis(50); // 挂起 300ms >> 超时 50ms
    });

    orch.begin().await.unwrap();
    tokio::time::sleep(Duration::from_millis(60)).await;
    orch.end().await.unwrap();
    assert_eq!(orch.state(), OrchestratorState::Preview);

    orch.confirm_send().await.unwrap();
    // 超时兜底：状态必须离开 Sending，进入 Error（保留文本）
    assert_eq!(orch.state(), OrchestratorState::Error);
    let seq = emitter.state_sequence();
    assert_eq!(
        seq.last().map(String::as_str),
        Some("error"),
        "注入超时应回 Error 态而非永久卡 Sending: {seq:?}"
    );
    let err_msg = emitter
        .events
        .lock()
        .unwrap()
        .iter()
        .rev()
        .find(|(e, p)| e == "kotone://state" && p["state"] == "error")
        .and_then(|(_, p)| p["payload"]["message"].as_str().map(String::from))
        .unwrap_or_default();
    assert!(err_msg.contains("超时"), "错误信息应含超时提示: {err_msg}");
    assert!(
        emitter
            .events
            .lock()
            .unwrap()
            .iter()
            .any(|(e, p)| e == "kotone://state"
                && p["state"] == "error"
                && p["payload"]["text"] == "对面打野在下路"),
        "Error 态应保留文本供重试"
    );

    // 重试：第二次发送不再挂起，Error 态 confirm_send 重新进入 Sending 并成功
    orch.confirm_send().await.unwrap();
    assert_eq!(orch.state(), OrchestratorState::Success);
}

/// Error 状态下可带编辑后文本重试
/// Error 状态重试发送保留的原文（ADR-006：重试不可改文本，改文本 = Esc 重说）
#[tokio::test]
async fn error_state_retry_sends_retained_text() {
    let sent = Arc::new(Mutex::new(Vec::new()));
    let injector: Arc<dyn Injector> = Arc::new(FlakyInjector {
        attempts: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        sent: sent.clone(),
    });
    let (orch, _emitter) = make_orchestrator_with(false, injector);

    orch.begin().await.unwrap();
    tokio::time::sleep(Duration::from_millis(60)).await;
    orch.end().await.unwrap();
    orch.confirm_send().await.unwrap();
    assert_eq!(orch.state(), OrchestratorState::Error);

    orch.confirm_send().await.unwrap();
    assert_eq!(orch.state(), OrchestratorState::Success);
    // FlakyInjector 首次失败不落记录，重试成功应发出保留的原文
    assert_eq!(
        sent.lock().unwrap().as_slice(),
        &["对面打野在下路".to_string()]
    );
}

/// 无文本的 Error（如引擎未就绪）不可重试，且仍自动回 Idle
#[tokio::test]
async fn error_without_text_rejects_retry_and_auto_idles() {
    let (orch, emitter, _s) = {
        let (o, e, s) = make_orchestrator(false);
        // NeverReadyEngine 恒未就绪（X-ASR 真机已就绪，不能再用它模拟）
        o.settings().write().unwrap().stt_engine = "never-ready".into();
        (o, e, s)
    };
    let _ = orch.begin().await;
    assert_eq!(orch.state(), OrchestratorState::Error);
    // 无待发送文本：confirm_send 拒绝
    assert!(orch.confirm_send().await.is_err());
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
    orch.confirm_send().await.unwrap();
    assert_eq!(orch.state(), OrchestratorState::Error);

    orch.cancel().await;
    assert_eq!(orch.state(), OrchestratorState::Idle);
    assert!(orch.confirm_send().await.is_err(), "取消后不可再重试");
}

/// 目标窗口记忆与恢复：begin 捕获前台 hwnd，do_send 先恢复焦点再注入
#[tokio::test]
async fn target_window_captured_on_begin_and_restored_before_send() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let focus: Arc<dyn FocusBackend> = Arc::new(MockFocusBackend::new(log.clone(), 0xBEEF, true));
    let injector: Arc<dyn Injector> = Arc::new(LoggingInjector { log: log.clone() });
    let (orch, _emitter) = make_orchestrator_full(false, injector, focus);

    orch.begin().await.unwrap();
    tokio::time::sleep(Duration::from_millis(30)).await;
    orch.end().await.unwrap();
    assert_eq!(orch.state(), OrchestratorState::Preview);
    orch.confirm_send().await.unwrap();
    assert_eq!(orch.state(), OrchestratorState::Success);

    let ops = log.lock().unwrap().clone();
    assert_eq!(
        ops.first().map(String::as_str),
        Some("capture"),
        "begin 应捕获前台窗口"
    );
    let restore_pos = ops.iter().position(|s| s == "restore:48879"); // 0xBEEF
    let send_pos = ops.iter().position(|s| s.starts_with("send:"));
    assert!(restore_pos.is_some(), "发送前应恢复记录的 hwnd: {ops:?}");
    assert!(
        restore_pos.unwrap() < send_pos.unwrap(),
        "必须先恢复焦点再注入: {ops:?}"
    );
}

/// 焦点恢复失败（窗口已关闭）不阻断流程：注入器仍被调用（前台守卫已移除，直发当前前台）
#[tokio::test]
async fn send_proceeds_when_focus_restore_fails() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let focus: Arc<dyn FocusBackend> = Arc::new(MockFocusBackend::new(log.clone(), 1, false));
    let injector: Arc<dyn Injector> = Arc::new(LoggingInjector { log: log.clone() });
    let (orch, _emitter) = make_orchestrator_full(false, injector, focus);

    orch.begin().await.unwrap();
    tokio::time::sleep(Duration::from_millis(30)).await;
    orch.end().await.unwrap();
    orch.confirm_send().await.unwrap();

    let ops = log.lock().unwrap().clone();
    assert!(ops.contains(&"restore:1".to_string()));
    assert!(
        ops.iter().any(|s| s.starts_with("send:")),
        "恢复失败也应继续走注入（直发当前前台）: {ops:?}"
    );
}

/// preview 热键确认：toggle 模式下 Preview 态再按热键 = 确认发送（不取消会话）
#[tokio::test]
async fn hotkey_toggle_in_preview_confirms_send() {
    let (orch, _emitter, sent) = make_orchestrator(false);
    orch.begin().await.unwrap();
    tokio::time::sleep(Duration::from_millis(30)).await;
    orch.on_hotkey_toggle().await; // Listening → Transcribing → Preview
    assert_eq!(orch.state(), OrchestratorState::Preview);

    orch.on_hotkey_toggle().await; // Preview → 确认发送
    assert_eq!(
        orch.state(),
        OrchestratorState::Success,
        "Preview 态热键应确认发送而非取消"
    );
    assert_eq!(
        sent.lock().unwrap().as_slice(),
        &["对面打野在下路".to_string()]
    );
}

/// toggle 模式：Sending 态再按热键仍是取消（不受 preview→confirm 路由影响）
#[tokio::test]
async fn hotkey_toggle_during_sending_cancels() {
    /// 慢注入器：~500ms，期间可被取消令牌中断
    struct SlowInjector;
    impl Injector for SlowInjector {
        fn send(
            &self,
            _text: &str,
            _profile: &GameProfile,
            cancel: CancelToken,
        ) -> Result<(), InjectError> {
            for _ in 0..50 {
                if cancel.is_cancelled() {
                    return Err(InjectError::new("发送已取消"));
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Ok(())
        }
    }

    let (orch, _emitter) = make_orchestrator_with(false, Arc::new(SlowInjector));
    orch.begin().await.unwrap();
    tokio::time::sleep(Duration::from_millis(30)).await;
    orch.end().await.unwrap();
    assert_eq!(orch.state(), OrchestratorState::Preview);

    // 后台确认发送，等进入 Sending 后按热键取消
    let orch2 = orch.clone();
    let handle = tokio::spawn(async move { orch2.confirm_send().await });
    for _ in 0..200 {
        if orch.state() == OrchestratorState::Sending {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert_eq!(orch.state(), OrchestratorState::Sending);

    orch.on_hotkey_toggle().await;
    assert_eq!(
        orch.state(),
        OrchestratorState::Idle,
        "Sending 态热键应取消"
    );
    let _ = handle.await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        orch.state(),
        OrchestratorState::Idle,
        "取消后不应被过期结果改写"
    );
}

/// hold 模式：松手不立刻结束——宽限期内保持 Listening 继续收音，
/// 倒计时到点才 finalize 发送（0.1.6 修复：松手即停会丢掉仍在
/// 传输链路上与「还在空气中」的句尾音频，稳定丢最后 1-2 字）
#[tokio::test]
async fn hotkey_hold_release_waits_grace_before_sending() {
    let (orch, _emitter, sent) = make_orchestrator(true);
    orch.settings().write().unwrap().hotkey.mode = kotone_core::hotkey::HotkeyMode::Hold;

    orch.on_hotkey_hold(true).await;
    assert_eq!(orch.state(), OrchestratorState::Listening);
    tokio::time::sleep(Duration::from_millis(60)).await;

    orch.on_hotkey_hold(false).await;
    // 松手后不立刻结束：宽限期（测试值 10ms）内仍是 Listening
    assert_eq!(orch.state(), OrchestratorState::Listening);
    assert!(sent.lock().unwrap().is_empty());

    // 宽限期到点 → finalize → 直发（测试 toast_dwell 同为 10ms，
    // Success 可能已自动回 Idle，以实际发送为准）
    tokio::time::sleep(Duration::from_millis(80)).await;
    assert_eq!(
        sent.lock().unwrap().as_slice(),
        &["对面打野在下路".to_string()]
    );
    assert_ne!(orch.state(), OrchestratorState::Listening);
}

/// 宽限期内再次按下 = 收回结束、继续这句（对讲机直觉）；
/// 随后正常松手仍能完整结束发送
#[tokio::test]
async fn hotkey_hold_press_during_grace_cancels_pending_end() {
    let (orch, _emitter, sent) = make_orchestrator(true);
    orch.settings().write().unwrap().hotkey.mode = kotone_core::hotkey::HotkeyMode::Hold;

    orch.on_hotkey_hold(true).await;
    tokio::time::sleep(Duration::from_millis(30)).await;
    orch.on_hotkey_hold(false).await; // 松手 → 宽限倒计时开始（10ms）
    orch.on_hotkey_hold(true).await; // 立刻重按 → 收回结束
    tokio::time::sleep(Duration::from_millis(80)).await;
    assert_eq!(
        orch.state(),
        OrchestratorState::Listening,
        "宽限期内重按应收回结束、继续收音"
    );
    assert!(sent.lock().unwrap().is_empty());

    // 继续说，再次松手 → 这次走完宽限期正常发送
    orch.on_hotkey_hold(false).await;
    tokio::time::sleep(Duration::from_millis(80)).await;
    assert_eq!(sent.lock().unwrap().len(), 1, "只发送一次");
}

/// hold 模式：非 Idle 态的按下事件忽略（不弹错误 toast 冲掉预览文本）
#[tokio::test]
async fn hotkey_hold_press_in_preview_is_ignored() {
    let (orch, _emitter, sent) = make_orchestrator(false);
    orch.begin().await.unwrap();
    tokio::time::sleep(Duration::from_millis(30)).await;
    orch.end().await.unwrap();
    assert_eq!(orch.state(), OrchestratorState::Preview);

    orch.on_hotkey_hold(true).await;
    assert_eq!(
        orch.state(),
        OrchestratorState::Preview,
        "Preview 态按下事件应被忽略"
    );
    assert!(sent.lock().unwrap().is_empty());

    // 之后仍可正常确认发送
    orch.confirm_send().await.unwrap();
    assert_eq!(orch.state(), OrchestratorState::Success);
}

/// hold 模式：Success toast 驻留期内按下 = 立即开始下一句（「第一次按没反应」修复）
#[tokio::test]
async fn hotkey_hold_press_during_success_starts_next_session() {
    let (orch, _emitter, sent) = make_orchestrator(true);
    orch.begin().await.unwrap();
    tokio::time::sleep(Duration::from_millis(60)).await;
    orch.end().await.unwrap();
    assert_eq!(orch.state(), OrchestratorState::Success);

    // 驻留期（toast_dwell=10ms 测试值）内按下：不再被忽略，清掉 toast 并开新会话
    orch.on_hotkey_hold(true).await;
    assert_eq!(orch.state(), OrchestratorState::Listening);
    assert_eq!(sent.lock().unwrap().len(), 1, "上一句只发送一次");

    orch.cancel().await;
}

/// hold 模式：注入失败（带文本 Error）后按下热键 = 重试发送
/// （0.1.5 回归：hold 模式带文本 Error 时按下被忽略，llhook 捕获正常但状态机
/// 不再迁移，用户只能重启 runtime——真实用户诊断包实锤）
#[tokio::test]
async fn hotkey_hold_press_in_error_with_text_retries_send() {
    let sent = Arc::new(Mutex::new(Vec::new()));
    let injector: Arc<dyn Injector> = Arc::new(FlakyInjector {
        attempts: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        sent: sent.clone(),
    });
    let (orch, _emitter) = make_orchestrator_with(true, injector);

    orch.begin().await.unwrap();
    tokio::time::sleep(Duration::from_millis(60)).await;
    orch.end().await.unwrap();
    assert_eq!(
        orch.state(),
        OrchestratorState::Error,
        "首次注入失败应停在带文本 Error"
    );

    // 按下热键 = 重试发送（FlakyInjector 第二次成功）
    orch.on_hotkey_hold(true).await;
    assert_eq!(
        orch.state(),
        OrchestratorState::Success,
        "带文本 Error 态按下热键应重试发送，而不是忽略"
    );
    assert_eq!(
        sent.lock().unwrap().as_slice(),
        &["对面打野在下路".to_string()],
        "重试应发出保留的原文"
    );
}

/// toggle 模式：Success 驻留期内点按同样立即开始下一句（与 hold 同语义）
#[tokio::test]
async fn hotkey_toggle_during_success_starts_next_session() {
    let (orch, _emitter, sent) = make_orchestrator(true);
    orch.begin().await.unwrap();
    tokio::time::sleep(Duration::from_millis(60)).await;
    orch.end().await.unwrap();
    assert_eq!(orch.state(), OrchestratorState::Success);

    orch.on_hotkey_toggle().await;
    assert_eq!(orch.state(), OrchestratorState::Listening);
    assert_eq!(sent.lock().unwrap().len(), 1, "上一句只发送一次");

    orch.cancel().await;
}

/// 空转录「无事发生」：finalize 空文本 → 不发送、不写 history、直接回 Idle
#[tokio::test]
async fn empty_finalize_returns_idle_silently() {
    let dir = tempfile::tempdir().unwrap();
    let sent = Arc::new(Mutex::new(Vec::new()));
    let injector: Arc<dyn Injector> = Arc::new(RecordingInjector { sent: sent.clone() });
    let focus: Arc<dyn FocusBackend> = Arc::new(MockFocusBackend::new(
        Arc::new(Mutex::new(Vec::new())),
        42,
        true,
    ));
    let mut settings = Settings::default();
    settings.stt_engine = "stub-final".into();
    settings.auto_send = true; // C1 直发模式：空文本也不应触发注入器敲回车
    settings.active_profile_id = None;
    settings.eval_recording = false;
    // history 保持默认开启（落账目录指临时目录），验证空转录不写记录
    let settings = Arc::new(RwLock::new(settings));
    let mut registry = EngineRegistry::new();
    registry.register(Box::new(StubFinalEngine { final_text: "" }));
    let emitter = Arc::new(VecEmitter::default());
    let mut orch = Orchestrator::new(
        settings,
        Arc::new(registry),
        Arc::new(MockAudioBackend),
        injector,
        focus,
        emitter.clone(),
    );
    orch.toast_dwell = Duration::from_millis(10);
    orch.finalize_timeout = Duration::from_secs(2);
    orch.focus_restore_delay = Duration::ZERO;
    orch.history_dir = Some(dir.path().to_path_buf());
    let orch = orch.into_arc();

    orch.begin().await.unwrap();
    tokio::time::sleep(Duration::from_millis(30)).await;
    orch.end().await.unwrap();

    // 状态直接回 Idle：不经 Preview/Sending/Success，也不发 Error toast
    assert_eq!(orch.state(), OrchestratorState::Idle);
    let seq = emitter.state_sequence();
    for unexpected in ["preview", "sending", "success", "error"] {
        assert!(
            !seq.contains(&unexpected.to_string()),
            "空转录不应经过 {unexpected}: {seq:?}"
        );
    }
    assert!(
        seq.contains(&"idle".to_string()),
        "应发出 idle 状态事件: {seq:?}"
    );
    // 注入器未被调用（空文本不该敲出两个回车）
    assert!(sent.lock().unwrap().is_empty(), "空转录不应触发注入");
    // 不写 history 记录
    assert!(
        kotone_core::history::list_in(dir.path())
            .unwrap()
            .is_empty(),
        "空转录不应落 history 记录"
    );
    // 之后可正常开始新会话
    orch.begin().await.unwrap();
    assert_eq!(orch.state(), OrchestratorState::Listening);
    orch.cancel().await;
}

/// 带 eval 录档的 orchestrator：录档目录指向临时目录（不污染真实 ~/.kotone/eval）
fn make_orchestrator_with_eval(
    eval_dir: std::path::PathBuf,
) -> (Arc<Orchestrator>, Arc<VecEmitter>, Arc<Mutex<Vec<String>>>) {
    let sent = Arc::new(Mutex::new(Vec::new()));
    let injector: Arc<dyn Injector> = Arc::new(RecordingInjector { sent: sent.clone() });
    let focus: Arc<dyn FocusBackend> = Arc::new(MockFocusBackend::new(
        Arc::new(Mutex::new(Vec::new())),
        42,
        true,
    ));
    let mut settings = Settings::default();
    settings.stt_engine = "mock-stream".into();
    settings.active_profile_id = None;
    settings.eval_recording = true;
    settings.history.mode = kotone_core::history::HistoryMode::Off; // 不写真实 ~/.kotone/history
    let settings = Arc::new(RwLock::new(settings));
    let mut registry = EngineRegistry::new();
    kotone_stt::register_builtin(&mut registry);
    let emitter = Arc::new(VecEmitter::default());
    let mut orch = Orchestrator::new(
        settings,
        Arc::new(registry),
        Arc::new(MockAudioBackend),
        injector,
        focus,
        emitter.clone(),
    );
    orch.toast_dwell = Duration::from_millis(10);
    orch.finalize_timeout = Duration::from_secs(2);
    orch.focus_restore_delay = Duration::ZERO;
    orch.eval_dir = Some(eval_dir);
    (orch.into_arc(), emitter, sent)
}

/// eval 录档：finalize 成功后落盘 wav + json（字段契约对齐 docs/development.md §5.4）
#[tokio::test]
async fn eval_recording_written_on_finalize() {
    let dir = tempfile::tempdir().unwrap();
    let (orch, _emitter, _sent) = make_orchestrator_with_eval(dir.path().to_path_buf());

    orch.begin().await.unwrap();
    // 录足 1s+ 假音频（Windows 定时器粒度 ~15.6ms，500ms 实际等待 ≈ 1.5s 音频）
    tokio::time::sleep(Duration::from_millis(500)).await;
    orch.end().await.unwrap();

    let sessions = kotone_core::eval::list_sessions_at(dir.path()).unwrap();
    assert_eq!(sessions.len(), 1, "finalize 成功应录档一次");
    let s = &sessions[0];
    assert_eq!(s.engine_id, "mock-stream");
    assert!(s.audio_ms >= 800, "audioMs 应接近录制时长: {s:?}");
    assert!(!s.partials.is_empty(), "mock 流式引擎应录到 partial: {s:?}");
    assert!(
        s.first_partial_ms.is_some() && s.first_partial_ms == s.partials.first().map(|p| p.t),
        "firstPartialMs 应等于首条 partial 的相对时间戳: {s:?}"
    );
    assert_eq!(s.final_text, "对面打野在下路");
    assert_eq!(s.human_label, None);
    assert!(dir.path().join(format!("{}.wav", s.session_id)).exists());
    let pcm =
        kotone_core::eval::read_wav(&dir.path().join(format!("{}.wav", s.session_id))).unwrap();
    assert_eq!(
        pcm.len() as u64,
        s.audio_ms * 16,
        "wav 采样数应与 audioMs 一致"
    );
}

/// eval 录档：取消的会话不录（目录保持为空）
#[tokio::test]
async fn eval_recording_discarded_on_cancel() {
    let dir = tempfile::tempdir().unwrap();
    let (orch, _emitter, _sent) = make_orchestrator_with_eval(dir.path().to_path_buf());

    orch.begin().await.unwrap();
    tokio::time::sleep(Duration::from_millis(60)).await;
    orch.cancel().await;

    assert!(
        kotone_core::eval::list_sessions_at(dir.path())
            .unwrap()
            .is_empty(),
        "取消的会话不应录档"
    );
}

/// eval 录档：evalRecording 关闭时不落盘
#[tokio::test]
async fn eval_recording_off_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let (orch, _emitter, _sent) = {
        let (o, e, s) = make_orchestrator_with_eval(dir.path().to_path_buf());
        o.settings().write().unwrap().eval_recording = false;
        (o, e, s)
    };
    orch.begin().await.unwrap();
    tokio::time::sleep(Duration::from_millis(60)).await;
    orch.end().await.unwrap();
    assert!(
        kotone_core::eval::list_sessions_at(dir.path())
            .unwrap()
            .is_empty(),
        "evalRecording 关闭时不应录档"
    );
}

// ---------- 模式 3「说一句就走」（one-shot：A2 + B3 + C1，ADR-007） ----------

/// 脚本化 VAD：按预设序列逐帧返回语音判定（序列用完后恒返回尾部值）
struct ScriptVad {
    script: Vec<bool>,
    idx: usize,
}

impl ScriptVad {
    /// n_speech 帧语音后恒静音
    fn speech_then_silence(n_speech: usize) -> Self {
        Self {
            script: vec![true; n_speech],
            idx: 0,
        }
    }
    /// 恒静音（永不判停）
    fn all_silence() -> Self {
        Self {
            script: Vec::new(),
            idx: 0,
        }
    }
}

impl kotone_core::vad::Vad for ScriptVad {
    fn push_frame(&mut self, _frame: &[f32]) -> Result<bool, String> {
        let v = self.script.get(self.idx).copied().unwrap_or(false);
        self.idx += 1;
        Ok(v)
    }
    fn reset(&mut self) {
        self.idx = 0;
    }
}

/// one-shot 测试架：预设 interactionMode=one-shot + 快阈值（210ms）+ 脚本 VAD
fn make_one_shot_orchestrator(
    vad: ScriptVad,
) -> (Arc<Orchestrator>, Arc<VecEmitter>, Arc<Mutex<Vec<String>>>) {
    make_vad_mode_orchestrator(vad, kotone_core::interaction::InteractionMode::OneShot)
}

/// VAD 判停模式通用测试架（one-shot / solo）：指定预设 + 快阈值（210ms）+ 脚本 VAD。
/// 脚本 VAD 只喂第一个会话；判停/停止后的再次 begin 拿到恒静音 VAD（不再判停）。
fn make_vad_mode_orchestrator(
    vad: ScriptVad,
    mode: kotone_core::interaction::InteractionMode,
) -> (Arc<Orchestrator>, Arc<VecEmitter>, Arc<Mutex<Vec<String>>>) {
    let sent = Arc::new(Mutex::new(Vec::new()));
    let injector: Arc<dyn Injector> = Arc::new(RecordingInjector { sent: sent.clone() });
    let focus: Arc<dyn FocusBackend> = Arc::new(MockFocusBackend::new(
        Arc::new(Mutex::new(Vec::new())),
        42,
        true,
    ));
    let mut settings = Settings::default();
    settings.stt_engine = "mock-stream".into();
    settings.active_profile_id = None;
    settings.eval_recording = false;
    settings.history.mode = kotone_core::history::HistoryMode::Off; // 不写真实 ~/.kotone/history
    settings.interaction_mode = Some(mode);
    settings.vad_silence_ms = 210; // 7 帧静音即判停（测试快进）
    let settings = Arc::new(RwLock::new(settings));
    let mut registry = EngineRegistry::new();
    kotone_stt::register_builtin(&mut registry);
    let emitter = Arc::new(VecEmitter::default());
    let mut orch = Orchestrator::new(
        settings,
        Arc::new(registry),
        Arc::new(MockAudioBackend),
        injector,
        focus,
        emitter.clone(),
    );
    orch.toast_dwell = Duration::from_millis(10);
    orch.finalize_timeout = Duration::from_secs(2);
    orch.focus_restore_delay = Duration::ZERO;
    let vad = std::sync::Mutex::new(Some(vad));
    orch.vad_factory = Some(Arc::new(move || {
        Ok(Box::new(
            vad.lock()
                .unwrap()
                .take()
                .unwrap_or_else(ScriptVad::all_silence),
        ) as Box<dyn kotone_core::vad::Vad>)
    }));
    (orch.into_arc(), emitter, sent)
}

/// 轮询等待目标状态（VAD 判停是异步的：pump → end() → finalize → 发送）
async fn wait_state(orch: &Orchestrator, want: OrchestratorState, timeout: Duration) {
    let start = std::time::Instant::now();
    while orch.state() != want {
        assert!(
            start.elapsed() < timeout,
            "等待状态 {want:?} 超时（当前 {:?}）",
            orch.state()
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// 模式 3 全链路：A2 点按开始 → B3 VAD 判停（无需再按）→ C1 直发 → Success
#[tokio::test]
async fn one_shot_vad_stop_auto_sends() {
    // 剧本：30 帧语音（900ms > 最短保护 500ms）后恒静音 → 210ms 阈值判停
    let (orch, emitter, sent) = make_one_shot_orchestrator(ScriptVad::speech_then_silence(30));

    orch.on_hotkey_toggle().await; // A2：点按开始
    assert_eq!(orch.state(), OrchestratorState::Listening);

    // B3：VAD 判停自动结束（不用任何按键）→ C1 直发 → Success
    wait_state(&orch, OrchestratorState::Success, Duration::from_secs(5)).await;

    // 判停事件已外发（CLI/前端可观测）
    assert!(
        emitter
            .events
            .lock()
            .unwrap()
            .iter()
            .any(|(e, _)| e == "kotone://vad-stop"),
        "应发出 kotone://vad-stop 事件"
    );
    // C1：转写完直接发送 mock 最终文本，全程无 Preview
    assert_eq!(sent.lock().unwrap().as_slice(), ["对面打野在下路"]);
    assert!(
        !emitter.state_sequence().contains(&"preview".to_string()),
        "one-shot 不应经过 Preview: {:?}",
        emitter.state_sequence()
    );
    // toast 后自动回 Idle
    wait_state(&orch, OrchestratorState::Idle, Duration::from_secs(2)).await;
}

/// 模式 3 热键兜底：VAD 失效（恒静音不判停）时再按热键强制结束仍生效
#[tokio::test]
async fn one_shot_hotkey_force_end_when_vad_never_stops() {
    let (orch, _emitter, sent) = make_one_shot_orchestrator(ScriptVad::all_silence());

    orch.on_hotkey_toggle().await;
    assert_eq!(orch.state(), OrchestratorState::Listening);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        orch.state(),
        OrchestratorState::Listening,
        "恒静音剧本下 VAD 不应判停"
    );

    orch.on_hotkey_toggle().await; // 热键强制结束（B3 兜底恒在）
    wait_state(&orch, OrchestratorState::Success, Duration::from_secs(5)).await;
    assert_eq!(sent.lock().unwrap().as_slice(), ["对面打野在下路"]);
}

/// 模式 3 未接入 VAD 工厂：begin 报清晰错误（Error toast 后自动回 Idle）
#[tokio::test]
async fn one_shot_without_vad_factory_begin_fails() {
    let (orch, _emitter, sent) = make_orchestrator(true);
    orch.settings().write().unwrap().interaction_mode =
        Some(kotone_core::interaction::InteractionMode::OneShot);
    // vad_factory 保持 None（未接入）

    let err = orch.begin().await.unwrap_err();
    assert!(err.contains("VAD"), "错误应指明 VAD 未接入: {err}");
    // begin 失败走 Error toast → 自动回 Idle；不产生任何发送
    wait_state(&orch, OrchestratorState::Idle, Duration::from_secs(2)).await;
    assert!(sent.lock().unwrap().is_empty());
}

// ---------- 模式 4「独奏模式」（solo：A2 + B3 + C1 + 连续，发完不停机） ----------

/// solo 全链路：VAD 判停 → 直发 → 不显示 Success、不等待 toast，立即回到 Listening
#[tokio::test]
async fn solo_send_returns_to_listening() {
    let (orch, emitter, sent) = make_vad_mode_orchestrator(
        ScriptVad::speech_then_silence(30),
        kotone_core::interaction::InteractionMode::Solo,
    );

    orch.on_hotkey_toggle().await; // A2：点按开始持续收音
    assert_eq!(orch.state(), OrchestratorState::Listening);

    // B3 判停 → C1 直发；发送成功后直接续听，不经过会阻塞下一句话的 Success toast。
    let start = std::time::Instant::now();
    while sent.lock().unwrap().is_empty() {
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "等待 solo 注入超时"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(sent.lock().unwrap().as_slice(), ["对面打野在下路"]);

    // 续段拿恒静音 VAD，不再判停；状态序列也不应闪过 Success。
    wait_state(&orch, OrchestratorState::Listening, Duration::from_secs(5)).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        orch.state(),
        OrchestratorState::Listening,
        "solo 发送完成后应持续监听：{:?}",
        emitter.state_sequence()
    );
    assert!(
        !emitter.state_sequence().contains(&"success".to_string()),
        "solo 不应闪过 Success toast：{:?}",
        emitter.state_sequence()
    );
    assert_eq!(sent.lock().unwrap().len(), 1, "续段无语音不应重复发送");

    // 停止：再点按热键 → Idle
    orch.on_hotkey_toggle().await;
    wait_state(&orch, OrchestratorState::Idle, Duration::from_secs(2)).await;
}

/// solo 停止语义：Listening 态再点按热键 = 停止会话（丢弃在途段，不发送）
#[tokio::test]
async fn solo_toggle_stops_without_sending() {
    let (orch, emitter, sent) = make_vad_mode_orchestrator(
        ScriptVad::all_silence(),
        kotone_core::interaction::InteractionMode::Solo,
    );

    orch.on_hotkey_toggle().await;
    assert_eq!(orch.state(), OrchestratorState::Listening);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(orch.state(), OrchestratorState::Listening, "恒静音不判停");

    orch.on_hotkey_toggle().await; // 再点按 = 停止（不是把在途段发出去）
    wait_state(&orch, OrchestratorState::Idle, Duration::from_secs(2)).await;
    assert!(
        sent.lock().unwrap().is_empty(),
        "停止不应发送在途段: {:?}",
        sent.lock().unwrap()
    );
    let seq = emitter.state_sequence();
    assert!(
        !seq.contains(&"sending".to_string()),
        "停止路径不应经过 Sending: {seq:?}"
    );
}

// ---------- history：终态落账（HistoryDraft → history.jsonl） ----------

/// 带 history 的 orchestrator：history_dir 指向临时目录；
/// eval_dir 给 Some 时同时开启 evalRecording（历史音频不依赖该开关）。
fn make_history_orchestrator(
    auto_send: bool,
    injector: Arc<dyn Injector>,
    history_dir: std::path::PathBuf,
    eval_dir: Option<std::path::PathBuf>,
) -> Arc<Orchestrator> {
    let mut settings = Settings::default();
    // 同 make_orchestrator_full：沿用 toggle + autoSend 推导，显式置 None
    settings.interaction_mode = None;
    settings.stt_engine = "mock-stream".into();
    settings.auto_send = auto_send;
    settings.active_profile_id = Some("lol".into()); // 验证 profileId 落账
    settings.eval_recording = eval_dir.is_some();
    // history 默认 capped/1000/不含音频（测试按需再改 settings）
    let settings = Arc::new(RwLock::new(settings));
    let mut registry = EngineRegistry::new();
    kotone_stt::register_builtin(&mut registry);
    registry.register(Box::new(NeverReadyEngine));
    let focus: Arc<dyn FocusBackend> = Arc::new(MockFocusBackend::new(
        Arc::new(Mutex::new(Vec::new())),
        42,
        true,
    ));
    let mut orch = Orchestrator::new(
        settings,
        Arc::new(registry),
        Arc::new(MockAudioBackend),
        injector,
        focus,
        Arc::new(VecEmitter::default()),
    );
    orch.toast_dwell = Duration::from_millis(10);
    orch.finalize_timeout = Duration::from_secs(2);
    orch.focus_restore_delay = Duration::ZERO;
    orch.history_dir = Some(history_dir);
    orch.eval_dir = eval_dir;
    orch.into_arc()
}

/// sent 终态：一条完整记录（文本/引擎/profile/时长/延迟/sessionId）
#[tokio::test]
async fn history_sent_records_one_entry() {
    let dir = tempfile::tempdir().unwrap();
    let sent = Arc::new(Mutex::new(Vec::new()));
    let injector: Arc<dyn Injector> = Arc::new(RecordingInjector { sent: sent.clone() });
    let orch = make_history_orchestrator(true, injector, dir.path().to_path_buf(), None);

    orch.begin().await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await; // 等 mock-stream 首个 partial
    orch.end().await.unwrap();
    assert_eq!(orch.state(), OrchestratorState::Success);

    let records = kotone_core::history::list_in(dir.path()).unwrap();
    assert_eq!(records.len(), 1, "records: {records:?}");
    let r = &records[0];
    assert_eq!(r.outcome, kotone_core::history::HistoryOutcome::Sent);
    assert_eq!(r.final_text, "对面打野在下路");
    assert_eq!(r.engine_id, "mock-stream");
    assert_eq!(r.profile_id.as_deref(), Some("lol"));
    assert!(r.audio_ms > 0, "录音时长应累计");
    assert!(r.first_partial_ms.is_some(), "500ms 后应已出现首个 partial");
    assert!(r.finalize_latency_ms.is_some());
    assert!(r.error.is_none());
    assert!(r.audio_file.is_none(), "includeAudio 默认关闭");
    assert!(!r.session_id.is_empty() && !r.ts.is_empty());
}

/// cancelled 终态：Listening 中取消，记一条 cancelled（无最终文本）
#[tokio::test]
async fn history_cancel_during_listening_records_cancelled() {
    let dir = tempfile::tempdir().unwrap();
    let sent = Arc::new(Mutex::new(Vec::new()));
    let injector: Arc<dyn Injector> = Arc::new(RecordingInjector { sent });
    let orch = make_history_orchestrator(false, injector, dir.path().to_path_buf(), None);

    orch.begin().await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    orch.cancel().await;

    let records = kotone_core::history::list_in(dir.path()).unwrap();
    assert_eq!(records.len(), 1, "records: {records:?}");
    assert_eq!(
        records[0].outcome,
        kotone_core::history::HistoryOutcome::Cancelled
    );
    assert!(records[0].final_text.is_empty());
    assert!(records[0].finalize_latency_ms.is_none());
}

/// error → 重试成功：同 sessionId 写 error + sent 两条（刻意的失败→重试叙事）
#[tokio::test]
async fn history_error_retry_writes_two_entries_same_session() {
    let dir = tempfile::tempdir().unwrap();
    let sent = Arc::new(Mutex::new(Vec::new()));
    let injector: Arc<dyn Injector> = Arc::new(FlakyInjector {
        attempts: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        sent: sent.clone(),
    });
    let orch = make_history_orchestrator(false, injector, dir.path().to_path_buf(), None);

    orch.begin().await.unwrap();
    tokio::time::sleep(Duration::from_millis(60)).await;
    orch.end().await.unwrap();
    orch.confirm_send().await.unwrap();
    assert_eq!(orch.state(), OrchestratorState::Error);
    orch.confirm_send().await.unwrap();
    assert_eq!(orch.state(), OrchestratorState::Success);

    let records = kotone_core::history::list_in(dir.path()).unwrap();
    assert_eq!(records.len(), 2, "records: {records:?}");
    // list 新→旧：sent 在前，error 在后
    assert_eq!(
        records[0].outcome,
        kotone_core::history::HistoryOutcome::Sent
    );
    assert_eq!(
        records[1].outcome,
        kotone_core::history::HistoryOutcome::Error
    );
    assert_eq!(
        records[0].session_id, records[1].session_id,
        "同会话重试应同 sessionId"
    );
    assert!(records[1].error.is_some(), "error 记录应带错误信息");
    assert_eq!(records[0].final_text, "对面打野在下路");
}

/// error → Esc 取消：error 已落账，cancel 是清理动作，不双记 cancelled
#[tokio::test]
async fn history_error_then_cancel_does_not_double_record() {
    let dir = tempfile::tempdir().unwrap();
    let sent = Arc::new(Mutex::new(Vec::new()));
    let injector: Arc<dyn Injector> = Arc::new(FlakyInjector {
        attempts: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        sent: sent.clone(),
    });
    let orch = make_history_orchestrator(false, injector, dir.path().to_path_buf(), None);

    orch.begin().await.unwrap();
    tokio::time::sleep(Duration::from_millis(60)).await;
    orch.end().await.unwrap();
    orch.confirm_send().await.unwrap();
    assert_eq!(orch.state(), OrchestratorState::Error);
    orch.cancel().await;

    let records = kotone_core::history::list_in(dir.path()).unwrap();
    assert_eq!(records.len(), 1, "error 后的 cancel 不应双记: {records:?}");
    assert_eq!(
        records[0].outcome,
        kotone_core::history::HistoryOutcome::Error
    );
}

/// mode=off：完整走一遍 sent 流程，零记录零文件
#[tokio::test]
async fn history_off_mode_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let sent = Arc::new(Mutex::new(Vec::new()));
    let injector: Arc<dyn Injector> = Arc::new(RecordingInjector { sent: sent.clone() });
    let orch = make_history_orchestrator(true, injector, dir.path().to_path_buf(), None);
    orch.settings().write().unwrap().history.mode = kotone_core::history::HistoryMode::Off;

    orch.begin().await.unwrap();
    tokio::time::sleep(Duration::from_millis(60)).await;
    orch.end().await.unwrap();
    assert_eq!(orch.state(), OrchestratorState::Success);

    assert!(kotone_core::history::list_in(dir.path())
        .unwrap()
        .is_empty());
    assert!(
        !dir.path().join("history.jsonl").exists(),
        "off 模式不应产生文件"
    );
}

/// includeAudio：即使 evalRecording 关闭，sent 也独立写入 history/audio/。
#[tokio::test]
async fn history_include_audio_works_when_eval_recording_is_off() {
    let dir = tempfile::tempdir().unwrap();
    let sent = Arc::new(Mutex::new(Vec::new()));
    let injector: Arc<dyn Injector> = Arc::new(RecordingInjector { sent: sent.clone() });
    let orch = make_history_orchestrator(true, injector, dir.path().to_path_buf(), None);
    orch.settings().write().unwrap().history.include_audio = true;
    assert!(!orch.settings().read().unwrap().eval_recording);

    orch.begin().await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    orch.end().await.unwrap();
    assert_eq!(orch.state(), OrchestratorState::Success);

    let records = kotone_core::history::list_in(dir.path()).unwrap();
    assert_eq!(records.len(), 1);
    let r = &records[0];
    let audio_file = r.audio_file.clone().expect("includeAudio 应落音频文件名");
    assert_eq!(audio_file, format!("{}.wav", r.session_id));
    let wav = dir.path().join("audio").join(&audio_file);
    assert!(wav.exists(), "history/audio/ 下应有独立 wav");
    assert!(
        !kotone_core::eval::read_wav(&wav).unwrap().is_empty(),
        "历史音频应包含本次会话 PCM"
    );
}

// ---------- 松手丢字 P0：end() 采音侧排空 ----------

/// 一次性吐完固定数量 chunk 后关闭通道的 mock 后端（模拟松手瞬间通道里
/// 已缓冲但 pump 还没消费的 PCM；任务结束 pcm_tx 随之释放 → 通道关闭）
struct BurstAudioBackend {
    chunks: usize,
}

impl AudioBackend for BurstAudioBackend {
    fn start(&self, _device_id: &str) -> Result<AudioHandle, String> {
        let (pcm_tx, pcm_rx) = mpsc::unbounded_channel::<Vec<f32>>();
        let (level_tx, level_rx) = mpsc::unbounded_channel::<f32>();
        drop(level_tx);
        let n = self.chunks;
        tokio::spawn(async move {
            for _ in 0..n {
                if pcm_tx.send(vec![0.1f32; 800]).is_err() {
                    break;
                }
            }
        });
        Ok(AudioHandle::detached(pcm_rx, level_rx))
    }
}

/// P0 回归：松手（end）时通道里已缓冲未消费的 PCM 必须全部灌进 session
/// 再 finalize——6 × 50ms chunk（共 300ms 音频）一个都不能丢
#[tokio::test]
async fn end_drains_buffered_pcm_before_finalize() {
    let dir = tempfile::tempdir().unwrap();
    let sent = Arc::new(Mutex::new(Vec::new()));
    let injector: Arc<dyn Injector> = Arc::new(RecordingInjector { sent: sent.clone() });
    let focus: Arc<dyn FocusBackend> = Arc::new(MockFocusBackend::new(
        Arc::new(Mutex::new(Vec::new())),
        42,
        true,
    ));
    let mut settings = Settings::default();
    settings.stt_engine = "mock-stream".into();
    settings.auto_send = true;
    settings.active_profile_id = None;
    settings.eval_recording = false;
    let settings = Arc::new(RwLock::new(settings));
    let mut registry = EngineRegistry::new();
    kotone_stt::register_builtin(&mut registry);
    let mut orch = Orchestrator::new(
        settings,
        Arc::new(registry),
        Arc::new(BurstAudioBackend { chunks: 6 }),
        injector,
        focus,
        Arc::new(VecEmitter::default()),
    );
    orch.toast_dwell = Duration::from_millis(10);
    orch.finalize_timeout = Duration::from_secs(2);
    orch.focus_restore_delay = Duration::ZERO;
    orch.history_dir = Some(dir.path().to_path_buf());
    let orch = orch.into_arc();

    orch.begin().await.unwrap();
    // 等突发 chunk 全部进通道（pump 可能已消费一部分——不影响总数断言），
    // 随后立即松手：剩余的必须靠排空路径灌入
    tokio::time::sleep(Duration::from_millis(30)).await;
    orch.end().await.unwrap();
    assert_eq!(orch.state(), OrchestratorState::Success);

    let records = kotone_core::history::list_in(dir.path()).unwrap();
    assert_eq!(records.len(), 1, "records: {records:?}");
    assert_eq!(
        records[0].audio_ms, 300,
        "松手时通道里缓冲的 PCM 必须全部灌进 session（6 × 50ms = 300ms）"
    );
}

// ---------- 真机回归：对讲机「说完立刻松手」丢句尾（0.1.6 用户反馈） ----------

/// 按真实节奏（每 50ms 一个 chunk）推送 16k wav 的采集后端；
/// 推完后保持通道敞开（模拟真实设备不会主动断流）。
struct WavPacedBackend {
    pcm: Vec<f32>,
}

impl AudioBackend for WavPacedBackend {
    fn start(&self, _device_id: &str) -> Result<AudioHandle, String> {
        let (pcm_tx, pcm_rx) = mpsc::unbounded_channel::<Vec<f32>>();
        let (level_tx, level_rx) = mpsc::unbounded_channel::<f32>();
        let pcm = self.pcm.clone();
        tokio::spawn(async move {
            for chunk in pcm.chunks(kotone_core::audio::CHUNK_SAMPLES) {
                if pcm_tx.send(chunk.to_vec()).is_err() {
                    return;
                }
                let _ = level_tx.send(0.1f32);
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            // 推完保持 sender 存活：通道不断开，模拟真实采集常驻
            tokio::time::sleep(Duration::from_secs(120)).await;
        });
        Ok(AudioHandle::detached(pcm_rx, level_rx))
    }
}

/// 真 X-ASR + 异步全链路：音频一推完立刻 end()（= 说完立刻松手的最坏时序），
/// 断言注入文本包含句尾。KOTONE_TEST_WAV / KOTONE_TEST_EXPECTED 指定语料与句尾。
/// 手动跑：
///   KOTONE_TEST_WAV=path.wav KOTONE_TEST_EXPECTED=句尾 \
///   cargo test -p kotone-core --features kotone-stt/engine-sherpa \
///     --test orchestrator hold_release_right_after_speech -- --ignored --nocapture
#[tokio::test]
#[ignore = "依赖真机 X-ASR 模型与测试 wav，手动跑"]
async fn hold_release_right_after_speech_keeps_sentence_tail() {
    let wav = std::env::var("KOTONE_TEST_WAV").expect("KOTONE_TEST_WAV 未设置");
    let expected = std::env::var("KOTONE_TEST_EXPECTED").expect("KOTONE_TEST_EXPECTED 未设置");
    let pcm = kotone_core::eval::read_wav(std::path::Path::new(&wav)).expect("读取测试 wav 失败");
    let audio_ms = pcm.len() as u64 * 1000 / kotone_core::eval::SAMPLE_RATE as u64;

    let sent = Arc::new(Mutex::new(Vec::new()));
    let injector: Arc<dyn Injector> = Arc::new(RecordingInjector { sent: sent.clone() });
    let focus: Arc<dyn FocusBackend> = Arc::new(MockFocusBackend::new(
        Arc::new(Mutex::new(Vec::new())),
        42,
        true,
    ));

    let mut settings = Settings::default();
    settings.interaction_mode = None;
    settings.hotkey.mode = kotone_core::hotkey::HotkeyMode::Hold; // B1 松手结束
    settings.stt_engine = "sherpa-onnx-x-asr-zh-en".into();
    settings.auto_send = true; // hold 直发
    settings.active_profile_id = None;
    settings.eval_recording = false;
    settings.history.mode = kotone_core::history::HistoryMode::Capped;
    let settings = Arc::new(RwLock::new(settings));

    let mut registry = EngineRegistry::new();
    kotone_stt::register_builtin(&mut registry);
    assert!(
        registry
            .get("sherpa-onnx-x-asr-zh-en")
            .map(|e| e.is_ready())
            .unwrap_or(false),
        "X-ASR 模型未就绪（kotone-cli download x-asr-480ms-streaming-zh-en-punct-int8-2026-06-05）"
    );

    let emitter = Arc::new(VecEmitter::default());
    let mut orch = Orchestrator::new(
        settings,
        Arc::new(registry),
        Arc::new(WavPacedBackend { pcm }),
        injector,
        focus,
        emitter.clone(),
    );
    orch.toast_dwell = Duration::from_millis(10);
    orch.finalize_timeout = Duration::from_secs(10);
    orch.focus_restore_delay = Duration::ZERO;
    let hist_dir = tempfile::tempdir().unwrap();
    orch.history_dir = Some(hist_dir.path().to_path_buf());
    let orch = orch.into_arc();

    orch.begin().await.unwrap();
    // 等音频按真实节奏全部推完，立刻松手（最坏时序：不留任何自然尾音）；
    // 松手走生产同款宽限期路径（release_grace = 500ms 生产默认值），
    // 宽限期继续收音把仍在传输链路上的句尾接进 session，倒计时到点才结束
    tokio::time::sleep(Duration::from_millis(audio_ms + 150)).await;
    orch.on_hotkey_hold(false).await;
    assert_eq!(
        orch.state(),
        OrchestratorState::Listening,
        "松手后宽限期内应保持 Listening 继续收音"
    );
    // 宽限期 500ms + finalize + 发送（Success 经 toast_dwell 已自动回 Idle，以发送为准）
    tokio::time::sleep(Duration::from_millis(3000)).await;
    assert_ne!(orch.state(), OrchestratorState::Listening);

    let sent = sent.lock().unwrap();
    assert_eq!(sent.len(), 1, "应发送一次: {sent:?}");
    let final_text = &sent[0];
    eprintln!("全链路最终发送文本：{final_text}");
    let records = kotone_core::history::list_in(hist_dir.path()).unwrap();
    for r in &records {
        eprintln!(
            "history: audio_ms={} text={:?} outcome={:?}",
            r.audio_ms, r.final_text, r.outcome
        );
    }
    assert!(
        final_text.contains(&expected),
        "句尾「{expected}」丢失！最终文本：{final_text}"
    );
}
