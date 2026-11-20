//! kotone-agent：内置琴音 Agent 的 Kotone 专属层（ADR 014、ADR 015）。
//!
//! - [`catalog`]：工具清单（工具服务 + 内置动作 + 已保存的自动化），分组与查找；
//! - [`permissions`]：允许 / 询问 / 禁止、整体模式、默认规则、询问端口；
//! - [`port`]：把上面两者接成 `kotone_ai::agent::ToolPort`；
//! - [`prompt`]：系统提示（角色、当前游戏说明、已保存自动化摘要）；
//! - [`workspace`]：琴音的工作区（SOUL、身份、记忆、游戏记忆、Skill，ADR 022）；
//! - [`roles`]：可切换的角色（名字、头像、性格、声音、唤醒词各一份；记忆与 Skill 共用）；
//! - [`subagents`]：交给后台的事（子代理，ADR 029 第二期）的纯逻辑；
//! - [`exec`]：后台执行命令、读写文件（ADR 030，kotone-exec 转出给桌面端执行）。
//!
//! 循环与模型通信在 `kotone-ai::agent`；工具实现在 `kotone-tools`。本 crate 不依赖 Tauri，
//! 桌面专属的动作（播报、提醒、打字、执行命令）与询问界面经 [`port::BuiltinHost`] /
//! [`permissions::Asker`] 注入。

/// ADR 030 的执行引擎（规则、“读过外面的东西”的标记、执行）。工具清单里的说明由
/// [`catalog::exec_entries`] 从这里取，执行由桌面端经 [`port::BuiltinHost::exec`] 调用。
pub use kotone_exec as exec;

pub mod catalog;
pub mod permissions;
pub mod port;
pub mod prompt;
pub mod roles;
pub mod subagents;
pub mod workspace;

pub use catalog::Catalog;
pub use permissions::{Answer, Asker, Decide, Mode, PermissionRequest, Policy, Rule};
pub use port::{BuiltinHost, KotonePort, Scope, Session};

#[cfg(test)]
mod tests;
