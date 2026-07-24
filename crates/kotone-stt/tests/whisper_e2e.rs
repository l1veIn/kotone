//! whisper.cpp sidecar 真实端到端测试：真实二进制 + 真实 ggml-small 模型 +
//! 仓库 fixture 中文语音（tests/fixtures/zh-game-3s.wav，Windows SAPI 合成
//! 「对面打野在下路」，16kHz 16bit mono）。
//!
//! 默认 `#[ignore]`（要下载 ~8MB 运行时 + ~466MB 模型、跑真实推理）：
//!     cargo test -p kotone-stt --test whisper_e2e -- --ignored --nocapture
//!
//! 首次运行自动用 kotone-stt 自己的下载器补齐 bin 与模型（即「真实下载测试」）；
//! 已就绪则直接转写。

use kotone_core::stt::{SessionConfig, SttEngine};
use kotone_stt::{model, whisper_sidecar::WhisperSidecarEngine};

/// 极简 WAV 解码（fixture 已知为 16kHz 16bit mono PCM，容错其他 layout 只做基本校验）
fn decode_wav_f32(path: &std::path::Path) -> Vec<f32> {
    let b = std::fs::read(path).expect("读取 fixture wav 失败");
    assert!(b.len() > 44 && &b[0..4] == b"RIFF" && &b[8..12] == b"WAVE", "非 WAV 文件");
    let channels = u16::from_le_bytes(b[22..24].try_into().unwrap());
    let rate = u32::from_le_bytes(b[24..28].try_into().unwrap());
    let bits = u16::from_le_bytes(b[34..36].try_into().unwrap());
    assert_eq!((channels, rate, bits), (1, 16000, 16), "fixture 应为 16kHz 16bit mono");
    // 找 data chunk（44 是标准头；稳妥起见顺序扫 chunk）
    let mut off = 12;
    while off + 8 <= b.len() {
        let id = &b[off..off + 4];
        let size = u32::from_le_bytes(b[off + 4..off + 8].try_into().unwrap()) as usize;
        if id == b"data" {
            return b[off + 8..off + 8 + size]
                .chunks_exact(2)
                .map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0)
                .collect();
        }
        off += 8 + size + (size & 1); // chunk 按偶数对齐
    }
    panic!("WAV 中未找到 data chunk");
}

#[test]
#[ignore = "真实下载 + 真实转写：--ignored 显式运行"]
fn whisper_end_to_end_zh_speech() {
    // 1) 真实下载（已就绪则跳过）：bin → 模型，走生产下载路径（含 SHA256 校验）
    if !model::bin_installed() {
        eprintln!("[e2e] 下载 whisper-cli 运行时 …");
        model::download(model::WHISPER_BIN_ID, &|done, total| {
            eprintln!("[e2e] bin: {done}/{total:?}");
        })
        .expect("whisper-cli 运行时下载失败");
    }
    let model_id = "ggml-small";
    if !model::model_path(model_id).unwrap().exists() {
        eprintln!("[e2e] 下载 {model_id}（~466MB）…");
        model::download(model_id, &|_, _| {}).expect("模型下载失败");
    }

    // 2) 引擎就绪
    let engine = WhisperSidecarEngine;
    assert!(engine.is_ready(), "bin + 模型就绪后 is_ready 应为 true");

    // 3) fixture → session → finalize
    let wav = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/zh-game-3s.wav");
    let pcm = decode_wav_f32(&wav);
    assert!(pcm.len() > 16000, "fixture 至少 1s 音频");

    let cfg = SessionConfig {
        language: "zh".into(),
        hotwords: vec!["打野".into(), "下路".into()],
        options: serde_json::json!({ "model": model_id, "threads": 4 }),
    };
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let mut session = engine.start_session(&cfg, tx).expect("start_session 失败");
    session.push_audio(&pcm).unwrap();

    let started = std::time::Instant::now();
    let t = session.finalize().expect("finalize 失败");
    let wall = started.elapsed();
    eprintln!(
        "[e2e] 转写结果：「{}」（latency_ms={}，wall={wall:.1?}，音频={:.1}s）",
        t.text,
        t.latency_ms,
        pcm.len() as f32 / 16000.0
    );

    // 4) 断言：SAPI 合成音质一般，放宽到包含关键词「打野」
    assert!(!t.text.is_empty(), "转写结果不应为空");
    assert!(
        t.text.contains("打野"),
        "结果应包含热词「打野」，实际：「{}」",
        t.text
    );
    assert!(t.latency_ms > 0);
}
