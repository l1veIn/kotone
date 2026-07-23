//! Windows 注入实现：raw `windows` crate 直调 Win32，时序 1:1 对齐 LeagueAkari
//! `native/win32-x64/src/input/input.cc`（docs/tech-research.md §5.0）。
//!
//! 与 input.cc 的对应关系：
//! - `sendString`（KEYEVENTF_UNICODE 逐 UTF-16 code unit down+up）→ `send_unicode`
//! - `sendKey`（VK + MapVirtualKeyW scan code，down → 20ms → up）→ `key_down_up`
//! - `IsProcessForeground`（GetForegroundWindow + GetWindowThreadProcessId）→ `is_process_foreground`
//! - `getPidsByName`（Toolhelp 快照拿进程名）→ `process_name_from_pid`
//!
//! 合规红线：只做 SendInput + 剪贴板；不读写游戏内存、不 hook 渲染、不做驱动注入。

use std::time::Duration;

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
    TH32CS_SNAPPROCESS,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    MapVirtualKeyW, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS,
    KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, MAPVK_VK_TO_VSC, VIRTUAL_KEY, VK_BACK, VK_CONTROL,
    VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE, VK_F1, VK_F10, VK_F11, VK_F12, VK_F2, VK_F3, VK_F4,
    VK_F5, VK_F6, VK_F7, VK_F8, VK_F9, VK_HOME, VK_LEFT, VK_MENU, VK_RETURN, VK_RIGHT,
    VK_SHIFT, VK_SPACE, VK_TAB, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

use super::{send_sequence, CancelToken, InjectError, Injector, SendOps};
use crate::profile::GameProfile;

/// LeagueAkari pressEnter 实测值：sendKey(down) → sleep(20) → sendKey(up)
const KEY_HOLD_MS: u64 = 20;
/// 单次 SendInput 批量的 UTF-16 code unit 上限（每单元 2 个事件）
const UNICODE_CHUNK_UNITS: usize = 64;

/// 生产注入器：前台进程硬校验（否则明确报错「游戏不在前台」）→ §6 发送时序
pub struct WindowsInjector;

impl Injector for WindowsInjector {
    fn send(
        &self,
        text: &str,
        profile: &GameProfile,
        cancel: CancelToken,
    ) -> Result<(), InjectError> {
        if cancel.is_cancelled() {
            return Err(InjectError::new("发送已取消"));
        }
        if !is_process_foreground(&profile.process_names) {
            let target = if profile.process_names.is_empty() {
                "任意前台窗口".to_string()
            } else {
                profile.process_names.join(" / ")
            };
            let foreground = foreground_process_name().unwrap_or_else(|| "（无法获取）".into());
            let message = format!(
                "游戏不在前台：目标进程「{target}」未处于前台（当前前台：{foreground}），已中止发送"
            );
            // UIPI（§10 R-1）：目标进程权限高于自身时，前台切换失败多半由此引起，
            // 追加提权提示并置 needsElevation，前端据此引导管理员重启
            return Err(if needs_elevation_for(&profile.process_names) {
                InjectError::with_needs_elevation(format!(
                    "{message}；目标游戏正以管理员权限运行，请在设置页将 Kotone 以管理员身份重启"
                ))
            } else {
                InjectError::new(message)
            });
        }
        send_sequence(text, profile, &cancel, &WinOps)
    }
}

/// 目标进程权限是否高于自身（UIPI 判定）：
/// 按 profile 进程名找到运行中的 pid → TokenElevation 探测 → 纯逻辑判定。
/// 进程未运行 / 无法判断时不误报。
fn needs_elevation_for(process_names: &[String]) -> bool {
    let target_elevated = process_names
        .iter()
        .find_map(|name| find_pid_by_name(name))
        .and_then(crate::elevation::is_process_elevated);
    crate::elevation::decide_needs_elevation(target_elevated, crate::elevation::is_elevated())
}

/// 真实 SendOps：发 Win32 事件 / 操作剪贴板 / 真睡眠
struct WinOps;

