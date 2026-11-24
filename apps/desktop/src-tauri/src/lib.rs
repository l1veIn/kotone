//! Kotone Rust 核心：模块组装、共享状态与 IPC 命令。
//! 职责划分见 docs/development.md §5.1；IPC 契约见 §5.3（类型对齐 src/lib/ipc.ts）。

mod agent;
mod assistant;
mod assistant_input;
mod automation;
mod browser;
mod browser_cmd;
mod bubble;
mod call;
mod companion;
mod compatibility;
mod computer_use;
mod connections_cmd;
mod diagnostics;
mod diagnostics_cmd;
mod events;
mod fillers;
mod follow_up;
mod game_session;
mod game_tools;
mod games;
mod history_cmd;
mod hotkey;
mod key_request;
mod model_catalog;
mod models_cmd;
mod music;
mod music_cmd;
mod ocr_engines;
mod overlay;
mod overlay_layer;
mod overlay_window;
mod plugins;
mod plugins_cmd;
mod profiles_cmd;
mod recap;
mod roles;
mod runtime;
mod runtime_cmd;
mod settings_cmd;
mod speech_models;
mod speech_screen;
mod startup;
mod stop;
mod system_cmd;
mod tool_config;
mod tool_service;
mod tray;
mod tts_online;
mod usage;
mod voice_ask;
mod voice_chat;
mod voice_listen;
mod voice_reply;
mod voice_timing;
mod wake;
mod web_tools;
mod window;
mod workspace_cmd;

pub(crate) use connections_cmd::*;
pub(crate) use diagnostics_cmd::*;
pub(crate) use events::*;
pub(crate) use history_cmd::*;
pub(crate) use models_cmd::*;
pub(crate) use overlay_window::*;
pub(crate) use profiles_cmd::*;
pub(crate) use runtime_cmd::*;
pub(crate) use settings_cmd::*;
pub(crate) use startup::*;
pub(crate) use system_cmd::*;
pub(crate) use window::*;

use std::sync::{Arc, Mutex, RwLock};

use tauri::{AppHandle, Emitter as TauriEventEmitter, Manager};

use hotkey::{HotkeyManager, HotkeyStatus, InputEnvironmentCheck};
use kotone_core::audio::AudioDevice;
use kotone_core::connection::{Connection, ConnectionResolver, SecretStore};
use kotone_core::inject::{CancelToken, FocusBackend, InjectError, Injector};
use kotone_core::interaction::effective_hotkey_mode;
use kotone_core::orchestrator::{Emitter, Orchestrator, OrchestratorState};
use kotone_core::profile::{
    self, format_hotwords_export, GameProfile, HotwordMergeReport, ProfileDeleteOutcome,
};
use kotone_core::runtime::RuntimePhase;
use kotone_core::settings::{
    self, OverlayConfig, OverlayPosition, OverlayStyle, OverlayVisibility, Settings,
    SettingsRepository,
};
use kotone_core::stt::{EngineInfo, EngineRegistry};
use kotone_core::{log, process_log};
use kotone_platform_windows::inject::{WinFocusBackend, WindowsInjector};
use kotone_platform_windows::{
    audio as platform_audio, autostart, elevation, fullscreen, playback,
};
use kotone_stt::model;
use runtime::{RuntimeManager, RuntimeStatus};

/// 全局共享状态：settings 双端共享，orchestrator 是唯一业务状态所有者
pub struct SharedState {
    pub settings: Arc<RwLock<Settings>>,
    settings_repository: Arc<SettingsRepository>,
    settings_load_warning: Mutex<Option<String>>,
    pub orchestrator: Arc<Orchestrator>,
    pub engines: Arc<EngineRegistry>,
    pub processors: Arc<kotone_core::postprocess::ProcessorRegistry>,
    pub connections: Arc<dyn ConnectionResolver>,
    pub secrets: Arc<dyn SecretStore>,
    pub injector: Arc<dyn Injector>,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    browser::forward_runtime_log();
    // 见函数注释：自启动提权必须先于单实例插件的互斥锁创建。
    #[cfg(windows)]
    auto_elevate_before_tauri_init();

