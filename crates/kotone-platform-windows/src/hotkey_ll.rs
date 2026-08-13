//! Windows 低级键鼠钩子（WH_KEYBOARD_LL + WH_MOUSE_LL）热键源：
//! core `HotkeySource` 端口的实现。
//!
//! 背景：RegisterHotKey 在 LOL 等游戏前台不投递热键事件（实测日志实证）；
//! LeagueAkari 等游戏工具均用 LL 钩子解决。
//!
//! 线程模型：
//! - 钩子线程：安装 WH_KEYBOARD_LL / WH_MOUSE_LL + GetMessageW 消息循环
//!   （LL 钩子回调跑在安装线程上，该线程必须有消息循环）；
//!   退出时 UnhookWindowsHookEx（PostThreadMessageW(WM_QUIT) 触发）。
//! - 回调红线：只做「过滤 + channel 发送」，绝不写日志/做 IO/调业务；
//!   事件经 std::sync::mpsc 发给消费者线程，由它调用构造时注入的 sink。
//! - 匹配逻辑全部在 core 的 HookMatcher（kotone_core::hotkey），回调只是翻译层。
//!
//! 吞键策略：完整命中（主键 + 修饰键严格匹配）才 return 1 吞掉；
//! 其余按键一律 CallNextHookEx 放行。

#![cfg(windows)]

use std::sync::mpsc;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, MapVirtualKeyW, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT,
    KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, MAPVK_VK_TO_VSC, VIRTUAL_KEY, VK_CONTROL, VK_MENU,
    VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetMessageW, PeekMessageW, PostThreadMessageW, SetWindowsHookExW,
    UnhookWindowsHookEx, HHOOK, KBDLLHOOKSTRUCT, LLKHF_INJECTED, LLMHF_INJECTED, MSG,
    MSLLHOOKSTRUCT, PM_NOREMOVE, WH_KEYBOARD_LL, WH_MOUSE_LL, WM_KEYDOWN, WM_KEYUP, WM_QUIT,
    WM_SYSKEYDOWN, WM_SYSKEYUP, WM_XBUTTONDOWN, WM_XBUTTONUP, XBUTTON1, XBUTTON2,
};

use kotone_core::hotkey::{
    parse_hotkey, HookEvent, HookMatcher, HotkeyMode, HotkeySource, HotkeySpec, KeyAction,
    VK_ESCAPE, VK_XBUTTON1, VK_XBUTTON2,
};

/// 事件出口：构造 LlHookSource 时注入（Tauri 壳 spawn 进 runtime，CLI 送进自己的通道）
pub type HookSink = Box<dyn Fn(HookEvent) + Send + Sync>;

/// 捕获消息（热键录入）：组合键命中 / Esc 取消
enum CaptureMsg {
    Combo(HotkeySpec),
    Cancel,
}

/// 捕获结果（capture_next 回调入参）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureResult {
    /// 用户按下了组合键
    Captured(HotkeySpec),
    /// 用户按 Esc 或调用方 cancel_capture
    Cancelled,
    /// 超时未按键
    Timeout,
}

/// 合成输入预检结论。钩子安装失败或 `SendInput` 明确少发事件时返回 `Err`；
/// 安全软件可能只过滤 F24/标记事件，因此“未回环”不能直接证明真实链路不可用。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeOutcome {
    Verified,
    Inconclusive { observed: usize, expected: usize },
}

/// 钩子回调共享状态：匹配器 + 事件出口（OnceLock：回调必须是静态 fn）
struct HookShared {
    matcher: Mutex<HookMatcher>,
    tx: mpsc::Sender<HookEvent>,
    /// 捕获模式出口：Some 期间按键组合/Esc 走此通道而非正常事件流
    capture: Mutex<Option<mpsc::Sender<CaptureMsg>>>,
    /// 可用性探测出口：带专用 dwExtraInfo 的 F24 被钩子观察到时通知等待方。
    probe: Mutex<Option<ProbeRequest>>,
}

struct ProbeRequest {
    /// 只接受刚安装/刷新的 hook 线程回执，避免旧 hook 在退出竞态中造成假阳性。
    hook_thread_id: u32,
    tx: mpsc::Sender<()>,
}

static SHARED: OnceLock<HookShared> = OnceLock::new();

