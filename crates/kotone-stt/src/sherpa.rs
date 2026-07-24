//! 引擎 #2：sherpa-onnx 流式 Zipformer 中英双语（ADR-004）。
//!
//! feature `engine-sherpa` 控制编译：开启 = 真实实现（sherpa-onnx 官方 Rust
//! 绑定，静态链接）；关闭 = 占位注册（恒 is_ready=false，默认构建零原生依赖）。
//!
//! 流式语义：push_audio 边收边识别——accept_waveform → decode ready chunks →
//! 文本有变化即发 SttEvent::Partial；finalize 时 input_finished + 收尾 decode →
//! SttEvent::Final（latency_ms = 松手到最终文本的耗时）。
//!
//! 热词：`create_stream_with_hotwords`（per-stream，每行一个短语），profile
//! hotwords 直接注入，capabilities.hotwords = true。

use tokio::sync::mpsc;

use kotone_core::stt::{EngineCapabilities, SessionConfig, SttEngine};
#[cfg(not(feature = "engine-sherpa"))]
use kotone_core::stt::{SttEvent, SttSession};

pub const ENGINE_ID: &str = "sherpa-onnx-zipformer-zh";

#[cfg(feature = "engine-sherpa")]
mod imp {
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    use sherpa_onnx::{OnlineRecognizer, OnlineRecognizerConfig, OnlineStream};

    use kotone_core::stt::{SttEvent, SttSession, Transcript};

    use super::*;

    /// 默认推理线程数（engineOptions["threads"] 可覆盖）
    const DEFAULT_THREADS: u32 = 2;

    /// sherpa 引擎：懒加载共享 recognizer（模型加载 ~百毫秒级，复用避免每会话重建）。
    /// 单模型 MVP：recognizer 首次创建后绑定当时模型；切换模型需重启进程生效。
    pub struct SherpaEngine {
        recognizer: Mutex<Option<Arc<OnlineRecognizer>>>,
    }

    impl SherpaEngine {
        pub fn new() -> Self {
            Self {
                recognizer: Mutex::new(None),
            }
        }

        /// 取共享 recognizer（不存在则按当前模型创建）
        fn recognizer(&self, cfg: &SessionConfig) -> Result<Arc<OnlineRecognizer>, String> {
            let mut guard = self.recognizer.lock().unwrap();
            if let Some(r) = guard.as_ref() {
                return Ok(r.clone());
            }
            let id = crate::model::active_model(ENGINE_ID);
            if !crate::model::multi_model_ready(&id) {
                return Err(
                    "sherpa 模型未下载。请在设置页下载，或运行 kotone-cli download zipformer"
                        .into(),
                );
            }
            let dir = crate::model::multi_model_dir(&id).unwrap();

            let threads = cfg
                .options
                .get("threads")
                .and_then(|t| t.as_u64())
                .map(|t| t.clamp(1, 16) as i32)
                .unwrap_or(DEFAULT_THREADS as i32);

            let mut config = OnlineRecognizerConfig::default();
            let f = |name: &str| dir.join(name).to_string_lossy().into_owned();
            config.model_config.transducer.encoder =
                Some(f("encoder-epoch-99-avg-1.int8.onnx"));
            config.model_config.transducer.decoder = Some(f("decoder-epoch-99-avg-1.onnx"));
            config.model_config.transducer.joiner =
                Some(f("joiner-epoch-99-avg-1.int8.onnx"));
            config.model_config.tokens = Some(f("tokens.txt"));
            config.model_config.num_threads = threads;
            config.model_config.provider = Some("cpu".into());
            // modified_beam_search：contextual biasing（per-stream 热词）只支持该
            // 解码器；greedy_search 遇到带热词的 stream 会 SHERPA_ONNX_EXIT
            // （online-transducer-decoder.h Decode 基类分支）
            config.decoding_method = Some("modified_beam_search".into());
            config.max_active_paths = 4;
            config.hotwords_score = 1.5;
            // push-to-talk 自己管理语句边界，不用内置端点检测
            config.enable_endpoint = false;

            let recognizer = OnlineRecognizer::create(&config).ok_or_else(|| {
                format!("sherpa recognizer 创建失败（模型文件损坏？目录：{}）", dir.display())
            })?;
            let recognizer = Arc::new(recognizer);
            *guard = Some(recognizer.clone());
            Ok(recognizer)
        }
    }

    impl SttEngine for SherpaEngine {
        fn id(&self) -> &'static str {
            ENGINE_ID
        }

        fn display_name(&self) -> &str {
            "sherpa-onnx Zipformer 中文流式"
        }

        fn capabilities(&self) -> EngineCapabilities {
            EngineCapabilities {
                streaming: true,
                hotwords: true, // create_stream_with_hotwords
                gpu: false,
                offline: true,
                languages: vec!["zh".into(), "en".into()],
            }
        }

        fn is_ready(&self) -> bool {
            let id = crate::model::active_model(ENGINE_ID);
            crate::model::multi_model_ready(&id)
        }

