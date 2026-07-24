//! 提权检测与管理员重启：UIPI 提权方案的核心模块（docs/development.md §10 R-1）。
//!
//! 背景：目标游戏以高权限（管理员）运行时，Windows UIPI 会整体丢弃来自
//! 中权限进程的合成输入、前台切换也会失败。Kotone 默认 asInvoker 启动
//! （不强制 UAC），改为运行时检测 + 一键管理员重启 + 可选的自启动提权。
//!
//! - `is_elevated()`：OpenProcessToken + GetTokenInformation(TokenElevation)
//! - `is_process_elevated(pid)`：OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)
//!   + TokenElevation；OpenProcess 被拒（ERROR_ACCESS_DENIED）视为
//!   「目标权限高于我们」的信号 → Some(true)
//! - `restart_as_admin()`：ShellExecuteExW "runas" 重启自身 exe（带当前参数）

/// 自动提权重启时追加的命令行标记：子进程见到它即不再重复 runas（防循环）
pub const ELEVATED_RETRY_ARG: &str = "--kotone-elevated-spawn";

/// 目标进程提权探测结果分类（Win32 调用封装成此枚举，判定逻辑可单测）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElevationProbe {
    /// 成功读到 TokenElevation（true = 目标已提权）
    TokenElevated(bool),
    /// OpenProcess 访问被拒（ERROR_ACCESS_DENIED）：目标权限高于我们
    AccessDenied,
    /// 其他失败（进程已退出、句柄失效等）：无法判断
    Unknown,
}

/// 探测结果 → 对外语义：AccessDenied 视为提权信号，Unknown → None（无法判断）
pub fn interpret_probe(probe: ElevationProbe) -> Option<bool> {
    match probe {
        ElevationProbe::TokenElevated(b) => Some(b),
        ElevationProbe::AccessDenied => Some(true),
        ElevationProbe::Unknown => None,
    }
}

/// 「需要提示提权」纯判定：目标已提权且自身未提权（可单测）
pub fn decide_needs_elevation(target_elevated: Option<bool>, self_elevated: bool) -> bool {
    matches!(target_elevated, Some(true)) && !self_elevated
}

/// 自启动提权防循环判定（纯逻辑，可单测）：
/// 仅当「设置开启 + 当前未提权 + 本次不是 runas 重启出来的子进程」时才发起 runas。
/// 用户在 UAC 弹窗点「否」后子进程仍未提权，但 retry_marker_present = true，
/// 本次会话放弃重试，避免无限重启循环。
pub fn should_auto_elevate(
    run_as_admin_on_start: bool,
    elevated: bool,
    retry_marker_present: bool,
) -> bool {
    run_as_admin_on_start && !elevated && !retry_marker_present
}

/// 「激活 profile → 目标游戏 pid」纯逻辑链路（进程枚举注入，可单测）。
///
/// 断链修复：activeProfileId 指向的 profile 文件缺失/损坏时回退同 id 的内置
/// profile，不再因单点文件问题让 get_elevation_status 静默返回 null。
/// generic（无进程名）/ 未指定 profile / 进程未运行 → None（无法判断，不误报）。
pub fn resolve_active_game_pid(
    active_profile_id: Option<&str>,
    available: &[kotone_core::profile::GameProfile],
    find_pid: &mut dyn FnMut(&str) -> Option<u32>,
) -> Option<u32> {
    let id = active_profile_id?;
    let profile = available
        .iter()
        .find(|p| p.id == id)
        .cloned()
        .or_else(|| match id {
            "lol" => Some(kotone_core::profile::GameProfile::builtin_lol()),
            "generic" => Some(kotone_core::profile::GameProfile::builtin_generic()),
            _ => None,
        })?;
    if profile.process_names.is_empty() {
        return None;
    }
    profile.process_names.iter().find_map(|n| find_pid(n))
}

/// 当前进程命令行是否带有 runas 重启标记
pub fn retry_marker_present() -> bool {
    std::env::args().any(|a| a == ELEVATED_RETRY_ARG)
}

// ---------- Windows 实现 ----------

