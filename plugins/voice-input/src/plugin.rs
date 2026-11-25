//! 语音输入插件在插件清单里的那一面（ADR 038 D5）：清单。给她的工具没有：
//! 往游戏聊天里打字归本体，她替玩家发言是本体的 `game_chat.send`（ADR 039 D2），关掉这个插件也照样能用。
//!
//! 听写本身（热键 → 听一句 → 听到以后）不经插件契约：本体的“听一句”直接调 [`crate::VoiceInput`]
//! （`AfterHearing`），宿主按插件开关决定登不登记语音输入热键。听写的字交给本体的“发到游戏”入口
//! （`Orchestrator::send_to_game`：一把输入锁、等修饰键、发送时挡住玩家按键，ADR 041、ADR 032）打进聊天框。

use std::sync::{Arc, OnceLock};

use kotone_plugin_api::{BoxFuture, Host, Manifest, Plugin, ToolResult, ToolSpec};
use serde_json::Value;

pub const PAGE: &str = "ui://kotone/voice-input/home";

/// 清单里登记的命令（`[[commands]]` 的 id；完整名是 `voice-input.<id>`，ADR 040 D1）。
pub const CMD_TALK: &str = "talk";
pub const CMD_CHANNEL_CYCLE: &str = "channel-cycle";
pub const CMD_RESEND_LAST: &str = "resend-last";
pub const CMD_PP_TOGGLE: &str = "post-process-toggle";
pub const CMD_PP_CYCLE: &str = "post-process-cycle";

pub fn manifest() -> &'static Manifest {
    static MANIFEST: OnceLock<Manifest> = OnceLock::new();
    MANIFEST.get_or_init(|| {
        Manifest::parse(include_str!("../plugin.toml")).expect("plugins/voice-input/plugin.toml")
    })
}

#[derive(Default)]
pub struct VoiceInputPlugin;

impl VoiceInputPlugin {
    pub fn new() -> Self {
        Self
    }
}

impl Plugin for VoiceInputPlugin {
    fn manifest(&self) -> &Manifest {
        manifest()
    }

    /// 没有工具（`game_chat.send` 在本体，ADR 039 D2）。
    fn tools(&self) -> Vec<ToolSpec> {
        Vec::new()
    }

    fn call<'a>(
        &'a self,
        _host: Arc<dyn Host>,
        tool: &'a str,
        _arguments: &'a Value,
    ) -> BoxFuture<'a, Result<ToolResult, String>> {
        Box::pin(async move { Err(format!("语音输入插件没有 {tool}")) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_gives_her_no_tools() {
        let m = manifest();
        assert_eq!(m.id, "voice-input");
        assert!(m.enabled_by_default, "语音输入一直都能用：默认开");
        // 往游戏聊天里打字归本体（ADR 039 D2）：game_chat.send 不在这里登记，也没有给她的话
        assert!(m.tool_prefix.is_none());
        assert!(VoiceInputPlugin::new().tools().is_empty());
        assert_eq!(m.agent, kotone_plugin_api::AgentText::default());
        assert!(m.pages.iter().any(|p| p.uri == PAGE));
    }

    /// 清单里的命令就是本体对照表里语音输入的那 5 个（旧字段 `hotkey.key`、`channelCycleHotkey`…… 迁进热键表时
    /// 写的就是这些完整名，ADR 040 D8）：“说话”按住，全部要语音识别；不给她工具，也不带默认键。
    #[test]
    fn commands_are_the_five_voice_input_keys() {
        use kotone_plugin_api::CommandKind;
        let m = manifest();
        let ids: Vec<&str> = m.commands.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                CMD_TALK,
                CMD_CHANNEL_CYCLE,
                CMD_RESEND_LAST,
                CMD_PP_TOGGLE,
                CMD_PP_CYCLE
            ]
        );
        let legacy: Vec<String> = kotone_core::hotkey_bindings::LEGACY
            .iter()
            .filter(|l| l.command.starts_with("voice-input."))
            .map(|l| l.command.to_string())
            .collect();
        let full: Vec<String> = ids.iter().map(|id| format!("{}.{id}", m.id)).collect();
        assert_eq!(full, legacy);
        assert!(m.commands.iter().all(|c| c.listens));
        let kinds: Vec<CommandKind> = m.commands.iter().map(|c| c.kind).collect();
        assert_eq!(kinds[0], CommandKind::Hold);
        assert!(kinds[1..].iter().all(|k| *k == CommandKind::Press));
    }

    #[test]
    fn messages_match_the_manifest() {
        let zh: serde_json::Map<String, Value> =
            serde_json::from_str(include_str!("../messages/zh-CN.json")).unwrap();
        let en: serde_json::Map<String, Value> =
            serde_json::from_str(include_str!("../messages/en.json")).unwrap();
        let m = manifest();
        assert_eq!(zh["plugin_voice_input_name"], m.name.as_str());
        assert_eq!(zh["plugin_voice_input_description"], m.description.as_str());
        for key in zh.keys() {
            assert!(key.starts_with("plugin_voice_input_"), "{key}");
            assert!(en.contains_key(key), "en 缺 {key}");
        }
    }
}
