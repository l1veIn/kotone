//! kotone-cli：无 Tauri 的命令行前端（验收 core 可独立运行的关键证据）。
//!
//! 子命令（详见 docs/cli.md）：
//! - `send`：一次性注入（取代原 src-tauri/examples/inject_cli.rs）
//! - `listen`：热键全链路 JSONL；--wav / --no-hotkey 单次会话模式（自动化测试）
//! - `download`：模型 / whisper-cli 运行时下载
//! - `config`：show / get / set（点路径写入 ~/.kotone/config.json）
//! - `devices` / `play`：设备枚举 / wav 播放（虚拟声卡回路）
//! - `eval`：引擎评测——录档列表 / 语料回放（多引擎对比）/ 人工标注 / CER 报告

use clap::{Parser, Subcommand};

use kotone_core::profile::{self, GameProfile};
use kotone_core::settings::{self, HotkeyBackend, Settings};
use kotone_core::stt::EngineRegistry;
#[cfg(windows)]
use kotone_core::hotkey::HotkeySource;

#[derive(Parser)]
#[command(name = "kotone-cli", version, about = "Kotone（琴音）命令行前端")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// 一次性注入文本到前台窗口（默认 generic profile：Enter → Unicode 逐字 → Enter）
    Send {
        /// 要发送的文本
        #[arg(long)]
        text: String,
        /// ~/.kotone/profiles 中的 profile id（默认 generic，通配任意前台窗口）
        #[arg(long, default_value = "generic")]
        profile: String,
        /// 改走剪贴板 + Ctrl+V 备选路径
        #[arg(long)]
        clipboard: bool,
        /// 发送前等待毫秒数（手工切换到目标窗口用）
        #[arg(long, default_value_t = 0)]
        delay_ms: u64,
    },
    /// 前台常驻：LL 热键 → orchestrator → JSONL 打印全部事件（Ctrl+C 退出）；
    /// --wav / --no-hotkey 进入单次会话模式（自动化测试用，见 docs/cli.md）
    Listen {
        /// STT 引擎 id
        #[arg(long, default_value = "mock-stream")]
        engine: String,
        /// 游戏 profile id（缺省用配置文件值）
        #[arg(long)]
        profile: Option<String>,
        /// 热键（缺省用配置文件值，如 F8 / Alt+V）
        #[arg(long)]
        key: Option<String>,
        /// 触发模式 toggle|hold（缺省用配置文件值）
        #[arg(long)]
        mode: Option<String>,
        /// wav 直灌：把 16kHz wav 作为 PCM 源喂给 orchestrator（隐含 --no-hotkey）；
        /// 会话强制预览收尾不触发真实注入，喂完自动 finalize 退出
        #[arg(long)]
        wav: Option<String>,
        /// 跳过 LL 钩子：立即开始会话，--duration 到时自动结束（配合 --wav 或虚拟声卡）
        #[arg(long)]
        no_hotkey: bool,
        /// 会话时长（秒），到时自动结束退出；--wav 模式缺省 = 音频时长
        #[arg(long)]
        duration: Option<u64>,
        /// wav 喂入速度倍率（1.0 = 实时，0 = 全速）
        #[arg(long)]
        speed: Option<f64>,
    },
    /// 下载模型或 whisper-cli 运行时：bin | tiny | base | small
    Download {
        /// 下载目标：bin（whisper-cli 运行时）或模型短名（tiny/base/small）
        target: String,
    },
    /// 配置管理：show / get / set（点路径写入 ~/.kotone/config.json）
    Config {
        #[command(subcommand)]
        action: ConfigCommand,
    },
    /// 枚举音频输入（采集）与输出（播放）设备，标出默认与虚拟声卡
    Devices,
    /// 播放 16kHz wav 到输出设备（重采样到设备率；--device 为名称子串）
    Play {
        /// wav 文件路径（16kHz/16bit/mono，如 eval 录档或 fixtures）
        wav: String,
        /// 输出设备名子串（如 "CABLE Input"；缺省 = 系统默认输出）
        #[arg(long)]
        device: Option<String>,
    },
    /// 引擎评测：录档列表 / 语料回放 / 人工标注 / 对比报告
    Eval {
        #[command(subcommand)]
        action: EvalCommand,
    },
}

#[derive(Subcommand)]
enum ConfigCommand {
    /// 打印当前完整配置（JSON，含默认值合并结果）
    Show,
    /// 读取单个配置项（点路径，如 hotkey.key / autoSend）
    Get {
        key: String,
    },
    /// 写入配置项（点路径；支持 hotkey.key / hotkey.mode / hotkeyBackend /
    /// sttEngine / activeProfileId / autoSend / audioDeviceId / language /
    /// evalRecording / runAsAdminOnStart / interactionMode）
    Set {
        key: String,
        /// 写入的值；--capture 模式下省略（录入结果即值）
        value: Option<String>,
        /// 按键录入模式：仅 hotkey.key 可用，弹出提示后按下组合键即写入
        #[arg(long)]
        capture: bool,
    },
}

