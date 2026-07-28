//! kotone-cli：无 Tauri 的命令行前端（验收 core 可独立运行的关键证据）。
//!
//! 子命令（详见 docs/cli.md）：
//! - `send`：一次性注入（取代原 src-tauri/examples/inject_cli.rs）
//! - `listen`：热键全链路 JSONL；--wav / --no-hotkey 单次会话模式（自动化测试）
//! - `download`：模型下载（清单内任意模型 id，镜像策略见 config download.source）
//! - `config`：show / get / set（点路径写入 ~/.kotone/config.json）
//! - `devices` / `play`：设备枚举 / wav 播放（虚拟声卡回路）
//! - `eval`：引擎评测——录档列表 / 语料回放（多引擎对比）/ 人工标注 / CER 报告
//! - `doctor`：环境自检（设备/引擎/profile/提权/VAD/history，逐项 ✓/⚠/✗）
//! - `elevate <command> [args...]`：sudo 式——以管理员权限执行子命令（UIPI 提权，§10 R-1）
//! - `profile`：list / use / detect（游戏 profile 管理与前台匹配）
//! - `log`：识别历史 list / clear（core history 模块的 CLI 出口）

use clap::{Parser, Subcommand};

use kotone_core::profile::{self, GameProfile};
use kotone_core::settings::{self, HotkeyBackend, Settings};
use kotone_core::stt::EngineRegistry;
#[cfg(windows)]
use kotone_core::hotkey::HotkeySource;

/// wav 直灌会话模式的注入器（ADR-007）：不碰真实窗口——
/// one-shot（C1 直发）在无人值守测试里也绝不能触发真实注入，
/// 注入结果以 JSONL 打印供断言
#[cfg(windows)]
struct NullInjector;

#[cfg(windows)]
impl kotone_core::inject::Injector for NullInjector {
    fn send(
        &self,
        text: &str,
        _profile: &GameProfile,
        _cancel: kotone_core::inject::CancelToken,
    ) -> Result<(), kotone_core::inject::InjectError> {
        println!(
            "{}",
            serde_json::json!({ "event": "cli", "payload": { "inject": "null", "text": text } })
        );
        Ok(())
    }
}

