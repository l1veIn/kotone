//! 启动：命令行参数（后台、新手向导）、启动前自动提权、创建启动时的窗口。

use super::*;

/// 启动时如何处理新手向导。
///
/// - auto：仅配置中尚未完成时显示（正式用户默认）
/// - always：本次强制显示，但不重置持久化完成标记（回归测试）
/// - never：本次强制跳过（自动化/故障排查）
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum OnboardingLaunchMode {
    Auto,
    Always,
    Never,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StartupOptions {
    pub(crate) onboarding: OnboardingLaunchMode,
    /// `--background`：只驻留托盘，不打开设置窗口、不抢焦点（供 Agent / `kotone-cli app launch`）。
    pub(crate) background: bool,
}

pub(crate) fn has_background_flag<I, S>(args: I) -> bool
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    args.into_iter().any(|a| a.as_ref() == "--background")
}

pub(crate) fn parse_onboarding_launch_mode<I, S>(args: I) -> OnboardingLaunchMode
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut mode = OnboardingLaunchMode::Auto;
    let mut expect_value = false;
    for arg in args {
        let arg = arg.as_ref();
        let value = if expect_value {
            expect_value = false;
            Some(arg)
        } else if arg == "--onboarding" {
            expect_value = true;
            None
        } else {
            arg.strip_prefix("--onboarding=")
        };
        mode = match value {
            Some("always") => OnboardingLaunchMode::Always,
            Some("never") => OnboardingLaunchMode::Never,
            Some("auto") => OnboardingLaunchMode::Auto,
            _ => mode,
        };
    }
    mode
}

#[tauri::command]
pub(crate) fn get_startup_options(options: tauri::State<StartupOptions>) -> StartupOptions {
    options.inner().clone()
}

/// 单实例插件（tauri-plugin-single-instance 2.4 的 Windows 实现，`platform_impl/windows.rs`）用的名字和转交格式：
/// 互斥锁 `{identifier}-sim`、消息窗口类名 `-sic` / 标题 `-siw`、`WM_COPYDATA` 的 dwData 1542、
/// 内容 `工作目录|参数1|参数2…\0`。插件升级时 `single_instance_protocol_is_pinned` 会提醒回来核对。
const SINGLE_INSTANCE_ID: &str = "com.l1vein.kotone";
const SINGLE_INSTANCE_COPYDATA: usize = 1542;

#[cfg(windows)]
fn single_instance_window() -> (String, String) {
    (
        format!("{SINGLE_INSTANCE_ID}-sic"),
        format!("{SINGLE_INSTANCE_ID}-siw"),
    )
}

/// 自启动提权（§10 R-1、ADR 044 D1）：已经用现在的说法问过、设置开启、当前未提权且不是后台启动（`--background`）时，
/// runas 重启自身并立即退出。
/// 还没问过（`adminPromptRevision` 旧）时不重开：窗口里先说明一句（`AdminExplainDialog`），玩家点「好」再弹 UAC。
///
/// 必须在 Tauri Builder 初始化之前执行：单实例插件在插件初始化阶段创建
/// 命名互斥锁，若在 setup() 里才 runas，未提权父进程已持有该锁——提权子进程
/// 会把自己当成「第二实例」，把参数转发给正在退出的父进程后自杀，父进程随后
/// 也退出，表现为「双击后没有任何窗口」（0.1.4 用户反馈）。在这里提前判定并
/// std::process::exit，父进程从不触碰单实例锁 / WebView2，子进程是唯一实例。
///
/// 已经有一个 Kotone 在跑时不弹 UAC（只是要把它的窗口唤出来）：打得开它的互斥锁就交给单实例插件转交；
/// 打不开（它是管理员身份、这次是普通权限，ADR 044 默认管理员以后最常见：玩家再双击图标）时插件会以为
/// 自己是第一个、再起一份，所以这里照插件的格式自己转交（管理员那份在 [`create_startup_windows`] 里放开了
/// `WM_COPYDATA`），然后退出。
///
/// 防循环：子进程带 ELEVATED_RETRY_ARG 标记；用户取消 UAC 时 ShellExecuteExW
/// 直接返回错误，本进程按普通权限继续启动（不退出）。
#[cfg(windows)]
pub(crate) fn auto_elevate_before_tauri_init() {
    // 隔离实例（设了 KOTONE_HOME：开发、测试、外部 Agent 的独立实例）不参与默认管理员，和不参与单实例一样：
    // 不弹 UAC、窗口里也不说明（`compatibility::elevation_status` 报的版本是 0）
    if std::env::var_os("KOTONE_HOME").is_some() {
        return;
    }
    let settings = settings::load();
    let elevated = elevation::is_elevated();
    match elevation::running_instance(&format!("{SINGLE_INSTANCE_ID}-sim")) {
        elevation::RunningInstance::None => {}
        elevation::RunningInstance::Reachable => return,
        elevation::RunningInstance::Elevated => {
            let cwd = std::env::current_dir().unwrap_or_default();
            let args = std::env::args().collect::<Vec<_>>().join("|");
            let (class, title) = single_instance_window();
            let payload = format!("{}|{args}", cwd.to_string_lossy());
            if !elevation::forward_to_instance(&class, &title, SINGLE_INSTANCE_COPYDATA, &payload) {
                eprintln!("[kotone] 管理员身份的 Kotone 在运行，没能把窗口唤出来");
            }
            std::process::exit(0);
        }
    }
    // 后台启动（--background）不弹 UAC：安全桌面会把玩家从全屏游戏里拽出来（ADR 043 D6 同一条规矩）
    if !elevation::should_auto_elevate(
        settings.run_as_admin_on_start,
        settings.admin_prompt_revision,
        elevated,
        elevation::retry_marker_present(),
        has_background_flag(std::env::args()),
    ) {
        return;
    }
    match elevation::restart_for_auto_elevate() {
        Ok(()) => {
            // 提权副本已拉起：立刻退出，不做任何 Tauri/WebView2 初始化。
            std::process::exit(0);
        }
        Err(e) => eprintln!("[kotone] 自动提权重启失败: {e}"),
    }
}

