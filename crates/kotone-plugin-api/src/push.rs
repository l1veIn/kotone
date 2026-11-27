//! 推送（ADR 043 D3–D5）：插件按“时机 → 推送”发给玩家的东西，**不调模型、不花钱**。三种样子：
//!
//! - **卡片**：发进聊天（插件页面的小尺寸版本 + 给页面的数据，和工具结果的聊天卡片同一种）；
//! - **一行**：悬浮窗气泡里的一行字，可以分几格（三选一是一行三列，每格一个判断）；
//! - **语音**：她说一句固定的短话，几个说法轮着用。
//!
//! 全都记进聊天（一行和语音是一条小字记录）。给她的那句（`note`）作为一条隐藏的说明追加在对话末尾（ADR 039 D3）。
//!
//! 清单 `[[pushes]]`（[`PushSpec`]）声明插件会推哪几条：时机、做什么、哪几种样子、多要紧、默认开不开、要哪些数据开关。
//! 插件页上每条一个开关（`plugins.<id>.pushes.<推送 id>`），**由宿主执行**（[`Manifest::push_off_in`]）：关着的、
//! 没声明的、要的数据开关关着的，宿主直接丢掉（[`PushOutcome::Off`]）；只留这条声明过的样子、按 [`Push::shaped`] 截字。
//! 同一个 `key` 再推一次就是更新（D5）：内容变了原地重画，一样就什么都不做。
//!
//! 照 Android 的通知渠道（`NotificationChannel`：App 在清单里声明会发哪几类，系统给每类画开关、由系统执行开关）和
//! `NotificationManager.notify(id, …)`（同一个 id 再发一次就是更新）；Telegram 的 `editMessageText`。
//! 只给内置插件（单向门，ADR 039 D3）：[`crate::Host::push`] 的默认实现拒绝。

use kotone_core::settings::Settings;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{valid_id, yes, Manifest};

/// 推送的样子（清单 `forms`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PushForm {
    /// 卡片：发进聊天，悬浮窗的小窗里也弹出来。
    Card,
    /// 一行：悬浮窗气泡里的一行字（可以分几格）。
    Line,
    /// 语音：她说一句固定的短话。
    Voice,
}

/// 多要紧（语音推送谁先说，ADR 043 D3）：缺省 `normal`。
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum PushPriority {
    Low,
    #[default]
    Normal,
    High,
}

/// 清单里的一条内置推送（`[[pushes]]`）。插件页照它画一行：名字、时机、做什么、样子、开关。
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all(serialize = "camelCase"))]
pub struct PushSpec {
    /// 小写字母、数字和连字符（`opening`、`augment-pick`）；开关在 `plugins.<插件>.pushes.<id>`。
    pub id: String,
    /// 名字（“开局战绩”）。
    pub title: String,
    /// 时机（“进加载、10 个人的战绩查完（最多等 20 秒）”）。
    pub when: String,
    /// 做什么（“发一张双方战绩的卡片”）。
    pub what: String,
    /// 会用哪几种样子；推送里别的样子宿主丢掉。
    pub forms: Vec<PushForm>,
    /// 一行在悬浮窗的气泡里显示多久（ADR 043 D6）：`true` = 一直显示到插件说这条结束（[`crate::Host::push_done`]；
    /// 三选一：卡片在屏幕上时一直显示，卡片消失就收起）；缺省 `false` = 显示几秒就收起（一句提醒）。要有 `line` 这种样子。
    /// 只管悬浮窗：聊天里的记录照旧留着。
    #[serde(default)]
    pub until_done: bool,
    #[serde(default)]
    pub priority: PushPriority,
    #[serde(default = "yes")]
    pub enabled_by_default: bool,
    /// 要哪些数据开关开着（有一个关着就不推）。
    #[serde(default)]
    pub needs: Vec<String>,
}

