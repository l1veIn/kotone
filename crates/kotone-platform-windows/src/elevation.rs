//! 提权检测与管理员重启：UIPI 提权方案的核心模块（docs/development.md §10 R-1）。
//!
//! 背景：目标游戏以高权限（管理员）运行时，Windows UIPI 会整体丢弃来自
//! 中权限进程的合成输入、前台切换也会失败。Kotone 默认 asInvoker 启动
//! （不强制 UAC），改为运行时检测 + 一键管理员重启 + 可选的自启动提权。
//!
//! - `is_elevated()`：OpenProcessToken + GetTokenInformation(TokenElevation)
//! - `is_process_elevated(pid)`：OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)
//!   + TokenElevation；OpenProcess 被拒（ERROR_ACCESS_DENIED）视为
//!     「目标权限高于我们」的信号 → Some(true)
//! - `restart_as_admin()`：ShellExecuteExW "runas" 重启自身 exe（带当前参数，GUI 用）
//! - `run_elevated(args)`：sudo 式——runas 重启自身但参数完全由调用方给定
//!   （替换语义，CLI elevate <command> 用）
//! - `process_integrity(pid)`：进程的完整性级别（普通 = Medium，管理员 = High），测试看降权用
//! - `running_instance` / `forward_to_instance` / `allow_unelevated_messages`：管理员身份的 Kotone 在跑时，
//!   普通权限再打开一次也只是把窗口唤出来（ADR 044，见 `startup.rs`）
//!
//! ADR 044：默认以管理员身份运行（`runAsAdminOnStart` 默认开）；第一次先在窗口里说明原因再弹 UAC
//! （[`ADMIN_PROMPT_REVISION`]）。Kotone 打开的东西一律降回普通权限，见 `unelevated`。

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

/// 管理员身份的说明现在是第几版（设置里的 `adminPromptRevision` 比它小 = 还没用这一版的说法问过）。
/// 1 = ADR 044 的说法（呼叫键、说话键、往游戏里打字）。以后说法再变就加一，所有人再问一次。
pub const ADMIN_PROMPT_REVISION: u32 = 1;

/// 启动最早（Tauri 初始化之前）要不要以管理员身份重开自己（纯逻辑，可单测）：
/// 仅当「已经用现在的说法问过 + 设置开启 + 当前未提权 + 本次不是 runas 重启出来的子进程」时才发起 runas。
/// - 没问过（`prompt_revision` 比 [`ADMIN_PROMPT_REVISION`] 小：新装的、以前点过“不再提示”的、以前就开着的）：
///   先不重开，窗口里先说明原因，玩家点「好」再弹 UAC（ADR 044 D1）。
/// - 防循环：用户在 UAC 弹窗点「否」后子进程仍未提权，但 retry_marker_present = true，本次放弃重试。
/// - 后台启动（`--background`：外部 Agent、`kotone-cli app launch`）不重开：UAC 会切到安全桌面，把玩家从全屏游戏里
///   拽出来（和 ADR 043 D6“游戏里不抢焦点”同一条规矩）。照“UAC 点了否”那样以普通权限运行。
///   正常打开和开机自启（Run 键不带 `--background`）照旧重开。
pub fn should_auto_elevate(
    run_as_admin_on_start: bool,
    prompt_revision: u32,
    elevated: bool,
    retry_marker_present: bool,
    background_launch: bool,
) -> bool {
    prompt_revision >= ADMIN_PROMPT_REVISION
        && run_as_admin_on_start
        && !elevated
        && !retry_marker_present
        && !background_launch
}

/// 完整性级别：普通权限（Medium）。
pub const INTEGRITY_MEDIUM: u32 = 0x2000;
/// 完整性级别：管理员（High）。
pub const INTEGRITY_HIGH: u32 = 0x3000;

/// 已经在跑的 Kotone（单实例插件的互斥锁）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunningInstance {
    /// 没有。
    None,
    /// 有，打得开它的互斥锁：单实例插件照常把这次打开转交给它。
    Reachable,
    /// 有，但打不开它的互斥锁（它是管理员身份、我们不是）：单实例插件会以为自己是第一个，再起一份。
    Elevated,
}

