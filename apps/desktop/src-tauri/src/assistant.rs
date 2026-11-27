//! Desktop composition for reminders. Scheduling and AI planning live outside
//! the shell; this adapter owns speech and the non-activating presentation window.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use kotone_core::assistant::{
    is_legacy_reminder_session, AssistantConfig, AssistantHandler, AssistantPlan, AssistantRequest,
    TtsEngine, LEGACY_REMINDER_SESSION_PREFIX,
};
use kotone_core::automation::AutomationScheduler;
use kotone_core::cancel::CancellationToken;
use kotone_core::reminders::{DeliveryKind, Reminder};
use kotone_core::settings::Settings;
use kotone_core::speech::{SpeechOutput, SpeechRequest, SpeechVoice};
use kotone_platform_windows::output_volume::OutputVolumeLease;
use tauri::{AppHandle, Emitter, Manager};

pub struct AssistantRuntime {
    pub scheduler: Result<Arc<AutomationScheduler>, String>,
    pub handler: Arc<dyn AssistantHandler>,
    system_speaker: Result<Arc<dyn SpeechOutput>, String>,
    local_speaker: Mutex<Option<(std::path::PathBuf, Arc<kotone_tts::LocalSpeechOutput>)>>,
    /// 在线语音（OpenAI 兼容 / 火山）：每种一个输出线程，配置在每次播报时重新解析。
    online_speakers: Mutex<Vec<(TtsEngine, Arc<kotone_tts::LocalSpeechOutput>)>>,
    typed_request: Mutex<Option<CancellationToken>>,
    presentation: Mutex<Option<DanmakuPresentation>>,
    presentation_generation: AtomicU64,
    presentation_done: tokio::sync::Notify,
    /// 一次只播一样（提醒、回答、填充话）；边生成边念的回答也拿着它（[`hold_output`]）。
    output_lock: Arc<tokio::sync::Mutex<()>>,
    output_cancel: OutputCancellation,
    // A cancelled session may still be joining its capture thread while a new
    // session opens. Each microphone lease retains its own admission slot.
    active_captures: AtomicUsize,
    capture_output_volume: Mutex<CaptureOutputVolume>,
}

enum CaptureOutputVolume {
    Open(Option<OutputVolumeLease>),
    Closed,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DanmakuPresentation {
    id: u64,
    text: String,
    started_at_ms: u64,
    expires_at_ms: u64,
    speed_px_per_sec: u32,
    font_size: u32,
    top_offset_px: u32,
    color: String,
    emote: String,
}

#[derive(Default)]
struct OutputCancellation(Mutex<CancellationToken>);

impl OutputCancellation {
    // Called only after taking output_lock, so stopping one presentation never
    // discards a different reminder waiting for its turn.
    fn begin(&self) -> CancellationToken {
        let token = CancellationToken::default();
        *self.0.lock().unwrap() = token.clone();
        token
    }

    fn stop(&self) -> std::sync::MutexGuard<'_, CancellationToken> {
        let token = self.0.lock().unwrap();
        token.cancel();
        token
    }
}

impl AssistantRuntime {
    /// 系统语音不可用时的原因（统一模型清单 TTS 条目的就绪状态）。
    pub fn system_speech_error(&self) -> Option<String> {
        self.system_speaker.as_ref().err().cloned()
    }

    fn cancel_speech(&self) -> Result<(), String> {
        let local = self
            .local_speaker
            .lock()
            .unwrap()
            .as_ref()
            .map(|(_, speaker)| speaker.clone());
        // Always cancel both engines, including a voice that was queued before a
        // settings change. Do not hold the cache lock while waiting for playback.
        let system_result = self
            .system_speaker
            .as_ref()
            .map(|s| s.cancel())
            .unwrap_or(Ok(()));
        let local_result = local.map(|s| s.cancel()).unwrap_or(Ok(()));
        let online: Vec<_> = self
            .online_speakers
            .lock()
            .unwrap()
            .iter()
            .map(|(_, speaker)| speaker.clone())
            .collect();
        let online_result = online
            .into_iter()
            .map(|s| s.cancel())
            .fold(Ok(()), Result::and);
        system_result.and(local_result).and(online_result)
    }
}

/// 实际用哪个语音引擎：“自动”（默认）时跟着她用的模型走（ADR 048）——她的 API 连接那一家有声音、有 Key
/// （DeepSeek / MiniMax / 阿里云百炼）就用那家，否则系统语音。
pub fn effective_engine(app: &AppHandle, config: &AssistantConfig) -> TtsEngine {
    config.tts_engine_auto(|| {
        let companion_enabled = app.try_state::<crate::SharedState>().is_some_and(|state| {
            !state
                .settings
                .read()
                .unwrap()
                .ui
                .legacy_upgrade
                .uses_legacy_assistant()
        });
        if !companion_enabled {
            return None;
        }
        crate::tts_online::her_voice(app)
    })
}

fn selected_speaker(
    app: &AppHandle,
    config: &AssistantConfig,
) -> Result<Arc<dyn SpeechOutput>, String> {
    speaker_for_engine(app, effective_engine(app, config))
}

pub(crate) fn speaker_for_engine(
    app: &AppHandle,
    engine: TtsEngine,
) -> Result<Arc<dyn SpeechOutput>, String> {
    let runtime = app.state::<AssistantRuntime>();
    if engine == TtsEngine::System {
        return runtime.system_speaker.clone();
    }
    let online_kind = kotone_tts::catalog::kind_of(engine);
    if let Some(kind) = online_kind {
        let mut online = runtime.online_speakers.lock().unwrap();
        if let Some((_, speaker)) = online
            .iter()
            .find(|(e, speaker)| *e == engine && !speaker.is_closed())
        {
            return Ok(speaker.clone());
        }
        let resolver_app = app.clone();
        let speaker = Arc::new(kotone_tts::LocalSpeechOutput::online(
            kind,
            Arc::new(move || crate::tts_online::config(&resolver_app, kind)),
        )?);
        online.retain(|(e, _)| *e != engine);
        online.push((engine, speaker.clone()));
        return Ok(speaker);
    }
    let settings = app.state::<crate::SharedState>();
    let path = kotone_stt::model::models_dir_from(&settings.settings.read().unwrap())
        .join("speech")
        .join(kotone_tts::KOKORO_MODEL_DIR);
    let mut local = runtime.local_speaker.lock().unwrap();
    if let Some((loaded_path, speaker)) = local.as_ref() {
        if loaded_path == &path && !speaker.is_closed() {
            return Ok(speaker.clone());
        }
        speaker.close()?;
    }
    let speaker = Arc::new(kotone_tts::LocalSpeechOutput::new(path.clone())?);
    *local = Some((path, speaker.clone()));
    Ok(speaker)
}

/// Prevent new output while the shared model directory is being moved. Close
/// the inference worker too: cancelling audio alone does not release weights.
pub fn prepare_model_migration(
    state: &AssistantRuntime,
) -> Result<tokio::sync::MutexGuard<'_, ()>, String> {
    let guard = state
        .output_lock
        .try_lock()
        .map_err(|_| "正在播报或显示提醒，请稍后迁移模型目录".to_string())?;
    let mut local = state.local_speaker.lock().unwrap();
    if let Some((_, speaker)) = local.as_ref() {
        speaker.close()?;
    }
    // Keep a timed-out worker in the cache so retrying migration must still
    // cross its shutdown barrier, rather than moving files under live inference.
    local.take();
    Ok(guard)
}

/// 助手入口的请求去向：
/// - 默认交给内置琴音 Agent（`agent.inGame`，ADR 014 §4）：立即返回，结果在后台播出；
/// - `agent.inGame = false` 时用旧的单次陪玩（截图 + OCR + 一次大模型调用，提醒 / 打字 /
///   回答三选一）；
/// - `assistant.route = agent` 时原文转交外部 Agent（经工具服务发布 `user.request`）。
///
/// 热键语音与文字入口都经过这里，因此两者行为一致。
struct RoutingHandler {
    app: AppHandle,
    settings: Arc<RwLock<Settings>>,
    legacy_reminders: kotone_ai::reminders::ReminderAssistant,
}

/// 请求来源：设置窗口（文字入口与语音按钮）/ 工具提交 / 热键语音（按会话 ID 前缀区分）。
fn request_source(session_id: &str) -> &'static str {
    if session_id.starts_with("assistant-wake-") {
        "wake"
    } else if session_id.starts_with("assistant-ui-") {
        "assistant-text"
    } else if session_id.starts_with("assistant-tool-") {
        "tool"
    } else {
        "assistant-voice"
    }
}