/// 插件发的一条推送（[`crate::Host::push`]）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Push {
    /// 清单里 `[[pushes]]` 的 id；没声明、关着的宿主直接丢掉。
    pub trigger: String,
    /// 同一个插件同一个 key 再推一次就是更新（例如 `opening:<对局 id>`：同一局、同一种）。
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub card: Option<PushCard>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<PushLine>,
    /// 同一句话的几个说法，宿主轮着用；空 = 不说。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub voice: Vec<String>,
    /// 给她的一句（不超过 [`Push::NOTE_MAX`] 字）；空 = 不给。里面有别人起的名字，宿主按不可信数据交给模型。
    /// 要少给：只在关键时刻带（选人锁定、选人结束），中间的变化只更新卡片。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
    /// 更新时要提醒玩家（这张卡片上面已经有别的消息时，D5）：宿主把它挪到聊天最下面（2026-11-27 修订，同一个 key
    /// 仍然只有一条），悬浮窗再弹一次。不带就原地改、位置不动。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub alert: bool,
}

/// 卡片：清单 `pages` 里的一个页面 + 给它的数据。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PushCard {
    /// 卡片的名字（“开局战绩”），聊天里卡片上面那一行和给她的那句都用它。
    pub title: String,
    /// `ui://kotone/<插件>/<页面>`，要是清单 `pages` 里的。
    pub uri: String,
    /// 页面参数（照 MCP Apps 的 `_meta.ui`；没有为 null）。
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub params: Value,
    /// 给页面的数据（和工具结果的 `structuredContent` 一样：存进对话记录，**不发给模型**）。
    #[serde(default)]
    pub data: Value,
}

/// 一行：几格字（最多 [`PushLine::MAX_CELLS`] 格）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PushLine {
    pub cells: Vec<PushCell>,
}

/// 一行里的一格。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PushCell {
    /// 最多 [`PushCell::MAX_CHARS`] 个字。
    pub text: String,
    /// 好坏（界面上的颜色）；没有 = 普通。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tone: Option<PushTone>,
}

/// 一格的好坏：好 / 可以 / 不好。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PushTone {
    Good,
    Ok,
    Bad,
}

/// [`crate::Host::push`] 的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PushOutcome {
    /// 新的一条（这个 key 第一次推）。
    Posted,
    /// 同一个 key、内容变了：更新了（原地；带 `alert` 的挪到了聊天最下面）。
    Updated,
    /// 同一个 key、内容一样：什么都没做。
    Unchanged,
    /// 没推：这条推送关着（玩家在插件页关了、插件或它要的数据开关关着）、清单里没有，或者现在没有她的聊天。
    Off,
}

impl PushLine {
    /// 一行最多几格。
    pub const MAX_CELLS: usize = 3;
}

impl PushCell {
    /// 一格最多几个字（分了几格的一行：三选一的“首选 / 可选 / 可刷新”）。
    pub const MAX_CHARS: usize = 12;
    /// 只有一格的一行最多几个字：一整句话（同传翻好的一句，ADR 043 D11），气泡里显示得下两行左右。
    pub const ONE_CELL_MAX_CHARS: usize = 40;
}

impl Push {
    /// 给她的那句最多几个字（ADR 039：约 120 字；多出的宿主截掉）。
    pub const NOTE_MAX: usize = 160;
    /// 语音的一个说法最多几个字（固定的短话）。
    pub const VOICE_MAX: usize = 20;

    /// 宿主收推送时先做的：只留清单里这条声明过的样子；一行最多 3 格、每格最多 12 个字（空的格去掉；只有一格时是一整句，
    /// 最多 40 个字），语音每个说法最多 20 个字（空的去掉），`note` 最多 160 字。
    pub fn shaped(mut self, spec: &PushSpec) -> Self {
        let has = |form| spec.forms.contains(&form);
        if !has(PushForm::Card) {
            self.card = None;
        }
        self.line = self.line.filter(|_| has(PushForm::Line)).and_then(|line| {
            let cells: Vec<PushCell> = line
                .cells
                .into_iter()
                .filter(|c| !c.text.trim().is_empty())
                .take(PushLine::MAX_CELLS)
                .collect();
            let max = if cells.len() == 1 {
                PushCell::ONE_CELL_MAX_CHARS
            } else {
                PushCell::MAX_CHARS
            };
            let cells: Vec<PushCell> = cells
                .into_iter()
                .map(|c| PushCell {
                    text: clip(&c.text, max),
                    tone: c.tone,
                })
                .collect();
            (!cells.is_empty()).then_some(PushLine { cells })
        });
        self.voice = if has(PushForm::Voice) {
            self.voice
                .iter()
                .map(|v| v.trim().chars().take(Push::VOICE_MAX).collect::<String>())
                .filter(|v| !v.is_empty())
                .collect()
        } else {
            Vec::new()
        };
        self.note = clip(&self.note, Push::NOTE_MAX);
        if let Some(card) = &mut self.card {
            card.title = card.title.trim().to_string();
        }
        self.key = self.key.trim().to_string();
        self
    }