#[derive(Subcommand)]
enum EvalCommand {
    /// 列出 ~/.kotone/eval 中的录档会话
    List,
    /// 回放会话：指定引擎跑单引擎；不指定则全部就绪引擎对比
    Replay {
        /// 录档会话 ID（eval list 可查）
        session_id: String,
        /// 目标引擎 id（缺省 = 全部 is_ready 引擎）
        #[arg(long)]
        engine: Option<String>,
    },
    /// 回填人工标注（正确文本），供 CER 计算
    Label {
        /// 录档会话 ID
        session_id: String,
        /// 该段音频的正确文本
        #[arg(long)]
        text: String,
    },
    /// 已标注会话 × 就绪引擎的 CER / 延迟报告（Markdown 表）
    Report,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let code = match cli.command {
        Command::Send {
            text,
            profile,
            clipboard,
            delay_ms,
        } => cmd_send(&text, &profile, clipboard, delay_ms),
        Command::Listen {
            engine,
            profile,
            key,
            mode,
            wav,
            no_hotkey,
            duration,
            speed,
        } => cmd_listen(&engine, profile, key, mode, wav, no_hotkey, duration, speed).await,
        Command::Download { target } => cmd_download(&target).await,
        Command::Config { action } => match action {
            ConfigCommand::Show => cmd_config_show(),
            ConfigCommand::Get { key } => cmd_config_get(&key),
            ConfigCommand::Set { key, value, capture } => {
                cmd_config_set(&key, value.as_deref(), capture)
            }
        },
        Command::Devices => cmd_devices(),
        Command::Play { wav, device } => cmd_play(&wav, device),
        Command::Eval { action } => match action {
            EvalCommand::List => cmd_eval_list(),
            EvalCommand::Replay { session_id, engine } => cmd_eval_replay(&session_id, engine).await,
            EvalCommand::Label { session_id, text } => cmd_eval_label(&session_id, &text),
            EvalCommand::Report => cmd_eval_report().await,
        },
    };
    std::process::exit(code);
}

/// download：bin → whisper-cli 运行时；tiny/base/small → ggml 模型。单行刷新进度。
async fn cmd_download(target: &str) -> i32 {
    use std::io::Write as _;

    let id = match target {
        "bin" => kotone_stt::model::WHISPER_BIN_ID.to_string(),
        "tiny" | "base" | "small" => format!("ggml-{target}"),
        "zipformer" => "zipformer-bilingual-zh-en-2023-02-20".to_string(),
        other if other.starts_with("ggml-") || other.starts_with("zipformer-") => {
            other.to_string()
        }
        other => {
            eprintln!("未知下载目标：{other}（可选：bin / tiny / base / small / zipformer）");
            return 2;
        }
    };

    let id2 = id.clone();
    let result = tokio::task::spawn_blocking(move || {
        kotone_stt::model::download(&id2, &|done, total| {
            match total {
                Some(t) if t > 0 => {
                    print!("\r下载中 {id2} … {:5.1}%", done as f64 * 100.0 / t as f64)
                }
                _ => print!("\r下载中 {id2} … {done} 字节"),
            }
            let _ = std::io::stdout().flush();
        })
    })
    .await;

    match result {
        Ok(Ok(())) => {
            println!("\r下载完成：{id}                    ");
            0
        }
        Ok(Err(e)) => {
            println!("\r下载失败：{e}                    ");
            1
        }
        Err(e) => {
            println!("\r下载任务异常：{e}                    ");
            1
        }
    }
}

/// send：一次性注入。退出码 0 = 成功（INJECT_OK）；1 = 失败（INJECT_ERR）
fn cmd_send(text: &str, profile_id: &str, clipboard: bool, delay_ms: u64) -> i32 {
    use kotone_core::inject::{CancelToken, Injector};
    use kotone_platform_windows::inject::WindowsInjector;

    let mut profile = if profile_id == "generic" {
        GameProfile::builtin_generic()
    } else {
        match profile::get(profile_id) {
            Some(p) => p,
            None => {
                eprintln!("INJECT_ERR: profile 「{profile_id}」不存在");
                return 2;
            }
        }
    };
    if clipboard {
        profile.prefer_clipboard_paste = true;
    }
    if delay_ms > 0 {
        std::thread::sleep(std::time::Duration::from_millis(delay_ms));
    }

    match WindowsInjector.send(text, &profile, CancelToken::default()) {
        Ok(()) => {
            println!("INJECT_OK");
            0
        }
        Err(e) => {
            println!("INJECT_ERR: {}", e.message);
            1
        }
    }
}

