//! 一把输入锁（ADR 041 D4）：往游戏里打字的所有发话人（语音输入、她、外部 Agent、自动化的 `chat()`）、
//! `game.act`、按键程序共用，后来的按先来后到排队，不交错。
//!
//! - **实例只有一份，由宿主（桌面端）建**，交给入口（[`crate::send_to_game::GameChat`]）、`game.act`、按键程序；
//!   不做成 `static`：插件链接了本 crate 也拿不到宿主那一份，只能经宿主给的入口按键（ADR 038、原则 13）。
//! - 粒度由拿锁的一方定：一条发言（全部行）/ 一次 `game.act`（整批）/ 按键程序的每一下。松开键不拿锁。
//! - 等到截止时间还没轮到就返回 [`LockError::Busy`]（说谁占着），不丢不重发（ADR 032）。
//! - 不可重入：同一线程已经拿着时再拿，直接返回 `Busy`，不会把自己锁死。
//! - 内部的 `Mutex` 只在改队列时短暂持有；拿着输入锁去按键是这把锁的本意（它就是“谁的手在键盘上”）。

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::ThreadId;
use std::time::{Duration, Instant};

use crate::send_to_game::Speaker;

/// 正常情况下排队最多等多久（ADR 041 第 1 期设计 3.12）：正常最长的占用者是另一条 10 行的发言
/// （约 3.1 秒）加它等修饰键（最多 1 秒）；比这还久，这句话也过时了。
pub const INPUT_LOCK_WAIT: Duration = Duration::from_secs(5);

/// 排队时多久看一次取消。
const CANCEL_POLL: Duration = Duration::from_millis(50);

/// 谁在占着键盘鼠标（排不上时告诉玩家 / 她是谁占着）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputUser {
    /// 往游戏聊天里发一句（经入口）。
    Chat(Speaker),
    /// 她在操作游戏（`game.act` 一整批）。
    GameAct,
    /// 自动化程序的一下按键 / 打字 / 点击。
    Program,
}

impl InputUser {
    /// 给玩家 / 她看的说法。
    pub fn describe(self) -> &'static str {
        match self {
            Self::Chat(Speaker::Player) => "正在发你用语音输入说的话",
            Self::Chat(Speaker::Companion) => "她正在往游戏聊天里发字",
            Self::Chat(Speaker::Agent) => "外部 Agent 正在往游戏聊天里发字",
            Self::Chat(Speaker::Automation) => "自动化正在往游戏聊天里发字",
            Self::GameAct => "她正在操作游戏",
            Self::Program => "自动化程序正在按键",
        }
    }
}

/// 等到截止时间还没轮到。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputBusy {
    /// 截止时占着锁的一方（极少数情况下刚好放开，为 None）。
    pub holder: Option<InputUser>,
    pub waited_ms: u64,
}

impl std::fmt::Display for InputBusy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let who = self.holder.map_or("键盘正被占着", InputUser::describe);
        let seconds = self.waited_ms.div_ceil(1000).max(1);
        write!(
            f,
            "{who}，等了 {seconds} 秒还没轮到，这次没发；等它做完再来"
        )
    }
}

/// 拿锁失败。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockError {
    Busy(InputBusy),
    Cancelled,
}

#[derive(Default)]
struct State {
    /// 当前持有者：(票号, 谁, 哪个线程)
    holder: Option<(u64, InputUser, ThreadId)>,
    /// 排队的票号，先来后到
    queue: VecDeque<u64>,
    next: u64,
}

pub struct InputLock {
    state: Mutex<State>,
    changed: Condvar,
}

/// 拿着输入锁。drop 时放开并叫醒排在后面的（可以在别的线程 drop）。
pub struct InputGuard {
    lock: Arc<InputLock>,
    ticket: u64,
}

impl std::fmt::Debug for InputGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InputGuard")
            .field("ticket", &self.ticket)
            .finish()
    }
}

impl Drop for InputGuard {
    fn drop(&mut self) {
        let mut state = self.lock.state.lock().unwrap();
        if state.holder.is_some_and(|(t, _, _)| t == self.ticket) {
            state.holder = None;
        }
        drop(state);
        self.lock.changed.notify_all();
    }
}