    let startup_options = StartupOptions {
        onboarding: parse_onboarding_launch_mode(std::env::args()),
        background: has_background_flag(std::env::args()),
    };
    // 隔离实例（设了 KOTONE_HOME：开发、测试、外部 Agent 的独立实例）不参与单实例：
    // 否则正式安装在运行时，隔离实例一启动就把正式实例的窗口唤到前台然后退出。
    // 隔离实例的配置要自己关掉热键，免得和正式实例抢按键（见 scripts/manual-e2e/README.md）。
    let mut builder = tauri::Builder::default();
    if std::env::var_os("KOTONE_HOME").is_none() {
        builder = builder
            // 单实例：第二实例启动时唤起已有实例的设置窗口并退出自身，
            // 避免旧进程未退出导致热键「already registered」与 WebView2 类注册冲突
            .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
                // Agent 以后台方式再次启动：实例已在运行，保持静默，不唤起窗口、不抢焦点
                if has_background_flag(args.iter().map(String::as_str)) {
                    return;
                }
                if let Some(win) = app.get_webview_window("main") {
                    let _ = win.show();
                    let _ = win.unminimize();
                    let _ = win.set_focus();
                }
                // 测试时常从已在托盘运行的实例再次执行
                // `kotone.exe --onboarding=always`。第二实例不会进入 setup，
                // 因此必须把强制打开请求转发给现有设置窗口。
                if parse_onboarding_launch_mode(args.iter().map(String::as_str))
                    == OnboardingLaunchMode::Always
                {
                    let _ = app.emit_to("main", "kotone://open-onboarding", ());
                }
            }));
    }
    builder
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        // 关闭按钮不退出应用：main / overlay 窗口 CloseRequested 一律转 hide（托盘常驻）；
        // 仅托盘菜单「退出」（app.exit）真正结束进程。
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if matches!(window.label(), "overlay" | "danmaku") {
                    if let Some(win) = window.app_handle().get_webview_window(window.label()) {
                        hide_window(&win);
                    }
                } else {
                    let _ = window.hide();
                }
                api.prevent_close();
            }
        })
        .setup(move |app| {
            log::init();
            let previous_panic_hook = std::panic::take_hook();
            std::panic::set_hook(Box::new(move |info| {
                log::log(&format!("panic: {info}"));
                previous_panic_hook(info);
            }));
            log::log(&format!(
                "startup: version={} elevated={} arch={}",
                env!("CARGO_PKG_VERSION"),
                elevation::is_elevated(),
                std::env::consts::ARCH
            ));
            tray::setup_tray(app.handle())?;
            app.manage(startup_options.clone());

            // 首次运行：默认配置 + 内置 profile 落盘（~/.kotone/）
            let loaded_settings = settings::load_with_diagnostic();
            let settings_load_warning = loaded_settings.warning;
            let settings = loaded_settings.settings;
            process_log::set_recording_enabled(settings.diagnostics.recording);
            let mut app_started =
                process_log::ProcessEvent::new(process_log::app_session_id(), "app_started");
            app_started.context.engine_id = Some(settings.stt_engine.clone());
            app_started.context.model_id =
                Some(model::active_model_from(&settings, &settings.stt_engine));
            app_started.context.profile_id = settings.active_profile_id.clone();
            app_started.context.elevated = Some(elevation::is_elevated());
            let _ = process_log::record(app_started);

            // 自启动提权已前移到 run() 顶部（auto_elevate_before_tauri_init）：
            // 单实例插件初始化早于 setup，父进程若在此才 runas 退出，提权子进程
            // 会被单实例锁误判为第二实例而自杀，导致「双击无反应」。

            if let Err(e) = profile::ensure_builtin() {
                eprintln!("[kotone] 内置 profile 落盘失败: {e}");
            }

            // silero VAD 模型随二进制打包，首次启动解压到模型目录；
            // 失败仅记日志不阻断启动（one-shot 判停缺失时 begin 会报清晰错误）
            match model::ensure_vad_model() {
                Ok(written) => {
                    if written {
                        log::log("silero VAD 模型已写入模型目录");
                    }
                }
                Err(e) => log::log(&format!("silero VAD 模型写入失败: {e}")),
            }

            // 用量账本（usage.jsonl）：之后的在线调用都记进去。
            usage::start();
            // 旧版提醒沿用自己的音色；琴音角色在选择体验后才初始化。
            let companion_enabled = !settings.ui.legacy_upgrade.uses_legacy_assistant();
            if companion_enabled {
                roles::remember(&settings.agent.role_id);
            }
            let settings = Arc::new(RwLock::new(settings));
            let settings_repository = Arc::new(SettingsRepository::new(settings.clone()));
            let mut registry = EngineRegistry::new();
            kotone_stt::register_builtin(&mut registry);
            let engines = Arc::new(registry);
            let mut processor_registry = kotone_core::postprocess::ProcessorRegistry::new();
            let secrets: Arc<dyn SecretStore> = Arc::new(kotone_ai::secrets::KeyringSecretStore);
            let connections: Arc<dyn ConnectionResolver> = Arc::new(
                kotone_ai::secrets::SecretBackedResolver::new(settings.clone(), secrets.clone()),
            );
            if let Err(error) = kotone_postprocess::register_with_connections(
                &mut processor_registry,
                connections.clone(),
            ) {
                return Err(Box::<dyn std::error::Error>::from(error));
            }
            let processors = Arc::new(processor_registry);
            let emitter: Arc<dyn Emitter> = Arc::new(TauriEmitter {
                app: app.handle().clone(),
                vis_gen: Arc::new(std::sync::atomic::AtomicU64::new(0)),
                hidden_for_companion: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            });
            let injector: Arc<dyn Injector> = Arc::new(WindowsInjector);
            let focus: Arc<dyn FocusBackend> = Arc::new(WinFocusBackend);
            // 语音唤醒（ADR 017）：唤醒后的那次会话改用唤醒线程转来的音频，其余照常开麦克风。
            let wake_runtime = wake::WakeRuntime::default();
            let audio_backend: Arc<dyn kotone_core::audio::AudioBackend> =
                Arc::new(wake::WakeAudioBackend {
                    inner: Arc::new(platform_audio::CpalBackend),
                    handoff: wake_runtime.handoff(),
                });
            app.manage(wake_runtime);
            app.manage(voice_ask::VoiceAsk::default());
            app.manage(bubble::BubbleRuntime::default());
            app.manage(speech_screen::SpeechScreen::default());
            app.manage(computer_use::ComputerUse::default());
            app.manage(voice_chat::VoiceChat::default());
            app.manage(voice_listen::VoiceListen::default());
            app.manage(key_request::KeyRequests::default());
            app.manage(browser::BrowserRuntime::default());
            let assistant_runtime =
                assistant::initialize(app.handle(), settings.clone(), connections.clone());

            #[allow(unused_mut)] // vad-silero feature 关闭时无可变接线
            let mut orchestrator = Orchestrator::new(
                settings.clone(),
                engines.clone(),
                audio_backend,
                injector.clone(),
                focus,
                emitter,
            )
            .with_processors(processors.clone())
            .with_secrets(secrets.clone())
            .with_settings_repository(settings_repository.clone());
            if let Ok(scheduler) = &assistant_runtime.scheduler {
                orchestrator = orchestrator
                    .with_assistant(assistant_runtime.handler.clone(), scheduler.clone());
            }
            let capture_app = app.handle().clone();
            orchestrator = orchestrator
                .with_before_capture(Arc::new(move || assistant::before_capture(&capture_app)));
            let capture_app = app.handle().clone();
            orchestrator = orchestrator
                .with_after_capture(Arc::new(move || assistant::after_capture(&capture_app)));
            // VAD 接线（ADR-007）：vad-silero feature 开启时注入 silero 工厂；
            // 默认构建不接入——one-shot 模式 begin 会报清晰错误
            #[cfg(feature = "vad-silero")]
            {
                orchestrator.vad_factory = Some(kotone_stt::vad::silero_factory());
            }
            let orchestrator = orchestrator.into_arc();
            orchestrator.suspend_runtime_inputs();

            app.manage(SharedState {
                settings: settings.clone(),
                settings_repository,
                settings_load_warning: Mutex::new(settings_load_warning),
                orchestrator: orchestrator.clone(),
                engines,
                processors,
                connections,
                secrets,
                injector,
            });
            app.manage(HotkeyManager::new(app.handle(), orchestrator.clone()));
            app.manage(RuntimeManager::new());
            app.manage(ResourceMonitor(Mutex::new(sysinfo::System::new())));
            app.manage(overlay::OverlayManager::default());
            app.manage(assistant_runtime);
            app.manage(speech_models::SpeechModelManager::default());
            app.manage(ocr_engines::OcrEngines::new(app.handle()));
            app.manage(companion::CompanionRuntime::default());
            app.manage(automation::AutomationManager::open());
            if companion_enabled {
                automation::ensure_builtin(app.handle());
            }
            automation::start_schedules(app.handle());
            // 内置琴音 Agent：对话存储与运行状态（ADR 014 / 015）
            app.manage(agent::AgentRuntime::open(companion_enabled));
            // 放歌（ADR 034）：播放器、登录状态（只有登录过才联网查账号）
            music::init(app.handle());
            // 插件（ADR 038）：各自开后台循环（英雄联盟：客户端开着时盯对局阶段，进加载界面先查这局的战绩）
            plugins::init(app.handle());
            // 游戏上方的覆盖层（ADR 038）：插件给了条目才建窗口；跟着前台的目标窗口挪、切走就藏
            overlay_layer::start(app.handle());
            if companion_enabled {
                // 琴音原来的名字和头像记进她的角色卡（只做一次）。
                roles::migrate(app.handle());
                // 填充话（“我看看”“稍等哈”……）先合成好（ADR 029 §四）。等启动的事忙完再做。
                let fillers_app = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    fillers::refresh(&fillers_app);
                });
            }
            // 对局复盘：订阅游戏中心的“一局结束了”就整理（agent.matchRecap，默认开）。
            recap::start(app.handle());
            // 游戏中心（ADR 036）：看进程和前台窗口，合成游戏状态、发游戏事件、按事件运行自动化。
            games::start(app.handle());
            // 先接上程序执行端口，再启动调度器（到点的程序才有人执行）。
            automation::attach(app.handle());
            assistant::start(app.handle());
            // 游戏语音（ADR 020）：设置里开着就开始转发（之后跟着设置保存启停）。
            voice_chat::sync(app.handle());
            // 可选本机工具服务：独立于 ASR 运行时，只在启动时读取开关（docs/tool-service.md）
            let tool_service_enabled = settings.read().unwrap().tool_service.enabled;
            tool_service::start(app.handle(), tool_service_enabled);
            compatibility::start_fullscreen_monitor(app.handle().clone());
            compatibility::watch_input_refusals(app.handle().clone());

            // 启动时的窗口（设置、弹幕、气泡）在全部状态 manage() 之后才建：页面一加载就会发 IPC，
            // 走快速 IPC（ipc.localhost）时可能早于 setup 结束到达，命令取不到状态会崩溃。
            create_startup_windows(app.handle(), companion_enabled);
            for label in ["main", "overlay", "danmaku"] {
                if let Some(win) = app.get_webview_window(label) {
                    harden_webview(&win);
                }
            }
            // 开发调试：`pnpm tauri dev -- --console` 启动后自动打开 webview 控制台。
            // `open_devtools` 只在 Tauri 的开发构建中可用，必须从 release 编译中移除。
            #[cfg(debug_assertions)]
            {
                if std::env::args().any(|a| a == "--console") {
                    if let Some(win) = app.get_webview_window("main") {
                        win.open_devtools();
                    }
                }
            }
            overlay::reconcile(app.handle(), false);

            // 应用级热键（启动 / 停止 Kotone）：runtime 未运行时也必须生效，
            // 故在启动阶段就落到低级钩子上（未配置时空操作，不安装钩子）。
            {
                let manager = app.state::<HotkeyManager>();
                let app_hotkeys = settings.read().unwrap().clone();
                manager.apply_app_hotkeys(&app_hotkeys);
            }
            // 语音识别还没就绪：按语音输入热键 / 呼叫键只提示怎么启动，键照常传给游戏（D8）。
            runtime::refresh_hints(app.handle());
            tray::refresh(app.handle());

            // 语音识别运行时（core runtime 状态机）默认 Stopped。ADR 028：在线识别打开就能用，
            // 直接 start；本地识别只有开了「打开时自动启动」（ui.autoStart）才自动启动
            // （warmup → 注册热键 → 显示悬浮窗，进度经 kotone://runtime 推送）。
            if runtime::should_start_on_launch(&settings.read().unwrap()) {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = runtime::start(&handle).await {
                        log::log(&format!("auto-start failed: {e}"));
                    }
                });
            }

            // 启动即显示设置窗口：全部窗口默认隐藏时，应用看起来像「没起来」。
            // 托盘常驻语义不变——关闭此窗口只是隐藏，进程仍在托盘。
            // 自动提权重启的场合此分支在提权子进程中执行，同样弹窗，符合预期。
            // `--background`（Agent 启动）只驻留托盘，不显示窗口、不抢焦点
            let background = app.state::<StartupOptions>().background;
            if !background {
                if let Some(win) = app.get_webview_window("main") {
                    let _ = win.show();
                    let _ = win.set_focus();
                }
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            ping,
            export_diagnostics,
            clear_diagnostics,
            log_frontend_error,
            get_startup_options,
            get_settings,
            get_overlay_health,
            get_overlay_state,
            list_post_processors,
            list_connection_presets,
            list_connections,
            upsert_connection,
            delete_connection,
            test_post_processing,
            get_settings_load_warning,
            update_settings,
            list_audio_devices,
            set_audio_device,
            list_stt_engines,
            set_stt_engine,
            get_engine_options,
            list_profiles,
            save_profile,
            export_hotwords,
            export_builtin_blocklist,
            import_hotwords,
            export_profile,
            import_profile,
            delete_profile,
            get_profile_icon,
            console_chat_status,
            console_chat_set,
            save_overlay_position,
            get_elevation_status,
            get_hotkey_status,
            check_input_environment,
            detect_hotkey_conflicts,
            start_hotkey_capture,
            cancel_hotkey_capture,
            restart_as_admin,
            get_autostart_enabled,
            set_autostart_enabled,
            preview_sfx,
            list_models,
            download_model,
            cancel_download,
            get_model_install_guide,
            open_model_dest_dir,
            set_active_model,
            get_runtime_status,
            start_runtime,
            stop_runtime,
            get_models_dir,
            set_models_dir,
            delete_model,
            open_models_dir,
            open_history_dir,
            open_external,
            get_history,
            clear_history,
            delete_history_record,
            read_history_audio,
            get_resource_usage,
            confirm_send,
            cancel_session,
            simulate_send,
            assistant::list_reminders,
            assistant::cancel_reminder,
            assistant::assistant_submit_text,
            assistant::cancel_assistant_request,
            assistant_input::assistant_voice_begin,
            assistant_input::assistant_voice_end,
            assistant_input::assistant_voice_confirm,
            assistant_input::assistant_voice_cancel,
            assistant::list_speech_voices,
            assistant::preview_reminder,
            assistant::assistant_call,
            assistant::stop_speech,
            assistant::get_danmaku,
            assistant::finish_danmaku,
            speech_models::get_speech_model_status,
            speech_models::download_speech_model,
            wake::wake_status,
            bubble::bubble_snapshot,
            bubble::bubble_preview,
            wake::wake_download_model,
            speech_models::cancel_speech_model_download,
            automation::automation_runs,
            automation::automation_saved_list,
            games::game_state,
            automation::automation_get,
            automation::automation_save,
            automation::automation_delete,
            automation::automation_run_saved,
            automation::automation_stop,
            usage::usage_summary,
            usage::usage_conversation,
            roles::roles_list,
            roles::roles_get,
            roles::roles_switch,
            roles::roles_update,
            roles::roles_create,
            roles::roles_delete,
            roles::roles_export,
            roles::roles_import,
            roles::roles_check_wake,
            agent::agent_send,
            workspace_cmd::workspace_overview,
            key_request::agent_submit_key,
            game_tools::game_icon,
            game_tools::launch_installed_game,
            workspace_cmd::workspace_read,
            workspace_cmd::workspace_write,
            workspace_cmd::workspace_delete_game,
            workspace_cmd::workspace_set_avatar,
            workspace_cmd::workspace_avatar,
            workspace_cmd::workspace_delete_skill,
            workspace_cmd::workspace_import_skill,
            workspace_cmd::workspace_open_dir,
            agent::agent_stop,
            call::call_active,
            agent::agent_answer,
            agent::agent_state,
            agent::agent_tools,
            agent::agent_conversation,
            agent::agent_new_conversation,
            agent::agent_select_conversation,
            agent::agent_delete_conversation,
            agent::agent_subagent_messages,
            agent::agent_subagent_stop,
            agent::agent_subagent_continue,
            agent::agent_subagent_answer,
            plugins_cmd::plugins_list,
            plugins_cmd::plugin_call,
            music_cmd::music_state,
            music_cmd::music_login_start,
            music_cmd::music_login_cancel,
            music_cmd::music_logout,
            music_cmd::music_search,
            music_cmd::music_home,
            music_cmd::music_playlist,
            music_cmd::music_play,
            music_cmd::music_control,
            music_cmd::music_queue,
            music_cmd::music_set_quality,
            browser_cmd::browser_state,
            browser_cmd::browser_set_bounds,
            browser_cmd::browser_control,
            browser_cmd::browser_handover_done,
            browser_cmd::browser_clear_data,
            browser_cmd::browser_navigate,
            browser_cmd::browser_tab_new,
            browser_cmd::browser_tab_close,
            browser_cmd::browser_tab_select,
            browser_cmd::browser_extensions,
            browser_cmd::browser_extension_add,
            browser_cmd::browser_extension_remove,
            browser_cmd::browser_extension_toggle,
            browser_cmd::browser_probe,
            agent::agent_set_permissions,
            agent::agent_set_automation_hidden,
            companion::companion_self_check,
            companion::connection_self_check,
            companion::connection_models,
            game_tools::installed_games,
            companion::companion_status,
            companion::companion_clear_chat_region,
            model_catalog::list_catalog_models,
            model_catalog::download_catalog_model,
            model_catalog::import_catalog_model,
            model_catalog::delete_catalog_model,
            model_catalog::activate_catalog_model,
            model_catalog::get_catalog_install_guide,
            model_catalog::open_catalog_model_dir,
            voice_chat::voice_chat_status,
            voice_chat::voice_chat_install,
            voice_chat::voice_chat_default_mic,
            voice_chat::voice_chat_test,
            voice_chat::voice_chat_say,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Kotone application")
        .run(|app, event| {
            if matches!(event, tauri::RunEvent::Exit) {
                tool_service::shutdown(app);
                automation::shutdown(app);
                voice_listen::stop_all(app);
                assistant::shutdown(app);
                wake::shutdown(app);
            }
        });
}
