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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentConfig {
    #[serde(default)]
    pub permissions: Policy,
    /// 不交给 AI 的已保存自动化（默认全部交给，最多 30 个；ADR 014“已确认”）。
    #[serde(default)]
    pub hidden_automations: Vec<String>,
    /// 陪玩热键与快捷指令里的“一句话”交给琴音 Agent（ADR 014 §4，默认开）。
    /// 关闭时仍用旧的单次陪玩；`assistant.route = agent`（转交外部 Agent）时不看这一项。
    #[serde(default = "default_true")]
    pub in_game: bool,
    /// 游戏内交给琴音后，1.5 秒还没出声就先说一句“好，我看看”（ADR 017 §5，默认开）。
    #[serde(default = "default_true")]
    pub progress_speech: bool,
    /// 琴音的身份（ADR 022）：名字、头像。填了的优先于工作区的 IDENTITY.md。新字段。
    #[serde(default)]
    pub identity: IdentityConfig,
    /// 子会话（ADR 022）：缺省模型（空 = 和琴音同一个模型）。新字段。
    #[serde(default)]
    pub subagents: SubagentsConfig,
    /// 一段对话估到这么多 token 时，把较早的部分整理成摘要（照 Goose / DeepSeek Harness 的上下文压缩）；
    /// 0 = 不整理（太长时只能丢最早的轮次）。新字段。
    #[serde(default = "default_compact_at_tokens")]
    pub compact_at_tokens: u64,
    /// 当前角色（工作区 `roles/<id>`；默认 `kotone` = 琴音）。换角色走 `roles.switch`，
    /// 它会同时把名字、头像写进 `identity`，并开一段新对话。新字段。
    #[serde(default = "default_role_id")]
    pub role_id: String,
    /// 对局复盘：游戏一退出，把这一局和琴音的对话、提醒、自动化整理成复盘，写进这个游戏的记忆（默认开）。新字段。
    #[serde(default = "default_true")]
    pub match_recap: bool,
    /// DeepSeek 内容审核（`safety_check`，ADR 026）：审核拒绝时撤回回答并说明（默认开）。别的服务商忽略。新字段。
    #[serde(default = "default_true")]
    pub safety_check: bool,
    /// 游戏里（呼叫、唤醒、快捷指令）的思考强度：`low` / `high` / `max`，`none` = 不思考。
    /// 空 = 不思考（ADR 029：语音回合要快；0.2.0 早期空 = 服务商默认）。只对 DeepSeek 生效（ADR 026）。新字段。
    #[serde(default = "default_game_effort")]
    pub game_effort: String,
    /// 聊天窗口里的思考强度（同上）。新字段。
    #[serde(default)]
    pub chat_effort: String,
    /// 推送时弹出小窗（ADR 043 D6，“角色 → 交互 → 悬浮窗”；原来是消息提示，ADR 039 D4）和旧的“自动卡片”总开关（ADR 039 D3，只读旧值，
    /// ADR 043 D4）：都默认开。新字段（原则 19：旧配置没有这一项时按默认开；旧版本读到它会忽略）。名字是只有自动卡片时起的，不改。
    #[serde(default)]
    pub auto_cards: AutoCardsConfig,
    // `musicSite`（放歌用哪个网站，ADR 031）在 ADR 034 去掉了：放歌改成 `music` 工具，旧配置里的值读到就丢掉。
}

fn default_game_effort() -> String {
    DEFAULT_GAME_EFFORT.into()
}

/// 游戏里默认的思考强度：空，桌面端按“不思考”处理（ADR 029）。玩家本机实测“你能做什么”开着思考
/// 模型要 19.6 秒（157 个推理 token），Pipecat、xiaozhi 对 DeepSeek 也默认关掉思考。
/// 落盘仍是空字符串：不给已有字段写新值（配置兼容）。
pub const DEFAULT_GAME_EFFORT: &str = "";

fn default_role_id() -> String {
    "kotone".into()
}

/// 默认 6.4 万 token：DeepSeek 的上下文有 100 万，但每次请求都会把整段对话带上，
/// 太长了首字更慢、缓存没命中时也更贵；6.4 万大约是几十轮带读屏的语音对话。
pub const DEFAULT_COMPACT_AT_TOKENS: u64 = 64_000;

fn default_compact_at_tokens() -> u64 {
    DEFAULT_COMPACT_AT_TOKENS
}

/// 某个时刻（毫秒）的本地时间 `HH:MM`。
pub fn local_hh_mm(ms: u64) -> String {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_millis_opt(ms as i64)
        .single()
        .map(|t| t.format("%H:%M").to_string())
        .unwrap_or_default()
}

