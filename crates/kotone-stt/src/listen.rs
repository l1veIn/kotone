//! 同传的耳朵用本体的语音识别（ADR 021、ADR 039 D2；共享资源评估第二节第 7 条）。
//!
//! 同传听的是某个程序的声音（进程回环），不是麦克风，所以它和呼叫、语音输入共用的是**识别**：
//! 同一个注册表、同一个模型目录，同一个模型、同样的设置在进程里只加载一份（`offline_sherpa` 按模型缓存）。
//! 两路可以同时识别；一次只能一个的是麦克风（`Orchestrator`），不是识别器。
//!
//! 用哪个识别只有一条规矩：**你选的识别听得懂外语（能力声明 `languages` 里有 `auto`，或者中英日韩都有）
//! 就用它，否则用本地 SenseVoice**（ADR 021 已确认的第一版）。现成的能力声明决定，不加设置。
//! 照的是 VRCT（麦克风和别人的声音共用一处选识别）和 Android 的系统识别服务（调用方带着语言问、缺的模型
//! 下载一次）。

use kotone_core::settings::Settings;
use kotone_core::stt::{canonical_stt_engine, EngineRegistry, SessionConfig, SttEngine};

use crate::model::{
    active_model_from, is_remote_model, multi_model_ready_from, DEEPSEEK_ASR_STT_ID,
    SHERPA_OFFLINE_ENGINE_ID,
};
use crate::transcriber::Utterance;

/// 本地的多语言模型（中英日韩粤，自动判语种）。
pub const SENSEVOICE_ID: &str = "sense-voice-zh-en-ja-ko-yue-2024-07-17";

/// 没有能听懂外语的识别时怎么办（原则 21：说下一步）。
pub const NEED_SENSEVOICE: &str = "同传要能听懂外语的识别：在「设置 → 模型 → 语音识别」下载 SenseVoice 多语言（约 230 MB），或者换成能自动判语种的在线识别（DeepSeek）";

/// 同传这一路用的识别。
#[derive(Debug, Clone, PartialEq)]
pub struct ListenModel {
    pub engine_id: String,
    pub model_id: String,
    /// 本地模型：停止本地语音识别时同传跟着停。
    pub local: bool,
    /// 就是玩家选的那个识别（不是另用的 SenseVoice）：本地的话，本地语音识别没启动时先启动（同呼叫按钮），
    /// 两边共用那一份。另用 SenseVoice 时（玩家用 X-ASR 之类）不启动本地语音识别、只加载 SenseVoice——
    /// 启动会把玩家那个模型也加载进来，没下载时还会失败。
    pub follows_selected: bool,
    /// 每一句的会话配置（不带连接的密钥，见 [`add_connection`]）。
    pub config: SessionConfig,
}

/// 能听懂外语：`auto`（自动判语种），或者中、英、日、韩都认得。
pub fn understands_foreign(languages: &[String]) -> bool {
    languages.iter().any(|l| l == "auto")
        || ["zh", "en", "ja", "ko"]
            .iter()
            .all(|need| languages.iter().any(|l| l == need))
}

/// 同传用哪个（引擎 id, 模型 id）：你选的听得懂外语就是它，否则本地 SenseVoice。
pub fn choose(
    selected_engine: &str,
    selected_model: &str,
    languages: &[String],
) -> (String, String) {
    if understands_foreign(languages) {
        (selected_engine.to_string(), selected_model.to_string())
    } else {
        (
            SHERPA_OFFLINE_ENGINE_ID.to_string(),
            SENSEVOICE_ID.to_string(),
        )
    }
}

