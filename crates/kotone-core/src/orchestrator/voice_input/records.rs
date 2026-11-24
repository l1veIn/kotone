use crate::orchestrator::capture::CaptureObserver;
use crate::orchestrator::*;

/// history 草稿：会话期间累计指标，终态（sent/cancelled/error）时落账一条记录
pub(crate) struct HistoryDraft {
    /// 与 eval 录档共用的 sessionId（begin 生成一次同时喂两边，可互查）
    pub(crate) session_id: String,
    pub(crate) started: std::time::Instant,
    pub(crate) engine_id: String,
    pub(crate) profile_id: Option<String>,
    /// 已喂入的采样数（落账时按 eval::SAMPLE_RATE 换算 ms；persist 裁剪后
    /// 重算为实际语音时长，与保存的 wav 一致）
    pub(crate) audio_samples: u64,
    /// 历史音频独立缓冲；仅 history.includeAudio 开启时分配，
    /// 不依赖 evalRecording，也不会写入 ~/.kotone/eval。
    pub(crate) audio_pcm: Option<Vec<f32>>,
    /// finalize 成功后写入 history/audio 的相对文件名。
    pub(crate) audio_file: Option<String>,
    pub(crate) first_partial_ms: Option<u64>,
    pub(crate) final_text: String,
    pub(crate) source_text: Option<String>,
    pub(crate) process_duration_ms: Option<u64>,
    pub(crate) finalize_latency_ms: Option<u64>,
    /// error 终态已落账（Error 态保留文本供重试：重试成功仍写 sent；
    /// error 后的 cancel 是清理动作，不再双记 cancelled）
    pub(crate) reported: bool,
    /// VAD 判定过语音（one-shot/solo 模式；取消时据此判断「是否真的开过口」，
    /// 比 STT partial 可靠——噪音也会让流式引擎出 partial，但 silero 判定 speech
    /// 才是「疑似说话」。非 VAD 模式恒 false，判据退化到 first_partial_ms）
    pub(crate) vad_speech_seen: bool,
    /// VAD 判定为语音的采样区间（半开 [start, end)，采样号相对 audio_pcm 起点；
    /// 相邻段间隙 < SPEECH_GAP_SAMPLES 时合并保留句间短停顿）。
    /// 仅 VAD 模式填充；persist 时按区间拼接裁剪掉段首/段尾静音。
    pub(crate) speech_segments: Vec<(usize, usize)>,
}

/// 语音区间合并间隙：相邻语音段间 ≤ 此采样数视为句间停顿（保留），
/// 超过则断开（300ms @16kHz）。句内换气/短语停顿一般 < 300ms。
const SPEECH_GAP_SAMPLES: usize = 480 * 10; // 300ms

impl HistoryDraft {
    /// 开始收音时建（与 eval 录档共用 session id）；只有 history.includeAudio 开着才另存 PCM。
    pub(super) fn new(session_id: String, engine_id: &str, settings: &Settings) -> Self {
        HistoryDraft {
            session_id,
            started: std::time::Instant::now(),
            engine_id: engine_id.to_string(),
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
        }
    }

    /// 追加一段语音区间（按 VAD 帧判定逐步喂入）：与上一段间隙小则合并。
    pub(crate) fn push_speech_range(&mut self, start: usize, end: usize) {
        if start >= end {
            return;
        }
        if let Some(last) = self.speech_segments.last_mut() {
            if start <= last.1 + SPEECH_GAP_SAMPLES {
                last.1 = last.1.max(end);
                return;
            }
        }
        self.speech_segments.push((start, end));
    }

    #[cfg(test)]
    pub(crate) fn speech_harness() -> Self {
        HistoryDraft {
            session_id: "test".to_string(),
            started: std::time::Instant::now(),
            engine_id: "test".to_string(),
            profile_id: None,
            audio_samples: 0,
            audio_pcm: None,
            audio_file: None,
            first_partial_ms: None,
            final_text: String::new(),
            source_text: None,
            process_duration_ms: None,
            finalize_latency_ms: None,
            reported: false,
            vad_speech_seen: false,
            speech_segments: Vec::new(),
        }
    }
}

/// 识别记录草稿旁听录音：每次通知锁一次草稿。
/// PCM 计采样数（includeAudio 开时另存一份）；只记第一个字的时间；
/// VAD 语音帧标记「开过口」（取消判据）并记录语音区间（历史音频裁剪用）。
impl CaptureObserver for Mutex<HistoryDraft> {
    fn on_pcm(&self, chunk: &[f32]) {
        let mut h = self.lock().unwrap();
        h.audio_samples += chunk.len() as u64;
        if let Some(pcm) = &mut h.audio_pcm {
            pcm.extend_from_slice(chunk);
        }
    }

