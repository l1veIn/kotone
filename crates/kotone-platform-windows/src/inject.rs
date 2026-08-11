//! Windows 注入生产实现：raw `windows` crate 直调 Win32（SendInput / 焦点恢复）。
//! 端口（Injector / FocusBackend / SendOps / send_sequence）在 kotone-core。
//! 非 Windows 编译为保持可编译的兜底实现（运行时明确报错，MVP Windows-first）。

#[cfg(windows)]
mod windows_imp {
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

    use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND};
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        MapVirtualKeyW, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS,
        KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, MAPVK_VK_TO_VSC, VIRTUAL_KEY, VK_BACK, VK_CONTROL,
        VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE, VK_F1, VK_F10, VK_F11, VK_F12, VK_F2, VK_F3, VK_F4,
        VK_F5, VK_F6, VK_F7, VK_F8, VK_F9, VK_HOME, VK_LEFT, VK_MENU, VK_RETURN, VK_RIGHT,
        VK_SHIFT, VK_SPACE, VK_TAB, VK_UP,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindowThreadProcessId, IsWindow, SetForegroundWindow,
    };

    use kotone_core::inject::{
        send_sequence, CancelToken, FocusBackend, InjectError, Injector, SendOps, TargetWindow,
    };
    use kotone_core::profile::GameProfile;

    /// LeagueAkari pressEnter 实测值：sendKey(down) → sleep(20) → sendKey(up)
    const KEY_HOLD_MS: u64 = 20;
    /// 单次 SendInput 批量的 UTF-16 code unit 上限（每单元 2 个事件）
    const UNICODE_CHUNK_UNITS: usize = 64;

    /// 生产注入器：§6 发送时序直发当前前台窗口。
    /// 前台守卫已彻底移除（用户拍板）：热键触发时前台是什么窗口就往哪发——
    /// 用户在记事本测试 LoL profile 也必须能发出去；进程名匹配仅用于
    /// 「按前台游戏自动激活 profile」这类不阻塞发送的用途（profile::find_by_process）。
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
            send_sequence(text, profile, &cancel, &WinOps)
        }
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
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: vk,
                    wScan: scan,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }

    fn send_inputs(inputs: &[INPUT]) -> Result<(), InjectError> {
        let sent = unsafe { SendInput(inputs, std::mem::size_of::<INPUT>() as i32) };
        if sent as usize == inputs.len() {
            Ok(())
        } else {
            let err = windows::core::Error::from_win32();
            // 0/N 全部未落地 = 系统级拦截。两类成因需要不同指引：
            // 1) UIPI：目标进程完整性级别更高（如管理员运行的游戏），SendInput 被
            //    拒绝——Win32 文档明示返回值与 GetLastError 均不保证指示 UIPI，但
            //    ERROR_ACCESS_DENIED 出现时优先归因提权场景（提权可解）；
            // 2) 安全软件主动防御 / 反作弊内核层拦截——与目标窗口无关，连记事本都
            //    会失败，单独标记让前端弹窗引导排查（提权无法解决）。
            // 部分落地（N>0）仍是普通失败（时序/焦点干扰）。
            if sent == 0 {
                use windows::Win32::Foundation::ERROR_ACCESS_DENIED;
                if err.code().0 as u32 == ERROR_ACCESS_DENIED.0 {
                    return Err(InjectError::with_needs_elevation(format!(
                        "SendInput 被拒绝（0/{} 成功）：目标程序可能以更高权限运行\
                         （如管理员模式启动的游戏）。请以管理员身份运行 Kotone，\
                         或将目标程序改为普通权限: {err}",
                        inputs.len()
                    )));
                }
                return Err(InjectError::with_input_blocked(format!(
                    "SendInput 被系统拦截（0/{} 成功），通常是安全软件或游戏反作弊拦截了模拟输入: {err}",
                    inputs.len()
                )));
            }
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
        let main_vk = vk_from_name(main).ok_or_else(|| {
            InjectError::new(format!("无法识别的键名「{main}」（完整输入：{spec}）"))
        })?;
        let mut mod_vks = Vec::with_capacity(mods.len());
        for m in mods {
            mod_vks.push(vk_from_name(m).ok_or_else(|| {
                InjectError::new(format!("无法识别的修饰键「{m}」（完整输入：{spec}）"))
            })?);
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

    // ---------- 原语 3：前台进程查询（诊断/通配用，不阻塞发送） ----------

    /// 前台进程是否匹配进程名列表（大小写不敏感；空列表 = 通配恒 true）。
    /// 保留作诊断与「按前台游戏匹配 profile」类非阻塞用途；发送路径已不做
    /// 前台守卫（见 WindowsInjector 注释）。
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
                let mut entry = PROCESSENTRY32W {
                    dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
                    ..Default::default()
                };
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

    // ---------- 原语 4：目标窗口记忆与焦点恢复 ----------

    /// 生产焦点后端：GetForegroundWindow 捕获 + SetForegroundWindow 恢复
    /// （失败时 AttachThreadInput 附加到前台/目标线程后重试）。
    pub struct WinFocusBackend;

    impl FocusBackend for WinFocusBackend {
        fn foreground_window(&self) -> Option<TargetWindow> {
            let hwnd = unsafe { GetForegroundWindow() };
            if hwnd.0.is_null() {
                None
            } else {
                Some(TargetWindow(hwnd.0 as usize))
            }
        }

        fn restore(&self, target: TargetWindow) -> bool {
            let hwnd = HWND(target.0 as *mut std::ffi::c_void);
            unsafe {
                // 窗口已关闭/失效：由调用方继续直发当前前台（前台守卫已移除）
                if !IsWindow(Some(hwnd)).as_bool() {
                    return false;
                }
                if SetForegroundWindow(hwnd).as_bool() {
                    return true;
                }
                // SetForegroundWindow 受限（焦点归属别的线程）：把本线程输入队列
                // 附加到前台线程与目标线程，取得前台切换权限后重试
                let cur = GetCurrentThreadId();
                let fg = GetForegroundWindow();
                let fg_tid = if fg.0.is_null() {
                    0
                } else {
                    GetWindowThreadProcessId(fg, None)
                };
                let target_tid = GetWindowThreadProcessId(hwnd, None);
                let attached_fg =
                    fg_tid != 0 && fg_tid != cur && AttachThreadInput(cur, fg_tid, true).as_bool();
                let attached_target = target_tid != 0
                    && target_tid != cur
                    && AttachThreadInput(cur, target_tid, true).as_bool();

                let ok = SetForegroundWindow(hwnd).as_bool();

                if attached_fg {
                    let _ = AttachThreadInput(cur, fg_tid, false);
                }
                if attached_target {
                    let _ = AttachThreadInput(cur, target_tid, false);
                }
                ok
            }
        }
    }

    /// Toolhelp 快照按可执行文件名（大小写不敏感）查 PID；取首个匹配
    pub fn find_pid_by_name(name: &str) -> Option<u32> {
        unsafe {
            let snapshot: HANDLE = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0).ok()?;
            let result = (|| {
                let mut entry = PROCESSENTRY32W {
                    dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
                    ..Default::default()
                };
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
    }
}

#[cfg(windows)]
pub use windows_imp::*;

/// 非 Windows 兜底：保持可编译，运行时返回明确错误（macOS/Linux 注入见 §3.7 平台策略）
#[cfg(not(windows))]
mod fallback {
    use kotone_core::inject::{CancelToken, FocusBackend, InjectError, Injector, TargetWindow};
    use kotone_core::profile::GameProfile;

    pub fn send_unicode(_text: &str) -> Result<(), InjectError> {
        Err(InjectError::new(
            "send_unicode 仅 Windows 实现（MVP Windows-first）",
        ))
    }

    pub fn key_down_up(_key: &str) -> Result<(), InjectError> {
        Err(InjectError::new(
            "key_down_up 仅 Windows 实现（MVP Windows-first）",
        ))
    }

    pub fn is_process_foreground(process_names: &[String]) -> bool {
        // 空列表 = 通配任意前台进程；非 Windows 无法校验，保守按通配处理
        process_names.is_empty()
    }

    pub fn foreground_process_name() -> Option<String> {
        None
    }

    pub fn foreground_pid() -> Option<u32> {
        None
    }

    pub fn process_name_from_pid(_pid: u32) -> Option<String> {
        None
    }

    pub fn find_pid_by_name(_name: &str) -> Option<u32> {
        None
    }

    /// 非 Windows 兜底焦点后端：无法捕获/恢复，恒返回空
    pub struct WinFocusBackend;

    impl FocusBackend for WinFocusBackend {
        fn foreground_window(&self) -> Option<TargetWindow> {
            None
        }
        fn restore(&self, _target: TargetWindow) -> bool {
            false
        }
    }

    pub struct WindowsInjector;

    impl Injector for WindowsInjector {
        fn send(
            &self,
            _text: &str,
            _profile: &GameProfile,
            cancel: CancelToken,
        ) -> Result<(), InjectError> {
            if cancel.is_cancelled() {
                return Err(InjectError::new("发送已取消"));
            }
            Err(InjectError::new(
                "当前平台暂不支持输入注入（MVP Windows-first）",
            ))
        }
    }
}

#[cfg(not(windows))]
pub use fallback::*;
