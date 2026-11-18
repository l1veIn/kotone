// 桌面入口始终使用 GUI 子系统，debug 安装包也不额外打开控制台；诊断日志仍写入文件。
#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

fn main() {
    // `kotone.exe <子命令>`（如 `mcp`、`tools call`、`app status`）就是命令行，与 kotone-cli 相同。
    // 必须在 Tauri 之前分流：单实例插件会把第二个进程的参数转给已运行的实例并让它退出。
    // GUI 子系统没有控制台：经管道调用（Agent、插件、MCP 客户端）时输入输出与退出码照常；
    // 在终端里手动使用请用 kotone-cli。
    if std::env::args_os()
        .nth(1)
        .is_some_and(|arg| arg.to_str().is_some_and(kotone_cli::is_subcommand))
    {
        std::process::exit(kotone_cli::main_with_args(std::env::args_os()));
    }
    kotone_lib::run();
}