impl SendOps for WinOps {
    fn key_down_up(&self, key: &str) -> Result<(), InjectError> {
        key_down_up(key)
    }
    fn send_unicode(&self, text: &str) -> Result<(), InjectError> {
        send_unicode(text)
    }
    fn clipboard_read(&self) -> Option<String> {
        let mut cb = arboard::Clipboard::new().ok()?;
        cb.get_text().ok()
    }
    fn clipboard_write(&self, text: &str) -> Result<(), InjectError> {
        let mut cb = arboard::Clipboard::new()
            .map_err(|e| InjectError::new(format!("打开剪贴板失败: {e}")))?;
        cb.set_text(text)
            .map_err(|e| InjectError::new(format!("写入剪贴板失败: {e}")))
    }
    fn sleep_ms(&self, ms: u64) {
        std::thread::sleep(Duration::from_millis(ms));
    }
}

// ---------- 原语 1：Unicode 逐字输入 ----------

/// `str::encode_utf16()` 逐 code unit 发 KEYEVENTF_UNICODE down+up；
/// 代理对（emoji 等）按 u16 单元遍历天然正确（对齐 input.cc 的 u16string 处理）。
pub fn send_unicode(text: &str) -> Result<(), InjectError> {
    if text.is_empty() {
        return Ok(());
    }
    let units: Vec<u16> = text.encode_utf16().collect();
    for chunk in units.chunks(UNICODE_CHUNK_UNITS) {
        let mut inputs: Vec<INPUT> = Vec::with_capacity(chunk.len() * 2);
        for &unit in chunk {
            inputs.push(unicode_input(unit, false));
            inputs.push(unicode_input(unit, true));
        }
        send_inputs(&inputs)?;
    }
    Ok(())
}

fn unicode_input(unit: u16, up: bool) -> INPUT {
    let mut flags = KEYEVENTF_UNICODE;
    if up {
        flags |= KEYEVENTF_KEYUP;
    }
    keyboard_input(VIRTUAL_KEY(0), unit, flags)
}

// ---------- 原语 2：单键 / 组合键 down+up ----------

