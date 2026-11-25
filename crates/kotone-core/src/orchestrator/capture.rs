use super::*;

mod pump;

use self::pump::{spawn_level_forwarder, CapturePump, VadDegradeSignal, VadStop};

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

/// 录音的旁听者（评测录档、识别记录）：都要 PCM、识别中的字（第一个字的时间）、
/// VAD 判为语音的段落。pump 在自己的任务里按登记顺序逐个通知，各自在自己的锁里记账；
/// 旁听者不能拦住或改变喂给识别的音频。
pub trait CaptureObserver: Send + Sync {
    /// 一块要喂给识别的 PCM（16kHz mono f32），在喂给 STT session 之前通知
    fn on_pcm(&self, chunk: &[f32]);
    /// 识别中的字（每个 partial 一次）
    fn on_partial(&self, text: &str);
    /// VAD 判为语音的一帧：采样区间 [start, end)，相对本次收音的第一块 PCM
    fn on_speech_frame(&self, start: usize, end: usize);
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
    pub async fn begin_resumed(&self, purpose: SessionPurpose, gen: u64) -> Result<(), String> {
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

    /// 副作用顺序固定：建 STT session → before_capture → 开录音 → 发布会话（代际 +1）。
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
        let SttStart {
            engine_id,
            session,
            stt_rx,
        } = self.start_stt_session(&settings)?;
        let (session, capture_completion, mut handle) = self.open_microphone(&settings, session)?;
        let pcm_rx = handle.pcm_rx.take().expect("audio handle pcm channel");
        let level_rx = handle.level_rx.take().expect("audio handle level channel");
        let audio_err_rx = handle.error_rx.take();

        let (stop_tx, stop_rx) = oneshot::channel::<()>();
        let (session_tx, session_rx) = oneshot::channel::<Box<dyn SttSession>>();
        let cancelled_flag = Arc::new(AtomicBool::new(false));

        let (process_case_id, recorder, record) =
            self.start_capture_records(purpose, &settings, &engine_id);
        let (vad, vad_warning_code) = self.prepare_vad(&settings, policy, no_speech_timeout_ms);
        let continuous_session = policy.continuous && vad.is_some();
        let vad_degrade = Arc::new(VadDegradeSignal::default());

        // 旁听者顺序固定：评测录档在前、识别记录在后。
        let mut observers: Vec<Arc<dyn CaptureObserver>> = Vec::new();
        if let Some(recorder) = &recorder {
            observers.push(Arc::new(recorder.clone()));
        }
        if let Some(observer) = record.observer() {
            observers.push(observer);
        }

        let me = self.me.read().unwrap().clone();
        // begin 持有 op；所有异步结束回调必须携带本次即将发布的会话代际。
        let capture_gen = self.inner.lock().unwrap().gen + 1;
        let pump = CapturePump {
            session,
            stop_rx,
            session_tx,
            pcm_rx,
            stt_rx,
            audio_err_rx,
            vad,
            cancelled: cancelled_flag.clone(),
            observers,
            emitter: self.emitter.clone(),
            case_id: process_case_id,
            vad_degrade: vad_degrade.clone(),
            max_session_duration: self.max_session_duration,
            first_partial_seen: false,
            vad_frame_pos: 0,
            me,
            capture_gen,
        }
        .spawn();
        let level_task = spawn_level_forwarder(self.emitter.clone(), level_rx);

        self.publish_listening(
            resumed,
            PreparedCapture {
                policy,
                active: ActiveSession {
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
                },
                record,
                continuous_session,
                vad_degrade,
                vad_warning_code,
            },
        );
        Ok(())
    }

    /// 查引擎、组会话配置（热词、远程连接）、建 STT session。
    fn start_stt_session(&self, settings: &Settings) -> Result<SttStart, SessionBeginError> {
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
        crate::stt::add_remote_connection(
            &stt_settings,
            &engine_id,
            &mut cfg,
            self.secrets.as_deref(),
        );

        let (stt_tx, stt_rx) = mpsc::unbounded_channel::<SttEvent>();
        let session = engine
            .start_session(&cfg, stt_tx)
            .map_err(|error| SessionBeginError::new("STT_SESSION_START_FAILED", error))?;
        Ok(SttStart {
            engine_id,
            session,
            stt_rx,
        })
    }

    /// before_capture（壳先停本机播报）→ 开录音。失败时取消已建的 session；
    /// CaptureCompletion 先于 before_capture 建好，失败返回时随之 drop（after_capture 恰好一次）。
    fn open_microphone(
        &self,
        settings: &Settings,
        session: Box<dyn SttSession>,
    ) -> Result<(Box<dyn SttSession>, CaptureCompletion, AudioHandle), SessionBeginError> {
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
        let handle = match self.audio.start(&settings.audio_device_id) {
            Ok(h) => h,
            Err(e) => {
                let mut session = session;
                session.cancel();
                return Err(SessionBeginError::new("AUDIO_CAPTURE_START_FAILED", e));
            }
        };
        Ok((session, capture_completion, handle))
    }

