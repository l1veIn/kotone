//! 热键的“只在游戏里”（ADR 054）：每 250 毫秒看一眼最前面的窗口，是认得的游戏就把它的游戏配置 id 写进
//! `kotone_core::hotkey_options` 的原子量，钩子回调只读那个原子量（回调里不做 IO、不拿别的锁）。
//!
//! 窗口没换（进程号一样）不查进程名；换了才查一次、照游戏配置的进程名对（和 `games.rs` 的前台判断同一个规则）。
//! 游戏列表 2 秒的那一轮太慢：从桌面切进游戏后侧键要马上能用，切出来要马上照常是“后退”。

use std::time::Duration;

use kotone_core::hotkey_options::set_foreground_game;
use kotone_core::profile;

/// 多久看一次最前面的窗口。
const POLL: Duration = Duration::from_millis(250);
/// 游戏配置（进程名）多久重读一次（磁盘）。
const PROFILES_EVERY: u32 = 40;

/// 启动时开一个线程一直看着。
pub fn start() {
    let _ = std::thread::Builder::new()
        .name("kotone-hotkey-gate".into())
        .spawn(run);
}

fn run() {
    let mut last_pid: Option<u32> = None;
    let mut games = game_profiles();
    let mut ticks: u32 = 0;
    loop {
        std::thread::sleep(POLL);
        ticks = ticks.wrapping_add(1);
        if ticks % PROFILES_EVERY == 0 {
            games = game_profiles();
            last_pid = None; // 游戏配置可能改了：下一轮重新对一次
        }
        let pid = kotone_platform_windows::inject::foreground_pid();
        if pid == last_pid {
            continue;
        }
        last_pid = pid;
        let game = pid
            .and_then(kotone_platform_windows::inject::process_name_from_pid)
            .and_then(|name| {
                games
                    .iter()
                    .find(|g| profile::matches_process(g, &name))
                    .map(|g| g.id.clone())
            });
        set_foreground_game(game.as_deref());
    }
}

/// 有进程名的游戏配置（认得出窗口的）。
fn game_profiles() -> Vec<profile::GameProfile> {
    profile::list()
        .into_iter()
        .filter(|g| !g.process_names.is_empty())
        .collect()
}