/// listen 退出码：0 = 会话成功（Preview/Success），1 = 错误，2 = 中断/用法错误
///
/// 两种模式：
/// - 默认（热键模式）：LL 钩子热键 → orchestrator → JSONL 事件流，Ctrl+C 退出（=2）
/// - 会话模式（--wav / --no-hotkey）：立即开始一段会话，到时自动 finalize 退出，
///   无人值守自动化测试用（docs/cli.md）
#[cfg(windows)]
#[allow(clippy::too_many_arguments)]
async fn cmd_listen(
    engine: &str,
    profile_id: Option<String>,
    key: Option<String>,
    mode: Option<String>,
    wav: Option<String>,
    no_hotkey: bool,
    duration: Option<u64>,
    speed: Option<f64>,
) -> i32 {
    if wav.is_some() || no_hotkey {
        cmd_listen_session(engine, wav, duration, speed).await
    } else {
        cmd_listen_hotkey(engine, profile_id, key, mode).await
    }
}

/// 会话模式：跳过热键，begin → 等待（wav 时长或 --duration）→ end → 按终态给退出码。
/// wav 直灌用 WavFileBackend（强制预览收尾，不触发真实注入）；
/// --no-hotkey 无 wav 时用配置采集设备（可指向虚拟声卡 CABLE Output）。
#[cfg(windows)]
async fn cmd_listen_session(
    engine: &str,
    wav: Option<String>,
    duration: Option<u64>,
    speed: Option<f64>,
) -> i32 {
    use std::sync::{Arc, RwLock};

    use kotone_core::audio::AudioBackend;
    use kotone_core::orchestrator::{Emitter, Orchestrator, OrchestratorState};
    use kotone_platform_windows::audio::CpalBackend;
    use kotone_platform_windows::inject::{WinFocusBackend, WindowsInjector};
    use kotone_platform_windows::wav_audio::WavFileBackend;

    kotone_core::log::init();

    let mut settings = settings::load();
    settings.stt_engine = engine.to_string();
    let speed = speed.unwrap_or(1.0);

    // 音频后端与等待时长：wav 直灌按音频时长（/倍速），否则必须显式 --duration
    let (audio_backend, wait_ms): (Arc<dyn AudioBackend>, u64) = match &wav {
        Some(path) => {
            let backend = WavFileBackend::new(path, speed);
            let audio_ms = match backend.audio_ms() {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("读取 wav 失败: {e}");
                    return 1;
                }
            };
            // wav 模式强制预览收尾：无人值守场景绝不能触发真实注入
            settings.auto_send = false;
            let auto_wait = if speed > 0.0 {
                (audio_ms as f64 / speed) as u64 + 500
            } else {
                1000 // 全速喂入很快，finalize 耗时在 end() 内等待
            };
            (
                Arc::new(backend),
                duration.map(|d| d * 1000).unwrap_or(auto_wait),
            )
        }
        None => {
            let d = match duration {
                Some(d) => d * 1000,
                None => {
                    eprintln!("--no-hotkey 需配合 --duration <秒>（或使用 --wav <file>）");
                    return 2;
                }
            };
            (Arc::new(CpalBackend), d)
        }
    };

    let settings = Arc::new(RwLock::new(settings));
    let mut registry = EngineRegistry::new();
    kotone_stt::register_builtin(&mut registry);
    if registry.get(engine).is_none() {
        eprintln!("未注册的 STT 引擎: {engine}");
        return 2;
    }

    let emitter: Arc<dyn Emitter> = Arc::new(JsonlEmitter { hotkey: None });
    let orchestrator = Arc::new(Orchestrator::new(
        settings,
        Arc::new(registry),
        audio_backend,
        Arc::new(WindowsInjector),
        Arc::new(WinFocusBackend),
        emitter,
    ));

    if let Err(e) = orchestrator.begin().await {
        println!(
            "{}",
            serde_json::json!({ "event": "cli", "payload": { "result": "error", "message": e } })
        );
        return 1;
    }

    let interrupted = tokio::select! {
        _ = tokio::signal::ctrl_c() => true,
        _ = tokio::time::sleep(std::time::Duration::from_millis(wait_ms)) => false,
    };
    if interrupted {
        orchestrator.cancel().await;
        println!(
            "{}",
            serde_json::json!({ "event": "cli", "payload": { "result": "interrupted" } })
        );
        return 2;
    }

    if let Err(e) = orchestrator.end().await {
        println!(
            "{}",
            serde_json::json!({ "event": "cli", "payload": { "result": "error", "message": e } })
        );
        return 1;
    }
    let state = orchestrator.state();
    let ok = matches!(
        state,
        OrchestratorState::Preview | OrchestratorState::Success
    );
    println!(
        "{}",
        serde_json::json!({ "event": "cli", "payload": { "result": if ok { "ok" } else { "error" }, "state": format!("{state:?}") } })
    );
    if ok {
        0
    } else {
        1
    }
}

