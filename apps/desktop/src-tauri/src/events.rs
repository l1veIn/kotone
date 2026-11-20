//! 核心事件转发到界面（TauriEmitter）：状态、悬浮窗显隐、提示音、过程记录。

use super::*;

/// 生产事件出口：转发为 Tauri 事件；联动 Esc 取消键注册与 overlay 窗口显隐。
/// overlay 显隐规则（后端驱动，幂等，与前端逻辑不冲突）按 `overlay.visibility` 分档：
/// - always（常驻）：会话态（Listening/…/Error）→ show；Running 期间 idle 不隐藏
///   （悬浮窗兼作运行指示）；Stopped 由 stop_runtime 显式隐藏。
/// - on_demand（用时浮现）：平时隐藏；Listening/Transcribing/Preview/Sending → show；
///   Success 延迟 ~600ms 自动隐藏（vis_gen 代际防新会话误藏）；
///   Error 无条件显示并保持到用户关闭/重试，避免核心链路失败只进日志；
///   solo 连续模式保持显示直到会话停止（idle → hide）。
/// - never（不显示）：所有状态都强制隐藏；热键与语音输入流程不受影响。
pub(crate) struct TauriEmitter {
    pub(crate) app: AppHandle,
    /// overlay 显隐代际：每个状态事件 +1；延迟隐藏任务只认调度时的代际
    pub(crate) vis_gen: Arc<std::sync::atomic::AtomicU64>,
    /// 为琴音的会话藏起了常驻悬浮窗：会话结束（idle）时放回来
    pub(crate) hidden_for_companion: Arc<std::sync::atomic::AtomicBool>,
}

/// 琴音的录音会话开始 / 判停（ADR 029）：开口时截屏、语音识别费用先扣住等交给对话、计时。
fn on_companion_state(app: &AppHandle, payload: &serde_json::Value) {
    match payload.get("state").and_then(|s| s.as_str()).unwrap_or("") {
        "listening" => {
            kotone_core::usage::hold_asr();
            speech_screen::on_listening(app);
        }
        "transcribing" => voice_timing::speech_ended(),
        _ => {}
    }
}

/// 琴音（角色）的录音会话：气泡只属于角色、悬浮窗只属于语音输入（ADR 028）。
/// 0.1.x 的提醒助手仍是语音输入法的一部分，照旧用悬浮窗。
fn is_companion_session(payload: &serde_json::Value) -> bool {
    payload.pointer("/payload/purpose").and_then(|v| v.as_str()) == Some("assistant")
        && payload
            .pointer("/payload/legacyReminder")
            .and_then(|v| v.as_bool())
            != Some(true)
}

