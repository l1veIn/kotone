//! 假的 `BetterGI.exe`（真机脚本 `scripts/manual-e2e/genshin_bgi_real.py` 用）：把收到的命令行参数记进旁边的
//! `fake-bettergi-args.jsonl`（一行一次），然后退出。不打开任何窗口、不碰原神、不碰玩家的 BetterGI。
//!
//! `cargo build -p kotone-plugin-genshin --example fake_bettergi`，脚本把它复制到 `target/genshin-e2e/BetterGI/BetterGI.exe`。

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let exe = std::env::current_exe().expect("exe path");
    let log = exe.with_file_name("fake-bettergi-args.jsonl");
    let line = format!(
        "{}\n",
        serde_json::json!({"args": args, "pid": std::process::id()})
    );
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log)
        .expect("args log");
    file.write_all(line.as_bytes()).expect("write args");
}
