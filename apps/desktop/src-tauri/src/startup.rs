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

/// 自启动提权（§10 R-1）：设置开启且当前未提权时，runas 重启自身并立即退出。
///
/// 必须在 Tauri Builder 初始化之前执行：单实例插件在插件初始化阶段创建
/// 命名互斥锁，若在 setup() 里才 runas，未提权父进程已持有该锁——提权子进程
/// 会把自己当成「第二实例」，把参数转发给正在退出的父进程后自杀，父进程随后
/// 也退出，表现为「双击后没有任何窗口」（0.1.4 用户反馈）。在这里提前判定并
/// std::process::exit，父进程从不触碰单实例锁 / WebView2，子进程是唯一实例。
///
/// 防循环：子进程带 ELEVATED_RETRY_ARG 标记；用户取消 UAC 时 ShellExecuteExW
/// 直接返回错误，本进程按普通权限继续启动（不退出）。
#[cfg(windows)]
pub(crate) fn auto_elevate_before_tauri_init() {
    let settings = settings::load();
    if !elevation::should_auto_elevate(
        settings.run_as_admin_on_start,
        elevation::is_elevated(),
        elevation::retry_marker_present(),
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
pub(crate) fn create_startup_windows(app: &tauri::AppHandle, companion_enabled: bool) {
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
    use super::{parse_onboarding_launch_mode, OnboardingLaunchMode};

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