/// 热键模式：LL 钩子热键 → orchestrator → JSONL 事件流。
/// 证明 core + stt + platform 三个 crate 无 Tauri 可跑通全链路。Ctrl+C 退出（码 2）。
#[cfg(windows)]
async fn cmd_listen_hotkey(
    engine: &str,
    profile_id: Option<String>,
    key: Option<String>,
    mode: Option<String>,
) -> i32 {
    use std::sync::{Arc, RwLock};

    use kotone_core::hotkey::{HookEvent, HotkeyMode, HotkeySource};
    use kotone_core::orchestrator::{Emitter, Orchestrator};
    use kotone_platform_windows::audio::CpalBackend;
    use kotone_platform_windows::hotkey_ll::LlHookSource;
    use kotone_platform_windows::inject::{WinFocusBackend, WindowsInjector};

    kotone_core::log::init();

    // 配置值注入：schema/存储在 core，这里只读值并覆盖命令行参数
    let mut settings = settings::load();
    settings.stt_engine = engine.to_string();
    if let Some(p) = profile_id {
        settings.active_profile_id = Some(p);
    }
    let hotkey_key = key.unwrap_or_else(|| settings.hotkey.key.clone());
    let hotkey_mode = match mode.as_deref().unwrap_or("") {
        "hold" => HotkeyMode::Hold,
        "toggle" => HotkeyMode::Toggle,
        _ => settings.hotkey.mode,
    };
    let settings = Arc::new(RwLock::new(settings));

    let mut registry = EngineRegistry::new();
    kotone_stt::register_builtin(&mut registry);
    if registry.get(engine).is_none() {
        eprintln!("未注册的 STT 引擎: {engine}");
        return 2;
    }

    // 热键事件：sink → tokio channel → pump 任务调 orchestrator
    let (ev_tx, mut ev_rx) = tokio::sync::mpsc::unbounded_channel::<HookEvent>();
    let hotkey = Arc::new(LlHookSource::new(Box::new(move |ev| {
        let _ = ev_tx.send(ev);
    })));

    let emitter: Arc<dyn Emitter> = Arc::new(JsonlEmitter {
        hotkey: Some(hotkey.clone()),
    });
    let orchestrator = Arc::new(Orchestrator::new(
        settings,
        Arc::new(registry),
        Arc::new(CpalBackend),
        Arc::new(WindowsInjector),
        Arc::new(WinFocusBackend),
        emitter,
    ));

    if let Err(e) = hotkey.register(&hotkey_key, hotkey_mode) {
        eprintln!("注册热键失败: {e}");
        return 1;
    }
    println!(
        "{}",
        serde_json::json!({
            "event": "cli",
            "payload": {
                "message": "listen 已启动（Ctrl+C 退出）",
                "hotkey": hotkey_key,
                "mode": format!("{hotkey_mode:?}"),
                "backend": "llhook",
                "note": format!("hotkeyBackend 配置项当前为 {:?}", HotkeyBackend::Auto),
            }
        })
    );

    // 热键事件泵
    let pump = {
        let orch = orchestrator.clone();
        tokio::spawn(async move {
            while let Some(ev) = ev_rx.recv().await {
                match ev {
                    HookEvent::HoldPressed => orch.on_hotkey_hold(true).await,
                    HookEvent::HoldReleased => orch.on_hotkey_hold(false).await,
                    HookEvent::Toggle => orch.on_hotkey_toggle().await,
                    HookEvent::Cancel => orch.cancel().await,
                }
            }
        })
    };

    let _ = tokio::signal::ctrl_c().await;
    println!("{}", serde_json::json!({ "event": "cli", "payload": { "message": "退出（中断）" } }));
    pump.abort();
    hotkey.shutdown();
    orchestrator.cancel().await;
    2
}

/// JSONL 事件出口：全部 core 事件打印到 stdout；
/// 热键模式下联动热键源的 Esc 取消使能（state != idle 期间）
#[cfg(windows)]
struct JsonlEmitter {
    hotkey: Option<std::sync::Arc<kotone_platform_windows::hotkey_ll::LlHookSource>>,
}

#[cfg(windows)]
impl kotone_core::orchestrator::Emitter for JsonlEmitter {
    fn emit(&self, event: &str, payload: serde_json::Value) {
        println!("{}", serde_json::json!({ "event": event, "payload": payload }));
        if let Some(hotkey) = &self.hotkey {
            if event == "kotone://state" {
                let state = payload.get("state").and_then(|s| s.as_str()).unwrap_or("");
                hotkey.set_cancel_active(!state.is_empty() && state != "idle");
            }
        }
    }
}

#[cfg(not(windows))]
#[allow(clippy::too_many_arguments)]
async fn cmd_listen(
    _engine: &str,
    _profile_id: Option<String>,
    _key: Option<String>,
    _mode: Option<String>,
    _wav: Option<String>,
    _no_hotkey: bool,
    _duration: Option<u64>,
    _speed: Option<f64>,
) -> i32 {
    eprintln!("listen 子命令仅 Windows 支持（LL 钩子热键，MVP Windows-first）");
    1
}

