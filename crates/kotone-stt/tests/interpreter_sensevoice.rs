//! 同传（ADR 021）和本体共用一份 SenseVoice（共享资源评估第二节第 7 条；ADR 039 实施记录“同传用本体的识别”）。
//!
//! - `interpreter_reuses_the_loaded_sensevoice`：本体启动本地语音识别（`SherpaOfflineEngine::warmup`，`runtime.rs`）
//!   以后，同传（`listen::listen_model` + `listen::recognize`，`voice_listen.rs`）不再加载一份。2026-11-25 修之前
//!   同传用 `Transcriber::new` 自己建，+276 MB（复现 (b)）；修完是回归测试。
//! - `sensevoice_auto_is_as_good_as_zh_for_chinese`：SenseVoice 的识别语言是建识别器时定的；缺省改成 `auto`
//!   （和界面一致）的依据：中、英、日、韩游戏短句上 `auto` 对中文不变差、日语韩语好得多。
//!
//! 都要真模型，默认忽略：`KOTONE_SENSEVOICE_DIR`（SenseVoice 模型目录）、`KOTONE_SPEECH_WAV`（16 位单声道英语
//! wav）、`KOTONE_LANG_SAMPLES`（`scripts/manual-e2e/sensevoice_lang_samples.py` 生成的录音），
//! `cargo test -p kotone-stt --features engine-sherpa,vad-silero --test interpreter_sensevoice -- --ignored --nocapture --test-threads=1`
#![cfg(all(windows, feature = "engine-sherpa", feature = "vad-silero"))]

use std::path::{Path, PathBuf};

use kotone_core::stt::{SessionConfig, SttEngine};

const SENSEVOICE_ID: &str = "sense-voice-zh-en-ja-ko-yue-2024-07-17";
const MB: f64 = 1024.0 * 1024.0;

#[repr(C)]
#[derive(Default)]
struct ProcessMemoryCountersEx {
    cb: u32,
    page_fault_count: u32,
    peak_working_set_size: usize,
    working_set_size: usize,
    quota_peak_paged_pool_usage: usize,
    quota_paged_pool_usage: usize,
    quota_peak_non_paged_pool_usage: usize,
    quota_non_paged_pool_usage: usize,
    pagefile_usage: usize,
    peak_pagefile_usage: usize,
    private_usage: usize,
}

#[link(name = "kernel32")]
extern "system" {
    fn GetCurrentProcess() -> isize;
    fn K32GetProcessMemoryInfo(
        process: isize,
        counters: *mut ProcessMemoryCountersEx,
        cb: u32,
    ) -> i32;
}

/// （私有字节，工作集），MB。
fn memory() -> (f64, f64) {
    let mut c = ProcessMemoryCountersEx {
        cb: std::mem::size_of::<ProcessMemoryCountersEx>() as u32,
        ..Default::default()
    };
    // SAFETY: 传进去的是本进程的伪句柄和一块大小写对的结构体。
    let ok = unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut c, c.cb) };
    assert!(ok != 0, "K32GetProcessMemoryInfo failed");
    (c.private_usage as f64 / MB, c.working_set_size as f64 / MB)
}

fn sensevoice_dir() -> PathBuf {
    PathBuf::from(std::env::var("KOTONE_SENSEVOICE_DIR").expect("KOTONE_SENSEVOICE_DIR"))
}

/// 隔离的 KOTONE_HOME：模型目录指向 SenseVoice 所在的目录，选中的识别是 SenseVoice（本地、非流式）。
fn isolated_home(models: &Path) -> tempfile::TempDir {
    let home = tempfile::tempdir().unwrap();
    let config = serde_json::json!({
        "models": {"dir": models.to_string_lossy()},
        "sttEngine": "sherpa-offline",
        "activeModelId": SENSEVOICE_ID,
    });
    std::fs::write(home.path().join("config.json"), config.to_string()).unwrap();
    std::env::set_var("KOTONE_HOME", home.path());
    home
}

/// 16 位 wav → 16 kHz 单声道 f32（测试用的线性插值重采样）。
fn read_wav_16k(path: &Path) -> Vec<f32> {
    let wav = std::fs::read(path).unwrap();
    let channels = u16::from_le_bytes([wav[22], wav[23]]) as usize;
    let rate = u32::from_le_bytes(wav[24..28].try_into().unwrap());
    let data = wav.windows(4).position(|w| w == b"data").unwrap() + 8;
    let pcm: Vec<f32> = wav[data..]
        .chunks_exact(2 * channels)
        .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0)
        .collect();
    let step = rate as f64 / 16_000.0;
    (0..(pcm.len() as f64 / step) as usize)
        .map(|i| {
            let x = i as f64 * step;
            let (j, f) = (x as usize, x.fract() as f32);
            let a = pcm[j];
            let b = *pcm.get(j + 1).unwrap_or(&a);
            a + (b - a) * f
        })
        .collect()
}