impl InputLock {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(State::default()),
            changed: Condvar::new(),
        })
    }

    /// 现在谁占着。
    pub fn holder(&self) -> Option<InputUser> {
        self.state.lock().unwrap().holder.map(|(_, who, _)| who)
    }

    /// 阻塞等到轮到自己。`deadline` 到了还没轮到 → `Busy`（自己从队列里退出，不影响后面的人）；
    /// 每 50 毫秒看一次 `cancelled()`，取消了返回 `Cancelled`。
    pub fn acquire(
        self: &Arc<Self>,
        who: InputUser,
        deadline: Instant,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<InputGuard, LockError> {
        let started = Instant::now();
        let me = std::thread::current().id();
        let mut state = self.state.lock().unwrap();
        if let Some((_, holder, thread)) = state.holder {
            if thread == me {
                // 不可重入：同一线程再拿会把自己锁死，直接说占着
                return Err(LockError::Busy(InputBusy {
                    holder: Some(holder),
                    waited_ms: 0,
                }));
            }
        }
        let ticket = state.next;
        state.next += 1;
        state.queue.push_back(ticket);
        loop {
            if state.holder.is_none() && state.queue.front() == Some(&ticket) {
                state.queue.pop_front();
                state.holder = Some((ticket, who, me));
                return Ok(InputGuard {
                    lock: self.clone(),
                    ticket,
                });
            }
            let now = Instant::now();
            let leave = if cancelled() {
                Some(LockError::Cancelled)
            } else if now >= deadline {
                Some(LockError::Busy(InputBusy {
                    holder: state.holder.map(|(_, who, _)| who),
                    waited_ms: started.elapsed().as_millis() as u64,
                }))
            } else {
                None
            };
            if let Some(error) = leave {
                state.queue.retain(|t| *t != ticket);
                drop(state);
                // 排在我后面的可能正等着我让开
                self.changed.notify_all();
                return Err(error);
            }
            let wait = deadline.saturating_duration_since(now).min(CANCEL_POLL);
            state = self.changed.wait_timeout(state, wait).unwrap().0;
        }
    }

    /// 最多等 [`INPUT_LOCK_WAIT`]，不可取消（`game.act`、按键程序用）。
    pub fn acquire_for(self: &Arc<Self>, who: InputUser) -> Result<InputGuard, InputBusy> {
        match self.acquire(who, Instant::now() + INPUT_LOCK_WAIT, &|| false) {
            Ok(guard) => Ok(guard),
            Err(LockError::Busy(busy)) => Err(busy),
            Err(LockError::Cancelled) => unreachable!("不可取消"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    fn soon(ms: u64) -> Instant {
        Instant::now() + Duration::from_millis(ms)
    }

    #[test]
    fn second_waits_for_the_first_and_order_is_first_come_first_served() {
        let lock = InputLock::new();
        let first = lock
            .acquire(InputUser::GameAct, soon(100), &|| false)
            .unwrap();
        let order = Arc::new(Mutex::new(Vec::new()));
        let mut workers = Vec::new();
        for i in 0..3 {
            let lock = lock.clone();
            let order = order.clone();
            workers.push(std::thread::spawn(move || {
                let _g = lock
                    .acquire(InputUser::Program, soon(2_000), &|| false)
                    .unwrap();
                order.lock().unwrap().push(i);
                std::thread::sleep(Duration::from_millis(10));
            }));
            // 让它们按 0、1、2 的顺序排进队
            std::thread::sleep(Duration::from_millis(30));
        }
        assert_eq!(lock.holder(), Some(InputUser::GameAct));
        assert!(order.lock().unwrap().is_empty(), "第一个没放，谁也拿不到");
        drop(first);
        for w in workers {
            w.join().unwrap();
        }
        assert_eq!(*order.lock().unwrap(), vec![0, 1, 2]);
        assert_eq!(lock.holder(), None);
    }

    #[test]
    fn waiting_past_the_deadline_is_busy_and_names_the_holder() {
        let lock = InputLock::new();
        let _held = lock
            .acquire(InputUser::Chat(Speaker::Companion), soon(100), &|| false)
            .unwrap();
        let other = lock.clone();
        let busy = std::thread::spawn(move || {
            other
                .acquire(InputUser::Chat(Speaker::Player), soon(80), &|| false)
                .unwrap_err()
        })
        .join()
        .unwrap();
        let LockError::Busy(busy) = busy else {
            panic!("{busy:?}")
        };
        assert_eq!(busy.holder, Some(InputUser::Chat(Speaker::Companion)));
        assert!(busy.waited_ms >= 70, "{busy:?}");
        assert!(
            busy.to_string().contains("她正在往游戏聊天里发字"),
            "{busy}"
        );
    }

    #[test]
    fn a_waiter_that_gives_up_does_not_block_the_ones_behind_it() {
        let lock = InputLock::new();
        let held = lock
            .acquire(InputUser::GameAct, soon(100), &|| false)
            .unwrap();
        let impatient = {
            let lock = lock.clone();
            std::thread::spawn(move || lock.acquire(InputUser::Program, soon(50), &|| false))
        };
        std::thread::sleep(Duration::from_millis(20));
        let patient = {
            let lock = lock.clone();
            std::thread::spawn(move || {
                lock.acquire(InputUser::Program, soon(2_000), &|| false)
                    .is_ok()
            })
        };
        assert!(impatient.join().unwrap().is_err());
        drop(held);
        assert!(patient.join().unwrap(), "排在放弃者后面的照样轮得到");
    }

    #[test]
    fn cancelling_while_waiting_returns_cancelled() {
        let lock = InputLock::new();
        let _held = lock
            .acquire(InputUser::GameAct, soon(100), &|| false)
            .unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let waiter = {
            let lock = lock.clone();
            let cancel = cancel.clone();
            std::thread::spawn(move || {
                lock.acquire(InputUser::Program, soon(5_000), &|| {
                    cancel.load(Ordering::SeqCst)
                })
            })
        };
        std::thread::sleep(Duration::from_millis(30));
        cancel.store(true, Ordering::SeqCst);
        let started = Instant::now();
        assert_eq!(waiter.join().unwrap().unwrap_err(), LockError::Cancelled);
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn a_guard_dropped_on_another_thread_releases() {
        let lock = InputLock::new();
        let guard = lock
            .acquire(InputUser::Chat(Speaker::Agent), soon(100), &|| false)
            .unwrap();
        std::thread::spawn(move || drop(guard)).join().unwrap();
        assert_eq!(lock.holder(), None);
        assert!(lock.acquire_for(InputUser::Program).is_ok());
    }

    #[test]
    fn taking_it_twice_on_the_same_thread_is_busy_not_a_deadlock() {
        let lock = InputLock::new();
        let _held = lock.acquire_for(InputUser::GameAct).unwrap();
        let started = Instant::now();
        let busy = lock.acquire_for(InputUser::Program).unwrap_err();
        assert_eq!(busy.holder, Some(InputUser::GameAct));
        assert!(started.elapsed() < Duration::from_millis(50));
    }
}
