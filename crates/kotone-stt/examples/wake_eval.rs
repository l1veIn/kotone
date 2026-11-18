//! 唤醒词实测：把一批 16 kHz 单声道 WAV 喂给真实的唤醒模型（和桌面端同一个 `WakeDetector`），
//! 统计每个唤醒词喊中几次、在普通说话 / 游戏声音里误唤醒几次。
//!
//! ```text
//! cargo run -p kotone-stt --features engine-sherpa --example wake_eval -- <模型目录> <清单.json> [low|normal|high]
//! ```
//!
//! 清单：`[{"wav": "a.wav", "keyword": "小雨小雨", "readings": [], "expect": 1}, ...]`；
//! `keyword` 为空 = 负样本（只统计误唤醒），这时用 `keywords` 字段列出要同时监听的唤醒词。
//! 输出一行一个 JSON：文件、唤醒词、期望次数、命中次数、命中的词。

use std::path::Path;

use kotone_stt::wake::WakeDetector;
use kotone_stt::wake_word;
use serde_json::{json, Value};

fn keyword_lines(words: &[(String, Vec<String>)]) -> Result<String, String> {
    let mut out = String::new();
    for (text, readings) in words {
        let check = wake_word::check(text, readings);
        if !check.ok {
            return Err(format!("{text}: {}", check.problems.join("；")));
        }
        out.push_str(&wake_word::keyword_line(&check.tokens, &check.text));
        out.push('\n');
    }
    Ok(out)
}

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    let dir = Path::new(args.get(1).ok_or("缺少模型目录")?);
    let manifest: Vec<Value> = serde_json::from_str(
        &std::fs::read_to_string(args.get(2).ok_or("缺少清单")?).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let sensitivity = args.get(3).map(String::as_str).unwrap_or("normal");
    let base = Path::new(&args[2]).parent().unwrap_or(Path::new("."));
    for item in manifest {
        let wav = base.join(item["wav"].as_str().unwrap_or_default());
        let words: Vec<(String, Vec<String>)> = match item["keyword"].as_str() {
            Some(k) if !k.is_empty() => vec![(
                k.to_string(),
                serde_json::from_value(item["readings"].clone()).unwrap_or_default(),
            )],
            _ => item["keywords"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(|k| (k.to_string(), Vec::new()))
                        .collect()
                })
                .unwrap_or_default(),
        };
        let lines = keyword_lines(&words)?;
        let mut detector = WakeDetector::new(dir, &lines, sensitivity)?;
        let samples = kotone_core::eval::read_wav(&wav)?;
        let mut hits = Vec::new();
        for chunk in samples.chunks(1600) {
            if let Some(k) = detector.accept(chunk) {
                hits.push(k);
            }
        }
        if let Some(k) = detector.accept(&vec![0.0; 16_000]) {
            hits.push(k);
        }
        println!(
            "{}",
            json!({
                "wav": item["wav"],
                "keyword": item["keyword"],
                "voice": item["voice"],
                "expect": item["expect"],
                "seconds": samples.len() as f64 / 16_000.0,
                "hits": hits.len(),
                "words": hits,
            })
        );
    }
    Ok(())
}
