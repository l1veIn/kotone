//! kotone-cli：无 Tauri 的命令行前端（验收 core 可独立运行的关键证据）。
//!
//! 子命令：
//! - `send`：一次性注入（取代原 src-tauri/examples/inject_cli.rs）
//! - `listen`：前台常驻全链路——LL 钩子热键 → orchestrator → 全部事件以 JSONL 打印
//! - `download`：模型 / whisper-cli 运行时下载
//! - `eval`：引擎评测——录档列表 / 语料回放（多引擎对比）/ 人工标注 / CER 报告

use clap::{Parser, Subcommand};

use kotone_core::profile::{self, GameProfile};
use kotone_core::settings::{self, HotkeyBackend};
use kotone_core::stt::EngineRegistry;

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
    /// 前台常驻：LL 热键 → orchestrator → JSONL 打印全部事件（Ctrl+C 退出）
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
    },
    /// 下载模型或 whisper-cli 运行时：bin | tiny | base | small
    Download {
        /// 下载目标：bin（whisper-cli 运行时）或模型短名（tiny/base/small）
        target: String,
    },
    /// 引擎评测：录档列表 / 语料回放 / 人工标注 / 对比报告
    Eval {
        #[command(subcommand)]
        action: EvalCommand,
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
        } => cmd_listen(&engine, profile, key, mode).await,
        Command::Download { target } => cmd_download(&target).await,
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

/// listen：LL 钩子热键 → orchestrator → JSONL 事件流。
/// 证明 core + stt + platform 三个 crate 无 Tauri 可跑通全链路。
#[cfg(windows)]
async fn cmd_listen(
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

    /// JSONL 事件出口：全部 core 事件打印到 stdout；
    /// 同时驱动热键源的 Esc 取消使能（state != idle 期间）
    struct JsonlEmitter {
        hotkey: Arc<LlHookSource>,
    }
    impl Emitter for JsonlEmitter {
        fn emit(&self, event: &str, payload: serde_json::Value) {
            println!("{}", serde_json::json!({ "event": event, "payload": payload }));
            if event == "kotone://state" {
                let state = payload.get("state").and_then(|s| s.as_str()).unwrap_or("");
                self.hotkey
                    .set_cancel_active(!state.is_empty() && state != "idle");
            }
        }
    }

    let emitter: Arc<dyn Emitter> = Arc::new(JsonlEmitter {
        hotkey: hotkey.clone(),
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
    println!("{}", serde_json::json!({ "event": "cli", "payload": { "message": "退出" } }));
    pump.abort();
    hotkey.shutdown();
    orchestrator.cancel().await;
    0
}

#[cfg(not(windows))]
async fn cmd_listen(
    _engine: &str,
    _profile_id: Option<String>,
    _key: Option<String>,
    _mode: Option<String>,
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