    /// 截过以后的检查：有 key；至少还有一种样子；卡片的页面是这个插件清单 `pages` 里的、有名字、参数是对象。
    pub fn check(&self, manifest: &Manifest) -> Result<(), String> {
        if self.key.trim().is_empty() {
            return Err("推送要有 key".into());
        }
        if self.card.is_none() && self.line.is_none() && self.voice.is_empty() {
            return Err(format!(
                "推送「{}」里没有清单声明过的样子（卡片 / 一行 / 语音）",
                self.trigger
            ));
        }
        if let Some(card) = &self.card {
            if card.title.trim().is_empty() {
                return Err("推送的卡片要有 title".into());
            }
            if manifest.page(&card.uri).is_none() {
                return Err(format!(
                    "{} 不是「{}」插件清单里的页面",
                    card.uri, manifest.name
                ));
            }
            if !card.params.is_null() && !card.params.is_object() {
                return Err("推送的卡片的 params 要是对象".into());
            }
        }
        Ok(())
    }
}

/// 截到 `max` 个字，多了在末尾加“…”。
pub fn clip(text: &str, max: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

impl Manifest {
    /// 检查 `[[pushes]]`：id 的写法、不重名；有名字、时机、做什么；样子至少一种、不重复；`needs` 是自己的数据开关。
    pub(crate) fn check_pushes(&self) -> Result<(), String> {
        let mut seen = std::collections::HashSet::new();
        for push in &self.pushes {
            if !valid_id(&push.id) || !seen.insert(push.id.as_str()) {
                return Err(format!(
                    "插件 {} 的推送 “{}” 重名或写法不对（只能用小写字母、数字和连字符）",
                    self.id, push.id
                ));
            }
            if push.title.trim().is_empty()
                || push.when.trim().is_empty()
                || push.what.trim().is_empty()
            {
                return Err(format!(
                    "插件 {} 的推送 {} 要写 title、when、what（插件页上照着画）",
                    self.id, push.id
                ));
            }
            let mut forms = std::collections::HashSet::new();
            if push.forms.is_empty() || !push.forms.iter().all(|f| forms.insert(*f)) {
                return Err(format!(
                    "插件 {} 的推送 {} 的 forms 至少写一种（card / line / voice），不重复",
                    self.id, push.id
                ));
            }
            if push.until_done && !push.forms.contains(&PushForm::Line) {
                return Err(format!(
                    "插件 {} 的推送 {} 写了 until_done，forms 里要有 line（它只管悬浮窗里的那一行）",
                    self.id, push.id
                ));
            }
            self.check_needs(&format!("推送 {}", push.id), &push.needs)?;
        }
        Ok(())
    }

    /// 清单里的一条推送。
    pub fn push_spec(&self, id: &str) -> Option<&PushSpec> {
        self.pushes.iter().find(|p| p.id == id)
    }

    /// 这条推送的开关在哪个位置（只看这一条，不看插件和数据开关开没开；插件页上画的就是它）：
    /// 玩家动过（`plugins.<id>.pushes.<推送>`）按他的；没动过按清单的默认——但旧配置里“自动卡片”总开关
    /// （`agent.autoCards.enabled`）是关的，**只有卡片**的推送算关（读旧写新，ADR 043 D4：rc.9 的自动卡片就是这样的；
    /// 还有一行、语音的，例如海斗三选一，不受它管）。清单里没有这条 = 关。
    pub fn push_switch_in(&self, settings: &Settings, id: &str) -> bool {
        let Some(spec) = self.push_spec(id) else {
            return false;
        };
        if let Some(on) = settings.plugin_push(&self.id, id) {
            return on;
        }
        if !settings.agent.auto_cards.enabled && spec.forms.iter().all(|f| *f == PushForm::Card) {
            return false;
        }
        spec.enabled_by_default
    }

    /// 这条推送现在为什么不会推（“英雄联盟插件关着”“英雄联盟插件的“读客户端数据”关着”“英雄联盟的推送“开局战绩”关着”）；
    /// None = 会推。清单里没有这条也算不会推。宿主收推送时照它丢掉。
    pub fn push_off_in(&self, settings: &Settings, id: &str) -> Option<String> {
        let Some(spec) = self.push_spec(id) else {
            return Some(format!("{}插件没有声明推送 {id}", self.name));
        };
        if let Some(off) = self.needs_off(settings, &spec.needs) {
            return Some(off);
        }
        (!self.push_switch_in(settings, id))
            .then(|| format!("{}的推送“{}”关着", self.name, spec.title))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEMO: &str = r#"
id = "demo"
name = "示例"
description = "测试用的插件"

[[sources]]
id = "live"
name = "读局内数据"

[[pages]]
uri = "ui://kotone/demo/match"
title = "这局"

[[pushes]]
id = "opening"
title = "开局战绩"
when = "进加载"
what = "一张双方战绩的卡片"
forms = ["card"]
needs = ["live"]

[[pushes]]
id = "augment-pick"
title = "海克斯三选一"
when = "三选一出现、每次刷新"
what = "三张各一个判断"
forms = ["line", "voice"]
priority = "high"
enabled_by_default = false
until_done = true
"#;

    fn demo() -> Manifest {
        Manifest::parse(DEMO).unwrap()
    }

    /// 默认设置合并上一段 JSON（和界面保存一样按字段合并）。
    fn settings(patch: &str) -> Settings {
        fn merge(into: &mut Value, patch: Value) {
            match (into, patch) {
                (Value::Object(into), Value::Object(patch)) => {
                    for (k, v) in patch {
                        merge(into.entry(k).or_insert(Value::Null), v);
                    }
                }
                (into, patch) => *into = patch,
            }
        }
        let mut value = serde_json::to_value(Settings::default()).unwrap();
        merge(&mut value, serde_json::from_str(patch).unwrap());
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn pushes_parse_with_defaults_and_go_to_the_ui_in_camel_case() {
        let m = demo();
        assert_eq!(m.pushes.len(), 2);
        let opening = m.push_spec("opening").unwrap();
        assert_eq!(opening.forms, [PushForm::Card]);
        assert_eq!(opening.priority, PushPriority::Normal, "缺省 normal");
        assert!(opening.enabled_by_default, "缺省开");
        assert!(!opening.until_done, "缺省显示几秒就收起");
        let pick = m.push_spec("augment-pick").unwrap();
        assert_eq!(pick.priority, PushPriority::High);
        assert!(pick.until_done);
        assert!(
            PushPriority::High > PushPriority::Normal && PushPriority::Normal > PushPriority::Low
        );
        let v = serde_json::to_value(&m).unwrap();
        assert_eq!(v["pushes"][1]["enabledByDefault"], false);
        assert_eq!(
            v["pushes"][1]["forms"],
            serde_json::json!(["line", "voice"])
        );
        // 没写 [[pushes]] 的旧清单照样读
        let old = Manifest::parse("id = \"x\"\nname = \"x\"\ndescription = \"x\"\n").unwrap();
        assert!(old.pushes.is_empty());
    }

    #[test]
    fn manifest_pushes_are_checked() {
        let base = "id = \"demo\"\nname = \"示例\"\ndescription = \"说明\"\n[[sources]]\nid = \"live\"\nname = \"读\"\n";
        let push = |body: &str| format!("{base}[[pushes]]\n{body}\n");
        let ok = "id = \"a\"\ntitle = \"t\"\nwhen = \"w\"\nwhat = \"x\"\nforms = [\"card\"]";
        assert!(Manifest::parse(&push(ok)).is_ok());
        for bad in [
            // 重名
            format!("{ok}\n[[pushes]]\n{ok}"),
            // 写法不对
            ok.replace("id = \"a\"", "id = \"A b\""),
            // 没有时机
            ok.replace("when = \"w\"", "when = \" \""),
            // 没有样子、样子重复、不认得的样子
            ok.replace("[\"card\"]", "[]"),
            ok.replace("[\"card\"]", "[\"card\", \"card\"]"),
            ok.replace("[\"card\"]", "[\"popup\"]"),
            // 要的数据开关不存在
            format!("{ok}\nneeds = [\"nope\"]"),
            // 不认得的字段
            format!("{ok}\nsound = true"),
            // until_done 只管一行
            format!("{ok}\nuntil_done = true"),
        ] {
            assert!(Manifest::parse(&push(&bad)).is_err(), "应该不收：{bad}");
        }
    }

    /// 开关由宿主执行：插件关着、要的数据开关关着、这一条关着、清单里没有，都不推；读旧：总开关关着时只有卡片的算关，
    /// 玩家在插件页上动了这一条就按他的。
    #[test]
    fn the_host_decides_whether_a_push_is_on() {
        let m = demo();
        let fresh = settings("{}");
        assert_eq!(m.push_off_in(&fresh, "opening"), None);
        assert!(m.push_switch_in(&fresh, "opening"));
        assert!(m.push_off_in(&fresh, "augment-pick").is_some(), "默认关");
        assert!(m.push_off_in(&fresh, "nope").is_some(), "清单里没有");
        assert!(!m.push_switch_in(&fresh, "nope"));

        let plugin_off = settings(r#"{"plugins":{"demo":{"enabled":false}}}"#);
        assert!(m
            .push_off_in(&plugin_off, "opening")
            .unwrap()
            .contains("插件关着"));
        let needs_off = settings(r#"{"plugins":{"demo":{"sources":{"live":false}}}}"#);
        assert!(m
            .push_off_in(&needs_off, "opening")
            .unwrap()
            .contains("读局内数据"));
        let switched =
            settings(r#"{"plugins":{"demo":{"pushes":{"opening":false,"augment-pick":true}}}}"#);
        assert!(m
            .push_off_in(&switched, "opening")
            .unwrap()
            .contains("开局战绩"));
        assert_eq!(m.push_off_in(&switched, "augment-pick"), None);
        // 关掉插件时开关的位置不变（插件页上灰着）
        assert!(m.push_switch_in(&plugin_off, "opening"));

        // 读旧：“自动卡片”总开关关着 → 只有卡片的推送算关，别的照默认
        let legacy = settings(r#"{"agent":{"autoCards":{"enabled":false,"popup":true}}}"#);
        assert!(!m.push_switch_in(&legacy, "opening"));
        assert!(m.push_off_in(&legacy, "opening").is_some());
        // 卡片之外还有一行、语音的（三选一：问要不要下载模型的那张是卡片）不受总开关管
        let mixed = Manifest::parse(
            &DEMO
                .replace(
                    "forms = [\"line\", \"voice\"]",
                    "forms = [\"line\", \"voice\", \"card\"]",
                )
                .replace("enabled_by_default = false\n", ""),
        )
        .unwrap();
        assert_eq!(
            mixed.push_spec("augment-pick").unwrap().forms,
            [PushForm::Line, PushForm::Voice, PushForm::Card]
        );
        assert!(mixed.push_switch_in(&legacy, "augment-pick"));
        assert!(!mixed.push_switch_in(&legacy, "opening"));
        let legacy_line = settings(
            r#"{"agent":{"autoCards":{"enabled":false}},"plugins":{"demo":{"pushes":{"augment-pick":true}}}}"#,
        );
        assert_eq!(m.push_off_in(&legacy_line, "augment-pick"), None);
        // 玩家在插件页上打开了这一条：按他的
        let touched = settings(
            r#"{"agent":{"autoCards":{"enabled":false}},"plugins":{"demo":{"pushes":{"opening":true}}}}"#,
        );
        assert_eq!(m.push_off_in(&touched, "opening"), None);
    }

    /// 宿主只留声明过的样子、按规矩截字；截完什么都不剩、卡片的页面不是自己的，都不收。
    #[test]
    fn pushes_are_shaped_by_the_manifest() {
        let m = demo();
        let pick = m.push_spec("augment-pick").unwrap();
        let cell = |text: &str, tone| PushCell {
            text: text.into(),
            tone,
        };
        let push = Push {
            trigger: "augment-pick".into(),
            key: " pick:1 ".into(),
            card: Some(PushCard {
                title: "不该有".into(),
                uri: "ui://kotone/demo/match".into(),
                ..PushCard::default()
            }),
            line: Some(PushLine {
                cells: vec![
                    cell("首选", Some(PushTone::Good)),
                    cell(" ", None),
                    cell("可选可选可选可选可选可选可选可选", Some(PushTone::Ok)),
                    cell("可刷新", Some(PushTone::Bad)),
                    cell("第四格", None),
                ],
            }),
            voice: vec!["中间那个".into(), "".into(), "长".repeat(30)],
            note: "长".repeat(400),
            alert: false,
        }
        .shaped(pick);
        assert!(push.card.is_none(), "这一条没声明卡片");
        assert_eq!(push.key, "pick:1");
        let cells = &push.line.as_ref().unwrap().cells;
        assert_eq!(cells.len(), PushLine::MAX_CELLS, "空的格去掉，最多 3 格");
        assert_eq!(cells[0].tone, Some(PushTone::Good));
        assert_eq!(cells[1].text.chars().count(), PushCell::MAX_CHARS);
        assert!(cells[1].text.ends_with('…'));
        assert_eq!(cells[2].text, "可刷新");
        assert_eq!(push.voice.len(), 2, "空的说法去掉");
        assert_eq!(push.voice[1].chars().count(), Push::VOICE_MAX);
        assert_eq!(push.note.chars().count(), Push::NOTE_MAX);
        assert!(push.check(&m).is_ok());

        // 只有一格的一行是一整句（同传翻好的一句）：最多 40 个字，不按一格 12 个字截
        let sentence = |text: &str| {
            Push {
                trigger: "augment-pick".into(),
                key: "pick:2".into(),
                line: Some(PushLine {
                    cells: vec![cell(text, None), cell(" ", None)],
                }),
                ..Push::default()
            }
            .shaped(pick)
            .line
            .unwrap()
            .cells
        };
        let long = "小心，他们三个人从左边的房子后面绕过来了";
        assert_eq!(sentence(long)[0].text, long, "一句 20 个字照样显示全");
        let cells = sentence(&"长".repeat(60));
        assert_eq!(cells.len(), 1);
        assert_eq!(cells[0].text.chars().count(), PushCell::ONE_CELL_MAX_CHARS);
        assert!(cells[0].text.ends_with('…'));

        // 只推了没声明的样子：什么都不剩
        let opening = m.push_spec("opening").unwrap();
        let only_line = Push {
            trigger: "opening".into(),
            key: "opening:1".into(),
            line: Some(PushLine {
                cells: vec![cell("x", None)],
            }),
            ..Push::default()
        }
        .shaped(opening);
        assert!(only_line.check(&m).is_err());
        // 卡片的页面要是自己清单里的
        let card = |uri: &str| {
            Push {
                trigger: "opening".into(),
                key: "opening:1".into(),
                card: Some(PushCard {
                    title: "开局战绩".into(),
                    uri: uri.into(),
                    ..PushCard::default()
                }),
                ..Push::default()
            }
            .shaped(opening)
        };
        assert!(card("ui://kotone/demo/match").check(&m).is_ok());
        assert!(card("ui://kotone/music/player").check(&m).is_err());
        let mut nameless = card("ui://kotone/demo/match");
        nameless.card.as_mut().unwrap().title = " ".into();
        assert!(nameless.shaped(opening).check(&m).is_err());
        let mut keyless = card("ui://kotone/demo/match");
        keyless.key = " ".into();
        assert!(keyless.shaped(opening).check(&m).is_err());
    }

    #[test]
    fn pushes_go_to_the_ui_in_camel_case_without_empty_parts() {
        let push = Push {
            trigger: "opening".into(),
            key: "opening:1".into(),
            line: Some(PushLine {
                cells: vec![PushCell {
                    text: "首选".into(),
                    tone: Some(PushTone::Good),
                }],
            }),
            ..Push::default()
        };
        let v = serde_json::to_value(&push).unwrap();
        assert_eq!(v["line"]["cells"][0]["tone"], "good");
        assert!(v.get("card").is_none() && v.get("voice").is_none() && v.get("alert").is_none());
        assert_eq!(
            serde_json::to_value(PushOutcome::Unchanged).unwrap(),
            "unchanged"
        );
        let back: Push = serde_json::from_value(v).unwrap();
        assert_eq!(back, push);
    }
}
