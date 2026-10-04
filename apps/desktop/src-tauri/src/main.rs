// 桌面入口始终使用 GUI 子系统，debug 安装包也不额外打开控制台；诊断日志仍写入文件。
#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

fn main() {
    kotone_lib::run();
}