impl Emitter for TauriEmitter {
    fn emit(&self, event: &str, payload: serde_json::Value) {
        use tauri::Emitter as _;
        if event == "kotone://state" {
            overlay::remember_state(&self.app, &payload);
        }
        let _ = self.app.emit(event, payload.clone());
        // 琴音的语音会话只在对话气泡里显示；气泡关着时也不借用语音输入的悬浮窗（D10）。
        let companion_session = event == "kotone://state" && is_companion_session(&payload);
        if companion_session {
            on_companion_state(&self.app, &payload);
        }
        match event {
            "kotone://state" => bubble::on_state(&self.app, &payload),
            "kotone://partial" => {
                if let Some(text) = payload.get("text").and_then(|t| t.as_str()) {
                    bubble::on_partial(&self.app, text);
                }
            }
            "kotone://vad-no-speech" => bubble::on_no_speech(&self.app),
            _ => {}
        }
        if event == "kotone://state" {
            let state = payload.get("state").and_then(|s| s.as_str()).unwrap_or("");
            let text_chars = payload
                .pointer("/payload/text")
                .and_then(|v| v.as_str())
                .map(|text| text.chars().count())
                .unwrap_or(0);
            let has_error = payload.pointer("/payload/message").is_some();
            log::log(&format!(
                "state -> {state} text_chars={text_chars} has_error={has_error}"
            ));
            // 会话激活期间（含 Preview）临时注册 Esc 全局取消键，回 Idle 即注销。
            // Preview 态同样需要 Esc：overlay 不抢焦点，Esc 是预览确认的主要键盘出口。
            if let Some(mgr) = self.app.try_state::<HotkeyManager>() {
                mgr.set_cancel_enabled(&self.app, state != "idle" && !state.is_empty());
            }
            // 后端驱动 overlay 显隐（显隐一律走原始 Win32 SW_SHOWNA/SW_HIDE 路径，
            // 不碰 tao set_visible——tao 缓存 diff 短路坑，见 hide_window 注释）
            if companion_session {
                if let Some(win) = self.app.get_webview_window("overlay") {
                    hide_window(&win);
                    self.hidden_for_companion
                        .store(true, std::sync::atomic::Ordering::SeqCst);
                }
            } else if let Some(win) = self.app.get_webview_window("overlay") {
                let (visibility, continuous) = self
                    .app
                    .try_state::<SharedState>()
                    .map(|s| {
                        let visibility = s.settings.read().unwrap().overlay.visibility;
                        (visibility, s.orchestrator.continuous_session())
                    })
                    .unwrap_or((OverlayVisibility::Always, false));
                let gen = self
                    .vis_gen
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                    + 1;
                let running = self
                    .app
                    .try_state::<RuntimeManager>()
                    .map(|rt| rt.phase() == RuntimePhase::Running)
                    .unwrap_or(false);
                let action = overlay_window_action(visibility, state, continuous, running);
                // 独占全屏下激活置顶浮窗可能使 Direct3D 游戏最小化。
                // 会话可继续全程靠热键/声音操作，但对本次 Show 做安全拦截，
                // 并让设置根页记住提示（即使当时窗口隐藏/不在通用页）。
                let block_for_fullscreen = action == OnDemandOverlayAction::Show
                    && fullscreen::is_exclusive_fullscreen_active() == Some(true);
                if block_for_fullscreen {
                    hide_window(&win);
                    record_overlay_event("fullscreen_state", "state", Some(true));
                    let _ = self.app.emit(
                        "kotone://fullscreen-warning",
                        serde_json::json!({ "exclusiveFullscreen": true }),
                    );
                    log::log("overlay show blocked: exclusive fullscreen game is active");
                } else {
                    match action {
                        OnDemandOverlayAction::Show => {
                            // Error 可能从 Idle 直接到达（例如音频设备打开失败），不能
                            // 假设 Listening 已经显示过窗口；它会保持到用户明确确认。
                            show_window_no_focus(&win);
                            record_overlay_event("overlay_show", "state", Some(false));
                        }
                        OnDemandOverlayAction::HideSuccessAfterDwell => {
                            let app = self.app.clone();
                            let vis_gen = self.vis_gen.clone();
                            tauri::async_runtime::spawn(async move {
                                tokio::time::sleep(std::time::Duration::from_millis(600)).await;
                                if vis_gen.load(std::sync::atomic::Ordering::SeqCst) != gen {
                                    return; // 600ms 内已有新状态事件（新会话/停止），不藏
                                }
                                let handle = app.clone();
                                let _ = app.run_on_main_thread(move || {
                                    if vis_gen.load(std::sync::atomic::Ordering::SeqCst) != gen
                                        || current_overlay_action(&handle).map(|(_, action)| action)
                                            != Some(OnDemandOverlayAction::HideSuccessAfterDwell)
                                    {
                                        return;
                                    }
                                    if let Some(win) = handle.get_webview_window("overlay") {
                                        hide_window(&win);
                                        record_overlay_event(
                                            "overlay_hide",
                                            "success-dwell",
                                            fullscreen::is_exclusive_fullscreen_active(),
                                        );
                                    }
                                });
                            });
                        }
                        OnDemandOverlayAction::Hide => {
                            hide_window(&win);
                            record_overlay_event("overlay_hide", "state", Some(false));
                        }
                        // always + Running + idle，以及 continuous（solo）的 success：保持现状；
                        // 但刚为琴音的会话藏起了常驻悬浮窗，就放回来。
                        OnDemandOverlayAction::Keep => {
                            if state == "idle"
                                && self
                                    .hidden_for_companion
                                    .swap(false, std::sync::atomic::Ordering::SeqCst)
                                && visibility == OverlayVisibility::Always
                                && fullscreen::is_exclusive_fullscreen_active() != Some(true)
                            {
                                show_window_no_focus(&win);
                            }
                        }
                    }
                }
            }
        } else if event == "kotone://channel" {
            peek_on_demand_overlay(&self.app, &self.vis_gen, "channel", 1200);
        } else if event == "kotone://pp-status" {
            peek_on_demand_overlay(&self.app, &self.vis_gen, "pp-status", 2000);
        } else if event == "kotone://process" {
            record_process_event(&self.app, &payload);
            maybe_play_feedback_sfx(&self.app, &payload);
        }
    }
}