/// `SendInput` 探测事件的专用标记（ASCII "KTNP"）。优先用于标识探测来源；
/// 兼容会清除 flags/extraInfo 的输入过滤器时，短暂 probe 槽内也接受 F24 键码。
const PROBE_EXTRA_INFO: usize = 0x4B54_4E50;
const PROBE_VK: VIRTUAL_KEY = VIRTUAL_KEY(0x87); // VK_F24

/// LL 钩子热键源：首次 register 时启动钩子线程与消费者线程，
/// 之后改键/改模式只更新共享匹配器配置，不重建线程。
pub struct LlHookSource {
    sink: Arc<HookSink>,
    state: Mutex<LlHookState>,
    /// 覆盖“刷新 hook → 安装 probe 槽 → SendInput → 等回执”的完整临界区。
    probe_lock: Mutex<()>,
}

#[derive(Default)]
struct LlHookState {
    started: bool,
    hook_thread_id: Option<u32>,
    /// 已正式注册热键（register 置 true、unregister 置 false）；
    /// 捕获模式用它区分「占位启动」与「真实注册」，捕获结束后据此恢复禁用
    armed: bool,
}

impl LlHookSource {
    pub fn new(sink: HookSink) -> Self {
        Self {
            sink: Arc::new(sink),
            state: Mutex::new(LlHookState::default()),
            probe_lock: Mutex::new(()),
        }
    }

    /// 停止钩子线程（WM_QUIT → 消息循环退出 → UnhookWindowsHookEx）
    pub fn shutdown(&self) {
        let tid = self.state.lock().unwrap().hook_thread_id.take();
        if let Some(tid) = tid {
            unsafe {
                let _ = PostThreadMessageW(tid, WM_QUIT, WPARAM(0), LPARAM(0));
            }
        }
    }

