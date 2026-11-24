//! Streamable HTTP：用进程内的测试服务器（rmcp 自己的服务端，`kotone_mcp::testing`）测会话复用、`clientInfo`、
//! 请求头、错误、超时、读资源。

use std::time::{Duration, Instant};

use kotone_mcp::testing::{handler, text_reply, TestServer};
use kotone_mcp::{McpClient, McpError, ServerConfig, Stage};
use serde_json::{json, Value};

fn tools() -> Value {
    json!([
        {"name": "lookup", "description": "查一个英雄", "inputSchema": {"type": "object"}},
        {"name": "slow", "inputSchema": {"type": "object"}},
        {"name": "broken", "inputSchema": {"type": "object"}},
        {"name": "refuse", "inputSchema": {"type": "object"}},
    ])
}

async fn server() -> TestServer {
    TestServer::start(
        tools(),
        json!([{"uri": "ui://test/page", "name": "page", "mimeType": "text/html;profile=mcp-app", "text": "<p>页面</p>"}]),
        handler(|name, args| async move {
            match name.as_str() {
                "lookup" => text_reply(&format!(
                    "class {} {{}}",
                    args["champion"].as_str().unwrap_or("?")
                )),
                "slow" => {
                    tokio::time::sleep(Duration::from_secs(3)).await;
                    text_reply("late")
                }
                "broken" => Err((-32602, "bad".to_string())),
                _ => Ok(json!({"isError": true, "content": [{"type": "text", "text": "no such champion"}]})),
            }
        }),
    )
    .await
}

#[tokio::test]
async fn one_session_for_many_calls() {
    let server = server().await;
    let config = ServerConfig::http(server.url())
        .header("x-kotone-test", "yes")
        .client("kotone", "0.2.0");
    let client = McpClient::connect(&config).await.unwrap();
    assert_eq!(client.server_info().name, "kotone-test-server");
    assert_eq!(
        client.server_info().instructions.as_deref(),
        Some("测试服务器")
    );
    assert_eq!(client.process_id(), None);

    let a = client
        .call_tool("lookup", json!({"champion": "IRELIA"}))
        .await
        .unwrap();
    assert_eq!(a.text(), "class IRELIA {}");
    let b = client
        .call_tool("lookup", json!({"champion": "YASUO"}))
        .await
        .unwrap();
    assert_eq!(b.text(), "class YASUO {}");
    assert_eq!(client.list_tools().await.unwrap().len(), 4);

    let log = server.log();
    assert_eq!(log.initializes, 1, "只握手一次：{log:?}");
    // initialize、notifications/initialized、两次 tools/call、tools/list
    assert_eq!(log.posts, 5, "{log:?}");
    assert_eq!(
        log.posts_with_session, 4,
        "握手以后每个请求都带着会话：{log:?}"
    );
    let init = log.initialize.unwrap();
    assert_eq!(init["clientInfo"]["name"], "kotone");
    assert_eq!(init["clientInfo"]["version"], "0.2.0");
    assert_eq!(init["protocolVersion"], "2025-11-25");
    assert_eq!(log.user_agent.as_deref(), Some("kotone"));
    assert_eq!(
        log.custom_headers.get("x-kotone-test").map(String::as_str),
        Some("yes")
    );
    assert_eq!(log.calls.len(), 2);
    assert_eq!(log.calls[1].1["champion"], "YASUO");
    client.close().await;
    assert!(!client.is_connected());
    assert_eq!(
        client.call_tool("lookup", json!({})).await.unwrap_err(),
        McpError::Closed
    );
}

#[tokio::test]
async fn errors_keep_their_kind() {
    let server = server().await;
    let client = McpClient::connect(&ServerConfig::http(server.url()))
        .await
        .unwrap();
    match client.call_tool("broken", json!({})).await {
        Err(McpError::Server { code, message }) => {
            assert_eq!(code, -32602);
            assert_eq!(message, "bad");
        }
        other => panic!("{other:?}"),
    }
    let refused = client.call_tool("refuse", json!({})).await.unwrap();
    assert!(refused.is_error);
    assert_eq!(refused.text(), "no such champion");
    // 出过错，会话照样能用
    assert_eq!(
        client
            .call_tool("lookup", json!({"champion": "AHRI"}))
            .await
            .unwrap()
            .text(),
        "class AHRI {}"
    );
    client.close().await;
}

#[tokio::test]
async fn a_slow_call_times_out() {
    let server = server().await;
    let client = McpClient::connect(
        &ServerConfig::http(server.url()).tool_timeout(Duration::from_millis(300)),
    )
    .await
    .unwrap();
    let started = Instant::now();
    assert_eq!(
        client.call_tool("slow", json!({})).await.unwrap_err(),
        McpError::Timeout {
            stage: Stage::Request,
            after: Duration::from_millis(300)
        }
    );
    assert!(started.elapsed() < Duration::from_secs(2));
    client.close().await;
}

#[tokio::test]
async fn resources_are_listed_and_read() {
    let server = server().await;
    let client = McpClient::connect(&ServerConfig::http(server.url()))
        .await
        .unwrap();
    let list = client.list_resources().await.unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].uri, "ui://test/page");
    let contents = client.read_resource("ui://test/page").await.unwrap();
    assert_eq!(contents[0].text.as_deref(), Some("<p>页面</p>"));
    assert_eq!(
        contents[0].mime_type.as_deref(),
        Some("text/html;profile=mcp-app")
    );
    client.close().await;
}

#[tokio::test]
async fn nobody_listening_is_a_connect_error() {
    // 先占一个端口再放掉：大概率没人在听
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let config = ServerConfig::http(format!("http://127.0.0.1:{port}/mcp"))
        .startup_timeout(Duration::from_secs(10));
    let error = McpClient::connect(&config).await.unwrap_err();
    assert!(matches!(error, McpError::Connect(_)), "{error:?} / {error}");
}
