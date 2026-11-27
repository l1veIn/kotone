//! 英雄联盟插件的工具（ADR 035 D2）：名字、说明、参数。实现在 [`crate::watcher`]。
//! 给她的工具都马上返回（主会话也有，ADR 029）；结果里有别人起的名字的（战绩、技能记录、网上的统计……）按不可信数据
//! 交给她。`lol.match` 只写英雄、不写玩家名，是可信的，还带一份给自动化程序的结构化数据（ADR 046 D8）。
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
/// 只给页面用：按客户端历史偏移翻页。
pub const TOOL_HISTORY: &str = "lol.history";
/// 网上的统计（数据开关 `stats`）：一个英雄、强度榜。
pub const TOOL_CHAMPION: &str = "lol.champion";
pub const TOOL_TIER: &str = "lol.tier";
/// 操作客户端（数据开关 `control`，默认关）。
pub const TOOL_CLIENT: &str = "lol.client";
/// 只给页面用：全部英雄的名字（设置里排自动选 / 禁的顺序）。
pub const TOOL_CHAMPIONS: &str = "lol.champions";
/// 一键喊话（ADR 041 第 3 期）：现生成几行、经本体的发话入口发到游戏聊天。给她和热键（插件命令），不给页面。
pub const TOOL_SHOUT: &str = "lol.shout";
/// 只给页面用：记下玩家给一块数据选的来源（点来源标签换的，ADR 043 D8），存在插件设置里。
pub const TOOL_SOURCE: &str = "lol.source";
/// 只给页面用：海克斯三选一要的本地文字识别模型，卡片上的“下载 / 不用”（ADR 043 D7）。
pub const TOOL_AUGMENT_OCR: &str = "lol.augment_ocr";
/// 只给页面用：选人卡片上的“一键导入”（符文页、出装方案、召唤师技能；要“操作客户端”开着，ADR 043 B 轮）。
pub const TOOL_IMPORT: &str = "lol.import";