    /// 闭环检测低级键盘钩子是否真正可用。
    ///
    /// 仅 `SetWindowsHookExW` 成功并不足以证明钩子未被安全软件静默拦截。本探测
    /// 通过 `SendInput` 发送一组带专用标记的 F24 down/up，并等待本钩子回调确认；
    /// 回调会吞掉探测事件，避免传给当前前台窗口。完整回环返回 `Verified`；
    /// `SendInput` 已接受但未回环属于 `Inconclusive`，后续由用户的真实物理按键
    /// 完成判定，避免把“只过滤探测 F24”的环境误报成整个输入链路不可用。
    pub fn probe_available(&self, timeout: Duration) -> Result<ProbeOutcome, String> {
        let _probe_guard = self
            .probe_lock
            .try_lock()
            .map_err(|_| "低级键盘钩子自检已在进行中".to_string())?;
        if let Some(shared) = SHARED.get() {
            if shared.capture.lock().unwrap().is_some() {
                return Err("热键录入进行中，暂时无法执行钩子自检".into());
            }
        }

        let was_started = self.state.lock().unwrap().started;
        if !was_started {
            // 用禁用的 F24 占位 matcher 启动基础设施；探测结束后仍保持未注册状态。
            self.register("F24", HotkeyMode::Toggle)?;
            self.state.lock().unwrap().armed = false;
            if let Some(shared) = SHARED.get() {
                shared.matcher.lock().unwrap().set_enabled(false);
            }
        } else {
            // Windows 可能静默移除超时钩子；每次探测前先换成新 hook。
            self.restart_hook_thread()?;
        }

        let shared = SHARED.get().ok_or("LL 钩子未启动".to_string())?;
        let hook_thread_id = self
            .state
            .lock()
            .unwrap()
            .hook_thread_id
            .ok_or("LL 钩子线程未就绪".to_string())?;
        let (tx, rx) = mpsc::channel::<()>();
        {
            let mut slot = shared.probe.lock().unwrap();
            if slot.is_some() {
                return Err("低级键盘钩子自检已在进行中".into());
            }
            *slot = Some(ProbeRequest { hook_thread_id, tx });
        }

        let inputs = [probe_input(false), probe_input(true)];
        let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
        if sent as usize != inputs.len() {
            // 若只落地了 down，尽力补一个 up，避免残留悬键。
            if sent == 1 {
                let _ =
                    unsafe { SendInput(&[probe_input(true)], std::mem::size_of::<INPUT>() as i32) };
            }
            *shared.probe.lock().unwrap() = None;
            let error = windows::core::Error::from_win32();
            return Err(format!(
                "SendInput 探测事件被系统拦截（{sent}/{} 成功）：{error}",
                inputs.len()
            ));
        }

        // down/up 都必须由同一个新 hook 收到后才算通过；这样清理 probe 槽时不会
        // 把尚未处理的 F24 key-up 泄漏给前台窗口。
        let deadline = Instant::now() + timeout;
        let mut observed = 0;
        while observed < inputs.len() {
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                break;
            };
            if rx.recv_timeout(remaining).is_err() {
                break;
            }
            observed += 1;
        }
        *shared.probe.lock().unwrap() = None;
        if observed == inputs.len() {
            kotone_core::log::log("WH_KEYBOARD_LL availability probe passed");
            Ok(ProbeOutcome::Verified)
        } else {
            Ok(ProbeOutcome::Inconclusive {
                observed,
                expected: inputs.len(),
            })
        }
    }

    /// 捕获下一个按键组合（热键录入：设置页「点击录入」/ CLI --capture）。
    ///
    /// 非阻塞：装槽后起 waiter 线程等结果（recv_timeout），结果经 `cb` 回调。
    /// 未注册过热键时用占位配置（F24/Toggle，捕获优先于匹配）启动钩子
    /// 基础设施，捕获结束后恢复禁用。已有捕获进行中返回 Err。
    pub fn capture_next(
        &self,
        cb: Box<dyn Fn(CaptureResult) + Send + Sync>,
        timeout: Duration,
    ) -> Result<(), String> {
        // 并发守卫：已有捕获进行中
        if let Some(shared) = SHARED.get() {
            if shared.capture.lock().unwrap().is_some() {
                return Err("已有热键捕获进行中".into());
            }
        }

        // SendInput 明确少发事件时立即失败；合成 F24 未回环只记诊断，
        // 随后的真实物理按键才是 LL Hook 可用性的最终判据。
        if let ProbeOutcome::Inconclusive { observed, expected } =
            self.probe_available(Duration::from_millis(250))?
        {
            kotone_core::log::log(&format!(
                "WH_KEYBOARD_LL synthetic probe inconclusive ({observed}/{expected}); \
                 continuing with physical-key capture"
            ));
        }
        let was_armed = self.state.lock().unwrap().armed;

        let shared = SHARED.get().ok_or("LL 钩子未启动".to_string())?;
        let (tx, rx) = mpsc::channel::<CaptureMsg>();
        {
            let mut slot = shared.capture.lock().unwrap();
            if slot.is_some() {
                return Err("已有热键捕获进行中".into());
            }
            *slot = Some(tx);
        }
        shared.matcher.lock().unwrap().set_capture_active(true);
        kotone_core::log::log("llhook capture started");

        std::thread::Builder::new()
            .name("kotone-llhook-capture".into())
            .spawn(move || {
                let result = match rx.recv_timeout(timeout) {
                    Ok(CaptureMsg::Combo(spec)) => CaptureResult::Captured(spec),
                    Ok(CaptureMsg::Cancel) => CaptureResult::Cancelled,
                    Err(_) => CaptureResult::Timeout,
                };
                // 清理：关捕获模式 + 清槽；未正式注册过则恢复禁用
                if let Some(shared) = SHARED.get() {
                    shared.matcher.lock().unwrap().set_capture_active(false);
                    *shared.capture.lock().unwrap() = None;
                    if !was_armed {
                        shared.matcher.lock().unwrap().set_enabled(false);
                    }
                }
                kotone_core::log::log(&format!("llhook capture ended: {result:?}"));
                (cb)(result);
            })
            .map_err(|e| format!("启动捕获线程失败: {e}"))?;
        Ok(())
    }

    /// 取消进行中的捕获（设置页关闭/超时兜底）：waiter 将收到 Cancelled
    pub fn cancel_capture(&self) {
        if let Some(shared) = SHARED.get() {
            if let Some(tx) = shared.capture.lock().unwrap().take() {
                let _ = tx.send(CaptureMsg::Cancel);
            }
        }
    }

    /// 在保持共享 matcher / 事件消费者不变的前提下重建键盘与鼠标 LL 钩子。
    ///
    /// 先确认新 hook 安装成功，再通知旧消息循环退出，因此重建失败不会把仍可用
    /// 的旧 hook 一并拆掉。短暂重叠期间 matcher 的按下态会自然去重事件。
    fn restart_hook_thread(&self) -> Result<(), String> {
        let old_tid = self.state.lock().unwrap().hook_thread_id;
        let new_tid = spawn_hook_thread()?;
        {
            let mut state = self.state.lock().unwrap();
            state.started = true;
            state.hook_thread_id = Some(new_tid);
        }
        if let Some(tid) = old_tid {
            if tid != new_tid {
                unsafe {
                    let _ = PostThreadMessageW(tid, WM_QUIT, WPARAM(0), LPARAM(0));
                }
            }
        }
        kotone_core::log::log("WH_KEYBOARD_LL + WH_MOUSE_LL hooks refreshed");
        Ok(())
    }
}