/// 解析键名（如 "Enter"、"Ctrl+V"）并发送 down+up；
/// scan code 经 MapVirtualKeyW(MAPVK_VK_TO_VSC)，比只填 wVk 更接近真实键盘。
/// down 成功后一定补 up（包括中途出错），不留悬键。
pub fn key_down_up(spec: &str) -> Result<(), InjectError> {
    let (mods, main) = parse_key_spec(spec)?;
    let mut first_err: Option<InjectError> = None;

    for &vk in &mods {
        if let Err(e) = vk_event(vk, true) {
            first_err.get_or_insert(e);
        }
    }
    let main_down = vk_event(main, true);
    if let Err(e) = main_down {
        // 主键没按下：只需把已按下的修饰键补 up
        for &vk in mods.iter().rev() {
            let _ = vk_event(vk, false);
        }
        return Err(e);
    }
    std::thread::sleep(Duration::from_millis(KEY_HOLD_MS));
    if let Err(e) = vk_event(main, false) {
        first_err.get_or_insert(e);
    }
    for &vk in mods.iter().rev() {
        if let Err(e) = vk_event(vk, false) {
            first_err.get_or_insert(e);
        }
    }
    match first_err {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

fn vk_event(vk: VIRTUAL_KEY, down: bool) -> Result<(), InjectError> {
    let scan = unsafe { MapVirtualKeyW(u32::from(vk.0), MAPVK_VK_TO_VSC) } as u16;
    let flags = if down {
        KEYBD_EVENT_FLAGS(0)
    } else {
        KEYEVENTF_KEYUP
    };
    send_inputs(&[keyboard_input(vk, scan, flags)])
}

fn keyboard_input(vk: VIRTUAL_KEY, scan: u16, flags: KEYBD_EVENT_FLAGS) -> INPUT {
    let mut input = INPUT::default();
    input.r#type = INPUT_KEYBOARD;
    input.Anonymous = INPUT_0 {
        ki: KEYBDINPUT {
            wVk: vk,
            wScan: scan,
            dwFlags: flags,
            time: 0,
            dwExtraInfo: 0,
        },
    };
    input
}

fn send_inputs(inputs: &[INPUT]) -> Result<(), InjectError> {
    let sent = unsafe { SendInput(inputs, std::mem::size_of::<INPUT>() as i32) };
    if sent as usize == inputs.len() {
        Ok(())
    } else {
        let err = windows::core::Error::from_win32();
        Err(InjectError::new(format!(
            "SendInput 被系统拦截（{}/{} 成功）: {err}",
            sent,
            inputs.len()
        )))
    }
}

// ---------- 键名解析（纯逻辑，可单测） ----------

/// 解析 "Enter" / "Ctrl+V" 形式：返回（修饰键列表, 主键）
fn parse_key_spec(spec: &str) -> Result<(Vec<VIRTUAL_KEY>, VIRTUAL_KEY), InjectError> {
    let parts: Vec<&str> = spec
        .split('+')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    let (main, mods) = parts
        .split_last()
        .ok_or_else(|| InjectError::new(format!("键名「{spec}」为空")))?;
    let main_vk = vk_from_name(main)
        .ok_or_else(|| InjectError::new(format!("无法识别的键名「{main}」（完整输入：{spec}）")))?;
    let mut mod_vks = Vec::with_capacity(mods.len());
    for m in mods {
        mod_vks.push(
            vk_from_name(m).ok_or_else(|| {
                InjectError::new(format!("无法识别的修饰键「{m}」（完整输入：{spec}）"))
            })?,
        );
    }
    Ok((mod_vks, main_vk))
}

fn vk_from_name(name: &str) -> Option<VIRTUAL_KEY> {
    let n = name.trim();
    if n.chars().count() == 1 {
        let c = n.chars().next()?.to_ascii_uppercase();
        match c {
            'A'..='Z' | '0'..='9' => return Some(VIRTUAL_KEY(c as u16)),
            _ => {}
        }
    }
    let vk = match n.to_ascii_lowercase().as_str() {
        "enter" | "return" => VK_RETURN,
        "esc" | "escape" => VK_ESCAPE,
        "tab" => VK_TAB,
        "space" => VK_SPACE,
        "backspace" => VK_BACK,
        "delete" | "del" => VK_DELETE,
        "shift" => VK_SHIFT,
        "ctrl" | "control" => VK_CONTROL,
        "alt" => VK_MENU,
        "up" => VK_UP,
        "down" => VK_DOWN,
        "left" => VK_LEFT,
        "right" => VK_RIGHT,
        "home" => VK_HOME,
        "end" => VK_END,
        "f1" => VK_F1,
        "f2" => VK_F2,
        "f3" => VK_F3,
        "f4" => VK_F4,
        "f5" => VK_F5,
        "f6" => VK_F6,
        "f7" => VK_F7,
        "f8" => VK_F8,
        "f9" => VK_F9,
        "f10" => VK_F10,
        "f11" => VK_F11,
        "f12" => VK_F12,
        _ => return None,
    };
    Some(vk)
}

// ---------- 原语 3：前台进程校验 ----------

/// 发送前硬性校验：目标游戏进程必须为前台进程。
/// process_names 为空 = 通配任意前台进程（恒 true）；大小写不敏感。
pub fn is_process_foreground(process_names: &[String]) -> bool {
    if process_names.is_empty() {
        return true;
    }
    match foreground_process_name() {
        Some(name) => process_names.iter().any(|p| p.eq_ignore_ascii_case(&name)),
        None => false,
    }
}

/// 当前前台窗口所属进程的可执行文件名（如 "notepad.exe"）
pub fn foreground_process_name() -> Option<String> {
    process_name_from_pid(foreground_pid()?)
}

/// 当前前台窗口所属进程 PID（无前台窗口时为 None）
pub fn foreground_pid() -> Option<u32> {
    let mut pid: u32 = 0;
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.0.is_null() {
            return None;
        }
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
    }
    if pid == 0 {
        None
    } else {
        Some(pid)
    }
}

/// Toolhelp 快照按 PID 查进程名（对齐 LeagueAkari tools.cc 的进程枚举路径）
pub fn process_name_from_pid(pid: u32) -> Option<String> {
    unsafe {
        let snapshot: HANDLE = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0).ok()?;
        let result = (|| {
            let mut entry = PROCESSENTRY32W::default();
            entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
            Process32FirstW(snapshot, &mut entry).ok()?;
            loop {
                if entry.th32ProcessID == pid {
                    let len = entry
                        .szExeFile
                        .iter()
                        .position(|&c| c == 0)
                        .unwrap_or(entry.szExeFile.len());
                    return Some(String::from_utf16_lossy(&entry.szExeFile[..len]));
                }
                if Process32NextW(snapshot, &mut entry).is_err() {
                    return None;
                }
            }
        })();
        let _ = CloseHandle(snapshot);
        result
    }
}

