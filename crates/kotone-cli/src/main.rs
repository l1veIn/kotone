//! kotone-cli：无 Tauri 的命令行前端（验收 core 可独立运行的关键证据）。
//!
//! 子命令：
//! - `send`：一次性注入（取代原 src-tauri/examples/inject_cli.rs）
//! - `listen`：前台常驻全链路——LL 钩子热键 → orchestrator → 全部事件以 JSONL 打印
//! - `eval`：评测入口（stub，eval 模块未实现）

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
    /// 引擎评测（未实现）
    Eval {},
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
        Command::Eval {} => {
            println!("eval 子命令未实现（eval 模块签名就位，录档/回放/导出待做）");
            1
        }
    };
    std::process::exit(code);
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