/// 单个命令行参数 → lpParameters 片段（MSVC / CommandLineToArgvW 解析规则，
/// 纯逻辑可单测）：不含空白与引号的非空参数原样透传；其余包双引号，
/// 内部 `"` 转义为 `\"`，且紧邻引号的连续反斜杠翻倍（其余 `\` 原样）。
/// 这样 `listen --profile "lol oce"` 经 shell 去引号后能原样还原给提权副本。
fn quote_arg(arg: &str) -> String {
    if !arg.is_empty() && !arg.chars().any(|c| c.is_whitespace() || c == '"') {
        return arg.to_string();
    }
    let mut out = String::with_capacity(arg.len() + 2);
    out.push('"');
    let mut backslashes = 0usize;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                // 引号前的反斜杠翻倍，再加一个转义引号自身的反斜杠
                out.push_str(&"\\".repeat(backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            _ => {
                out.push_str(&"\\".repeat(backslashes));
                backslashes = 0;
                out.push(c);
            }
        }
    }
    // 收尾引号前的反斜杠同样翻倍
    out.push_str(&"\\".repeat(backslashes * 2));
    out.push('"');
    out
}

/// 参数列表 → lpParameters 串（空格分隔；纯逻辑可单测）
fn build_params(args: &[String]) -> String {
    args.iter()
        .map(|a| quote_arg(a))
        .collect::<Vec<_>>()
        .join(" ")
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
    use windows::Win32::Security::{
        GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
    };
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

    /// 进程的完整性级别（`TokenIntegrityLevel` 的最后一个子权限：0x2000 普通、0x3000 管理员）；读不到为 None。
    pub fn process_integrity(pid: u32) -> Option<u32> {
        use windows::Win32::Security::{
            GetSidSubAuthority, GetSidSubAuthorityCount, TokenIntegrityLevel, TOKEN_MANDATORY_LABEL,
        };
        unsafe {
            let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
            let level = (|| {
                let mut token = HANDLE::default();
                OpenProcessToken(process, TOKEN_QUERY, &mut token).ok()?;
                let level = (|| {
                    let mut len = 0u32;
                    let _ = GetTokenInformation(token, TokenIntegrityLevel, None, 0, &mut len);
                    if len == 0 {
                        return None;
                    }
                    let mut buf = vec![0u64; (len as usize).div_ceil(8)];
                    GetTokenInformation(
                        token,
                        TokenIntegrityLevel,
                        Some(buf.as_mut_ptr() as *mut _),
                        len,
                        &mut len,
                    )
                    .ok()?;
                    let label = &*(buf.as_ptr() as *const TOKEN_MANDATORY_LABEL);
                    let sid = label.Label.Sid;
                    let count = *GetSidSubAuthorityCount(sid);
                    if count == 0 {
                        return None;
                    }
                    Some(*GetSidSubAuthority(sid, count as u32 - 1))
                })();
                let _ = CloseHandle(token);
                level
            })();
            let _ = CloseHandle(process);
            level
        }
    }

    /// 单实例插件的互斥锁在不在、打不打得开。
    pub fn running_instance(mutex_name: &str) -> super::RunningInstance {
        use windows::Win32::System::Threading::{OpenMutexW, SYNCHRONIZATION_SYNCHRONIZE};
        let name: Vec<u16> = mutex_name.encode_utf16().chain([0]).collect();
        unsafe {
            match OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE, false, PCWSTR(name.as_ptr())) {
                Ok(handle) => {
                    let _ = CloseHandle(handle);
                    super::RunningInstance::Reachable
                }
                Err(_) if GetLastError() == ERROR_ACCESS_DENIED => super::RunningInstance::Elevated,
                Err(_) => super::RunningInstance::None,
            }
        }
    }

    /// 照单实例插件的做法把这次打开转交给在跑的实例（它的消息窗口收 `WM_COPYDATA`）。
    /// 对方是管理员身份时，要它先调过 [`allow_unelevated_messages`]，不然 Windows 会挡掉（UIPI）。
    pub fn forward_to_instance(class: &str, title: &str, tag: usize, payload: &str) -> bool {
        use windows::Win32::Foundation::{LPARAM, WPARAM};
        use windows::Win32::System::DataExchange::COPYDATASTRUCT;
        use windows::Win32::UI::WindowsAndMessaging::{
            FindWindowW, SendMessageTimeoutW, SMTO_ABORTIFHUNG, WM_COPYDATA,
        };
        let class: Vec<u16> = class.encode_utf16().chain([0]).collect();
        let title: Vec<u16> = title.encode_utf16().chain([0]).collect();
        let mut bytes = payload.as_bytes().to_vec();
        bytes.push(0);
        unsafe {
            let Ok(hwnd) = FindWindowW(PCWSTR(class.as_ptr()), PCWSTR(title.as_ptr())) else {
                return false;
            };
            let data = COPYDATASTRUCT {
                dwData: tag,
                cbData: bytes.len() as u32,
                lpData: bytes.as_mut_ptr() as *mut _,
            };
            let mut result = 0usize;
            let sent = SendMessageTimeoutW(
                hwnd,
                WM_COPYDATA,
                WPARAM(0),
                LPARAM(&data as *const COPYDATASTRUCT as isize),
                SMTO_ABORTIFHUNG,
                5_000,
                Some(&mut result),
            );
            sent.0 != 0
        }
    }

    /// 让普通权限的进程也能给这个窗口发 `WM_COPYDATA`（`ChangeWindowMessageFilterEx`）：
    /// 管理员身份的 Kotone 在跑时，玩家再双击图标（普通权限）也能把窗口唤出来。
    pub fn allow_unelevated_messages(class: &str, title: &str) -> Result<(), String> {
        use windows::Win32::UI::WindowsAndMessaging::{
            ChangeWindowMessageFilterEx, FindWindowW, MSGFLT_ALLOW, WM_COPYDATA,
        };
        let class: Vec<u16> = class.encode_utf16().chain([0]).collect();
        let title: Vec<u16> = title.encode_utf16().chain([0]).collect();
        unsafe {
            let hwnd = FindWindowW(PCWSTR(class.as_ptr()), PCWSTR(title.as_ptr()))
                .map_err(|e| format!("找不到单实例的窗口：{e}"))?;
            ChangeWindowMessageFilterEx(hwnd, WM_COPYDATA, MSGFLT_ALLOW, None)
                .map_err(|e| format!("ChangeWindowMessageFilterEx：{e}"))
        }
    }

    /// ShellExecuteExW "runas" 重启自身 exe。
    /// extra_args 追加在现有参数之后（自动提权路径用来传防循环标记）。
    /// 成功仅表示新进程已拉起；当前进程是否退出由调用方决定。
    pub fn restart_with_extra_args(extra_args: &[&str]) -> Result<(), String> {
        // 现有参数剔除旧的防循环标记（避免标记逐次堆积），再按需追加。
        // 也去掉 --background：后台启动的那份不自动提权（ADR 044），玩家在窗口里点了重开，重开的那份要显示窗口
        let mut args: Vec<String> = std::env::args()
            .skip(1)
            .filter(|a| a != super::ELEVATED_RETRY_ARG && a != "--background")
            .collect();
        args.extend(extra_args.iter().map(|s| s.to_string()));
        shell_execute_runas(&super::build_params(&args))
    }

    /// sudo 式提权：runas 重启自身 exe，参数完全由调用方给定（替换语义，
    /// 不带当前进程参数）。CLI elevate <command> 用：提权副本在新控制台
    /// 执行该子命令（典型 `kotone-cli elevate listen`）。
    pub fn restart_with_args(args: &[String]) -> Result<(), String> {
        shell_execute_runas(&super::build_params(args))
    }

    /// ShellExecuteExW "runas" 拉起自身 exe + 给定参数串。
    /// 成功仅表示新进程已拉起；当前进程是否退出由调用方决定。
    fn shell_execute_runas(params: &str) -> Result<(), String> {
        let exe = std::env::current_exe().map_err(|e| format!("获取自身 exe 路径失败: {e}"))?;

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

/// sudo 式提权执行：以给定参数 runas 重启自身（替换语义，不带当前进程参数）。
/// 成功仅表示新进程已拉起；调用方负责退出当前进程。
#[cfg(windows)]
pub fn run_elevated(args: &[String]) -> Result<(), String> {
    win::restart_with_args(args)
}

/// 进程的完整性级别（[`INTEGRITY_MEDIUM`]、[`INTEGRITY_HIGH`]……）；读不到为 None。
#[cfg(windows)]
pub fn process_integrity(pid: u32) -> Option<u32> {
    win::process_integrity(pid)
}

/// 单实例插件的互斥锁（`mutex_name`）在不在、打不打得开。
#[cfg(windows)]
pub fn running_instance(mutex_name: &str) -> RunningInstance {
    win::running_instance(mutex_name)
}

/// 把这次打开转交给在跑的实例（单实例插件的消息窗口 `class` / `title`，`WM_COPYDATA` 的 `tag` 和内容照插件）。
#[cfg(windows)]
pub fn forward_to_instance(class: &str, title: &str, tag: usize, payload: &str) -> bool {
    win::forward_to_instance(class, title, tag, payload)
}

/// 让普通权限的进程也能给单实例插件的消息窗口发 `WM_COPYDATA`。
#[cfg(windows)]
pub fn allow_unelevated_messages(class: &str, title: &str) -> Result<(), String> {
    win::allow_unelevated_messages(class, title)
}

#[cfg(not(windows))]
pub fn is_elevated() -> bool {
    false
}

#[cfg(not(windows))]
pub fn process_integrity(_pid: u32) -> Option<u32> {
    None
}

#[cfg(not(windows))]
pub fn running_instance(_mutex_name: &str) -> RunningInstance {
    RunningInstance::None
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

#[cfg(not(windows))]
pub fn run_elevated(_args: &[String]) -> Result<(), String> {
    Err("提权执行仅 Windows 支持（MVP Windows-first）".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interpret_probe_mapping() {
        assert_eq!(
            interpret_probe(ElevationProbe::TokenElevated(true)),
            Some(true)
        );
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
    fn quote_arg_passthrough_and_wrapping() {
        // 无空白无引号：原样透传（典型：listen / --engine / mock-stream）
        assert_eq!(quote_arg("listen"), "listen");
        assert_eq!(quote_arg("--engine"), "--engine");
        // 含空白：包双引号（lol oce 经 shell 去引号后仍能还原为一个参数）
        assert_eq!(quote_arg("lol oce"), "\"lol oce\"");
        // 空参数：空引号对
        assert_eq!(quote_arg(""), "\"\"");
    }

    #[test]
    fn quote_arg_escapes_quotes_and_backslashes() {
        // 内嵌引号 → \"（MSVC / CommandLineToArgvW 规则）
        assert_eq!(quote_arg("say \"hi\""), "\"say \\\"hi\\\"\"");
        // 普通反斜杠不转义
        assert_eq!(quote_arg("C:\\tmp\\a b"), "\"C:\\tmp\\a b\"");
        // 紧邻收尾引号的反斜杠翻倍
        assert_eq!(
            quote_arg("C:\\Program Files\\"),
            "\"C:\\Program Files\\\\\""
        );
        // 引号前的反斜杠翻倍 + 引号自身转义
        assert_eq!(quote_arg("a\\\""), "\"a\\\\\\\"\"");
    }

    #[test]
    fn build_params_joins_with_spaces() {
        let args: Vec<String> = ["listen", "--engine", "mock-stream"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(build_params(&args), "listen --engine mock-stream");
        let args: Vec<String> = ["send", "--text", "打野 在下路"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(build_params(&args), "send --text \"打野 在下路\"");
        assert_eq!(build_params(&[]), "");
    }

    #[test]
    fn should_auto_elevate_truth_table() {
        let asked = ADMIN_PROMPT_REVISION;
        // 问过 + 开启 + 未提权 + 无标记 + 不是后台启动 → 发起 runas（包括开机自启）
        assert!(should_auto_elevate(true, asked, false, false, false));
        // 以后的版本也算问过
        assert!(should_auto_elevate(true, asked + 1, false, false, false));
        // 未开启（玩家点了「先不用」或在设置里关了）→ 不动
        assert!(!should_auto_elevate(false, asked, false, false, false));
        // 已提权 → 不再 runas（正常 runas 成功后的子进程路径）
        assert!(!should_auto_elevate(true, asked, true, false, false));
        assert!(!should_auto_elevate(true, asked, true, true, false));
        // 防循环：runas 子进程仍未提权（用户取消 UAC）→ 本次会话放弃
        assert!(!should_auto_elevate(true, asked, false, true, false));
        // 后台启动（--background）：不管别的怎样都不弹 UAC
        for setting in [true, false] {
            for revision in [0, asked] {
                for marker in [true, false] {
                    assert!(!should_auto_elevate(setting, revision, false, marker, true));
                }
            }
        }
        // 没用现在的说法问过（新装的 0、以前点过“不再提示”的、以前就开着的）→ 先不重开，窗口里先说明
        for setting in [true, false] {
            for elevated in [true, false] {
                for marker in [true, false] {
                    for background in [true, false] {
                        assert!(!should_auto_elevate(
                            setting, 0, elevated, marker, background
                        ));
                    }
                }
            }
        }
    }

    #[cfg(windows)]
    #[test]
    fn own_integrity_matches_elevation() {
        let level = process_integrity(std::process::id()).expect("读得到自己的完整性级别");
        if is_elevated() {
            assert!(level >= INTEGRITY_HIGH, "{level:#x}");
        } else {
            assert!(level <= INTEGRITY_MEDIUM, "{level:#x}");
        }
        assert_eq!(process_integrity(0), None);
    }

    #[cfg(windows)]
    #[test]
    fn running_instance_sees_a_mutex() {
        use windows::core::PCWSTR;
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::System::Threading::CreateMutexW;
        let name = format!("kotone-test-instance-{}", std::process::id());
        assert_eq!(running_instance(&name), RunningInstance::None);
        let wide: Vec<u16> = name.encode_utf16().chain([0]).collect();
        let handle = unsafe { CreateMutexW(None, true, PCWSTR(wide.as_ptr())) }.unwrap();
        assert_eq!(running_instance(&name), RunningInstance::Reachable);
        unsafe {
            let _ = CloseHandle(handle);
        }
        assert_eq!(running_instance(&name), RunningInstance::None);
    }

    /// 转交照单实例插件的格式：一个不显示的窗口（不抢焦点）收 WM_COPYDATA，内容 = `tag` 和以 0 结尾的文字。
    #[cfg(windows)]
    #[test]
    fn forward_reaches_the_instance_window() {
        use std::sync::mpsc;
        use std::sync::Mutex;
        use windows::core::PCWSTR;
        use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
        use windows::Win32::System::DataExchange::COPYDATASTRUCT;
        use windows::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
            PostQuitMessage, RegisterClassW, TranslateMessage, MSG, WINDOW_EX_STYLE, WM_COPYDATA,
            WNDCLASSW, WS_POPUP,
        };

        static GOT: Mutex<Option<(usize, String)>> = Mutex::new(None);
        unsafe extern "system" fn proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
            if msg == WM_COPYDATA {
                let data = unsafe { &*(l.0 as *const COPYDATASTRUCT) };
                let bytes = unsafe {
                    std::slice::from_raw_parts(data.lpData as *const u8, data.cbData as usize)
                };
                *GOT.lock().unwrap() = Some((data.dwData, String::from_utf8_lossy(bytes).into()));
                unsafe { PostQuitMessage(0) };
                return LRESULT(1);
            }
            unsafe { DefWindowProcW(hwnd, msg, w, l) }
        }

        let class = format!("kotone-test-sic-{}", std::process::id());
        let title = format!("kotone-test-siw-{}", std::process::id());
        let (ready_tx, ready_rx) = mpsc::channel();
        let (c, t) = (class.clone(), title.clone());
        let pump = std::thread::spawn(move || unsafe {
            let class_w: Vec<u16> = c.encode_utf16().chain([0]).collect();
            let title_w: Vec<u16> = t.encode_utf16().chain([0]).collect();
            let wc = WNDCLASSW {
                lpfnWndProc: Some(proc),
                lpszClassName: PCWSTR(class_w.as_ptr()),
                ..Default::default()
            };
            RegisterClassW(&wc);
            // 不调 ShowWindow：一直不显示
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                PCWSTR(class_w.as_ptr()),
                PCWSTR(title_w.as_ptr()),
                WS_POPUP,
                0,
                0,
                0,
                0,
                None,
                None,
                None,
                None,
            )
            .unwrap();
            ready_tx.send(()).unwrap();
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            let _ = DestroyWindow(hwnd);
        });
        ready_rx.recv().unwrap();
        // 同一个完整性级别的窗口本来就收得到；放开也不出错
        allow_unelevated_messages(&class, &title).unwrap();
        assert!(forward_to_instance(
            &class,
            &title,
            1542,
            "C:\\dir|kotone.exe|--background"
        ));
        pump.join().unwrap();
        assert_eq!(
            GOT.lock().unwrap().clone(),
            Some((1542, "C:\\dir|kotone.exe|--background\0".to_string()))
        );
        assert!(!forward_to_instance(
            "kotone-no-such-class",
            "none",
            1542,
            "x"
        ));
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