// ---------- eval：录档列表 / 语料回放 / 人工标注 / 对比报告 ----------

/// 评测用引擎注册表（注入全部内置引擎；回放是 core 的无 GUI 消费场景）
fn eval_registry() -> std::sync::Arc<EngineRegistry> {
    let mut registry = EngineRegistry::new();
    kotone_stt::register_builtin(&mut registry);
    std::sync::Arc::new(registry)
}

fn truncate_chars(s: &str, max: usize) -> String {
    let mut it = s.chars();
    let taken: String = it.by_ref().take(max).collect();
    if it.next().is_some() {
        format!("{taken}…")
    } else {
        taken
    }
}

/// eval list：录档会话表（新→旧）
fn cmd_eval_list() -> i32 {
    match kotone_core::eval::list_sessions() {
        Ok(sessions) => {
            if sessions.is_empty() {
                println!("暂无录档会话（确认设置中 evalRecording 开启，并完成一次识别会话）");
                return 0;
            }
            println!(
                "{:<20} {:<26} {:>7} {:>9} {:>8} {:<4} {}",
                "会话 ID", "引擎", "音频 s", "partials", "最终 ms", "标注", "最终文本"
            );
            for s in &sessions {
                let audio_s = format!("{:.1}", s.audio_ms as f64 / 1000.0);
                let labeled = if s.human_label.is_some() { "✓" } else { "" };
                println!(
                    "{:<20} {:<26} {:>7} {:>9} {:>8} {:<4} {}",
                    s.session_id,
                    s.engine_id,
                    audio_s,
                    s.partials.len(),
                    s.final_ms,
                    labeled,
                    truncate_chars(&s.final_text, 24)
                );
            }
            println!(
                "\n共 {} 条（容量上限 {}，目录 ~/.kotone/eval/）",
                sessions.len(),
                kotone_core::eval::MAX_SESSIONS
            );
            0
        }
        Err(e) => {
            eprintln!("读取录档失败: {e}");
            1
        }
    }
}

fn print_replay_detail(r: &kotone_core::eval::EvalResult) {
    println!("回放完成：{} × {}", r.session_id, r.engine_id);
    println!(
        "首字延迟：{}   最终延迟：{}ms   CER：{}",
        r.first_partial_ms
            .map(|v| format!("{v}ms"))
            .unwrap_or_else(|| "—（非流式）".into()),
        r.final_ms,
        r.cer
            .map(|c| format!("{c:.3}"))
            .unwrap_or_else(|| "—（未标注）".into())
    );
    println!("最终文本：{}", r.final_text);
    if !r.partials.is_empty() {
        println!("partials：");
        for p in &r.partials {
            println!("  [{:>5}ms] {}", p.t, p.text);
        }
    }
}

/// eval replay：指定引擎单跑；缺省对全部就绪引擎跑对比表
async fn cmd_eval_replay(session_id: &str, engine: Option<String>) -> i32 {
    let registry = eval_registry();
    match engine {
        Some(engine_id) => {
            let reg = registry.clone();
            let sid = session_id.to_string();
            let r = tokio::task::spawn_blocking(move || {
                kotone_core::eval::replay(&sid, &engine_id, &reg)
            })
            .await;
            match r {
                Ok(Ok(result)) => {
                    print_replay_detail(&result);
                    0
                }
                Ok(Err(e)) => {
                    eprintln!("回放失败: {e}");
                    1
                }
                Err(e) => {
                    eprintln!("回放任务异常: {e}");
                    1
                }
            }
        }
        None => {
            let ready: Vec<_> = registry
                .list_info()
                .into_iter()
                .filter(|i| i.is_ready)
                .collect();
            if ready.is_empty() {
                eprintln!("没有就绪的引擎（先 kotone-cli download 安装模型）");
                return 1;
            }
            println!(
                "{:<26} {:>8} {:>8} {:>8}  {}",
                "引擎", "首字 ms", "最终 ms", "CER", "最终文本"
            );
            let mut failed = 0;
            for info in &ready {
                let reg = registry.clone();
                let sid = session_id.to_string();
                let eid = info.id.clone();
                let r = tokio::task::spawn_blocking(move || {
                    kotone_core::eval::replay(&sid, &eid, &reg)
                })
                .await;
                match r {
                    Ok(Ok(result)) => {
                        println!(
                            "{:<26} {:>8} {:>8} {:>8}  {}",
                            info.id,
                            result
                                .first_partial_ms
                                .map(|v| v.to_string())
                                .unwrap_or_else(|| "—".into()),
                            result.final_ms,
                            result
                                .cer
                                .map(|c| format!("{c:.3}"))
                                .unwrap_or_else(|| "—".into()),
                            truncate_chars(&result.final_text, 24)
                        );
                    }
                    Ok(Err(e)) => {
                        println!("{:<26}  回放失败: {e}", info.id);
                        failed += 1;
                    }
                    Err(e) => {
                        println!("{:<26}  回放任务异常: {e}", info.id);
                        failed += 1;
                    }
                }
            }
            if failed == ready.len() as i32 {
                1
            } else {
                0
            }
        }
    }
}

