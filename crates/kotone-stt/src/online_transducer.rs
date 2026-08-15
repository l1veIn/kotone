//! 在线 transducer 流式引擎骨架（ADR-004）：sherpa.rs（zipformer 中英双语）与
//! xasr.rs（X-ASR 流式中英标点）共用的 recognizer/session 实现。
//!
//! 差异全部收敛到 [`OnlineTransducerSpec]：引擎 id / 展示名 / 语言 / 三个模型
//! 文件名 / 热词建模单元 / 可选 bpe_vocab。model_type 不显式设置——encoder.onnx 元数据自带
//! （zipformer2 / zipformer2r），C 侧自动探测。
//!
//! feature `engine-sherpa` 控制编译：开启 = 真实实现；关闭 = 占位注册
//! （恒 is_ready=false，默认构建零原生依赖）。
//!
//! 流式语义：push_audio 边收边识别——accept_waveform → decode ready chunks →
//! 文本有变化即发 SttEvent::Partial；finalize 时 input_finished + 收尾 decode →
//! SttEvent::Final（latency_ms = 松手到最终文本的耗时）。
//!
//! 热词：`create_stream_with_hotwords`（per-stream，短语之间用 `/` 分隔），profile
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
    /// sherpa 热词编码使用的建模单元，例如 bpe / cjkchar / cjkchar+bpe。
    pub modeling_unit: &'static str,
    /// Some = 热词编码需要 SentencePiece **文本** vocab，值为文件名
    /// （bpe.vocab；不存在时可从 bpe.model 现场导出）。传入前必经格式探测
    /// （P0：把二进制 bpe.model 当 vocab 传入会让 C++ exit）。
    pub bpe_vocab_file: Option<&'static str>,
    /// 模型未下载时的错误提示
    pub not_ready_hint: &'static str,
}

