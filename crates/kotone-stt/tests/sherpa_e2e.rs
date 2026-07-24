//! sherpa-onnx 流式 Zipformer 真实端到端测试：真实模型（~200MB）+
//! 仓库 fixture 中文语音（tests/fixtures/zh-game-3s.wav，与 whisper E2E 同源，
//! 便于两引擎并排对比）。
//!
//! 双门控：需 feature `engine-sherpa` + 显式 --ignored：
//!     cargo test -p kotone-stt --features engine-sherpa --test sherpa_e2e -- --ignored --nocapture
//!
//! 首次运行自动用 kotone-stt 自己的下载器补齐模型（即「真实下载测试」）。

#![cfg(feature = "engine-sherpa")]

use kotone_core::stt::{SessionConfig, SttEngine, SttEvent};
use kotone_stt::{model, sherpa::SherpaEngine};

/// 极简 WAV 解码（与 whisper_e2e 同款：fixture 已知为 16kHz 16bit mono PCM）
fn decode_wav_f32(path: &std::path::Path) -> Vec<f32> {
    let b = std::fs::read(path).expect("读取 fixture wav 失败");
    assert!(b.len() > 44 && &b[0..4] == b"RIFF" && &b[8..12] == b"WAVE", "非 WAV 文件");
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
        off += 8 + size + (size & 1);
    }
    panic!("WAV 中未找到 data chunk");
}

#[test]
#[ignore = "真实下载 + 真实转写：--ignored 显式运行"]
fn sherpa_streaming_end_to_end_zh_speech() {
    let model_id = "zipformer-bilingual-zh-en-2023-02-20";

    // 1) 真实下载（已就绪则跳过）：走生产下载路径（多文件 + 逐文件 SHA256）
    if !model::multi_model_ready(model_id) {
        eprintln!("[e2e] 下载 {model_id}（~200MB）…");
        model::download(model_id, &|done, total| {
            eprintln!("[e2e] model: {done}/{total:?}");
        })
        .expect("sherpa 模型下载失败");
    }

    // 2) 引擎就绪
    let engine = SherpaEngine::new();
    assert!(engine.is_ready(), "模型齐备后 is_ready 应为 true");

    // 3) fixture → session，按 0.25s chunk 喂入（模拟实时采集节奏）
    let wav = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/zh-game-3s.wav");
    let pcm = decode_wav_f32(&wav);
    assert!(pcm.len() > 16000, "fixture 至少 1s 音频");

    let cfg = SessionConfig {
        language: "zh".into(),
        hotwords: vec!["打野".into(), "下路".into()],
        options: serde_json::json!({ "threads": 4 }),
    };
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let mut session = engine.start_session(&cfg, tx).expect("start_session 失败");

    let chunk = 4000usize; // 0.25s @16kHz
    let t_start = std::time::Instant::now();
    let mut first_partial_at = None;
    for c in pcm.chunks(chunk) {
        session.push_audio(c).unwrap();
        if first_partial_at.is_none() {
            if let Ok(SttEvent::Partial { text }) = rx.try_recv() {
                first_partial_at = Some((t_start.elapsed(), text));
            }
        }
    }
    let t = session.finalize().expect("finalize 失败");

    // 收 partial 序列
    let mut partials = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        if let SttEvent::Partial { text } = ev {
            partials.push(text);
        }
    }
    eprintln!(
        "[e2e] partial 序列（{} 条）：{:?}",
        partials.len(),
        partials
    );
    if let Some((at, text)) = &first_partial_at {
        eprintln!("[e2e] 首条 partial：{at:.2?} → 「{text}」");
    }
    eprintln!(
        "[e2e] final：「{}」（latency_ms={}，音频={:.1}s）",
        t.text,
        t.latency_ms,
        pcm.len() as f32 / 16000.0
    );

    // 4) 断言：流式语义（有 partial）+ final 合理（含关键词；SAPI 音质放宽）
    assert!(!partials.is_empty(), "流式引擎应产生 partial 序列");
    assert!(!t.text.is_empty(), "final 文本不应为空");
    assert!(
        t.text.contains("打野"),
        "final 应包含热词「打野」，实际：「{}」",
        t.text
    );
}