/// 隔离配置里换掉 SenseVoice 的“识别语言”（玩家在「设置 → 模型」里选的；空 = 没选）。
fn set_player_language(models: &Path, language: &str) {
    let home = PathBuf::from(std::env::var("KOTONE_HOME").unwrap());
    let mut config = serde_json::json!({
        "models": {"dir": models.to_string_lossy()},
        "sttEngine": "sherpa-offline",
        "activeModelId": SENSEVOICE_ID,
    });
    if !language.is_empty() {
        config["modelConfigs"] = serde_json::json!({ SENSEVOICE_ID: {"language": language} });
    }
    std::fs::write(home.join("config.json"), config.to_string()).unwrap();
}

/// 本体启动本地语音识别时的会话配置（`runtime.rs` 的 `session_config_from`）。
fn core_config(settings: &kotone_core::settings::Settings) -> SessionConfig {
    SessionConfig {
        language: settings.language.clone(),
        options: settings.session_options("sherpa-offline"),
        ..Default::default()
    }
}

/// 本体已经加载了 SenseVoice，同传开始听（`listen::listen_model` + `listen::recognize`）时不再加载一份：
/// 修之前 `Transcriber::new` 另建一个，+276 MB（复现 (b)）；修完 < 50 MB，注册表里只有一份。
/// 玩家在模型配置里专门选了 `zh` 时，同传（要 `auto`）才另加载一份（缓存按模型 + 语言分格）。
#[test]
#[ignore = "needs the SenseVoice model and an English wav (KOTONE_SENSEVOICE_DIR, KOTONE_SPEECH_WAV)"]
fn interpreter_reuses_the_loaded_sensevoice() {
    use kotone_stt::listen;

    let dir = sensevoice_dir();
    let models = dir.parent().unwrap();
    let _home = isolated_home(models);
    assert_eq!(
        kotone_stt::model::multi_model_dir(SENSEVOICE_ID).as_deref(),
        Some(dir.as_path()),
        "隔离配置的模型目录没生效"
    );
    let english = read_wav_16k(Path::new(
        &std::env::var("KOTONE_SPEECH_WAV").expect("KOTONE_SPEECH_WAV"),
    ));
    let mut registry = kotone_core::stt::EngineRegistry::new();
    kotone_stt::register_builtin(&mut registry);
    // 看份数用：loaded() 在具体类型上，另建一个和注册表里一样的引擎。
    let offline = kotone_stt::sherpa_runtime::SherpaOfflineEngine::new;

    let before = memory();
    let settings = kotone_core::settings::load();
    let engine = registry.get("sherpa-offline").unwrap();
    // 本体：玩家选了 SenseVoice，“启动本地语音识别”= warmup（runtime.rs start_inner）。
    engine
        .warmup(&core_config(&settings))
        .expect("本体加载 SenseVoice");
    let core = memory();
    // 同传：voice_listen.rs → listen_model + 每句 recognize。
    let model = listen::listen_model(&settings, &registry).expect("同传的识别");
    assert_eq!(
        (model.engine_id.as_str(), model.model_id.as_str()),
        ("sherpa-offline", SENSEVOICE_ID)
    );
    let heard = listen::recognize(engine, &model.config, &english)
        .expect("同传识别一句")
        .expect("认出一句话");
    let both = memory();
    let core_mb = core.0 - before.0;
    let interpreter_mb = both.0 - core.0;
    println!(
        "private: before {:.0} MB, core SenseVoice +{core_mb:.0} MB, interpreter +{interpreter_mb:.0} MB \
         (working set {:.0} → {:.0} → {:.0} MB); heard {heard:?}",
        before.0, before.1, core.1, both.1
    );
    assert!(
        core_mb > 100.0,
        "本体那份应该是一整个模型（{core_mb:.0} MB），测量不对"
    );
    assert!(
        interpreter_mb < 50.0,
        "同传又加载了一份 SenseVoice：+{interpreter_mb:.0} MB（本体那份 +{core_mb:.0} MB）"
    );
    let text = heard.text.to_lowercase();
    assert!(
        text.contains("house") || text.contains("enem"),
        "{}",
        heard.text
    );
    assert_eq!(heard.language, "en");

    // 停止本地语音识别 = 卸掉（runtime.rs stop）：两边用的是同一份，一起没了。
    engine.unload();
    let unloaded = memory();
    println!("after unload: private {:.0} MB", unloaded.0);
    assert!(
        unloaded.0 - before.0 < 50.0,
        "卸掉以后还有 {:.0} MB",
        unloaded.0 - before.0
    );

    // 玩家专门把 SenseVoice 定成中文：同传（auto）另加载一份，玩家自己的不受影响。
    set_player_language(models, "zh");
    let settings = kotone_core::settings::load();
    let probe = offline();
    probe.warmup(&core_config(&settings)).unwrap();
    let model = listen::listen_model(&settings, &registry).unwrap();
    listen::recognize(&probe, &model.config, &english).unwrap();
    assert_eq!(probe.loaded(), 2, "玩家定了 zh、同传要 auto：两份");
    set_player_language(models, "");
    let settings = kotone_core::settings::load();
    let probe = offline();
    probe.warmup(&core_config(&settings)).unwrap();
    let model = listen::listen_model(&settings, &registry).unwrap();
    listen::recognize(&probe, &model.config, &english).unwrap();
    assert_eq!(probe.loaded(), 1, "没定（缺省 auto）：一份");
}

