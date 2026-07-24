//! Mock 流式引擎（id: `mock-stream`）：全链路联调用。
//! 不接任何模型，按喂入的音频量（每 0.5s @16kHz）发一条假 partial，
//! finalize 返回固定最终文本 + 实测延迟。

use std::time::Instant;

use tokio::sync::mpsc;

use kotone_core::stt::{EngineCapabilities, SessionConfig, SttEngine, SttEvent, SttSession, Transcript};

/// 每积累多少 16kHz 采样发一条 partial（8000 = 0.5s）
const PARTIAL_EVERY_SAMPLES: usize = 8000;

/// 假 partial 剧本（与 docs/development.md §5.4 eval 示例同语料）
const PARTIALS: [&str; 3] = ["对面", "对面打野", "对面打野在下"];

/// 假最终文本
const FINAL_TEXT: &str = "对面打野在下路";

pub struct MockStreamEngine;

impl SttEngine for MockStreamEngine {
    fn id(&self) -> &'static str {
        "mock-stream"
    }

    fn display_name(&self) -> &str {
        "Mock 流式引擎（联调用）"
    }

    fn capabilities(&self) -> EngineCapabilities {
        EngineCapabilities {
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
        _cfg: &SessionConfig,
        events: mpsc::UnboundedSender<SttEvent>,
    ) -> Result<Box<dyn SttSession>, String> {
        Ok(Box::new(MockSession {
            events,
            started: Instant::now(),
            buffered: 0,
            next_partial: 0,
            cancelled: false,
        }))
    }
}

struct MockSession {
    events: mpsc::UnboundedSender<SttEvent>,
    started: Instant,
    /// 已累计的采样数
    buffered: usize,
    /// 下一条待发 partial 的下标
    next_partial: usize,
    cancelled: bool,
}

impl SttSession for MockSession {
    fn push_audio(&mut self, pcm: &[f32]) -> Result<(), String> {
        if self.cancelled {
            return Err("会话已取消".into());
        }
        self.buffered += pcm.len();
        // 按音频量推进 partial 剧本
        while self.next_partial < PARTIALS.len()
            && self.buffered >= (self.next_partial + 1) * PARTIAL_EVERY_SAMPLES
        {
            let text = PARTIALS[self.next_partial].to_string();
            // 通道已关闭（如 orchestrator 提前退出）不算错误
            let _ = self.events.send(SttEvent::Partial { text });
            self.next_partial += 1;
        }
        Ok(())
    }

    fn finalize(self: Box<Self>) -> Result<Transcript, String> {
        if self.cancelled {
            return Err("会话已取消".into());
        }
        let latency_ms = self.started.elapsed().as_millis() as u32;
        let _ = self.events.send(SttEvent::Final {
            text: FINAL_TEXT.into(),
            latency_ms,
        });
        Ok(Transcript {
            text: FINAL_TEXT.into(),
            latency_ms,
        })
    }

    fn cancel(&mut self) {
        self.cancelled = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn start() -> (Box<dyn SttSession>, mpsc::UnboundedReceiver<SttEvent>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let engine = MockStreamEngine;
        let session = engine
            .start_session(&SessionConfig::default(), tx)
            .unwrap();
        (session, rx)
    }

    fn silence(seconds: f32) -> Vec<f32> {
        vec![0.0f32; (16000.0 * seconds) as usize]
    }

    #[test]
    fn engine_metadata() {
        let e = MockStreamEngine;
        assert_eq!(e.id(), "mock-stream");
        assert!(e.is_ready());
        assert!(e.capabilities().streaming);
    }

    #[test]
    fn partials_emitted_by_audio_volume() {
        let (mut s, mut rx) = start();

        // 0.4s：不足 0.5s，无 partial
        s.push_audio(&silence(0.4)).unwrap();
        assert!(rx.try_recv().is_err());

        // 再 0.2s（累计 0.6s）：第一条 partial
        s.push_audio(&silence(0.2)).unwrap();
        match rx.try_recv() {
            Ok(SttEvent::Partial { text }) => assert_eq!(text, "对面"),
            other => panic!("expected partial, got {other:?}"),
        }

        // 一次性推 1.5s（累计 2.1s）：连发第二、三条
        s.push_audio(&silence(1.5)).unwrap();
        match rx.try_recv() {
            Ok(SttEvent::Partial { text }) => assert_eq!(text, "对面打野"),
            other => panic!("expected partial, got {other:?}"),
        }
        match rx.try_recv() {
            Ok(SttEvent::Partial { text }) => assert_eq!(text, "对面打野在下"),
            other => panic!("expected partial, got {other:?}"),
        }
        assert!(rx.try_recv().is_err(), "剧本已用完，不应再有 partial");

        let t = s.finalize().unwrap();
        assert_eq!(t.text, FINAL_TEXT);
    }

    #[test]
    fn finalize_emits_final_event() {
        let (mut s, mut rx) = start();
        s.push_audio(&silence(0.1)).unwrap();
        let t = s.finalize().unwrap();
        assert_eq!(t.text, FINAL_TEXT);
        match rx.try_recv() {
            Ok(SttEvent::Final { text, latency_ms }) => {
                assert_eq!(text, FINAL_TEXT);
                assert_eq!(latency_ms, t.latency_ms);
            }
            other => panic!("expected final event, got {other:?}"),
        }
    }

    #[test]
    fn cancel_blocks_push_and_finalize() {
        let (mut s, _rx) = start();
        s.push_audio(&silence(0.1)).unwrap();
        s.cancel();
        assert!(s.push_audio(&silence(0.1)).is_err());
        assert!(s.finalize().is_err());
    }

    #[test]
    fn finalize_without_audio_still_works() {
        let (s, _rx) = start();
        let t = s.finalize().unwrap();
        assert_eq!(t.text, FINAL_TEXT);
    }
}