impl HotkeySource for LlHookSource {
    /// 注册/改键：首次调用安装钩子并启动事件泵；失败返回 Err（调用方回退 RegisterHotKey）
    fn register(&self, key: &str, mode: HotkeyMode) -> Result<(), String> {
        let spec = parse_hotkey(key)
            .ok_or_else(|| format!("无法解析热键「{key}」（LL 钩子后端不支持该键名）"))?;

        let mut state = self.state.lock().unwrap();
        if state.started {
            // 钩子已在跑：只更新匹配配置
            if let Some(shared) = SHARED.get() {
                shared.matcher.lock().unwrap().set_config(spec, mode);
            }
            state.armed = true;
            kotone_core::log::log(&format!("llhook reconfigured: {key} ({mode:?})"));
            drop(state);
            self.restart_hook_thread()?;
            return Ok(());
        }

        // 首次启动：事件通道 + 钩子线程 + 消费者线程
        let (tx, rx) = mpsc::channel::<HookEvent>();
        let shared = HookShared {
            matcher: Mutex::new(HookMatcher::new(spec, mode)),
            tx,
            capture: Mutex::new(None),
            probe: Mutex::new(None),
        };
        SHARED
            .set(shared)
            .map_err(|_| "LL 钩子共享状态已初始化（内部错误）".to_string())?;

        let thread_id = spawn_hook_thread()?;

        let sink = self.sink.clone();
        std::thread::Builder::new()
            .name("kotone-llhook-events".into())
            .spawn(move || consumer_main(rx, sink))
            .map_err(|e| format!("启动钩子事件线程失败: {e}"))?;

        state.started = true;
        state.armed = true;
        state.hook_thread_id = Some(thread_id);
        kotone_core::log::log(&format!("llhook backend started: {key} ({mode:?})"));
        Ok(())
    }

    /// 注销：匹配器置为禁用（全部按键放行），钩子线程保留以便快速重注册
    fn unregister(&self) {
        self.state.lock().unwrap().armed = false;
        if let Some(shared) = SHARED.get() {
            shared.matcher.lock().unwrap().set_enabled(false);
            kotone_core::log::log("llhook unregistered (matcher disabled)");
        }
    }

    /// 会话激活标志：Esc 取消使能
    fn set_cancel_active(&self, active: bool) {
        if let Some(shared) = SHARED.get() {
            shared.matcher.lock().unwrap().set_session_active(active);
        }
    }

    /// 频道切换键（ADR-008）：更新匹配器的切换 spec；None/解析失败 = 关闭
    fn set_cycle_key(&self, key: Option<&str>) -> Result<(), String> {
        let spec = match key {
            Some(k) if !k.trim().is_empty() => Some(parse_hotkey(k).ok_or_else(|| {
                format!("无法解析频道切换热键「{k}」（LL 钩子后端不支持该键名）")
            })?),
            _ => None,
        };
        if let Some(shared) = SHARED.get() {
            shared.matcher.lock().unwrap().set_cycle_spec(spec);
        }
        kotone_core::log::log(&format!("llhook cycle key: {spec:?}"));
        Ok(())
    }

    /// 重发最近一条热键：更新匹配器的重发 spec；None/解析失败 = 关闭
    fn set_resend_key(&self, key: Option<&str>) -> Result<(), String> {
        let spec = match key {
            Some(k) if !k.trim().is_empty() => Some(
                parse_hotkey(k)
                    .ok_or_else(|| format!("无法解析重发热键「{k}」（LL 钩子后端不支持该键名）"))?,
            ),
            _ => None,
        };
        if let Some(shared) = SHARED.get() {
            shared.matcher.lock().unwrap().set_resend_spec(spec);
        }
        kotone_core::log::log(&format!("llhook resend key: {spec:?}"));
        Ok(())
    }
}

