//! 英雄联盟插件（ADR 035、036、038）：像 LeagueAkari 那样查这局 10 个人的战绩、找突破口、记对面的
//! 召唤师技能；带游戏中心的 `lol` 适配器（对局阶段、局内的事）。
//!
//! - [`client`]：数据开关“读客户端数据”（`client`）背后的东西：本机的客户端接口（LCU）和局内接口，
//!   战绩、突破口、技能计时，都是纯函数加一个只连 127.0.0.1 的 HTTP 客户端；
//! - [`tools`]：给她和页面的工具（名字、说明、参数）；
//! - [`pages`]：页面的数据（这局、战绩、单局详情，照 MCP Apps 的 `structuredContent`）；
//! - [`stats`]：数据开关“用网上的统计”（`stats`）背后的东西：腾讯 101、aramhex-data、外号和名字表，
//!   以及默认关的 OP.GG（`opgg`）；工具在 `stats_tools.rs`；
//! - [`watcher`]：后台每 2 秒看一次客户端（进加载界面就查战绩、往游戏中心报阶段和局内的事），以及工具的实现；
//! - [`app`]：做成 MCP App 的页面（`app/` 里独立的 HTML，ADR 039 D5），Kotone 和外部宿主显示同一份；
//! - [`chat`]：发到游戏（ADR 041 D3，数据开关“在客户端里发聊天” `chat`）：客户端在前台时（选英雄、房间里）经客户端的
//!   聊天接口发，不按键；
//! - 一键喊话（ADR 041 第 3 期，`watcher/shout.rs`）：`lol.shout` 现生成几行（对面闪现、战绩、开黑），经宿主交给本体唯一的
//!   发话入口（`Host::send_to_game`）；清单 `[[commands]]` 登记四条热键预设调它。
//!
//! 只依赖插件契约（`kotone-plugin-api`）和 `kotone-core` 的纯数据类型；进程、游戏中心、设置都由宿主给。

use std::sync::{Arc, Mutex, OnceLock};

use kotone_plugin_api::{
    BoxFuture, ChatSend, ChatSent, ChatStop, Host, Manifest, Plugin, Resource, ToolResult, ToolSpec,
};
use serde_json::Value;

pub mod app;
pub mod chat;
pub mod client;
pub mod control;
#[cfg(test)]
mod event_catalog_tests;
mod overlays;
pub mod pages;
pub mod stats;
mod stats_tools;
pub mod tools;
mod watcher;

pub use chat::SOURCE_CHAT;
pub use control::SOURCE_CONTROL;
pub use stats_tools::{SOURCE_OPGG, SOURCE_STATS};
pub use watcher::SOURCE_CLIENT;

/// 清单（`plugin.toml`）。
pub fn manifest() -> &'static Manifest {
    static MANIFEST: OnceLock<Manifest> = OnceLock::new();
    MANIFEST.get_or_init(|| {
        Manifest::parse(include_str!("../plugin.toml")).expect("plugins/lol/plugin.toml")
    })
}

/// 英雄联盟插件。状态（连着的客户端、这局的人和战绩、技能记录）都在内存里，这局结束清掉技能记录。
#[derive(Default)]
pub struct LolPlugin {
    state: Arc<Mutex<watcher::State>>,
    /// 网上的统计（第一次用时按宿主给的目录建）。
    stats: OnceLock<stats::Stats>,
    /// 覆盖层（选英雄、海克斯评分）现在的样子。
    overlay: Mutex<overlays::OverlayState>,
    /// 操作客户端（自动接受、自动选 / 禁）做过什么。
    control: Mutex<control::ControlState>,
}

impl LolPlugin {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Plugin for LolPlugin {
    fn manifest(&self) -> &Manifest {
        manifest()
    }

    fn tools(&self) -> Vec<ToolSpec> {
        tools::specs()
    }

