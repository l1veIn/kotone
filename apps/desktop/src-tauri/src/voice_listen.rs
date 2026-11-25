//! 耳朵（ADR 021）：`voice.listen.start / stop` 的桌面实现。
//!
//! - 只录指定程序的声音（[`kotone_platform_windows::app_audio`]，进程回环；不是麦克风）→ Silero VAD 切句
//!   （[`kotone_stt::transcriber::Segmenter`]）→ 每一句交给**本体的语音识别**（[`kotone_stt::listen`]：
//!   你选的识别听得懂外语就用它，否则本地 SenseVoice；和呼叫、语音输入同一个注册表、同一份模型）→ 作为一条
//!   消息交给**调用它的会话**（[`crate::agent::deliver_heard`]），同时在事件总线发 `voice.heard`；
//! - 切句和识别各一个线程：在线识别一句要一两秒，切句那边照常收声音，不丢下一句的开头；
//! - 用本地模型时：玩家停止本地语音识别时一起停（[`stop_local`]），那段对话里收到一句说明；用的就是玩家选的
//!   那个本地模型时，本地语音识别没启动就先启动（和聊天页的呼叫按钮一样，两边共用一份）；另用 SenseVoice 时
//!   （玩家用 X-ASR 之类）只加载 SenseVoice，不去启动玩家那个。用在线识别时每句上传给识别服务商、照常记进用量（按句计费）；
//! - 监听归会话所有（ADR 022）：会话结束（`session.stop`、出错、Kotone 退出）时由代码停掉，
//!   不靠模型记得调 `voice.listen.stop`；停止热键与语音“停下”结束全部子会话，监听一起停；
//! - 只在内存里处理，不存音频；有人在听时气泡显示“正在听队友”。
//!
//! 2026-11-25 以前这里自己建了一个固定的 SenseVoice（共享资源评估第二节第 7 条）：用在线识别的人也得下载、
//! 和所选相同时加载两份、停了本地语音识别还在跑。

use std::collections::HashMap;
use std::sync::Mutex;

use kotone_platform_windows::app_audio::{self, MultiCapture};
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

/// 本地语音识别停了，用本地模型的同传跟着停时，交给那段对话的话。
const STOPPED_WITH_LOCAL: &str = "（本地语音识别停了，同传的监听也一起停了。要接着听，先启动本地语音识别（或者让玩家点同传页面上的“开始”），再 voice.listen.start。）";