/// Toolhelp 快照按可执行文件名（大小写不敏感）查 PID；取首个匹配
pub fn find_pid_by_name(name: &str) -> Option<u32> {
    unsafe {
        let snapshot: HANDLE = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0).ok()?;
        let result = (|| {
            let mut entry = PROCESSENTRY32W::default();
            entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
            Process32FirstW(snapshot, &mut entry).ok()?;
            loop {
                let len = entry
                    .szExeFile
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(entry.szExeFile.len());
                let exe = String::from_utf16_lossy(&entry.szExeFile[..len]);
                if exe.eq_ignore_ascii_case(name) {
                    return Some(entry.th32ProcessID);
                }
                if Process32NextW(snapshot, &mut entry).is_err() {
                    return None;
                }
            }
        })();
        let _ = CloseHandle(snapshot);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vk_from_name_common_keys() {
        assert_eq!(vk_from_name("Enter"), Some(VK_RETURN));
        assert_eq!(vk_from_name("return"), Some(VK_RETURN));
        assert_eq!(vk_from_name("Esc"), Some(VK_ESCAPE));
        assert_eq!(vk_from_name("Space"), Some(VK_SPACE));
        assert_eq!(vk_from_name("Ctrl"), Some(VK_CONTROL));
        assert_eq!(vk_from_name("Control"), Some(VK_CONTROL));
        assert_eq!(vk_from_name("Alt"), Some(VK_MENU));
        assert_eq!(vk_from_name("F8"), Some(VK_F8));
        assert_eq!(vk_from_name("v"), Some(VIRTUAL_KEY(u16::from(b'V'))));
        assert_eq!(vk_from_name("A"), Some(VIRTUAL_KEY(u16::from(b'A'))));
        assert_eq!(vk_from_name("7"), Some(VIRTUAL_KEY(u16::from(b'7'))));
    }

    #[test]
    fn vk_from_name_rejects_unknown() {
        assert_eq!(vk_from_name("NotAKey"), None);
        assert_eq!(vk_from_name("回车"), None);
        assert_eq!(vk_from_name(""), None);
    }

    #[test]
    fn parse_single_key() {
        let (mods, main) = parse_key_spec("Enter").unwrap();
        assert!(mods.is_empty());
        assert_eq!(main, VK_RETURN);
    }

    #[test]
    fn parse_combo_key() {
        let (mods, main) = parse_key_spec("Ctrl+V").unwrap();
        assert_eq!(mods, vec![VK_CONTROL]);
        assert_eq!(main, VIRTUAL_KEY(u16::from(b'V')));

        let (mods, main) = parse_key_spec("ctrl + shift + enter").unwrap();
        assert_eq!(mods, vec![VK_CONTROL, VK_SHIFT]);
        assert_eq!(main, VK_RETURN);
    }

    #[test]
    fn parse_invalid_key_errors() {
        assert!(parse_key_spec("").is_err());
        assert!(parse_key_spec("Ctrl+NotAKey").is_err());
        assert!(parse_key_spec("NotAKey+V").is_err());
    }

    #[test]
    fn foreground_process_name_is_available() {
        // 测试进程运行时必有某个前台窗口所属的进程可被枚举（也可能无前台窗口，
        // 此时为 None 属合法）；这里只验证函数不 panic、返回类型正确。
        let name = foreground_process_name();
        if let Some(n) = &name {
            assert!(!n.is_empty());
        }
        // 空列表 = 通配，恒 true
        assert!(is_process_foreground(&[]));
    }

    #[test]
    fn find_pid_by_name_finds_self() {
        // 用自身 exe 名验证 Toolhelp 反查链路
        let exe = std::env::current_exe().unwrap();
        let name = exe.file_name().unwrap().to_string_lossy().to_string();
        let pid = find_pid_by_name(&name);
        assert_eq!(pid, Some(std::process::id()));
        // 不存在的进程名 → None（不误报）
        assert!(find_pid_by_name("kotone-definitely-not-running-xyz.exe").is_none());
    }

    #[test]
    fn not_foreground_error_carries_elevation_hint_only_when_target_elevated() {
        // 目标进程不存在 → 无法判断权限 → 不带 needsElevation（不发任何按键，安全）
        let mut profile = GameProfile::builtin_generic();
        profile.process_names = vec!["kotone-definitely-not-running-xyz.exe".into()];
        let err = WindowsInjector
            .send("hi", &profile, CancelToken::default())
            .unwrap_err();
        assert!(err.message.contains("游戏不在前台"));
        assert!(
            !err.needs_elevation,
            "目标进程不存在时不应误报提权: {}",
            err.message
        );
        assert!(!err.message.contains("管理员"));
    }
}