impl AssistantHandler for RoutingHandler {
    fn handle(
        &self,
        request: AssistantRequest,
        cancel: CancellationToken,
    ) -> kotone_core::assistant::AssistantFuture<'_> {
        let (route, in_game, legacy) = {
            let settings = self.settings.read().unwrap();
            (
                settings.assistant.route,
                settings.agent.in_game,
                settings.ui.legacy_upgrade.uses_legacy_assistant(),
            )
        };
        if legacy || is_legacy_reminder_session(&request.session_id) {
            return self.legacy_reminders.handle(request, cancel);
        }
        if route != kotone_core::assistant::AssistantRoute::Agent && in_game {
            return Box::pin(async move {
                if cancel.is_cancelled() {
                    return Err("助手指令已取消".into());
                }
                let message = crate::agent::submit_from_assistant(
                    &self.app,
                    request.text,
                    match request_source(&request.session_id) {
                        "assistant-text" => "assistant-text",
                        "tool" => "tool",
                        "wake" => "wake-voice",
                        _ => "voice",
                    },
                )?;
                Ok(AssistantPlan {
                    message,
                    reminders: vec![],
                })
            });
        }
        if route != kotone_core::assistant::AssistantRoute::Agent {
            let source = match request_source(&request.session_id) {
                "assistant-text" => crate::companion::Source::Text,
                "tool" => crate::companion::Source::Tool,
                _ => crate::companion::Source::Voice,
            };
            return Box::pin(async move {
                crate::companion::handle_assistant(&self.app, request.text, source, cancel).await
            });
        }
        Box::pin(async move {
            if cancel.is_cancelled() {
                return Err("助手指令已取消".into());
            }
            let submitted = crate::tool_service::submit_user_request(
                &self.app,
                &request.text,
                // 外部 Agent 的来源只有三种：唤醒也算语音
                match request_source(&request.session_id) {
                    "wake" => "assistant-voice",
                    other => other,
                },
            )?;
            let id = submitted["requestId"].as_str().unwrap_or_default();
            Ok(AssistantPlan {
                message: format!("已转交 Agent（请求 {id}）"),
                reminders: vec![],
            })
        })
    }
}

pub fn initialize(
    app: &AppHandle,
    settings: Arc<RwLock<Settings>>,
    connections: Arc<dyn kotone_core::connection::ConnectionResolver>,
) -> AssistantRuntime {
    let speaker = kotone_platform_windows::speech::WindowsSpeechOutput::new()
        .map(|speaker| Arc::new(speaker) as Arc<dyn SpeechOutput>);
    // 提醒与程序共用自动化调度器（ADR 013）；旧 reminders.json 只读导入一次，不改不删。
    // ADR 045：调度器只剩程序循环，提醒由程序里的 `say()` / `toast()` 投递（经 `deliver_text`）。
    let scheduler =
        AutomationScheduler::open(kotone_core::settings::kotone_dir().join("automation-runs.json"));
    if let Ok(scheduler) = &scheduler {
        match scheduler
            .import_legacy_reminders(&kotone_core::settings::kotone_dir().join("reminders.json"))
        {
            Ok(0) => {}
            Ok(n) => {
                kotone_core::log::log(&format!("automation: imported {n} legacy reminder records"))
            }
            Err(e) => kotone_core::log::log(&format!("automation: reminders import skipped: {e}")),
        }
        let handle = app.clone();
        scheduler.set_on_changed(Arc::new(move || {
            let _ = handle.emit("kotone://reminders-updated", ());
            let _ = handle.emit("kotone://automations-updated", ());
        }));
    }
    AssistantRuntime {
        scheduler,
        handler: Arc::new(RoutingHandler {
            app: app.clone(),
            legacy_reminders: kotone_ai::reminders::ReminderAssistant::new(
                connections,
                settings.clone(),
            ),
            settings,
        }),
        system_speaker: speaker,
        local_speaker: Mutex::new(None),
        online_speakers: Mutex::new(Vec::new()),
        typed_request: Mutex::new(None),
        presentation: Mutex::new(None),
        presentation_generation: AtomicU64::new(0),
        presentation_done: tokio::sync::Notify::new(),
        output_lock: Arc::new(tokio::sync::Mutex::new(())),
        output_cancel: OutputCancellation::default(),
        active_captures: AtomicUsize::new(0),
        capture_output_volume: Mutex::new(CaptureOutputVolume::Open(None)),
    }
}

pub fn start(app: &AppHandle) {
    let state = app.state::<AssistantRuntime>();
    if let Ok(scheduler) = &state.scheduler {
        let scheduler = scheduler.clone();
        tauri::async_runtime::spawn(async move {
            let _ = scheduler.start().await;
        });
    }
}

pub fn shutdown(app: &AppHandle) {
    if let Some(state) = app.try_state::<AssistantRuntime>() {
        *state.capture_output_volume.lock().unwrap() = CaptureOutputVolume::Closed;
        drop(state.output_cancel.stop());
        if let Some(token) = state.typed_request.lock().unwrap().as_ref() {
            token.cancel();
        }
        if let Ok(scheduler) = &state.scheduler {
            scheduler.shutdown();
        }
        let _ = state.cancel_speech();
    }
}

pub fn silence(app: &AppHandle) {
    if let Some(state) = app.try_state::<AssistantRuntime>() {
        let _ = state.cancel_speech();
    }
}

/// 立即用当前语音说一句短话（唤醒应答“嗯？/我在”）：先停掉正在播的，播完才返回。
/// 阻塞，从普通线程调用；不经过提醒队列，也不改弹幕。
pub fn speak_now(app: &AppHandle, text: &str) -> Result<(), String> {
    let state = app.state::<AssistantRuntime>();
    if state.active_captures.load(Ordering::SeqCst) != 0 {
        return Err("正在录音，不播应答".into());
    }
    // 她正在边生成边念时喊了唤醒词：先停下（打断）
    if crate::voice_reply::interrupt() {
        crate::follow_up::interrupted();
    }
    state.cancel_speech()?;
    let config = app
        .state::<crate::SharedState>()
        .settings
        .read()
        .unwrap()
        .assistant
        .clone();
    let speaker = selected_speaker(app, &config)?;
    crate::voice_reply::remember_spoken(text);
    SPEAKING_NOW.fetch_add(1, Ordering::SeqCst);
    let result = speaker.speak(speech_request(
        effective_engine(app, &config),
        &config,
        text,
    ));
    SPEAKING_NOW.fetch_sub(1, Ordering::SeqCst);
    result
}

/// 正在用 [`speak_now`] 说的句数（应答、追问）。
static SPEAKING_NOW: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// 正在用 [`speak_now`] 说一句吗（通话里没有回声消除时，这时麦克风里是她自己，ADR 033）。
pub fn speaking_now() -> bool {
    SPEAKING_NOW.load(Ordering::SeqCst) != 0
}

/// 当前的语音引擎与一句话的合成请求（游戏语音“说给队友”用）。
pub fn speaker_and_request(
    app: &AppHandle,
    text: &str,
) -> Result<(Arc<dyn SpeechOutput>, kotone_core::speech::SpeechRequest), String> {
    let config = app
        .state::<crate::SharedState>()
        .settings
        .read()
        .unwrap()
        .assistant
        .clone();
    let speaker = selected_speaker(app, &config)?;
    let request = speech_request(effective_engine(app, &config), &config, text);
    Ok((speaker, request))
}

/// 插件用她的声音在玩家的扬声器上说一句（`Host::speak`，例如游戏语音说给队友的话玩家自己也听一遍）。
/// 和她自己的话走同一条路（[`take_turn_and_speak`]）：排在她的话、提醒、填充话后面，放的时候算她在出声。
/// 放完才返回。没放出来、又不是玩家让她停下的，照提醒的做法改用弹幕显示这句（玩家至少看得到），
/// 原因也交回给插件。
pub async fn speak_in_turn(app: &AppHandle, text: &str) -> Result<(), String> {
    let (speaker, request) = {
        let (app, text) = (app.clone(), text.to_string());
        // 换了模型目录时取扬声器要等旧的关掉（最多 10 秒）：不在异步线程里等
        tauri::async_runtime::spawn_blocking(move || speaker_and_request(&app, &text))
            .await
            .map_err(|e| e.to_string())??
    };
    let runtime = app.state::<AssistantRuntime>();
    let started = std::time::Instant::now();
    let waited_for_her = she_is_audible(app);
    let result = take_turn_and_speak(
        &runtime.output_lock,
        &runtime.output_cancel,
        &runtime.active_captures,
        speaker,
        request,
        None,
    )
    .await;
    if waited_for_her {
        kotone_core::log::log(&format!(
            "assistant: 插件的话排在她后面放（共 {} 毫秒）",
            started.elapsed().as_millis()
        ));
    }
    match result {
        Ok(()) => Ok(()),
        Err(Unspoken::Stopped) => Err("玩家让她停下了，这句没有放完".into()),
        Err(Unspoken::TooLate) => Err("过时了，这句没有放".into()),
        Err(Unspoken::Failed(error)) => {
            kotone_core::log::log(&format!("assistant: 插件的话没能放出来，改用弹幕：{error}"));
            if let Err(e) = deliver_text(app, text.to_string(), vec![DeliveryKind::Danmaku]).await {
                kotone_core::log::log(&format!("assistant: 弹幕也没能显示：{e}"));
            }
            Err(format!("没能在玩家这边放出来：{error}"))
        }
    }
}

