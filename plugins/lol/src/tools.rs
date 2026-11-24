//! 英雄联盟插件的工具（ADR 035 D2）：名字、说明、参数。实现在 [`crate::watcher`]。
//! 给她的工具都马上返回（主会话也有，ADR 029），结果里有别人起的名字，按不可信数据交给她。
//! 带页面的（照 MCP Apps）：`lol.scout` → 这局，`lol.player` → 战绩，`lol.game`（只给页面）→ 单局详情。

use kotone_plugin_api::ToolSpec;
use serde_json::json;

use crate::pages::{PAGE_GAME, PAGE_MATCH, PAGE_PLAYER};

pub const TOOL_MATCH: &str = "lol.match";
pub const TOOL_SPELL: &str = "lol.spell";
pub const TOOL_SCOUT: &str = "lol.scout";
pub const TOOL_PLAYER: &str = "lol.player";
/// 只给页面用：一局的详情。
pub const TOOL_GAME: &str = "lol.game";
/// 网上的统计（数据开关 `stats`）：一个英雄、强度榜。
pub const TOOL_CHAMPION: &str = "lol.champion";
pub const TOOL_TIER: &str = "lol.tier";

pub fn specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec::new(
            TOOL_MATCH,
            "英雄联盟这局现在的样子（马上返回）：模式、游戏时间、比分；10 个人的英雄、玩家名、召唤师技能、\
等级、KDA、补刀、装备、是否阵亡；对面谁的闪现没好（玩家报过的，几点好）、谁没带闪；最近的龙、塔、团灭。\
“谁没闪”“对面谁没带闪”“对面 AD 出了什么”“现在几比几”都用它。不在对局里时会说。",
            json!({"type": "object", "additionalProperties": false, "properties": {}}),
        )
        .needs(&["client"])
        .instant()
        // 玩家名是别人起的
        .untrusted(),
        ToolSpec::new(
            TOOL_SPELL,
            "记下对面（或任何人）交了召唤师技能（英雄联盟的接口里没有冷却，只能玩家说了才记）：\
玩家说“阿狸闪了”“亚索刚交了点燃”就调用，champion 写英雄（称号、名字、英文名都行），spell 默认闪现；\
玩家说“十秒前”就填 ago。按现在的游戏时间、模式和那个人的装备算好几点好，返回一句可以发到聊天框的话（chat，例如“亚索 闪 14:32好”）：\
玩家说了“发出去 / 发聊天框”才用 game_chat.send 发。记错了用 clear: true 清掉（不写 champion 清全部）。",
            json!({"type": "object", "additionalProperties": false, "properties": {
                "champion": {"type": "string", "maxLength": 30, "description": "英雄：称号、名字或英文名（“阿狸”“九尾妖狐”“Ahri”）"},
                "spell": {"type": "string", "maxLength": 10, "description": "召唤师技能，默认“闪现”（也可以是“点燃 / 引燃”“传送”“治疗”“屏障”“虚弱”“净化”“幽灵疾步”“惩戒”“标记”）"},
                "ago": {"type": "integer", "minimum": 0, "maximum": 300, "description": "几秒前交的，默认 0（刚交）"},
                "clear": {"type": "boolean", "description": "true：清掉记错的"}
            }}),
        )
        .needs(&["client"])
        .instant()
        .untrusted(),
        ToolSpec::new(
            TOOL_SCOUT,
            "查英雄联盟这局 10 个人最近的战绩（每人最近 20 局、全部模式，和这局同一模式的单独算）：\
段位、胜负、连胜连败、KDA、场均死、分均伤害、这个英雄玩过几局（大乱斗、海斗这类随机英雄的模式不看）、谁和谁开黑；隐藏战绩的会标出来。\
最后一行是按数据算出的“突破口”和原因。进加载界面 Kotone 就开始查，问的时候多半已经查好。\
只问一个人时 player 写英雄名或玩家名。转述时说成信息（“对面辅助最近五连败、劫第一次玩”），隐藏战绩的要提一句。",
            json!({"type": "object", "additionalProperties": false, "properties": {
                "player": {"type": "string", "maxLength": 40, "description": "只看这一个人：英雄名或玩家名；不写 = 全部 10 个人"}
            }}),
        )
        .page(PAGE_MATCH)
        .needs(&["client"])
        .instant()
        .untrusted(),
        ToolSpec::new(
            TOOL_PLAYER,
            "查一个英雄联盟玩家最近的战绩（最近 20 局、全部模式）：段位、胜负、KDA、连胜连败、最近 5 局用什么英雄打成什么样；隐藏战绩的会标出来。玩家说“我最近怎么样”“查一下我的战绩”就不写参数（查自己）；“对面亚索最近怎么样”写这局里的英雄名或玩家名；查不在这局的人要写完整的“名字#编号”。聊天里会出现他的战绩卡片，点开能看每一局，你只说一两句要紧的。",
            json!({"type": "object", "additionalProperties": false, "properties": {
                "player": {"type": "string", "maxLength": 60, "description": "这局里的英雄名或玩家名，或者“名字#编号”；不写 = 自己"},
                "puuid": {"type": "string", "maxLength": 100, "description": "页面里点进某个人时用"}
            }}),
        )
        .page(PAGE_PLAYER)
        .needs(&["client"])
        .instant()
        .untrusted(),
        ToolSpec::new(
            TOOL_GAME,
            "一局的详情：两边 10 个人的英雄、KDA、伤害、经济、补刀、装备、海克斯（战绩页面里点开某一局时用）。",
            json!({"type": "object", "additionalProperties": false, "required": ["gameId"], "properties": {
                "gameId": {"type": "integer", "minimum": 1}
            }}),
        )
                .app_only()
        .page(PAGE_GAME)
        .needs(&["client"])
        .untrusted(),
        ToolSpec::new(
            TOOL_CHAMPION,
            "查一个英雄联盟英雄的统计（国服，腾讯 101 / 掌盟，每天更新；海斗还有 aramhex-data 的英雄 × 海克斯胜率）：\
峡谷是这一路的强度（T0–T5）、胜率、选用率、禁用率，克制关系（难打 / 好打的英雄和胜率），核心出装、鞋、符文、加点、召唤师技能；\
海斗是英雄榜名次、胜率、最搭的队友、最好的海克斯（比这个英雄平均胜率高几个百分点）。champion 写名字、称号或外号（“刀妹”“亚索”）。\
mode：rift 峡谷 / mayhem 海斗，不写时这局是海斗就查海斗，否则查峡谷；lane 写分路（上 / 野 / 中 / 下 / 辅），不写用这个英雄最常走的路。\
“亚索克制谁”“刀妹出什么”“海斗里狗头拿什么海克斯”都用它。聊天里会出现英雄卡片，你只说一两句要紧的，说出处（“掌盟的数据”）。",
            json!({"type": "object", "additionalProperties": false, "required": ["champion"], "properties": {
                "champion": {"type": "string", "minLength": 1, "maxLength": 30, "description": "英雄的名字、称号或外号"},
                "mode": {"enum": ["rift", "mayhem"], "description": "rift 召唤师峡谷 / mayhem 海克斯大乱斗"},
                "lane": {"type": "string", "maxLength": 10, "description": "分路：上 / 野 / 中 / 下 / 辅（峡谷）"}
            }}),
        )
        .page(crate::stats_tools::PAGE_CHAMPION)
        .needs(&["stats"])
        .instant()
        .untrusted(),
        ToolSpec::new(
            TOOL_TIER,
            "英雄联盟的强度榜（国服，腾讯 101 / 掌盟）：峡谷每一路的 T0–T5、胜率、选用率、禁用率；海斗的英雄榜。\
“现在上单什么强”“海斗什么英雄强”用它。mode 不写时这局是海斗就看海斗，否则看峡谷；lane 不写时每一路列前几名。",
            json!({"type": "object", "additionalProperties": false, "properties": {
                "mode": {"enum": ["rift", "mayhem"]},
                "lane": {"type": "string", "maxLength": 10, "description": "分路：上 / 野 / 中 / 下 / 辅"}
            }}),
        )
        .page(crate::stats_tools::PAGE_TIER)
        .needs(&["stats"])
        .instant()
        .untrusted(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tools_for_her_and_the_pages() {
        let specs = specs();
        let names: Vec<&str> = specs.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            names,
            [
                TOOL_MATCH,
                TOOL_SPELL,
                TOOL_SCOUT,
                TOOL_PLAYER,
                TOOL_GAME,
                TOOL_CHAMPION,
                TOOL_TIER
            ]
        );
        for s in &specs {
            assert_eq!(s.parameters["type"], "object");
            assert!(s.untrusted, "{} 里有别人写的字", s.name);
            assert!(s.for_app());
            let source = if matches!(s.name.as_str(), TOOL_CHAMPION | TOOL_TIER) {
                "stats"
            } else {
                "client"
            };
            assert_eq!(s.needs, [source], "{}", s.name);
        }
        // 给她的都马上返回；单局详情只给页面
        for s in specs.iter().filter(|s| s.name != TOOL_GAME) {
            assert!(s.instant && s.for_model(), "{}", s.name);
        }
        let game = specs.iter().find(|s| s.name == TOOL_GAME).unwrap();
        assert!(!game.for_model());
        assert_eq!(specs[2].page.as_deref(), Some(PAGE_MATCH));
    }
}
