//! 内置 Agent 的用户设置（config.json `agent` 段，ADR 014 §5 / ADR 015）。
//!
//! 只放可持久化的选择；规则怎么判、怎么问在 `kotone-agent`。配置兼容（docs/development.md）：
//! 这是新字段，旧版本会忽略它；本版本读到不认识的模式或规则值时按默认处理，
//! 不会把整个配置当成损坏。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// 整体模式。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    /// 全部直接执行（用户设为“禁止”的仍禁止）。
    AllowAll,
    /// 只能读状态、看屏幕和播报；其余一律禁止。
    ReadOnly,
    /// 读与常规游戏操作直接执行，有风险的先问。也是不认识的值的去处。
    #[default]
    #[serde(other)]
    Default,
}

/// 单个工具的规则。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Rule {
    Allow,
    Deny,
    /// 也是不认识的值的去处：宁可多问。
    #[serde(other)]
    Ask,
}

/// 用户的权限设置。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Policy {
    #[serde(default)]
    pub mode: Mode,
    /// 按工具名覆盖默认规则。
    #[serde(default)]
    pub rules: BTreeMap<String, Rule>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentConfig {
    #[serde(default)]
    pub permissions: Policy,
    /// 不交给 AI 的已保存自动化（默认全部交给，最多 30 个；ADR 014“已确认”）。
    #[serde(default)]
    pub hidden_automations: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_values_fall_back_instead_of_failing() {
        let config: AgentConfig = serde_json::from_str(
            r#"{"permissions": {"mode": "yolo", "rules": {"settings.update": "maybe", "app.quit": "deny"}}}"#,
        )
        .unwrap();
        assert_eq!(config.permissions.mode, Mode::Default);
        assert_eq!(config.permissions.rules["settings.update"], Rule::Ask);
        assert_eq!(config.permissions.rules["app.quit"], Rule::Deny);
        let empty: AgentConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(empty, AgentConfig::default());
        let json = serde_json::to_value(AgentConfig::default()).unwrap();
        assert_eq!(json["permissions"]["mode"], "default");
    }
}
