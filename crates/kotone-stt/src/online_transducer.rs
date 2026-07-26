//! 在线 transducer 流式引擎骨架（ADR-004）：sherpa.rs（zipformer 中英双语）与
//! xasr.rs（X-ASR 流式中英标点）共用的 recognizer/session 实现。
//!
//! 差异全部收敛到 [`OnlineTransducerSpec]：引擎 id / 展示名 / 语言 / 三个模型
//! 文件名 / 可选 bpe_vocab（Some 时设 modeling_unit="cjkchar+bpe"，X-ASR 类
//! 模型需要）。model_type 不显式设置——encoder.onnx 元数据自带
//! （zipformer2 / zipformer2r），C 侧自动探测。
//!
//! feature `engine-sherpa` 控制编译：开启 = 真实实现；关闭 = 占位注册
//! （恒 is_ready=false，默认构建零原生依赖）。
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

/// 在线 transducer 引擎的静态规格（实例差异全部在此）
pub struct OnlineTransducerSpec {
    pub engine_id: &'static str,
    pub display_name: &'static str,
    pub languages: &'static [&'static str],
    pub encoder_file: &'static str,
    pub decoder_file: &'static str,
    pub joiner_file: &'static str,
    /// Some = 模型为 cjkchar+bpe 建模单元（X-ASR），值是热词用的**文本** vocab
    /// 文件名（bpe.vocab；不存在时可从 bpe.model 现场导出）。仅配置了热词时
    /// 才会传给 C 侧，且传入前必经格式探测（P0：二进制 bpe.model 会让 C++ exit）
    pub bpe_vocab_file: Option<&'static str>,
    /// 模型未下载时的错误提示
    pub not_ready_hint: &'static str,
}

/// bpe_vocab 门控（P0 止血核心，纯函数便于无 feature 测试）：
/// C++ Validate 要求 modeling_unit=cjkchar+bpe 时 bpe_vocab 必须指向存在的
/// 文件，且创建 recognizer 时立即解析——格式不符直接 exit 进程。
/// 因此只有「文本 vocab 存在且通过格式探测」才返回 Some（resolve 内部会
/// 尝试从 bpe.model 现场导出兜底）；否则 None，调用方不得设置
/// modeling_unit/bpe_vocab（识别不受影响，仅热词降级）。
pub fn gated_bpe_vocab(
    spec: &OnlineTransducerSpec,
    dir: &std::path::Path,
) -> Option<std::path::PathBuf> {
    spec.bpe_vocab_file
        .and_then(|name| crate::model::resolve_bpe_vocab(dir, name))
}

#[cfg(feature = "engine-sherpa")]
pub mod imp {
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    use sherpa_onnx::{OnlineRecognizer, OnlineRecognizerConfig, OnlineStream};

    use kotone_core::stt::{SttEvent, SttSession, Transcript};

    use super::*;

    /// 默认推理线程数（engineOptions["threads"] 可覆盖）
    const DEFAULT_THREADS: u32 = 2;

    /// 流式收尾静音尾帧时长（毫秒）：sherpa-onnx 官方在线流式示例在
    /// input_finished 前补 ~0.8s 静音，触发模型吐出 lookahead（右上下文）
    /// 中的最后 token——不补则松手即 finalize 会丢句尾（P0 实锤：X-ASR
    /// 丢「吗」）。X-ASR 为 480ms chunk，800ms ≈ 1.7 个 chunk，覆盖其 lookahead
    pub const TAIL_PADDING_MS: usize = 800;

    /// 收尾 decode 轮数上限（防挂死）：每轮至少消化一个 ready chunk（几十毫秒
    /// 音频），256 轮 ≈ 8s+ 音频当量，远超尾帧所需；异常模型不得卡住发送流程
    pub const MAX_FINALIZE_DECODE_ROUNDS: u32 = 256;

    /// 静音尾帧（16kHz mono f32 全零）
    pub fn silence_tail() -> Vec<f32> {
        vec![0.0; TAIL_PADDING_MS * 16000 / 1000]
    }

    /// 在线 transducer 引擎：懒加载共享 recognizer（模型加载 ~百毫秒级，复用
    /// 避免每会话重建）。recognizer 首次创建后绑定当时模型；切换模型需重启进程生效。
    pub struct OnlineTransducerEngine {
        spec: &'static OnlineTransducerSpec,
        recognizer: Mutex<Option<Arc<OnlineRecognizer>>>,
    }

    impl OnlineTransducerEngine {
        pub fn from_spec(spec: &'static OnlineTransducerSpec) -> Self {
            Self {
                spec,
                recognizer: Mutex::new(None),
            }
        }