pub fn specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec::new(
            TOOL_MATCH,
            "英雄联盟这局现在的样子（马上返回）：模式、游戏时间、比分；10 个人的英雄、召唤师技能、\
等级、KDA、补刀、装备、是否阵亡；对面谁的闪现没好（玩家报过的，几点好）、谁没带闪；最近的龙、塔、团灭。\
不带玩家名（要名字和战绩用 lol.scout）。“谁没闪”“对面谁没带闪”“对面 AD 出了什么”“现在几比几”都用它。不在对局里时会说。\
自动化的程序里 tool(\"lol.match\") 拿到的是结构化数据：status（ok / not_in_game / error）、reason（不是 ok 时：\
loading / champ_select / not_in_game / client_closed，或读不到的原因）、mode（CLASSIC / ARAM / KIWI……）、gameTime（秒）、\
score {ally, enemy}（击杀数）、alive {ally, enemy}（活着几个，我方含自己）、players[]（我方在前）：champion、\
alias（英文 id）、team（ally / enemy）、me、bot、level、kills、deaths、assists、cs、dead、respawnSeconds、\
position（峡谷的 TOP / JUNGLE / MIDDLE / BOTTOM / UTILITY，别的模式空）、spells[{key（SummonerFlash 这类）, name, \
readyAt（玩家报过、还没好的才有：游戏时间到几秒时好）}]、items[{id, name}]。没有别人的金币、技能冷却和海克斯。",
            json!({"type": "object", "additionalProperties": false, "properties": {}}),
        )
        .needs(&["client"])
        .instant(),
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
段位、胜负、连胜连败、KDA、场均死、分均伤害、这个英雄玩过几局（大乱斗、海斗这类随机英雄的模式不看）、谁和谁开黑；隐藏战绩的会标出来，\
隐藏了身份的人、电脑只有英雄（查不到，也不去查）。只给事实，不评价人：玩家问谁是突破口时，由你按这些数据说。\
进加载界面 Kotone 就开始查，问的时候多半已经查好。\
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
                "puuid": {"type": "string", "maxLength": 100, "description": "页面里点进某个人时用"},
                "includeProfileBackground": {"type": "boolean", "description": "召唤师主页读取自己的生涯原画时用"}
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
            TOOL_HISTORY,
            "主页、战绩页分页看一个人的战绩（客户端给的最近 50 局里切页，重开、自定义不算）。offset 用上一页返回的 nextOffset，首页为 0；\
clientGames 是客户端一共给了几局记录（翻到头时页面写清）。",
            json!({"type": "object", "additionalProperties": false, "required": ["puuid", "offset", "pageSize"], "properties": {
                "puuid": {"type": "string", "minLength": 1, "maxLength": 100},
                "offset": {"type": "integer", "minimum": 0, "maximum": u32::MAX},
                "pageSize": {"type": "integer", "enum": [10, 20]}
            }}),
        )
        .app_only()
        .needs(&["client"])
        .untrusted(),
        ToolSpec::new(
            TOOL_CHAMPION,
            "查一个英雄联盟英雄的统计（国服，腾讯 101 / 掌盟，每天更新；海斗还有 aramhex-data 的英雄 × 海克斯胜率）：\
峡谷是这一路的强度（T0–T5）、胜率、选用率、禁用率，克制关系（难打 / 好打的英雄和胜率），核心出装、鞋、符文、加点、召唤师技能；\
海斗是英雄榜名次、胜率、最搭的队友、最好的海克斯（比这个英雄平均胜率高几个百分点）。champion 写名字、称号或外号（“刀妹”“亚索”）。\
mode：rift 峡谷 / mayhem 海斗，不写时这局是海斗就查海斗，否则查峡谷；lane 写分路（上 / 野 / 中 / 下 / 辅），不写用这个英雄最常走的路。\
source 换数据源：峡谷 qq101（腾讯 101，国服）/ opgg（OP.GG，全球）；海斗的海克斯 aramhex（aramhex-data）/ opgg。不写按玩家在页面上选的\
（默认国服用腾讯 101、海斗的海克斯用 aramhex-data）。一次只用一个数据源，不同来源的数字不放在一起比。\
“亚索克制谁”“刀妹出什么”“海斗里狗头拿什么海克斯”都用它。聊天里会出现英雄卡片，你只说一两句要紧的，说出处（“掌盟的数据”“OP.GG 的全球数据”）。",
            json!({"type": "object", "additionalProperties": false, "required": ["champion"], "properties": {
                "champion": {"type": "string", "minLength": 1, "maxLength": 30, "description": "英雄的名字、称号或外号"},
                "mode": {"enum": ["rift", "mayhem"], "description": "rift 召唤师峡谷 / mayhem 海克斯大乱斗"},
                "lane": {"type": "string", "maxLength": 10, "description": "分路：上 / 野 / 中 / 下 / 辅（峡谷）"},
                "source": {"enum": ["qq101", "opgg", "aramhex"], "description": "数据源：峡谷 qq101 / opgg；海斗的海克斯 aramhex / opgg；不写 = 玩家选的"}
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
        ToolSpec::new(
            TOOL_CLIENT,
            "操作英雄联盟客户端（照 LeagueAkari，只走客户端的本机接口）：accept 接受对局、decline 拒绝；选英雄阶段 pick 选并锁定、\
ban 禁、hover 只亮出来（还没轮到也行）。champion 写名字、称号或外号。**只在玩家明确说了才用**（“帮我接受”“帮我选亚索”“禁掉劫”），\
不要自己决定选谁；没轮到、不在这个阶段时会说。有封号风险，玩家要先在插件里打开“操作客户端”。",
            json!({"type": "object", "additionalProperties": false, "required": ["action"], "properties": {
                "action": {"enum": ["accept", "decline", "pick", "ban", "hover"]},
                "champion": {"type": "string", "maxLength": 30, "description": "pick / ban / hover 时：英雄的名字、称号或外号"}
            }}),
        )
        .needs(&["client", "control"])
        .side_effects(&["controls-game-client"])
        .instant(),
        shout_spec(),
        ToolSpec::new(
            TOOL_CHAMPIONS,
            "全部英雄的名字和 id（插件设置里排自动选 / 禁英雄的顺序用）。",
            json!({"type": "object", "additionalProperties": false, "properties": {}}),
        )
        .app_only()
        .page(crate::pages::PAGE_SETTINGS),
        ToolSpec::new(
            TOOL_SOURCE,
            "记下玩家给一块数据选的来源（页面上点来源标签换的）：kind 是哪一块（rift 峡谷的英雄数据、mayhemAugments 海斗的海克斯），\
source 是来源。存在插件设置里，下次打开页面、她查的时候照这个用。",
            json!({"type": "object", "additionalProperties": false, "required": ["kind", "source"], "properties": {
                "kind": {"enum": ["rift", "mayhemAugments"]},
                "source": {"enum": ["qq101", "opgg", "aramhex"]}
            }}),
        )
        .app_only(),
        ToolSpec::new(
            TOOL_AUGMENT_OCR,
            "海克斯三选一要用的本地文字识别模型（问要不要下载的那张卡片上的按钮）：action = download 下载推荐的本地模型\
（在后台下，不改全局的文字识别选择）、decline 不用（记进插件设置，以后不再问）、status 现在怎样。",
            json!({"type": "object", "additionalProperties": false, "properties": {
                "action": {"enum": ["status", "download", "decline"]}
            }}),
        )
        .needs(&["stats"])
        .app_only()
        .page(crate::pushes::PAGE_AUGMENT_OCR),
        ToolSpec::new(
            TOOL_IMPORT,
            "一键导入（选人卡片上的按钮）：把选人卡片上“我的英雄”那份——峡谷是腾讯 101 这一路的符文、出装、召唤师技能，海斗是 aramhex-data 的核心三件和召唤师技能（没有符文）——经客户端的本机接口写进去：符文页（改上次导入的那页，没有就新建，满了换掉当前页）、这个英雄的出装方案、召唤师技能（闪现照你原来放的那边）。只在选英雄时、“操作客户端”开着时；返回每一样写成了什么。",
            json!({"type": "object", "additionalProperties": false, "properties": {}}),
        )
        .needs(&["client", "control"])
        .side_effects(&["controls-game-client"])
        .app_only(),
    ]
}

/// `lol.shout`：四条热键预设（`plugin.toml` 的 `[[commands]]`）调的就是它。
fn shout_spec() -> ToolSpec {
    let mut spec = ToolSpec::new(
        TOOL_SHOUT,
        "一键喊话（马上返回）：按这局已有的数据现生成几行，经 Kotone 发到英雄联盟的聊天。what = flash：对面谁的闪现没好、\
几点好（只有玩家报过、lol.spell 记下的，接口里没有冷却）和谁没带闪；records：一边的战绩（段位、近 20 局胜负、KDA、连胜连败，\
一人一行）；premade：谁和谁开黑。side 说的是谁的情况（不是频道）：enemy 对面（默认）、ally 我方、all 两边（开黑默认 all）。\
只写英雄、不发玩家名。玩家说了“把对面闪现 / 战绩 / 开黑发出去”才用（一次调用，不用先查再发）；只看会发什么用 preview: true。\
没东西可喊（没记过谁交闪、战绩还没查完）时不发，返回原因。选英雄、房间里经客户端的聊天接口发（只有队伍频道），对局里按键发、\
前台要是游戏（和 game_chat.send 同一个入口）。",
        json!({"type": "object", "additionalProperties": false, "required": ["what"], "properties": {
            "what": {"enum": ["flash", "records", "premade"], "description": "flash 对面闪现、records 战绩、premade 开黑"},
            "side": {"enum": ["enemy", "ally", "all"], "description": "说谁的情况：enemy 对面（默认）、ally 我方、all 两边（开黑默认 all）"},
            "channel": {"enum": ["team", "all"], "description": "发到哪个频道，默认队伍"},
            "preview": {"type": "boolean", "description": "true：只返回会发的几行，不发"}
        }}),
    )
    .needs(&["client"])
    .side_effects(&["input"])
    .instant();
    // 页面用不着：发话只在她、外部 Agent、热键调的时候（宿主只在这几种调用里让插件经入口发）
    spec.visibility = vec![kotone_plugin_api::Visibility::Model];
    spec
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
                TOOL_HISTORY,
                TOOL_CHAMPION,
                TOOL_TIER,
                TOOL_CLIENT,
                TOOL_SHOUT,
                TOOL_CHAMPIONS,
                TOOL_SOURCE,
                TOOL_AUGMENT_OCR,
                TOOL_IMPORT
            ]
        );
        for s in &specs {
            assert_eq!(s.parameters["type"], "object");
            assert_eq!(s.for_app(), s.name != TOOL_SHOUT, "{}", s.name);
            let needs: &[&str] = match s.name.as_str() {
                TOOL_CHAMPION | TOOL_TIER | TOOL_AUGMENT_OCR => &["stats"],
                TOOL_CLIENT | TOOL_IMPORT => &["client", "control"],
                TOOL_CHAMPIONS | TOOL_SOURCE => &[],
                _ => &["client"],
            };
            assert_eq!(s.needs, needs, "{}", s.name);
            // 读到别人写的字的都按不可信数据；lol.match 不写玩家名（ADR 046 D8）
            let trusted = matches!(
                s.name.as_str(),
                TOOL_MATCH
                    | TOOL_CLIENT
                    | TOOL_CHAMPIONS
                    | TOOL_SHOUT
                    | TOOL_SOURCE
                    | TOOL_AUGMENT_OCR
                    | TOOL_IMPORT
            );
            assert_eq!(s.untrusted, !trusted, "{}", s.name);
        }
        let client = specs.iter().find(|s| s.name == TOOL_CLIENT).unwrap();
        assert_eq!(
            client.side_effects,
            ["controls-game-client"],
            "默认先问玩家"
        );
        // 给她的都马上返回；单局详情和英雄列表只给页面
        for s in specs.iter().filter(|s| {
            !matches!(
                s.name.as_str(),
                TOOL_GAME
                    | TOOL_HISTORY
                    | TOOL_CHAMPIONS
                    | TOOL_SOURCE
                    | TOOL_AUGMENT_OCR
                    | TOOL_IMPORT
            )
        }) {
            assert!(s.instant && s.for_model(), "{}", s.name);
        }
        // 一键喊话：和 game_chat.send 一样的副作用（默认允许，绑热键时不问）
        let shout = specs.iter().find(|s| s.name == TOOL_SHOUT).unwrap();
        assert_eq!(shout.side_effects, ["input"]);
        let game = specs.iter().find(|s| s.name == TOOL_GAME).unwrap();
        assert!(!game.for_model());
        let history = specs.iter().find(|s| s.name == TOOL_HISTORY).unwrap();
        assert!(history.for_app() && !history.for_model());
        // 换数据源只给页面（ADR 043 D8：点来源标签）
        let source = specs.iter().find(|s| s.name == TOOL_SOURCE).unwrap();
        assert!(!source.for_model() && source.for_app());
        // 下载识别模型的按钮只给页面（ADR 043 D7），页面是那张卡片
        let ocr = specs.iter().find(|s| s.name == TOOL_AUGMENT_OCR).unwrap();
        assert!(!ocr.for_model() && ocr.for_app());
        assert_eq!(ocr.page.as_deref(), Some(crate::pushes::PAGE_AUGMENT_OCR));
        // 一键导入只给页面（卡片上的按钮），要“操作客户端”，副作用和 lol.client 一样
        let import = specs.iter().find(|s| s.name == TOOL_IMPORT).unwrap();
        assert!(!import.for_model() && import.for_app());
        assert_eq!(import.side_effects, ["controls-game-client"]);
        assert_eq!(specs[2].page.as_deref(), Some(PAGE_MATCH));
    }
}