/// 推送的语音（ADR 043 D3，`agent/push.rs` 排好了轮到它）：和插件的话走同一条路（排在她的话、提醒、填充话后面，放的时候
/// 算她在出声，跟着她停），只是有期限：到 `deadline` 还没轮到开口（她一直在说、麦克风一直开着）就不放了（过时了），
/// 返回 `Ok(false)`。放完返回 `Ok(true)`。放不出来（不是被停下、不是过时）时和插件的话一样改用弹幕显示这句。
pub async fn speak_push(
    app: &AppHandle,
    text: &str,
    deadline: std::time::Instant,
) -> Result<bool, String> {
    let (speaker, request) = {
        let (app, text) = (app.clone(), text.to_string());
        tauri::async_runtime::spawn_blocking(move || speaker_and_request(&app, &text))
            .await
            .map_err(|e| e.to_string())??
    };
    let runtime = app.state::<AssistantRuntime>();
    let result = take_turn_and_speak(
        &runtime.output_lock,
        &runtime.output_cancel,
        &runtime.active_captures,
        speaker,
        request,
        Some(deadline),
    )
    .await;
    match result {
        Ok(()) => Ok(true),
        Err(Unspoken::TooLate) => Ok(false),
        Err(Unspoken::Stopped) => Err("玩家让她停下了，这句没有放完".into()),
        Err(Unspoken::Failed(error)) => {
            kotone_core::log::log(&format!(
                "assistant: 推送的语音没能放出来，改用弹幕：{error}"
            ));
            if let Err(e) = deliver_text(app, text.to_string(), vec![DeliveryKind::Danmaku]).await {
                kotone_core::log::log(&format!("assistant: 弹幕也没能显示：{e}"));
            }
            Err(format!("没能在玩家这边放出来：{error}"))
        }
    }
}

/// 插件的一句话为什么没放出来。
#[derive(Debug, PartialEq)]
enum Unspoken {
    /// 玩家让她停下了（停止、呼叫键打断：[`stop_speech`]）。
    Stopped,
    /// 到期限还没轮到开口（推送的语音：过时了，[`speak_push`]）。
    TooLate,
    /// 别的原因：麦克风一直开着、语音出错、超时。
    Failed(String),
}

/// 麦克风开着时插件的话最多等这么久（同提醒、边生成边念的回答）。
const PLUGIN_CAPTURE_WAIT: Duration = Duration::from_secs(30);

/// 轮到了再说（插件的话，[`speak_in_turn`]）：
/// 1. 排队拿播报锁——她的话（边生成边念的回答、整段念的话）、提醒、填充话放完才轮到；放的时候一直拿着，
///    她下一句也等它放完，[`she_is_audible`] 也就是真的（音乐压低、通话里不把这句当成玩家说的）；
/// 2. 麦克风开着先不出声（免得录进去），一直开着就不放了；
/// 3. 记进“她最近说过的话”（通话里开着回声消除时认回声用），和她整段念的话一样；
/// 4. 玩家让她停下（[`stop_speech`]）、或者调用方不等了（future 被丢掉）时这句也停；
/// 5. 给了 `deadline`（推送的语音）：到那时还没拿到播报锁、麦克风还开着，就不放了（[`Unspoken::TooLate`]）；开口以后不看它。
async fn take_turn_and_speak(
    lock: &tokio::sync::Mutex<()>,
    outputs: &OutputCancellation,
    captures: &AtomicUsize,
    speaker: Arc<dyn SpeechOutput>,
    request: SpeechRequest,
    deadline: Option<std::time::Instant>,
) -> Result<(), Unspoken> {
    let _turn = match deadline {
        None => lock.lock().await,
        Some(at) => {
            match tokio::time::timeout_at(tokio::time::Instant::from_std(at), lock.lock()).await {
                Ok(turn) => turn,
                Err(_) => return Err(Unspoken::TooLate),
            }
        }
    };
    let stop = outputs.begin();
    let text = request.text.clone();
    const TOO_LATE: &str = "过时了";
    let spoken = until_output_stopped(&stop, async {
        let waited = std::time::Instant::now();
        while captures.load(Ordering::SeqCst) != 0 {
            if deadline.is_some_and(|at| std::time::Instant::now() >= at) {
                return Err(TOO_LATE.to_string());
            }
            if waited.elapsed() >= PLUGIN_CAPTURE_WAIT {
                return Err("麦克风一直开着，这句没有放".to_string());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        crate::voice_reply::remember_spoken(&text);
        let mut playing = StopOnDrop(Some(speaker.clone()));
        let task = queue_speech(speaker, request, captures, &stop)?;
        match tokio::time::timeout(Duration::from_secs(60), task).await {
            Ok(Ok(result)) => {
                playing.0 = None;
                result
            }
            Ok(Err(_)) => Err("语音工作线程异常".into()),
            Err(_) => Err("语音播报超时".into()),
        }
    })
    .await;
    match spoken {
        Ok(()) => {
            // 念完的时刻再记一次（“最近”从念完算）
            crate::voice_reply::remember_spoken(&text);
            Ok(())
        }
        Err(_) if stop.is_cancelled() => Err(Unspoken::Stopped),
        Err(error) if error == TOO_LATE => Err(Unspoken::TooLate),
        Err(error) => Err(Unspoken::Failed(error)),
    }
}

/// 这句还没放完就被丢下（停下、超时、调用方不等了）：语音也停。
struct StopOnDrop(Option<Arc<dyn SpeechOutput>>);

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        if let Some(speaker) = self.0.take() {
            let _ = speaker.cancel();
        }
    }
}

/// 她这会儿在出声没有——“她在说话”只有这一种判断：音乐压低（`Host::she_is_audible`）、唤醒和通话的
/// 半双工（没有回声消除时她说话就不听）都用它。算进去的：边生成边念的回答（[`crate::voice_reply`]）、
/// 应答和追问（[`speak_now`]）、拿着播报锁的一切（整段念的话、提醒、填充话、插件的话）。
pub fn she_is_audible(app: &AppHandle) -> bool {
    audible(output_busy(app))
}

fn audible(output_busy: bool) -> bool {
    output_busy || speaking_now() || crate::voice_reply::active()
}

/// 先把这些短句合成进缓存（唤醒应答“嗯？/我在”），用时立刻出声、不用现场请求在线语音。
pub fn prefetch(app: &AppHandle, texts: &[&str]) {
    let config = app
        .state::<crate::SharedState>()
        .settings
        .read()
        .unwrap()
        .assistant
        .clone();
    let Ok(speaker) = selected_speaker(app, &config) else {
        return;
    };
    let engine = effective_engine(app, &config);
    for text in texts {
        speaker.prefetch(speech_request(engine, &config, text));
    }
}

/// 现在的语音合成“引擎 + 声音 + 语速”（填充话缓存按它区分，见 [`crate::fillers`]）。
/// 系统语音不经过合成缓存，返回 None。
pub fn speech_signature(app: &AppHandle) -> Option<String> {
    let config = app
        .state::<crate::SharedState>()
        .settings
        .read()
        .unwrap()
        .assistant
        .clone();
    let engine = effective_engine(app, &config);
    if engine == TtsEngine::System {
        return None;
    }
    let request = speech_request(engine, &config, "");
    Some(format!(
        "{engine:?}|{}|{}",
        request.voice_id.unwrap_or_default(),
        request.rate
    ))
}

/// 用当前语音把一句话合成进缓存，不播（阻塞，从普通线程调用）。之后播这句时直接出声。
/// 系统语音不需要合成，直接返回。
pub fn synthesize_cached(app: &AppHandle, text: &str) -> Result<(), String> {
    let config = app
        .state::<crate::SharedState>()
        .settings
        .read()
        .unwrap()
        .assistant
        .clone();
    let engine = effective_engine(app, &config);
    if engine == TtsEngine::System {
        return Ok(());
    }
    let speaker = speaker_for_engine(app, engine)?;
    speaker
        .synthesize(speech_request(engine, &config, text))
        .map(|_| ())
}

/// 边生成边念（ADR 029 §六）用的合成方式与声音：DeepSeek 用流式接口，Kokoro / 在线语音逐句合成，
/// 系统语音逐句念。声音、音量、语速与整段播报相同（角色的声音优先）。
pub fn stream_engine(
    app: &AppHandle,
) -> Result<(kotone_tts::StreamEngine, kotone_tts::StreamOptions), String> {
    let config = app
        .state::<crate::SharedState>()
        .settings
        .read()
        .unwrap()
        .assistant
        .clone();
    let engine = effective_engine(app, &config);
    let request = speech_request(engine, &config, "");
    let stream_engine = match engine {
        TtsEngine::Deepseek => kotone_tts::StreamEngine::Deepseek(crate::tts_online::config(
            app,
            kotone_tts::online::OnlineKind::Deepseek,
        )?),
        TtsEngine::System => {
            kotone_tts::StreamEngine::Speak(app.state::<AssistantRuntime>().system_speaker.clone()?)
        }
        _ => kotone_tts::StreamEngine::Synthesize(speaker_for_engine(app, engine)?),
    };
    Ok((
        stream_engine,
        kotone_tts::StreamOptions {
            voice_id: request.voice_id,
            volume: request.volume,
            rate: request.rate,
            ..kotone_tts::StreamOptions::default()
        },
    ))
}

/// 排队拿播报（前面的提醒、填充话播完才轮到）：边生成边念的回答念完才放开。
pub async fn hold_output(app: &AppHandle) -> Option<tokio::sync::OwnedMutexGuard<()>> {
    let lock = app.try_state::<AssistantRuntime>()?.output_lock.clone();
    Some(lock.lock_owned().await)
}

