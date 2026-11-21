//! Source 2 控制台通道（ADR 032 决定 3，照 cs2-chat-translator）：不开聊天框直接发。
//!
//! ```text
//! 把 say_team "文字" 写进 <游戏>/<cfgDir>/kotone_say.cfg → 模拟按一下 F23（游戏里绑了 exec kotone_say）
//! ```
//!
//! 聊天框不打开，键盘始终归游戏；没有输入法的事；玩家同时按什么都不影响。
//!
//! - **开关就是 `autoexec.cfg` 里的一行**（`bind "F23" "exec kotone_say" // Kotone <写入时间>`），
//!   不另存设置。打开 / 关闭只在游戏没运行时做，改之前把原文件备份到
//!   `<KOTONE_HOME>/agent/backups/<毫秒>/<盘符>/<原路径>`（照 ADR 030）。往游戏目录只写这两个文件。
//! - **每次发送都判断走哪条路**（[`route`]）：profile 声明了 `consoleChat`、这个频道有命令、
//!   找得到 Steam 安装目录、`autoexec.cfg` 里有 Kotone 那一行、游戏进程晚于那一行启动、
//!   前台是这个游戏；缺一个就走键盘路径（`input_gate` 保护的那条）。
//! - `kotone_say.cfg` 是唯一在游戏运行时写的文件，只归 Kotone；发出约 1 秒后清空，
//!   免得玩家误触触发键时重发上一条。
//!
//! 纯函数（整理文字、找 / 加 / 删那一行、选路）不碰文件和 Win32，单测直接测。

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use chrono::{DateTime, FixedOffset, Local};
use kotone_core::inject::InjectError;
use kotone_core::profile::{ConsoleChat, GameProfile};

/// 每次发送写的文件（在游戏的 cfg 目录里）。
pub const SAY_FILE: &str = "kotone_say.cfg";
/// 游戏启动时自动执行的配置。
pub const AUTOEXEC_FILE: &str = "autoexec.cfg";
const EXEC: &str = "exec kotone_say";
const MARKER: &str = "// Kotone";
const STAMP_FORMAT: &str = "%Y-%m-%d %H:%M:%S %:z";
/// 发出后多久清空 `kotone_say.cfg`。
const CLEAR_AFTER: Duration = Duration::from_millis(1000);

// ---------- 纯函数 ----------

/// 整理文字（ADR 032 决定 3 第 5 条）：换行换成空格；英文双引号换成单引号；首尾去空白。
/// 空消息返回 None（不发）。分号、`//`、反斜杠要不要处理，等真机验证 S4 后再定。
pub fn sanitize(text: &str) -> Option<String> {
    let cleaned: String = text
        .chars()
        .map(|c| match c {
            '\r' | '\n' => ' ',
            '"' => '\'',
            c => c,
        })
        .collect();
    let cleaned = cleaned.trim();
    (!cleaned.is_empty()).then(|| cleaned.to_string())
}

/// `kotone_say.cfg` 的内容：一行 `say_team "…"`（UTF-8，无 BOM）。
pub fn say_line(command: &str, text: &str) -> String {
    format!("{command} \"{text}\"\n")
}

/// `autoexec.cfg` 里 Kotone 的那一行（不含换行）。
pub fn bind_line(trigger_key: &str, written_at: DateTime<FixedOffset>) -> String {
    format!(
        "bind \"{trigger_key}\" \"{EXEC}\" {MARKER} {}",
        written_at.format(STAMP_FORMAT)
    )
}

/// 认出来的 Kotone 那一行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KotoneBind {
    /// 绑的键（玩家可能改过）。
    pub key: String,
    /// 写入时间；玩家改坏了就是 None（当成没生效）。
    pub written_at: Option<DateTime<FixedOffset>>,
}

fn kotone_line(line: &str) -> Option<KotoneBind> {
    let line = line.trim();
    let (command, comment) = line.split_once(MARKER)?;
    let command = command.trim();
    if !command.to_ascii_lowercase().starts_with("bind") || !command.contains(EXEC) {
        return None;
    }
    // bind 后面的第一个参数：带引号就取引号里的，否则取到空白为止
    let rest = command[4..].trim_start();
    let key = match rest.strip_prefix('"') {
        Some(quoted) => quoted.split('"').next().unwrap_or(""),
        None => rest.split_whitespace().next().unwrap_or(""),
    }
    .to_string();
    let written_at = DateTime::parse_from_str(comment.trim(), STAMP_FORMAT).ok();
    Some(KotoneBind { key, written_at })
}

/// `autoexec.cfg` 里最后一行 Kotone 的绑定（有好几行时以最后一行为准，和游戏执行顺序一致）。
pub fn find_bind(autoexec: &[u8]) -> Option<KotoneBind> {
    autoexec
        .split(|&b| b == b'\n')
        .rev()
        .find_map(|line| kotone_line(&String::from_utf8_lossy(line)))
}

