//! Desktop composition for reminders. Scheduling and AI planning live outside
//! the shell; this adapter owns speech and the non-activating presentation window.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use kotone_core::assistant::{
    AssistantConfig, AssistantHandler, AssistantPlan, AssistantRequest, SpeechEngine,
};
use kotone_core::cancel::CancellationToken;
use kotone_core::reminders::{
    DeliveryFuture, DeliveryKind, Reminder, ReminderDelivery, ReminderScheduler,
};
use kotone_core::settings::Settings;
use kotone_core::speech::{SpeechOutput, SpeechRequest, SpeechVoice};
use tauri::{AppHandle, Emitter, Manager};

pub struct AssistantRuntime {
    pub scheduler: Result<Arc<ReminderScheduler>, String>,
    pub handler: Arc<dyn AssistantHandler>,
    system_speaker: Result<Arc<dyn SpeechOutput>, String>,
    local_speaker: Mutex<Option<(std::path::PathBuf, Arc<kotone_tts::LocalSpeechOutput>)>>,
    typed_request: Mutex<Option<CancellationToken>>,
    presentation: Mutex<Option<DanmakuPresentation>>,
    presentation_generation: AtomicU64,
    presentation_done: tokio::sync::Notify,
    output_lock: tokio::sync::Mutex<()>,
    output_cancel: OutputCancellation,
    // A cancelled session may still be joining its capture thread while a new
    // session opens. Each microphone lease retains its own admission slot.
    active_captures: AtomicUsize,
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
        system_result.and(local_result)
    }
}

fn selected_speaker(
    app: &AppHandle,
    config: &AssistantConfig,
) -> Result<Arc<dyn SpeechOutput>, String> {
    let runtime = app.state::<AssistantRuntime>();
    if config.speech_engine == SpeechEngine::System {
        return runtime.system_speaker.clone();
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

pub fn initialize(
    app: &AppHandle,
    settings: Arc<RwLock<Settings>>,
    connections: Arc<dyn kotone_core::connection::ConnectionResolver>,
) -> AssistantRuntime {
    let speaker = kotone_platform_windows::speech::WindowsSpeechOutput::new()
        .map(|speaker| Arc::new(speaker) as Arc<dyn SpeechOutput>);
    let scheduler = ReminderScheduler::open(
        kotone_core::settings::kotone_dir().join("reminders.json"),
        Arc::new(DesktopDelivery { app: app.clone() }),
    );
    if let Ok(scheduler) = &scheduler {
        let handle = app.clone();
        scheduler.set_on_changed(Arc::new(move || {
            let _ = handle.emit("kotone://reminders-updated", ());
        }));
    }
    AssistantRuntime {
        scheduler,
        handler: Arc::new(kotone_ai::reminders::ReminderAssistant::new(
            connections,
            settings,
        )),
        system_speaker: speaker,
        local_speaker: Mutex::new(None),
        typed_request: Mutex::new(None),
        presentation: Mutex::new(None),
        presentation_generation: AtomicU64::new(0),
        presentation_done: tokio::sync::Notify::new(),
        output_lock: tokio::sync::Mutex::new(()),
        output_cancel: OutputCancellation::default(),
        active_captures: AtomicUsize::new(0),
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
    if let Some(window) = app.get_webview_window("danmaku") {
        let _ = window.set_ignore_cursor_events(true);
    }
}

pub fn shutdown(app: &AppHandle) {
    if let Some(state) = app.try_state::<AssistantRuntime>() {
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

pub fn before_capture(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<AssistantRuntime>();
    // Set before purging: output admission sees even the interval before Listening.
    state.active_captures.fetch_add(1, Ordering::SeqCst);
    state.cancel_speech()
}

pub fn after_capture(app: &AppHandle) {
    if let Some(state) = app.try_state::<AssistantRuntime>() {
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

impl ReminderDelivery for DesktopDelivery {
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
                    let speaker = selected_speaker(&self.app, &config);
                    match speaker {
                        Ok(speaker) => {
                            guard.speaker = Some(speaker.clone());
                            match queue_speech(
                                speaker.clone(),
                                speech_request(&config, &reminder.text),
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

fn speech_request(config: &kotone_core::assistant::AssistantConfig, text: &str) -> SpeechRequest {
    let voice = match config.speech_engine {
        SpeechEngine::Kokoro => &config.kokoro_voice_id,
        SpeechEngine::System => &config.voice_id,
    };
    SpeechRequest {
        text: text.to_string(),
        voice_id: (!voice.is_empty()).then(|| voice.clone()),
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
                speaker.speak_if_current(request, generation)
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
pub async fn assistant_submit_text(app: AppHandle, text: String) -> Result<AssistantPlan, String> {
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
    let request = AssistantRequest {
        session_id: format!("assistant-ui-{}", kotone_core::eval::new_session_id()),
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

#[tauri::command]
pub fn cancel_assistant_request(state: tauri::State<AssistantRuntime>) {
    if let Some(cancel) = state.typed_request.lock().unwrap().as_ref() {
        cancel.cancel();
    }
}

#[tauri::command]
pub async fn list_speech_voices(app: AppHandle) -> Result<Vec<SpeechVoice>, String> {
    let config = app
        .state::<crate::SharedState>()
        .settings
        .read()
        .unwrap()
        .assistant
        .clone();
    if config.speech_engine == SpeechEngine::Kokoro {
        return Ok(kotone_tts::kokoro_voices());
    }
    let speaker = selected_speaker(&app, &config)?;
    tokio::task::spawn_blocking(move || speaker.voices())
        .await
        .map_err(|_| "读取系统语音失败".to_string())?
}

#[tauri::command]
pub async fn preview_reminder(app: AppHandle, kind: String) -> Result<(), String> {
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
            return Err("未知提醒方式".into());
        }
        let speaker = selected_speaker(&app, &config)?;
        guard.speaker = Some(speaker.clone());
        let result = queue_speech(
            speaker,
            speech_request(&config, "打野有闪了。该打小龙了。"),
            &runtime.active_captures,
            &output_cancel,
        )?
        .await
        .map_err(|_| "语音试听失败".to_string())?;
        guard.speaker = None;
        result
    })
    .await
}

#[tauri::command]
pub fn stop_speech(app: AppHandle) {
    let runtime = app.state::<AssistantRuntime>();
    // Keep the slot locked through native cleanup: a new output may start only
    // after this stop has finished, otherwise its freshly queued voice could
    // be purged by the previous output's stop request.
    let _output_cancel = runtime.output_cancel.stop();
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
        let request = || speech_request(&AssistantConfig::default(), "打野有闪了。");
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

    #[tokio::test]
    async fn capture_prevents_voice_admission_without_stopping_visual_output() {
        let output = CancellationToken::default();
        let speaker = Arc::new(TestSpeaker::default());
        let result = queue_speech(
            speaker.clone(),
            speech_request(&AssistantConfig::default(), "打野有闪了。"),
            &AtomicUsize::new(1),
            &output,
        );
        assert!(result.unwrap_err().contains("录音已开始"));
        assert_eq!(speaker.spoken.load(Ordering::SeqCst), 0);
        assert!(output.run_if_active(|| "visual fallback").is_some());
    }
}
