//! 语音输入插件在插件清单里的那一面（ADR 038 D5）：清单、推送卡片上“重发”的工具（只给页面）。给她的工具没有：
//! 往游戏聊天里打字归本体，她替玩家发言是本体的 `game_chat.send`（ADR 039 D2），关掉这个插件也照样能用。
//! 推送和事件（ADR 043 D10）见 [`crate::pushes`]：宿主在启动时交来（[`Plugin::start`]），发完一句由“听到以后”那一面推。
//!
//! 听写本身（热键 → 听一句 → 听到以后）不经插件契约：本体的“听一句”直接调 [`crate::VoiceInput`]
//! （`AfterHearing`），宿主按插件开关决定登不登记语音输入热键。听写的字交给本体的“发到游戏”入口
//! （`Orchestrator::send_to_game`：一把输入锁、等修饰键、发送时挡住玩家按键，ADR 041、ADR 032）打进聊天框。

use std::sync::{Arc, OnceLock};

use kotone_plugin_api::{BoxFuture, Host, Manifest, Plugin, ToolResult, ToolSpec};
use serde_json::{json, Value};

use crate::pushes::TOOL_RESEND;
use crate::send::Resent;
use crate::VoiceInputExt;

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

    /// 只有推送卡片上的“重发”，只给页面（玩家点的）；给她的没有（`game_chat.send` 在本体，ADR 039 D2）。
    fn tools(&self) -> Vec<ToolSpec> {
        vec![ToolSpec::new(
            TOOL_RESEND,
            "把一句没发出去的话再发一次（推送卡片上的“重发”按钮）：回到当时要发去的游戏窗口、用当时的频道发；切不回去就不发。",
            json!({
                "type": "object",
                "properties": {
                    "id": {"type": "string", "description": "那一句的 id（卡片数据里的 id，识别记录的 sessionId）"},
                    "channel": {
                        "type": "object",
                        "description": "卡片上记的当时的频道（Kotone 重启过时用）：{game, id}",
                        "properties": {"game": {"type": "string"}, "id": {"type": "string"}}
                    }
                },
                "required": ["id"]
            }),
        )
        .app_only()
        .side_effects(&["input"])]
    }

    fn call<'a>(
        &'a self,
        _host: Arc<dyn Host>,
        tool: &'a str,
        arguments: &'a Value,
    ) -> BoxFuture<'a, Result<ToolResult, String>> {
        Box::pin(async move {
            if tool != TOOL_RESEND {
                return Err(format!("语音输入插件没有 {tool}"));
            }
            let id = arguments
                .get("id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .ok_or("要写 id（卡片数据里的 id）")?;
            let channel = arguments.get("channel").and_then(|c| {
                Some((
                    c.get("game")?.as_str()?.to_string(),
                    c.get("id")?.as_str()?.to_string(),
                ))
            });
            let Some(orch) = crate::pushes::attached() else {
                return Ok(resend_result(Resent::Refused(
                    "语音输入还没准备好，等一下再点重发".into(),
                )));
            };
            let Some(voice) = orch.voice_input() else {
                return Ok(resend_result(Resent::Refused(
                    "语音输入没接上，没法重发".into(),
                )));
            };
            Ok(resend_result(voice.resend_card(&orch, id, channel).await))
        })
    }

    /// 宿主交来的那一份留给推送台（发完一句推卡片、报事件）。
    fn start(self: Arc<Self>, host: Arc<dyn Host>) {
        crate::pushes::set_host(host);
    }
}

