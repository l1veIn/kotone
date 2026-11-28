//! 命令（ADR 040 D1）：能绑热键的一件事。本体的和插件的用同一个结构登记（原则 15）：
//! 插件写在 `plugin.toml` 的 `[[commands]]` 里（和页面、数据开关、游戏事件一样是清单里的静态东西），
//! 本体自己的写在这里的 [`core_commands`]（像本体的工具也用 `ToolSpec` 登记）。
//!
//! 完整名是 `<来源>.<命令 id>`：本体的来源是 `kotone`（第三方插件不能叫这个名字），插件的是插件 id
//! （`voice-input.talk`）。热键表里绑的就是完整名（`hotkeys.bindings[].command`，见 [`crate::hotkey_bindings`]）。
//!
//! 插件不带默认键（D4，照 Obsidian 的 `no-default-hotkeys`）：这里没有默认键这个字段，清单又是
//! `deny_unknown_fields`，写了读不进来。新装时的三个默认键写在本体的默认设置里（[`crate::hotkey_bindings::DEFAULT_BINDINGS`]）。

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 本体命令的来源。
pub const CORE_SOURCE: &str = "kotone";

/// 按一下还是按住。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CommandKind {
    /// 按一下：只在按下时调一次。
    #[default]
    Press,
    /// 按住：按下、松开各调一次（按物理顺序）。
    Hold,
}

/// 按下还是松开（hold 命令两个都有，press 命令只有按下）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Press,
    Release,
}

/// 一个可以绑热键的命令。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all(serialize = "camelCase"))]
pub struct CommandSpec {
    /// 来源里唯一（小写字母、数字、连字符）：`talk`、`channel-cycle`。
    pub id: String,
    /// 名字（热键表里那一行叫什么；中文，给她和外部 Agent，界面用 messages）。
    pub name: String,
    /// 一句话：按了做什么。
    pub description: String,
    #[serde(default)]
    pub kind: CommandKind,
    /// 要“听一句”（语音识别）：识别没在跑时不登记，改成只看不吞——键照常给游戏，按了提示识别还没准备好
    /// （ADR 040 实施记录，主会话定的第 2 条）。
    #[serde(default)]
    pub listens: bool,
    /// 调插件自己的一个工具、带固定参数（D1 的第二种命令，第 3 期落地）：英雄联盟的“喊对面闪现”是
    /// `lol.shout {what: "flash", side: "enemy", channel: "team"}`。按下时照这个工具的权限调（宿主判，和自己加的
    /// “调一个工具”同一条路，`hotkey_custom`），不用语音识别；只能是插件自己的工具、按一下、不听（清单检查）。
    /// 本体的命令没有。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    /// 调工具时的固定参数（对象；不写 = 空对象）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<Value>,
}

impl CommandSpec {
    fn new(id: &str, name: &str, description: &str, listens: bool) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            description: description.into(),
            kind: CommandKind::Press,
            listens,
            tool: None,
            args: None,
        }
    }
}

/// 本体自己的命令（来源 `kotone`）。快捷指令（第 3 期，D5）还不是命令；Esc 固定，不能绑。
pub fn core_commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new(
            "call",
            "呼叫键",
            "按一下她就在气泡里听你说；通话中再按一次挂断",
            true,
        ),
        CommandSpec::new(
            "stop",
            "停下热键",
            "立刻停下她正在做的事和所有自动化，按住的键一起松开",
            false,
        ),
        CommandSpec::new(
            "speech-toggle",
            "启动 / 停止本地语音识别热键",
            "按一下启动本地语音识别，再按一下停止",
            false,
        ),
    ]
}

/// 完整名拆成（来源，命令 id）：`voice-input.talk` → (`voice-input`, `talk`)。
pub fn split_command(command: &str) -> Option<(&str, &str)> {
    let (source, id) = command.split_once('.')?;
    (!source.is_empty() && !id.is_empty()).then_some((source, id))
}

/// 命令 id 的写法（和插件 id 一样）。
pub fn valid_command_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 40
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !id.starts_with('-')
        && !id.ends_with('-')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hotkey_bindings::{CMD_CALL, CMD_SPEECH_TOGGLE, CMD_STOP};

    #[test]
    fn core_commands_are_the_kotone_ones_in_the_legacy_table() {
        let names: Vec<String> = core_commands()
            .iter()
            .map(|c| format!("{CORE_SOURCE}.{}", c.id))
            .collect();
        assert_eq!(names, vec![CMD_CALL, CMD_STOP, CMD_SPEECH_TOGGLE]);
        assert!(core_commands().iter().all(|c| valid_command_id(&c.id)));
        // 呼叫要听；停下、启停识别在识别没跑时也要能用
        let listens: Vec<bool> = core_commands().iter().map(|c| c.listens).collect();
        assert_eq!(listens, vec![true, false, false]);
    }

    #[test]
    fn full_names_split_into_source_and_id() {
        assert_eq!(
            split_command("voice-input.talk"),
            Some(("voice-input", "talk"))
        );
        assert_eq!(split_command("kotone.call"), Some(("kotone", "call")));
        assert_eq!(split_command("talk"), None);
        assert_eq!(split_command(".talk"), None);
        assert!(!valid_command_id("Talk") && !valid_command_id("-a") && !valid_command_id(""));
    }
}