/// 热键音效反馈：开始监听 → 录制音（soundFeedback.record）；消息注入成功 → 发送音
/// （soundFeedback.send）。两者独立开关/音量/音效选择；solo 逐句续录不重复播录制音。
/// 播放非阻塞、失败只记 stderr，不影响语音链路。
fn maybe_play_feedback_sfx(app: &AppHandle, payload: &serde_json::Value) {
    let Some(activity) = payload.get("activity").and_then(|v| v.as_str()) else {
        return;
    };
    let Some(state) = app.try_state::<SharedState>() else {
        return;
    };
    let sfx = state.settings.read().unwrap().sound_feedback.clone();
    let (item, is_record) = match activity {
        "capture_started" => {
            assistant::silence(app);
            if payload.pointer("/data/resumed").and_then(|v| v.as_bool()) == Some(true) {
                return; // solo 逐句续录：只留发送音
            }
            (sfx.record, true)
        }
        "injection_succeeded" => (sfx.send, false),
        _ => return,
    };
    if !item.enabled || item.volume == 0 {
        return;
    }
    let id = playback::SfxId::from_id(&item.sound_id)
        .unwrap_or_else(|| playback::SfxId::default_for(is_record));
    let gain = (item.volume as f32).clamp(0.0, 100.0) / 100.0;
    playback::play_sfx(id, gain);
}

pub(crate) fn record_process_event(app: &AppHandle, payload: &serde_json::Value) {
    let Some(case_id) = payload.get("caseId").and_then(|v| v.as_str()) else {
        return;
    };
    let Some(activity) = payload.get("activity").and_then(|v| v.as_str()) else {
        return;
    };
    let mut event = process_log::ProcessEvent::new(case_id, activity);
    if let Some(state) = app.try_state::<SharedState>() {
        let settings = state.settings.read().unwrap();
        event.context.engine_id = Some(settings.stt_engine.clone());
        event.context.model_id = Some(model::active_model_from(&settings, &settings.stt_engine));
        event.context.profile_id = settings.active_profile_id.clone();
        event.context.interaction_mode = settings.interaction_mode.as_ref().map(|mode| {
            serde_json::to_string(mode)
                .unwrap_or_default()
                .trim_matches('"')
                .to_string()
        });
    }
    event.context.elevated = Some(elevation::is_elevated());
    if let Some(data) = payload.get("data") {
        event.data.outcome = data
            .get("outcome")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        event.data.error_code = data
            .get("errorCode")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        event.data.duration_ms = data.get("durationMs").and_then(|v| v.as_u64());
        event.data.audio_ms = data.get("audioMs").and_then(|v| v.as_u64());
        event.data.text_chars = data.get("textChars").and_then(|v| v.as_u64());
    }
    if let Err(error) = process_log::record(event) {
        log::log(&format!("process event write failed: {error}"));
    }
}
