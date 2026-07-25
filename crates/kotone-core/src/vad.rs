//! 语音活动检测（VAD）端口与静音判停纯逻辑（ADR-006 决策点 B3 / ADR-007）。
//!
//! 分层：
//! - `Vad` port：帧级语音判定（实现：kotone-stt 的 silero-vad ONNX，feature-gated）；
//! - `FrameSplitter`：任意长度 PCM chunk → 30ms 定长帧（采集端推 50ms chunk，
//!   silero 内部窗口 32ms，30ms 帧喂入由其自行缓冲）；
//! - `SilenceStopTracker`：判停状态机（纯逻辑可单测）——见过语音后连续静音
//!   ≥ 阈值判停；会话最短时长保护防误触（开始即安静不判停）。
//!
//! orchestrator 的 PCM 泵在 Listening 态把帧并行喂给 VAD 与 STT session；
//! 判停后走与 `end()` 相同的路径，热键强制结束恒在兜底（VAD 失效可手动结束）。

use std::sync::Arc;

/// VAD 帧长：30ms @16kHz（480 samples）
pub const VAD_FRAME_SAMPLES: usize = 480;
/// 帧时长（ms）：判停状态机的时间步进
pub const VAD_FRAME_MS: u32 = 30;

/// 默认静音判停阈值（ms）：config.json `vadSilenceMs` 的缺省值
pub const DEFAULT_SILENCE_MS: u32 = 700;
/// 会话最短时长保护（ms）：短于此不判停（防误触；固定常量，不开放配置）
pub const MIN_SESSION_MS: u32 = 500;
/// `vadSilenceMs` 允许的配置范围（orchestrator 使用时 clamp）
pub const SILENCE_MS_RANGE: std::ops::RangeInclusive<u32> = 200..=5_000;

/// 帧级语音判定端口：16kHz mono f32，每次喂一帧（`VAD_FRAME_SAMPLES`）。
///
/// 实现负责模型推理与内部缓冲；本端口只暴露「该帧之后是否处于语音中」。
/// 会话级状态（判停阈值/最短保护）在 `SilenceStopTracker`，不在实现内。
pub trait Vad: Send {
    /// 喂一帧，返回当前是否处于语音中
    fn push_frame(&mut self, frame: &[f32]) -> Result<bool, String>;
    /// 重置检测状态（新会话）
    fn reset(&mut self);
}

/// VAD 实例工厂（每会话一个实例，避免跨会话状态泄漏）；
/// 模型未下载/未编译时返回清晰错误
pub type VadFactory = Arc<dyn Fn() -> Result<Box<dyn Vad>, String> + Send + Sync>;

/// 任意长度 PCM → 定长帧切分（残留不足一帧的尾巴留到下次喂入时补齐）
#[derive(Default)]
pub struct FrameSplitter {
    buf: std::collections::VecDeque<f32>,
}

impl FrameSplitter {
    pub fn new() -> Self {
        Self::default()
    }

    /// 喂入一段 PCM，返回切出的完整帧（每帧 `VAD_FRAME_SAMPLES`）
    pub fn push(&mut self, samples: &[f32]) -> Vec<Vec<f32>> {
        self.buf.extend(samples.iter().copied());
        let mut out = Vec::new();
        while self.buf.len() >= VAD_FRAME_SAMPLES {
            out.push(self.buf.drain(..VAD_FRAME_SAMPLES).collect());
        }
        out
    }
}

/// 判停决策
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopDecision {
    Continue,
    Stop,
}

/// 静音判停状态机（B3 核心纯逻辑）：
/// - 必须先见过语音（`seen_speech`）才允许判停——开始即安静不算「说完」；
/// - 见过语音后连续静音 ≥ `silence_ms` 判停；
/// - 会话时长 < `min_session_ms` 时不判停（最短保护，防误触）。
pub struct SilenceStopTracker {
    silence_ms: u32,
    min_session_ms: u32,
    seen_speech: bool,
    silence_run_ms: u32,
    elapsed_ms: u32,
}

impl SilenceStopTracker {
    pub fn new(silence_ms: u32, min_session_ms: u32) -> Self {
        Self {
            silence_ms,
            min_session_ms,
            seen_speech: false,
            silence_run_ms: 0,
            elapsed_ms: 0,
        }
    }

