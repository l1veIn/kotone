use super::*;

/// 每会话一份 VAD（B3 静音判停，ADR-007）：模型实例、30ms 切帧、判停状态机。
pub(super) struct VadStop {
    pub(super) vad: Box<dyn crate::vad::Vad>,
    pub(super) splitter: crate::vad::FrameSplitter,
    pub(super) tracker: crate::vad::SilenceStopTracker,
}

/// VAD 推理失败的上报状态，pump 与 begin 共享。
/// `inference_status`：0 = 正常，1 = 推理失败但尚未上报，2 = 已上报。PCM pump 可能在
/// Listening 首个状态事件之前就拿到音频，这两个原子量保证降级不会漏报、
/// 不会重复上报，也不会被随后发出的普通 Listening 事件覆盖。
#[derive(Default)]
pub(super) struct VadDegradeSignal {
    pub(super) inference_status: AtomicU8,
    pub(super) listening_state_emitted: AtomicBool,
}

/// PCM pump：录音 → session.push_audio（VAD 策略下并行喂 VAD）；
/// STT partial → kotone://partial；VAD 判停 → 触发 end()（与热键结束同路径）
pub(super) struct CapturePump {
    pub(super) session: Box<dyn SttSession>,
    pub(super) stop_rx: oneshot::Receiver<()>,
    pub(super) session_tx: oneshot::Sender<Box<dyn SttSession>>,
    pub(super) pcm_rx: crate::audio::AudioReceiver<Vec<f32>>,
    pub(super) stt_rx: mpsc::UnboundedReceiver<SttEvent>,
    pub(super) audio_err_rx: Option<mpsc::UnboundedReceiver<String>>,
    pub(super) vad: Option<VadStop>,
    /// 取消标记：收尾时 session.cancel() 还是交还 session
    pub(super) cancelled: Arc<AtomicBool>,
    pub(super) recorder: Option<crate::eval::SessionRecorder>,
    pub(super) history: Option<Arc<Mutex<HistoryDraft>>>,
    pub(super) emitter: Arc<dyn Emitter>,
    pub(super) case_id: String,
    pub(super) vad_degrade: Arc<VadDegradeSignal>,
    pub(super) max_session_duration: Duration,
    pub(super) first_partial_seen: bool,
    /// VAD 帧的累计采样偏移（相对 audio_pcm 起点；帧在流中连续，
    /// 与 chunk 边界无关，每切出一帧 +VAD_FRAME_SAMPLES）
    pub(super) vad_frame_pos: usize,
    /// 自引用（Weak）：VAD 判停 / 设备故障时回调 end / cancel / abort
    pub(super) me: Option<std::sync::Weak<Orchestrator>>,
    /// begin 持有 op；所有异步结束回调必须携带本次即将发布的会话代际。
    pub(super) capture_gen: u64,
}

impl CapturePump {
    pub(super) fn spawn(self) -> tokio::task::JoinHandle<()> {
        tokio::spawn(self.run())
    }

    fn orchestrator(&self) -> Option<Arc<Orchestrator>> {
        self.me.as_ref().and_then(|weak| weak.upgrade())
    }