    fn call<'a>(
        &'a self,
        host: Arc<dyn Host>,
        tool: &'a str,
        arguments: &'a Value,
    ) -> BoxFuture<'a, Result<ToolResult, String>> {
        Box::pin(async move { self.tool(&host, tool, arguments).await })
    }

    fn start(self: Arc<Self>, host: Arc<dyn Host>) {
        self.watch(host);
    }

    fn resource(&self, uri: &str) -> Option<Resource> {
        app::resource(uri)
    }

    /// 页面要的图（ADR 039 D6）：素材层 [`client::assets`]（缓存 → 客户端 → CommunityDragon），只放行三个前缀下的图片。
    fn asset<'a>(
        &'a self,
        host: Arc<dyn Host>,
        path: &'a str,
    ) -> BoxFuture<'a, Option<kotone_plugin_api::Asset>> {
        Box::pin(async move {
            let asset = self.fetch_asset(host.as_ref(), path).await.ok()?;
            Some(kotone_plugin_api::Asset {
                mime_type: asset.mime.into(),
                bytes: asset.bytes,
            })
        })
    }

    /// 发到游戏（ADR 041 D3）：客户端在前台时（选英雄、房间里）经客户端的聊天接口发（[`chat`]）。
    fn send_to_game<'a>(
        &'a self,
        host: Arc<dyn Host>,
        request: &'a ChatSend,
    ) -> BoxFuture<'a, Result<ChatSent, ChatStop>> {
        Box::pin(async move { self.send_chat(host.as_ref(), request).await })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_and_tools_agree() {
        let m = manifest();
        assert_eq!(m.id, "lol");
        assert_eq!(m.games, ["lol"]);
        assert_eq!(m.adapter.as_deref(), Some("lol"));
        assert!(m
            .source(SOURCE_CLIENT)
            .is_some_and(|s| s.enabled_by_default && !s.risk.is_empty()));
        m.check_tools(&tools::specs()).unwrap();
        // 发到游戏（ADR 041 第 2 期）：只登记客户端，不登记游戏本身（对局里照旧是本体按键）；要“读客户端数据”和
        // “在客户端里发聊天”；后者默认开、带风险提示
        assert_eq!(m.send_to_game.len(), 1);
        let chat = &m.send_to_game[0];
        assert_eq!(chat.game, "lol");
        assert!(chat.covers(crate::client::CLIENT_PROCESS));
        assert!(!chat.covers(crate::client::GAME_PROCESS));
        assert_eq!(chat.needs, [SOURCE_CLIENT, SOURCE_CHAT]);
        assert!(m
            .source(SOURCE_CHAT)
            .is_some_and(|s| s.enabled_by_default && s.risk.contains("issues/480")));
        assert!(
            m.agent.rule.starts_with("- 英雄联盟") && m.agent.rule.ends_with("不发玩家名）。\n")
        );
        // 一键喊话（ADR 041 第 3 期）：四条预设都调 lol.shout、发队伍频道、不听（清单里没有键这个字段）
        let presets: Vec<(&str, &str, &str)> = m
            .commands
            .iter()
            .map(|c| {
                let args = c.args.as_ref().unwrap();
                assert_eq!(c.tool.as_deref(), Some(tools::TOOL_SHOUT), "{}", c.id);
                assert_eq!(args["channel"], "team", "{}", c.id);
                assert!(!c.listens, "{}", c.id);
                assert!(
                    watcher::shout_args_ok(args),
                    "{} 的参数 lol.shout 认不得",
                    c.id
                );
                (
                    c.name.as_str(),
                    args["what"].as_str().unwrap(),
                    args["side"].as_str().unwrap(),
                )
            })
            .collect();
        assert_eq!(
            presets,
            [
                ("喊对面闪现", "flash", "enemy"),
                ("喊对面战绩", "records", "enemy"),
                ("喊我方战绩", "records", "ally"),
                ("喊开黑情况", "premade", "all"),
            ]
        );
        assert!(
            !m.agent.rule.contains('\\'),
            "TOML 的续行没吃掉：{}",
            m.agent.rule
        );
    }

    /// 界面上的文字（messages/<语言>.json）和清单一致：名字、说明、开关的名字和风险提示（中文）；英文也都有。
    #[test]
    fn messages_match_the_manifest() {
        let zh: serde_json::Map<String, Value> =
            serde_json::from_str(include_str!("../messages/zh-CN.json")).unwrap();
        let en: serde_json::Map<String, Value> =
            serde_json::from_str(include_str!("../messages/en.json")).unwrap();
        let m = manifest();
        let mut expect = vec![
            ("plugin_lol_name".to_string(), m.name.clone()),
            ("plugin_lol_description".to_string(), m.description.clone()),
        ];
        for c in &m.commands {
            expect.push((
                format!("plugin_lol_command_{}_name", c.id.replace('-', "_")),
                c.name.clone(),
            ));
        }
        // 内置推送（ADR 043 D4）：插件页「设置 → 推送」照这几条画
        for p in &m.pushes {
            let id = p.id.replace('-', "_");
            expect.push((format!("plugin_lol_push_{id}_title"), p.title.clone()));
            expect.push((format!("plugin_lol_push_{id}_when"), p.when.clone()));
            expect.push((format!("plugin_lol_push_{id}_what"), p.what.clone()));
        }
        for s in &m.sources {
            expect.push((format!("plugin_lol_source_{}_name", s.id), s.name.clone()));
            expect.push((
                format!("plugin_lol_source_{}_desc", s.id),
                s.description.clone(),
            ));
            expect.push((format!("plugin_lol_source_{}_risk", s.id), s.risk.clone()));
        }
        for (key, text) in expect {
            assert_eq!(
                zh.get(&key).and_then(Value::as_str),
                Some(text.as_str()),
                "{key}"
            );
            assert!(
                en.get(&key)
                    .and_then(Value::as_str)
                    .is_some_and(|t| !t.is_empty()),
                "en {key}"
            );
        }
        for key in zh.keys() {
            assert!(
                key.starts_with("plugin_lol_"),
                "插件的文案要以 plugin_lol_ 开头：{key}"
            );
            assert!(en.contains_key(key), "en 缺 {key}");
        }
    }
}