#[cfg(windows)]
mod win {
    use super::ElevationProbe;
    use std::mem::size_of;

    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{
        CloseHandle, GetLastError, ERROR_ACCESS_DENIED, ERROR_CANCELLED, HANDLE, HWND,
    };
    use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
    use windows::Win32::System::Threading::{
        GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows::Win32::UI::Shell::{ShellExecuteExW, SHELLEXECUTEINFOW};
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    /// 读取 token 的 TokenElevation；失败返回 None
    fn token_elevation(token: HANDLE) -> Option<bool> {
        let mut elevation = TOKEN_ELEVATION::default();
        let mut ret_len = 0u32;
        unsafe {
            GetTokenInformation(
                token,
                TokenElevation,
                Some(&mut elevation as *mut _ as *mut _),
                size_of::<TOKEN_ELEVATION>() as u32,
                &mut ret_len,
            )
            .ok()?;
        }
        Some(elevation.TokenIsElevated != 0)
    }

    /// 当前进程是否已提权；任何一步失败按未提权处理（保守，不影响发送路径）
    pub fn is_elevated() -> bool {
        unsafe {
            let mut token = HANDLE::default();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
                return false;
            }
            let elevated = token_elevation(token);
            let _ = CloseHandle(token);
            elevated.unwrap_or(false)
        }
    }

    /// 探测目标进程提权状态（返回分类枚举，由 interpret_probe 转语义）
    pub fn probe_process(pid: u32) -> ElevationProbe {
        unsafe {
            let handle = match OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
                Ok(h) => h,
                Err(_) => {
                    return if GetLastError() == ERROR_ACCESS_DENIED {
                        ElevationProbe::AccessDenied
                    } else {
                        ElevationProbe::Unknown
                    };
                }
            };
            let probe = (|| {
                let mut token = HANDLE::default();
                OpenProcessToken(handle, TOKEN_QUERY, &mut token).ok()?;
                let elevated = token_elevation(token);
                let _ = CloseHandle(token);
                elevated
            })();
            let _ = CloseHandle(handle);
            match probe {
                Some(b) => ElevationProbe::TokenElevated(b),
                None => ElevationProbe::Unknown,
            }
        }
    }

    /// 把路径/参数包成双引号（含空格时），拼成 lpParameters
    fn quote_arg(arg: &str) -> String {
        if arg.chars().any(char::is_whitespace) {
            format!("\"{arg}\"")
        } else {
            arg.to_string()
        }
    }

    /// ShellExecuteExW "runas" 重启自身 exe。
    /// extra_args 追加在现有参数之后（自动提权路径用来传防循环标记）。
    /// 成功仅表示新进程已拉起；当前进程是否退出由调用方决定。
    pub fn restart_with_extra_args(extra_args: &[&str]) -> Result<(), String> {
        let exe = std::env::current_exe().map_err(|e| format!("获取自身 exe 路径失败: {e}"))?;
        // 现有参数剔除旧的防循环标记（避免标记逐次堆积），再按需追加
        let mut args: Vec<String> = std::env::args()
            .skip(1)
            .filter(|a| a != super::ELEVATED_RETRY_ARG)
            .collect();
        args.extend(extra_args.iter().map(|s| s.to_string()));
        let params = args
            .iter()
            .map(|a| quote_arg(a))
            .collect::<Vec<_>>()
            .join(" ");

        let exe_w: Vec<u16> = exe.to_string_lossy().encode_utf16().chain([0]).collect();
        let verb_w: Vec<u16> = "runas".encode_utf16().chain([0]).collect();
        let params_w: Vec<u16> = params.encode_utf16().chain([0]).collect();

        let mut info = SHELLEXECUTEINFOW {
            cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
            hwnd: HWND::default(),
            lpVerb: PCWSTR(verb_w.as_ptr()),
            lpFile: PCWSTR(exe_w.as_ptr()),
            lpParameters: PCWSTR(params_w.as_ptr()),
            nShow: SW_SHOWNORMAL.0,
            ..Default::default()
        };
        unsafe {
            if ShellExecuteExW(&mut info).is_ok() {
                Ok(())
            } else {
                let err = GetLastError();
                if err == ERROR_CANCELLED {
                    Err("已取消提权（UAC 弹窗被拒绝），Kotone 继续以当前权限运行".into())
                } else {
                    Err(format!("以管理员身份重启失败: {err:?}"))
                }
            }
        }
    }
}

// ---------- 对外 API（Windows 真实实现 / 其他平台兜底） ----------

/// 当前进程是否已提权（非 Windows 恒 false）
#[cfg(windows)]
pub fn is_elevated() -> bool {
    win::is_elevated()
}

/// 目标进程是否提权：Some(true/false) = 已判定；None = 无法判断。
/// OpenProcess 被拒（ERROR_ACCESS_DENIED）视为「目标权限高于我们」→ Some(true)。
#[cfg(windows)]
pub fn is_process_elevated(pid: u32) -> Option<bool> {
    interpret_probe(win::probe_process(pid))
}

