//! 游戏语音插件（ADR 020、038）：她用虚拟麦克风 VB-CABLE 替玩家在游戏语音里对队友说话。
//!
//! - [`voice_chat`]：开关、状态、安装 VB-CABLE、切 Windows 默认麦克风、说一句、自检（原来桌面端的 `voice_chat.rs`）；
//! - [`router`]：真麦克风 + 她的语音 → CABLE Input 的转发与混音（原来系统适配层的 `voice_router`）；
//! - [`cable`]：VB-CABLE 的检测、静默安装、Windows 默认录音设备（原来系统适配层的 `voice_chat`）；
//! - [`config`]：插件自己的设置（原来 config.json 的 `voiceChat` 段）；
//! - [`pushes`]：她对队友说了一句 → 推送“她对队友说了”、事件 `game-voice.said`（ADR 043 D11）。
//!
//! 插件开着 = 转发开着。合成她的声音、在玩家的扬声器上说、改自己的设置都经宿主（[`Host`]）。

use std::sync::{Arc, OnceLock};

use kotone_plugin_api::{BoxFuture, Host, Manifest, Plugin, ToolResult, ToolSpec};
use serde_json::{json, Value};

pub mod cable;
pub mod config;
pub mod pushes;
pub mod router;
pub mod voice_chat;

pub const TOOL_STATUS: &str = "voice_chat.status";
pub const TOOL_INSTALL: &str = "voice_chat.install";
pub const TOOL_DEFAULT_MIC: &str = "voice_chat.default_mic";
pub const TOOL_SAY: &str = "voice_chat.say";
pub const TOOL_TEST: &str = "voice_chat.test";
pub const PAGE: &str = "ui://kotone/game-voice/home";

pub fn manifest() -> &'static Manifest {
    static MANIFEST: OnceLock<Manifest> = OnceLock::new();
    MANIFEST.get_or_init(|| {
        Manifest::parse(include_str!("../plugin.toml")).expect("plugins/game-voice/plugin.toml")
    })
}

/// 工具（名字和说明照搬进插件以前工具服务里的那五个，ADR 020）。
pub fn specs() -> Vec<ToolSpec> {
    let empty = || json!({"type": "object", "additionalProperties": false, "properties": {}});
    vec![
        ToolSpec::new(
            TOOL_STATUS,
            "Game voice chat (ADR 020: 琴音 speaks to teammates through a virtual mic). Returns driverInstalled (VB-CABLE, a VB-Audio donationware virtual audio cable), installing, devices {cableInput, cableOutput}, enabled (the game voice plugin is on = the router runs) / includeMic / micDeviceId / micGain (settings plugins.game-voice.settings.*), running (the mic + speech router is open), error, windowsDefaultMic / windowsCommunicationsMic, windowsDefaultIsCable, canRestoreDefaultMic, ttsCanSpeak (the Windows system voice cannot be sent into the game) and attribution. Setup order: voice_chat.install if not installed → in the game / voice app pick 'CABLE Output' as the microphone, or voice_chat.default_mic for games that only use the Windows default → voice_chat.test.",
            empty(),
        )
        .page(PAGE)
        .instant(),
        ToolSpec::new(
            TOOL_INSTALL,
            "Install VB-CABLE automatically: downloads the official VBCABLE_Driver_Pack46.zip from vb-audio.com (SHA-256 pinned), unpacks it under KOTONE_HOME/drivers and runs the official setup silently as administrator (Windows shows one UAC prompt; the user can refuse). Blocks until setup ends (up to a few minutes). Returns installed, needsReboot (VB-Audio says a reboot may be needed before the devices appear). Does nothing when already installed. Tell the user VB-CABLE is VB-Audio's donationware (vb-cable.com).",
            empty(),
        )
        .side_effects(&["installs-driver", "shows-uac-prompt"]),
        ToolSpec::new(
            TOOL_DEFAULT_MIC,
            "Change the Windows default recording device (default + communications): use=\"cable\" makes 'CABLE Output' the default microphone (for games that only use the Windows default, e.g. PUBG; the previous devices are remembered), use=\"restore\" puts the remembered devices back. Changes a system-wide setting: other apps (Discord, meetings) will also hear the virtual mic until restored.",
            json!({"type": "object", "additionalProperties": false, "required": ["use"], "properties": {"use": {"enum": ["cable", "restore"]}}}),
        )
        .side_effects(&["writes-system-audio"]),
        ToolSpec::new(
            TOOL_SAY,
            "Say one sentence to the player's teammates through the game's voice chat: synthesized once with the current voice, sent into the virtual mic (the player hears it too). Needs voice chat set up (see voice_chat.status) and a voice engine other than the Windows system voice. Other players hear it: keep it short and only say what the player asked you to say. Returns sent, durationMs.",
            json!({"type": "object", "additionalProperties": false, "required": ["text"], "properties": {"text": {"type": "string", "minLength": 1, "maxLength": 200, "description": "要让队友听到的一句话"}}}),
        )
        .side_effects(&["speaks-to-teammates"])
        .instant(),
        ToolSpec::new(
            TOOL_TEST,
            "Check the virtual mic end to end: plays a 1 s test tone into CABLE Input and records CABLE Output at the same time. Returns ok, peak (0-1), note. Teammates in voice chat may hear a short beep.",
            empty(),
        )
        .side_effects(&["speaks-to-teammates"]),
    ]
}

