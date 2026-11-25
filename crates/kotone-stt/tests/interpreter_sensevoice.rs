//! 同传（ADR 021）另起一份 SenseVoice 的复现（共享资源评估第二节第 7 条）。
//!
//! 走的就是产品里的两条路：本体的语音识别 = `SherpaOfflineEngine::warmup`（`runtime.rs` 启动时调），
//! 同传 = `Transcriber::new(<模型目录>/sherpa-onnx-sense-voice-…)`（`voice_listen.rs` 开始听时调）。
//!
//! - `interpreter_reuses_the_loaded_sensevoice`：本体已经加载了 SenseVoice，同传开始听时不该再加载一份
//!   （ADR 038 D3 第 7 条“识别模型也只该下一份”、ADR 039 D2“加载一份”）。修之前失败（复现 (b)）。
//! - `sensevoice_language_is_fixed_per_recognizer`：设计探针——SenseVoice 的识别语言是建识别器时定的
//!   （`sensevoice::configure`），本体缺省是 `zh`，同传要 `auto`；看一个识别器能不能按句换语言
//!   （sherpa-onnx 1.13.5 的 `OfflineStream::set_option`）。只打印、只断言 `auto` 认得出英语。
//!
//! 都要真模型，默认忽略：
//! `KOTONE_SENSEVOICE_DIR`（SenseVoice 模型目录）、`KOTONE_SPEECH_WAV`（16 位单声道英语 wav），
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

#[test]
#[ignore = "needs the SenseVoice model (KOTONE_SENSEVOICE_DIR)"]
fn interpreter_reuses_the_loaded_sensevoice() {
    let dir = sensevoice_dir();
    let _home = isolated_home(dir.parent().unwrap());
    assert_eq!(
        kotone_stt::model::multi_model_dir(SENSEVOICE_ID).as_deref(),
        Some(dir.as_path()),
        "隔离配置的模型目录没生效"
    );

    let before = memory();
    // 本体：玩家选了 SenseVoice，“启动本地语音识别”= warmup（runtime.rs start_inner）。
    let engine = kotone_stt::sherpa_runtime::SherpaOfflineEngine::new();
    let cfg = SessionConfig {
        options: serde_json::json!({"model": SENSEVOICE_ID}),
        ..Default::default()
    };
    engine.warmup(&cfg).expect("本体加载 SenseVoice");
    let core = memory();
    // 同传：voice_listen.rs start_listener → Transcriber::new(sensevoice_dir)。
    let transcriber = kotone_stt::transcriber::Transcriber::new(&dir).expect("同传加载 SenseVoice");
    let both = memory();
    let core_mb = core.0 - before.0;
    let interpreter_mb = both.0 - core.0;
    println!(
        "private: before {:.0} MB, core SenseVoice +{core_mb:.0} MB, interpreter +{interpreter_mb:.0} MB \
         (working set {:.0} → {:.0} → {:.0} MB)",
        before.0, before.1, core.1, both.1
    );
    // 卸载本体那份（runtime.rs stop 只卸本体的），同传那份还在。
    engine.unload();
    let after_stop = memory();
    println!(
        "after the core unloads: private {:.0} MB (still {:.0} MB above the start while the interpreter listens)",
        after_stop.0,
        after_stop.0 - before.0
    );
    drop(transcriber);
    assert!(
        core_mb > 100.0,
        "本体那份应该是一整个模型（{core_mb:.0} MB），测量不对"
    );
    assert!(
        interpreter_mb < 50.0,
        "同传又加载了一份 SenseVoice：+{interpreter_mb:.0} MB（本体那份 +{core_mb:.0} MB）"
    );
}

#[test]
#[ignore = "needs the SenseVoice model and an English wav (KOTONE_SENSEVOICE_DIR, KOTONE_SPEECH_WAV)"]
fn sensevoice_language_is_fixed_per_recognizer() {
    use sherpa_onnx::{OfflineRecognizer, OfflineRecognizerConfig, OfflineSenseVoiceModelConfig};

    let dir = sensevoice_dir();
    let english = read_wav_16k(Path::new(
        &std::env::var("KOTONE_SPEECH_WAV").expect("KOTONE_SPEECH_WAV"),
    ));
    let chinese =
        read_wav_16k(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/zh-game-3s.wav"));
    // 和 sensevoice::configure / Transcriber::new 一样的配置，只换语言。
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
    let decode = |r: &OfflineRecognizer, pcm: &[f32], option: Option<&str>| {
        let stream = r.create_stream();
        if let Some(language) = option {
            stream.set_option("language", language);
        }
        stream.accept_waveform(16_000, pcm);
        r.decode(&stream);
        stream.get_result().map(|x| x.text).unwrap_or_default()
    };
    let zh = recognizer("zh");
    let auto = recognizer("auto");
    let rows = [
        (
            "en audio, recognizer zh (the core's default)",
            decode(&zh, &english, None),
        ),
        (
            "en audio, recognizer auto (the interpreter)",
            decode(&auto, &english, None),
        ),
        (
            "en audio, recognizer zh + stream option en",
            decode(&zh, &english, Some("en")),
        ),
        (
            "en audio, recognizer zh + stream option auto",
            decode(&zh, &english, Some("auto")),
        ),
        ("zh audio, recognizer zh", decode(&zh, &chinese, None)),
        ("zh audio, recognizer auto", decode(&auto, &chinese, None)),
    ];
    for (what, text) in &rows {
        println!("{what}: {text}");
    }
    assert!(
        rows[1].1.to_lowercase().contains("house") || rows[1].1.to_lowercase().contains("enem"),
        "auto 应该认得出这句英语：{}",
        rows[1].1
    );
}