    /// eval 录档：evalRecording 开启时创建录档句柄，pump 边录边喂；
    /// finalize 成功后落盘，取消则随会话句柄整体丢弃（docs/adr/005）。
    /// history 音频使用自己的 PCM 缓冲，与评测录档完全解耦。
    /// process case id 在 begin 入口生成；history/eval 开启时复用同一 id，
    /// 便于诊断包里的 events.csv 与 history/eval 互查。
    fn start_capture_records(
        &self,
        purpose: SessionPurpose,
        settings: &Settings,
        engine_id: &str,
    ) -> (String, Option<crate::eval::SessionRecorder>, RecordDraft) {
        let process_case_id = self
            .inner
            .lock()
            .unwrap()
            .process_case_id
            .clone()
            .unwrap_or_else(|| format!("session-{}", crate::eval::new_session_id()));
        let recorder =
            (purpose == SessionPurpose::Dictation && settings.eval_recording).then(|| {
                let dir = self.eval_dir.clone().unwrap_or_else(crate::eval::eval_dir);
                crate::eval::SessionRecorder::new_with_id(dir, engine_id, process_case_id.clone())
            });
        // 识别记录草稿（语音输入；history 关着或不是听写时为空）
        let record = self.start_record(purpose, settings, engine_id, &process_case_id);
        (process_case_id, recorder, record)
    }

    /// B3（VAD 静音判停，ADR-007）：初始化失败不能让整个输入模式失效。
    /// 降级后本轮按普通 toggle 会话运行：再次按热键即可结束并发送；solo
    /// 暂停自动续段，避免第二次热键按原语义把在途音频直接丢弃。
    fn prepare_vad(
        &self,
        settings: &Settings,
        policy: InteractionPolicy,
        no_speech_timeout_ms: Option<u32>,
    ) -> (Option<VadStop>, Option<&'static str>) {
        let use_vad = policy.end == EndTrigger::VadSilence;
        if use_vad {
            match self.vad_factory.as_ref() {
                Some(factory) => match factory(&settings.vad) {
                    Ok(vad) => {
                        let silence_ms = settings.vad_silence_ms.clamp(
                            *crate::vad::SILENCE_MS_RANGE.start(),
                            *crate::vad::SILENCE_MS_RANGE.end(),
                        );
                        (
                            Some(VadStop {
                                vad,
                                splitter: crate::vad::FrameSplitter::new(),
                                tracker: crate::vad::SilenceStopTracker::new(
                                    silence_ms,
                                    crate::vad::MIN_SESSION_MS,
                                )
                                .with_no_speech_timeout(no_speech_timeout_ms),
                            }),
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
        }
    }

    /// 发布会话：代际 +1、进入 Listening、记下注入目标窗口，再发 Listening 与诊断事件。
    fn publish_listening(&self, resumed: bool, capture: PreparedCapture) {
        let PreparedCapture {
            policy,
            active,
            record,
            continuous_session,
            vad_degrade,
            vad_warning_code,
        } = capture;
        let mut inner = self.inner.lock().unwrap();
        inner.gen += 1;
        inner.state = OrchestratorState::Listening;
        inner.session_policy = Some(policy);
        inner.pending_text = None;
        inner.processing_cancel = None;
        // 目标窗口记忆：用户按下热键说话前所在的前台窗口 = 注入目标，
        // 发送前（do_send）会把焦点还给它，避免 preview 交互抢焦点导致注入打错窗口；
        // 识别记录草稿一并交给语音输入
        self.on_capture_started(self.focus.foreground_window(), record);
        inner.continuous_session =
            continuous_session && vad_degrade.inference_status.load(Ordering::SeqCst) == 0;
        inner.active = Some(active);
        drop(inner);
        let listening_payload = vad_warning_code.map(|_| {
            json!({
                "warning": self.vad_fallback_warning(),
                "manualStop": true
            })
        });
        self.emit_state(OrchestratorState::Listening, listening_payload);
        vad_degrade
            .listening_state_emitted
            .store(true, Ordering::SeqCst);
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
        if vad_degrade
            .inference_status
            .compare_exchange(1, 2, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            self.report_vad_inference_failure();
        }
        self.emit_process("capture_started", json!({ "resumed": resumed }));
    }

    /// VAD 推理失败（pump 或 begin 恰好一次）：仍在 Listening 时关掉连续模式并提示改用热键结束。
    /// 即使用户恰好已结束录音，也保留稳定错误码，但不把状态倒退回 Listening。
    fn report_vad_inference_failure(&self) {
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
}

/// start_stt_session 的产物：引擎 id、STT session 和它的事件通道。
struct SttStart {
    engine_id: String,
    session: Box<dyn SttSession>,
    stt_rx: mpsc::UnboundedReceiver<SttEvent>,
}

/// 准备好、等着发布进 Inner 的一次收音。
struct PreparedCapture {
    policy: InteractionPolicy,
    active: ActiveSession,
    record: RecordDraft,
    continuous_session: bool,
    vad_degrade: Arc<VadDegradeSignal>,
    vad_warning_code: Option<&'static str>,
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