struct Listener {
    app_name: String,
    /// 用的是本地模型：停止本地语音识别时一起停。
    local: bool,
    /// 先停录音，再停线程（丢弃顺序即字段顺序；录音停了，切句、识别两个线程跟着收尾）。
    _capture: MultiCapture,
    _workers: Vec<std::thread::JoinHandle<()>>,
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

/// 按现在的设置挑同传的识别，在线识别把连接（地址、模型、密钥）填好。
fn listen_model(app: &AppHandle) -> Result<kotone_stt::listen::ListenModel, String> {
    let shared = app.state::<crate::SharedState>();
    let settings = shared.settings.read().unwrap().clone();
    let mut model = kotone_stt::listen::listen_model(&settings, &shared.engines)?;
    if !model.local {
        // 和呼叫 / 语音输入同一份填法（kotone-core）
        kotone_core::stt::add_remote_connection(
            &settings,
            &model.engine_id,
            &mut model.config,
            Some(shared.secrets.as_ref()),
        );
    }
    Ok(model)
}

/// `voice.listen.start`：`owner` 是调用它的会话。同一个会话再调用会先停掉旧的。
/// `languages` 参数收下但不用：识别自动判语种（见 `kotone-agent` 里这个工具的说明）。
pub fn start(app: &AppHandle, owner: &str, arguments: &Value) -> Result<Value, String> {
    let app_name = match arguments.get("app").and_then(Value::as_str).map(str::trim) {
        Some(name) if !name.is_empty() => name.to_string(),
        _ => default_app()?,
    };
    let trees = app_audio::pids_by_name(&app_name).len();
    if trees == 0 {
        return Err(format!("{app_name} 没有在运行"));
    }
    let model = listen_model(app)?;
    stop_owner(app, owner);

    start_listener(app, owner, app_name, trees, model)
}

/// 本地语音识别没启动就先启动（和聊天页的呼叫按钮一样：玩家要用识别了）。在识别线程里调（会等）。
#[cfg(feature = "vad-silero")]
fn ensure_local_running(app: &AppHandle) -> Result<(), String> {
    use kotone_core::runtime::RuntimePhase;
    let phase = || app.state::<crate::runtime::RuntimeManager>().phase();
    if phase() == RuntimePhase::Stopped {
        kotone_core::log::log("listen: 本地语音识别没启动，先启动");
        tauri::async_runtime::block_on(crate::runtime::start(app))?;
    }
    for _ in 0..150 {
        match phase() {
            RuntimePhase::Running => return Ok(()),
            RuntimePhase::Stopped => break,
            _ => std::thread::sleep(std::time::Duration::from_millis(200)),
        }
    }
    Err("本地语音识别没启动起来".into())
}

#[cfg(feature = "vad-silero")]
fn start_listener(
    app: &AppHandle,
    owner: &str,
    app_name: String,
    trees: usize,
    model: kotone_stt::listen::ListenModel,
) -> Result<Value, String> {
    use std::sync::mpsc;

    let mut segmenter = kotone_stt::transcriber::Segmenter::new()?;
    let (tx, rx) = mpsc::sync_channel::<Vec<f32>>(64);
    let capture = MultiCapture::start(
        &app_name,
        Box::new(move |chunk: &[f32]| {
            // 切句跟不上时丢最新的块，不阻塞录音线程。
            let _ = tx.try_send(chunk.to_vec());
        }),
    )?;
    // 切好的句子；识别跟不上（在线识别一句一两秒）时最多攒 8 句，再多丢新的。
    let (segments_tx, segments_rx) = mpsc::sync_channel::<Vec<f32>>(8);
    let (go_tx, go_rx) = mpsc::channel::<()>();

    let (seg_owner, seg_name) = (owner.to_string(), app_name.clone());
    let segment_worker = std::thread::Builder::new()
        .name("kotone-listen-vad".into())
        .spawn(move || {
            // 录音停了（Listener 被丢弃）发送端随之关闭，这里结束。
            let (mut samples, mut peak, mut dropped) = (0usize, 0f32, 0usize);
            let mut send = |segment: Vec<f32>| {
                if segments_tx.try_send(segment).is_err() {
                    dropped += 1;
                }
            };
            while let Ok(chunk) = rx.recv() {
                samples += chunk.len();
                peak = chunk.iter().fold(peak, |m, x| m.max(x.abs()));
                segmenter.push(&chunk).into_iter().for_each(&mut send);
            }
            segmenter.flush().into_iter().for_each(&mut send);
            kotone_core::log::log(&format!(
                "listen: {seg_owner} heard {:.1}s of audio from {seg_name} (peak {peak:.2}){}",
                samples as f32 / 16_000.0,
                if dropped > 0 {
                    format!(", {dropped} sentences dropped (recognition too slow)")
                } else {
                    String::new()
                }
            ));
        })
        .map_err(|e| format!("无法启动切句线程：{e}"))?;

    let worker_app = app.clone();
    let worker_owner = owner.to_string();
    let worker_name = app_name.clone();
    let worker_model = model.clone();
    let recognize_worker = std::thread::Builder::new()
        .name("kotone-listen".into())
        .spawn(move || {
            // 等登记好再开始（失败时要能把自己从表里摘掉）。
            if go_rx.recv().is_err() {
                return;
            }
            let model = worker_model;
            let fail = |why: String| {
                kotone_core::log::log(&format!("listen: {worker_owner} can't recognize: {why}"));
                crate::agent::deliver_listen_note(
                    &worker_app,
                    &worker_owner,
                    &format!("（同传的监听停了：{why}）"),
                );
                stop_owner(&worker_app, &worker_owner);
            };
            if model.local && model.follows_selected {
                if let Err(e) = ensure_local_running(&worker_app) {
                    return fail(e);
                }
            }
            let engines = worker_app.state::<crate::SharedState>().engines.clone();
            let Some(engine) = engines.get(&model.engine_id) else {
                return fail(format!("这个版本没有 {} 这个识别", model.engine_id));
            };
            if model.local {
                // 本体已经加载了同一份时直接用（offline_sherpa 按模型缓存），否则现在加载。
                if let Err(e) = engine.warmup(&model.config) {
                    return fail(e);
                }
            }
            let (mut sentences, mut errors) = (0usize, 0usize);
            while let Ok(segment) = segments_rx.recv() {
                match kotone_stt::listen::recognize(engine, &model.config, &segment) {
                    Ok(Some(heard)) => {
                        sentences += 1;
                        publish(&worker_app, &worker_owner, &worker_name, &heard);
                    }
                    Ok(None) => {}
                    Err(e) => {
                        errors += 1;
                        if errors == 1 || errors % 10 == 0 {
                            kotone_core::log::log(&format!(
                                "listen: {worker_owner} recognition failed ({errors}): {e}"
                            ));
                        }
                    }
                }
            }
            kotone_core::log::log(&format!(
                "listen: {worker_owner} {sentences} sentences with {}{}",
                model.model_id,
                if errors > 0 {
                    format!(", {errors} failed")
                } else {
                    String::new()
                }
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
                local: model.local,
                _capture: capture,
                _workers: vec![segment_worker, recognize_worker],
            },
        );
    let _ = go_tx.send(());
    kotone_core::log::log(&format!(
        "listen: {owner} listens to {app_name} ({trees} processes) with {}{}",
        model.model_id,
        if model.local { "" } else { " (online)" }
    ));
    crate::bubble::listening_to(app, Some(&app_name));
    let note = if model.local {
        "每听到一句会作为一条消息进这段对话（本地识别）".to_string()
    } else {
        format!(
            "每听到一句会作为一条消息进这段对话；识别用在线的 {}（队友的声音会发给它识别，每句计费）",
            model.model_id
        )
    };
    Ok(json!({
        "listening": true,
        "app": app_name,
        "recognizer": model.model_id,
        "online": !model.local,
        "note": note,
    }))
}

#[cfg(not(feature = "vad-silero"))]
fn start_listener(
    _app: &AppHandle,
    _owner: &str,
    _app_name: String,
    _trees: usize,
    _model: kotone_stt::listen::ListenModel,
) -> Result<Value, String> {
    Err("这个版本没有带语音切句（silero VAD）".into())
}

#[cfg(feature = "vad-silero")]
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

/// 玩家停止本地语音识别：停掉用本地模型的监听，各自那段对话里说一句（在线识别的不受影响）。
pub fn stop_local(app: &AppHandle) {
    let stopped: Vec<(String, Listener)> = state(app)
        .map(|s| {
            let mut listeners = s.listeners.lock().unwrap();
            let owners: Vec<String> = listeners
                .iter()
                .filter(|(_, l)| l.local)
                .map(|(owner, _)| owner.clone())
                .collect();
            owners
                .into_iter()
                .filter_map(|owner| listeners.remove(&owner).map(|l| (owner, l)))
                .collect()
        })
        .unwrap_or_default();
    for (owner, listener) in stopped {
        drop(listener);
        kotone_core::log::log(&format!(
            "listen: {owner} stopped with the local recognition"
        ));
        crate::agent::deliver_listen_note(app, &owner, STOPPED_WITH_LOCAL);
    }
    if active(app).is_empty() {
        crate::bubble::listening_to(app, None);
    }
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
