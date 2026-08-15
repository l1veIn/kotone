//! 非流式 sherpa-onnx 引擎骨架（ADR-004）：sensevoice.rs / funasr_nano.rs
//! 共用的 recognizer/session 实现。
//!
//! 差异全部收敛到 [`OfflineSpec] + 一个 `configure` 函数（填充
//! OfflineRecognizerConfig 的模型家族字段）。骨架负责：recognizer 懒加载、
//! threads/provider、warmup/unload、PCM 缓冲会话（push_audio 只缓冲 →
//! finalize 一次性转写）。
//!
//! feature `engine-sherpa` 控制编译：开启 = 真实实现；关闭 = 占位注册
//! （恒 is_ready=false，默认构建零原生依赖）。
//!
//! 共享 recognizer 首次创建时绑定当时的活动模型与 language——之后切换模型/
//! 语言/热词需「重启生效」（与各 sherpa 引擎同一约定）。

use tokio::sync::mpsc;

use kotone_core::stt::{EngineCapabilities, SessionConfig, SttEngine};
#[cfg(not(feature = "engine-sherpa"))]
use kotone_core::stt::{SttEvent, SttSession};

/// 非流式引擎的静态规格（实例差异之一；模型配置由 configure 函数补齐）
pub struct OfflineSpec {
    pub engine_id: &'static str,
    pub display_name: &'static str,
    pub languages: &'static [&'static str],
    /// capabilities.hotwords（funasr-nano 支持模型级热词注入）
    pub hotwords: bool,
    /// 模型未下载时的错误提示
    pub not_ready_hint: &'static str,
}

#[cfg(feature = "engine-sherpa")]
pub mod imp {
    use std::path::Path;
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    use sherpa_onnx::{OfflineRecognizer, OfflineRecognizerConfig};

    use kotone_core::stt::{SttEvent, SttSession, Transcript};

    use super::*;

    /// 默认推理线程数（engineOptions["threads"] 可覆盖）
    const DEFAULT_THREADS: u32 = 2;

    /// 模型家族配置函数：给定会话配置与模型目录，填充 OfflineRecognizerConfig
    /// 的模型家族字段（sense_voice / funasr_nano 等）
    pub type ConfigureFn = fn(&SessionConfig, &Path, &mut OfflineRecognizerConfig);

    /// 非流式引擎：懒加载共享 recognizer（模型加载百毫秒级，复用避免每会话重建）。
    pub struct OfflineEngine {
        spec: &'static OfflineSpec,
        configure: ConfigureFn,
        recognizer: Mutex<Option<Arc<OfflineRecognizer>>>,
    }

    impl OfflineEngine {
        pub fn new(spec: &'static OfflineSpec, configure: ConfigureFn) -> Self {
            Self {
                spec,
                configure,
                recognizer: Mutex::new(None),
            }
        }

        /// 取共享 recognizer（不存在则按当前活动模型 + 本次会话语言创建）
        fn recognizer(&self, cfg: &SessionConfig) -> Result<Arc<OfflineRecognizer>, String> {
            let mut guard = self.recognizer.lock().unwrap();
            if let Some(r) = guard.as_ref() {
                return Ok(r.clone());
            }
            let id = crate::model::model_id_from_cfg(cfg, self.spec.engine_id);
            if !crate::model::multi_model_ready(&id) {
                return Err(self.spec.not_ready_hint.into());
            }
            match crate::model::recipe_of(&id) {
                Some(
                    crate::model::ModelRecipe::SenseVoice | crate::model::ModelRecipe::FunasrNano,
                ) => {}
                other => {
                    return Err(format!(
                        "模型 {id} 的配方 {other:?} 不能走非流式 sherpa"
                    ));
                }
            }
            let dir = crate::model::multi_model_dir(&id).unwrap();

            let threads = cfg
                .options
                .get("threads")
                .and_then(|t| t.as_u64())
                .map(|t| t.clamp(1, 16) as i32)
                .unwrap_or(DEFAULT_THREADS as i32);

            let mut config = OfflineRecognizerConfig::default();
            config.model_config.num_threads = threads;
            config.model_config.provider = Some("cpu".into());
            (self.configure)(cfg, &dir, &mut config);

            let recognizer = OfflineRecognizer::create(&config).ok_or_else(|| {
                format!(
                    "{} recognizer 创建失败（模型文件损坏？目录：{}）",
                    self.spec.display_name,
                    dir.display()
                )
            })?;
            let recognizer = Arc::new(recognizer);
            *guard = Some(recognizer.clone());
            Ok(recognizer)
        }
    }

    impl SttEngine for OfflineEngine {
        fn id(&self) -> &'static str {
            self.spec.engine_id
        }

        fn display_name(&self) -> &str {
            self.spec.display_name
        }

        fn capabilities(&self) -> EngineCapabilities {
            EngineCapabilities {
                streaming: false,
                hotwords: self.spec.hotwords,
                gpu: false,
                offline: true,
                languages: self.spec.languages.iter().map(|s| s.to_string()).collect(),
            }
        }

        fn is_ready(&self) -> bool {
            let id = crate::model::active_model(self.spec.engine_id);
            crate::model::multi_model_ready(&id)
        }

        /// 预热：显式创建共享 recognizer（模型入内存）；随后 start_session 直接复用
        fn warmup(&self, cfg: &SessionConfig) -> Result<(), String> {
            self.recognizer(cfg).map(|_| ())
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
            Ok(Box::new(OfflineSession {
                recognizer,
                pcm: Vec::new(),
                events,
                cancelled: false,
            }))
        }
    }

    /// 非流式会话：缓冲全部 PCM，finalize 一次性转写（进程内，无子进程）
    struct OfflineSession {
        recognizer: Arc<OfflineRecognizer>,
        pcm: Vec<f32>,
        events: mpsc::UnboundedSender<SttEvent>,
        cancelled: bool,
    }

    impl SttSession for OfflineSession {
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

    /// profile hotwords → 空格连接（funasr-nano 模型级热词字段的
    /// upstream 惯例格式；实际分隔语义待真机验证）
    pub(crate) fn join_hotwords(hotwords: &[String]) -> Option<String> {
        if hotwords.is_empty() {
            None
        } else {
            Some(hotwords.join(" "))
        }
    }
}

#[cfg(feature = "engine-sherpa")]
pub use imp::OfflineEngine;

/// 占位实现（feature 关闭时）：恒注册、恒未就绪
#[cfg(not(feature = "engine-sherpa"))]
pub struct OfflineEngine {
    spec: &'static OfflineSpec,
}

#[cfg(not(feature = "engine-sherpa"))]
impl OfflineEngine {
    pub fn new(spec: &'static OfflineSpec) -> Self {
        Self { spec }
    }
}

#[cfg(not(feature = "engine-sherpa"))]
impl SttEngine for OfflineEngine {
    fn id(&self) -> &'static str {
        self.spec.engine_id
    }

    fn display_name(&self) -> &str {
        self.spec.display_name
    }

    fn capabilities(&self) -> EngineCapabilities {
        EngineCapabilities {
            streaming: false,
            hotwords: self.spec.hotwords,
            gpu: false,
            offline: true,
            languages: self.spec.languages.iter().map(|s| s.to_string()).collect(),
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
        Err(format!(
            "{} 引擎未编译进当前构建（feature engine-sherpa 默认关闭）。\
             请用 --features engine-sherpa 构建",
            self.spec.display_name
        ))
    }
}
