//! Offline validation: writes a WAV without opening a speaker or user settings.
use kotone_core::speech::SpeechRequest;
use kotone_tts::KokoroEngine;

fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let directory = args
        .next()
        .ok_or("usage: synthesize MODEL_DIR WAV_PATH [VOICE_ID] [TEXT]")?;
    let output = args.next().ok_or("missing output WAV path")?;
    let voice = args.next().unwrap_or_else(|| "kokoro:zf_001".into());
    let text = args
        .next()
        .unwrap_or_else(|| "收到！小龙快刷新啦，记得提前占好位置。别急，稳住这波！".into());
    let started = std::time::Instant::now();
    let engine = KokoroEngine::open(std::path::Path::new(&directory))?;
    let loaded = started.elapsed();
    let audio = engine.synthesize(
        &SpeechRequest {
            text,
            voice_id: Some(voice.clone()),
            volume: 80,
            rate: 0,
        },
        || true,
    )?;
    let generation = started.elapsed() - loaded;
    if !sherpa_onnx::write(&output, &audio.samples, audio.sample_rate as i32) {
        return Err("failed to save wav".into());
    }
    let seconds = audio.samples.len() as f64 / audio.sample_rate as f64;
    println!(
        "voice={voice} load_ms={} synth_ms={} audio_seconds={seconds:.2} rtf={:.3} path={output}",
        loaded.as_millis(),
        generation.as_millis(),
        generation.as_secs_f64() / seconds
    );
    Ok(())
}
