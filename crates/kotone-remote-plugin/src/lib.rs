//! 第三方插件（[MCP 方案](../../../docs/research/2026-11-24-mcp-client.md) 第四节、第 2 期）：
//! **一个第三方 MCP 服务器 = 一个远程插件**（[`RemotePlugin`]，实现现有的 `Plugin`）。
//!
//! - **配置**（[`McpFile`]）：`KOTONE_HOME/mcp.json`，格式就是 `mcpServers`（事实标准，单向门照抄）；Key 写成
//!   `${input:名字}`、存在 Windows 凭据库；开关和信任在 `config.json` 的 `plugins.<id>`。
//! - **导入**（[`import`]）：粘贴配置，或者从 Claude Desktop、VS Code 的配置文件里挑；明文的 Key 搬进凭据库。
//! - **连接**（[`Server`]）：经 `kotone-mcp`（子进程 + Job Object、Streamable HTTP），懒连接、掉线重连、
//!   `tools/list_changed`、工具清单记在磁盘上。
//! - **工具**（[`tools`]）：名字 `<id>.<原名>`；只给页面的不进琴音的清单；询问规则照方案第七节第 3 条
//!   （没标只读的第一次问、可以总是允许；标了只读的，信任这个服务器之前也问）；结果一律不可信；
//!   说明和结果剥掉看不见的字符（[`sanitize`]）。
//! - **它的 MCP App**：`ui://` 不限于 `ui://kotone/`，按“哪个服务器给的”记归属（[`Server::owns_uri`]），
//!   页面从服务器的 `resources/read` 取，CSP 用服务器声明的（[`Server::read_app`]）。
//!
//! 宿主那一半（插件表、给第三方的那份更窄的宿主能力、她的工具清单的数量上限、不进工具服务、
//! 界面用的命令）在桌面端 `remote_plugins.rs`。
//!
//! 照谁做：Goose（rmcp 当客户端、一个扩展一份配置）、VS Code / Claude Desktop（`mcp.json`、信任提示、
//! 配置改了要重新信任）、Codex（`writes` 询问）、OpenClaw（“总是允许”的粒度）、Hermes Agent（剥掉看不见的字符、
//! 子进程只拿配置里写的环境变量）、Claude Code（结果和说明的长度上限）。

mod config;
pub mod import;
mod plugin;
pub mod sanitize;
mod server;
pub mod tools;

pub use config::{
    check_id, placeholder, placeholders, valid_key_name, Launch, McpFile, ServerSpec, FILE_NAME,
    RESERVED_IDS,
};
pub use plugin::{manifest, trusted_in, RemotePlugin, TRUST_KEY};
pub use server::{page_slug, About, Known, Server, Status, CACHE_FILE, RETRY_AFTER};
pub use tools::{fits_main, EFFECT_ACTION, EFFECT_UNTRUSTED_READ, MAX_MAIN_TOOLS};

#[cfg(test)]
mod tests;