    fn on_partial(&self, _text: &str) {
        let mut g = self.lock().unwrap();
        if g.first_partial_ms.is_none() {
            g.first_partial_ms = Some(g.started.elapsed().as_millis() as u64);
        }
    }

    fn on_speech_frame(&self, start: usize, end: usize) {
        let mut g = self.lock().unwrap();
        g.vad_speech_seen = true;
        g.push_speech_range(start, end);
    }
}

impl Orchestrator {
    /// history 终态落账：取草稿快照 → 组记录 → 追加到 history.jsonl。
    /// - mode=off / 无草稿 → 不落盘；已结束的会话仍终结草稿，避免重新启用后补写旧记录；
    /// - error：落账但保留草稿（Error 态可重试，重试成功同 sessionId 再写一条 sent，
    ///   两条记录是刻意的「失败→重试成功」叙事）；
    /// - cancelled：草稿已被 error 落账过则跳过（Error 后的 Esc 是清理动作，不双记）；
    /// - sent / cancelled：落账并终结草稿；
    /// - IO 失败静默记日志，绝不炸主流程。
    pub(super) fn write_history(
        &self,
        outcome: crate::history::HistoryOutcome,
        error: Option<String>,
    ) {
        let draft = match self.inner.lock().unwrap().voice.history.clone() {
            Some(d) => d,
            None => return,
        };
        let cfg = self.settings.read().unwrap().history.clone();
        if cfg.mode == crate::history::HistoryMode::Off {
            if outcome != crate::history::HistoryOutcome::Error {
                // Off 只禁止持久化，不延长已取消/发送会话的草稿生命周期。
                // Error 仍保留草稿，允许同一次听写失败后重试。
                let mut inner = self.inner.lock().unwrap();
                if inner
                    .voice
                    .history
                    .as_ref()
                    .is_some_and(|current| Arc::ptr_eq(current, &draft))
                {
                    inner.voice.history = None;
                }
            }
            return;
        }
        let record = {
            let mut g = draft.lock().unwrap();
            if outcome == crate::history::HistoryOutcome::Cancelled {
                if g.reported {
                    // error 已落账过的草稿跳过（Error 后的 Esc 是清理动作，不双记 cancelled）
                    return;
                }
                if g.first_partial_ms.is_none()
                    && !g.vad_speech_seen
                    && g.final_text.trim().is_empty()
                {
                    // 取消时没有任何可验证的语音产出 → 不落账：
                    // solo 结束独奏时刚开出的空段、或没开口就取消，都不是
                    // 「放弃了这句话」，与空转录「无事发生」语义一致（P2 用户反馈
                    // 「结束独奏总有一条未说完成取消」）。草稿一并终结防残留。
                    // 判据说明：VAD 模式以 vad_speech_seen（silero 判定过语音）为主，
                    // STT partial 只作非 VAD 模式兜底——流式引擎对持续环境音/背景
                    // 噪音也会出 partial，单靠 partial 会把沉默段误判为「开过口」；
                    // 非流式引擎（FunASR-Nano/SenseVoice 等）不发 partial 且无 VAD，
                    // 取消时无法确认是否开口，同样按无产出处理不落账。
                    drop(g);
                    self.inner.lock().unwrap().voice.history = None;
                    return;
                }
            } else {
                g.reported = true;
            }
            crate::history::HistoryRecord {
                session_id: g.session_id.clone(),
                ts: crate::eval::utc_now_iso(),
                engine_id: g.engine_id.clone(),
                profile_id: g.profile_id.clone(),
                final_text: g.final_text.clone(),
                source_text: g.source_text.clone(),
                process_duration_ms: g.process_duration_ms,
                audio_ms: g.audio_samples * 1000 / crate::eval::SAMPLE_RATE as u64,
                first_partial_ms: g.first_partial_ms,
                finalize_latency_ms: g.finalize_latency_ms,
                outcome,
                error,
                audio_file: g.audio_file.clone(),
            }
        };
        let dir = self
            .history_dir
            .clone()
            .unwrap_or_else(crate::history::history_dir);
        if let Err(e) = crate::history::append_in(&dir, &record, &cfg) {
            crate::log::log(&format!("history 落账失败（忽略）: {e}"));
        } else {
            // 历史已更新：通知前端（历史记录页挂载期间据此即时刷新，无需切页）
            // 未落账的路径（off / 无草稿 / 无产出取消）不会走到这里。
            self.emitter.emit(
                "kotone://history-updated",
                serde_json::json!({ "outcome": outcome }),
            );
        }
        if outcome != crate::history::HistoryOutcome::Error {
            self.inner.lock().unwrap().voice.history = None;
        }
    }