    async fn run(mut self) {
        let mut stt_push_error = None;
        let session_deadline = tokio::time::sleep(self.max_session_duration);
        tokio::pin!(session_deadline);
        // 松手收尾：end() 先停采集再发停止信号，pump 收到信号后把通道里
        // 已缓冲未消费的 PCM 全部灌进 session（松手丢字 P0：采音侧排空）；
        // 取消（cancelled）则立即停，音频丢弃
        loop {
            tokio::select! {
                biased;
                _ = &mut session_deadline => {
                    self.abort_at_duration_limit();
                    break;
                }
                r = &mut self.stop_rx => {
                    let _ = r;
                    if !self.cancelled.load(Ordering::SeqCst) {
                        // 非阻塞排空：此时采集线程已被 join（end 先 drop
                        // guard），通道里剩下的就是全部缓冲
                        stt_push_error = self.drain_buffered_pcm().err();
                    }
                    break;
                }
                chunk = self.pcm_rx.recv() => {
                    match chunk {
                        Some(c) => {
                            if let Err(e) = self.feed_pcm(&c) {
                                stt_push_error = Some(e);
                                break;
                            }
                            if self.feed_vad(&c) {
                                break;
                            }
                        }
                        None => break, // 采集结束
                    }
                }
                err = next_audio_error(&mut self.audio_err_rx) => {
                    if let Some(msg) = err {
                        self.abort_on_audio_error(msg);
                        break;
                    }
                    // 通道关闭 = 采集线程正常退出（pcm 分支会收尾）；停用该分支防 busy 循环
                    self.audio_err_rx = None;
                }
                ev = self.stt_rx.recv() => self.on_stt_event(ev),
            }
        }
        self.finish(stt_push_error).await;
    }

    fn abort_at_duration_limit(&self) {
        if let Some(me) = self.orchestrator() {
            tokio::spawn(async move {
                me.abort_with_code(
                    "单次录音已达 10 分钟上限，本次会话已中止，请重新开始",
                    "SESSION_DURATION_LIMIT",
                    None,
                )
                .await;
            });
        }
    }

    /// 录音设备中途故障（拔麦克风/驱动错误）：中止会话并 toast，
    /// 避免状态机永久停在 Listening 无感知（P1-⑤）
    fn abort_on_audio_error(&self, msg: String) {
        if let Some(me) = self.orchestrator() {
            let capture_gen = self.capture_gen;
            tokio::spawn(async move {
                me.abort_with_code(
                    &format!("{msg}（本次会话已中止，请检查麦克风后重试）"),
                    "AUDIO_DEVICE_ERROR",
                    Some(capture_gen),
                )
                .await;
            });
        }
    }

    fn drain_buffered_pcm(&mut self) -> Result<(), String> {
        while let Ok(c) = self.pcm_rx.try_recv() {
            self.feed_pcm(&c)?;
        }
        Ok(())
    }

    /// 一块 PCM：评测录档、识别记录草稿、STT session，顺序固定。
    fn feed_pcm(&mut self, c: &[f32]) -> Result<(), String> {
        if let Some(rec) = &self.recorder {
            rec.push_pcm(c);
        }
        if let Some(h) = &self.history {
            let mut h = h.lock().unwrap();
            h.audio_samples += c.len() as u64;
            if let Some(pcm) = &mut h.audio_pcm {
                pcm.extend_from_slice(c);
            }
        }
        self.session.push_audio(c)
    }

