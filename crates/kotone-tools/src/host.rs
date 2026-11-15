//! 桌面宿主端口：工具服务经此调用应用级能力（运行时启停、退出、交付回复、助手提交），
//! 本 crate 不依赖 Tauri。桌面端注入真实实现；测试使用 [`FakeAppHost`]（test-support）。
//!
//! 方法在工具服务的工作线程中被调用：同步方法必须快速返回；返回 future 的方法由服务
//! 在当前 tokio 运行时上等待，并与调用期限、服务关闭一起竞争（取消即丢弃 future，
//! 实现必须保证丢弃时停止原生工作，例如停止播报）。

use std::future::Future;
use std::pin::Pin;

use serde_json::Value;

pub type HostFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, String>> + Send + 'a>>;

/// 委托给宿主的配置类工具（[`AppHost::invoke`]）的错误：区分参数错误、忙与执行失败，
/// 服务据此映射为 `INVALID_ARGUMENTS` / `BUSY` / `HOST_ACTION_FAILED`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostError {
    InvalidArguments(String),
    Busy(String),
    Failed(String),
}

impl From<String> for HostError {
    fn from(message: String) -> Self {
        Self::Failed(message)
    }
}

pub type HostInvokeFuture<'a> = Pin<Box<dyn Future<Output = Result<Value, HostError>> + Send + 'a>>;

/// 回复交付方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplyOutput {
    Speech,
    Danmaku,
}

impl ReplyOutput {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "speech" => Some(Self::Speech),
            "danmaku" => Some(Self::Danmaku),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Speech => "speech",
            Self::Danmaku => "danmaku",
        }
    }
}

pub trait AppHost: Send + Sync {
    /// 应用信息（版本、启动方式、助手请求去向等），不含凭证。
    fn app_info(&self) -> Value;
    /// ASR 运行时完整状态（相位、是否需要重启、引擎/模型）。
    fn runtime_status(&self) -> Value;
    fn runtime_start(&self) -> HostFuture<'_, Value>;
    fn runtime_stop(&self) -> HostFuture<'_, Value>;
    /// 安排应用退出（应在本次响应送出后进行）。
    fn request_quit(&self) -> Result<(), String>;
    /// 向用户交付一段文字（播报 / 弹幕），完成后返回回执；future 被丢弃时必须停止交付。
    fn deliver(&self, text: String, outputs: Vec<ReplyOutput>) -> HostFuture<'_, Value>;
    /// 能否交付回复（没有桌面宿主时为 false，`agent.reply` 直接报错而不是先受理）。
    fn can_deliver(&self) -> bool {
        true
    }
    /// 等同于用户在助手文字入口提交（按 `assistant.route` 规划提醒或转交 Agent）。
    fn assistant_submit(&self, text: String) -> HostFuture<'_, Value>;
    /// 宿主是否实现配置类工具（[`crate::catalog::HOST_TOOLS`]）；决定这些工具的 `ready`。
    fn supports_invoke(&self) -> bool {
        false
    }
    /// 执行一个配置类工具（设置、模型、连接、profile、热键、系统、历史、诊断……）。
    /// `args` 已保证是 JSON 对象（缺省为空对象），由宿主按工具解析并校验；状态变更与
    /// 设置页走同一实现。长任务（下载）应受理后立即返回，结果经事件发布。
    fn invoke(&self, tool: &str, _args: Value) -> HostInvokeFuture<'_> {
        let message = format!("{NO_HOST}（{tool}）");
        Box::pin(async move { Err(HostError::Failed(message)) })
    }
}

/// 没有桌面宿主时（例如纯 CLI 测试）：应用级工具明确不可用。
pub struct NoAppHost;

pub(crate) const NO_HOST: &str = "本实例没有桌面宿主，应用级工具不可用";

impl AppHost for NoAppHost {
    fn app_info(&self) -> Value {
        serde_json::json!({"host": "none"})
    }
    fn runtime_status(&self) -> Value {
        serde_json::json!({"phase": "unknown"})
    }
    fn runtime_start(&self) -> HostFuture<'_, Value> {
        Box::pin(async { Err(NO_HOST.to_string()) })
    }
    fn runtime_stop(&self) -> HostFuture<'_, Value> {
        Box::pin(async { Err(NO_HOST.to_string()) })
    }
    fn request_quit(&self) -> Result<(), String> {
        Err(NO_HOST.into())
    }
    fn deliver(&self, _text: String, _outputs: Vec<ReplyOutput>) -> HostFuture<'_, Value> {
        Box::pin(async { Err(NO_HOST.to_string()) })
    }
    fn can_deliver(&self) -> bool {
        false
    }
    fn assistant_submit(&self, _text: String) -> HostFuture<'_, Value> {
        Box::pin(async { Err(NO_HOST.to_string()) })
    }
}