/// 按现在的设置挑同传的识别，并检查它能不能用（本地模型下载了没有、在线识别配好了没有）。
pub fn listen_model(settings: &Settings, engines: &EngineRegistry) -> Result<ListenModel, String> {
    let selected_engine = canonical_stt_engine(&settings.stt_engine);
    let selected_model = active_model_from(settings, &settings.stt_engine);
    let languages = engines
        .get(selected_engine)
        .map(|e| e.capabilities().languages)
        .unwrap_or_default();
    let (engine_id, model_id) = choose(selected_engine, &selected_model, &languages);
    let follows_selected = engine_id == selected_engine && model_id == selected_model;
    let local = !is_remote_model(&model_id);
    let engine = engines
        .get(&engine_id)
        .ok_or_else(|| format!("这个版本没有 {engine_id} 这个识别"))?;
    if local {
        if !multi_model_ready_from(settings, &model_id) {
            return Err(NEED_SENSEVOICE.to_string());
        }
    } else if !engine.is_ready() {
        return Err(format!(
            "现在选的在线识别（{}）还没配好：去「设置 → 模型 → 语音识别」看看",
            engine.display_name()
        ));
    }
    let mut options = settings.session_options(&engine_id);
    if let Some(map) = options.as_object_mut() {
        map.insert("model".into(), serde_json::json!(model_id));
        // 队友说什么语言事先不知道：SenseVoice 自动判（玩家自己选的 SenseVoice 缺省也是 auto，于是共用一份）。
        map.insert("language".into(), serde_json::json!("auto"));
    }
    Ok(ListenModel {
        engine_id,
        model_id,
        local,
        follows_selected,
        config: SessionConfig {
            language: "auto".into(),
            hotwords: Vec::new(),
            options,
            ..Default::default()
        },
    })
}

/// 在线识别要的连接（地址、模型、密钥）填进会话配置。和 `kotone-core` 里给呼叫 / 语音输入填的那份
/// （`orchestrator/capture.rs` 的 `enrich_remote_connection`）同一个规矩：模型配置里选了连接就用它，
/// DeepSeek 语音识别没选时用她的 DeepSeek 连接。`secret` 按连接 id 取密钥（凭据库）。
pub fn add_connection(
    settings: &Settings,
    model: &mut ListenModel,
    secret: impl Fn(&str) -> Option<String>,
) {
    if model.local {
        return;
    }
    let configured = model
        .config
        .options
        .get("connectionId")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string);
    let connection_id = configured.or_else(|| {
        (model.engine_id == DEEPSEEK_ASR_STT_ID)
            .then(|| settings.deepseek_connection_id())
            .flatten()
    });
    let Some(connection) = connection_id
        .as_deref()
        .and_then(|id| settings.connections.iter().find(|c| c.id == id))
    else {
        return;
    };
    if let Some(opts) = model.config.options.as_object_mut() {
        opts.insert("baseUrl".into(), serde_json::json!(connection.base_url));
        opts.insert("remoteModel".into(), serde_json::json!(connection.model));
        if let Some(key) = secret(&connection.id) {
            opts.insert("apiKey".into(), serde_json::json!(key));
        }
    }
}