/// DeepSeek `latest_reminder` 的日期部分（ADR 026）：`2026-11-18,星期三`（本地时间，取消息发出的那一刻，
/// 历史里每条都算得出同样的值，前缀缓存不受影响）。
pub fn reminder_date(ms: u64) -> String {
    use chrono::{Datelike, TimeZone};
    const WEEKDAYS: [&str; 7] = [
        "星期一",
        "星期二",
        "星期三",
        "星期四",
        "星期五",
        "星期六",
        "星期日",
    ];
    chrono::Local
        .timestamp_millis_opt(ms as i64)
        .single()
        .map(|t| {
            format!(
                "{},{}",
                t.format("%Y-%m-%d"),
                WEEKDAYS[t.weekday().num_days_from_monday() as usize]
            )
        })
        .unwrap_or_default()
}

/// 现在的本地时间，写进琴音每轮的附注：`2026-11-18 周三 14:05`。
pub fn now_label() -> String {
    use chrono::Datelike;
    const WEEKDAYS: [&str; 7] = ["周一", "周二", "周三", "周四", "周五", "周六", "周日"];
    let now = chrono::Local::now();
    format!(
        "{} {} {}",
        now.format("%Y-%m-%d"),
        WEEKDAYS[now.weekday().num_days_from_monday() as usize],
        now.format("%H:%M")
    )
}

/// 某个时刻（毫秒）的本地时间 `14:05:09`（玩家开口时的截图等）。
pub fn clock_label(at_ms: u64) -> String {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_millis_opt(at_ms as i64)
        .single()
        .map(|t| t.format("%H:%M:%S").to_string())
        .unwrap_or_default()
}

/// 琴音的身份。空 = 用工作区 `IDENTITY.md` 或默认（“琴音”、内置头像）。
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct IdentityConfig {
    pub name: String,
    /// 工作区里的头像文件名（设置页上传，≤ 2 MB）。
    pub avatar: String,
}

/// 推送（ADR 043）的两个旧开关（`agent.autoCards`，名字是只有自动卡片时起的）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AutoCardsConfig {
    /// 旧的“自动卡片”总开关（ADR 039 D3）。界面上去掉了（ADR 043 D4），只读旧值：是 false 时内置插件带卡片的推送算关，
    /// 直到玩家在插件页上动了那一条（`plugins.<id>.pushes.<推送>`，`kotone-plugin-api` 的 `Manifest::push_switch_in`）。
    pub enabled: bool,
    /// 推送卡片时弹出悬浮窗的小窗（ADR 043 D6；原来是消息提示，ADR 039 D4；关了推送照发）。
    pub popup: bool,
}

impl Default for AutoCardsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            popup: true,
        }
    }
}

/// 子会话的缺省设置（照 OpenClaw `agents.defaults.subagents`）。
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SubagentsConfig {
    /// 模型 id；空 = 和琴音同一个模型。
    pub model: String,
}

fn default_true() -> bool {
    true
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            permissions: Policy::default(),
            hidden_automations: Vec::new(),
            in_game: true,
            progress_speech: true,
            identity: IdentityConfig::default(),
            subagents: SubagentsConfig::default(),
            compact_at_tokens: DEFAULT_COMPACT_AT_TOKENS,
            role_id: default_role_id(),
            match_recap: true,
            safety_check: true,
            game_effort: default_game_effort(),
            chat_effort: String::new(),
            auto_cards: AutoCardsConfig::default(),
        }
    }
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
        assert!(empty.in_game, "in-game entry goes to the agent by default");
        let json = serde_json::to_value(AgentConfig::default()).unwrap();
        assert_eq!(json["permissions"]["mode"], "default");
    }

    /// ADR 039 D3、D4：旧配置没有 `autoCards`，读进来两个开关都是开；只写了一个的，另一个照默认。
    #[test]
    fn old_configs_get_auto_cards_and_popups_on() {
        let old: AgentConfig =
            serde_json::from_str(r#"{"inGame": true, "matchRecap": false}"#).unwrap();
        assert!(old.auto_cards.enabled && old.auto_cards.popup);
        let partial: AgentConfig =
            serde_json::from_str(r#"{"autoCards": {"popup": false}}"#).unwrap();
        assert!(partial.auto_cards.enabled && !partial.auto_cards.popup);
        let json = serde_json::to_value(AgentConfig::default()).unwrap();
        assert_eq!(json["autoCards"]["enabled"], true);
        assert_eq!(json["autoCards"]["popup"], true);
    }
}
