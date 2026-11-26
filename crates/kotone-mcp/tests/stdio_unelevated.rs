//! 以普通权限运行服务器（ADR 044 D2）：Kotone 是管理员身份时，stdio 服务器以桌面为父进程创建。这里用测试开关
//! （`KOTONE_SPAWN_UNELEVATED`，只在这个测试程序里设）强制走这条路：服务器能正常握手、调工具，父进程是桌面、
//! 完整性级别和桌面一样（平常的电脑上是普通 Medium；GitHub 的 Windows 机器上用户是管理员、关了 UAC，桌面本身就是
//! High），关掉以后它和它启动的进程都被结束（作业对象）。没有桌面（CI 的服务会话）就跳过。
//! 整套 `tests/stdio.rs` 也可以设上这个环境变量跑一遍：`KOTONE_SPAWN_UNELEVATED=1 cargo test -p kotone-mcp --test stdio`。
#![cfg(windows)]

use std::path::PathBuf;
use std::time::{Duration, Instant};

use kotone_mcp::{McpClient, ServerConfig};
use kotone_platform_windows::elevation::{process_integrity, INTEGRITY_MEDIUM};
use kotone_platform_windows::unelevated;
use serde_json::json;

/// `target/debug/examples/stdio_server.exe`（同 `tests/stdio.rs`：从测试程序往上一级级找）。
fn server_exe() -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    for dir in exe.ancestors().skip(1) {
        let path = dir.join("examples").join("stdio_server.exe");
        if path.is_file() {
            return path;
        }
    }
    panic!(
        "找不到 stdio_server.exe：先 `cargo test -p kotone-mcp`（会编译 examples/stdio_server.rs）"
    );
}

fn parent_of(pid: u32) -> Option<u32> {
    kotone_platform_windows::inject::process_list()
        .into_iter()
        .find(|(p, _, _)| *p == pid)
        .map(|(_, parent, _)| parent)
}

fn alive(pid: u32) -> bool {
    parent_of(pid).is_some()
}

async fn gone_within(pid: u32, limit: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < limit {
        if !alive(pid) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    !alive(pid)
}

async fn pid_of(client: &McpClient, tool: &str) -> u32 {
    client
        .call_tool(tool, json!({}))
        .await
        .unwrap()
        .text()
        .trim()
        .parse()
        .unwrap()
}

#[tokio::test]
async fn servers_run_unelevated_under_the_desktop() {
    let desktop = match unelevated::desktop_pid() {
        Ok(pid) => pid,
        Err(e) => {
            eprintln!("[stdio_unelevated] 跳过：{e}");
            return;
        }
    };
    // 这个测试程序只有这一个测试，设环境变量不影响别的测试
    std::env::set_var(unelevated::FORCE_ENV, "1");
    let config = ServerConfig::stdio(server_exe().to_string_lossy(), std::iter::empty::<&str>())
        .startup_timeout(Duration::from_secs(20))
        .client("kotone", "0.2.0-test");
    let client = McpClient::connect(&config).await.unwrap();
    assert_eq!(client.server_info().name, "kotone-stdio-test");
    let server = pid_of(&client, "pid").await;
    let child = pid_of(&client, "spawn_child").await;
    assert_eq!(client.process_id(), Some(server));
    assert_eq!(parent_of(server), Some(desktop), "服务器是桌面起的");
    // 拿的是桌面的令牌：和桌面一样；桌面是普通权限时就是普通
    let desktop_level = process_integrity(desktop).expect("读得到桌面的完整性级别");
    assert_eq!(process_integrity(server), Some(desktop_level), "服务器");
    assert_eq!(
        process_integrity(child),
        Some(desktop_level),
        "服务器启动的进程"
    );
    if desktop_level == INTEGRITY_MEDIUM {
        assert_eq!(process_integrity(server), Some(INTEGRITY_MEDIUM));
    } else {
        eprintln!("[stdio_unelevated] 桌面本身是 {desktop_level:#x}（关了 UAC 的管理员），只比对和桌面一样");
    }
    client.close().await;
    let server_gone = gone_within(server, Duration::from_secs(5)).await;
    let child_gone = gone_within(child, Duration::from_secs(5)).await;
    for pid in [child, server] {
        let _ = std::process::Command::new("taskkill")
            .args(["/F", "/PID", &pid.to_string()])
            .output();
    }
    assert!(server_gone, "服务器还在");
    assert!(child_gone, "服务器启动的进程还在");
}
