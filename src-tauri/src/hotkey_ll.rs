//! Windows 低级键盘钩子（WH_KEYBOARD_LL）热键后端。
//!
//! 背景：RegisterHotKey（tauri-plugin-global-shortcut 底层）在 LOL 等游戏前台时
//! 不投递热键事件（实测日志实证）；LeagueAkari 等游戏工具均用 LL 钩子解决。
//!
//! 线程模型：
//! - 钩子线程：SetWindowsHookExW(WH_KEYBOARD_LL) + GetMessageW 消息循环
//!   （LL 钩子回调跑在安装线程上，该线程必须有消息循环）；
//!   退出时 UnhookWindowsHookEx（PostThreadMessageW(WM_QUIT) 触发）。
//! - 回调红线：只做「过滤 + channel 发送」，绝不写日志/做 IO/调 orchestrator；
//!   事件经 std::sync::mpsc 发给消费者线程，由它 spawn 到 tokio runtime。
//! - 匹配逻辑全部在平台无关的 HookMatcher（hotkey_spec.rs），回调只是翻译层。
//!
//! 吞键策略：完整命中（主键 + 修饰键严格匹配）才 return 1 吞掉；
//! 其余按键一律 CallNextHookEx 放行。

#![cfg(windows)]

use std::sync::mpsc;
use std::sync::{Mutex, OnceLock};

use tauri::{AppHandle, Manager};
use windows::Win32::Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetMessageW, PostThreadMessageW, SetWindowsHookExW, UnhookWindowsHookEx,
    HHOOK, KBDLLHOOKSTRUCT, LLKHF_INJECTED, MSG, WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP, WM_QUIT,
    WM_SYSKEYDOWN, WM_SYSKEYUP,
};

use crate::hotkey::HotkeyMode;
use crate::hotkey_spec::{parse_hotkey, HookEvent, HookMatcher, KeyAction};
use crate::SharedState;

/// 钩子回调共享状态：匹配器 + 事件出口（OnceLock：回调必须是静态 fn）
struct HookShared {
    matcher: Mutex<HookMatcher>,
    tx: mpsc::Sender<HookEvent>,
}

static SHARED: OnceLock<HookShared> = OnceLock::new();

/// LL 钩子后端：首次 register 时启动钩子线程与消费者线程，
/// 之后改键/改模式只更新共享匹配器配置，不重建线程。
pub struct LlHook {
    state: Mutex<LlHookState>,
}

#[derive(Default)]
struct LlHookState {
    started: bool,
    hook_thread_id: Option<u32>,
}

impl Default for LlHook {
    fn default() -> Self {
        Self::new()
    }
}

impl LlHook {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(LlHookState::default()),
        }
    }

    /// 注册/改键：首次调用安装钩子并启动事件泵；失败返回 Err（调用方回退 RegisterHotKey）
    pub fn register(&self, app: &AppHandle, key: &str, mode: HotkeyMode) -> Result<(), String> {
        let spec = parse_hotkey(key)
            .ok_or_else(|| format!("无法解析热键「{key}」（LL 钩子后端不支持该键名）"))?;

        let mut state = self.state.lock().unwrap();
        if state.started {
            // 钩子已在跑：只更新匹配配置
            if let Some(shared) = SHARED.get() {
                shared.matcher.lock().unwrap().set_config(spec, mode);
            }
            crate::log::log(&format!("llhook reconfigured: {key} ({mode:?})"));
            return Ok(());
        }

        // 首次启动：事件通道 + 钩子线程 + 消费者线程
        let (tx, rx) = mpsc::channel::<HookEvent>();
        let shared = HookShared {
            matcher: Mutex::new(HookMatcher::new(spec, mode)),
            tx,
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

        let orch = app.state::<SharedState>().orchestrator.clone();
        std::thread::Builder::new()
            .name("kotone-llhook-events".into())
            .spawn(move || consumer_main(rx, orch))
            .map_err(|e| format!("启动钩子事件线程失败: {e}"))?;

        state.started = true;
        state.hook_thread_id = Some(thread_id);
        crate::log::log(&format!("llhook backend started: {key} ({mode:?})"));
        Ok(())
    }

    /// 注销：匹配器置为禁用（全部按键放行），钩子线程保留以便快速重注册
    pub fn unregister(&self) {
        if let Some(shared) = SHARED.get() {
            shared.matcher.lock().unwrap().set_enabled(false);
            crate::log::log("llhook unregistered (matcher disabled)");
        }
    }

    /// 会话激活标志：Esc 取消使能（由 HotkeyManager.set_cancel_enabled 转发）
    pub fn set_session_active(&self, active: bool) {
        if let Some(shared) = SHARED.get() {
            shared.matcher.lock().unwrap().set_session_active(active);
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
}

impl Drop for LlHook {
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
        crate::log::log("WH_KEYBOARD_LL hook installed");

        let mut msg = MSG::default();
        // GetMessageW 返回 false = 收到 WM_QUIT
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {}

        let _ = UnhookWindowsHookEx(hook);
        crate::log::log("WH_KEYBOARD_LL hook uninstalled");
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
                        if let Some(ev) = outcome.event {
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

/// 消费者线程：钩子事件 → orchestrator（spawn 进 tokio runtime）
fn consumer_main(rx: mpsc::Receiver<HookEvent>, orch: std::sync::Arc<crate::orchestrator::Orchestrator>) {
    for ev in rx {
        crate::log::log(&format!("llhook captured: {ev:?}"));
        let orch = orch.clone();
        match ev {
            HookEvent::HoldPressed => {
                tauri::async_runtime::spawn(async move {
                    orch.on_hotkey_hold(true).await;
                });
            }
            HookEvent::HoldReleased => {
                tauri::async_runtime::spawn(async move {
                    orch.on_hotkey_hold(false).await;
                });
            }
            HookEvent::Toggle => {
                tauri::async_runtime::spawn(async move {
                    orch.on_hotkey_toggle().await;
                });
            }
            HookEvent::Cancel => {
                tauri::async_runtime::spawn(async move {
                    orch.cancel().await;
                });
            }
        }
    }
}
