//! Windows 低级键盘钩子（WH_KEYBOARD_LL）热键源：core `HotkeySource` 端口的实现。
//!
//! 背景：RegisterHotKey 在 LOL 等游戏前台不投递热键事件（实测日志实证）；
//! LeagueAkari 等游戏工具均用 LL 钩子解决。
//!
//! 线程模型：
//! - 钩子线程：SetWindowsHookExW(WH_KEYBOARD_LL) + GetMessageW 消息循环
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
use std::time::Duration;

use windows::Win32::Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetMessageW, PostThreadMessageW, SetWindowsHookExW, UnhookWindowsHookEx,
    HHOOK, KBDLLHOOKSTRUCT, LLKHF_INJECTED, MSG, WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP, WM_QUIT,
    WM_SYSKEYDOWN, WM_SYSKEYUP,
};

use kotone_core::hotkey::{
    parse_hotkey, HookEvent, HookMatcher, HotkeyMode, HotkeySource, HotkeySpec, KeyAction,
    VK_ESCAPE,
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

/// 钩子回调共享状态：匹配器 + 事件出口（OnceLock：回调必须是静态 fn）
struct HookShared {
    matcher: Mutex<HookMatcher>,
    tx: mpsc::Sender<HookEvent>,
    /// 捕获模式出口：Some 期间按键组合/Esc 走此通道而非正常事件流
    capture: Mutex<Option<mpsc::Sender<CaptureMsg>>>,
}

static SHARED: OnceLock<HookShared> = OnceLock::new();

/// LL 钩子热键源：首次 register 时启动钩子线程与消费者线程，
/// 之后改键/改模式只更新共享匹配器配置，不重建线程。
pub struct LlHookSource {
    sink: Arc<HookSink>,
    state: Mutex<LlHookState>,
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

        let (was_started, was_armed) = {
            let state = self.state.lock().unwrap();
            (state.started, state.armed)
        };

        if !was_started {
            // 未注册过：用占位热键启动钩子线程（capture 模式优先于 enabled/匹配，
            // 不产生 HookEvent；捕获到的主键会被吞掉，结束后恢复禁用）
            self.register("F24", HotkeyMode::Toggle)?;
            self.state.lock().unwrap().armed = false;
            if let Some(shared) = SHARED.get() {
                shared.matcher.lock().unwrap().set_enabled(false);
            }
        }

        let shared = SHARED.get().ok_or("LL 钩子未启动".to_string())?;
        let (tx, rx) = mpsc::channel::<CaptureMsg>();
        *shared.capture.lock().unwrap() = Some(tx);
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
            return Ok(());
        }

        // 首次启动：事件通道 + 钩子线程 + 消费者线程
        let (tx, rx) = mpsc::channel::<HookEvent>();
        let shared = HookShared {
            matcher: Mutex::new(HookMatcher::new(spec, mode)),
            tx,
            capture: Mutex::new(None),
        };
        SHARED
            .set(shared)
            .map_err(|_| "LL 钩子共享状态已初始化（内部错误）".to_string())?;

        let (boot_tx, boot_rx) = mpsc::channel::<Result<u32, String>>();
        std::thread::Builder::new()
            .name("kotone-llhook".into())
            .spawn(move || hook_thread_main(boot_tx))
            .map_err(|e| format!("启动钩子线程失败: {e}"))?;

        let thread_id = match boot_rx.recv() {
            Ok(Ok(tid)) => tid,
            Ok(Err(e)) => return Err(format!("安装 WH_KEYBOARD_LL 钩子失败: {e}")),
            Err(_) => return Err("钩子线程启动后异常退出".into()),
        };

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
}

impl Drop for LlHookSource {
    fn drop(&mut self) {
        // 进程退出路径：通知钩子线程退出并卸钩（系统也会在进程结束时兜底清理）
        self.shutdown();
    }
}

/// 钩子线程主体：安装钩子 → 消息循环 → 退出时卸钩
fn hook_thread_main(boot: mpsc::Sender<Result<u32, String>>) {
    unsafe {
        let tid = GetCurrentThreadId();
        let hook = match SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), None::<HINSTANCE>, 0) {
            Ok(h) => h,
            Err(e) => {
                let _ = boot.send(Err(format!("SetWindowsHookExW: {e}")));
                return;
            }
        };
        let _ = boot.send(Ok(tid));
        kotone_core::log::log("WH_KEYBOARD_LL hook installed");

        let mut msg = MSG::default();
        // GetMessageW 返回 false = 收到 WM_QUIT
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {}

        let _ = UnhookWindowsHookEx(hook);
        kotone_core::log::log("WH_KEYBOARD_LL hook uninstalled");
    }
}

/// 钩子回调：只做翻译（Win32 事件 → HookMatcher）与吞键判定。
/// 红线：禁止日志/IO/重活；事件经 channel 发给消费者线程。
unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        if let Some(shared) = SHARED.get() {
            let kb = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
            // 跳过合成输入（如我们自己的 SendInput），避免自我触发
            if !kb.flags.contains(LLKHF_INJECTED) {
                let action = match wparam.0 as u32 {
                    WM_KEYDOWN | WM_SYSKEYDOWN => Some(KeyAction::Down),
                    WM_KEYUP | WM_SYSKEYUP => Some(KeyAction::Up),
                    _ => None,
                };
                if let Some(action) = action {
                    // 锁中毒（理论上的 panic 路径）也不能在 FFI 边界 unwrap：放行按键
                    if let Ok(mut matcher) = shared.matcher.lock() {
                        let outcome = matcher.on_key(kb.vkCode, action);
                        drop(matcher);
                        // 捕获模式（热键录入）：组合键 / Esc 取消走 capture 通道
                        let capture_tx = shared.capture.lock().unwrap().clone();
                        if let Some(tx) = capture_tx {
                            if let Some(spec) = outcome.captured {
                                let _ = tx.send(CaptureMsg::Combo(spec));
                            } else if action == KeyAction::Down && kb.vkCode == VK_ESCAPE {
                                let _ = tx.send(CaptureMsg::Cancel);
                            }
                        } else if let Some(ev) = outcome.event {
                            let _ = shared.tx.send(ev);
                        }
                        if outcome.swallow {
                            return LRESULT(1);
                        }
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