/// 麦克风开着吗（正在录音时不出声，免得把自己的话录进去）。
pub fn capturing(app: &AppHandle) -> bool {
    app.try_state::<AssistantRuntime>()
        .is_some_and(|state| state.active_captures.load(Ordering::SeqCst) != 0)
}

/// 马上用弹幕显示一句（边生成边念的回答：声音另外在念，不排播报队）。
pub fn show_danmaku(app: &AppHandle, text: &str) {
    let config = app
        .state::<crate::SharedState>()
        .settings
        .read()
        .unwrap()
        .assistant
        .clone();
    if let Err(error) = present_danmaku_active(app, text, &config) {
        kotone_core::log::log(&format!("assistant: 弹幕没能显示：{error}"));
    }
}

/// 先把要念的话合成好（不阻塞异步线程）。失败不要紧：播的时候还会再合成。
pub async fn prepare_speech(app: &AppHandle, text: &str) {
    let app = app.clone();
    let text = text.to_string();
    let _ = tauri::async_runtime::spawn_blocking(move || synthesize_cached(&app, &text)).await;
}

/// 正在播报（语音或弹幕，含正在等着播的提醒）吗。
pub fn output_busy(app: &AppHandle) -> bool {
    app.try_state::<AssistantRuntime>()
        .is_some_and(|state| state.output_lock.try_lock().is_err())
}

/// 等正在进行的播报结束（最多 `max`）。
pub async fn wait_output_idle(app: &AppHandle, max: Duration) {
    let started = std::time::Instant::now();
    while output_busy(app) && started.elapsed() < max {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// 呼叫键打断（ADR 029 §五）：她正在说（或回答正等着念）就立刻停下，气泡收起那句话，
/// 回答念完后也不再接着听（[`crate::follow_up::interrupted`]）。阻塞（停播最多等语音服务 2 秒）。
/// 返回是否打断了什么。
pub fn interrupt_speech(app: &AppHandle) -> bool {
    if !output_busy(app) && !crate::bubble::is_speaking(app) && !crate::voice_reply::active() {
        return false;
    }
    crate::follow_up::interrupted();
    // 边生成边念的回答先停（等输出设备关掉），对话里只留已经念出来的部分
    crate::voice_reply::interrupt();
    stop_speech(app.clone());
    crate::bubble::stop_speaking(app);
    kotone_core::log::log("assistant: call key interrupted the speech");
    true
}

/// 按了呼叫键：这一下要开始听（空闲或停在提示上）时，先打断她正在说的话，再开始听。
/// 正在听时再按是“说完了”，不打断。
pub async fn interrupt_before_call(app: &AppHandle) {
    let state = app.state::<crate::SharedState>().orchestrator.state();
    if !matches!(
        state,
        kotone_core::orchestrator::OrchestratorState::Idle
            | kotone_core::orchestrator::OrchestratorState::Success
            | kotone_core::orchestrator::OrchestratorState::Error
    ) {
        return;
    }
    let app = app.clone();
    let _ = tauri::async_runtime::spawn_blocking(move || interrupt_speech(&app)).await;
}

pub fn before_capture(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<AssistantRuntime>();
    let lower_speaker_volume = app
        .state::<crate::SharedState>()
        .settings
        .read()
        .unwrap()
        .lower_speaker_volume_while_listening;
    {
        // Pair the volume lease with actual captures, including overlapping capture teardown.
        let mut volume = state.capture_output_volume.lock().unwrap();
        state.active_captures.fetch_add(1, Ordering::SeqCst);
        if let CaptureOutputVolume::Open(lease @ None) = &mut *volume {
            if lower_speaker_volume {
                match OutputVolumeLease::acquire() {
                    Ok(acquired) => *lease = Some(acquired),
                    Err(error) => kotone_core::log::log(&format!(
                        "录音开始前降低扬声器音量失败（继续录音）：{error}"
                    )),
                }
            }
        }
    }
    // 停播失败（语音服务忙或已停止）不该让录音开不了：记下来，照常开始。
    if let Err(error) = state.cancel_speech() {
        kotone_core::log::log(&format!("录音开始前停止播报失败（继续录音）：{error}"));
    }
    // 边生成边念的回答也停（等输出设备关掉才开麦）：玩家开口就是打断她。
    // 通话里开着回声消除、玩家在她说话时开口（ADR 033）：只压低着，识别出来再决定停不停。
    if crate::voice_reply::ducked_for().is_none() && crate::voice_reply::interrupt() {
        crate::follow_up::interrupted();
    }
    Ok(())
}

pub fn after_capture(app: &AppHandle) {
    if let Some(state) = app.try_state::<AssistantRuntime>() {
        let mut volume = state.capture_output_volume.lock().unwrap();
        if state.active_captures.load(Ordering::SeqCst) == 1 {
            if let CaptureOutputVolume::Open(lease) = &mut *volume {
                drop(lease.take());
            }
        }
        state.active_captures.fetch_sub(1, Ordering::SeqCst);
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

struct DesktopDelivery {
    app: AppHandle,
    /// 用旧语音引擎（0.1 升级上来的提醒，[`AssistantConfig::legacy_tts_engine`]）。
    legacy_voice: bool,
}

// Dropping a future (cancellation, shutdown, timeout) must also stop native work.
struct OutputGuard {
    app: AppHandle,
    speaker: Option<Arc<dyn SpeechOutput>>,
    visual: Option<u64>,
}

impl Drop for OutputGuard {
    fn drop(&mut self) {
        if let Some(speaker) = &self.speaker {
            let _ = speaker.cancel();
        }
        if let Some(id) = self.visual {
            hide_danmaku(&self.app, id);
        }
    }
}

/// 一次投递的 future（以前 core 的 `ReminderDelivery` 端口，ADR 045 起只有 [`deliver_text`] 用）。
type DeliveryFuture<'a> =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send + 'a>>;

impl DesktopDelivery {
    fn deliver<'a>(
        &'a self,
        reminder: &'a Reminder,
        cancel: CancellationToken,
    ) -> DeliveryFuture<'a> {
        Box::pin(async move {
            let runtime = self.app.state::<AssistantRuntime>();
            let _output = tokio::select! {
                biased;
                _ = cancel.cancelled() => return Err("提醒已取消".into()),
                lock = runtime.output_lock.lock() => lock,
            };
            let output_cancel = runtime.output_cancel.begin();
            until_output_stopped(&output_cancel, async {
                let mut guard = OutputGuard {
                    app: self.app.clone(),
                    speaker: None,
                    visual: None,
                };
                let config = self
                    .app
                    .state::<crate::SharedState>()
                    .settings
                    .read()
                    .unwrap()
                    .assistant
                    .clone();
                let mut errors = Vec::new();
                let mut use_voice = reminder.delivery.contains(&DeliveryKind::Voice);
                let mut use_visual = reminder.delivery.contains(&DeliveryKind::Danmaku);
                // A reminder's voice must not be transcribed as a new user command.
                // Wait briefly for an active microphone, without blocking cancellation.
                let wait_started = std::time::Instant::now();
                loop {
                    let listening = runtime.active_captures.load(Ordering::SeqCst) != 0;
                    if !listening || !use_voice {
                        break;
                    }
                    if wait_started.elapsed() > Duration::from_secs(30) {
                        errors.push("录音持续进行，未播报语音".to_string());
                        use_voice = false;
                        use_visual = true;
                        break;
                    }
                    tokio::select! {
                        _ = cancel.cancelled() => return Err("提醒已取消".into()),
                        _ = tokio::time::sleep(Duration::from_millis(100)) => {}
                    }
                }
                if cancel.is_cancelled() {
                    return Err("提醒已取消".into());
                }
                if use_visual {
                    match present_danmaku(&self.app, &reminder.text, &config, &output_cancel) {
                        Ok(id) => {
                            guard.visual = Some(id);
                        }
                        Err(error) => errors.push(error),
                    }
                }
                let mut speech_error = None;
                if use_voice {
                    let engine = if self.legacy_voice {
                        config.legacy_tts_engine()
                    } else {
                        effective_engine(&self.app, &config)
                    };
                    let speaker = speaker_for_engine(&self.app, engine);
                    match speaker {
                        Ok(speaker) => {
                            guard.speaker = Some(speaker.clone());
                            match queue_speech(
                                speaker.clone(),
                                speech_request(
                                    engine,
                                    &config,
                                    &reminder.text,
                                ),
                                &runtime.active_captures,
                                &output_cancel,
                            ) {
                                Ok(task) => {
                                tokio::select! {
                                    _ = cancel.cancelled() => {
                                        return Err("提醒已取消".into());
                                    }
                                    result = tokio::time::timeout(Duration::from_secs(60), task) => {
                                        match result {
                                            Ok(Ok(Ok(()))) => { guard.speaker = None; },
                                            Ok(Ok(Err(error))) => { guard.speaker = None; speech_error = Some(error); },
                                            Ok(Err(_)) => speech_error = Some("语音工作线程异常".into()),
                                            Err(_) => { let _ = speaker.cancel(); speech_error = Some("语音播报超时".into()); }
                                        }
                                    }
                                }
                                }
                                Err(error) => { guard.speaker = None; speech_error = Some(error); }
                            }
                        }
                        Err(error) => speech_error = Some(error),
                    }
                }
                if let Some(error) = speech_error {
                    errors.push(format!("语音未完成：{error}"));
                    if !cancel.is_cancelled() && !use_visual {
                        match present_danmaku(&self.app, &reminder.text, &config, &output_cancel) {
                            Ok(id) => {
                                guard.visual = Some(id);
                            }
                            Err(error) => errors.push(error),
                        }
                    }
                }
                // Wait for the actual scrolling animation, with a native expiry fallback.
                if let Some(id) = guard.visual {
                    tokio::select! {
                        biased;
                        _ = cancel.cancelled() => return Err("提醒已取消".into()),
                        _ = wait_danmaku(&runtime, id) => {},
                    }
                }
                if errors.is_empty() {
                    Ok(())
                } else {
                    Err(errors.join("；"))
                }
                })
            .await
        })
    }
}

fn speech_request(
    engine: TtsEngine,
    config: &kotone_core::assistant::AssistantConfig,
    text: &str,
) -> SpeechRequest {
    // 声音编号可带提供方前缀（`system/…`、`kokoro/…`）；在线引擎的声音在模型配置里，
    // 每次播报时由合成线程解析。属于其他提供方的声音会在合成时报错，不会换声音。
    // 当前角色有自己的声音、并且属于正在用的引擎时，用角色的（换角色声音跟着换）。
    let role_voice = crate::roles::cached_voice();
    let role_voice = role_voice.trim();
    let provider = kotone_tts::catalog::provider_of(engine);
    let role_voice = role_voice
        .split_once('/')
        .filter(|(p, v)| *p == provider && !v.is_empty())
        .map(|_| role_voice.to_string());
    let voice = match (engine, role_voice) {
        (TtsEngine::System, Some(v)) => v.trim_start_matches("system/").to_string(),
        (_, Some(v)) => v,
        (TtsEngine::Kokoro, None) => config.kokoro_voice_id.trim().to_string(),
        (TtsEngine::System, None) => {
            let id = config.voice_id.trim();
            id.strip_prefix("system/").unwrap_or(id).to_string()
        }
        // 在线引擎的声音在模型配置里，合成时解析
        (_, None) => String::new(),
    };
    SpeechRequest {
        text: text.to_string(),
        voice_id: (!voice.is_empty()).then_some(voice),
        volume: config.speech_volume,
        rate: config.speech_rate,
    }
}

async fn until_output_stopped<T>(
    cancel: &CancellationToken,
    output: impl std::future::Future<Output = Result<T, String>>,
) -> Result<T, String> {
    tokio::select! {
        biased;
        _ = cancel.cancelled() => Err("提醒输出已停止".into()),
        result = output => result,
    }
}

/// 语音队列满了最多等这么久。
const QUEUE_FULL_WAIT: Duration = Duration::from_secs(3);

fn queue_speech(
    speaker: Arc<dyn SpeechOutput>,
    request: SpeechRequest,
    active_captures: &AtomicUsize,
    cancel: &CancellationToken,
) -> Result<tokio::task::JoinHandle<Result<(), String>>, String> {
    cancel
        .run_if_active(|| {
            // Admit the ticket before Stop purges the engine. A worker starting
            // late must retain that older ticket, never fetch a post-Stop one.
            let generation = speaker.generation();
            if active_captures.load(Ordering::SeqCst) != 0 {
                return Err("录音已开始，已停止语音提醒".into());
            }
            Ok(tokio::task::spawn_blocking(move || {
                // 本地语音的工作线程队列只有 2 格（预取、合成填充话、边生成边念的合成也往里放）：
                // 满了稍等再试，不丢这句（同 `kotone_tts` 合成时的重试）。被停下时票据失效，马上返回。
                let deadline = std::time::Instant::now() + QUEUE_FULL_WAIT;
                loop {
                    match speaker.speak_if_current(request.clone(), generation) {
                        Err(error)
                            if error.contains("队列已满")
                                && std::time::Instant::now() < deadline =>
                        {
                            std::thread::sleep(Duration::from_millis(100));
                        }
                        other => return other,
                    }
                }
            }))
        })
        .ok_or_else(|| "提醒输出已停止".to_string())?
}

fn present_danmaku(
    app: &AppHandle,
    text: &str,
    config: &AssistantConfig,
    cancel: &CancellationToken,
) -> Result<u64, String> {
    // Stop cancels this same token before hiding the window. A concurrent
    // fallback therefore either finishes showing before Stop hides it, or is
    // rejected; it cannot resurrect a presentation after Stop has returned.
    cancel
        .run_if_active(|| present_danmaku_active(app, text, config))
        .ok_or_else(|| "提醒输出已停止".to_string())?
}

fn present_danmaku_active(
    app: &AppHandle,
    text: &str,
    config: &AssistantConfig,
) -> Result<u64, String> {
    if crate::fullscreen::is_exclusive_fullscreen_active() == Some(true) {
        return Err(
            "独占全屏游戏无法使用弹幕提醒，请切换为无边框全屏或窗口化，或使用语音提醒".into(),
        );
    }
    let window = app.get_webview_window("danmaku").ok_or("弹幕窗口不可用")?;
    position_on_foreground_monitor(&window)?;
    window
        .set_ignore_cursor_events(true)
        .map_err(|e| e.to_string())?;
    let state = app.state::<AssistantRuntime>();
    let id = state.presentation_generation.fetch_add(1, Ordering::SeqCst) + 1;
    let started_at_ms = now_ms();
    let presentation = DanmakuPresentation {
        id,
        text: text.to_string(),
        started_at_ms,
        expires_at_ms: started_at_ms + 60_000,
        speed_px_per_sec: config.danmaku_speed_px_per_sec,
        font_size: config.danmaku_font_size,
        top_offset_px: config.danmaku_top_offset_px,
        color: config.danmaku_color.clone(),
        emote: config.danmaku_emote.clone(),
    };
    *state.presentation.lock().unwrap() = Some(presentation.clone());
    if let Err(error) = app.emit_to("danmaku", "kotone://danmaku", &presentation) {
        hide_danmaku(app, id);
        return Err(error.to_string());
    }
    crate::show_window_no_focus(&window);
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(60)).await;
        hide_danmaku(&handle, id);
    });
    Ok(id)
}

