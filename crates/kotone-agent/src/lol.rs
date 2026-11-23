//! 英雄联盟工具（ADR 035）：这局的样子、谁交了技能、这局玩家的战绩和突破口。
//! 数据来自本机的客户端接口和局内接口（`kotone-lol`），由桌面端执行；这里只有名字、说明和参数。

use serde_json::json;

use crate::catalog::{Source, ToolEntry};

pub const TOOL_MATCH: &str = "lol.match";
pub const TOOL_SPELL: &str = "lol.spell";
pub const TOOL_SCOUT: &str = "lol.scout";

/// 三个工具都交给桌面端执行。
pub const TOOLS: &[&str] = &[TOOL_MATCH, TOOL_SPELL, TOOL_SCOUT];

pub fn entries() -> Vec<ToolEntry> {
    vec![
        ToolEntry {
            name: TOOL_MATCH.into(),
            description: "英雄联盟这局现在的样子（马上返回）：模式、游戏时间、比分；10 个人的英雄、玩家名、召唤师技能、\
等级、KDA、补刀、装备、是否阵亡；对面谁的闪现没好（玩家报过的，几点好）、谁没带闪；最近的龙、塔、团灭。\
“谁没闪”“对面谁没带闪”“对面 AD 出了什么”“现在几比几”都用它。不在对局里时会说。"
                .into(),
            parameters: json!({"type": "object", "additionalProperties": false, "properties": {}}),
            side_effects: vec![],
            group: "game",
            source: Source::Builtin,
            // 玩家名是别人起的
            untrusted: true,
        },
        ToolEntry {
            name: TOOL_SPELL.into(),
            description: "记下对面（或任何人）交了召唤师技能（英雄联盟的接口里没有冷却，只能玩家说了才记）：\
玩家说“阿狸闪了”“亚索刚交了点燃”就调用，champion 写英雄（称号、名字、英文名都行），spell 默认闪现；\
玩家说“十秒前”就填 ago。按现在的游戏时间、模式和那个人的装备算好几点好，返回一句可以发到聊天框的话（chat，例如“亚索 闪 14:32好”）：\
玩家说了“发出去 / 发聊天框”才用 game_chat.send 发。记错了用 clear: true 清掉（不写 champion 清全部）。"
                .into(),
            parameters: json!({"type": "object", "additionalProperties": false, "properties": {
                "champion": {"type": "string", "maxLength": 30, "description": "英雄：称号、名字或英文名（“阿狸”“九尾妖狐”“Ahri”）"},
                "spell": {"type": "string", "maxLength": 10, "description": "召唤师技能，默认“闪现”（也可以是“点燃 / 引燃”“传送”“治疗”“屏障”“虚弱”“净化”“幽灵疾步”“惩戒”“标记”）"},
                "ago": {"type": "integer", "minimum": 0, "maximum": 300, "description": "几秒前交的，默认 0（刚交）"},
                "clear": {"type": "boolean", "description": "true：清掉记错的"}
            }}),
            side_effects: vec![],
            group: "game",
            source: Source::Builtin,
            untrusted: true,
        },
        ToolEntry {
            name: TOOL_SCOUT.into(),
            description: "查英雄联盟这局 10 个人最近的战绩（每人最近 20 局、全部模式，和这局同一模式的单独算）：\
段位、胜负、连胜连败、KDA、场均死、分均伤害、这个英雄玩过几局（大乱斗、海斗这类随机英雄的模式不看）、谁和谁开黑；隐藏战绩的会标出来。\
最后一行是按数据算出的“突破口”和原因。进加载界面 Kotone 就开始查，问的时候多半已经查好。\
只问一个人时 player 写英雄名或玩家名。转述时说成信息（“对面辅助最近五连败、劫第一次玩”），隐藏战绩的要提一句。"
                .into(),
            parameters: json!({"type": "object", "additionalProperties": false, "properties": {
                "player": {"type": "string", "maxLength": 40, "description": "只看这一个人：英雄名或玩家名；不写 = 全部 10 个人"}
            }}),
            side_effects: vec![],
            group: "game",
            source: Source::Builtin,
            untrusted: true,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn three_lol_tools_with_object_schemas() {
        let entries = entries();
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, TOOLS);
        for e in &entries {
            assert_eq!(e.parameters["type"], "object");
            assert!(e.untrusted, "{} 里有别人起的名字", e.name);
            assert_eq!(crate::catalog::group_of(&e.name), "game");
        }
    }
}