/// VAD 接线（ADR-007）：vad-silero feature 编译进时给 orchestrator 注入
/// silero 工厂；feature 关闭时原样收尾（one-shot begin 会报清晰错误）
#[cfg(windows)]
fn wire_vad(
    #[allow(unused_mut)] mut orchestrator: kotone_core::orchestrator::Orchestrator,
) -> std::sync::Arc<kotone_core::orchestrator::Orchestrator> {
    #[cfg(feature = "vad-silero")]
    {
        orchestrator.vad_factory = Some(kotone_stt::vad::silero_factory());
    }
    orchestrator.into_arc()
}

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
        /// 热键（缺省用配置文件值，如 CapsLock / Alt+V）
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
    /// 下载模型（清单内任意模型 id，如 x-asr-480ms-streaming-zh-en-punct-int8-2026-06-05 / silero-vad）
    Download {
        /// 下载目标：清单内任意模型 id（kotone-cli doctor / 引擎页可见列表）
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
    /// 环境自检：音频设备 / 引擎就绪 / 激活 profile / 提权链路 / VAD / history，
    /// 逐项 ✓/⚠/✗ 并给修复建议；有 ✗ 项时退出码 1
    Doctor,
    /// sudo 式提权：以管理员权限在新控制台执行子命令（UIPI：目标游戏提权
    /// 运行时注入必需，§10 R-1）。典型：kotone-cli elevate listen
    Elevate {
        /// 提权执行的子命令与全部参数（如 listen --engine mock-stream）
        #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// 游戏 profile：list 列表 / use 激活 / detect 前台进程匹配
    Profile {
        #[command(subcommand)]
        action: ProfileCommand,
    },
    /// 识别历史：list 列表 / clear 清空（~/.kotone/history/）
    Log {
        #[command(subcommand)]
        action: LogCommand,
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
    /// evalRecording / runAsAdminOnStart / interactionMode / vadSilenceMs /
    /// history.mode / history.maxRecords / history.includeAudio）
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

#[derive(Subcommand)]
enum ProfileCommand {
    /// 列出全部 profile（激活项标 *）
    List,
    /// 激活指定 profile（写入 activeProfileId）
    Use {
        /// profile id（profile list 可查）
        id: String,
    },
    /// 检测当前前台进程命中的 profile（调试匹配规则用）
    Detect,
}

#[derive(Subcommand)]
enum LogCommand {
    /// 列出识别历史（新→旧）
    List {
        /// 最多显示条数
        #[arg(long, default_value_t = 50)]
        limit: usize,
        /// 以 JSON 数组输出（脚本用）
        #[arg(long)]
        json: bool,
    },
    /// 清空全部历史记录（含音频）
    Clear {
        /// 跳过确认提示
        #[arg(long)]
        yes: bool,
    },
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
        Command::Doctor => cmd_doctor(),
        Command::Elevate { args } => cmd_elevate(&args),
        Command::Profile { action } => match action {
            ProfileCommand::List => cmd_profile_list(),
            ProfileCommand::Use { id } => cmd_profile_use(&id),
            ProfileCommand::Detect => cmd_profile_detect(),
        },
        Command::Log { action } => match action {
            LogCommand::List { limit, json } => cmd_log_list(limit, json),
            LogCommand::Clear { yes } => cmd_log_clear(yes),
        },
    };
    std::process::exit(code);
}

/// download：清单内任意模型 id 透传（x-asr / sense-voice / funasr / silero-vad 等）。
/// 单行刷新进度；下载源镜像策略由 settings.download 决定（model.rs 内部读取）。
async fn cmd_download(target: &str) -> i32 {
    use std::io::Write as _;

    let id = match kotone_stt::model::list() {
        Ok(list) if list.iter().any(|m| m.id == target) => target.to_string(),
        _ => {
            eprintln!("未知下载目标：{target}（可选：清单内任意模型 id，见引擎页或 docs/eval-playbook.md）");
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
        cmd_listen_session(engine, profile_id, wav, duration, speed).await
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
    profile_id: Option<String>,
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
    if let Some(profile_id) = profile_id {
        settings.active_profile_id = Some(profile_id);
    }
    let speed = speed.unwrap_or(1.0);

    // 音频后端 / 注入器 / 等待时长：wav 直灌按音频时长（/倍速）+ NullInjector
    // （one-shot C1 直发也不碰真实窗口）；否则用配置采集设备 + 真实注入器
    let (audio_backend, injector, wait_ms): (
        Arc<dyn AudioBackend>,
        Arc<dyn kotone_core::inject::Injector>,
        u64,
    ) = match &wav {
        Some(path) => {
            let backend = WavFileBackend::new(path, speed);
            let audio_ms = match backend.audio_ms() {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("读取 wav 失败: {e}");
                    return 1;
                }
            };
            // wav 模式非 one-shot 时强制预览收尾（auto_send=false）；
            // one-shot 预设的 C1 直发由 NullInjector 安全承接（JSONL 打印）
            settings.auto_send = false;
            let auto_wait = if speed > 0.0 {
                (audio_ms as f64 / speed) as u64 + 500
            } else {
                1000 // 全速喂入很快，finalize 耗时在 end() 内等待
            };
            (
                Arc::new(backend),
                Arc::new(NullInjector),
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
            (Arc::new(CpalBackend), Arc::new(WindowsInjector), d)
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
    let orchestrator = wire_vad(Orchestrator::new(
        settings,
        Arc::new(registry),
        audio_backend,
        injector,
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

    // 等会话收尾：VAD 判停（one-shot：pump 自己触发 end()，状态离开 Listening）
    // 或播完/超时后手动 end；Transcribing/Sending 是过渡态，继续等终态。
    // core 的 finalize 10s 超时兜底，终态等待给 15s 余量
    let wait_deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(wait_ms);
    let final_deadline = wait_deadline + std::time::Duration::from_secs(15);
    let mut manual_end_needed = false;
    let interrupted = tokio::select! {
        _ = tokio::signal::ctrl_c() => true,
        _ = async {
            loop {
                match orchestrator.state() {
                    OrchestratorState::Listening => {
                        if tokio::time::Instant::now() >= wait_deadline {
                            manual_end_needed = true;
                            break;
                        }
                    }
                    OrchestratorState::Transcribing | OrchestratorState::Sending => {
                        if tokio::time::Instant::now() >= final_deadline {
                            break; // 异常兜底（core finalize 超时会落 Error）
                        }
                    }
                    _ => break, // Preview / Success / Error（或 toast 后 Idle）
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        } => false,
    };
    if interrupted {
        orchestrator.cancel().await;
        println!(
            "{}",
            serde_json::json!({ "event": "cli", "payload": { "result": "interrupted" } })
        );
        return 2;
    }

    if manual_end_needed && orchestrator.state() == OrchestratorState::Listening {
        if let Err(e) = orchestrator.end().await {
            // VAD 判停与手动 end 竞态：状态已离开 Listening 说明 VAD 先赢了，走终态等待
            if orchestrator.state() == OrchestratorState::Listening {
                println!(
                    "{}",
                    serde_json::json!({ "event": "cli", "payload": { "result": "error", "message": e } })
                );
                return 1;
            }
        }
    }
    // 手动 end（或竞态落入 VAD 路径）后等终态（Sending → Success；finalize 进行中）
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(15);
    while matches!(
        orchestrator.state(),
        OrchestratorState::Transcribing | OrchestratorState::Sending
    ) && tokio::time::Instant::now() < deadline
    {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
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
        // 未显式指定时按 interactionMode 预设推导生效模式（与壳同一源）
        _ => kotone_core::interaction::effective_hotkey_mode(&settings),
    };

    // 提权预检（只警告不阻断）：激活 profile 的目标进程已提权而自身未提权时，
    // 注入会被 UIPI 整体丢弃——启动即提示（完整链路见 doctor，修复用 elevate）
    {
        use kotone_platform_windows::elevation;
        let profiles = profile::list();
        if let Some(pid) = elevation::resolve_active_game_pid(
            settings.active_profile_id.as_deref(),
            &profiles,
            &mut kotone_platform_windows::inject::find_pid_by_name,
        ) {
            if elevation::decide_needs_elevation(
                elevation::is_process_elevated(pid),
                elevation::is_elevated(),
            ) {
                eprintln!(
                    "⚠ 目标游戏进程（pid {pid}）以管理员权限运行而 Kotone 未提权：\
                     注入将被 UIPI 丢弃 → kotone-cli elevate listen（或以管理员身份重开终端）"
                );
            }
        }
    }
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
    let orchestrator = wire_vad(Orchestrator::new(
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
    "vadSilenceMs",
    "history.mode",
    "history.maxRecords",
    "history.includeAudio",
    "download.source",
    "download.ghProxy",
    "overlay.visibility",
    "overlay.style",
    "overlay.position",
    "overlay.draggable",
    "overlay.clickThrough",
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
        "autoSend"
        | "evalRecording"
        | "runAsAdminOnStart"
        | "history.includeAudio"
        | "overlay.draggable"
        | "overlay.clickThrough" => match raw {
            "true" => serde_json::Value::Bool(true),
            "false" => serde_json::Value::Bool(false),
            _ => return Err(format!("{key} 只接受 true/false（收到「{raw}」）")),
        },
        // 数值键（VAD 判停阈值，范围对齐 core vad::SILENCE_MS_RANGE）
        "vadSilenceMs" => match raw.parse::<u32>() {
            Ok(v) if (200..=5_000).contains(&v) => serde_json::Value::Number(v.into()),
            _ => {
                return Err(format!(
                    "{key} 只接受 200-5000 的整数毫秒（收到「{raw}」）"
                ))
            }
        },
        // 数值键（history capped 容量上限）
        "history.maxRecords" => match raw.parse::<u32>() {
            Ok(v) if (1..=100_000).contains(&v) => serde_json::Value::Number(v.into()),
            _ => {
                return Err(format!(
                    "{key} 只接受 1-100000 的整数条数（收到「{raw}」）"
                ))
            }
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

// ---------- doctor：环境自检（逐项 ✓/⚠/✗ + 修复建议；有 ✗ 退出码 1） ----------

fn cmd_doctor() -> i32 {
    let settings = settings::load();
    let mut failures = 0u32;

    // 1. 音频输入设备
    let inputs = kotone_platform_windows::audio::list_devices();
    if inputs.is_empty() {
        println!("✗ 音频输入设备：未枚举到任何采集设备（检查麦克风/驱动）");
        failures += 1;
    } else {
        let default = inputs
            .iter()
            .find(|d| d.id == "default")
            .map(|d| d.name.as_str())
            .unwrap_or("未知");
        println!("✓ 音频输入设备：{} 个（默认：{default}）", inputs.len());
    }
    for d in inputs.iter().filter(|d| looks_virtual(&d.name)) {
        println!("  ⚠ 虚拟声卡：{}（自动化测试路径二可用，日常采集别选它）", d.name);
    }

    // 2. STT 引擎就绪
    let mut registry = EngineRegistry::new();
    kotone_stt::register_builtin(&mut registry);
    match registry.get(&settings.stt_engine) {
        Some(e) if e.is_ready() => {
            println!("✓ STT 引擎「{}」就绪", settings.stt_engine);
        }
        Some(e) => {
            println!(
                "✗ STT 引擎「{}」（{}）未就绪：模型/二进制未下载 → kotone-cli download <模型>",
                settings.stt_engine,
                e.display_name()
            );
            failures += 1;
        }
        None => {
            println!(
                "✗ STT 引擎「{}」未注册（检查 sttEngine 拼写；已注册：{}）",
                settings.stt_engine,
                registry
                    .list_info()
                    .iter()
                    .map(|i| i.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            failures += 1;
        }
    }

    // 3. 激活 profile
    match settings.active_profile_id.as_deref() {
        Some(id) => match profile::get(id) {
            Some(p) => {
                let procs = if p.process_names.is_empty() {
                    "通配任意前台窗口".to_string()
                } else {
                    format!("processNames: {}", p.process_names.join(", "))
                };
                println!("✓ 激活 profile「{}」：{}（{procs}）", p.id, p.display_name);
            }
            None => {
                println!("✗ 激活 profile「{id}」不存在 → kotone-cli profile use <id>");
                failures += 1;
            }
        },
        None => println!("⚠ 未设置 activeProfileId（按内置 generic 通配；profile use <id> 激活）"),
    }

    // 4. 提权链路（UIPI §10 R-1）：目标进程提权 + 自身未提权 → 注入会被丢弃
    {
        use kotone_platform_windows::elevation;
        let self_elevated = elevation::is_elevated();
        let profiles = profile::list();
        let pid = elevation::resolve_active_game_pid(
            settings.active_profile_id.as_deref(),
            &profiles,
            &mut kotone_platform_windows::inject::find_pid_by_name,
        );
        match pid {
            Some(pid) => {
                let target = elevation::is_process_elevated(pid);
                if elevation::decide_needs_elevation(target, self_elevated) {
                    println!(
                        "✗ 目标游戏进程（pid {pid}）已提权而 Kotone 未提权：注入将被 UIPI 丢弃 → kotone-cli elevate listen（或以管理员身份重开终端）"
                    );
                    failures += 1;
                } else if target == Some(true) {
                    println!("✓ 提权链路：目标进程（pid {pid}）已提权，Kotone 同为管理员");
                } else {
                    println!("✓ 提权链路：目标进程（pid {pid}）未提权，无需 elevate");
                }
            }
            None => println!(
                "⚠ 提权链路：激活 profile 的目标进程未运行，无法判断（游戏启动后用 doctor 复查）"
            ),
        }
    }

    // 5. VAD 模型（one-shot 静音判停，ADR-007）
    if kotone_stt::model::vad_model_ready() {
        println!("✓ VAD 模型就绪（one-shot 静音判停可用）");
    } else {
        println!("⚠ VAD 模型未就绪：one-shot / solo 模式不可用（push-to-talk / dictation 不受影响）");
    }

    // 6. 录档与历史配置摘要
    println!(
        "{} evalRecording：{}（评测录档 → kotone-cli eval list）",
        if settings.eval_recording { "✓" } else { "⚠" },
        if settings.eval_recording { "开" } else { "关" }
    );
    let dl = &settings.download;
    let dl_desc = match dl.source {
        settings::DownloadSource::Auto => {
            format!("auto（镜像优先 hf-mirror.com + {}，失败回退官方）", dl.gh_proxy)
        }
        settings::DownloadSource::Official => "official（huggingface.co / github.com 直连）".into(),
        settings::DownloadSource::Mirror => {
            format!("mirror（仅镜像 hf-mirror.com + {}，不回退）", dl.gh_proxy)
        }
    };
    println!("✓ 下载源：{dl_desc}（kotone-cli config set download.source <auto|official|mirror>）");
    let h = &settings.history;
    let mode = match h.mode {
        kotone_core::history::HistoryMode::Capped => format!("capped（上限 {} 条）", h.max_records),
        kotone_core::history::HistoryMode::KeepAll => "keep-all（不裁剪）".to_string(),
        kotone_core::history::HistoryMode::Off => "off（不记录）".to_string(),
    };
    println!(
        "✓ history：{mode}，{}音频（kotone-cli log list 查看）",
        if h.include_audio { "含" } else { "不含" }
    );

    if failures > 0 {
        println!("\n{failures} 项未通过，按上方建议修复后复查");
        1
    } else {
        println!("\n全部关键项通过");
        0
    }
}

// ---------- elevate：sudo 式提权执行子命令（§10 R-1） ----------

fn cmd_elevate(args: &[String]) -> i32 {
    if kotone_platform_windows::elevation::is_elevated() {
        println!("已是管理员权限，无需提权");
        return 0;
    }
    // 提权副本在新控制台窗口执行给定子命令（典型 kotone-cli elevate listen），
    // 当前进程退出（main 的 exit(0)）
    match kotone_platform_windows::elevation::run_elevated(args) {
        Ok(()) => {
            println!(
                "已发起管理员执行「{}」（UAC 确认后在新控制台运行），当前进程退出",
                args.join(" ")
            );
            0
        }
        Err(e) => {
            eprintln!("提权失败: {e}");
            1
        }
    }
}

// ---------- profile：list / use / detect ----------

fn cmd_profile_list() -> i32 {
    let settings = settings::load();
    let profiles = profile::list();
    if profiles.is_empty() {
        println!("暂无 profile（~/.kotone/profiles/ 为空；内置 lol/generic 会在首次运行时落盘）");
        return 0;
    }
    for p in &profiles {
        let active = if settings.active_profile_id.as_deref() == Some(p.id.as_str()) {
            "*"
        } else {
            " "
        };
        let procs = if p.process_names.is_empty() {
            "通配任意前台窗口".to_string()
        } else {
            p.process_names.join(", ")
        };
        println!("{active} {:<10} {:<24} {procs}", p.id, p.display_name);
    }
    println!("\n* = 当前激活（config set activeProfileId / profile use 切换）");
    0
}

fn cmd_profile_use(id: &str) -> i32 {
    if profile::get(id).is_none() {
        eprintln!("profile「{id}」不存在（kotone-cli profile list 查看可用 id）");
        return 2;
    }
    cmd_config_set("activeProfileId", Some(id), false)
}

fn cmd_profile_detect() -> i32 {
    let name = match kotone_platform_windows::inject::foreground_process_name() {
        Some(n) => n,
        None => {
            eprintln!("无法读取前台进程（非 Windows 或无前台窗口）");
            return 1;
        }
    };
    println!("前台进程：{name}");
    match profile::find_by_process(&profile::list(), &name) {
        Some(p) => {
            println!("命中 profile「{}」：{}", p.id, p.display_name);
            0
        }
        None => {
            println!("未命中任何 profile（将按内置 generic 通配处理）");
            0
        }
    }
}

// ---------- log：识别历史 list / clear ----------

fn cmd_log_list(limit: usize, json: bool) -> i32 {
    let records = match kotone_core::history::list() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("读取历史失败: {e}");
            return 1;
        }
    };
    if json {
        let shown: Vec<_> = records.iter().take(limit).collect();
        match serde_json::to_string_pretty(&shown) {
            Ok(j) => println!("{j}"),
            Err(e) => {
                eprintln!("序列化失败: {e}");
                return 1;
            }
        }
        return 0;
    }
    if records.is_empty() {
        println!("暂无识别历史（history.mode 非 off 时，每次会话终态自动记录）");
        return 0;
    }
    println!(
        "{:<22} {:<26} {:>7} {:<10} {}",
        "时间", "引擎", "音频 s", "结局", "最终文本"
    );
    for r in records.iter().take(limit) {
        let outcome = match r.outcome {
            kotone_core::history::HistoryOutcome::Sent => "sent",
            kotone_core::history::HistoryOutcome::Cancelled => "cancelled",
            kotone_core::history::HistoryOutcome::Error => "error",
        };
        println!(
            "{:<22} {:<26} {:>7.1} {:<10} {}",
            r.ts,
            r.engine_id,
            r.audio_ms as f64 / 1000.0,
            outcome,
            truncate_chars(&r.final_text, 20)
        );
    }
    println!("\n共 {} 条（显示前 {limit} 条，--json 输出完整字段）", records.len());
    0
}

fn cmd_log_clear(yes: bool) -> i32 {
    if !yes {
        eprint!("确认清空全部识别历史（含音频文件）？[y/N] ");
        let mut line = String::new();
        if std::io::stdin().read_line(&mut line).is_err() {
            eprintln!("\n读取确认失败，已取消");
            return 2;
        }
        if !matches!(line.trim().to_lowercase().as_str(), "y" | "yes") {
            println!("已取消");
            return 0;
        }
    }
    match kotone_core::history::clear() {
        Ok(()) => {
            println!("已清空识别历史（~/.kotone/history/）");
            0
        }
        Err(e) => {
            eprintln!("清空失败: {e}");
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
        let s = apply_config_set(&Settings::default(), "overlay.draggable", "false").unwrap();
        assert!(!s.overlay.draggable);
        let s = apply_config_set(&Settings::default(), "overlay.clickThrough", "true").unwrap();
        assert!(s.overlay.click_through);
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
        let s = apply_config_set(&Settings::default(), "overlay.position", "bottom_right").unwrap();
        assert_eq!(
            s.overlay.position,
            kotone_core::settings::OverlayPosition::BottomRight
        );
        assert!(apply_config_set(&Settings::default(), "overlay.position", "outside").is_err());
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
        let s = apply_config_set(&Settings::default(), "interactionMode", "one-shot").unwrap();
        assert_eq!(s.interaction_mode, Some(InteractionMode::OneShot));
        let s = apply_config_set(&Settings::default(), "interactionMode", "solo").unwrap();
        assert_eq!(s.interaction_mode, Some(InteractionMode::Solo));
        assert!(apply_config_set(&Settings::default(), "interactionMode", "magic").is_err());
    }

    #[test]
    fn config_set_vad_silence_ms_numeric() {
        let s = apply_config_set(&Settings::default(), "vadSilenceMs", "900").unwrap();
        assert_eq!(s.vad_silence_ms, 900);
        assert!(apply_config_set(&Settings::default(), "vadSilenceMs", "abc").is_err());
        assert!(apply_config_set(&Settings::default(), "vadSilenceMs", "50").is_err());
        assert!(apply_config_set(&Settings::default(), "vadSilenceMs", "99999").is_err());
    }

    // ---------- elevate sudo 式参数解析 ----------

    #[test]
    fn elevate_requires_subcommand_args() {
        // 裸 elevate：clap 报错（required），不静默重启
        assert!(Cli::try_parse_from(["kotone-cli", "elevate"]).is_err());
    }

    #[test]
    fn elevate_collects_trailing_args_verbatim() {
        let cli = Cli::try_parse_from(["kotone-cli", "elevate", "listen"]).unwrap();
        match cli.command {
            Command::Elevate { args } => assert_eq!(args, ["listen"]),
            _ => panic!("expect Elevate"),
        }
        // 带选项与值的子命令整体透传（allow_hyphen_values 吃掉 --engine 等）
        let cli = Cli::try_parse_from([
            "kotone-cli",
            "elevate",
            "listen",
            "--engine",
            "mock-stream",
            "--profile",
            "lol oce",
        ])
        .unwrap();
        match cli.command {
            Command::Elevate { args } => {
                assert_eq!(args, ["listen", "--engine", "mock-stream", "--profile", "lol oce"])
            }
            _ => panic!("expect Elevate"),
        }
    }

    #[test]
    fn config_set_history_keys() {        use kotone_core::history::HistoryMode;
        // mode：字符串键，枚举值由 Settings 反序列化校验
        let s = apply_config_set(&Settings::default(), "history.mode", "keep-all").unwrap();
        assert_eq!(s.history.mode, HistoryMode::KeepAll);
        let s = apply_config_set(&Settings::default(), "history.mode", "off").unwrap();
        assert_eq!(s.history.mode, HistoryMode::Off);
        let s = apply_config_set(&Settings::default(), "history.mode", "capped").unwrap();
        assert_eq!(s.history.mode, HistoryMode::Capped);
        assert!(apply_config_set(&Settings::default(), "history.mode", "magic").is_err());
        // maxRecords：数值键，范围 1-100000
        let s = apply_config_set(&Settings::default(), "history.maxRecords", "500").unwrap();
        assert_eq!(s.history.max_records, 500);
        assert!(apply_config_set(&Settings::default(), "history.maxRecords", "0").is_err());
        assert!(apply_config_set(&Settings::default(), "history.maxRecords", "abc").is_err());
        assert!(apply_config_set(&Settings::default(), "history.maxRecords", "100001").is_err());
        // includeAudio：布尔键
        let s = apply_config_set(&Settings::default(), "history.includeAudio", "true").unwrap();
        assert!(s.history.include_audio);
        assert!(apply_config_set(&Settings::default(), "history.includeAudio", "yes").is_err());
        // 其余 history 字段不受单项写入影响
        let s = apply_config_set(&Settings::default(), "history.mode", "off").unwrap();
        assert_eq!(s.history.max_records, 1000);
        assert!(!s.history.include_audio);
    }

    #[test]
    fn config_get_dotted_path() {
        let s = Settings::default();
        assert_eq!(
            config_get_value(&s, "hotkey.key").unwrap(),
            serde_json::json!("CapsLock")
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
            None,
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
            None,
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
            None,
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
        let code = cmd_listen_session("mock-stream", None, None, None, None).await;
        assert_eq!(code, 2);
    }
}