fn hide_danmaku(app: &AppHandle, id: u64) {
    let state = app.state::<AssistantRuntime>();
    let mut presentation = state.presentation.lock().unwrap();
    if presentation.as_ref().map(|p| p.id) != Some(id) {
        return;
    }
    *presentation = None;
    if let Some(window) = app.get_webview_window("danmaku") {
        crate::hide_window(&window);
    }
    let _ = app.emit_to(
        "danmaku",
        "kotone://danmaku",
        Option::<DanmakuPresentation>::None,
    );
    state.presentation_done.notify_waiters();
}

async fn wait_danmaku(state: &AssistantRuntime, id: u64) {
    loop {
        // Register before inspecting state so a completion between the check and
        // await cannot be lost (Notify::notify_waiters does not retain a permit).
        let notified = state.presentation_done.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        if state.presentation.lock().unwrap().as_ref().map(|p| p.id) != Some(id) {
            return;
        }
        notified.await;
    }
}

#[tauri::command]
pub fn finish_danmaku(app: AppHandle, id: u64) {
    hide_danmaku(&app, id);
}

fn position_on_foreground_monitor(window: &tauri::WebviewWindow) -> Result<(), String> {
    #[cfg(windows)]
    {
        use windows::Win32::Graphics::Gdi::{
            GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
        };
        use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let ok = unsafe {
            GetMonitorInfoW(
                MonitorFromWindow(GetForegroundWindow(), MONITOR_DEFAULTTONEAREST),
                &mut info,
            )
        };
        if ok.as_bool() {
            let r = info.rcMonitor;
            window
                .set_position(tauri::PhysicalPosition::new(r.left, r.top))
                .map_err(|e| e.to_string())?;
            window
                .set_size(tauri::PhysicalSize::new(
                    (r.right - r.left) as u32,
                    (r.bottom - r.top) as u32,
                ))
                .map_err(|e| e.to_string())?;
            return Ok(());
        }
    }
    let monitor = window
        .primary_monitor()
        .map_err(|e| e.to_string())?
        .ok_or("无法确定弹幕显示器")?;
    window
        .set_position(*monitor.position())
        .map_err(|e| e.to_string())?;
    window.set_size(*monitor.size()).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn list_reminders(state: tauri::State<AssistantRuntime>) -> Result<Vec<Reminder>, String> {
    Ok(state.scheduler.as_ref().map_err(Clone::clone)?.list())
}

#[tauri::command]
pub fn cancel_reminder(state: tauri::State<AssistantRuntime>, id: String) -> Result<bool, String> {
    state.scheduler.as_ref().map_err(Clone::clone)?.cancel(&id)
}

#[tauri::command]
pub async fn assistant_submit_text(
    app: AppHandle,
    text: String,
    legacy_reminder: Option<bool>,
) -> Result<AssistantPlan, String> {
    let session_prefix = if legacy_reminder.unwrap_or(false) {
        "legacy-reminder-ui"
    } else {
        "assistant-ui"
    };
    submit_text(&app, text, session_prefix).await
}

/// 与文字入口相同的提交路径（工具服务 `assistant.submit` 以 `assistant-tool` 前缀调用）。
pub async fn submit_text(
    app: &AppHandle,
    text: String,
    session_prefix: &str,
) -> Result<AssistantPlan, String> {
    let state = app.state::<AssistantRuntime>();
    let scheduler = state.scheduler.as_ref().map_err(Clone::clone)?.clone();
    let cancel = CancellationToken::default();
    {
        let mut slot = state.typed_request.lock().unwrap();
        if slot.is_some() {
            return Err("上一条助手指令仍在处理".into());
        }
        *slot = Some(cancel.clone());
    }
    struct RequestGuard<'a>(&'a Mutex<Option<CancellationToken>>);
    impl Drop for RequestGuard<'_> {
        fn drop(&mut self) {
            if let Some(token) = self.0.lock().unwrap().take() {
                token.cancel();
            }
        }
    }
    let _request_guard = RequestGuard(&state.typed_request);
    let voice_only = app
        .state::<crate::SharedState>()
        .settings
        .read()
        .unwrap()
        .ui
        .legacy_upgrade
        .uses_legacy_assistant();
    let marker = if voice_only && !is_legacy_reminder_session(session_prefix) {
        LEGACY_REMINDER_SESSION_PREFIX
    } else {
        ""
    };
    let request = AssistantRequest {
        session_id: format!(
            "{marker}{session_prefix}-{}",
            kotone_core::eval::new_session_id()
        ),
        text,
        received_at_ms: now_ms(),
    };
    let id = request.session_id.clone();
    let received = request.received_at_ms;
    let result = state
        .handler
        .handle(request, cancel.clone())
        .await
        .and_then(|plan| {
            cancel
                .run_if_active(|| scheduler.schedule_batch(&id, received, &plan.reminders))
                .ok_or_else(|| "助手指令已取消".to_string())??;
            Ok(plan)
        });
    result
}

