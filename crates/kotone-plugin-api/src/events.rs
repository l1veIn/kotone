//! 插件登记的游戏事件（ADR 036 D4、ADR 038 D3 第 10 条）：带游戏中心适配器的插件在 `plugin.toml` 里用
//! `[[events]]` 登记自己会报的事，宿主把开着、关着的插件登记的都并进一份事件目录（[`event_catalog`]）。
//!
//! - **只有这一种登记方式**（原则 15）：本体只登记任何游戏都有的 `game.*`（`kotone_core::games::generic_events`），
//!   游戏专用的事都在插件的清单里；
//! - 类型以插件的适配器 id 开头（`<适配器>.<名字>`），不能用通用的 `game.`，同一个清单里不重名（[`Manifest::parse`] 查）；
//! - 插件只报得出自己登记过的事（宿主照 [`Manifest::declares_event`] 挡掉别的）；
//! - 插件关着、或某种事要的数据开关关着：目录里还有这种事（保存校验认得，旧的自动化不报废），标着现在为什么不会发生。

use kotone_core::games::{CatalogEntry, EventCatalog, EventSpec, GENERIC_ADAPTER, GENERIC_SCOPE};
use kotone_core::settings::Settings;

use crate::{valid_id, Manifest};

impl Manifest {
    /// 检查 `[[events]]`：要有适配器（不能叫 `game` / `generic`），类型以 `<适配器>.` 开头、写法照 ADR 036 D2，
    /// 有中文说明，`needs` 是自己的数据开关，不重名。
    pub(crate) fn check_events(&self) -> Result<(), String> {
        if self.events.is_empty() {
            return Ok(());
        }
        let adapter = match self.adapter.as_deref() {
            Some(a) if valid_id(a) && a != GENERIC_SCOPE && a != GENERIC_ADAPTER => a,
            _ => {
                return Err(format!(
                    "插件 {} 登记了游戏事件，要写 adapter（游戏中心适配器的 id，不能是 game 或 generic）",
                    self.id
                ))
            }
        };
        let mut seen = std::collections::HashSet::new();
        for spec in &self.events {
            spec.check(adapter)
                .map_err(|e| format!("插件 {} 的{e}", self.id))?;
            if !seen.insert(spec.kind.as_str()) {
                return Err(format!("插件 {} 的事件 {} 重名", self.id, spec.kind));
            }
            self.check_needs(&spec.kind, &spec.needs)?;
        }
        Ok(())
    }

    /// 这种事是不是这个插件登记过的（只有登记过的才报得出去）。
    pub fn declares_event(&self, kind: &str) -> bool {
        self.events.iter().any(|e| e.kind == kind)
    }

    /// 插件开着没有（设置 `plugins.<id>.enabled`，没写按清单的默认）。
    pub fn enabled_in(&self, settings: &Settings) -> bool {
        settings.plugin_enabled(&self.id, self.enabled_by_default)
    }

    /// 插件里的某个数据开关开着没有（插件关着也算关；没有这个开关算关）。
    pub fn source_enabled_in(&self, settings: &Settings, source: &str) -> bool {
        self.enabled_in(settings)
            && self.source(source).is_some_and(|s| {
                settings.plugin_source_enabled(&self.id, source, s.enabled_by_default)
            })
    }

    /// 这种事现在为什么不会发生（“英雄联盟插件关着”“英雄联盟插件的“读客户端数据”关着”）；None = 会发生。
    pub fn event_off(&self, settings: &Settings, spec: &EventSpec) -> Option<String> {
        if !self.enabled_in(settings) {
            return Some(format!("{}插件关着", self.name));
        }
        spec.needs
            .iter()
            .find(|s| !self.source_enabled_in(settings, s))
            .map(|s| {
                let name = self.source(s).map_or(s.as_str(), |x| x.name.as_str());
                format!("{}插件的“{name}”关着", self.name)
            })
    }
}