/// 去掉标点和空白、转小写，按字比较。
fn normalize(text: &str) -> Vec<char> {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// 字错误率：编辑距离 / 参考文字的字数。
fn cer(reference: &str, hypothesis: &str) -> f64 {
    let (r, h) = (normalize(reference), normalize(hypothesis));
    let mut row: Vec<usize> = (0..=h.len()).collect();
    for (i, rc) in r.iter().enumerate() {
        let mut prev = row[0];
        row[0] = i + 1;
        for (j, hc) in h.iter().enumerate() {
            let cur = row[j + 1];
            row[j + 1] = (prev + usize::from(rc != hc)).min(row[j] + 1).min(cur + 1);
            prev = cur;
        }
    }
    row[h.len()] as f64 / r.len().max(1) as f64
}

/// 识别语言 `zh`（本体缺省：`settings.language`）和 `auto`（同传）在中、英、日、韩游戏短句上的字错误率。
/// 决定 SenseVoice 能不能统一用 `auto`、共用一份（ADR 039 实施记录“同传用本体的识别”）：
/// `auto` 对中文不能明显变差（平均字错误率最多高 0.05），日语、韩语在 `auto` 下要认得出来。
///
/// 录音：`scripts/manual-e2e/sensevoice_lang_samples.py` 生成到 `KOTONE_LANG_SAMPLES`
/// （`<语言>-<n>.wav` + 同名 `.txt` 参考文字）。
#[test]
#[ignore = "needs the SenseVoice model and the language samples (KOTONE_SENSEVOICE_DIR, KOTONE_LANG_SAMPLES)"]
fn sensevoice_auto_is_as_good_as_zh_for_chinese() {
    use sherpa_onnx::{OfflineRecognizer, OfflineRecognizerConfig, OfflineSenseVoiceModelConfig};

    let dir = sensevoice_dir();
    let samples = PathBuf::from(std::env::var("KOTONE_LANG_SAMPLES").expect("KOTONE_LANG_SAMPLES"));
    // 和 sensevoice::configure 一样的配置，只换语言。
    let recognizer = |language: &str| {
        let mut config = OfflineRecognizerConfig::default();
        config.model_config.sense_voice = OfflineSenseVoiceModelConfig {
            model: Some(dir.join("model.int8.onnx").to_string_lossy().into_owned()),
            language: Some(language.into()),
            use_itn: true,
        };
        config.model_config.tokens = Some(dir.join("tokens.txt").to_string_lossy().into_owned());
        config.model_config.num_threads = 2;
        config.model_config.provider = Some("cpu".into());
        OfflineRecognizer::create(&config).expect("SenseVoice")
    };
    let decode = |r: &OfflineRecognizer, pcm: &[f32]| {
        let stream = r.create_stream();
        stream.accept_waveform(16_000, pcm);
        r.decode(&stream);
        stream.get_result().map(|x| x.text).unwrap_or_default()
    };
    let configs = [("zh", recognizer("zh")), ("auto", recognizer("auto"))];
    let mut files: Vec<PathBuf> = std::fs::read_dir(&samples)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "wav"))
        .collect();
    files.sort();
    assert!(!files.is_empty(), "{} 里没有录音", samples.display());
    // (语言, 识别语言) → 每句的字错误率
    let mut table: std::collections::BTreeMap<(String, &str), Vec<f64>> = Default::default();
    for wav in &files {
        let lang = wav
            .file_stem()
            .unwrap()
            .to_string_lossy()
            .split('-')
            .next()
            .unwrap()
            .to_string();
        let reference = std::fs::read_to_string(wav.with_extension("txt")).unwrap();
        let pcm = read_wav_16k(wav);
        for (name, r) in &configs {
            let text = decode(r, &pcm);
            let e = cer(&reference, &text);
            println!(
                "{:<10} {name:<4} cer {e:.2}  {reference} → {text}",
                wav.file_name().unwrap().to_string_lossy()
            );
            table.entry((lang.clone(), name)).or_default().push(e);
        }
    }
    let mean = |lang: &str, name: &str| {
        let v = &table[&(lang.to_string(), name)];
        v.iter().sum::<f64>() / v.len() as f64
    };
    for lang in ["zh", "en", "ja", "ko"] {
        if table.contains_key(&(lang.to_string(), "zh")) {
            println!(
                "{lang}: mean cer  zh {:.3}  auto {:.3}",
                mean(lang, "zh"),
                mean(lang, "auto")
            );
        }
    }
    assert!(
        mean("zh", "auto") <= mean("zh", "zh") + 0.05,
        "auto 让中文变差了：{:.3} 对 {:.3}",
        mean("zh", "auto"),
        mean("zh", "zh")
    );
    for lang in ["ja", "ko"] {
        assert!(
            mean(lang, "auto") < 0.5,
            "auto 认不出 {lang}：平均字错误率 {:.3}",
            mean(lang, "auto")
        );
    }
}