    /// VAD 帧判定与 STT 并行（同一 PCM 流）；
    /// 推理失败只禁用判停（热键强制结束兜底），不断会话。
    /// 返回 true = 会话已交给 end / cancel，pump 立即退出。
    fn feed_vad(&mut self, c: &[f32]) -> bool {
        let Some(VadStop {
            vad,
            splitter,
            tracker,
        }) = &mut self.vad
        else {
            return false;
        };
        let mut decision = crate::vad::StopDecision::Continue;
        let mut failed = None;
        for frame in splitter.push(c) {
            match vad.push_frame(&frame) {
                Ok(speech) => {
                    if speech {
                        // 语音判定：标记「开过口」（取消判据）
                        // 并记录语音区间（历史音频裁剪用）
                        if let Some(h) = &self.history {
                            let mut g = h.lock().unwrap();
                            g.vad_speech_seen = true;
                            g.push_speech_range(
                                self.vad_frame_pos,
                                self.vad_frame_pos + crate::vad::VAD_FRAME_SAMPLES,
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
            self.vad_frame_pos += crate::vad::VAD_FRAME_SAMPLES;
        }
        if let Some(e) = failed {
            crate::log::log(&format!(
                "VAD 推理失败，本次会话禁用静音判停（热键仍可结束）: {e}"
            ));
            self.vad_degrade.inference_status.store(1, Ordering::SeqCst);
            if self
                .vad_degrade
                .listening_state_emitted
                .load(Ordering::SeqCst)
                && self
                    .vad_degrade
                    .inference_status
                    .compare_exchange(1, 2, Ordering::SeqCst, Ordering::SeqCst)
                    .is_ok()
            {
                if let Some(me) = self.orchestrator() {
                    me.report_vad_inference_failure();
                }
            }
            self.vad = None;
            false
        } else if decision == crate::vad::StopDecision::NoSpeech {
            // 唤醒后一直没开口：安静取消，不转写、不提交
            self.emitter.emit(
                "kotone://vad-no-speech",
                json!({ "elapsedMs": tracker.elapsed_ms() }),
            );
            if let Some(me) = self.orchestrator() {
                let capture_gen = self.capture_gen;
                tokio::spawn(async move {
                    me.cancel_inner(None, Some(capture_gen)).await;
                });
            }
            true
        } else if decision == crate::vad::StopDecision::Stop {
            self.emitter.emit(
                "kotone://vad-stop",
                json!({ "elapsedMs": tracker.elapsed_ms() }),
            );
            if let Some(me) = self.orchestrator() {
                let capture_gen = self.capture_gen;
                tokio::spawn(async move {
                    let _ = me.end_inner(Some(capture_gen), None).await;
                });
            }
            // 立刻交出 session：end() 正在等 session_rx
            true
        } else {
            false
        }
    }

    fn on_stt_event(&mut self, ev: Option<SttEvent>) {
        match ev {
            Some(SttEvent::Partial { text }) => {
                if let Some(rec) = &self.recorder {
                    rec.push_partial(&text);
                }
                if let Some(h) = &self.history {
                    let mut g = h.lock().unwrap();
                    if g.first_partial_ms.is_none() {
                        g.first_partial_ms = Some(g.started.elapsed().as_millis() as u64);
                    }
                }
                if !self.first_partial_seen {
                    self.first_partial_seen = true;
                    self.emitter.emit(
                        "kotone://process",
                        json!({
                            "caseId": self.case_id.clone(),
                            "activity": "first_partial",
                            "data": { "textChars": text.chars().count() as u64 }
                        }),
                    );
                }
                self.emitter
                    .emit("kotone://partial", json!({ "text": text }));
            }
            // Final 由 finalize 返回值承载，这里不重复上屏
            Some(SttEvent::Final { .. }) | None => {}
        }
    }

    async fn finish(self, stt_push_error: Option<String>) {
        let CapturePump {
            mut session,
            session_tx,
            cancelled,
            me,
            ..
        } = self;
        if let Some(error) = stt_push_error {
            session.cancel();
            drop(session);
            if let Some(me) = me.as_ref().and_then(|w| w.upgrade()) {
                me.abort_with_code(
                    &format!("语音识别处理音频失败：{error}（本次会话已中止，请重试）"),
                    "STT_AUDIO_PUSH_FAILED",
                    None,
                )
                .await;
            }
            return;
        }
        if cancelled.load(Ordering::SeqCst) {
            session.cancel();
        }
        // 取消路径下接收端已 drop，send 失败则 session 随之释放
        let _ = session_tx.send(session);
    }
}

/// 录音错误通道；没有错误通道（mock/测试）时永不完成，不影响 select 的其它分支。
async fn next_audio_error(rx: &mut Option<mpsc::UnboundedReceiver<String>>) -> Option<String> {
    match rx {
        Some(rx) => rx.recv().await,
        None => std::future::pending::<Option<String>>().await,
    }
}

/// RMS 电平 → kotone://level（驱动波形，~50ms 一条）
pub(super) fn spawn_level_forwarder(
    emitter: Arc<dyn Emitter>,
    level_rx: crate::audio::AudioReceiver<f32>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut level_rx = level_rx;
        while let Some(rms) = level_rx.recv().await {
            emitter.emit("kotone://level", json!({ "rms": rms }));
        }
    })
}