/// 游戏语音插件。
#[derive(Default)]
pub struct GameVoicePlugin {
    state: Arc<voice_chat::VoiceChat>,
}

impl GameVoicePlugin {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Plugin for GameVoicePlugin {
    fn manifest(&self) -> &Manifest {
        manifest()
    }

    fn tools(&self) -> Vec<ToolSpec> {
        specs()
    }

    fn call<'a>(
        &'a self,
        host: Arc<dyn Host>,
        tool: &'a str,
        arguments: &'a Value,
    ) -> BoxFuture<'a, Result<ToolResult, String>> {
        Box::pin(async move {
            let state = self.state.clone();
            let value = match tool {
                TOOL_STATUS => voice_chat::status(&state, host.as_ref()),
                TOOL_INSTALL => voice_chat::install(&state, host.clone()).await?,
                TOOL_DEFAULT_MIC => {
                    let use_cable = match arguments.get("use").and_then(Value::as_str) {
                        Some("cable") => true,
                        Some("restore") => false,
                        _ => return Err("use 只能是 cable 或 restore".into()),
                    };
                    voice_chat::default_mic(host.clone(), use_cable).await?
                }
                TOOL_SAY => {
                    let text = arguments
                        .get("text")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|t| !t.is_empty() && t.chars().count() <= 200)
                        .ok_or("text 不能为空、不超过 200 字")?;
                    voice_chat::say(&state, host.clone(), text).await?
                }
                TOOL_TEST => voice_chat::test(&state, host.as_ref()).await?,
                other => return Err(format!("{other} 不是游戏语音的工具")),
            };
            Ok(ToolResult::content(value))
        })
    }

    /// 启动时：插件开着就开始转发。
    fn start(self: Arc<Self>, host: Arc<dyn Host>) {
        voice_chat::sync(self.state.clone(), host);
    }

    /// 每次保存设置：开关、麦克风、音量变了就重开转发（关掉插件就停）。
    fn settings_changed(&self, host: Arc<dyn Host>) {
        voice_chat::sync(self.state.clone(), host);
    }

    /// 玩家让她停下：还没对队友说完的话也停掉。
    fn stop(&self) {
        self.state.hush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_and_tools_agree() {
        let m = manifest();
        assert_eq!(m.id, "game-voice");
        assert_eq!(m.tool_prefix(), "voice_chat");
        assert!(!m.enabled_by_default, "原来的 voiceChat.enabled 默认关");
        m.check_tools(&specs()).unwrap();
        let names: Vec<String> = specs().into_iter().map(|t| t.name).collect();
        assert_eq!(
            names,
            [
                TOOL_STATUS,
                TOOL_INSTALL,
                TOOL_DEFAULT_MIC,
                TOOL_SAY,
                TOOL_TEST
            ]
        );
        // 主会话照旧有 say 和 status（马上返回），其余给后台
        let instant: Vec<String> = specs()
            .into_iter()
            .filter(|t| t.instant)
            .map(|t| t.name)
            .collect();
        assert_eq!(instant, [TOOL_STATUS, TOOL_SAY]);
        assert!(
            m.agent.rule.contains("voice_chat.say") && !m.agent.rule.contains("voiceChat.enabled")
        );
        assert!(m.agent.rule.ends_with('\n') && !m.agent.rule.contains('\\'));
    }

    /// ADR 043 D11：推送“她对队友说了”是卡片、默认开（插件本身默认关）、不要数据开关，卡片的页面在清单里；事件以插件 id
    /// 开头（没有游戏适配器）、挂在所有游戏上；她的设置说明里写着这条推送的开关在哪。
    #[test]
    fn the_push_and_the_event_are_declared() {
        use crate::pushes::{event_kind, EVENT_SAID, PAGE_SAID, PUSH_SAID, TITLE_SAID};
        use kotone_plugin_api::PushForm;
        let m = manifest();
        let spec = m.push_spec(PUSH_SAID).expect("清单里要有推送 said");
        assert_eq!(spec.title, TITLE_SAID);
        assert_eq!(spec.forms, [PushForm::Card]);
        assert!(spec.enabled_by_default && spec.needs.is_empty());
        assert!(m.page(PAGE_SAID).is_some());
        assert!(m.adapter.is_none());
        let catalog =
            kotone_plugin_api::event_catalog([m], &kotone_core::settings::Settings::default());
        let entry = catalog.get(&event_kind(EVENT_SAID)).unwrap();
        assert!(entry.spec.data.contains_key("text"));
        assert!(entry.spec.private.is_empty(), "她自己说的话：她本来就知道");
        let guide = kotone_core::settings_guide::FIELDS
            .iter()
            .find(|f| f.path == "plugins.*")
            .unwrap()
            .description;
        for push in &m.pushes {
            let path = format!("pushes.{} {}", push.id, push.title);
            assert!(guide.contains(&path), "设置说明里没有 {path}");
        }
        assert!(guide.contains("插件 → 游戏语音 → 设置 → 推送"));
    }

    #[test]
    fn messages_match_the_manifest() {
        let zh: serde_json::Map<String, Value> =
            serde_json::from_str(include_str!("../messages/zh-CN.json")).unwrap();
        let en: serde_json::Map<String, Value> =
            serde_json::from_str(include_str!("../messages/en.json")).unwrap();
        let m = manifest();
        assert_eq!(zh["plugin_game_voice_name"], m.name.as_str());
        assert_eq!(zh["plugin_game_voice_description"], m.description.as_str());
        // 推送的名字 / 时机 / 做什么、事件的名字（插件页「设置 → 推送」、自动化里的事件）：界面文字和清单里的一样
        for push in &m.pushes {
            let id = push.id.replace('-', "_");
            for (part, want) in [
                ("title", &push.title),
                ("when", &push.when),
                ("what", &push.what),
            ] {
                let key = format!("plugin_game_voice_push_{id}_{part}");
                assert_eq!(zh[&key], want.as_str(), "{key}");
            }
        }
        for event in &m.events {
            let name = event.kind.rsplit('.').next().unwrap();
            let key = format!("plugin_game_voice_event_{name}");
            assert_eq!(zh[&key], event.label.as_str(), "{key}");
        }
        for key in zh.keys() {
            assert!(key.starts_with("plugin_game_voice_"), "{key}");
            assert!(en.contains_key(key), "en 缺 {key}");
        }
    }
}