/// 向用户交付一段文字（Agent 回复）：复用提醒的交付路径——输出互斥、等待麦克风释放、
/// 语音与弹幕、失败降级。`kinds` 为空时使用助手设置中的默认提醒方式。future 被丢弃时
/// （工具期限 / 服务关闭）随交付守卫停止播报并收起弹幕。
pub async fn deliver_text(
    app: &AppHandle,
    text: String,
    kinds: Vec<DeliveryKind>,
) -> Result<Vec<DeliveryKind>, String> {
    deliver_text_with(app, text, kinds, CancellationToken::default(), false).await
}

/// [`deliver_text`]，可以从外面停下（程序里的 `say()`，ADR 045 D3）：`cancel` 取消后排队、等麦克风、说话、等弹幕
/// 都马上结束，语音停下、弹幕收起（投递自己收尾，调用方再等它一会儿）。`legacy_voice` = 用旧语音引擎
/// （0.1 升级上来的提醒）。
pub async fn deliver_text_with(
    app: &AppHandle,
    text: String,
    kinds: Vec<DeliveryKind>,
    cancel: CancellationToken,
    legacy_voice: bool,
) -> Result<Vec<DeliveryKind>, String> {
    let delivery = if kinds.is_empty() {
        app.state::<crate::SharedState>()
            .settings
            .read()
            .unwrap()
            .assistant
            .default_delivery
            .clone()
    } else {
        kinds
    };
    let now = now_ms();
    let reminder = Reminder {
        id: format!("agent-reply-{now}"),
        session_id: "agent-reply".into(),
        text,
        delivery: delivery.clone(),
        created_at_ms: now,
        due_at_ms: now,
        status: kotone_core::reminders::ReminderStatus::Delivering,
        error: None,
    };
    DesktopDelivery {
        app: app.clone(),
        legacy_voice,
    }
    .deliver(&reminder, cancel)
    .await?;
    Ok(delivery)
}

#[tauri::command]
pub fn cancel_assistant_request(state: tauri::State<AssistantRuntime>) {
    if let Some(cancel) = state.typed_request.lock().unwrap().as_ref() {
        cancel.cancel();
    }
}

#[tauri::command]
pub async fn list_speech_voices(
    app: AppHandle,
    legacy_reminder: Option<bool>,
) -> Result<Vec<SpeechVoice>, String> {
    let config = app
        .state::<crate::SharedState>()
        .settings
        .read()
        .unwrap()
        .assistant
        .clone();
    let engine = if legacy_reminder.unwrap_or(false) {
        config.legacy_tts_engine()
    } else {
        effective_engine(&app, &config)
    };
    if engine == TtsEngine::Kokoro {
        return Ok(kotone_tts::kokoro_voices());
    }
    if let Some(kind) = kotone_tts::catalog::kind_of(engine) {
        // MiniMax 有 Key 时在线拉（含复刻的声音），其余是内置列表
        return Ok(crate::speech_cmd::voices_for(&app, kind).await);
    }
    let speaker = speaker_for_engine(&app, engine)?;
    let voices = tokio::task::spawn_blocking(move || speaker.voices())
        .await
        .map_err(|_| "读取系统语音失败".to_string())??;
    // 声音编号统一写成 `system/<语音 token>`；旧配置里不带前缀的值照常可用。
    Ok(voices
        .into_iter()
        .map(|voice| SpeechVoice {
            id: kotone_tts::voice::qualify(kotone_tts::voice::SYSTEM, &voice.id),
            name: voice.name,
        })
        .collect())
}

#[tauri::command]
pub async fn preview_reminder(
    app: AppHandle,
    kind: String,
    legacy_reminder: Option<bool>,
) -> Result<(), String> {
    let runtime = app.state::<AssistantRuntime>();
    let _output = runtime
        .output_lock
        .try_lock()
        .map_err(|_| "正在执行提醒，请稍后测试".to_string())?;
    let output_cancel = runtime.output_cancel.begin();
    until_output_stopped(&output_cancel, async {
        let mut guard = OutputGuard {
            app: app.clone(),
            speaker: None,
            visual: None,
        };
        let config = app
            .state::<crate::SharedState>()
            .settings
            .read()
            .unwrap()
            .assistant
            .clone();
        if kind == "danmaku" {
            let id = present_danmaku(&app, "打野有闪了。", &config, &output_cancel)?;
            guard.visual = Some(id);
            wait_danmaku(&runtime, id).await;
            return Ok(());
        }
        if kind != "voice" {
            return Err("未知回答方式".into());
        }
        let engine = if legacy_reminder.unwrap_or(false) {
            config.legacy_tts_engine()
        } else {
            effective_engine(&app, &config)
        };
        let started = std::time::Instant::now();
        // 试听没声音时要能查：记下用的引擎、花了多久、结果。
        let log_result = |result: &Result<(), String>| {
            kotone_core::log::log(&format!(
                "speech preview: engine={engine:?} took={}ms result={}",
                started.elapsed().as_millis(),
                match result {
                    Ok(()) => "played".to_string(),
                    Err(e) => format!("error: {e}"),
                }
            ));
        };
        let speaker =
            speaker_for_engine(&app, engine).inspect_err(|e| log_result(&Err(e.clone())))?;
        guard.speaker = Some(speaker.clone());
        let result = queue_speech(
            speaker,
            speech_request(engine, &config, "打野有闪了。该打小龙了。"),
            &runtime.active_captures,
            &output_cancel,
        )
        .inspect_err(|e| log_result(&Err(e.clone())))?
        .await
        .map_err(|_| "语音试听失败".to_string())
        .and_then(|r| r);
        log_result(&result);
        guard.speaker = None;
        result
    })
    .await
}

/// 试听一个声音（ADR 048，声音设置页的“试听”）：用这个声音念 `text`，不管现在选的是哪一家。音量、语速照设置。
/// 和“试听语音”一样排队拿播报，正在播别的就报错；在线声音的结果按声音落盘（`cache::disk::also`），再听不花钱。
pub async fn try_voice(
    app: &AppHandle,
    engine: TtsEngine,
    voice_id: &str,
    text: &str,
) -> Result<(), String> {
    let runtime = app.state::<AssistantRuntime>();
    let _output = runtime
        .output_lock
        .try_lock()
        .map_err(|_| "正在播报，请稍后再试听".to_string())?;
    let output_cancel = runtime.output_cancel.begin();
    until_output_stopped(&output_cancel, async {
        let mut guard = OutputGuard {
            app: app.clone(),
            speaker: None,
            visual: None,
        };
        let config = app
            .state::<crate::SharedState>()
            .settings
            .read()
            .unwrap()
            .assistant
            .clone();
        let speaker = speaker_for_engine(app, engine)?;
        guard.speaker = Some(speaker.clone());
        let voice = match engine {
            TtsEngine::System => voice_id.trim_start_matches("system/").to_string(),
            _ => voice_id.to_string(),
        };
        let request = SpeechRequest {
            text: text.to_string(),
            voice_id: (!voice.is_empty()).then_some(voice),
            volume: config.speech_volume,
            rate: config.speech_rate,
        };
        let result = queue_speech(speaker, request, &runtime.active_captures, &output_cancel)?
            .await
            .map_err(|_| "试听失败".to_string())
            .and_then(|r| r);
        guard.speaker = None;
        result
    })
    .await
}