/// 去掉所有 Kotone 的绑定行；别的内容一个字节都不动（文件可能不是 UTF-8）。
pub fn without_bind(autoexec: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(autoexec.len());
    for line in autoexec.split_inclusive(|&b| b == b'\n') {
        if kotone_line(&String::from_utf8_lossy(line)).is_none() {
            out.extend_from_slice(line);
        }
    }
    out
}

/// 加上 Kotone 的绑定行：先去掉旧的，再追加在文件末尾；沿用文件原来的换行风格。
pub fn with_bind(autoexec: &[u8], line: &str) -> Vec<u8> {
    let mut out = without_bind(autoexec);
    let eol: &[u8] = if autoexec.windows(2).any(|w| w == b"\r\n") {
        b"\r\n"
    } else {
        b"\n"
    };
    if !out.is_empty() && !out.ends_with(b"\n") {
        out.extend_from_slice(eol);
    }
    out.extend_from_slice(line.as_bytes());
    out.extend_from_slice(eol);
    out
}

/// 前台窗口所属进程。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForegroundProcess {
    pub name: String,
    /// 完整路径；拿不到（例如权限更高的进程）时为 None。
    pub exe: Option<PathBuf>,
    /// 进程启动时间（Unix 毫秒）。
    pub started_ms: Option<i64>,
}

/// 选路的输入（都由调用方采集好）。
pub struct RouteInputs<'a> {
    pub profile: &'a GameProfile,
    /// Steam 安装目录（没找到 = None）。
    pub install_dir: Option<&'a Path>,
    /// `autoexec.cfg` 的内容（不存在 = None）。
    pub autoexec: Option<&'a [u8]>,
    pub foreground: Option<&'a ForegroundProcess>,
}

/// 走控制台通道要的东西。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Route {
    pub cfg_dir: PathBuf,
    pub trigger_key: String,
    /// `say_team` / `say`
    pub command: String,
}

/// 这次发送走不走控制台通道（ADR 032 决定 3 第 3 条）；Err = 走键盘路径的原因。
pub fn decide(inputs: &RouteInputs) -> Result<Route, &'static str> {
    let profile = inputs.profile;
    let console = profile.console_chat.as_ref().ok_or("没有声明控制台通道")?;
    let command = profile.channel_strategy(None).console_command;
    if command.is_empty() {
        return Err("这个频道没有控制台命令");
    }
    if profile.send_key.trim().is_empty() {
        // 只打进聊天框、不发送（陪玩打字 send=false）：控制台的 say 会直接发出去
        return Err("这次只打字不发送");
    }
    let install_dir = inputs.install_dir.ok_or("没找到 Steam 安装目录")?;
    let cfg_dir = cfg_dir_in(install_dir, console).map_err(|_| "cfg 目录无效")?;
    let bind = inputs
        .autoexec
        .and_then(find_bind)
        .ok_or("autoexec.cfg 里没有 Kotone 那一行")?;
    if !bind.key.eq_ignore_ascii_case(console.trigger_key.trim()) {
        return Err("autoexec.cfg 里绑的不是触发键");
    }
    let written_ms = bind
        .written_at
        .map(|t| t.timestamp_millis())
        .ok_or("autoexec.cfg 那一行的时间认不出来")?;
    let fg = inputs.foreground.ok_or("没有前台窗口")?;
    if !profile
        .process_names
        .iter()
        .any(|p| p.eq_ignore_ascii_case(&fg.name))
    {
        return Err("前台不是这个游戏");
    }
    let exe = fg.exe.as_deref().ok_or("拿不到前台程序的路径")?;
    if !path_inside(exe, install_dir) {
        return Err("前台程序不在这个安装目录里");
    }
    if !matches!(fg.started_ms, Some(started) if started > written_ms) {
        return Err("游戏是在打开开关之前启动的，重启游戏后才生效");
    }
    Ok(Route {
        cfg_dir,
        trigger_key: console.trigger_key.trim().to_string(),
        command,
    })
}

/// `<安装目录>/<cfgDir>`（cfgDir 已经过 profile 校验：相对路径、以 cfg 结尾）。
pub fn cfg_dir_in(install_dir: &Path, console: &ConsoleChat) -> Result<PathBuf, String> {
    let mut dir = install_dir.to_path_buf();
    for part in console.cfg_dir_parts()? {
        dir.push(part);
    }
    Ok(dir)
}

