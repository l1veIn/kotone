//! 连一个 MCP 服务器要的配置：怎么连（子进程或 HTTP）、超时、告诉服务器“我是谁”。
//!
//! 字段照 `mcpServers` 的事实标准（Claude Desktop / VS Code / Claude Code）：`command` / `args` / `env` / `cwd`
//! 或 `url` / `headers`。读哪个文件、文件长什么样是第 2 期的事（`KOTONE_HOME/mcp.json`），这里只是内存里的样子。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

/// 启动（起程序 + 握手）最多等多久（照 Codex `startup_timeout_sec` 的默认 10 秒）。
pub const DEFAULT_STARTUP_TIMEOUT: Duration = Duration::from_secs(10);
/// 每次请求（调工具、列工具、读资源）最多等多久（照 Codex `tool_timeout_sec` 的默认 60 秒）。
pub const DEFAULT_TOOL_TIMEOUT: Duration = Duration::from_secs(60);

/// 怎么连。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Transport {
    /// 起一个子进程，经标准输入输出说 JSON-RPC。
    Stdio {
        /// 程序：名字（按 PATH 找，Windows 上也认 `.cmd` / `.bat`）或完整路径。
        command: String,
        args: Vec<String>,
        /// 额外的环境变量（在安全基线之上；同名的覆盖基线）。
        env: BTreeMap<String, String>,
        /// 工作目录；不写就用 Kotone 的。
        cwd: Option<PathBuf>,
    },
    /// Streamable HTTP（2025-03-26 起的那种；不做旧的 HTTP+SSE）。
    Http {
        url: String,
        /// 每个请求都带的头（例如 `Authorization: Bearer …`）。
        headers: BTreeMap<String, String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerConfig {
    pub transport: Transport,
    pub startup_timeout: Duration,
    /// 每次请求的超时。
    pub tool_timeout: Duration,
    /// `initialize` 里的 `clientInfo.name`；HTTP 请求的 User-Agent 也用它。
    pub client_name: String,
    /// `initialize` 里的 `clientInfo.version`（Kotone 的版本）。
    pub client_version: String,
}

impl ServerConfig {
    fn new(transport: Transport) -> Self {
        Self {
            transport,
            startup_timeout: DEFAULT_STARTUP_TIMEOUT,
            tool_timeout: DEFAULT_TOOL_TIMEOUT,
            client_name: "kotone".into(),
            client_version: env!("CARGO_PKG_VERSION").into(),
        }
    }

    /// 子进程：`command` 加参数。
    pub fn stdio<I, S>(command: impl Into<String>, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self::new(Transport::Stdio {
            command: command.into(),
            args: args.into_iter().map(Into::into).collect(),
            env: BTreeMap::new(),
            cwd: None,
        })
    }

    /// Streamable HTTP：服务器的地址（例如 `https://mcp-api.op.gg/mcp`）。
    pub fn http(url: impl Into<String>) -> Self {
        Self::new(Transport::Http {
            url: url.into(),
            headers: BTreeMap::new(),
        })
    }

    /// 加一个环境变量（只对子进程有用）。
    pub fn env(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        if let Transport::Stdio { env, .. } = &mut self.transport {
            env.insert(name.into(), value.into());
        }
        self
    }

    /// 工作目录（只对子进程有用）。
    pub fn cwd(mut self, dir: impl Into<PathBuf>) -> Self {
        if let Transport::Stdio { cwd, .. } = &mut self.transport {
            *cwd = Some(dir.into());
        }
        self
    }

    /// 加一个请求头（只对 HTTP 有用）。
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        if let Transport::Http { headers, .. } = &mut self.transport {
            headers.insert(name.into(), value.into());
        }
        self
    }

    pub fn startup_timeout(mut self, timeout: Duration) -> Self {
        self.startup_timeout = timeout;
        self
    }

    pub fn tool_timeout(mut self, timeout: Duration) -> Self {
        self.tool_timeout = timeout;
        self
    }

    /// 告诉服务器“我是谁”。
    pub fn client(mut self, name: impl Into<String>, version: impl Into<String>) -> Self {
        self.client_name = name.into();
        self.client_version = version.into();
        self
    }
}
