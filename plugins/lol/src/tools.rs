//! 英雄联盟插件的工具（ADR 035 D2）：名字、说明、参数。实现在 [`crate::watcher`]。
//! 三个工具都马上返回（主会话也有，ADR 029），结果里有别人起的名字，按不可信数据交给她。

use kotone_plugin_api::ToolSpec;
use serde_json::json;

pub const TOOL_MATCH: &str = "lol.match";
pub const TOOL_SPELL: &str = "lol.spell";
pub const TOOL_SCOUT: &str = "lol.scout";

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
        .needs(&["client"])
        .instant()
        .untrusted(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn three_tools_for_her_and_the_page() {
        let specs = specs();
        let names: Vec<&str> = specs.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, [TOOL_MATCH, TOOL_SPELL, TOOL_SCOUT]);
        for s in &specs {
            assert_eq!(s.parameters["type"], "object");
            assert!(s.untrusted, "{} 里有别人起的名字", s.name);
            assert!(s.instant && s.for_model() && s.for_app());
            assert_eq!(s.needs, ["client"]);
        }
    }
}