/// 合并后的事件目录：本体的 `game.*`，加上每个插件 `[[events]]` 登记的（按 `manifests` 的顺序；关着的也在，
/// 标着为什么不会发生）。两个插件登记了同一种事时只留先登记的（宿主的测试查不许重名）。
pub fn event_catalog<'a>(
    manifests: impl IntoIterator<Item = &'a Manifest>,
    settings: &Settings,
) -> EventCatalog {
    let mut catalog = EventCatalog::generic();
    for m in manifests {
        let Some(adapter) = m.adapter.as_deref() else {
            continue;
        };
        for spec in &m.events {
            let _ = catalog.add(CatalogEntry {
                spec: spec.clone(),
                adapter: adapter.to_string(),
                plugin: Some((m.id.clone(), m.name.clone())),
                games: m.games.clone(),
                off: m.event_off(settings, spec),
            });
        }
    }
    catalog
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 一个假的插件“示例”（适配器 `demo`）。
    const DEMO: &str = r#"
id = "demo"
name = "示例"
description = "测试用的插件"
games = ["demo"]
adapter = "demo"

[[sources]]
id = "live"
name = "读局内数据"

[[events]]
type = "demo.boss_down"
label = "打掉了首领"
needs = ["live"]
data.team = "哪一边：ally（我方）/ enemy（对面）"
data.bossName = "首领的名字"

[[events]]
type = "demo.my_death"
label = "我阵亡了"
"#;

    fn settings(enabled: Option<bool>, live: Option<bool>) -> Settings {
        let mut s = Settings::default();
        let p = s.plugins.entry("demo".into()).or_default();
        p.enabled = enabled;
        if let Some(live) = live {
            p.sources.insert("live".into(), live);
        }
        s
    }

    #[test]
    fn plugins_register_their_events_in_the_manifest() {
        let m = Manifest::parse(DEMO).unwrap();
        assert_eq!(m.events.len(), 2);
        let boss = &m.events[0];
        assert_eq!(boss.kind, "demo.boss_down");
        assert_eq!(boss.needs, ["live"]);
        assert_eq!(
            boss.data.keys().map(String::as_str).collect::<Vec<_>>(),
            ["bossName", "team"]
        );
        assert!(m.declares_event("demo.my_death") && !m.declares_event("demo.ace"));
        // 发给界面时：{type, label, data, needs}
        let json = serde_json::to_value(&m).unwrap();
        assert_eq!(json["events"][0]["type"], "demo.boss_down");
        assert_eq!(json["events"][1]["data"], serde_json::json!({}));
        // CRLF 的文件一样
        assert_eq!(Manifest::parse(&DEMO.replace('\n', "\r\n")).unwrap(), m);
    }

    #[test]
    fn the_manifest_check_catches_bad_events() {
        for (bad, want) in [
            // 名字不以自己的适配器开头、用通用的 game.、写法不对
            (
                DEMO.replace("type = \"demo.boss_down\"", "type = \"lol.boss_down\""),
                "demo.<名字>",
            ),
            (
                DEMO.replace("type = \"demo.boss_down\"", "type = \"game.boss_down\""),
                "demo.<名字>",
            ),
            (
                DEMO.replace("type = \"demo.boss_down\"", "type = \"demo.BossDown\""),
                "demo.<名字>",
            ),
            // 重名
            (
                DEMO.replace("type = \"demo.my_death\"", "type = \"demo.boss_down\""),
                "重名",
            ),
            // 要的数据开关不是自己的
            (
                DEMO.replace("needs = [\"live\"]", "needs = [\"memory\"]"),
                "memory",
            ),
            // 字段名用 camelCase
            (DEMO.replace("data.bossName", "data.boss_name"), "camelCase"),
            // 没有说明
            (DEMO.replace("label = \"我阵亡了\"", "label = \"\""), "说明"),
            // 登记事件要有适配器，适配器不能叫 game
            (DEMO.replace("adapter = \"demo\"\n", ""), "adapter"),
            (
                DEMO.replace("adapter = \"demo\"", "adapter = \"game\"")
                    .replace("demo.", "game."),
                "adapter",
            ),
            // 不认识的键（打错字不会悄悄失效）
            (
                DEMO.replace("label = \"我阵亡了\"", "label = \"我阵亡了\"\nkind = \"x\""),
                "kind",
            ),
        ] {
            let error = Manifest::parse(&bad).unwrap_err();
            assert!(error.contains(want), "{want} → {error}");
        }
    }

    #[test]
    fn the_catalog_marks_what_will_not_happen_now() {
        let m = Manifest::parse(DEMO).unwrap();
        let catalog = |s: &Settings| event_catalog([&m], s);
        let generic = EventCatalog::generic().entries().len();

        // 开着：两种都会发生，挂在自己的游戏上
        let on = catalog(&Settings::default());
        assert_eq!(on.entries().len(), generic + 2);
        let boss = on.get("demo.boss_down").unwrap();
        assert_eq!(boss.adapter, "demo");
        assert_eq!(boss.games, ["demo"]);
        assert_eq!(boss.plugin, Some(("demo".into(), "示例".into())));
        assert!(boss.off.is_none());

        // 插件关着：还在目录里（认得），标着“示例插件关着”
        let closed = catalog(&settings(Some(false), None));
        assert!(closed.knows("demo.boss_down") && closed.knows("demo.*"));
        for kind in ["demo.boss_down", "demo.my_death"] {
            assert_eq!(
                closed.get(kind).unwrap().off.as_deref(),
                Some("示例插件关着")
            );
        }

        // 只关了数据开关：要它的那种不会发生，别的照常
        let live_off = catalog(&settings(None, Some(false)));
        assert_eq!(
            live_off.get("demo.boss_down").unwrap().off.as_deref(),
            Some("示例插件的“读局内数据”关着")
        );
        assert!(live_off.get("demo.my_death").unwrap().off.is_none());

        // 没有适配器的插件不登记事件；两个插件登记同一种事只留先登记的
        let twin = Manifest::parse(&DEMO.replace("id = \"demo\"", "id = \"demo-two\"")).unwrap();
        assert_eq!(
            event_catalog([&m, &twin], &Settings::default())
                .entries()
                .len(),
            generic + 2
        );
    }
}
