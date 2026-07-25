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
    let mut settings = Settings::default();
    settings.stt_engine = "mock-stream".into();
    settings.auto_send = auto_send;
    settings.active_profile_id = None; // 测试不依赖真实 ~/.kotone
    settings.eval_recording = false; // 默认不录档（需要时测试自行开启并覆盖 eval_dir）
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
        // NeverReadyEngine 恒未就绪（whisper/sherpa 真机均已就绪，不能再用它们模拟）
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
        // NeverReadyEngine 恒未就绪（whisper/sherpa 真机均已就绪，不能再用它们模拟）
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
    let focus: Arc<dyn FocusBackend> =
        Arc::new(MockFocusBackend::new(log.clone(), 0xBEEF, true));
    let injector: Arc<dyn Injector> = Arc::new(LoggingInjector { log: log.clone() });
    let (orch, _emitter) = make_orchestrator_full(false, injector, focus);

    orch.begin().await.unwrap();
    tokio::time::sleep(Duration::from_millis(30)).await;
    orch.end().await.unwrap();
    assert_eq!(orch.state(), OrchestratorState::Preview);
    orch.confirm_send().await.unwrap();
    assert_eq!(orch.state(), OrchestratorState::Success);

    let ops = log.lock().unwrap().clone();
    assert_eq!(ops.first().map(String::as_str), Some("capture"), "begin 应捕获前台窗口");
    let restore_pos = ops.iter().position(|s| s == "restore:48879"); // 0xBEEF
    let send_pos = ops.iter().position(|s| s.starts_with("send:"));
    assert!(restore_pos.is_some(), "发送前应恢复记录的 hwnd: {ops:?}");
    assert!(
        restore_pos.unwrap() < send_pos.unwrap(),
        "必须先恢复焦点再注入: {ops:?}"
    );
}

/// 焦点恢复失败（窗口已关闭）不阻断流程：注入器仍被调用，由原前台校验决定成败
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
        "恢复失败也应继续走注入（由前台校验报错）: {ops:?}"
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
    assert_eq!(orch.state(), OrchestratorState::Success, "Preview 态热键应确认发送而非取消");
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

    let (orch, _emitter) =
        make_orchestrator_with(false, Arc::new(SlowInjector));
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
    assert_eq!(orch.state(), OrchestratorState::Idle, "Sending 态热键应取消");
    let _ = handle.await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(orch.state(), OrchestratorState::Idle, "取消后不应被过期结果改写");
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
    assert_eq!(orch.state(), OrchestratorState::Preview, "Preview 态按下事件应被忽略");
    assert!(sent.lock().unwrap().is_empty());

    // 之后仍可正常确认发送
    orch.confirm_send().await.unwrap();
    assert_eq!(orch.state(), OrchestratorState::Success);
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
    (Arc::new(orch), emitter, sent)
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
    let pcm = kotone_core::eval::read_wav(&dir.path().join(format!("{}.wav", s.session_id))).unwrap();
    assert_eq!(pcm.len() as u64, s.audio_ms * 16, "wav 采样数应与 audioMs 一致");
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
        kotone_core::eval::list_sessions_at(dir.path()).unwrap().is_empty(),
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
        kotone_core::eval::list_sessions_at(dir.path()).unwrap().is_empty(),
        "evalRecording 关闭时不应录档"
    );
}
