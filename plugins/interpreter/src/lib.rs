//! 同传插件（ADR 021、038）。
//!
//! 同传本来就是她的本事：她把“听队友、翻译、显示”交给后台（`agent.start`），后台用本体的耳朵
//! （`voice.listen.start`）听某个程序，每听到一句就翻译（ADR 021：翻译不做成工具、不藏另一个模型）。
//! 这个插件加的是：
//!
//! - 玩家不叫她也能用的一个页面：设好听哪个程序、译成什么、怎么显示，点“开始”就等于玩家对她说了这句话
//!   （[`Host::player_says`]，出现在聊天里），“停下”同理；页面上显示她的耳朵现在在听哪些程序；
//! - 后台每翻好一句调的 [`TOOL_SHOW`]：插件把这一句推送出去（悬浮窗的气泡一行 + 聊天里“这局的同传”卡片），
//!   报事件 `interpreter.line`（ADR 043 D11，见 [`pushes`]）。

use std::sync::{Arc, Mutex, OnceLock};

use kotone_plugin_api::{BoxFuture, Host, Manifest, Plugin, ToolResult, ToolSpec};
use serde_json::{json, Value};

pub mod pushes;

pub const TOOL_STATUS: &str = "interpreter.status";
pub const TOOL_START: &str = "interpreter.start";
pub const TOOL_STOP: &str = "interpreter.stop";
/// 后台每翻好一句调一次：推送这一句（[`pushes`]）。
pub const TOOL_SHOW: &str = "interpreter.show";
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

/// 前三个只给页面用：她自己的做法是 `agent.start` + `voice.listen.start`（规则写在清单的 `[agent]` 里）。
/// [`TOOL_SHOW`] 给后台（不是马上返回的工具，主会话没有它）：每翻好一句调一次。
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
        ToolSpec::new(
            TOOL_SHOW,
            "同传：把翻好的一句给玩家看（在后台当同传时，每翻好一句调一次，代替 danmaku）。text 写译文；heard 写听到的原话；app 写在听的程序（voice.listen 的那个，例如 Discord.exe）。插件把译文显示在悬浮窗的气泡里（玩家选了弹幕时不显示：同一句再用 danmaku），并记进聊天里“这局的同传”卡片（一局一张）。返回 shown：false = 玩家关了这条推送，改用 danmaku 显示。",
            json!({"type": "object", "additionalProperties": false, "required": ["text"], "properties": {
                "text": {"type": "string", "minLength": 1, "maxLength": 300, "description": "译文"},
                "heard": {"type": "string", "maxLength": 300, "description": "听到的原话"},
                "app": {"type": "string", "maxLength": 64, "description": "在听的程序（进程名）"}
            }}),
        )
        .side_effects(&["speaks-or-shows-text"]),
    ]
}

/// 插件自己的设置（`plugins.interpreter.settings`）。
#[derive(Debug, Clone, PartialEq)]
pub struct Prefs {
    /// 听哪个程序（进程名，例如 `Discord.exe`）；空 = 现在的游戏。
    pub app: String,
    /// 译成什么。
    pub target: String,
    /// 用气泡显示：译文进悬浮窗的气泡（推送的一行，ADR 043 D11）。默认是；玩家选了弹幕（`display = "danmaku"`）
    /// 时不是。2026-11-26 以前默认弹幕：那时选的“气泡”照旧是气泡，存了 `danmaku` 的照旧是弹幕。
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
            bubble: settings.get("display").and_then(Value::as_str) != Some("danmaku"),
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

/// “这局的同传”卡片记到哪了（插件只有一份）。
fn desk() -> &'static Mutex<pushes::Desk> {
    static DESK: OnceLock<Mutex<pushes::Desk>> = OnceLock::new();
    DESK.get_or_init(|| Mutex::new(pushes::Desk::default()))
}

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
        arguments: &'a Value,
    ) -> BoxFuture<'a, Result<ToolResult, String>> {
        Box::pin(async move {
            let prefs = Prefs::from_settings(&host.settings());
            let value = match tool {
                TOOL_STATUS => json!({
                    "listening": host.listening(),
                    "sessionListeners": host.background_task_listeners("tongchuan"),
                    "recent": desk().lock().unwrap().recent(),
                    "app": prefs.app,
                    "target": prefs.target,
                    "display": if prefs.bubble { "bubble" } else { "danmaku" },
                    "targets": TARGETS,
                }),
                TOOL_START => {
                    let text = prefs.start_text();
                    host.player_says(&text)?;
                    // 重新点了开始：下一句另起一张卡片（同一局的照旧并在一起）
                    desk().lock().unwrap().reset();
                    json!({"said": text})
                }
                TOOL_STOP => {
                    host.player_says(STOP_TEXT)?;
                    desk().lock().unwrap().reset();
                    json!({"said": STOP_TEXT})
                }
                TOOL_SHOW => {
                    pushes::show(desk(), host.as_ref(), &prefs, arguments, pushes::now_ms())?
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
        // 页面的三个只给页面；interpreter.show 给后台（不马上返回：主会话没有它），会在屏幕上显示字
        for t in specs() {
            if t.name == TOOL_SHOW {
                assert!(t.for_model() && !t.instant && !t.untrusted);
                assert_eq!(t.side_effects, ["speaks-or-shows-text"]);
            } else {
                assert!(!t.for_model() && t.for_app(), "{}", t.name);
            }
        }
        // 她的做法：交给后台、用耳朵听、每句用 interpreter.show 显示（要弹幕时再用 danmaku）、停下时 agent.stop
        for word in [
            "agent.start",
            "voice.listen.start",
            TOOL_SHOW,
            "danmaku",
            "agent.stop",
            STOP_TEXT,
        ] {
            assert!(m.agent.rule.contains(word), "{word}");
        }
        assert!(m.agent.rule.ends_with('\n') && !m.agent.rule.contains('\\'));
    }

    #[test]
    fn the_start_sentence_says_what_the_page_set() {
        // 2026-11-26 起默认气泡（悬浮窗的一行，ADR 043 D11）；存了弹幕的照旧弹幕
        let p = Prefs::from_settings(&json!({}));
        assert_eq!(
            p.start_text(),
            "帮我盯着队友说的话，当同传：听 现在的游戏，每句外语翻成中文，用气泡显示。"
        );
        assert!(!Prefs::from_settings(&json!({"display": "danmaku"})).bubble);
        assert_eq!(
            Prefs::from_settings(&json!({"display": "danmaku"})).start_text(),
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
        // 推送的名字 / 时机 / 做什么、事件的名字（插件页「设置 → 推送」、自动化里的事件）：界面文字和清单里的一样
        for push in &m.pushes {
            let id = push.id.replace('-', "_");
            for (part, want) in [
                ("title", &push.title),
                ("when", &push.when),
                ("what", &push.what),
            ] {
                let key = format!("plugin_interpreter_push_{id}_{part}");
                assert_eq!(zh[&key], want.as_str(), "{key}");
            }
        }
        for event in &m.events {
            let name = event.kind.rsplit('.').next().unwrap();
            let key = format!("plugin_interpreter_event_{name}");
            assert_eq!(zh[&key], event.label.as_str(), "{key}");
        }
        for key in zh.keys() {
            assert!(key.starts_with("plugin_interpreter_"), "{key}");
            assert!(en.contains_key(key), "en 缺 {key}");
        }
    }
}
