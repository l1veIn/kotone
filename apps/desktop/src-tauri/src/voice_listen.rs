//! 耳朵（ADR 021）：`voice.listen.start / stop` 的桌面实现。
//!
//! - 只录指定程序的声音（[`kotone_platform_windows::app_audio`]，进程回环）→ Silero VAD 切句 →
//!   SenseVoice 识别（[`kotone_stt::transcriber`]）→ 每一句作为一条消息交给**调用它的会话**
//!   （[`crate::agent::deliver_heard`]），同时在事件总线发 `voice.heard`（外部 Agent 也能订阅）；
//! - 监听归会话所有（ADR 022）：会话结束（`session.stop`、出错、Kotone 退出）时由代码停掉，
//!   不靠模型记得调 `voice.listen.stop`；停止热键与语音“停下”结束全部子会话，监听一起停；
//! - 只在内存里处理，不存音频；有人在听时气泡显示“正在听队友”。

use std::collections::HashMap;
use std::sync::Mutex;

use kotone_platform_windows::app_audio::{self, MultiCapture};
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

struct Listener {
    app_name: String,
    /// 先停录音，再停转写线程（丢弃顺序即字段顺序）。
    _capture: MultiCapture,
    _worker: std::thread::JoinHandle<()>,
}

#[derive(Default)]
pub struct VoiceListen {
    listeners: Mutex<HashMap<String, Listener>>,
}

fn state(app: &AppHandle) -> Option<tauri::State<'_, VoiceListen>> {
    app.try_state::<VoiceListen>()
}

pub fn is_listening(app: &AppHandle, owner: &str) -> bool {
    state(app).is_some_and(|s| s.listeners.lock().unwrap().contains_key(owner))
}

/// 正在听的（会话 id，程序名）。
pub fn active(app: &AppHandle) -> Vec<(String, String)> {
    state(app)
        .map(|s| {
            s.listeners
                .lock()
                .unwrap()
                .iter()
                .map(|(owner, l)| (owner.clone(), l.app_name.clone()))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(all(feature = "engine-sherpa", feature = "vad-silero"))]
fn sensevoice_dir(app: &AppHandle) -> std::path::PathBuf {
    let settings = app
        .state::<crate::SharedState>()
        .settings
        .read()
        .unwrap()
        .clone();
    kotone_stt::model::models_dir_from(&settings)
        .join("sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17")
}

/// 缺省听哪个程序：当前前台窗口的进程（不能是 Kotone 自己）。
fn default_app() -> Result<String, String> {
    use kotone_core::tools::WindowObserver;
    let window = kotone_platform_windows::window_tools::WindowsWindowObserver
        .foreground_window()
        .map_err(|e| e.to_string())?
        .ok_or("没有前台窗口：指定 app（如 TslGame.exe、Discord.exe）")?;
    if window.identity.pid == std::process::id() {
        return Err("前台是 Kotone 自己：指定 app（如 TslGame.exe、Discord.exe）".into());
    }
    window
        .process_name
        .ok_or_else(|| "读不到前台程序的名字：指定 app".to_string())
}

/// `voice.listen.start`：`owner` 是调用它的会话。同一个会话再调用会先停掉旧的。
pub fn start(app: &AppHandle, owner: &str, arguments: &Value) -> Result<Value, String> {
    let app_name = match arguments.get("app").and_then(Value::as_str).map(str::trim) {
        Some(name) if !name.is_empty() => name.to_string(),
        _ => default_app()?,
    };
    let trees = app_audio::pids_by_name(&app_name).len();
    if trees == 0 {
        return Err(format!("{app_name} 没有在运行"));
    }
    stop_owner(app, owner);

    start_listener(app, owner, app_name, trees)
}

#[cfg(all(feature = "engine-sherpa", feature = "vad-silero"))]
fn start_listener(
    app: &AppHandle,
    owner: &str,
    app_name: String,
    trees: usize,
) -> Result<Value, String> {
    let mut transcriber = kotone_stt::transcriber::Transcriber::new(&sensevoice_dir(app))?;
    let (tx, rx) = std::sync::mpsc::sync_channel::<Vec<f32>>(64);
    let capture = MultiCapture::start(
        &app_name,
        Box::new(move |chunk: &[f32]| {
            // 转写跟不上时丢最新的块，不阻塞录音线程。
            let _ = tx.try_send(chunk.to_vec());
        }),
    )?;
    let worker_app = app.clone();
    let worker_owner = owner.to_string();
    let worker_name = app_name.clone();
    let worker = std::thread::Builder::new()
        .name("kotone-listen".into())
        .spawn(move || {
            // 录音停了（Listener 被丢弃）发送端随之关闭，这里结束。
            let (mut samples, mut peak, mut sentences) = (0usize, 0f32, 0usize);
            while let Ok(chunk) = rx.recv() {
                samples += chunk.len();
                peak = chunk.iter().fold(peak, |m, x| m.max(x.abs()));
                for heard in transcriber.push(&chunk) {
                    sentences += 1;
                    publish(&worker_app, &worker_owner, &worker_name, &heard);
                }
            }
            for heard in transcriber.flush() {
                sentences += 1;
                publish(&worker_app, &worker_owner, &worker_name, &heard);
            }
            kotone_core::log::log(&format!(
                "listen: {worker_owner} heard {:.1}s of audio (peak {peak:.2}), {sentences} sentences",
                samples as f32 / 16_000.0
            ));
        })
        .map_err(|e| format!("无法启动识别线程：{e}"))?;
    state(app)
        .ok_or("监听尚未就绪")?
        .listeners
        .lock()
        .unwrap()
        .insert(
            owner.to_string(),
            Listener {
                app_name: app_name.clone(),
                _capture: capture,
                _worker: worker,
            },
        );
    kotone_core::log::log(&format!(
        "listen: {owner} listens to {app_name} ({trees} processes)"
    ));
    crate::bubble::listening_to(app, Some(&app_name));
    Ok(json!({"listening": true, "app": app_name, "note": "每听到一句会作为一条消息进这个会话"}))
}

#[cfg(not(all(feature = "engine-sherpa", feature = "vad-silero")))]
fn start_listener(
    _app: &AppHandle,
    _owner: &str,
    _app_name: String,
    _trees: usize,
) -> Result<Value, String> {
    Err("这个版本没有带本地语音识别".into())
}

#[cfg(all(feature = "engine-sherpa", feature = "vad-silero"))]
fn publish(
    app: &AppHandle,
    owner: &str,
    app_name: &str,
    heard: &kotone_stt::transcriber::Utterance,
) {
    crate::tool_service::publish(
        app,
        "voice.heard",
        json!({"app": app_name, "text": heard.text, "language": heard.language, "durationMs": heard.duration_ms}),
    );
    crate::agent::deliver_heard(app, owner, app_name, heard.language, &heard.text);
}

/// 停掉这个会话名下的监听。返回有没有停掉。
pub fn stop_owner(app: &AppHandle, owner: &str) -> bool {
    let removed = state(app).and_then(|s| s.listeners.lock().unwrap().remove(owner));
    let stopped = removed.is_some();
    drop(removed);
    if stopped {
        kotone_core::log::log(&format!("listen: {owner} stopped"));
        if active(app).is_empty() {
            crate::bubble::listening_to(app, None);
        }
    }
    stopped
}

/// 全部停掉（Kotone 退出）。
pub fn stop_all(app: &AppHandle) {
    let all: Vec<Listener> = state(app)
        .map(|s| {
            s.listeners
                .lock()
                .unwrap()
                .drain()
                .map(|(_, l)| l)
                .collect()
        })
        .unwrap_or_default();
    drop(all);
}