/// 呼叫琴音的按钮（聊天页）：和呼叫键一样，按一次就在气泡里听。
/// ADR 028：语音识别没就绪（本地识别未启动）时先启动，准备好再开始听；不让玩家先去点「启动」。
#[tauri::command]
pub async fn assistant_call(app: AppHandle) -> Result<(), String> {
    let state = app.state::<crate::SharedState>();
    let timeout = {
        let settings = state.settings.read().unwrap();
        if settings.ui.legacy_upgrade.uses_legacy_assistant() {
            return Err("先添加 DeepSeek Key，才能呼叫琴音".into());
        }
        settings.assistant.wake.no_speech_timeout_ms
    };
    // 通话中再点一下 = 挂断（同呼叫键，ADR 033 D1）
    if crate::call::active() {
        crate::call::hang_up_by_key(&app).await;
        return Ok(());
    }
    ensure_voice_ready(&app).await?;
    interrupt_before_call(&app).await;
    state.orchestrator.call_assistant(None, timeout).await;
    // 选了通话就接通（ADR 033）：这句说完以后一直在听，直到挂断。说一句就走、录音笔不接通
    // （按钮没有“再按一下”，录音笔也照说一句就走，ADR 033 2026-11-26 修订）
    crate::call::start(&app);
    Ok(())
}

/// 等语音识别就绪：停着就启动（本地识别先加载模型），正在启动 / 停止就等它稳定下来。
async fn ensure_voice_ready(app: &AppHandle) -> Result<(), String> {
    use kotone_core::runtime::RuntimePhase;
    for _ in 0..150 {
        let phase = app.state::<crate::runtime::RuntimeManager>().phase();
        match phase {
            RuntimePhase::Running => return Ok(()),
            RuntimePhase::Stopped => {
                crate::runtime::start(app).await?;
                return Ok(());
            }
            RuntimePhase::Starting | RuntimePhase::Stopping => {
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            }
        }
    }
    Err("语音识别还在准备，稍后再试".into())
}

#[tauri::command]
pub fn stop_speech(app: AppHandle) {
    let runtime = app.state::<AssistantRuntime>();
    // Keep the slot locked through native cleanup: a new output may start only
    // after this stop has finished, otherwise its freshly queued voice could
    // be purged by the previous output's stop request.
    let _output_cancel = runtime.output_cancel.stop();
    crate::voice_reply::interrupt();
    silence(&app);
    let id = runtime.presentation.lock().unwrap().as_ref().map(|p| p.id);
    if let Some(id) = id {
        hide_danmaku(&app, id);
    }
}

