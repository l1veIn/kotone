//! 同传插件（ADR 021、038）。
//!
//! 同传本来就是她的本事：她把“听队友、翻译、显示”交给后台（`agent.start`），后台用本体的耳朵
//! （`voice.listen.start`）听某个程序，每听到一句就翻译、用弹幕或气泡显示（ADR 021：翻译不做成工具、不藏
//! 另一个模型）。这个插件只加玩家不叫她也能用的那部分：一个页面，设好听哪个程序、译成什么、怎么显示，
//! 点“开始”就等于玩家对她说了这句话（[`Host::player_says`]，出现在聊天里），“停下”同理；页面上显示
//! 她的耳朵现在在听哪些程序。

use std::sync::{Arc, OnceLock};

use kotone_plugin_api::{BoxFuture, Host, Manifest, Plugin, ToolResult, ToolSpec};
use serde_json::{json, Value};

pub const TOOL_STATUS: &str = "interpreter.status";
pub const TOOL_START: &str = "interpreter.start";
pub const TOOL_STOP: &str = "interpreter.stop";
pub const PAGE: &str = "ui://kotone/interpreter/home";

/// 译成什么（值, 名字）。第一个是默认。
pub const TARGETS: &[&str] = &["中文", "英语", "日语", "韩语"];
/// 停下时替玩家说的话（她的规则里写着这样说就 `agent.stop`）。
pub const STOP_TEXT: &str = "停止同传";

pub fn manifest() -> &'static Manifest {
    static MANIFEST: OnceLock<Manifest> = OnceLock::new();
    MANIFEST.get_or_init(|| {
        Manifest::parse(include_str!("../plugin.toml")).expect("plugins/interpreter/plugin.toml")
    })
}

/// 都只给页面用：她自己的做法是 `agent.start` + `voice.listen.start`（规则写在清单的 `[agent]` 里）。
pub fn specs() -> Vec<ToolSpec> {
    let empty = || json!({"type": "object", "properties": {}});
    vec![
        ToolSpec::new(
            TOOL_STATUS,
            "同传页面：设置和她的耳朵现在在听哪些程序",
            empty(),
        )
        .app_only(),
        ToolSpec::new(TOOL_START, "同传页面：替玩家对她说“开始同传”", empty()).app_only(),
        ToolSpec::new(TOOL_STOP, "同传页面：替玩家对她说“停止同传”", empty()).app_only(),
    ]
}

/// 插件自己的设置（`plugins.interpreter.settings`）。
#[derive(Debug, Clone, PartialEq)]
pub struct Prefs {
    /// 听哪个程序（进程名，例如 `Discord.exe`）；空 = 现在的游戏。
    pub app: String,
    /// 译成什么。
    pub target: String,
    /// 用气泡显示（默认弹幕）。
    pub bubble: bool,
}

impl Prefs {
    pub fn from_settings(settings: &Value) -> Self {
        let text = |key: &str| {
            settings
                .get(key)
                .and_then(Value::as_str)
                .map(str::trim)
                .unwrap_or("")
                .to_string()
        };
        let target = text("target");
        Self {
            app: text("app").chars().take(64).collect(),
            target: if TARGETS.contains(&target.as_str()) {
                target
            } else {
                TARGETS[0].to_string()
            },
            bubble: settings.get("display").and_then(Value::as_str) == Some("bubble"),
        }
    }

    /// 点“开始”时替玩家说的话（开头和玩家平时说的一样：“帮我盯着队友”）。
    pub fn start_text(&self) -> String {
        let app = if self.app.is_empty() {
            "现在的游戏".to_string()
        } else {
            self.app.clone()
        };
        let display = if self.bubble { "气泡" } else { "弹幕" };
        format!(
            "帮我盯着队友说的话，当同传：听 {app}，每句外语翻成{}，用{display}显示。",
            self.target
        )
    }
}

/// 同传插件。
pub struct InterpreterPlugin;

impl Plugin for InterpreterPlugin {
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
        _arguments: &'a Value,
    ) -> BoxFuture<'a, Result<ToolResult, String>> {
        Box::pin(async move {
            let prefs = Prefs::from_settings(&host.settings());
            let value = match tool {
                TOOL_STATUS => json!({
                    "listening": host.listening(),
                    "app": prefs.app,
                    "target": prefs.target,
                    "display": if prefs.bubble { "bubble" } else { "danmaku" },
                    "targets": TARGETS,
                }),
                TOOL_START => {
                    let text = prefs.start_text();
                    host.player_says(&text)?;
                    json!({"said": text})
                }
                TOOL_STOP => {
                    host.player_says(STOP_TEXT)?;
                    json!({"said": STOP_TEXT})
                }
                other => return Err(format!("{other} 不是同传的工具")),
            };
            Ok(ToolResult::content(value))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_and_tools_agree() {
        let m = manifest();
        assert_eq!(m.id, "interpreter");
        assert!(m.enabled_by_default, "同传以前一直能用：默认开");
        m.check_tools(&specs()).unwrap();
        assert!(specs().iter().all(|t| !t.for_model() && t.for_app()));
        // 她的做法：交给后台、用耳朵听、停下时 agent.stop
        for word in ["agent.start", "voice.listen.start", "agent.stop", STOP_TEXT] {
            assert!(m.agent.rule.contains(word), "{word}");
        }
        assert!(m.agent.rule.ends_with('\n') && !m.agent.rule.contains('\\'));
    }

    #[test]
    fn the_start_sentence_says_what_the_page_set() {
        let p = Prefs::from_settings(&json!({}));
        assert_eq!(
            p.start_text(),
            "帮我盯着队友说的话，当同传：听 现在的游戏，每句外语翻成中文，用弹幕显示。"
        );
        let p = Prefs::from_settings(
            &json!({"app": " Discord.exe ", "target": "日语", "display": "bubble"}),
        );
        assert_eq!(
            p.start_text(),
            "帮我盯着队友说的话，当同传：听 Discord.exe，每句外语翻成日语，用气泡显示。"
        );
        // 不认识的语言按默认
        assert_eq!(
            Prefs::from_settings(&json!({"target": "火星文"})).target,
            "中文"
        );
    }

    #[test]
    fn messages_match_the_manifest() {
        let zh: serde_json::Map<String, Value> =
            serde_json::from_str(include_str!("../messages/zh-CN.json")).unwrap();
        let en: serde_json::Map<String, Value> =
            serde_json::from_str(include_str!("../messages/en.json")).unwrap();
        let m = manifest();
        assert_eq!(zh["plugin_interpreter_name"], m.name.as_str());
        assert_eq!(zh["plugin_interpreter_description"], m.description.as_str());
        for key in zh.keys() {
            assert!(key.starts_with("plugin_interpreter_"), "{key}");
            assert!(en.contains_key(key), "en 缺 {key}");
        }
    }
}