/// eval label：回填人工标注（正确文本），供 CER 计算
fn cmd_eval_label(session_id: &str, text: &str) -> i32 {
    match kotone_core::eval::label(session_id, text) {
        Ok(s) => {
            println!(
                "已标注 {}：「{}」（引擎识别：「{}」）",
                s.session_id, text, s.final_text
            );
            println!("提示：eval report 将对已标注会话计算各引擎 CER");
            0
        }
        Err(e) => {
            eprintln!("标注失败: {e}");
            1
        }
    }
}

/// eval report：已标注会话 × 就绪引擎的 CER / 延迟 Markdown 报告
async fn cmd_eval_report() -> i32 {
    let registry = eval_registry();
    let r = tokio::task::spawn_blocking(move || kotone_core::eval::report(&registry)).await;
    match r {
        Ok(Ok(md)) => {
            println!("{md}");
            0
        }
        Ok(Err(e)) => {
            eprintln!("报告生成失败: {e}");
            1
        }
        Err(e) => {
            eprintln!("报告任务异常: {e}");
            1
        }
    }
}

// ---------- config：配置管理（点路径写入，走 core settings 唯一写入口） ----------

/// config set 支持的键（点路径，对齐 config.json 的 camelCase 键名）
const CONFIG_SETTABLE_KEYS: &[&str] = &[
    "hotkey.key",
    "hotkey.mode",
    "hotkeyBackend",
    "sttEngine",
    "activeProfileId",
    "autoSend",
    "audioDeviceId",
    "language",
    "evalRecording",
    "runAsAdminOnStart",
    "interactionMode",
];

/// 点路径写入：current 上套 patch → Settings 反序列化校验（枚举值在此拦截）。
/// 纯逻辑可单测；文件 IO 在 cmd 包装层。
fn apply_config_set(current: &Settings, key: &str, raw: &str) -> Result<Settings, String> {
    if !CONFIG_SETTABLE_KEYS.contains(&key) {
        return Err(format!(
            "不支持的配置键「{key}」（支持：{}）",
            CONFIG_SETTABLE_KEYS.join(", ")
        ));
    }
    let value = match key {
        // 布尔键在命令行层先校验，给出清晰报错
        "autoSend" | "evalRecording" | "runAsAdminOnStart" => match raw {
            "true" => serde_json::Value::Bool(true),
            "false" => serde_json::Value::Bool(false),
            _ => return Err(format!("{key} 只接受 true/false（收到「{raw}」）")),
        },
        // 字符串与枚举键：原样写入，枚举由 Settings 反序列化校验
        _ => serde_json::Value::String(raw.to_string()),
    };
    let patch = match key.split_once('.') {
        Some((top, sub)) => serde_json::json!({ top: { sub: value } }),
        None => serde_json::json!({ key: value }),
    };
    let mut merged =
        serde_json::to_value(current).map_err(|e| format!("序列化当前配置失败: {e}"))?;
    settings::merge_json(&mut merged, &patch);
    serde_json::from_value(merged).map_err(|e| format!("值「{raw}」对 {key} 不合法: {e}"))
}

/// 点路径读取（只读，允许任意存在的路径）
fn config_get_value(settings: &Settings, key: &str) -> Result<serde_json::Value, String> {
    let root =
        serde_json::to_value(settings).map_err(|e| format!("序列化配置失败: {e}"))?;
    let mut cur = &root;
    for part in key.split('.') {
        cur = cur
            .get(part)
            .ok_or_else(|| format!("配置项「{key}」不存在"))?;
    }
    Ok(cur.clone())
}

fn cmd_config_show() -> i32 {
    let s = settings::load();
    match serde_json::to_string_pretty(&s) {
        Ok(j) => {
            println!("{j}");
            0
        }
        Err(e) => {
            eprintln!("序列化配置失败: {e}");
            1
        }
    }
}

