//! eval 回放 E2E：SAPI fixture 造假录档 → 全部就绪真实引擎回放 → 对比表。
//!
//! `#[ignore]`：依赖真机模型安装情况（whisper-cli + ggml 模型），手动跑：
//! ```text
//! cargo test -p kotone-stt --test eval_e2e -- --ignored
//! cargo test -p kotone-stt --features engine-sherpa --test eval_e2e -- --ignored
//! ```
//! 默认构建覆盖 whisper + mock；sherpa 需 engine-sherpa feature（ADR-004）。

use std::path::PathBuf;

use kotone_core::eval::{self, EvalSession};
use kotone_core::stt::EngineRegistry;

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/zh-game-3s.wav")
}

#[test]
#[ignore = "E2E：依赖真机 whisper-cli + 模型，手动跑"]
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
        // SAPI 合成语音原文（见 whisper_e2e / sherpa_e2e 的基准）
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
        ready.iter().any(|i| i.id == "whisper-cpp-sidecar"),
        "E2E 前提：whisper-cli + ggml 模型已安装（kotone-cli download bin / small）"
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

    // whisper 是基线引擎：必须能回放且文本非空
    let w = eval::replay_at(dir.path(), &session.session_id, "whisper-cpp-sidecar", &registry)
        .expect("whisper 回放应成功");
    assert!(!w.final_text.is_empty(), "whisper 应产出文本");
    assert!(w.first_partial_ms.is_none(), "whisper 非流式，无首字延迟");

    // mock 回放固定文本，与标注一致 → CER = 0
    let m = eval::replay_at(dir.path(), &session.session_id, "mock-stream", &registry)
        .expect("mock 回放应成功");
    assert_eq!(m.final_text, "对面打野在下路");
    assert_eq!(m.cer, Some(0.0));
    assert!(!m.partials.is_empty(), "mock 流式应有 partial 时间线");

    // sherpa（engine-sherpa feature 开启且模型齐备时）：流式引擎应有 partial
    #[cfg(feature = "engine-sherpa")]
    if ready.iter().any(|i| i.id == "sherpa-onnx-zipformer-zh") {
        let s = eval::replay_at(
            dir.path(),
            &session.session_id,
            "sherpa-onnx-zipformer-zh",
            &registry,
        )
        .expect("sherpa 回放应成功");
        assert!(!s.final_text.is_empty(), "sherpa 应产出文本");
        assert!(
            s.first_partial_ms.is_some(),
            "sherpa 流式引擎应有首字延迟: {s:?}"
        );
    }
}
