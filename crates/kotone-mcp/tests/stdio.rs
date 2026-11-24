//! 子进程（stdio）服务器：用 `examples/stdio_server.rs`（rmcp 自己的服务端）测列工具、调用、读资源、超时、
//! 环境变量、`.cmd` 包装和参数转义、`tools/list_changed`，以及关掉 / 丢掉连接时服务器和它启动的进程都被结束（Job Object）。
#![cfg(windows)]

use std::path::PathBuf;
use std::time::{Duration, Instant};

use kotone_mcp::{McpClient, McpError, ServerConfig, Stage};
use serde_json::json;

/// `target/debug/examples/stdio_server.exe`（`cargo test -p kotone-mcp` 会先编译 examples）。
fn server_exe() -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    let profile = exe.parent().unwrap().parent().unwrap();
    let path = profile.join("examples").join("stdio_server.exe");
    assert!(
        path.is_file(),
        "{} 不存在：先 `cargo test -p kotone-mcp`（会编译 examples/stdio_server.rs）",
        path.display()
    );
    path
}

fn config(args: &[&str]) -> ServerConfig {
    ServerConfig::stdio(server_exe().to_string_lossy(), args.iter().copied())
        .startup_timeout(Duration::from_secs(20))
        .client("kotone", "0.2.0-test")
}

fn alive(pid: u32) -> bool {
    let out = std::process::Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).contains(&format!("\"{pid}\""))
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

fn kill(pid: u32) {
    let _ = std::process::Command::new("taskkill")
        .args(["/F", "/PID", &pid.to_string()])
        .output();
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
async fn lists_calls_and_reads() {
    let client = McpClient::connect(&config(&[])).await.unwrap();
    assert_eq!(client.server_info().name, "kotone-stdio-test");
    assert_eq!(client.server_info().protocol_version, "2025-11-25");
    assert!(client.is_connected());

    let tools = client.list_tools().await.unwrap();
    assert_eq!(tools.len(), 8, "{tools:?}");
    let echo = tools.iter().find(|t| t.name == "echo").unwrap();
    assert_eq!(echo.annotations.as_ref().unwrap()["readOnlyHint"], true);
    assert_eq!(echo.input_schema["type"], "object");

    let result = client
        .call_tool("echo", json!({"text": "亚索"}))
        .await
        .unwrap();
    assert_eq!(result.text(), "亚索");
    assert!(!result.is_error);
    assert_eq!(result.structured_content.unwrap()["echo"], "亚索");

    let failed = client.call_tool("fail", json!(null)).await.unwrap();
    assert!(failed.is_error);
    assert_eq!(failed.text(), "no such champion");

    match client.call_tool("nope", json!({})).await {
        Err(McpError::Server { message, .. }) => assert!(message.contains("nope"), "{message}"),
        other => panic!("{other:?}"),
    }
    assert!(matches!(
        client.call_tool("echo", json!([1])).await,
        Err(McpError::Config(_))
    ));

    let resources = client.list_resources().await.unwrap();
    assert_eq!(resources.len(), 1);
    assert_eq!(resources[0].uri, "ui://kotone-test/page");
    assert_eq!(
        resources[0].mime_type.as_deref(),
        Some("text/html;profile=mcp-app")
    );
    let contents = client.read_resource("ui://kotone-test/page").await.unwrap();
    assert_eq!(contents.len(), 1);
    assert!(contents[0].text.as_deref().unwrap().contains("<p>hi</p>"));
    assert!(contents[0].meta.as_ref().unwrap()["ui"]["csp"].is_object());
    assert!(client.read_resource("ui://kotone-test/none").await.is_err());
    client.close().await;
}

#[tokio::test]
async fn environment_is_baseline_plus_configured() {
    let client = McpClient::connect(&config(&[]).env("KOTONE_TEST_VALUE", "42"))
        .await
        .unwrap();
    let env = |name: &'static str| {
        let client = &client;
        async move {
            client
                .call_tool("env", json!({"name": name}))
                .await
                .unwrap()
                .text()
        }
    };
    assert_eq!(env("KOTONE_TEST_VALUE").await, "42");
    // cargo 给测试进程设的变量不在基线里，传不下去
    assert!(std::env::var("CARGO_MANIFEST_DIR").is_ok());
    assert_eq!(env("CARGO_MANIFEST_DIR").await, "<unset>");
    assert_ne!(env("SYSTEMROOT").await, "<unset>");
    assert_ne!(env("PATH").await, "<unset>");
    client.close().await;
}