        fn start_session(
            &self,
            cfg: &SessionConfig,
            events: mpsc::UnboundedSender<SttEvent>,
        ) -> Result<Box<dyn SttSession>, String> {
            let recognizer = self.recognizer(cfg)?;
            // per-stream 热词：每行一个短语
            let stream = if cfg.hotwords.is_empty() {
                recognizer.create_stream()
            } else {
                recognizer.create_stream_with_hotwords(&format_hotwords(&cfg.hotwords))
            };
            Ok(Box::new(SherpaSession {
                recognizer,
                stream: Some(stream),
                events,
                last_text: String::new(),
                cancelled: false,
            }))
        }
    }

    struct SherpaSession {
        recognizer: Arc<OnlineRecognizer>,
        stream: Option<OnlineStream>,
        events: mpsc::UnboundedSender<SttEvent>,
        /// 上次已外发的识别文本（变化检测：只发增量）
        last_text: String,
        cancelled: bool,
    }

    impl SherpaSession {
        /// decode 所有 ready chunk，返回当前识别文本
        fn decode_ready(&self) -> String {
            let stream = self.stream.as_ref().expect("stream taken");
            while self.recognizer.is_ready(stream) {
                self.recognizer.decode(stream);
            }
            self.recognizer
                .get_result(stream)
                .map(|r| r.text)
                .unwrap_or_default()
        }

        /// 文本有变化则发 Partial
        fn emit_if_changed(&mut self, text: String) {
            if !text.is_empty() && text != self.last_text {
                self.last_text = text.clone();
                let _ = self.events.send(SttEvent::Partial { text });
            }
        }
    }

    impl SttSession for SherpaSession {
        fn push_audio(&mut self, pcm: &[f32]) -> Result<(), String> {
            if self.cancelled {
                return Err("会话已取消".into());
            }
            let stream = self.stream.as_ref().expect("stream taken");
            stream.accept_waveform(16000, pcm);
            let text = self.decode_ready();
            self.emit_if_changed(text);
            Ok(())
        }

        fn finalize(mut self: Box<Self>) -> Result<Transcript, String> {
            if self.cancelled {
                return Err("会话已取消".into());
            }
            let started = Instant::now();
            let stream = self.stream.take().expect("stream taken");
            stream.input_finished();
            while self.recognizer.is_ready(&stream) {
                self.recognizer.decode(&stream);
            }
            let text = self
                .recognizer
                .get_result(&stream)
                .map(|r| r.text)
                .unwrap_or_default();
            let latency_ms = started.elapsed().as_millis() as u32;
            drop(stream); // 显式释放 C 资源

            let _ = self.events.send(SttEvent::Final {
                text: text.clone(),
                latency_ms,
            });
            Ok(Transcript { text, latency_ms })
        }

        fn cancel(&mut self) {
            self.cancelled = true;
            // 无子进程；stream 随 session drop 释放
        }
    }

    /// profile hotwords → sherpa 格式（每行一个短语）
    pub(crate) fn format_hotwords(hotwords: &[String]) -> String {
        hotwords.join("\n")
    }
}

#[cfg(feature = "engine-sherpa")]
pub use imp::SherpaEngine;

/// 占位实现（feature 关闭时）：恒注册、恒未就绪
#[cfg(not(feature = "engine-sherpa"))]
pub struct SherpaEngine;

#[cfg(not(feature = "engine-sherpa"))]
impl SherpaEngine {
    pub fn new() -> Self {
        Self
    }
}

#[cfg(not(feature = "engine-sherpa"))]
impl SttEngine for SherpaEngine {
    fn id(&self) -> &'static str {
        ENGINE_ID
    }

    fn display_name(&self) -> &str {
        "sherpa-onnx Zipformer 中文流式"
    }

    fn capabilities(&self) -> EngineCapabilities {
        EngineCapabilities {
            streaming: true,
            hotwords: true,
            gpu: false,
            offline: true,
            languages: vec!["zh".into(), "en".into()],
        }
    }

    fn is_ready(&self) -> bool {
        false
    }

    fn start_session(
        &self,
        _cfg: &SessionConfig,
        _events: mpsc::UnboundedSender<SttEvent>,
    ) -> Result<Box<dyn SttSession>, String> {
        Err(
            "sherpa 引擎未编译进当前构建（feature engine-sherpa 默认关闭）。\
             请用 --features engine-sherpa 构建"
                .into(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_metadata_consistent() {
        let e = SherpaEngine::new();
        assert_eq!(e.id(), "sherpa-onnx-zipformer-zh");
        let caps = e.capabilities();
        assert!(caps.streaming, "sherpa 必须是流式引擎");
        assert!(caps.offline);
    }

    #[cfg(feature = "engine-sherpa")]
    #[test]
    fn hotwords_format_one_phrase_per_line() {
        assert_eq!(
            imp::format_hotwords(&["闪现".into(), "大龙".into(), "gank".into()]),
            "闪现\n大龙\ngank"
        );
        assert_eq!(imp::format_hotwords(&[]), "");
    }

    /// partial 变化检测逻辑（不依赖原生库，用同构状态机验证）
    #[test]
    fn partial_emit_only_on_change() {
        struct ChangeDetector {
            last: String,
            emitted: Vec<String>,
        }
        impl ChangeDetector {
            fn emit_if_changed(&mut self, text: &str) {
                if !text.is_empty() && text != self.last {
                    self.last = text.to_string();
                    self.emitted.push(text.to_string());
                }
            }
        }
        let mut d = ChangeDetector {
            last: String::new(),
            emitted: Vec::new(),
        };
        d.emit_if_changed(""); // 空文本不发
        d.emit_if_changed("对面");
        d.emit_if_changed("对面"); // 重复不发
        d.emit_if_changed("对面打野");
        d.emit_if_changed("对面打野在下路");
        assert_eq!(d.emitted, vec!["对面", "对面打野", "对面打野在下路"]);
    }

    #[test]
    fn ready_state_matches_model_files() {
        let e = SherpaEngine::new();
        #[cfg(feature = "engine-sherpa")]
        {
            let id = crate::model::active_model(ENGINE_ID);
            assert_eq!(e.is_ready(), crate::model::multi_model_ready(&id));
        }
        #[cfg(not(feature = "engine-sherpa"))]
        {
            assert!(!e.is_ready(), "占位引擎恒未就绪");
            assert!(e
                .start_session(&SessionConfig::default(), mpsc::unbounded_channel().0)
                .is_err());
        }
    }
}