        /// 取共享 recognizer（不存在则按当前模型创建）
        fn recognizer(&self, cfg: &SessionConfig) -> Result<Arc<OnlineRecognizer>, String> {
            let mut guard = self.recognizer.lock().unwrap();
            if let Some(r) = guard.as_ref() {
                return Ok(r.clone());
            }
            let id = crate::model::active_model(self.spec.engine_id);
            if !crate::model::multi_model_ready(&id) {
                return Err(self.spec.not_ready_hint.into());
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
            config.model_config.transducer.encoder = Some(f(self.spec.encoder_file));
            config.model_config.transducer.decoder = Some(f(self.spec.decoder_file));
            config.model_config.transducer.joiner = Some(f(self.spec.joiner_file));
            config.model_config.tokens = Some(f("tokens.txt"));
            config.model_config.num_threads = threads;
            config.model_config.provider = Some("cpu".into());
            // cjkchar+bpe 建模单元（X-ASR）：C++ Validate 要求 bpe_vocab 指向
            // 存在文件且创建时立即解析（格式不符直接 exit，P0 根因）——
            // 只有探测合格才设置 modeling_unit+bpe_vocab，否则整体不设
            // （识别不受影响，仅热词降级）
            if self.spec.bpe_vocab_file.is_some() {
                match gated_bpe_vocab(self.spec, &dir) {
                    Some(vocab) => {
                        config.model_config.modeling_unit = Some("cjkchar+bpe".into());
                        config.model_config.bpe_vocab =
                            Some(vocab.to_string_lossy().into_owned());
                    }
                    None => {
                        kotone_core::log::log(&format!(
                            "{}: bpe.vocab 缺失或格式不符，未设置 cjkchar+bpe（识别不受影响，热词降级）",
                            self.spec.engine_id
                        ));
                    }
                }
            }
            // modified_beam_search：contextual biasing（per-stream 热词）只支持该
            // 解码器；greedy_search 遇到带热词的 stream 会 SHERPA_ONNX_EXIT
            // （online-transducer-decoder.h Decode 基类分支）
            config.decoding_method = Some("modified_beam_search".into());
            config.max_active_paths = 4;
            config.hotwords_score = 1.5;
            // push-to-talk 自己管理语句边界，不用内置端点检测
            config.enable_endpoint = false;

            let recognizer = OnlineRecognizer::create(&config).ok_or_else(|| {
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

    impl SttEngine for OnlineTransducerEngine {
        fn id(&self) -> &'static str {
            self.spec.engine_id
        }

        fn display_name(&self) -> &str {
            self.spec.display_name
        }

        fn capabilities(&self) -> EngineCapabilities {
            EngineCapabilities {
                streaming: true,
                hotwords: true, // create_stream_with_hotwords
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
        fn warmup(&self) -> Result<(), String> {
            // 与懒加载同一路径；engineOptions 的 threads 只在创建时生效，
            // 预热用默认线程数（SessionConfig::default 无 options）
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
            // per-stream 热词：每行一个短语
            let stream = if cfg.hotwords.is_empty() {
                recognizer.create_stream()
            } else {
                recognizer.create_stream_with_hotwords(&format_hotwords(&cfg.hotwords))
            };
            Ok(Box::new(OnlineTransducerSession {
                recognizer,
                stream: Some(stream),
                events,
                last_text: String::new(),
                cancelled: false,
            }))
        }
    }

    struct OnlineTransducerSession {
        recognizer: Arc<OnlineRecognizer>,
        stream: Option<OnlineStream>,
        events: mpsc::UnboundedSender<SttEvent>,
        /// 上次已外发的识别文本（变化检测：只发增量）
        last_text: String,
        cancelled: bool,
    }

    impl OnlineTransducerSession {
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

    impl SttSession for OnlineTransducerSession {
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
            // 静音尾帧：触发模型输出 lookahead 中残留的最后 token（松手丢字 P0）
            stream.accept_waveform(16000, &silence_tail());
            stream.input_finished();
            // 循环 decode 直到没有 ready chunk（不是只 decode 一次），
            // 轮数上限防挂死：异常模型不能卡住发送流程
            let mut rounds = 0u32;
            while self.recognizer.is_ready(&stream) && rounds < MAX_FINALIZE_DECODE_ROUNDS {
                self.recognizer.decode(&stream);
                rounds += 1;
            }
            if rounds >= MAX_FINALIZE_DECODE_ROUNDS {
                kotone_core::log::log(
                    "流式收尾 decode 达到轮数上限（模型异常？），按当前结果收尾",
                );
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
pub use imp::OnlineTransducerEngine;

/// 占位实现（feature 关闭时）：恒注册、恒未就绪
#[cfg(not(feature = "engine-sherpa"))]
pub struct OnlineTransducerEngine {
    spec: &'static OnlineTransducerSpec,
}

#[cfg(not(feature = "engine-sherpa"))]
impl OnlineTransducerEngine {
    pub fn from_spec(spec: &'static OnlineTransducerSpec) -> Self {
        Self { spec }
    }
}

#[cfg(not(feature = "engine-sherpa"))]
impl SttEngine for OnlineTransducerEngine {
    fn id(&self) -> &'static str {
        self.spec.engine_id
    }

    fn display_name(&self) -> &str {
        self.spec.display_name
    }

    fn capabilities(&self) -> EngineCapabilities {
        EngineCapabilities {
            streaming: true,
            hotwords: true,
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
