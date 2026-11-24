use super::voice_input::send::SEND_STILL_RUNNING;
use super::*;

struct SessionBeginError {
    code: &'static str,
    message: String,
}

impl SessionBeginError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// 会话从哪里开始：决定会话 ID 前缀、语音按钮的操作权限与唤醒专用的收音策略。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SessionOrigin {
    Hotkey,
    /// 设置窗口的语音按钮（只有它能操作自己的会话）。
    WindowVoice,
    /// 语音唤醒（ADR 017）。
    Wake {
        no_speech_timeout_ms: u32,
    },
}

fn kotone_core_assistant_one_shot() -> crate::interaction::InteractionMode {
    crate::assistant::AssistantInteractionMode::OneShot.into()
}

/// 进行中的会话句柄
pub(super) struct ActiveSession {
    pub(super) purpose: SessionPurpose,
    pub(super) stop_tx: Option<oneshot::Sender<()>>,
    pub(super) session_rx: Option<oneshot::Receiver<Box<dyn SttSession>>>,
    /// pump 线程据此决定收尾时 session.cancel() 还是交还 session
    pub(super) cancelled_flag: Arc<AtomicBool>,
    /// 录音采集 guard：drop 即停止采集线程
    pub(super) guard: Option<CaptureGuard>,
    pub(super) pump: tokio::task::JoinHandle<()>,
    pub(super) level_task: tokio::task::JoinHandle<()>,
    /// eval 录档句柄（仅 evalRecording 开启时存在；历史音频使用独立缓冲）
    pub(super) recorder: Option<crate::eval::SessionRecorder>,
}

/// The release callback belongs to the actual microphone handle, not a UI state.
/// AudioHandle::drop joins the capture thread before admission is released.
pub(super) struct CaptureGuard {
    audio: Option<AudioHandle>,
    _completion: CaptureCompletion,
}

impl Drop for CaptureGuard {
    fn drop(&mut self) {
        drop(self.audio.take());
    }
}

struct CaptureCompletion(Option<Arc<dyn Fn() + Send + Sync>>);

impl Drop for CaptureCompletion {
    fn drop(&mut self) {
        if let Some(finished) = &self.0 {
            finished();
        }
    }
}

impl Orchestrator {
    // ---------- 状态机操作 ----------

    /// 开始一次「按下到松手」的会话：建 STT session → 开录音 → Listening
    pub async fn begin(&self) -> Result<(), String> {
        self.begin_inner(false, SessionPurpose::Dictation, None, None)
            .await
    }

    /// solo 连续模式逐句续录：与 begin 同路径，但「录制音」提示由壳做区分
    /// （capture_started 事件带 resumed=true，不播录制音效，只保留逐句发送音）。
    pub(super) async fn begin_resumed(
        &self,
        purpose: SessionPurpose,
        gen: u64,
    ) -> Result<(), String> {
        self.begin_inner(true, purpose, Some(gen), None).await
    }

    pub(super) async fn begin_inner(
        &self,
        resumed: bool,
        purpose: SessionPurpose,
        expected_gen: Option<u64>,
        input_ticket: Option<u64>,
    ) -> Result<(), String> {
        let _op = self.op.lock().await;
        if !self.input_ticket_is_current(input_ticket) {
            return Ok(());
        }
        self.begin_admitted(resumed, purpose, expected_gen, false)
    }

    /// 调用方持有 op；所有来源共享录音入场和失败收尾，owner 只限制操作权限。
    pub(super) fn begin_admitted(
        &self,
        resumed: bool,
        purpose: SessionPurpose,
        expected_gen: Option<u64>,
        assistant_voice_owner: bool,
    ) -> Result<(), String> {
        let origin = if assistant_voice_owner {
            SessionOrigin::WindowVoice
        } else {
            SessionOrigin::Hotkey
        };
        self.begin_admitted_with(resumed, purpose, expected_gen, origin)
    }