fn normalized(path: &Path) -> String {
    path.to_string_lossy()
        .replace('/', "\\")
        .trim_start_matches(r"\\?\")
        .trim_end_matches('\\')
        .to_lowercase()
}

fn path_inside(path: &Path, dir: &Path) -> bool {
    let (path, dir) = (normalized(path), normalized(dir));
    !dir.is_empty() && path.len() > dir.len() && path.starts_with(&dir) && {
        path.as_bytes()[dir.len()] == b'\\'
    }
}

/// 改文件前备份的目标：`<backups>/<毫秒>/<盘符>/<原路径>`（照 ADR 030 / kotone-exec）。
pub fn backup_path(backups: &Path, stamp_ms: i64, path: &Path) -> PathBuf {
    use std::path::Component;
    let mut dest = backups.join(stamp_ms.to_string());
    for c in path.components() {
        match c {
            Component::Prefix(p) => {
                let s = p.as_os_str().to_string_lossy();
                let s = s.trim_start_matches(r"\\?\");
                if let Some(unc) = s.strip_prefix(r"\\").or_else(|| s.strip_prefix("UNC\\")) {
                    dest.push("UNC");
                    for part in unc.split('\\').filter(|p| !p.is_empty()) {
                        dest.push(part);
                    }
                } else {
                    dest.push(s.trim_end_matches(':'));
                }
            }
            Component::Normal(n) => dest.push(n),
            _ => {}
        }
    }
    dest
}

/// 开关落到 cfg 目录里的两个文件（调用方已确认游戏没运行）：打开 = `autoexec.cfg` 去掉旧的
/// Kotone 行再在末尾追加一行（没有就新建）+ 建空的 `kotone_say.cfg`；关闭 = 删掉那一行和
/// `kotone_say.cfg`。改动已有的 `autoexec.cfg` 之前先备份到 `backups`。
pub fn write_switch(
    cfg: &Path,
    trigger_key: &str,
    enabled: bool,
    backups: &Path,
    now: DateTime<FixedOffset>,
) -> Result<(), String> {
    if !cfg.is_dir() {
        return Err(format!("游戏目录里没有 {}", cfg.display()));
    }
    let autoexec_path = cfg.join(AUTOEXEC_FILE);
    let say_path = cfg.join(SAY_FILE);
    let old = match std::fs::read(&autoexec_path) {
        Ok(bytes) => Some(bytes),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(format!("读 {} 失败：{e}", autoexec_path.display())),
    };
    let next = if enabled {
        Some(with_bind(
            old.as_deref().unwrap_or_default(),
            &bind_line(trigger_key, now),
        ))
    } else {
        old.as_deref()
            .filter(|bytes| find_bind(bytes).is_some())
            .map(without_bind)
    };
    if let Some(next) = next {
        if let Some(old) = &old {
            let dest = backup_path(backups, now.timestamp_millis(), &autoexec_path);
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent).map_err(|e| format!("建备份目录失败：{e}"))?;
            }
            std::fs::write(&dest, old).map_err(|e| format!("备份 autoexec.cfg 失败：{e}"))?;
        }
        std::fs::write(&autoexec_path, next)
            .map_err(|e| format!("写 {} 失败：{e}", autoexec_path.display()))?;
    }
    if enabled {
        std::fs::write(&say_path, b"")
            .map_err(|e| format!("建 {} 失败：{e}", say_path.display()))?;
    } else if say_path.exists() {
        std::fs::remove_file(&say_path)
            .map_err(|e| format!("删 {} 失败：{e}", say_path.display()))?;
    }
    Ok(())
}

// ---------- 状态与开关（设置页） ----------

/// 设置页显示的状态。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    /// profile 声明了控制台通道。
    pub supported: bool,
    /// Steam 安装目录（没找到 = None）。
    pub install_dir: Option<String>,
    /// 游戏的 cfg 目录（找到安装目录才有）。
    pub cfg_dir: Option<String>,
    /// `autoexec.cfg` 里有 Kotone 那一行。
    pub enabled: bool,
    /// 那一行的写入时间（原样）。
    pub written_at: Option<String>,
    /// 绑的键和配置里的触发键不一样（玩家改过文件）。
    pub key_mismatch: bool,
    pub trigger_key: String,
    /// 游戏正在运行（这时不能切换）。
    pub game_running: bool,
    /// 游戏在运行，但是在打开开关之前启动的（重启游戏后生效）。
    pub needs_restart: bool,
}

#[cfg(windows)]
pub use imp::*;