    /// finalize 成功后把历史音频缓冲写入 history/audio。
    /// 关闭评测录档不影响该路径；取消/空转录不会调用，因此不留下无效音频。
    /// VAD 模式（one-shot/solo）下按语音区间裁剪：段首思考静音与段尾判停
    /// 尾音不再进保存的 wav（用户反馈「独奏历史音频很长、含大量沉默」）；
    /// 非 VAD 模式无语音区间标记，原样保存。
    pub(super) fn persist_history_audio(&self) {
        let cfg = self.settings.read().unwrap().history.clone();
        if cfg.mode == crate::history::HistoryMode::Off || !cfg.include_audio {
            return;
        }
        let draft = match self.inner.lock().unwrap().voice.history.clone() {
            Some(d) => d,
            None => return,
        };
        let (session_id, pcm, segments) = {
            let mut g = draft.lock().unwrap();
            (
                g.session_id.clone(),
                g.audio_pcm.take(),
                g.speech_segments.clone(),
            )
        };
        let Some(pcm) = pcm else {
            return;
        };
        // 按语音区间裁剪（VAD 模式有区间；非 VAD 模式 segments 空 → 原样）
        let trimmed = trim_speech_pcm(&pcm, &segments);
        if trimmed.is_empty() {
            crate::log::log("history 音频裁剪后为空（忽略保存）");
            return;
        }
        // 实际语音时长写回草稿：记录 audio_ms 与保存的 wav 一致
        draft.lock().unwrap().audio_samples = trimmed.len() as u64;
        let dir = self
            .history_dir
            .clone()
            .unwrap_or_else(crate::history::history_dir);
        let audio_file = crate::history::write_audio_in(&dir, &session_id, &trimmed);
        if audio_file.is_none() {
            crate::log::log("history 音频落盘失败（忽略）");
        }
        draft.lock().unwrap().audio_file = audio_file;
    }
}

/// 语音区间前导缓冲：silero 判定到 speech 时语音已实际开始（判定延迟约
/// 1-数帧），段首保留判定点之前的一段 PCM，避免字头/辅音被裁掉
/// （用户反馈「录制语音听不到第一个字」）。150ms @16kHz = 2400 采样。
const PRE_ROLL_SAMPLES: usize = 2400;
/// 语音区间尾随缓冲：保留尾音/气息，避免句尾被切。
const POST_ROLL_SAMPLES: usize = 2400;

/// 按语音区间裁剪历史音频 PCM（纯函数，可单测）。
/// 区间为半开 [start, end)，采样号相对 pcm 起点；每个区间向前/向后各扩展
/// PRE/POST_ROLL_SAMPLES 缓冲后拼接，扩展后重叠的段合并（段间停顿
/// < 300ms 自然连成一段），整体 clamp 到 pcm 长度。
/// segments 空 = 非 VAD 模式（无裁剪标记）→ 返回原样。
pub(crate) fn trim_speech_pcm(pcm: &[f32], segments: &[(usize, usize)]) -> Vec<f32> {
    if segments.is_empty() {
        return pcm.to_vec();
    }
    let n = pcm.len();
    let mut expanded: Vec<(usize, usize)> = Vec::with_capacity(segments.len());
    // 合并阈值 = PRE_ROLL + POST_ROLL = 4800 采样（300ms），与
    // SPEECH_GAP_SAMPLES（push 阶段的句间合并）数值恰好相等，但语义独立：
    // 这里是「缓冲扩展后重叠即合并」的防御性冗余，改其一无需同步另一处。
    for &(s, e) in segments {
        let ns = s.saturating_sub(PRE_ROLL_SAMPLES);
        let ne = (e + POST_ROLL_SAMPLES).min(n);
        if let Some(last) = expanded.last_mut() {
            if ns <= last.1 {
                // 与前段重叠/相接（缓冲覆盖了段间停顿）：合并
                last.1 = last.1.max(ne);
                continue;
            }
        }
        expanded.push((ns, ne));
    }
    let cap = expanded.iter().map(|&(s, e)| e.saturating_sub(s)).sum();
    let mut out = Vec::with_capacity(cap);
    for &(s, e) in &expanded {
        if s < e {
            out.extend_from_slice(&pcm[s..e]);
        }
    }
    out
}
