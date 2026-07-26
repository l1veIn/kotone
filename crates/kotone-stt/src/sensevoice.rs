//! 引擎 #3：sherpa-onnx SenseVoice 多语言（非流式，人工评测第三选手）。
//!
//! feature `engine-sherpa` 控制编译（与 zipformer 共享同一份 sherpa-onnx 原生
//! 依赖）：开启 = 真实实现；关闭 = 占位注册（恒 is_ready=false）。
//!
//! 非流式会话语义：push_audio 只缓冲 PCM（16kHz mono f32）→ finalize 时
//! 一次性 OfflineRecognizer 转写（参考 whisper sidecar 的会话语义，但在进程内，
//! 无子进程开销）。识别质量口碑优于 zipformer，代价是边说边不出字。
//!
//! SenseVoice 不支持热词注入（sherpa-onnx 的 per-stream hotwords 仅流式
//! transducer 系支持），capabilities.hotwords = false。
//!
//! 共享 recognizer 懒加载（模型 ~230MB int8，加载百毫秒级，复用避免每会话重建）：
//! 首次创建时绑定当时的活动模型与 language——之后切换模型/语言需「重启生效」
//! （与 zipformer 引擎同一约定）。

use tokio::sync::mpsc;

use kotone_core::stt::{EngineCapabilities, SessionConfig, SttEngine};
#[cfg(not(feature = "engine-sherpa"))]
use kotone_core::stt::{SttEvent, SttSession};

pub const ENGINE_ID: &str = "sherpa-onnx-sensevoice";

#[cfg(feature = "engine-sherpa")]
mod imp {
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    use sherpa_onnx::{
        OfflineRecognizer, OfflineRecognizerConfig, OfflineSenseVoiceModelConfig,
    };

    use kotone_core::stt::{SttEvent, SttSession, Transcript};

    use super::*;

    /// 默认推理线程数（engineOptions["threads"] 可覆盖）
    const DEFAULT_THREADS: u32 = 2;

    /// SessionConfig.language → SenseVoice language（支持 zh/en/ja/ko/yue/auto；
    /// 其它值兜底 auto 自动判别）
    pub(crate) fn map_language(lang: &str) -> String {
        match lang {
            "zh" | "en" | "ja" | "ko" | "yue" => lang.to_string(),
            _ => "auto".to_string(),
        }
    }

    /// SenseVoice 引擎：懒加载共享 recognizer（对齐 sherpa.rs 的模式）。
    pub struct SenseVoiceEngine {
        recognizer: Mutex<Option<Arc<OfflineRecognizer>>>,
    }

    impl SenseVoiceEngine {
        pub fn new() -> Self {
            Self {
                recognizer: Mutex::new(None),
            }
        }

        /// 取共享 recognizer（不存在则按当前活动模型 + 本次会话语言创建）
        fn recognizer(&self, cfg: &SessionConfig) -> Result<Arc<OfflineRecognizer>, String> {
            let mut guard = self.recognizer.lock().unwrap();
            if let Some(r) = guard.as_ref() {
                return Ok(r.clone());
            }
            let id = crate::model::active_model(ENGINE_ID);
            if !crate::model::multi_model_ready(&id) {
                return Err(
                    "SenseVoice 模型未下载。请在设置页下载，或运行 kotone-cli download sense-voice"
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

            let f = |name: &str| dir.join(name).to_string_lossy().into_owned();
            let mut config = OfflineRecognizerConfig::default();
            config.model_config.sense_voice = OfflineSenseVoiceModelConfig {
                model: Some(f("model.int8.onnx")),
                language: Some(map_language(&cfg.language)),
                use_itn: true, // 逆文本正则（数字/标点落形），聊天场景更自然
            };
            config.model_config.tokens = Some(f("tokens.txt"));
            config.model_config.num_threads = threads;
            config.model_config.provider = Some("cpu".into());

            let recognizer = OfflineRecognizer::create(&config).ok_or_else(|| {
                format!(
                    "SenseVoice recognizer 创建失败（模型文件损坏？目录：{}）",
                    dir.display()
                )
            })?;
            let recognizer = Arc::new(recognizer);
            *guard = Some(recognizer.clone());
            Ok(recognizer)
        }
    }

    impl SttEngine for SenseVoiceEngine {
        fn id(&self) -> &'static str {
            ENGINE_ID
        }

        fn display_name(&self) -> &str {
            "sherpa-onnx SenseVoice 多语言"
        }

        fn capabilities(&self) -> EngineCapabilities {
            EngineCapabilities {
                streaming: false,
                hotwords: false, // SenseVoice 不支持热词注入
                gpu: false,
                offline: true,
                languages: vec![
                    "zh".into(),
                    "en".into(),
                    "ja".into(),
                    "ko".into(),
                    "yue".into(),
                ],
            }
        }

        fn is_ready(&self) -> bool {
            let id = crate::model::active_model(ENGINE_ID);
            crate::model::multi_model_ready(&id)
        }

        /// 预热：显式创建共享 recognizer（模型入内存）；随后 start_session 直接复用
        fn warmup(&self) -> Result<(), String> {
            self.recognizer(&SessionConfig::default()).map(|_| ())
        }

        /// 卸载：释放共享 recognizer（重新「启动」或下次会话时重建）
        fn unload(&self) {
            *self.recognizer.lock().unwrap() = None;
        }

        fn start_session(
            &self,
            cfg: &SessionConfig,
            events: mpsc::UnboundedSender<SttEvent>,
        ) -> Result<Box<dyn SttSession>, String> {
            let recognizer = self.recognizer(cfg)?;
            Ok(Box::new(SenseVoiceSession {
                recognizer,
                pcm: Vec::new(),
                events,
                cancelled: false,
            }))
        }
    }