    pub(super) fn begin_admitted_with(
        &self,
        resumed: bool,
        purpose: SessionPurpose,
        expected_gen: Option<u64>,
        origin: SessionOrigin,
    ) -> Result<(), String> {
        let assistant_voice_owner = origin == SessionOrigin::WindowVoice;
        if !self.runtime_inputs_enabled.load(Ordering::SeqCst) {
            return Err("运行时已停止，不能开始新会话".into());
        }
        let settings = self.settings.read().unwrap().clone();
        let mut policy = Self::configured_policy(&settings, purpose);
        let no_speech_timeout_ms = match origin {
            SessionOrigin::Wake {
                no_speech_timeout_ms,
            } => {
                // 唤醒后：说完即提交（与助手“一句话”预设相同），不等热键。
                policy = InteractionPolicy::from_preset(kotone_core_assistant_one_shot());
                Some(no_speech_timeout_ms)
            }
            _ => None,
        };
        {
            let mut inner = self.inner.lock().unwrap();
            if expected_gen.is_some_and(|gen| inner.gen != gen) {
                return Ok(());
            }
            if inner.state != OrchestratorState::Idle {
                return Err(format!("当前状态 {:?} 不能开始新会话", inner.state));
            }
            if inner.send_is_running() {
                return Err(SEND_STILL_RUNNING.into());
            }
            if resumed && !policy.continuous {
                // 本段沿用旧策略完成，下一段若已改成按键触发则等待真实按键。
                // 不能因旧段的 continuous=true 自动开启新的对讲机会话。
                inner.continuous_session = false;
                drop(inner);
                self.emit_state(OrchestratorState::Idle, None);
                return Ok(());
            }
            inner.purpose = purpose;
            // 在入场时确定请求去向，保留至预览、重试以及提醒到期。
            let legacy_reminder = purpose == SessionPurpose::Assistant
                && !matches!(origin, SessionOrigin::Wake { .. })
                && settings.ui.legacy_upgrade.has_legacy_reminders();
            let source = match origin {
                SessionOrigin::WindowVoice => "assistant-ui-voice-",
                SessionOrigin::Wake { .. } => "assistant-wake-",
                SessionOrigin::Hotkey => "session-",
            };
            inner.process_case_id = Some(format!(
                "{}{source}{}-{}",
                if legacy_reminder {
                    LEGACY_REMINDER_SESSION_PREFIX
                } else {
                    ""
                },
                crate::eval::new_session_id(),
                NEXT_PROCESS_SESSION_ID.fetch_add(1, Ordering::Relaxed),
            ));
            inner.assistant_voice_owner = assistant_voice_owner;
        }
        match self.begin_locked(resumed, purpose, settings, policy, no_speech_timeout_ms) {
            Ok(()) => Ok(()),
            Err(e) => {
                // 开始失败（如引擎/麦克风未就绪）：Error 保持到用户关闭或重试
                self.emit_process(
                    "session_failed",
                    json!({ "outcome": "error", "errorCode": e.code }),
                );
                self.toast_error(&e.message);
                Err(e.message)
            }
        }
    }