    /// 喂一帧的语音判定，返回是否应判停
    pub fn push(&mut self, speech: bool) -> StopDecision {
        self.elapsed_ms += VAD_FRAME_MS;
        if speech {
            self.seen_speech = true;
            self.silence_run_ms = 0;
        } else if self.seen_speech {
            self.silence_run_ms += VAD_FRAME_MS;
        }
        if self.seen_speech
            && self.silence_run_ms >= self.silence_ms
            && self.elapsed_ms >= self.min_session_ms
        {
            StopDecision::Stop
        } else {
            StopDecision::Continue
        }
    }

    pub fn seen_speech(&self) -> bool {
        self.seen_speech
    }

    pub fn elapsed_ms(&self) -> u32 {
        self.elapsed_ms
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------- FrameSplitter ----------

    #[test]
    fn splitter_cuts_exact_frames() {
        let mut s = FrameSplitter::new();
        // 50ms chunk（800 samples）→ 1 帧（480）+ 残留 320
        let frames = s.push(&vec![0.1; 800]);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].len(), VAD_FRAME_SAMPLES);
        // 再喂 800：残留 320 + 800 = 1120 → 2 帧（960）+ 残留 160
        let frames = s.push(&vec![0.1; 800]);
        assert_eq!(frames.len(), 2);
        // 再喂 800：160 + 800 = 960 → 2 帧，无残留
        let frames = s.push(&vec![0.1; 800]);
        assert_eq!(frames.len(), 2);
        let frames = s.push(&vec![0.1; 1]);
        assert!(frames.is_empty(), "不足一帧不出帧");
    }

    // ---------- SilenceStopTracker ----------

    /// 连续喂 n 帧相同判定，返回最后一次的决策
    fn push_n(t: &mut SilenceStopTracker, speech: bool, n: usize) -> StopDecision {
        let mut d = StopDecision::Continue;
        for _ in 0..n {
            d = t.push(speech);
        }
        d
    }

    #[test]
    fn silence_without_speech_never_stops() {
        // 开始即安静：未见过语音，永不判停
        let mut t = SilenceStopTracker::new(700, 500);
        for _ in 0..100 {
            assert_eq!(t.push(false), StopDecision::Continue);
        }
        assert!(!t.seen_speech());
    }

    #[test]
    fn stops_after_speech_then_threshold_silence() {
        let mut t = SilenceStopTracker::new(700, 500);
        // 1s 语音（34 帧 ≈ 1020ms，超过最短保护 500ms）
        assert_eq!(push_n(&mut t, true, 34), StopDecision::Continue);
        // 静音 690ms（23 帧）：未到阈值不判停
        assert_eq!(push_n(&mut t, false, 23), StopDecision::Continue);
        // 第 24 帧静音（累计 720ms ≥ 700ms）→ 判停
        assert_eq!(t.push(false), StopDecision::Stop);
    }

    #[test]
    fn min_session_protection_delays_stop() {
        // 阈值 210ms，最短保护 500ms：语音 300ms + 静音 210ms 时阈值已到，
        // 但会话才 510ms... 构造更极限的：语音 240ms（8 帧）+ 静音 210ms（7 帧），
        // 会话 450ms < 500ms → 不判停；再静音一帧（480ms）仍 < 500ms → 不判停；
        // 再一帧（510ms）→ 判停
        let mut t = SilenceStopTracker::new(210, 500);
        push_n(&mut t, true, 8); // 240ms 语音
        assert_eq!(push_n(&mut t, false, 7), StopDecision::Continue); // 450ms，静音 210 已到阈值
        assert_eq!(t.push(false), StopDecision::Continue); // 480ms 仍不足
        assert_eq!(t.push(false), StopDecision::Stop); // 510ms ≥ 500ms → 判停
    }

    #[test]
    fn speech_reset_silence_run() {
        // 静音中间插一帧语音 → 连续静音重新累计
        let mut t = SilenceStopTracker::new(210, 100);
        push_n(&mut t, true, 10);
        assert_eq!(push_n(&mut t, false, 6), StopDecision::Continue); // 180ms 静音
        assert_eq!(t.push(true), StopDecision::Continue); // 插一帧语音
        assert_eq!(push_n(&mut t, false, 6), StopDecision::Continue); // 重新累计 180ms
        assert_eq!(t.push(false), StopDecision::Stop); // 210ms → 判停
    }
}
