//! `kotone-cli.exe`：控制台入口（开发与自动化）。命令实现在 lib，桌面端 `kotone.exe` 共用同一套。

fn main() {
    std::process::exit(kotone_cli::main_with_args(std::env::args_os()));
}