/// 识别一句（一句一个会话；本地模型的会话很便宜，共用识别器）。不算一句话（噪声、“嗯”）时 None。
/// 在线识别是阻塞的网络请求：在自己的线程里调，不要在异步运行时里调。
pub fn recognize(
    engine: &dyn SttEngine,
    config: &SessionConfig,
    pcm: &[f32],
) -> Result<Option<Utterance>, String> {
    let (events, _ignored) = tokio::sync::mpsc::unbounded_channel();
    let mut session = engine.start_session(config, events)?;
    session.push_audio(pcm)?;
    let transcript = session.finalize()?;
    Ok(Utterance::from_text(&transcript.text, pcm.len()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use kotone_core::stt::{EngineCapabilities, SttEvent, SttSession, Transcript};
    use tokio::sync::mpsc;

    fn langs(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    /// 只有能力声明和“好了没有”的假引擎；识别出的字是固定的。
    struct Fake {
        id: &'static str,
        languages: &'static [&'static str],
        ready: bool,
        says: &'static str,
    }

    struct FakeSession(&'static str, usize);

    impl SttSession for FakeSession {
        fn push_audio(&mut self, pcm: &[f32]) -> Result<(), String> {
            self.1 += pcm.len();
            Ok(())
        }
        fn finalize(self: Box<Self>) -> Result<Transcript, String> {
            Ok(Transcript {
                text: self.0.to_string(),
                latency_ms: 0,
            })
        }
        fn cancel(&mut self) {}
    }

    impl SttEngine for Fake {
        fn id(&self) -> &'static str {
            self.id
        }
        fn display_name(&self) -> &str {
            self.id
        }
        fn capabilities(&self) -> EngineCapabilities {
            EngineCapabilities {
                streaming: false,
                hotwords: false,
                gpu: false,
                offline: false,
                languages: langs(self.languages),
            }
        }
        fn is_ready(&self) -> bool {
            self.ready
        }
        fn start_session(
            &self,
            _cfg: &SessionConfig,
            _events: mpsc::UnboundedSender<SttEvent>,
        ) -> Result<Box<dyn SttSession>, String> {
            Ok(Box::new(FakeSession(self.says, 0)))
        }
    }

    fn registry(deepseek_ready: bool) -> EngineRegistry {
        let mut r = EngineRegistry::new();
        for (id, languages, ready) in [
            ("sherpa-streaming", &["zh", "en"][..], true),
            ("sherpa-offline", &["zh", "en", "ja", "ko", "yue"][..], true),
            ("deepseek-asr", &["auto"][..], deepseek_ready),
            ("remote-openai-compat", &["auto"][..], true),
            ("volcano-asr", &["zh"][..], true),
            ("iflytek-asr", &["zh"][..], true),
        ] {
            r.register(Box::new(Fake {
                id,
                languages,
                ready,
                says: "Enemy behind.",
            }));
        }
        r
    }

    /// 模型目录在临时目录里；`with_sensevoice` 时放好 SenseVoice 的文件（大小对就算下载了）。
    fn settings(engine: &str, model: &str, with_sensevoice: bool) -> (Settings, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        if with_sensevoice {
            let m = crate::model::SHERPA_MODELS
                .iter()
                .find(|m| m.id == SENSEVOICE_ID)
                .unwrap();
            let model_dir = dir.path().join(m.dir);
            std::fs::create_dir_all(&model_dir).unwrap();
            for f in m.files {
                let file = std::fs::File::create(model_dir.join(f.name)).unwrap();
                file.set_len(f.size_bytes).unwrap();
            }
        }
        let mut s = Settings::default();
        s.models.dir = dir.path().to_string_lossy().into_owned();
        s.stt_engine = engine.into();
        s.active_model_id = model.into();
        (s, dir)
    }

    #[test]
    fn foreign_means_auto_or_all_four() {
        assert!(understands_foreign(&langs(&["auto"])));
        assert!(understands_foreign(&langs(&[
            "zh", "en", "ja", "ko", "yue"
        ])));
        assert!(!understands_foreign(&langs(&["zh", "en"])));
        assert!(!understands_foreign(&langs(&["zh", "en", "ja"])));
        assert!(!understands_foreign(&langs(&["zh"])));
        assert!(!understands_foreign(&[]));
    }

    #[test]
    fn the_one_rule() {
        let x_asr = "x-asr-480ms-streaming-zh-en-punct-int8-2026-06-05";
        let table = [
            // 你选的听得懂外语 → 就用它
            (
                ("deepseek-asr", "deepseek-asr"),
                &["auto"][..],
                ("deepseek-asr", "deepseek-asr"),
            ),
            (
                ("remote-openai-compat", "openai-compat-stt"),
                &["auto"][..],
                ("remote-openai-compat", "openai-compat-stt"),
            ),
            (
                ("sherpa-offline", SENSEVOICE_ID),
                &["zh", "en", "ja", "ko", "yue"][..],
                ("sherpa-offline", SENSEVOICE_ID),
            ),
            // 听不懂 → 本地 SenseVoice
            (
                ("sherpa-streaming", x_asr),
                &["zh", "en"][..],
                ("sherpa-offline", SENSEVOICE_ID),
            ),
            (
                ("sherpa-offline", "funasr-nano-2025-12-30"),
                &["zh", "en", "ja"][..],
                ("sherpa-offline", SENSEVOICE_ID),
            ),
            (
                ("volcano-asr", "volcano-asr"),
                &["zh"][..],
                ("sherpa-offline", SENSEVOICE_ID),
            ),
            (
                ("iflytek-asr", "iflytek-asr"),
                &["zh"][..],
                ("sherpa-offline", SENSEVOICE_ID),
            ),
        ];
        for ((engine, model), languages, (want_engine, want_model)) in table {
            assert_eq!(
                choose(engine, model, &langs(languages)),
                (want_engine.to_string(), want_model.to_string()),
                "{engine}/{model}"
            );
        }
    }

    #[test]
    fn online_users_need_no_download() {
        let (s, _dir) = settings("deepseek-asr", "deepseek-asr", false);
        let m = listen_model(&s, &registry(true)).unwrap();
        assert_eq!(
            (m.engine_id.as_str(), m.local, m.follows_selected),
            ("deepseek-asr", false, true)
        );
        assert_eq!(m.config.language, "auto");
        // 在线识别还没配好：说清楚去哪儿配
        let e = listen_model(&s, &registry(false)).unwrap_err();
        assert!(e.contains("设置 → 模型 → 语音识别"), "{e}");
    }

    #[test]
    fn x_asr_users_get_sensevoice_or_are_told_to_download_it() {
        let x_asr = "x-asr-480ms-streaming-zh-en-punct-int8-2026-06-05";
        let (s, _dir) = settings("sherpa-streaming", x_asr, false);
        assert_eq!(
            listen_model(&s, &registry(true)).unwrap_err(),
            NEED_SENSEVOICE
        );
        let (s, _dir) = settings("sherpa-streaming", x_asr, true);
        let m = listen_model(&s, &registry(true)).unwrap();
        assert_eq!(
            (m.engine_id.as_str(), m.model_id.as_str(), m.local),
            ("sherpa-offline", SENSEVOICE_ID, true)
        );
        assert_eq!(m.config.options["model"], SENSEVOICE_ID);
        assert_eq!(m.config.options["language"], "auto");
        assert!(!m.follows_selected, "另用 SenseVoice：不去启动玩家的 X-ASR");
    }

    #[test]
    fn sensevoice_users_share_the_same_recognizer_settings() {
        let (mut s, _dir) = settings("sherpa-offline", SENSEVOICE_ID, true);
        s.engine_options = serde_json::json!({"sherpa-offline": {"threads": 3}});
        let m = listen_model(&s, &registry(true)).unwrap();
        assert_eq!(m.model_id, SENSEVOICE_ID);
        assert!(m.follows_selected);
        // 和本体启动时一样的线程数、同一个模型：offline_sherpa 的缓存键一样，只加载一份
        assert_eq!(m.config.options["threads"], 3);
        assert_eq!(m.config.options["model"], SENSEVOICE_ID);
    }

    #[test]
    fn the_deepseek_connection_is_filled_in() {
        let (mut s, _dir) = settings("deepseek-asr", "deepseek-asr", false);
        s.connections.push(kotone_core::connection::Connection {
            id: "ds".into(),
            display_name: "DeepSeek".into(),
            kind: Default::default(),
            provider: "deepseek".into(),
            base_url: "https://api.deepseek.com/v1".into(),
            model: "deepseek-chat".into(),
        });
        let mut m = listen_model(&s, &registry(true)).unwrap();
        add_connection(&s, &mut m, |id| (id == "ds").then(|| "sk-test".to_string()));
        assert_eq!(m.config.options["baseUrl"], "https://api.deepseek.com/v1");
        assert_eq!(m.config.options["apiKey"], "sk-test");
    }

    #[test]
    fn one_sentence_per_session() {
        let fake = Fake {
            id: "deepseek-asr",
            languages: &["auto"],
            ready: true,
            says: " Enemy behind the red house. ",
        };
        let u = recognize(&fake, &SessionConfig::default(), &vec![0.0; 32_000])
            .unwrap()
            .unwrap();
        assert_eq!(
            (u.text.as_str(), u.language, u.duration_ms),
            ("Enemy behind the red house.", "en", 2000)
        );
        let noise = Fake {
            says: "嗯", ..fake
        };
        assert_eq!(
            recognize(&noise, &SessionConfig::default(), &[0.0; 8000]).unwrap(),
            None
        );
    }
}
