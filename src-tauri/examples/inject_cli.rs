//! 注入 CLI（记事本集成测试 / 手工调试用，不随应用打包）
//!
//! 用法：
//!   inject_cli <text> [--clipboard] [--profile <id>] [--delay-ms <n>]
//!
//! - 默认使用内置 generic profile（Enter → Unicode 逐字 → Enter，通配任意前台窗口）
//! - `--clipboard`：改走剪贴板 + Ctrl+V 备选路径
//! - `--profile <id>`：从 ~/.kotone/profiles 加载指定 profile
//! - `--delay-ms <n>`：发送前等待 n 毫秒（手工切换到目标窗口用）
//!
//! 退出码：0 = 发送成功（输出 INJECT_OK）；1 = 失败（输出 INJECT_ERR: ...）

use kotone_lib::inject::{CancelToken, Injector, WindowsInjector};
use kotone_lib::profile::{self, GameProfile};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(text) = args.first().cloned() else {
        eprintln!("usage: inject_cli <text> [--clipboard] [--profile <id>] [--delay-ms <n>]");
        std::process::exit(2);
    };

    let mut profile = GameProfile::builtin_generic();
    let mut delay_ms: u64 = 0;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--clipboard" => profile.prefer_clipboard_paste = true,
            "--profile" => {
                i += 1;
                let id = args.get(i).map(String::as_str).unwrap_or("");
                match profile::get(id) {
                    Some(p) => profile = p,
                    None => {
                        eprintln!("INJECT_ERR: profile 「{id}」不存在");
                        std::process::exit(2);
                    }
                }
            }
            "--delay-ms" => {
                i += 1;
                delay_ms = args.get(i).and_then(|s| s.parse().ok()).unwrap_or(0);
            }
            other => {
                eprintln!("INJECT_ERR: 未知参数 {other}");
                std::process::exit(2);
            }
        }
        i += 1;
    }

    if delay_ms > 0 {
        std::thread::sleep(std::time::Duration::from_millis(delay_ms));
    }

    let injector = WindowsInjector;
    match injector.send(&text, &profile, CancelToken::default()) {
        Ok(()) => println!("INJECT_OK"),
        Err(e) => {
            println!("INJECT_ERR: {}", e.message);
            std::process::exit(1);
        }
    }
}