#[tauri::command]
pub fn get_danmaku(state: tauri::State<AssistantRuntime>) -> Option<DanmakuPresentation> {
    state
        .presentation
        .lock()
        .unwrap()
        .clone()
        .filter(|p| p.expires_at_ms > now_ms())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    struct Cleanup(Arc<AtomicBool>);

    #[derive(Default)]
    struct TestSpeaker {
        generation: AtomicU64,
        spoken: AtomicUsize,
    }

    impl SpeechOutput for TestSpeaker {
        fn voices(&self) -> Result<Vec<SpeechVoice>, String> {
            Ok(Vec::new())
        }

        fn generation(&self) -> u64 {
            self.generation.load(Ordering::SeqCst)
        }

        fn speak_if_current(&self, _: SpeechRequest, ticket: u64) -> Result<(), String> {
            if ticket != self.generation() {
                return Err("播报已取消".into());
            }
            self.spoken.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        fn cancel(&self) -> Result<(), String> {
            self.generation.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    impl Drop for Cleanup {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    #[tokio::test]
    async fn stop_interrupts_output_waiting_for_capture_and_runs_cleanup() {
        let cancel = CancellationToken::default();
        let cleaned = Arc::new(AtomicBool::new(false));
        let (ready, started) = tokio::sync::oneshot::channel();
        let output_cancel = cancel.clone();
        let cleanup = cleaned.clone();
        let task = tokio::spawn(async move {
            until_output_stopped(&output_cancel, async {
                let _cleanup = Cleanup(cleanup);
                let _ = ready.send(());
                std::future::pending::<Result<(), String>>().await
            })
            .await
        });
        started.await.unwrap();
        cancel.cancel();
        let result = tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result.unwrap_err(), "提醒输出已停止");
        assert!(cleaned.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn stop_wins_over_ready_speech_failure_without_showing_fallback() {
        let cancel = CancellationToken::default();
        let output_cancel = cancel.clone();
        let fallback = Arc::new(AtomicBool::new(false));
        let visible = fallback.clone();
        let (ready, started) = tokio::sync::oneshot::channel();
        let (failure, failed) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            until_output_stopped(&output_cancel, async {
                let _ = ready.send(());
                failed.await.unwrap();
                visible.store(true, Ordering::SeqCst);
                Err::<(), _>("播报已取消".to_string())
            })
            .await
        });
        started.await.unwrap();
        cancel.cancel();
        failure.send(()).unwrap();
        assert_eq!(task.await.unwrap().unwrap_err(), "提醒输出已停止");
        assert!(!fallback.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn ordinary_output_failure_is_preserved() {
        let cancel = CancellationToken::default();
        let result = until_output_stopped(&cancel, async {
            Err::<(), _>("语音设备不可用".to_string())
        })
        .await;
        assert_eq!(result.unwrap_err(), "语音设备不可用");
    }

    #[tokio::test]
    async fn stopping_current_output_allows_the_next_reminder() {
        let outputs = OutputCancellation::default();
        let current = outputs.begin();
        drop(outputs.stop());
        assert!(current.is_cancelled());
        let next = outputs.begin();
        let result = until_output_stopped(&next, async { Ok(()) }).await;
        assert!(result.is_ok());
        assert!(current.is_cancelled());
    }

    #[test]
    fn stopped_output_cannot_commit_a_late_visual_fallback() {
        let outputs = OutputCancellation::default();
        let current = outputs.begin();
        drop(outputs.stop());
        assert!(current
            .run_if_active(|| panic!("a stopped reminder must not show a fallback"))
            .is_none());
        let next = outputs.begin();
        assert_eq!(
            next.run_if_active(|| "next reminder"),
            Some("next reminder")
        );
    }

    #[tokio::test]
    async fn stopped_output_cannot_queue_voice_after_the_engine_was_purged() {
        let outputs = OutputCancellation::default();
        let current = outputs.begin();
        let speaker = Arc::new(TestSpeaker::default());
        {
            let _stop = outputs.stop();
            speaker.cancel().unwrap();
        }
        let captures = AtomicUsize::new(0);
        let request = || {
            speech_request(
                TtsEngine::System,
                &AssistantConfig::default(),
                "打野有闪了。",
            )
        };
        assert!(queue_speech(speaker.clone(), request(), &captures, &current).is_err());
        assert_eq!(speaker.spoken.load(Ordering::SeqCst), 0);
        let next = outputs.begin();
        queue_speech(speaker.clone(), request(), &captures, &next)
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(speaker.spoken.load(Ordering::SeqCst), 1);
    }

    // ---------- 插件经宿主出声（`Host::speak`，游戏语音“说给队友”时玩家这边听的那一遍） ----------

    /// 假扬声器：开始说一句就记一笔，然后一直“放着”，等测试放行；被停下（`cancel`）就返回“已取消”。
    /// `full` 是前几次说的时候回“队列已满”（本地语音的工作线程队列只有 2 格）。
    #[derive(Default)]
    struct PlayingSpeaker {
        generation: AtomicU64,
        full: AtomicUsize,
        started: AtomicUsize,
        finished: AtomicUsize,
        released: Mutex<bool>,
        wake: std::sync::Condvar,
    }

    impl PlayingSpeaker {
        fn queue_full_for(times: usize) -> Arc<Self> {
            let speaker = Self::default();
            speaker.full.store(times, Ordering::SeqCst);
            Arc::new(speaker)
        }

        fn release(&self) {
            *self.released.lock().unwrap() = true;
            self.wake.notify_all();
        }

        fn started(&self) -> usize {
            self.started.load(Ordering::SeqCst)
        }
    }

    impl SpeechOutput for PlayingSpeaker {
        fn voices(&self) -> Result<Vec<SpeechVoice>, String> {
            Ok(Vec::new())
        }

        fn generation(&self) -> u64 {
            self.generation.load(Ordering::SeqCst)
        }

        fn speak_if_current(&self, _: SpeechRequest, ticket: u64) -> Result<(), String> {
            if ticket != self.generation() {
                return Err("播报已取消".into());
            }
            // 大于 0 才减 1；不用 fetch_update（新工具链弃用）/ try_update（旧工具链没有）
            let mut left = self.full.load(Ordering::SeqCst);
            while left > 0 {
                match self
                    .full
                    .compare_exchange(left, left - 1, Ordering::SeqCst, Ordering::SeqCst)
                {
                    Ok(_) => return Err("本地语音队列已满或服务已停止".into()),
                    Err(now) => left = now,
                }
            }
            self.started.fetch_add(1, Ordering::SeqCst);
            let playing = std::time::Instant::now();
            let mut released = self.released.lock().unwrap();
            while !*released {
                if ticket != self.generation() {
                    return Err("播报已取消".into());
                }
                // 测试断言失败、没人放行时也要结束（tokio 退出时会等这条线程）
                if playing.elapsed() > Duration::from_secs(5) {
                    return Err("测试里一直没放行".into());
                }
                released = self
                    .wake
                    .wait_timeout(released, Duration::from_millis(10))
                    .unwrap()
                    .0;
            }
            self.finished.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        fn cancel(&self) -> Result<(), String> {
            self.generation.fetch_add(1, Ordering::SeqCst);
            self.wake.notify_all();
            Ok(())
        }
    }

    /// 宿主那一份播报状态：同一把播报锁、同一个“停下”、同一个“麦克风开着”。
    #[derive(Clone, Default)]
    struct Turns {
        lock: Arc<tokio::sync::Mutex<()>>,
        outputs: Arc<OutputCancellation>,
        captures: Arc<AtomicUsize>,
    }

    impl Turns {
        /// 插件说一句（和 `DesktopHost::speak` 走同一个函数）。
        fn plugin_says(
            &self,
            speaker: &Arc<PlayingSpeaker>,
        ) -> tokio::task::JoinHandle<Result<(), Unspoken>> {
            let turns = self.clone();
            let speaker: Arc<dyn SpeechOutput> = speaker.clone();
            tokio::spawn(async move {
                let request =
                    speech_request(TtsEngine::Kokoro, &AssistantConfig::default(), "我去拿龙了");
                take_turn_and_speak(
                    &turns.lock,
                    &turns.outputs,
                    &turns.captures,
                    speaker,
                    request,
                    None,
                )
                .await
            })
        }
    }

    async fn eventually(what: &str, done: impl Fn() -> bool) {
        let started = std::time::Instant::now();
        while !done() {
            assert!(started.elapsed() < Duration::from_secs(3), "等不到：{what}");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    #[tokio::test]
    async fn plugin_speech_waits_until_she_has_finished() {
        let turns = Turns::default();
        let speaker = Arc::new(PlayingSpeaker::default());
        // 她正在说（边生成边念的回答、提醒、填充话都拿着这把播报锁）
        let her_words = turns.lock.clone().lock_owned().await;
        let plugin = turns.plugin_says(&speaker);
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(
            speaker.started(),
            0,
            "她还在说：插件的话要排在她后面，不能两段同时放"
        );
        drop(her_words);
        eventually("轮到插件的话", || speaker.started() == 1).await;
        speaker.release();
        plugin.await.unwrap().unwrap();
    }

    /// 推送的语音（ADR 043 D3，`speak_push`）：她一直在说，到期限还没轮到就不放了（过时了），不叠在她的话上，也不占着锁；
    /// 期限前轮到了照常放。
    #[tokio::test]
    async fn a_push_voice_that_waits_past_its_deadline_is_dropped() {
        let turns = Turns::default();
        let speaker = Arc::new(PlayingSpeaker::default());
        let her_words = turns.lock.clone().lock_owned().await;
        let request = speech_request(TtsEngine::Kokoro, &AssistantConfig::default(), "中间那个");
        let deadline = std::time::Instant::now() + Duration::from_millis(150);
        let late = take_turn_and_speak(
            &turns.lock,
            &turns.outputs,
            &turns.captures,
            speaker.clone(),
            request.clone(),
            Some(deadline),
        )
        .await;
        assert_eq!(late, Err(Unspoken::TooLate));
        assert_eq!(speaker.started(), 0, "过时了：一个字都不放");
        drop(her_words);
        // 她说完了、期限还没到：照常放
        let on_time = {
            let turns = turns.clone();
            let speaker: Arc<dyn SpeechOutput> = speaker.clone();
            tokio::spawn(async move {
                let deadline = std::time::Instant::now() + Duration::from_secs(5);
                take_turn_and_speak(
                    &turns.lock,
                    &turns.outputs,
                    &turns.captures,
                    speaker,
                    request,
                    Some(deadline),
                )
                .await
            })
        };
        eventually("轮到推送的语音", || speaker.started() == 1).await;
        speaker.release();
        on_time.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn her_next_words_wait_for_plugin_speech() {
        let turns = Turns::default();
        let speaker = Arc::new(PlayingSpeaker::default());
        let plugin = turns.plugin_says(&speaker);
        eventually("插件的话开始放", || speaker.started() == 1).await;
        // 她要开口了（`hold_output`：边生成边念的回答第一个字到了就排队拿播报）
        let her_turn = tokio::time::timeout(Duration::from_millis(200), turns.lock.lock()).await;
        assert!(
            her_turn.is_err(),
            "插件的话还在放：她下一句要等它放完，不能叠在一起"
        );
        speaker.release();
        plugin.await.unwrap().unwrap();
        assert!(turns.lock.try_lock().is_ok(), "放完了就让出来");
    }

    #[tokio::test]
    async fn plugin_speech_counts_as_her_voice() {
        let turns = Turns::default();
        let speaker = Arc::new(PlayingSpeaker::default());
        assert!(!audible(turns.lock.try_lock().is_err()));
        let plugin = turns.plugin_says(&speaker);
        eventually("插件的话开始放", || speaker.started() == 1).await;
        assert!(
            audible(turns.lock.try_lock().is_err()),
            "插件用她的声音在放：算她在出声（音乐压低；通话里没有回声消除时不把这句当成玩家说的）"
        );
        speaker.release();
        plugin.await.unwrap().unwrap();
        assert!(!audible(turns.lock.try_lock().is_err()));
    }

    #[tokio::test]
    async fn plugin_speech_is_not_dropped_when_the_speech_queue_is_full() {
        let turns = Turns::default();
        let speaker = PlayingSpeaker::queue_full_for(2);
        speaker.release();
        let result = turns.plugin_says(&speaker).await.unwrap();
        assert_eq!(result, Ok(()), "队列满了稍等再放，不丢");
        assert_eq!(speaker.finished.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn plugin_speech_waits_for_the_microphone() {
        let turns = Turns::default();
        let speaker = Arc::new(PlayingSpeaker::default());
        speaker.release();
        turns.captures.store(1, Ordering::SeqCst);
        let plugin = turns.plugin_says(&speaker);
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(
            speaker.started(),
            0,
            "麦克风开着：先不出声，免得把她的声音录成玩家说的"
        );
        turns.captures.store(0, Ordering::SeqCst);
        plugin.await.unwrap().unwrap();
        assert_eq!(speaker.started(), 1);
    }

    #[tokio::test]
    async fn stopping_her_also_stops_plugin_speech() {
        // 还在等麦克风关：停下就不放了
        let turns = Turns::default();
        let speaker = Arc::new(PlayingSpeaker::default());
        turns.captures.store(1, Ordering::SeqCst);
        let waiting = turns.plugin_says(&speaker);
        tokio::time::sleep(Duration::from_millis(100)).await;
        drop(turns.outputs.stop());
        assert_eq!(waiting.await.unwrap(), Err(Unspoken::Stopped));
        assert_eq!(speaker.started(), 0);
        // 正在放：停下就停（不算“没放出来”，不改用弹幕）
        turns.captures.store(0, Ordering::SeqCst);
        let playing = turns.plugin_says(&speaker);
        eventually("插件的话开始放", || speaker.started() == 1).await;
        drop(turns.outputs.stop());
        assert_eq!(playing.await.unwrap(), Err(Unspoken::Stopped));
        assert_eq!(speaker.finished.load(Ordering::SeqCst), 0);
        assert!(turns.lock.try_lock().is_ok());
    }

    #[tokio::test]
    async fn plugin_speech_stops_when_the_caller_gives_up() {
        // 插件不等了（游戏语音停下时丢掉还在排队、正在放的那一遍）
        let turns = Turns::default();
        let speaker = Arc::new(PlayingSpeaker::default());
        let playing = turns.plugin_says(&speaker);
        eventually("插件的话开始放", || speaker.started() == 1).await;
        playing.abort();
        eventually("语音被停下", || speaker.generation() > 0).await;
        assert_eq!(speaker.finished.load(Ordering::SeqCst), 0);
        eventually("播报锁让出来", || turns.lock.try_lock().is_ok()).await;
    }

    #[tokio::test]
    async fn a_failure_other_than_a_stop_is_reported() {
        let turns = Turns::default();
        let speaker = PlayingSpeaker::queue_full_for(usize::MAX);
        let result = turns.plugin_says(&speaker).await.unwrap();
        assert_eq!(
            result,
            Err(Unspoken::Failed("本地语音队列已满或服务已停止".into())),
            "一直满（服务停了）：等一会儿还是不行就说出来，宿主改用弹幕、告诉插件"
        );
    }

    #[tokio::test]
    async fn capture_prevents_voice_admission_without_stopping_visual_output() {
        let output = CancellationToken::default();
        let speaker = Arc::new(TestSpeaker::default());
        let result = queue_speech(
            speaker.clone(),
            speech_request(
                TtsEngine::System,
                &AssistantConfig::default(),
                "打野有闪了。",
            ),
            &AtomicUsize::new(1),
            &output,
        );
        assert!(result.unwrap_err().contains("录音已开始"));
        assert_eq!(speaker.spoken.load(Ordering::SeqCst), 0);
        assert!(output.run_if_active(|| "visual fallback").is_some());
    }
}