#[tokio::test]
async fn a_slow_request_times_out_and_the_connection_stays() {
    let client = McpClient::connect(&config(&[]).tool_timeout(Duration::from_millis(300)))
        .await
        .unwrap();
    let started = Instant::now();
    match client.call_tool("sleep", json!({"ms": 5000})).await {
        Err(McpError::Timeout { stage, after }) => {
            assert_eq!(stage, Stage::Request);
            assert_eq!(after, Duration::from_millis(300));
        }
        other => panic!("{other:?}"),
    }
    assert!(started.elapsed() < Duration::from_secs(3));
    assert_eq!(
        client
            .call_tool("echo", json!({"text": "还在"}))
            .await
            .unwrap()
            .text(),
        "还在"
    );
    client.close().await;
}

#[tokio::test]
async fn a_server_that_never_answers_times_out_at_startup() {
    let started = Instant::now();
    let error =
        McpClient::connect(&config(&["--hang"]).startup_timeout(Duration::from_millis(800)))
            .await
            .unwrap_err();
    assert!(
        matches!(
            error,
            McpError::Timeout {
                stage: Stage::Startup,
                ..
            }
        ),
        "{error:?}"
    );
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[tokio::test]
async fn a_crash_at_startup_says_why() {
    let error = McpClient::connect(&config(&["--crash"])).await.unwrap_err();
    let text = error.to_string();
    assert!(matches!(error, McpError::Spawn(_)), "{error:?}");
    assert!(text.contains("退出码 3"), "{text}");
    assert!(text.contains("KOTONE_TEST_TOKEN is missing"), "{text}");
}

#[tokio::test]
async fn a_missing_program_is_named() {
    let error = McpClient::connect(&ServerConfig::stdio("kotone-no-such-mcp-server", ["x"]))
        .await
        .unwrap_err();
    assert!(matches!(error, McpError::Spawn(_)), "{error:?}");
    assert!(
        error.to_string().contains("kotone-no-such-mcp-server"),
        "{error}"
    );
}

#[tokio::test]
async fn close_ends_the_server_and_what_it_started() {
    let client = McpClient::connect(&config(&[])).await.unwrap();
    let server = pid_of(&client, "pid").await;
    let child = pid_of(&client, "spawn_child").await;
    assert_eq!(client.process_id(), Some(server));
    assert!(alive(server) && alive(child));
    client.close().await;
    let server_gone = gone_within(server, Duration::from_secs(5)).await;
    let child_gone = gone_within(child, Duration::from_secs(5)).await;
    kill(child);
    kill(server);
    assert!(server_gone, "服务器还在");
    assert!(child_gone, "服务器启动的进程还在");
    assert!(!client.is_connected());
    assert_eq!(
        client.call_tool("echo", json!({})).await.unwrap_err(),
        McpError::Closed
    );
}

/// Kotone 退出 / 崩溃时不会调 close：Job 的句柄关掉，里面的进程一起结束。
#[tokio::test]
async fn dropping_the_client_ends_everything() {
    let client = McpClient::connect(&config(&[])).await.unwrap();
    let server = pid_of(&client, "pid").await;
    let child = pid_of(&client, "spawn_child").await;
    drop(client);
    let server_gone = gone_within(server, Duration::from_secs(5)).await;
    let child_gone = gone_within(child, Duration::from_secs(5)).await;
    kill(child);
    kill(server);
    assert!(server_gone, "服务器还在");
    assert!(child_gone, "服务器启动的进程还在");
}

#[tokio::test]
async fn tool_list_changes_are_counted() {
    let client = McpClient::connect(&config(&[])).await.unwrap();
    let mut changed = client.tools_changed();
    assert_eq!(*changed.borrow(), 0);
    client.call_tool("change_tools", json!({})).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), changed.changed())
        .await
        .expect("没收到 tools/list_changed")
        .unwrap();
    assert_eq!(*changed.borrow(), 1);
    client.close().await;
}

/// `npx` 这类名字解析成 `.cmd`，由标准库经 cmd.exe 跑；带空格和 `&` 的参数原样到达，不会被 cmd 当成第二条命令。
#[tokio::test]
async fn a_cmd_wrapper_found_on_path_gets_its_arguments_intact() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("fake-mcp-server.cmd"),
        format!("@\"{}\" %*\r\n", server_exe().display()),
    )
    .unwrap();
    let marker = dir.path().join("pwned.txt");
    let path = format!(
        "{};{}",
        dir.path().display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let tricky = format!("a b&echo x>{}", marker.display());
    let config = ServerConfig::stdio("fake-mcp-server", ["--tag".to_string(), tricky.clone()])
        .env("PATH", path)
        .startup_timeout(Duration::from_secs(20));
    let client = McpClient::connect(&config).await.unwrap();
    let argv: Vec<String> =
        serde_json::from_str(&client.call_tool("argv", json!({})).await.unwrap().text()).unwrap();
    assert_eq!(argv, vec!["--tag".to_string(), tricky]);
    assert!(!marker.exists(), "参数被 cmd 当成了命令");
    client.close().await;
}