fn spawn_hook_thread() -> Result<u32, String> {
    let (boot_tx, boot_rx) = mpsc::channel::<Result<u32, String>>();
    std::thread::Builder::new()
        .name("kotone-llhook".into())
        .spawn(move || hook_thread_main(boot_tx))
        .map_err(|e| format!("启动钩子线程失败: {e}"))?;

    match boot_rx.recv() {
        Ok(Ok(tid)) => Ok(tid),
        Ok(Err(e)) => Err(format!("安装低层键鼠钩子失败: {e}")),
        Err(_) => Err("钩子线程启动后异常退出".into()),
    }
}

impl Drop for LlHookSource {
    fn drop(&mut self) {
        // 进程退出路径：通知钩子线程退出并卸钩（系统也会在进程结束时兜底清理）
        self.shutdown();
    }
}

/// 钩子线程主体：安装键盘/鼠标钩子 → 消息循环 → 退出时卸钩
fn hook_thread_main(boot: mpsc::Sender<Result<u32, String>>) {
    unsafe {
        let tid = GetCurrentThreadId();
        // WH_KEYBOARD_LL 的回调通过安装线程的消息泵分发。先显式创建消息队列，
        // 避免调用方收到 ready 后立即发送探测/用户立即按键，而线程尚无消息队列。
        let mut msg = MSG::default();
        let _ = PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE);
        let keyboard_hook =
            match SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), None::<HINSTANCE>, 0) {
                Ok(h) => h,
                Err(e) => {
                    let _ = boot.send(Err(format!("SetWindowsHookExW(WH_KEYBOARD_LL): {e}")));
                    return;
                }
            };
        let mouse_hook =
            match SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook_proc), None::<HINSTANCE>, 0) {
                Ok(h) => h,
                Err(e) => {
                    let _ = UnhookWindowsHookEx(keyboard_hook);
                    let _ = boot.send(Err(format!("SetWindowsHookExW(WH_MOUSE_LL): {e}")));
                    return;
                }
            };
        // ready 之前完成日志等可能阻塞的工作；收到 ready 后线程下一步立即进入
        // GetMessageW，不能留出“hook 已返回成功但消息泵尚未工作”的竞态窗口。
        kotone_core::log::log("WH_KEYBOARD_LL + WH_MOUSE_LL hooks installed");
        let _ = boot.send(Ok(tid));

        // GetMessageW 返回 false = 收到 WM_QUIT
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {}

        let _ = UnhookWindowsHookEx(mouse_hook);
        let _ = UnhookWindowsHookEx(keyboard_hook);
        kotone_core::log::log("WH_KEYBOARD_LL + WH_MOUSE_LL hooks uninstalled");
    }
}

fn probe_input(up: bool) -> INPUT {
    let scan = unsafe { MapVirtualKeyW(u32::from(PROBE_VK.0), MAPVK_VK_TO_VSC) } as u16;
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: PROBE_VK,
                wScan: scan,
                dwFlags: if up {
                    KEYEVENTF_KEYUP
                } else {
                    KEYBD_EVENT_FLAGS(0)
                },
                time: 0,
                dwExtraInfo: PROBE_EXTRA_INFO,
            },
        },
    }
}

/// 把一个物理键盘/鼠标主键事件交给共享匹配器，并投递捕获或业务事件。
/// 返回 true 表示该事件应由钩子吞掉。
fn dispatch_key_event(shared: &HookShared, vk: u32, action: KeyAction) -> bool {
    let outcome = {
        // 锁中毒（理论上的 panic 路径）也不能在 FFI 边界 unwrap：放行输入。
        let Ok(mut matcher) = shared.matcher.lock() else {
            return false;
        };
        // 修饰键以物理状态为准，键盘与鼠标回调共用这条同步路径。
        matcher.sync_modifiers(
            unsafe { GetAsyncKeyState(VK_CONTROL.0 as i32) } < 0,
            unsafe { GetAsyncKeyState(VK_MENU.0 as i32) } < 0,
            unsafe { GetAsyncKeyState(VK_SHIFT.0 as i32) } < 0,
        );
        matcher.on_key(vk, action)
    };

    let capture_tx = shared
        .capture
        .lock()
        .ok()
        .and_then(|slot| slot.as_ref().cloned());
    if let Some(tx) = capture_tx {
        if let Some(spec) = outcome.captured {
            let _ = tx.send(CaptureMsg::Combo(spec));
        } else if action == KeyAction::Down && vk == VK_ESCAPE {
            let _ = tx.send(CaptureMsg::Cancel);
        }
    } else if let Some(ev) = outcome.event {
        let _ = shared.tx.send(ev);
    }
    outcome.swallow
}

