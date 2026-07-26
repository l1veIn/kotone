//! eval 回放 E2E：SAPI fixture 造假录档 → 全部就绪真实引擎回放 → 对比表。
//!
//! `#[ignore]`：依赖真机模型安装情况（X-ASR 模型），手动跑：
//! ```text
//! cargo test -p kotone-stt --features engine-sherpa --test eval_e2e -- --ignored
//! ```
//! sherpa 系引擎需 engine-sherpa feature（ADR-004）。

use std::path::PathBuf;

use kotone_core::eval::{self, EvalSession};
use kotone_core::stt::EngineRegistry;

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/zh-game-3s.wav")
}

#[test]
#[ignore = "E2E：依赖真机 X-ASR 模型，手动跑"]
fn replay_fixture_across_ready_engines() {
    let dir = tempfile::tempdir().unwrap();
    let pcm = eval::read_wav(&fixture_path()).expect("fixture wav 应可解码");
    let session = EvalSession {
        session_id: "e2e-fixture-zh-game".into(),
        engine_id: "sapi-fixture".into(),
        started_at: eval::utc_now_iso(),
        audio_ms: pcm.len() as u64 * 1000 / 16000,
        first_partial_ms: None,
        final_ms: 0,
        partials: vec![],
        final_text: String::new(),
        // SAPI 合成语音原文
        human_label: Some("对面打野在下路".into()),
    };
    eval::record_session_at(dir.path(), &session, &pcm).unwrap();

    let mut registry = EngineRegistry::new();
    kotone_stt::register_builtin(&mut registry);
    let ready: Vec<_> = registry
        .list_info()
        .into_iter()
        .filter(|i| i.is_ready)
        .collect();
    assert!(
        ready.iter().any(|i| i.id == "sherpa-onnx-x-asr-zh-en"),
        "E2E 前提：X-ASR 模型已安装（kotone-cli download x-asr-480ms-streaming-zh-en-punct-int8-2026-06-05）"
    );

    println!(
        "\n{:<26} {:>8} {:>8} {:>8}  {}",
        "引擎", "首字 ms", "最终 ms", "CER", "最终文本"
    );
    for info in &ready {
        match eval::replay_at(dir.path(), &session.session_id, &info.id, &registry) {
            Ok(r) => println!(
                "{:<26} {:>8} {:>8} {:>8}  {}",
                info.id,
                r.first_partial_ms
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "—".into()),
                r.final_ms,
                r.cer.map(|c| format!("{c:.3}"))
                    .unwrap_or_else(|| "—".into()),
                r.final_text
            ),
            Err(e) => println!("{:<26}  回放失败: {e}", info.id),
        }
    }

    // X-ASR 是基线引擎：必须能回放、文本非空、流式有首字延迟
    let x = eval::replay_at(
        dir.path(),
        &session.session_id,
        "sherpa-onnx-x-asr-zh-en",
        &registry,
    )
    .expect("X-ASR 回放应成功");
    assert!(!x.final_text.is_empty(), "X-ASR 应产出文本");
    assert!(
        x.first_partial_ms.is_some(),
        "X-ASR 流式引擎应有首字延迟: {x:?}"
    );

    // mock 回放固定文本，与标注一致 → CER = 0
    let m = eval::replay_at(dir.path(), &session.session_id, "mock-stream", &registry)
        .expect("mock 回放应成功");
    assert_eq!(m.final_text, "对面打野在下路");
    assert_eq!(m.cer, Some(0.0));
    assert!(!m.partials.is_empty(), "mock 流式应有 partial 时间线");
}