    fn begin_locked(
        &self,
        resumed: bool,
        purpose: SessionPurpose,
        settings: Settings,
        policy: InteractionPolicy,
        no_speech_timeout_ms: Option<u32>,
    ) -> Result<(), SessionBeginError> {
        if purpose == SessionPurpose::Assistant && self.assistant.is_none() {
            return Err(SessionBeginError::new(
                "ASSISTANT_NOT_CONFIGURED",
                "助手尚未配置，请先设置助手连接",
            ));
        }
        let stt_settings = self
            .runtime_stt_settings
            .read()
            .unwrap()
            .clone()
            .unwrap_or_else(|| settings.clone());
        let engine_id = stt_settings.stt_engine.clone();
        let engine = self.engines.get(&engine_id).ok_or_else(|| {
            SessionBeginError::new(
                "STT_ENGINE_NOT_REGISTERED",
                format!("未注册的 STT 引擎: {engine_id}"),
            )
        })?;
        if !engine.is_ready() {
            return Err(SessionBeginError::new(
                "STT_ENGINE_NOT_READY",
                format!(
                    "引擎「{}」未就绪（模型未下载），请到「设置 → 模型 → 语音识别」下载模型",
                    engine.display_name()
                ),
            ));
        }

        let active_profile = settings
            .active_profile_id
            .as_deref()
            .and_then(profile::get)
            .unwrap_or_else(GameProfile::builtin_generic);
        let mut cfg = build_session_config(&stt_settings, &engine_id, &active_profile);
        enrich_remote_connection(&stt_settings, &engine_id, &mut cfg, self.secrets.as_deref());

        let (stt_tx, stt_rx) = mpsc::unbounded_channel::<SttEvent>();
        let session = engine
            .start_session(&cfg, stt_tx)
            .map_err(|error| SessionBeginError::new("STT_SESSION_START_FAILED", error))?;

        let capture_completion = CaptureCompletion(self.after_capture.clone());
        if let Some(before_capture) = &self.before_capture {
            if let Err(error) = before_capture() {
                let mut session = session;
                session.cancel();
                return Err(SessionBeginError::new(
                    "AUDIO_CAPTURE_PREPARATION_FAILED",
                    error,
                ));
            }
        }

        // 开录音失败要有清晰错误；已建的 session 取消掉
        let mut handle = match self.audio.start(&settings.audio_device_id) {
            Ok(h) => h,
            Err(e) => {
                let mut session = session;
                session.cancel();
                return Err(SessionBeginError::new("AUDIO_CAPTURE_START_FAILED", e));
            }
        };
        let pcm_rx = handle.pcm_rx.take().expect("audio handle pcm channel");
        let level_rx = handle.level_rx.take().expect("audio handle level channel");
        let audio_err_rx = handle.error_rx.take();

        let (stop_tx, stop_rx) = oneshot::channel::<()>();
        let (session_tx, session_rx) = oneshot::channel::<Box<dyn SttSession>>();
        let cancelled_flag = Arc::new(AtomicBool::new(false));

        // eval 录档：evalRecording 开启时创建录档句柄，pump 边录边喂；
        // finalize 成功后落盘，取消则随会话句柄整体丢弃（docs/adr/005）。
        // history 音频使用自己的 PCM 缓冲，与评测录档完全解耦。
        // process case id 在 begin 入口生成；history/eval 开启时复用同一 id，
        // 便于诊断包里的 events.csv 与 history/eval 互查。
        let history_on = purpose == SessionPurpose::Dictation
            && settings.history.mode != crate::history::HistoryMode::Off;
        let process_case_id = self
            .inner
            .lock()
            .unwrap()
            .process_case_id
            .clone()
            .unwrap_or_else(|| format!("session-{}", crate::eval::new_session_id()));
        let session_id = history_on.then(|| process_case_id.clone());
        let recorder =
            (purpose == SessionPurpose::Dictation && settings.eval_recording).then(|| {
                let dir = self.eval_dir.clone().unwrap_or_else(crate::eval::eval_dir);
                crate::eval::SessionRecorder::new_with_id(dir, &engine_id, process_case_id.clone())
            });
        let history_draft = session_id.map(|sid| {
            Arc::new(Mutex::new(HistoryDraft {
                session_id: sid,
                started: std::time::Instant::now(),
                engine_id: engine_id.clone(),
                profile_id: settings.active_profile_id.clone(),
                audio_samples: 0,
                audio_pcm: settings.history.include_audio.then(Vec::new),
                audio_file: None,
                first_partial_ms: None,
                final_text: String::new(),
                source_text: None,
                process_duration_ms: None,
                finalize_latency_ms: None,
                reported: false,
                vad_speech_seen: false,
                speech_segments: Vec::new(),
            }))
        });

        // B3（VAD 静音判停，ADR-007）：初始化失败不能让整个输入模式失效。
        // 降级后本轮按普通 toggle 会话运行：再次按热键即可结束并发送；solo
        // 暂停自动续段，避免第二次热键按原语义把在途音频直接丢弃。
        let use_vad = policy.end == EndTrigger::VadSilence;
        let (vad_state, vad_warning_code) = if use_vad {
            match self.vad_factory.as_ref() {
                Some(factory) => match factory(&settings.vad) {
                    Ok(vad) => {
                        let silence_ms = settings.vad_silence_ms.clamp(
                            *crate::vad::SILENCE_MS_RANGE.start(),
                            *crate::vad::SILENCE_MS_RANGE.end(),
                        );
                        (
                            Some((
                                vad,
                                crate::vad::FrameSplitter::new(),
                                crate::vad::SilenceStopTracker::new(
                                    silence_ms,
                                    crate::vad::MIN_SESSION_MS,
                                )
                                .with_no_speech_timeout(no_speech_timeout_ms),
                            )),
                            None,
                        )
                    }
                    Err(error) => {
                        crate::log::log(&format!(
                            "VAD 初始化失败，本次会话降级为热键结束: {error}"
                        ));
                        (None, Some("VAD_INIT_FAILED"))
                    }
                },
                None => {
                    crate::log::log("VAD 工厂未接入，本次会话降级为热键结束");
                    (None, Some("VAD_NOT_CONFIGURED"))
                }
            }
        } else {
            (None, None)
        };
        let continuous_session = policy.continuous && vad_state.is_some();
        // 0 = 正常，1 = 推理失败但尚未上报，2 = 已上报。PCM pump 可能在
        // Listening 首个状态事件之前就拿到音频，这两个原子量保证降级不会漏报、
        // 不会重复上报，也不会被随后发出的普通 Listening 事件覆盖。
        let vad_inference_status = Arc::new(AtomicU8::new(0));
        let listening_state_emitted = Arc::new(AtomicBool::new(false));

        // PCM pump：录音 → session.push_audio（VAD 策略下并行喂 VAD）；
        // STT partial → kotone://partial；VAD 判停 → 触发 end()（与热键结束同路径）
        let emitter = self.emitter.clone();
        let pump_flag = cancelled_flag.clone();
        let pump_recorder = recorder.clone();
        let pump_history = history_draft.clone();
        let pump_case_id = process_case_id.clone();
        let pump_vad_inference_status = vad_inference_status.clone();
        let pump_listening_state_emitted = listening_state_emitted.clone();
        let max_session_duration = self.max_session_duration;
        let mut pump_first_partial_seen = false;
        // VAD 帧的累计采样偏移（相对 audio_pcm 起点；帧在流中连续，
        // 与 chunk 边界无关，每切出一帧 +VAD_FRAME_SAMPLES）
        let mut vad_frame_pos = 0usize;
        let me_weak = self.me.read().unwrap().clone();
        // begin 持有 op；所有异步结束回调必须携带本次即将发布的会话代际。
        let capture_gen = self.inner.lock().unwrap().gen + 1;
        let pump = tokio::spawn(async move {
            let mut session = session;
            let mut stop_rx = stop_rx;
            let mut pcm_rx = pcm_rx;
            let mut stt_rx = stt_rx;
            let mut audio_err_rx = audio_err_rx;
            let mut vad_state = vad_state;
            let mut stt_push_error = None;
            let session_deadline = tokio::time::sleep(max_session_duration);
            tokio::pin!(session_deadline);
            // 松手收尾：end() 先停采集再发停止信号，pump 收到信号后把通道里
            // 已缓冲未消费的 PCM 全部灌进 session（松手丢字 P0：采音侧排空）；
            // 取消（pump_flag）则立即停，音频丢弃
            loop {
                tokio::select! {
                    biased;
                    _ = &mut session_deadline => {
                        if let Some(me) = me_weak.as_ref().and_then(|weak| weak.upgrade()) {
                            tokio::spawn(async move {
                                me.abort_with_code(
                                    "单次录音已达 10 分钟上限，本次会话已中止，请重新开始",
                                    "SESSION_DURATION_LIMIT",
                                    None,
                                )
                                .await;
                            });
                        }
                        break;
                    }
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
                                    let mut h = h.lock().unwrap();
                                    h.audio_samples += c.len() as u64;
                                    if let Some(pcm) = &mut h.audio_pcm {
                                        pcm.extend_from_slice(&c);
                                    }
                                }
                                if let Err(e) = session.push_audio(&c) {
                                    stt_push_error = Some(e);
                                    break;
                                }
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
                                    let mut h = h.lock().unwrap();
                                    h.audio_samples += c.len() as u64;
                                    if let Some(pcm) = &mut h.audio_pcm {
                                        pcm.extend_from_slice(&c);
                                    }
                                }
                                if let Err(e) = session.push_audio(&c) {
                                    stt_push_error = Some(e);
                                    break;
                                }
                                // VAD 帧判定与 STT 并行（同一 PCM 流）；
                                // 推理失败只禁用判停（热键强制结束兜底），不断会话
                                if let Some((vad, splitter, tracker)) = &mut vad_state {
                                    let mut decision = crate::vad::StopDecision::Continue;
                                    let mut failed = None;
                                    for frame in splitter.push(&c) {
                                        match vad.push_frame(&frame) {
                                            Ok(speech) => {
                                                if speech {
                                                    // 语音判定：标记「开过口」（取消判据）
                                                    // 并记录语音区间（历史音频裁剪用）
                                                    if let Some(h) = &pump_history {
                                                        let mut g = h.lock().unwrap();
                                                        g.vad_speech_seen = true;
                                                        g.push_speech_range(
                                                            vad_frame_pos,
                                                            vad_frame_pos
                                                                + crate::vad::VAD_FRAME_SAMPLES,
                                                        );
                                                    }
                                                }
                                                decision = tracker.push(speech);
                                                if decision != crate::vad::StopDecision::Continue {
                                                    break;
                                                }
                                            }
                                            Err(e) => {
                                                failed = Some(e);
                                                break;
                                            }
                                        }
                                        vad_frame_pos += crate::vad::VAD_FRAME_SAMPLES;
                                    }
                                    if let Some(e) = failed {
                                        crate::log::log(&format!(
                                            "VAD 推理失败，本次会话禁用静音判停（热键仍可结束）: {e}"
                                        ));
                                        pump_vad_inference_status.store(1, Ordering::SeqCst);
                                        if pump_listening_state_emitted.load(Ordering::SeqCst)
                                            && pump_vad_inference_status
                                                .compare_exchange(
                                                    1,
                                                    2,
                                                    Ordering::SeqCst,
                                                    Ordering::SeqCst,
                                                )
                                                .is_ok()
                                        {
                                            let me = me_weak
                                                .as_ref()
                                                .and_then(|weak| weak.upgrade());
                                            let listening = {
                                                if let Some(me) = &me {
                                                    let mut inner = me.inner.lock().unwrap();
                                                    if inner.state
                                                        == OrchestratorState::Listening
                                                    {
                                                        inner.continuous_session = false;
                                                        true
                                                    } else {
                                                        false
                                                    }
                                                } else {
                                                    false
                                                }
                                            };
                                            if let Some(me) = me {
                                                if listening {
                                                    me.emit_state(
                                                        OrchestratorState::Listening,
                                                        Some(json!({
                                                            "warning": me.vad_fallback_warning(),
                                                            "manualStop": true,
                                                            "preserveText": true
                                                        })),
                                                    );
                                                }
                                                // 即使用户恰好已结束录音，也保留稳定错误码，
                                                // 但不把状态倒退回 Listening。
                                                me.emit_process(
                                                    "vad_degraded",
                                                    json!({
                                                        "outcome": "warning",
                                                        "errorCode": "VAD_INFERENCE_FAILED",
                                                        "fallback": "hotkey_stop"
                                                    }),
                                                );
                                            }
                                        }
                                        vad_state = None;
                                    } else if decision == crate::vad::StopDecision::NoSpeech {
                                        // 唤醒后一直没开口：安静取消，不转写、不提交
                                        emitter.emit(
                                            "kotone://vad-no-speech",
                                            json!({ "elapsedMs": tracker.elapsed_ms() }),
                                        );
                                        if let Some(me) =
                                            me_weak.as_ref().and_then(|w| w.upgrade())
                                        {
                                            tokio::spawn(async move {
                                                me.cancel_inner(None, Some(capture_gen)).await;
                                            });
                                        }
                                        break;
                                    } else if decision == crate::vad::StopDecision::Stop {
                                        emitter.emit(
                                            "kotone://vad-stop",
                                            json!({ "elapsedMs": tracker.elapsed_ms() }),
                                        );
                                        if let Some(me) =
                                            me_weak.as_ref().and_then(|w| w.upgrade())
                                        {
                                            tokio::spawn(async move {
                                                let _ = me.end_inner(Some(capture_gen), None).await;
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
                    err = async {
                        match &mut audio_err_rx {
                            Some(rx) => rx.recv().await,
                            // 无错误通道（mock/测试）：分支永不完成，不影响原有 select
                            None => std::future::pending::<Option<String>>().await,
                        }
                    } => {
                        if let Some(msg) = err {
                            // 录音设备中途故障（拔麦克风/驱动错误）：中止会话并 toast，
                            // 避免状态机永久停在 Listening 无感知（P1-⑤）
                            if let Some(me) = me_weak.as_ref().and_then(|w| w.upgrade()) {
                                let msg2 = msg.clone();
                                tokio::spawn(async move {
                                    me.abort_with_code(
                                        &format!("{msg2}（本次会话已中止，请检查麦克风后重试）"),
                                        "AUDIO_DEVICE_ERROR",
                                        Some(capture_gen),
                                    )
                                    .await;
                                });
                            }
                            break;
                        }
                        // 通道关闭 = 采集线程正常退出（pcm 分支会收尾）；停用该分支防 busy 循环
                        audio_err_rx = None;
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
                                if !pump_first_partial_seen {
                                    pump_first_partial_seen = true;
                                    emitter.emit(
                                        "kotone://process",
                                        json!({
                                            "caseId": pump_case_id.clone(),
                                            "activity": "first_partial",
                                            "data": { "textChars": text.chars().count() as u64 }
                                        }),
                                    );
                                }
                                emitter.emit("kotone://partial", json!({ "text": text }));
                            }
                            // Final 由 finalize 返回值承载，这里不重复上屏
                            Some(SttEvent::Final { .. }) | None => {}
                        }
                    }
                }
            }
            if let Some(error) = stt_push_error {
                session.cancel();
                drop(session);
                if let Some(me) = me_weak.as_ref().and_then(|w| w.upgrade()) {
                    me.abort_with_code(
                        &format!("语音识别处理音频失败：{error}（本次会话已中止，请重试）"),
                        "STT_AUDIO_PUSH_FAILED",
                        None,
                    )
                    .await;
                }
                return;
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
        inner.session_policy = Some(policy);
        inner.pending_text = None;
        inner.processing_cancel = None;
        // 目标窗口记忆：用户按下热键说话前所在的前台窗口 = 注入目标，
        // 发送前（do_send）会把焦点还给它，避免 preview 交互抢焦点导致注入打错窗口
        inner.target_window = self.focus.foreground_window();
        inner.history = history_draft;
        inner.continuous_session =
            continuous_session && vad_inference_status.load(Ordering::SeqCst) == 0;
        inner.active = Some(ActiveSession {
            purpose,
            stop_tx: Some(stop_tx),
            session_rx: Some(session_rx),
            cancelled_flag,
            guard: Some(CaptureGuard {
                audio: Some(handle),
                _completion: capture_completion,
            }),
            pump,
            level_task,
            recorder,
        });
        drop(inner);
        let listening_payload = vad_warning_code.map(|_| {
            json!({
                "warning": self.vad_fallback_warning(),
                "manualStop": true
            })
        });
        self.emit_state(OrchestratorState::Listening, listening_payload);
        listening_state_emitted.store(true, Ordering::SeqCst);
        if let Some(error_code) = vad_warning_code {
            self.emit_process(
                "vad_degraded",
                json!({
                    "outcome": "warning",
                    "errorCode": error_code,
                    "fallback": "hotkey_stop"
                }),
            );
        }
        if vad_inference_status
            .compare_exchange(1, 2, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            let listening = {
                let mut inner = self.inner.lock().unwrap();
                if inner.state == OrchestratorState::Listening {
                    inner.continuous_session = false;
                    true
                } else {
                    false
                }
            };
            if listening {
                self.emit_state(
                    OrchestratorState::Listening,
                    Some(json!({
                        "warning": self.vad_fallback_warning(),
                        "manualStop": true,
                        "preserveText": true
                    })),
                );
            }
            self.emit_process(
                "vad_degraded",
                json!({
                    "outcome": "warning",
                    "errorCode": "VAD_INFERENCE_FAILED",
                    "fallback": "hotkey_stop"
                }),
            );
        }
        self.emit_process("capture_started", json!({ "resumed": resumed }));
        Ok(())
    }
}

/// 会话配置组装（纯函数，可单测）：profile 热词 → SessionConfig.hotwords，
/// 引擎专有配置取 engineOptions[engine_id]（缺省 Null）
pub(super) fn build_session_config(
    settings: &Settings,
    engine_id: &str,
    active_profile: &GameProfile,
) -> SessionConfig {
    SessionConfig {
        language: settings.language.clone(),
        hotwords: active_profile.hotwords.clone(),
        hotwords_score: settings.hotwords_score,
        options: settings.session_options(engine_id),
    }
}

fn enrich_remote_connection(
    settings: &Settings,
    engine_id: &str,
    cfg: &mut SessionConfig,
    secrets: Option<&dyn crate::connection::SecretStore>,
) {
    let configured = cfg
        .options
        .get("connectionId")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string);
    // DeepSeek 语音识别不用单独选连接：用琴音的 DeepSeek 连接，没有就用第一条 DeepSeek 连接。
    let connection_id = configured.or_else(|| {
        (engine_id == DEEPSEEK_ASR_ENGINE_ID)
            .then(|| settings.deepseek_connection_id())
            .flatten()
    });
    let Some(connection_id) = connection_id else {
        return;
    };
    let Some(connection) = settings
        .connections
        .iter()
        .find(|item| item.id == connection_id)
    else {
        return;
    };
    if let Some(opts) = cfg.options.as_object_mut() {
        opts.insert("baseUrl".into(), serde_json::json!(connection.base_url));
        opts.insert("remoteModel".into(), serde_json::json!(connection.model));
        if let Some(store) = secrets {
            if let Ok(Some(key)) = store.get(&connection_id) {
                opts.insert("apiKey".into(), serde_json::json!(key));
            }
        }
    }
}

/// 与 `kotone_stt::model::DEEPSEEK_ASR_STT_ID` 相同（core 不依赖 stt）。
const DEEPSEEK_ASR_ENGINE_ID: &str = "deepseek-asr";