/// bpe_vocab 门控（P0 止血核心，纯函数便于无 feature 测试）：
/// C++ Validate 要求 modeling_unit=bpe/cjkchar+bpe 时 bpe_vocab 必须指向存在的
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
            let id = crate::model::model_id_from_cfg(cfg, self.spec.engine_id);
            if !crate::model::multi_model_ready(&id) {
                return Err(self.spec.not_ready_hint.into());
            }
            if let Some(recipe) = crate::model::recipe_of(&id) {
                if recipe != crate::model::ModelRecipe::ZipformerTransducer {
                    return Err(format!(
                        "模型 {id} 的配方 {recipe:?} 不能走流式 transducer"
                    ));
                }
            }
            let dir = crate::model::multi_model_dir(&id).unwrap();
            crate::model::validate_models_dir_path(&dir)?;

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
            // BPE 类建模单元：C++ Validate 要求 bpe_vocab 指向
            // 存在文件且创建时立即解析（格式不符直接 exit，P0 根因）——
            // 只有探测合格才设置 modeling_unit+bpe_vocab，否则整体不设
            // （识别不受影响，仅热词降级）
            if self.spec.bpe_vocab_file.is_some() {
                match gated_bpe_vocab(self.spec, &dir) {
                    Some(vocab) => {
                        config.model_config.modeling_unit = Some(self.spec.modeling_unit.into());
                        config.model_config.bpe_vocab = Some(vocab.to_string_lossy().into_owned());
                    }
                    None => {
                        kotone_core::log::log(&format!(
                            "{}: bpe.vocab 缺失或格式不符，未设置 {}（识别不受影响，热词降级）",
                            self.spec.engine_id, self.spec.modeling_unit
                        ));
                    }
                }
            }
            // modified_beam_search：contextual biasing（per-stream 热词）只支持该
            // 解码器；greedy_search 遇到带热词的 stream 会 SHERPA_ONNX_EXIT
            // （online-transducer-decoder.h Decode 基类分支）
            config.decoding_method = Some("modified_beam_search".into());
            // 4/8 条路径会在热词加分前过早剪掉罕见同音字候选（实测
            // 「悠米」只剩「优米」）；16 条可保留候选供 context graph 重排。
            config.max_active_paths = 16;
            // 热词加分来自 SessionConfig（orchestrator 由 settings.hotwordsScore 填充；
            // 越高热词越易命中，也越容易把背景噪声识别成热词）。默认 3.5（core/settings.rs）
            config.hotwords_score = cfg.hotwords_score;
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
        fn warmup(&self, cfg: &SessionConfig) -> Result<(), String> {
            // 与懒加载同一路径；threads 与 hotwords_score 都是
            // recognizer 级配置，必须由运行时启动快照传入。
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
            // per-stream 动态热词接口与 hotwords_file 的格式不同：
            // 多个短语必须以 `/` 分隔，不能用换行。换行会让整份 profile
            // 被当成一个超长短语，表面创建成功、实际几乎不可能命中。
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
                kotone_core::log::log("流式收尾 decode 达到轮数上限（模型异常？），按当前结果收尾");
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

    fn is_cjk(ch: char) -> bool {
        matches!(
            ch,
            '\u{3400}'..='\u{4DBF}'
                | '\u{4E00}'..='\u{9FFF}'
                | '\u{F900}'..='\u{FAFF}'
                | '\u{20000}'..='\u{2FA1F}'
        )
    }

    /// X-ASR 的 SentencePiece 词表以「带词首标记的单个汉字」为单位。
    /// 因此中文连续文本要先拆成独立词元；ASCII 连续串保留为一个词元：
    /// `悠米` → `悠 米`，`红buff` → `红 buff`。
    fn format_hotword_phrase(hotword: &str) -> String {
        let mut units = Vec::new();
        let mut non_cjk = String::new();
        let flush_non_cjk = |units: &mut Vec<String>, buf: &mut String| {
            if !buf.is_empty() {
                units.push(std::mem::take(buf));
            }
        };

        for ch in hotword.trim().chars() {
            if is_cjk(ch) {
                flush_non_cjk(&mut units, &mut non_cjk);
                units.push(ch.to_string());
            } else if ch.is_whitespace() || ch == '/' {
                flush_non_cjk(&mut units, &mut non_cjk);
            } else {
                non_cjk.push(ch);
            }
        }
        flush_non_cjk(&mut units, &mut non_cjk);
        units.join(" ")
    }

    /// profile hotwords → sherpa per-stream 格式（短语之间以 `/` 分隔）。
    ///
    /// 注意：`hotwords_file` 才是每行一个短语；`create_stream_with_hotwords`
    /// 的字符串参数使用 `/`，两者不能混用。中文还必须先按字拆分，
    /// 否则 X-ASR 的 BPE 会把第二个字编码成 `<unk>` 并丢弃整条热词。
    pub(crate) fn format_hotwords(hotwords: &[String]) -> String {
        hotwords
            .iter()
            .map(|word| format_hotword_phrase(word))
            .filter(|word| !word.is_empty())
            .collect::<Vec<_>>()
            .join("/")
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

#[cfg(all(test, feature = "engine-sherpa"))]
mod tests {
    use super::imp;

    /// 热词端到端链路的引擎段：SessionConfig.hotwords → sherpa per-stream
    /// 热词参数（短语以 `/` 分隔，create_stream_with_hotwords 的输入格式）
    #[test]
    fn hotwords_format_for_sherpa_stream() {
        let words = vec![
            "打野".to_string(),
            "gank".to_string(),
            "盲僧 R 闪".to_string(),
        ];
        assert_eq!(imp::format_hotwords(&words), "打 野/gank/盲 僧 R 闪");
        assert_eq!(
            imp::format_hotwords(&["悠米".into(), "红buff".into(), " ADC ".into()]),
            "悠 米/红 buff/ADC"
        );
        assert_eq!(
            imp::format_hotwords(&["海克斯/强化".into()]),
            "海 克 斯 强 化"
        );
        assert_eq!(imp::format_hotwords(&[]), "");
    }
}