/// 建启动时就要有的窗口（`tauri.conf.json` 里标了 `create: false`；悬浮窗另由 [`overlay`] 按需建）。
///
/// 这些窗口建的时候都藏着，配置里都是 `focus: false`：建窗口时要焦点，WebView2 会把键盘焦点移进还藏着的
/// 窗口（wry 的 `MoveFocus`），把它激活成前台，`--background` 启动就抢走了游戏或终端的焦点。要显示设置窗口的
/// 地方（正常启动、托盘、第二个实例、点消息提示）都用 [`show_and_focus`]。
pub(crate) fn create_startup_windows(app: &tauri::AppHandle, companion_enabled: bool) {
    // 管理员身份运行时（ADR 044）：让普通权限再打开的那一次也能把窗口唤出来（见 auto_elevate_before_tauri_init）
    #[cfg(windows)]
    if elevation::is_elevated() && std::env::var_os("KOTONE_HOME").is_none() {
        let (class, title) = single_instance_window();
        if let Err(e) = elevation::allow_unelevated_messages(&class, &title) {
            log::log(&format!(
                "startup: allow second-instance messages failed: {e}"
            ));
        }
    }
    for label in ["danmaku", "companion", "main"] {
        if (label == "companion" && !companion_enabled) || app.get_webview_window(label).is_some() {
            continue;
        }
        let Some(config) = app
            .config()
            .app
            .windows
            .iter()
            .find(|config| config.label == label)
            .cloned()
        else {
            log::log(&format!("startup: missing window configuration {label}"));
            continue;
        };
        match tauri::WebviewWindowBuilder::from_config(app, &config)
            .and_then(|builder| builder.build())
        {
            // 弹幕窗口鼠标穿透（原来在 assistant::start 里设，那时窗口还没建）。
            Ok(window) if label == "danmaku" => {
                let _ = window.set_ignore_cursor_events(true);
            }
            Ok(_) => {}
            Err(e) => log::log(&format!("startup: create window {label} failed: {e}")),
        }
    }
}

#[cfg(test)]
mod startup_options_tests {
    use super::{has_background_flag, parse_onboarding_launch_mode, OnboardingLaunchMode};
    use kotone_platform_windows::elevation::{should_auto_elevate, ADMIN_PROMPT_REVISION};

    /// 后台启动（外部 Agent、`kotone-cli app launch`）不弹 UAC；正常打开、开机自启（Run 键没有参数）照旧。
    #[test]
    fn background_launches_never_auto_elevate() {
        let decide = |args: &[&str]| {
            should_auto_elevate(
                true,
                ADMIN_PROMPT_REVISION,
                false,
                false,
                has_background_flag(args.iter().copied()),
            )
        };
        assert!(!decide(&["kotone.exe", "--background"]));
        assert!(!decide(&[
            "kotone.exe",
            "--onboarding=never",
            "--background"
        ]));
        assert!(decide(&["kotone.exe"]), "开机自启 / 双击图标");
        assert!(decide(&["kotone.exe", "--onboarding=always"]));
    }

    /// 自己转交给管理员身份的实例时照抄了单实例插件的名字和格式：identifier 改了、插件升级了都要回来核对。
    #[test]
    fn single_instance_protocol_is_pinned() {
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert_eq!(conf["identifier"], super::SINGLE_INSTANCE_ID);
        let lock: Vec<&str> = include_str!("../../../../Cargo.lock")
            .lines()
            .map(str::trim)
            .collect();
        let at = lock
            .iter()
            .position(|l| *l == r#"name = "tauri-plugin-single-instance""#)
            .expect("Cargo.lock 里有 tauri-plugin-single-instance");
        assert_eq!(
            lock[at + 1],
            r#"version = "2.4.3""#,
            "tauri-plugin-single-instance 升级了：核对它的互斥锁名、消息窗口名和 WM_COPYDATA 格式（startup.rs）"
        );
    }

    #[test]
    fn onboarding_mode_defaults_to_auto() {
        assert_eq!(
            parse_onboarding_launch_mode(["kotone.exe"]),
            OnboardingLaunchMode::Auto
        );
    }

    #[test]
    fn onboarding_mode_accepts_equals_and_split_forms() {
        assert_eq!(
            parse_onboarding_launch_mode(["kotone.exe", "--onboarding=always"]),
            OnboardingLaunchMode::Always
        );
        assert_eq!(
            parse_onboarding_launch_mode(["kotone.exe", "--onboarding", "never"]),
            OnboardingLaunchMode::Never
        );
    }

    #[test]
    fn invalid_onboarding_mode_keeps_last_valid_value() {
        assert_eq!(
            parse_onboarding_launch_mode([
                "kotone.exe",
                "--onboarding=always",
                "--onboarding=invalid",
            ]),
            OnboardingLaunchMode::Always
        );
    }
}