/// 重发的结果 → 给页面的（`content.status`：sent / failed / refused；卡片本身由推送原地更新）。
fn resend_result(resent: Resent) -> ToolResult {
    ToolResult::content(match resent {
        Resent::Sent(o) => json!({
            "status": "sent",
            "game": o.game,
            "gameName": o.game_name,
            "channel": o.channel,
            "channelName": o.channel_name,
        }),
        Resent::Failed(reason) => json!({"status": "failed", "reason": reason}),
        Resent::Refused(reason) => json!({"status": "refused", "reason": reason}),
    })
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
        let tools = VoiceInputPlugin::new().tools();
        assert!(tools.iter().all(|t| !t.for_model()), "没有给她的工具");
        // 只有推送卡片上的“重发”，只给页面，会动键盘
        assert_eq!(
            tools.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
            [TOOL_RESEND]
        );
        assert!(tools[0].for_app() && tools[0].side_effects == ["input"]);
        m.check_tools(&tools).unwrap();
        assert_eq!(m.agent, kotone_plugin_api::AgentText::default());
        assert!(m.pages.iter().any(|p| p.uri == PAGE));
    }

    /// ADR 043 D10：两条推送（发出去的话、没发出去）都是卡片、默认开、不要数据开关，卡片的页面在清单里；两件事
    /// （发出去一句、没发出去）以插件 id 开头（它没有游戏适配器），进合并后的事件目录、挂在所有游戏上。
    #[test]
    fn pushes_and_events_are_declared() {
        use crate::pushes::{
            event_kind, EVENT_FAILED, EVENT_SENT, PAGE_FAILED, PAGE_SENT, PUSH_FAILED, PUSH_SENT,
            TITLE_FAILED, TITLE_SENT,
        };
        use kotone_plugin_api::PushForm;
        let m = manifest();
        for (id, title, page) in [
            (PUSH_SENT, TITLE_SENT, PAGE_SENT),
            (PUSH_FAILED, TITLE_FAILED, PAGE_FAILED),
        ] {
            let spec = m
                .push_spec(id)
                .unwrap_or_else(|| panic!("清单里要有推送 {id}"));
            assert_eq!(spec.title, title);
            assert_eq!(spec.forms, [PushForm::Card]);
            assert!(spec.enabled_by_default && spec.needs.is_empty());
            assert!(m.page(page).is_some(), "{page}");
        }
        assert!(m.adapter.is_none(), "不是游戏插件，没有游戏适配器");
        let catalog =
            kotone_plugin_api::event_catalog([m], &kotone_core::settings::Settings::default());
        for name in [EVENT_SENT, EVENT_FAILED] {
            let kind = event_kind(name);
            assert!(m.declares_event(&kind), "{kind}");
            let entry = catalog.get(&kind).unwrap();
            assert!(entry.games.is_empty() && entry.off.is_none());
        }
        let sent = catalog.get(&event_kind(EVENT_SENT)).unwrap();
        for field in ["text", "channel", "channelName"] {
            assert!(sent.spec.data.contains_key(field), "{field}");
        }
        // 发出去的那句话不给她看（game.state），只交给自动化：她只经“发出去的话”推送的说明知道，玩家关得掉
        assert_eq!(sent.spec.private, ["text"]);
        // 她的设置说明（settings.describe）里写着这两条推送的开关在哪，她才帮得上“把语音输入的推送关了”
        let guide = kotone_core::settings_guide::FIELDS
            .iter()
            .find(|f| f.path == "plugins.*")
            .unwrap()
            .description;
        for push in &m.pushes {
            let path = format!("pushes.{} {}", push.id, push.title);
            assert!(guide.contains(&path), "设置说明里没有 {path}");
        }
        assert!(guide.contains("插件 → 语音输入 → 设置 → 推送"));
        assert!(catalog
            .get(&event_kind(EVENT_FAILED))
            .unwrap()
            .spec
            .data
            .contains_key("reason"));
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
        // 热键表里那一行的名字（ADR 040）：界面文字和清单里给她看的一样
        for command in &m.commands {
            let key = format!(
                "plugin_voice_input_command_{}_name",
                command.id.replace('-', "_")
            );
            assert_eq!(zh[&key], command.name.as_str(), "{key}");
        }
        assert_eq!(zh["plugin_voice_input_description"], m.description.as_str());
        // 推送的名字 / 时机 / 做什么、事件的名字（插件页「设置 → 推送」、自动化里的事件）：界面文字和清单里的一样
        for push in &m.pushes {
            let id = push.id.replace('-', "_");
            for (part, want) in [
                ("title", &push.title),
                ("when", &push.when),
                ("what", &push.what),
            ] {
                let key = format!("plugin_voice_input_push_{id}_{part}");
                assert_eq!(zh[&key], want.as_str(), "{key}");
            }
        }
        for event in &m.events {
            let name = event.kind.rsplit('.').next().unwrap();
            let key = format!("plugin_voice_input_event_{name}");
            assert_eq!(zh[&key], event.label.as_str(), "{key}");
        }
        for key in zh.keys() {
            assert!(key.starts_with("plugin_voice_input_"), "{key}");
            assert!(en.contains_key(key), "en 缺 {key}");
        }
    }
}