/// 以管理员身份重启自身（带当前参数）。成功仅表示新进程已拉起；
/// 调用方（IPC 命令）负责退出当前进程。
#[cfg(windows)]
pub fn restart_as_admin() -> Result<(), String> {
    win::restart_with_extra_args(&[])
}

/// 自启动提权路径：runas 重启并追加防循环标记
#[cfg(windows)]
pub fn restart_for_auto_elevate() -> Result<(), String> {
    win::restart_with_extra_args(&[ELEVATED_RETRY_ARG])
}

#[cfg(not(windows))]
pub fn is_elevated() -> bool {
    false
}

#[cfg(not(windows))]
pub fn is_process_elevated(_pid: u32) -> Option<bool> {
    None
}

#[cfg(not(windows))]
pub fn restart_as_admin() -> Result<(), String> {
    Err("提权重启仅 Windows 支持（MVP Windows-first）".into())
}

#[cfg(not(windows))]
pub fn restart_for_auto_elevate() -> Result<(), String> {
    Err("提权重启仅 Windows 支持（MVP Windows-first）".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interpret_probe_mapping() {
        assert_eq!(interpret_probe(ElevationProbe::TokenElevated(true)), Some(true));
        assert_eq!(
            interpret_probe(ElevationProbe::TokenElevated(false)),
            Some(false)
        );
        // OpenProcess 被拒 = 目标权限高于我们 → 视为提权信号
        assert_eq!(interpret_probe(ElevationProbe::AccessDenied), Some(true));
        // 其他失败 → 无法判断
        assert_eq!(interpret_probe(ElevationProbe::Unknown), None);
    }

    #[test]
    fn decide_needs_elevation_truth_table() {
        // 目标提权 + 自身未提权 → 需要提示
        assert!(decide_needs_elevation(Some(true), false));
        // 自身已提权 → 不需要
        assert!(!decide_needs_elevation(Some(true), true));
        // 目标未提权 → 不需要
        assert!(!decide_needs_elevation(Some(false), false));
        // 无法判断 → 不误报
        assert!(!decide_needs_elevation(None, false));
    }

    #[test]
    fn should_auto_elevate_truth_table() {
        // 开启 + 未提权 + 无标记 → 发起 runas
        assert!(should_auto_elevate(true, false, false));
        // 未开启 → 不动
        assert!(!should_auto_elevate(false, false, false));
        // 已提权 → 不再 runas（正常 runas 成功后的子进程路径）
        assert!(!should_auto_elevate(true, true, false));
        assert!(!should_auto_elevate(true, true, true));
        // 防循环：runas 子进程仍未提权（用户取消 UAC）→ 本次会话放弃
        assert!(!should_auto_elevate(true, false, true));
    }

    #[test]
    fn resolve_active_game_pid_chain() {
        use kotone_core::profile::GameProfile;
        let available = vec![GameProfile::builtin_lol(), GameProfile::builtin_generic()];

        // 命中：激活 lol 且进程在运行（进程枚举 mock 大小写不敏感）
        let pid = resolve_active_game_pid(Some("lol"), &available, &mut |n| {
            if n.eq_ignore_ascii_case("league of legends.exe") {
                Some(42)
            } else {
                None
            }
        });
        assert_eq!(pid, Some(42));

        // profile 文件缺失（available 为空）→ 回退内置 lol，链路不断
        let pid = resolve_active_game_pid(Some("lol"), &[], &mut |_| Some(7));
        assert_eq!(pid, Some(7));

        // 进程未运行 → None（无法判断，不误报）
        assert_eq!(
            resolve_active_game_pid(Some("lol"), &available, &mut |_| None),
            None
        );
        // 未指定 profile → None
        assert_eq!(
            resolve_active_game_pid(None, &available, &mut |_| Some(1)),
            None
        );
        // generic 无进程名 → None
        assert_eq!(
            resolve_active_game_pid(Some("generic"), &available, &mut |_| Some(1)),
            None
        );
        // 未知 profile id（磁盘与内置都没有）→ None
        assert_eq!(
            resolve_active_game_pid(Some("ghost"), &available, &mut |_| Some(1)),
            None
        );
    }

    #[cfg(windows)]
    #[test]
    fn is_elevated_real_call_runs() {
        // 本机真实调用：普通 shell 应为 false；这里只验证调用跑通并打印结果
        let elevated = is_elevated();
        eprintln!("[elevation test] is_elevated() = {elevated}");
        // 自举一致性：以 PROCESS_QUERY_LIMITED_INFORMATION 打开自己一定成功，
        // 读到的 TokenElevation 必须与 is_elevated() 一致
        assert_eq!(is_process_elevated(std::process::id()), Some(elevated));
    }
}
