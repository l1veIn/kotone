//! Kotone 的 MCP 客户端（[MCP 方案](../../../docs/research/2026-11-24-mcp-client.md) 4.2、第 1 期）。
//!
//! 在官方 Rust SDK `rmcp` 上包一层，只做两种传输：
//!
//! - **子进程（stdio）**：起一个程序，经标准输入输出说 JSON-RPC。程序放进 Windows Job Object
//!   （`KILL_ON_JOB_CLOSE`，`kotone_platform_windows::job`）：关掉连接时连同它启动的进程一起结束，
//!   Kotone 自己退出或崩溃时系统关掉句柄，也一起结束。环境变量只给配置里写的，加一组安全基线
//!   （照 Codex 的 `WINDOWS_CORE_ENV_VARS`、Hermes 的“只传白名单”），不把 Kotone 自己的环境整个传下去。
//!   `npx` / `uvx` 这类名字按 PATH + PATHEXT 解析成真实路径（照 Codex）；解析出来是 `.cmd` / `.bat` 的，
//!   由 Rust 标准库经 `cmd.exe` 跑，参数转义和拒绝不安全的参数都是标准库做的（Rust 1.77.2 起，CVE-2024-24576）。
//! - **Streamable HTTP**（不做旧的 HTTP+SSE）：会话复用；服务器说会话过期（404）时 rmcp 自己重新握手。
//!
//! 对外的接口：[`McpClient::connect`] → [`McpClient::list_tools`] / [`McpClient::call_tool`] /
//! [`McpClient::list_resources`] / [`McpClient::read_resource`] → [`McpClient::close`]。
//! 超时照 Codex（启动 10 秒、每次请求 60 秒，可配置）。服务器发 `notifications/tools/list_changed` 时
//! [`McpClient::tools_changed`] 的计数加一（第 2 期用：从下一段对话起生效，ADR 024）。
//!
//! 对外的类型（[`Tool`]、[`ToolCallResult`]……）是协议原样的字段，不露 rmcp 的类型：rmcp 变得快，
//! 换版本时只改这个 crate（原则 13：每个模块藏住一个可能会变的决定）。
//!
//! 另有 [`fetch_icon`]：下载 `serverInfo.icons` 里 `https://` 的头像（ADR 049 D7，只认 https、≤200 KB、不跳到别的主机）。
//!
//! 不做：OAuth、`.mcpb`、目录（第 6 期）；显示 MCP Apps（第 3 期，这里先把 `read_resource` 留好）。
//!
//! 照谁做：Goose（`aaif-goose/goose`，Apache-2.0）用 rmcp 当客户端的包法（每次请求带超时、
//! 子进程不弹窗口、留着 stderr 给启动失败时的说明）；Codex（`openai/codex`，Apache-2.0）的超时、
//! 环境变量基线和程序路径解析；Hermes Agent（MIT）的“子进程只拿白名单里的环境变量”。

mod client;
mod config;
mod error;
mod http_transport;
mod icon;
mod stdio;
mod types;

#[cfg(feature = "test-support")]
pub mod testing;

pub use client::McpClient;
pub use config::{
    Handshake, ServerConfig, Transport, DEFAULT_STARTUP_TIMEOUT, DEFAULT_TOOL_TIMEOUT,
};
pub use error::{status_text, McpError, Stage};
pub use icon::{fetch_icon, sniff_icon, Icon, ICON_MAX_BYTES, ICON_TIMEOUT};
pub use types::{Resource, ResourceContents, ServerInfo, Tool, ToolCallResult};