#[cfg(windows)]
mod imp {
    use super::*;
    use windows::Win32::Foundation::{CloseHandle, FILETIME};
    use windows::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };

    /// 这个 profile 对应的 Steam 安装目录（先比进程名，再比名字；ADR 032 决定 3 第 3 条）。
    pub fn install_dir(profile: &GameProfile) -> Option<PathBuf> {
        if profile.process_names.is_empty() {
            return None;
        }
        let games = crate::games::steam_games();
        crate::games::find_match(&games, &profile.display_name, &profile.process_names)
            .map(|g| PathBuf::from(&g.install_dir))
            .filter(|dir| dir.is_dir())
    }

    fn unix_ms(ft: FILETIME) -> i64 {
        let ticks = (u64::from(ft.dwHighDateTime) << 32) | u64::from(ft.dwLowDateTime);
        (ticks as i64 - 116_444_736_000_000_000) / 10_000
    }

    /// 进程的完整路径和启动时间。
    pub fn process_info(pid: u32) -> (Option<PathBuf>, Option<i64>) {
        unsafe {
            let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
                return (None, None);
            };
            let mut buf = vec![0u16; 1024];
            let mut len = buf.len() as u32;
            let exe = QueryFullProcessImageNameW(
                handle,
                PROCESS_NAME_WIN32,
                windows::core::PWSTR(buf.as_mut_ptr()),
                &mut len,
            )
            .is_ok()
            .then(|| PathBuf::from(String::from_utf16_lossy(&buf[..len as usize])));
            let (mut created, mut exited, mut kernel, mut user) = Default::default();
            let started =
                GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user)
                    .is_ok()
                    .then(|| unix_ms(created));
            let _ = CloseHandle(handle);
            (exe, started)
        }
    }

    fn foreground() -> Option<ForegroundProcess> {
        let pid = crate::inject::foreground_pid()?;
        let (exe, started_ms) = process_info(pid);
        let name = exe
            .as_deref()
            .and_then(Path::file_name)
            .map(|n| n.to_string_lossy().into_owned())
            .or_else(|| crate::inject::process_name_from_pid(pid))?;
        Some(ForegroundProcess {
            name,
            exe,
            started_ms,
        })
    }

    /// 发送时选路：采集输入再交给 [`decide`]。只在 profile 声明了控制台通道时有开销。
    pub fn route(profile: &GameProfile) -> Result<Route, &'static str> {
        let console = profile.console_chat.as_ref().ok_or("没有声明控制台通道")?;
        // 便宜的条件先判：不是这个游戏在前台就不扫 Steam 库、不读文件
        let fg = foreground();
        let fg_matches = fg.as_ref().is_some_and(|fg| {
            profile
                .process_names
                .iter()
                .any(|p| p.eq_ignore_ascii_case(&fg.name))
        });
        let install = if fg_matches {
            install_dir(profile)
        } else {
            None
        };
        let autoexec = install
            .as_deref()
            .and_then(|dir| cfg_dir_in(dir, console).ok())
            .and_then(|cfg| std::fs::read(cfg.join(AUTOEXEC_FILE)).ok());
        decide(&RouteInputs {
            profile,
            install_dir: install.as_deref(),
            autoexec: autoexec.as_deref(),
            foreground: fg.as_ref(),
        })
    }

    /// 写 `kotone_say.cfg` 与清空它互斥；值 = 最近一次写入的序号。
    static SAY_FILE_SEQ: Mutex<u64> = Mutex::new(0);

    /// 走控制台通道发一条（ADR 032 决定 3 第 4 条）：整理文字 → 写文件 → 按一下触发键（按住
    /// 50ms）→ 约 1 秒后清空文件。
    pub fn send(route: &Route, text: &str) -> Result<(), InjectError> {
        let text = sanitize(text).ok_or_else(|| InjectError::new("要发的文字是空的"))?;
        let path = route.cfg_dir.join(SAY_FILE);
        let seq = {
            let mut seq = SAY_FILE_SEQ.lock().unwrap_or_else(|e| e.into_inner());
            std::fs::write(&path, say_line(&route.command, &text))
                .map_err(|e| InjectError::new(format!("写 {SAY_FILE} 失败：{e}")))?;
            *seq += 1;
            *seq
        };
        let result = crate::inject::key_down_up(&route.trigger_key);
        std::thread::Builder::new()
            .name("kotone-console-say-clear".into())
            .spawn(move || {
                std::thread::sleep(CLEAR_AFTER);
                let current = SAY_FILE_SEQ.lock().unwrap_or_else(|e| e.into_inner());
                if *current == seq {
                    let _ = std::fs::write(&path, b"");
                }
            })
            .ok();
        let (outcome, error_code) = match &result {
            Ok(()) => ("ok", None),
            Err(_) => ("error", Some("INJECTION_FAILED".to_string())),
        };
        kotone_core::process_log::record_activity(
            "console_chat_send",
            kotone_core::process_log::EventData {
                detail: Some(format!(
                    "command={} chars={}",
                    route.command,
                    text.chars().count()
                )),
                outcome: Some(outcome.into()),
                error_code,
                ..Default::default()
            },
        );
        result
    }

    /// 游戏在运行吗（任一进程名对上）。
    pub fn game_running(profile: &GameProfile) -> bool {
        crate::inject::process_list().iter().any(|(_, _, exe)| {
            profile
                .process_names
                .iter()
                .any(|p| p.eq_ignore_ascii_case(exe))
        })
    }

    /// 最早启动的那个游戏进程的启动时间（判断“开关之后才启动”用）。
    fn game_started_ms(profile: &GameProfile) -> Option<i64> {
        crate::inject::process_list()
            .iter()
            .filter(|(_, _, exe)| {
                profile
                    .process_names
                    .iter()
                    .any(|p| p.eq_ignore_ascii_case(exe))
            })
            .filter_map(|(pid, _, _)| process_info(*pid).1)
            .min()
    }

    /// 设置页的状态。
    pub fn status(profile: &GameProfile) -> Status {
        let console = profile.console_chat.as_ref();
        let trigger_key = console.map(|c| c.trigger_key.clone()).unwrap_or_default();
        let mut status = Status {
            supported: console.is_some(),
            install_dir: None,
            cfg_dir: None,
            enabled: false,
            written_at: None,
            key_mismatch: false,
            trigger_key,
            game_running: false,
            needs_restart: false,
        };
        let Some(console) = console else {
            return status;
        };
        status.game_running = game_running(profile);
        let Some(install) = install_dir(profile) else {
            return status;
        };
        status.install_dir = Some(install.to_string_lossy().into_owned());
        let Ok(cfg) = cfg_dir_in(&install, console) else {
            return status;
        };
        status.cfg_dir = Some(cfg.to_string_lossy().into_owned());
        let bind = std::fs::read(cfg.join(AUTOEXEC_FILE))
            .ok()
            .and_then(|text| find_bind(&text));
        if let Some(bind) = bind {
            status.enabled = true;
            status.key_mismatch = !bind.key.eq_ignore_ascii_case(console.trigger_key.trim());
            status.written_at = bind.written_at.map(|t| t.format(STAMP_FORMAT).to_string());
            if status.game_running {
                let written = bind.written_at.map(|t| t.timestamp_millis());
                status.needs_restart = match (game_started_ms(profile), written) {
                    (Some(started), Some(written)) => started <= written,
                    _ => true,
                };
            }
        }
        status
    }

    /// 打开 / 关闭（ADR 032 决定 3 第 2 条）：只在游戏没运行时改；改 `autoexec.cfg` 之前备份到
    /// `backups`（`<KOTONE_HOME>/agent/backups`）。打开 = 追加那一行（没有文件就新建）+ 建空的
    /// `kotone_say.cfg`；关闭 = 删掉那一行和 `kotone_say.cfg`。
    pub fn set_enabled(
        profile: &GameProfile,
        enabled: bool,
        backups: &Path,
    ) -> Result<Status, String> {
        let console = profile
            .console_chat
            .as_ref()
            .ok_or("这个游戏配置不支持控制台通道")?;
        if game_running(profile) {
            return Err(format!(
                "「{}」正在运行：先退出游戏再切换（游戏开着时不改它的目录）",
                profile.display_name
            ));
        }
        let install = install_dir(profile).ok_or_else(|| {
            format!(
                "没找到「{}」的 Steam 安装目录（控制台通道只支持 Steam 版）",
                profile.display_name
            )
        })?;
        let cfg = cfg_dir_in(&install, console)?;
        write_switch(
            &cfg,
            console.trigger_key.trim(),
            enabled,
            backups,
            Local::now().fixed_offset(),
        )?;
        kotone_core::log::log(&format!(
            "console chat {} for {} ({})",
            if enabled { "enabled" } else { "disabled" },
            profile.id,
            cfg.display()
        ));
        Ok(status(profile))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 真机（只按一个 F23，进前台的测试窗口）：写 `kotone_say.cfg` → 按触发键（带 Kotone 标记）→
    /// 约 1 秒后清空。`KOTONE_CONSOLE_E2E_DIR` 指向 `target_window.ps1 -Chat -Dir` 的目录。
    /// 前台不是测试窗口时直接失败，不按键。
    #[test]
    #[ignore = "real desktop: presses F23 into the foreground test window"]
    fn send_writes_the_file_presses_the_trigger_and_clears_it() {
        use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowTextW};
        let dir = PathBuf::from(
            std::env::var("KOTONE_CONSOLE_E2E_DIR").expect("set KOTONE_CONSOLE_E2E_DIR"),
        );
        let mut title = [0u16; 256];
        let len = unsafe { GetWindowTextW(GetForegroundWindow(), &mut title) } as usize;
        let title = String::from_utf16_lossy(&title[..len]);
        assert_eq!(
            title, "KotoneScriptTarget",
            "STOP: the test window is not in front"
        );
        let log_before = std::fs::read_to_string(dir.join("events.log")).unwrap_or_default();
        let route = Route {
            cfg_dir: dir.clone(),
            trigger_key: "F23".into(),
            command: "say_team".into(),
        };
        send(&route, " 推中 \"一起\"\n走 ").unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join(SAY_FILE)).unwrap(),
            "say_team \"推中 '一起' 走\"\n"
        );
        std::thread::sleep(Duration::from_millis(1500));
        assert_eq!(
            std::fs::read(dir.join(SAY_FILE)).unwrap(),
            b"",
            "cleared after ~1s"
        );
        let log = std::fs::read_to_string(dir.join("events.log")).unwrap();
        let fresh = &log[log_before.len()..];
        assert!(fresh.contains("DOWN F23 shift=False x=4B544E49"), "{fresh}");
        assert!(fresh.contains("UP F23 x=4B544E49"), "{fresh}");
    }

    #[test]
    fn switch_writes_only_the_two_files_and_backs_up_first() {
        let root = tempfile::tempdir().unwrap();
        let cfg = root.path().join("game").join("dota").join("cfg");
        let backups = root.path().join("home").join("agent").join("backups");
        std::fs::create_dir_all(&cfg).unwrap();
        std::fs::write(cfg.join("config.cfg"), "unrelated").unwrap();

        // 没有 autoexec.cfg：新建，不需要备份
        let t1 = at("2026-11-22 01:00:00 +08:00");
        write_switch(&cfg, "F23", true, &backups, t1).unwrap();
        let first = std::fs::read(cfg.join(AUTOEXEC_FILE)).unwrap();
        assert_eq!(find_bind(&first).unwrap().written_at, Some(t1));
        assert_eq!(std::fs::read(cfg.join(SAY_FILE)).unwrap(), b"");
        assert!(!backups.exists());

        // 玩家后来自己加了一行；再打开一次：仍只有一行 Kotone 的，原文件先备份
        let mut edited = first.clone();
        edited.extend_from_slice(b"bind F2 \"say_team back\"\n");
        std::fs::write(cfg.join(AUTOEXEC_FILE), &edited).unwrap();
        let t2 = at("2026-11-22 02:00:00 +08:00");
        write_switch(&cfg, "F23", true, &backups, t2).unwrap();
        let second = String::from_utf8(std::fs::read(cfg.join(AUTOEXEC_FILE)).unwrap()).unwrap();
        assert_eq!(second.matches("exec kotone_say").count(), 1);
        assert!(second.contains("say_team back"));
        let backup = backup_path(&backups, t2.timestamp_millis(), &cfg.join(AUTOEXEC_FILE));
        assert_eq!(std::fs::read(backup).unwrap(), edited);

        // 关闭：删掉那一行和 kotone_say.cfg，玩家的行和别的文件都不动
        let t3 = at("2026-11-22 03:00:00 +08:00");
        write_switch(&cfg, "F23", false, &backups, t3).unwrap();
        assert_eq!(
            std::fs::read(cfg.join(AUTOEXEC_FILE)).unwrap(),
            b"bind F2 \"say_team back\"\n"
        );
        assert!(!cfg.join(SAY_FILE).exists());
        assert_eq!(std::fs::read(cfg.join("config.cfg")).unwrap(), b"unrelated");
        let mut names: Vec<_> = std::fs::read_dir(&cfg)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, ["autoexec.cfg", "config.cfg"]);

        // 再关一次：没有那一行，什么都不改、不备份
        let before = std::fs::read_dir(&backups).unwrap().count();
        write_switch(
            &cfg,
            "F23",
            false,
            &backups,
            at("2026-11-22 04:00:00 +08:00"),
        )
        .unwrap();
        assert_eq!(std::fs::read_dir(&backups).unwrap().count(), before);

        // cfg 目录不存在：报错，不建目录
        let missing = root.path().join("nope").join("cfg");
        assert!(write_switch(&missing, "F23", true, &backups, t1).is_err());
        assert!(!missing.exists());
    }
    use kotone_core::profile::ConsoleChat;

    fn at(s: &str) -> DateTime<FixedOffset> {
        DateTime::parse_from_str(s, STAMP_FORMAT).unwrap()
    }

    #[test]
    fn sanitize_follows_the_adr_rules() {
        assert_eq!(
            sanitize("  他说\"推塔\"\r\n快来  ").as_deref(),
            Some("他说'推塔'  快来")
        );
        assert_eq!(sanitize(" \n "), None);
        assert_eq!(sanitize("中文 😀 ok").as_deref(), Some("中文 😀 ok"));
        // 分号、// 、反斜杠暂不处理（等 S4）
        assert_eq!(
            sanitize("a; b // c \\ d").as_deref(),
            Some("a; b // c \\ d")
        );
        assert_eq!(say_line("say_team", "推中"), "say_team \"推中\"\n");
    }

    #[test]
    fn bind_line_round_trips() {
        let line = bind_line("F23", at("2026-11-22 01:23:45 +08:00"));
        assert_eq!(
            line,
            "bind \"F23\" \"exec kotone_say\" // Kotone 2026-11-22 01:23:45 +08:00"
        );
        let bind = find_bind(line.as_bytes()).unwrap();
        assert_eq!(bind.key, "F23");
        assert_eq!(bind.written_at, Some(at("2026-11-22 01:23:45 +08:00")));
    }

    #[test]
    fn adds_the_line_to_a_missing_or_existing_autoexec() {
        let line = bind_line("F23", at("2026-11-22 01:00:00 +08:00"));
        // 没有 autoexec.cfg
        assert_eq!(with_bind(b"", &line), format!("{line}\n").into_bytes());
        // 已有内容、末尾没换行、CRLF 风格：沿用 CRLF，原内容不动
        let old = "// 我的设置\r\nbind \"F1\" \"say gg\"".as_bytes();
        let out = String::from_utf8(with_bind(old, &line)).unwrap();
        assert_eq!(
            out,
            format!("// 我的设置\r\nbind \"F1\" \"say gg\"\r\n{line}\r\n")
        );
        // 非 UTF-8 的字节原样保留
        let gbk: &[u8] = b"// \xc4\xe3\xba\xc3\n";
        assert!(with_bind(gbk, &line).starts_with(gbk));
    }

    #[test]
    fn enabling_twice_keeps_one_line_and_disabling_removes_only_ours() {
        let first = bind_line("F23", at("2026-11-22 01:00:00 +08:00"));
        let second = bind_line("F23", at("2026-11-22 02:00:00 +08:00"));
        let once = with_bind(b"exec my_binds\n", &first);
        let twice = with_bind(&once, &second);
        let text = String::from_utf8(twice.clone()).unwrap();
        assert_eq!(text.matches("exec kotone_say").count(), 1);
        assert_eq!(
            find_bind(&twice).unwrap().written_at,
            Some(at("2026-11-22 02:00:00 +08:00"))
        );
        assert_eq!(without_bind(&twice), b"exec my_binds\n");
        // 没有那一行时什么都不动
        assert_eq!(without_bind(b"exec my_binds\n"), b"exec my_binds\n");
    }

    #[test]
    fn player_edits_to_the_file_are_respected() {
        // 玩家在我们那一行后面又加了内容、把键改成 F22：仍认得是 Kotone 的行，删除时只删这一行
        let text = "bind \"F22\" \"exec kotone_say\" // Kotone 2026-11-22 01:00:00 +08:00\nbind \"F2\" \"say_team 回城\"\n";
        let bind = find_bind(text.as_bytes()).unwrap();
        assert_eq!(bind.key, "F22");
        assert_eq!(
            without_bind(text.as_bytes()),
            "bind \"F2\" \"say_team 回城\"\n".as_bytes()
        );
        // 改坏了时间：认得这一行，但时间为空
        let broken = find_bind(b"bind F23 \"exec kotone_say\" // Kotone yesterday").unwrap();
        assert_eq!(broken.key, "F23");
        assert_eq!(broken.written_at, None);
        // 只是注释里提到 Kotone 的别的行不算
        assert!(
            find_bind(b"// Kotone wrote nothing here\nbind F5 \"say hi\" // Kotone\n").is_none()
        );
    }

    fn dota() -> GameProfile {
        let mut p = GameProfile::builtin_dota2();
        p.console_chat = Some(ConsoleChat {
            cfg_dir: "game/dota/cfg".into(),
            trigger_key: "F23".into(),
        });
        p.channels[0].console_command = Some("say_team".into());
        p.channels[1].console_command = Some("say".into());
        p
    }

    struct Fixture {
        install: PathBuf,
        autoexec: Vec<u8>,
        fg: ForegroundProcess,
    }

    fn fixture() -> Fixture {
        let written = at("2026-11-22 01:00:00 +08:00");
        Fixture {
            install: PathBuf::from(r"D:\SteamLibrary\steamapps\common\dota 2 beta"),
            autoexec: with_bind(b"", &bind_line("F23", written)),
            fg: ForegroundProcess {
                name: "dota2.exe".into(),
                exe: Some(PathBuf::from(
                    r"D:\SteamLibrary\steamapps\common\dota 2 beta\game\bin\win64\dota2.exe",
                )),
                started_ms: Some(written.timestamp_millis() + 60_000),
            },
        }
    }

    fn run(p: &GameProfile, f: &Fixture) -> Result<Route, &'static str> {
        decide(&RouteInputs {
            profile: p,
            install_dir: Some(&f.install),
            autoexec: Some(&f.autoexec),
            foreground: Some(&f.fg),
        })
    }

    #[test]
    fn all_conditions_met_routes_to_the_console() {
        let p = dota();
        let f = fixture();
        let route = run(&p, &f).unwrap();
        assert_eq!(route.command, "say_team");
        assert_eq!(route.trigger_key, "F23");
        assert_eq!(
            route.cfg_dir,
            PathBuf::from(r"D:\SteamLibrary\steamapps\common\dota 2 beta\game\dota\cfg")
        );
        // “所有人”频道：发送用配置的默认频道换成了 all
        let all = p.for_channel(&p.channel_strategy(Some("all")));
        assert_eq!(run(&all, &f).unwrap().command, "say");
    }

    #[test]
    fn each_missing_condition_falls_back_to_the_keyboard() {
        let p = dota();
        let f = fixture();
        // 1. 找不到安装目录
        let no_install = decide(&RouteInputs {
            profile: &p,
            install_dir: None,
            autoexec: Some(&f.autoexec),
            foreground: Some(&f.fg),
        });
        assert!(no_install.is_err());
        // 2. autoexec.cfg 没有那一行 / 文件不存在
        let mut g = fixture();
        g.autoexec = b"exec my_binds\n".to_vec();
        assert!(run(&p, &g).is_err());
        assert!(decide(&RouteInputs {
            profile: &p,
            install_dir: Some(&f.install),
            autoexec: None,
            foreground: Some(&f.fg),
        })
        .is_err());
        // 3. 游戏在写入那一行之前就启动了
        let mut g = fixture();
        g.fg.started_ms = Some(at("2026-11-22 00:59:00 +08:00").timestamp_millis());
        assert_eq!(
            run(&p, &g),
            Err("游戏是在打开开关之前启动的，重启游戏后才生效")
        );
        // 4. 前台不是这个游戏（或是另一份安装）
        let mut g = fixture();
        g.fg.name = "notepad.exe".into();
        assert_eq!(run(&p, &g), Err("前台不是这个游戏"));
        let mut g = fixture();
        g.fg.exe = Some(PathBuf::from(
            r"E:\other\dota 2 beta\game\bin\win64\dota2.exe",
        ));
        assert!(run(&p, &g).is_err());
        assert!(decide(&RouteInputs {
            profile: &p,
            install_dir: Some(&f.install),
            autoexec: Some(&f.autoexec),
            foreground: None,
        })
        .is_err());
    }

    #[test]
    fn profile_without_the_section_or_command_stays_on_the_keyboard() {
        let f = fixture();
        assert!(
            run(&GameProfile::builtin_dota2(), &f).is_err(),
            "builtin has no consoleChat"
        );
        let mut p = dota();
        p.channels[0].console_command = None;
        assert_eq!(run(&p, &f), Err("这个频道没有控制台命令"));
        // 只打字不发送（send=false 清空了发送键）：say 会直接发出去，所以走键盘
        let mut p = dota();
        p.send_key.clear();
        assert_eq!(run(&p, &f), Err("这次只打字不发送"));
        // 玩家把绑定改成了别的键
        let mut g = fixture();
        g.autoexec = with_bind(b"", &bind_line("F22", at("2026-11-22 01:00:00 +08:00")));
        assert_eq!(run(&dota(), &g), Err("autoexec.cfg 里绑的不是触发键"));
    }

    #[test]
    fn path_inside_needs_a_separator_boundary() {
        let dir = Path::new(r"D:\Games\dota 2 beta");
        assert!(path_inside(
            Path::new(r"d:\games\DOTA 2 BETA\game\dota2.exe"),
            dir
        ));
        assert!(path_inside(
            Path::new(r"\\?\D:\Games\dota 2 beta\x.exe"),
            dir
        ));
        assert!(!path_inside(Path::new(r"D:\Games\dota 2 beta2\x.exe"), dir));
        assert!(!path_inside(dir, dir));
    }

    #[test]
    fn backups_keep_the_drive_and_path() {
        let dest = backup_path(
            Path::new(r"C:\home\agent\backups"),
            42,
            Path::new(r"D:\SteamLibrary\steamapps\common\dota 2 beta\game\dota\cfg\autoexec.cfg"),
        );
        assert_eq!(
            dest,
            PathBuf::from(
                r"C:\home\agent\backups\42\D\SteamLibrary\steamapps\common\dota 2 beta\game\dota\cfg\autoexec.cfg"
            )
        );
    }
}