    /// 非流式会话：缓冲全部 PCM，finalize 一次性转写（进程内，无子进程）
    struct SenseVoiceSession {
        recognizer: Arc<OfflineRecognizer>,
        pcm: Vec<f32>,
        events: mpsc::UnboundedSender<SttEvent>,
        cancelled: bool,
    }

    impl SttSession for SenseVoiceSession {
        fn push_audio(&mut self, pcm: &[f32]) -> Result<(), String> {
            if self.cancelled {
                return Err("会话已取消".into());
            }
            self.pcm.extend_from_slice(pcm);
            Ok(())
        }

        fn finalize(self: Box<Self>) -> Result<Transcript, String> {
            if self.cancelled {
                return Err("会话已取消".into());
            }
            if self.pcm.is_empty() {
                return Err("没有音频数据".into());
            }
            let started = Instant::now();
            let stream = self.recognizer.create_stream();
            stream.accept_waveform(16000, &self.pcm);
            self.recognizer.decode(&stream);
            let text = stream.get_result().map(|r| r.text).unwrap_or_default();
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
            self.pcm.clear();
        }
    }
}

#[cfg(feature = "engine-sherpa")]
pub use imp::SenseVoiceEngine;

/// 占位实现（feature 关闭时）：恒注册、恒未就绪
#[cfg(not(feature = "engine-sherpa"))]
pub struct SenseVoiceEngine;

#[cfg(not(feature = "engine-sherpa"))]
impl SenseVoiceEngine {
    pub fn new() -> Self {
        Self
    }
}

#[cfg(not(feature = "engine-sherpa"))]
impl SttEngine for SenseVoiceEngine {
    fn id(&self) -> &'static str {
        ENGINE_ID
    }

    fn display_name(&self) -> &str {
        "sherpa-onnx SenseVoice 多语言"
    }

    fn capabilities(&self) -> EngineCapabilities {
        EngineCapabilities {
            streaming: false,
            hotwords: false,
            gpu: false,
            offline: true,
            languages: vec![
                "zh".into(),
                "en".into(),
                "ja".into(),
                "ko".into(),
                "yue".into(),
            ],
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
            "SenseVoice 引擎未编译进当前构建（feature engine-sherpa 默认关闭）。\
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
        let e = SenseVoiceEngine::new();
        assert_eq!(e.id(), "sherpa-onnx-sensevoice");
        let caps = e.capabilities();
        assert!(!caps.streaming, "SenseVoice 必须是非流式引擎");
        assert!(!caps.hotwords, "SenseVoice 不支持热词");
        assert!(caps.offline);
        for lang in ["zh", "en", "ja", "ko", "yue"] {
            assert!(caps.languages.iter().any(|l| l == lang), "缺语言 {lang}");
        }
    }

    #[cfg(feature = "engine-sherpa")]
    #[test]
    fn language_mapping() {
        assert_eq!(imp::map_language("zh"), "zh");
        assert_eq!(imp::map_language("yue"), "yue");
        assert_eq!(imp::map_language("auto"), "auto");
        assert_eq!(imp::map_language(""), "auto");
        assert_eq!(imp::map_language("fr"), "auto");
    }

    #[test]
    fn ready_state_matches_model_files() {
        let e = SenseVoiceEngine::new();
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