/// MSLLHOOKSTRUCT.mouseData 高位字 → core 使用的侧键 VK 码。
fn xbutton_vk(mouse_data: u32) -> Option<u32> {
    match (mouse_data >> 16) as u16 {
        XBUTTON1 => Some(VK_XBUTTON1),
        XBUTTON2 => Some(VK_XBUTTON2),
        _ => None,
    }
}

/// 钩子回调：只做翻译（Win32 事件 → HookMatcher）与吞键判定。
/// 红线：禁止日志/IO/重活；事件经 channel 发给消费者线程。
unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        if let Some(shared) = SHARED.get() {
            let kb = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
            // 闭环探测事件优先处理：确认收到后吞掉，不进入正常热键匹配，也不传给前台。
            // 某些输入过滤软件会清除 LLKHF_INJECTED 或 dwExtraInfo，但仍把事件
            // 交给钩子。probe 槽只开放 250ms，期间按 F24 键码识别比强依赖标记
            // 更兼容；线程 id 约束仍确保回执来自刚安装的新 hook。
            if kb.vkCode == u32::from(PROBE_VK.0) {
                if let Ok(slot) = shared.probe.lock() {
                    if let Some(probe) = slot.as_ref() {
                        if probe.hook_thread_id == unsafe { GetCurrentThreadId() } {
                            let _ = probe.tx.send(());
                            return LRESULT(1);
                        }
                    }
                }
                return unsafe { CallNextHookEx(None::<HHOOK>, code, wparam, lparam) };
            }
            // 跳过合成输入（如我们自己的 SendInput），避免自我触发
            if !kb.flags.contains(LLKHF_INJECTED) {
                let action = match wparam.0 as u32 {
                    WM_KEYDOWN | WM_SYSKEYDOWN => Some(KeyAction::Down),
                    WM_KEYUP | WM_SYSKEYUP => Some(KeyAction::Up),
                    _ => None,
                };
                if let Some(action) = action {
                    if dispatch_key_event(shared, kb.vkCode, action) {
                        return LRESULT(1);
                    }
                }
            }
        }
    }
    unsafe { CallNextHookEx(None::<HHOOK>, code, wparam, lparam) }
}

/// 低层鼠标钩子：只翻译 XBUTTON1/XBUTTON2，其他鼠标操作完全放行。
unsafe extern "system" fn mouse_hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        if let Some(shared) = SHARED.get() {
            let mouse = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
            if mouse.flags & LLMHF_INJECTED == 0 {
                let action = match wparam.0 as u32 {
                    WM_XBUTTONDOWN => Some(KeyAction::Down),
                    WM_XBUTTONUP => Some(KeyAction::Up),
                    _ => None,
                };
                if let (Some(vk), Some(action)) = (xbutton_vk(mouse.mouseData), action) {
                    if dispatch_key_event(shared, vk, action) {
                        return LRESULT(1);
                    }
                }
            }
        }
    }
    unsafe { CallNextHookEx(None::<HHOOK>, code, wparam, lparam) }
}

/// 消费者线程：钩子事件 → 构造时注入的 sink（调用方决定如何调度到业务层）
fn consumer_main(rx: mpsc::Receiver<HookEvent>, sink: Arc<HookSink>) {
    for ev in rx {
        kotone_core::log::log(&format!("llhook captured: {ev:?}"));
        (sink)(ev);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xbutton_mouse_data_maps_to_hotkey_codes() {
        assert_eq!(xbutton_vk(u32::from(XBUTTON1) << 16), Some(VK_XBUTTON1));
        assert_eq!(xbutton_vk(u32::from(XBUTTON2) << 16), Some(VK_XBUTTON2));
        assert_eq!(xbutton_vk(0), None);
    }

    #[test]
    #[ignore = "manual Windows desktop integration smoke test"]
    fn send_input_round_trip_reaches_fresh_ll_hook() {
        let source = LlHookSource::new(Box::new(|_| {}));
        source
            .probe_available(Duration::from_secs(1))
            .map(|outcome| assert_eq!(outcome, ProbeOutcome::Verified))
            .expect("SendInput → WH_KEYBOARD_LL round trip should pass");
    }
}
