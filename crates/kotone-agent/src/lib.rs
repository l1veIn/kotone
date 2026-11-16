//! kotone-agent：内置琴音 Agent 的 Kotone 专属层（ADR 014、ADR 015）。
//!
//! - [`catalog`]：工具清单（工具服务 + 内置动作 + 已保存的自动化），分组与查找；
//! - [`permissions`]：允许 / 询问 / 禁止、整体模式、默认规则、询问端口；
//! - [`port`]：把上面两者接成 `kotone_ai::agent::ToolPort`；
//! - [`prompt`]：系统提示（角色、当前游戏说明、已保存自动化摘要）。
//!
//! 循环与模型通信在 `kotone-ai::agent`；工具实现在 `kotone-tools`。本 crate 不依赖 Tauri，
//! 桌面专属的动作（播报、提醒、打字）与询问界面经 [`port::BuiltinHost`] / [`permissions::Asker`] 注入。

pub mod catalog;
pub mod permissions;
pub mod port;
pub mod prompt;

pub use catalog::Catalog;
pub use permissions::{Answer, Asker, Decide, Mode, PermissionRequest, Policy, Rule};
pub use port::{BuiltinHost, KotonePort, Session};

#[cfg(test)]
mod tests;