fn cmd_config_get(key: &str) -> i32 {
    let s = settings::load();
    match config_get_value(&s, key) {
        Ok(v) => {
            match &v {
                serde_json::Value::String(s) => println!("{s}"),
                other => println!("{other}"),
            }
            0
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

/// 按键录入（--capture）：LL 钩子捕获下一个组合键，返回配置串（如 "Ctrl+Alt+V"）
#[cfg(windows)]
fn capture_hotkey_combo() -> Result<String, String> {
    use kotone_platform_windows::hotkey_ll::{CaptureResult, LlHookSource};

    let source = LlHookSource::new(Box::new(|_| {}));
    let (tx, rx) = std::sync::mpsc::channel::<CaptureResult>();
    source.capture_next(
        Box::new(move |r| {
            let _ = tx.send(r);
        }),
        std::time::Duration::from_secs(30),
    )?;
    eprintln!("请按下热键组合…（Esc 取消，30 秒超时）");
    match rx.recv().map_err(|_| "捕获通道异常断开".to_string())? {
        CaptureResult::Captured(spec) => Ok(spec.combo_name()),
        CaptureResult::Cancelled => Err("已取消录入".into()),
        CaptureResult::Timeout => Err("超时未按键".into()),
    }
}

#[cfg(not(windows))]
fn capture_hotkey_combo() -> Result<String, String> {
    Err("--capture 热键录入仅支持 Windows".into())
}

fn cmd_config_set(key: &str, value: Option<&str>, capture: bool) -> i32 {
    // --capture：按键录入（仅 hotkey.key；结果即写入值）
    if capture {
        if key != "hotkey.key" {
            eprintln!("--capture 仅支持 hotkey.key（收到「{key}」）");
            return 2;
        }
        let combo = match capture_hotkey_combo() {
            Ok(c) => c,
            Err(e) => {
                eprintln!("{e}");
                return 1;
            }
        };
        println!("捕获到组合键: {combo}");
        return cmd_config_set(key, Some(&combo), false);
    }
    let value = match value {
        Some(v) => v,
        None => {
            eprintln!("缺少 value 参数（录入 hotkey.key 可用 --capture）");
            return 2;
        }
    };
    let current = settings::load();
    let next = match apply_config_set(&current, key, value) {
        Ok(n) => n,
        Err(e) => {
            eprintln!("{e}");
            return 2;
        }
    };
    // sttEngine 额外校验：必须是已注册引擎（拼错引擎 id 是脚本高发错误）
    if key == "sttEngine" {
        let mut registry = EngineRegistry::new();
        kotone_stt::register_builtin(&mut registry);
        if registry.get(&next.stt_engine).is_none() {
            eprintln!("未注册的 STT 引擎: {}（未写入）", next.stt_engine);
            return 2;
        }
    }
    match settings::save(&next) {
        Ok(()) => {
            println!("已写入 {key} = {value}");
            0
        }
        Err(e) => {
            eprintln!("保存配置失败: {e}");
            1
        }
    }
}

// ---------- devices：音频设备枚举 ----------

/// 名称关键词判断虚拟声卡（VB-CABLE / Virtual Audio / 虚拟）
fn looks_virtual(name: &str) -> bool {
    let n = name.to_lowercase();
    n.contains("cable") || n.contains("virtual") || name.contains("虚拟")
}

fn print_device_line(kind: &str, d: &kotone_core::audio::AudioDevice) {
    let mut desc = String::new();
    if d.id == "default" {
        desc.push_str(&d.name);
        desc.push_str(" [默认]");
    } else if d.id != d.name {
        desc.push_str(&d.name);
    }
    if looks_virtual(&d.name) {
        desc.push_str(" [虚拟声卡]");
    }
    // 管道分隔：脚本可 `cut -d'|' -f2` 提取设备 id
    println!("{kind} | {} | {desc}", d.id);
}

fn cmd_devices() -> i32 {
    println!("== 音频输入（采集；audioDeviceId 用第 2 列）==");
    for d in kotone_platform_windows::audio::list_devices() {
        print_device_line("IN ", &d);
    }
    println!("== 音频输出（播放；play --device 用名称子串）==");
    for d in kotone_platform_windows::audio::list_output_devices() {
        print_device_line("OUT", &d);
    }
    0
}

// ---------- play：wav 播放到输出设备（虚拟声卡回路的关键一半） ----------

fn cmd_play(wav: &str, device: Option<String>) -> i32 {
    let path = std::path::PathBuf::from(wav);
    match kotone_platform_windows::playback::play_wav(&path, device.as_deref()) {
        Ok(()) => {
            println!("播放完成：{wav}");
            0
        }
        Err(e) => {
            eprintln!("播放失败: {e}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------- config 点路径 ----------

    #[test]
    fn config_set_hotkey_key() {
        let s = apply_config_set(&Settings::default(), "hotkey.key", "F9").unwrap();
        assert_eq!(s.hotkey.key, "F9");
        // 其余字段不受影响
        assert_eq!(s.hotkey.mode, kotone_core::hotkey::HotkeyMode::Toggle);
    }

    #[test]
    fn config_set_hotkey_mode_validates_enum() {
        let s = apply_config_set(&Settings::default(), "hotkey.mode", "hold").unwrap();
        assert_eq!(s.hotkey.mode, kotone_core::hotkey::HotkeyMode::Hold);
        assert!(apply_config_set(&Settings::default(), "hotkey.mode", "press").is_err());
    }

    #[test]
    fn config_set_hotkey_backend() {
        for (raw, expect) in [
            ("auto", HotkeyBackend::Auto),
            ("llhook", HotkeyBackend::Llhook),
            ("register", HotkeyBackend::Register),
        ] {
            let s = apply_config_set(&Settings::default(), "hotkeyBackend", raw).unwrap();
            assert_eq!(s.hotkey_backend, expect);
        }
        assert!(apply_config_set(&Settings::default(), "hotkeyBackend", "magic").is_err());
    }

    #[test]
    fn config_set_bool_keys() {
        let s = apply_config_set(&Settings::default(), "autoSend", "true").unwrap();
        assert!(s.auto_send);
        let s = apply_config_set(&Settings::default(), "evalRecording", "false").unwrap();
        assert!(!s.eval_recording);
        assert!(apply_config_set(&Settings::default(), "autoSend", "1").is_err());
        assert!(apply_config_set(&Settings::default(), "autoSend", "yes").is_err());
    }

    #[test]
    fn config_set_string_keys() {
        let s = apply_config_set(&Settings::default(), "sttEngine", "mock-stream").unwrap();
        assert_eq!(s.stt_engine, "mock-stream");
        let s = apply_config_set(&Settings::default(), "audioDeviceId", "CABLE Output").unwrap();
        assert_eq!(s.audio_device_id, "CABLE Output");
        let s = apply_config_set(&Settings::default(), "activeProfileId", "lol").unwrap();
        assert_eq!(s.active_profile_id.as_deref(), Some("lol"));
        let s = apply_config_set(&Settings::default(), "language", "en").unwrap();
        assert_eq!(s.language, "en");
    }

    #[test]
    fn config_set_rejects_unknown_key() {
        let e = apply_config_set(&Settings::default(), "no.such.key", "x").unwrap_err();
        assert!(e.contains("不支持的配置键"), "{e}");
        assert!(apply_config_set(&Settings::default(), "auto_send", "true").is_err());
    }

    #[test]
    fn config_set_interaction_mode() {
        use kotone_core::interaction::InteractionMode;
        let s = apply_config_set(&Settings::default(), "interactionMode", "push-to-talk").unwrap();
        assert_eq!(s.interaction_mode, Some(InteractionMode::PushToTalk));
        let s = apply_config_set(&Settings::default(), "interactionMode", "dictation").unwrap();
        assert_eq!(s.interaction_mode, Some(InteractionMode::Dictation));
        assert!(apply_config_set(&Settings::default(), "interactionMode", "magic").is_err());
    }

    #[test]
    fn config_get_dotted_path() {
        let s = Settings::default();
        assert_eq!(
            config_get_value(&s, "hotkey.key").unwrap(),
            serde_json::json!("F8")
        );
        assert_eq!(config_get_value(&s, "autoSend").unwrap(), serde_json::json!(false));
        assert_eq!(
            config_get_value(&s, "hotkey.mode").unwrap(),
            serde_json::json!("toggle")
        );
        assert!(config_get_value(&s, "hotkey.nosuch").is_err());
        assert!(config_get_value(&s, "nosuch").is_err());
    }

    // ---------- devices 虚拟声卡识别 ----------

    #[test]
    fn virtual_device_keyword_detection() {
        assert!(looks_virtual("CABLE Output (VB-Audio Virtual Cable)"));
        assert!(looks_virtual("CABLE Input (VB-Audio Virtual Cable)"));
        assert!(looks_virtual("VoiceMeeter Virtual Output"));
        assert!(looks_virtual("虚拟声卡驱动"));
        assert!(!looks_virtual("麦克风 (Realtek(R) Audio)"));
        assert!(!looks_virtual("Microphone (USB Audio Device)"));
    }

    // ---------- listen 会话模式退出码（wav 直灌 + mock 引擎全链路） ----------

    fn fixture_wav() -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../kotone-stt/tests/fixtures/zh-game-3s.wav")
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn listen_session_wav_mock_engine_exits_0() {
        // mock-stream：固定文本「对面打野在下路」，全速喂入 → Preview 收尾 → 0
        let code = cmd_listen_session(
            "mock-stream",
            Some(fixture_wav().to_string_lossy().into_owned()),
            None,
            Some(0.0),
        )
        .await;
        assert_eq!(code, 0);
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn listen_session_unknown_engine_exits_2() {
        let code = cmd_listen_session(
            "no-such-engine",
            Some(fixture_wav().to_string_lossy().into_owned()),
            None,
            Some(0.0),
        )
        .await;
        assert_eq!(code, 2);
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn listen_session_missing_wav_exits_1() {
        let code = cmd_listen_session(
            "mock-stream",
            Some("no/such/file.wav".into()),
            None,
            Some(0.0),
        )
        .await;
        assert_eq!(code, 1);
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn listen_session_no_hotkey_requires_duration() {
        let code = cmd_listen_session("mock-stream", None, None, None).await;
        assert_eq!(code, 2);
    }
}
