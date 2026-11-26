//! 把这局、战绩、技能记录写成给她看的几行字（工具结果）。玩家名是别人起的，工具结果整体按不可信数据交给模型。

use std::collections::BTreeMap;

use crate::client::clock;
use crate::client::gamedata::GameData;
use crate::client::history::PlayerHistory;
use crate::client::live::{LiveGame, LivePlayer};
use crate::client::scout::{self, Summary};
use crate::client::spells::SpellTracker;

/// 模式的中文名（局内接口的 `gameMode`）。
pub fn mode_name(mode: &str) -> &str {
    match mode.to_ascii_uppercase().as_str() {
        "CLASSIC" => "召唤师峡谷",
        "SWIFTPLAY" => "快速模式",
        "ARAM" => "极地大乱斗",
        "KIWI" => "海克斯大乱斗",
        "URF" | "ARURF" => "无限火力",
        "CHERRY" => "斗魂竞技场",
        "PRACTICETOOL" => "训练模式",
        "TUTORIAL" => "新手教程",
        _ => mode,
    }
}

/// 局内的一个人：英雄全名（“疾风剑豪 亚索”）和短名（“亚索”）。
pub fn champion_names(player: &LivePlayer, data: Option<&GameData>) -> (String, String) {
    match data.and_then(|d| d.champion_by_alias(&player.alias)) {
        Some(c) => (c.label(), c.short().to_string()),
        None => (player.champion.clone(), player.champion.clone()),
    }
}

/// 龙的中文名（局内接口的 `DragonType`）。
pub(crate) fn dragon(kind: &str) -> &str {
    match kind {
        "Fire" => "火龙",
        "Water" => "水龙",
        "Earth" => "土龙",
        "Air" => "风龙",
        "Hextech" => "海克斯龙",
        "Chemtech" => "炼金龙",
        "Elder" => "远古龙",
        _ => "小龙",
    }
}

fn side(game: &LiveGame, name: &str) -> &'static str {
    match game.is_ally_name(name) {
        Some(true) => "我方",
        Some(false) => "对面",
        None => "",
    }
}

/// 最近的几件大事（龙、男爵、先锋、塔、水晶、团灭、多杀），最多 `limit` 条，旧的在前。
fn recent_events(game: &LiveGame, limit: usize) -> Vec<String> {
    let mut out: Vec<String> = game
        .events
        .iter()
        .filter_map(|e| {
            let who = side(game, &e.killer);
            let what = match e.name.as_str() {
                "DragonKill" => {
                    let mut kind = e.detail.split(' ');
                    let dragon = dragon(kind.next().unwrap_or(""));
                    let stolen = if e.detail.contains("抢的") {
                        "（抢的）"
                    } else {
                        ""
                    };
                    format!("{who}拿了{dragon}{stolen}")
                }
                "BaronKill" => format!("{who}拿了大龙"),
                "HeraldKill" => format!("{who}拿了峡谷先锋"),
                "TurretKilled" => format!("{who}推掉一座塔"),
                "InhibKilled" => format!("{who}推掉一个水晶"),
                "Ace" => {
                    let acing = if e.detail.contains(game.my_team()) && !game.my_team().is_empty() {
                        "我方"
                    } else {
                        "对面"
                    };
                    format!("{acing}团灭了另一边")
                }
                "Multikill" => {
                    let n: i64 = e.detail.trim().parse().unwrap_or(0);
                    if n < 3 {
                        return None;
                    }
                    format!("{who}{} {n} 杀", e.killer)
                }
                "FirstBlood" => "一血".to_string(),
                _ => return None,
            };
            Some(format!("{} {what}", clock(e.time)))
        })
        .collect();
    let skip = out.len().saturating_sub(limit);
    out.drain(..skip);
    out
}

fn player_line(
    p: &LivePlayer,
    data: Option<&GameData>,
    tracker: &SpellTracker,
    now: f64,
    is_me: bool,
) -> String {
    let (full, _) = champion_names(p, data);
    let mut parts = vec![if is_me {
        format!("{full}（你）")
    } else {
        full
    }];
    if !p.name.is_empty() {
        parts.push(format!("「{}」", p.name));
    }
    parts.push(format!(
        "{} 级 {}/{}/{} 补刀 {}",
        p.level, p.kills, p.deaths, p.assists, p.cs
    ));
    let spells: Vec<String> = p
        .spells
        .iter()
        .map(|s| match tracker.ready_at(&p.alias, s, now) {
            Some(ready) => format!("{s}（{} 好）", clock(ready)),
            None => s.clone(),
        })
        .collect();
    if !spells.is_empty() {
        parts.push(spells.join("、"));
    }
    let items: Vec<&str> = p
        .items
        .iter()
        .map(|i| i.name.as_str())
        .filter(|n| !n.is_empty())
        .collect();
    if !items.is_empty() {
        parts.push(format!("装备：{}", items.join("、")));
    }
    if p.dead {
        parts.push(format!("阵亡，{:.0} 秒后复活", p.respawn.max(0.0)));
    }
    if p.bot {
        parts.push("电脑".into());
    } else if p.hidden_identity() {
        parts.push("隐藏了身份".into());
    }
    parts.join(" · ")
}

/// 对面的闪现：谁没好（几点好）、谁没带。
pub fn enemy_flash_line(
    game: &LiveGame,
    data: Option<&GameData>,
    tracker: &SpellTracker,
) -> String {
    let now = game.time;
    let mut down = Vec::new();
    let mut without = Vec::new();
    for p in game.enemies() {
        let (_, short) = champion_names(p, data);
        // 认闪现只用 `carries_flash`（不看显示的名字；认不出就不说没带）
        if p.carries_flash() == Some(false) {
            without.push(short);
        } else if let Some(ready) = tracker.ready_at(&p.alias, "闪现", now) {
            down.push(format!(
                "{short} {} 好（还有 {}）",
                clock(ready),
                clock(ready - now)
            ));
        }
    }
    let mut parts = Vec::new();
    parts.push(if down.is_empty() {
        "没记到谁交了闪".to_string()
    } else {
        format!("没闪：{}", down.join("、"))
    });
    if !without.is_empty() {
        parts.push(format!("没带闪：{}", without.join("、")));
    }
    format!("对面的闪现：{}", parts.join("；"))
}

/// `lol.match` 的结果。
pub fn render_match(game: &LiveGame, data: Option<&GameData>, tracker: &SpellTracker) -> String {
    let (ours, theirs) = game.score();
    let mut out = vec![
        format!(
            "{} · 游戏时间 {} · 比分 我方 {ours} : {theirs} 对面",
            mode_name(&game.mode),
            clock(game.time)
        ),
        enemy_flash_line(game, data, tracker),
    ];
    let others = tracker
        .down(game.time)
        .into_iter()
        .filter(|u| u.spell != "闪现")
        .map(|u| format!("{} {}（{} 好）", u.champion, u.spell, clock(u.ready_at)))
        .collect::<Vec<_>>();
    if !others.is_empty() {
        out.push(format!("记着的其他技能：{}", others.join("、")));
    }
    let me = game.me().map(|p| p.riot_id.clone()).unwrap_or_default();
    out.push("我方：".into());
    for p in game.allies() {
        out.push(format!(
            "- {}",
            player_line(p, data, tracker, game.time, p.riot_id == me)
        ));
    }
    out.push("对面：".into());
    for p in game.enemies() {
        out.push(format!(
            "- {}",
            player_line(p, data, tracker, game.time, false)
        ));
    }
    let events = recent_events(game, 5);
    if !events.is_empty() {
        out.push(format!("最近：{}", events.join("；")));
    }
    out.join("\n")
}

/// 这个人是谁看得到看不到（ADR 043 D9）。只有接口自己标了是电脑的才算电脑；没有 puuid 的真人照样列出来，
/// 只显示英雄，不去反查。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Identity {
    /// 有 puuid，查战绩（还没查完时 `history` 是 None）。
    #[default]
    Known,
    /// 隐藏了身份（主播模式）：加载、对局里客户端不给 puuid，局内接口里名字是空的。不查。
    Hidden,
    /// 选人时看不到名字（排位的队友、选人时的对面）：先不查，进了加载按 gameflow 的名单再看。
    NameHidden,
    /// 电脑（客户端的 `botDifficulty` / `playerType`、局内接口的 `isBot`）。不查。
    Bot,
}

impl Identity {
    /// 页面数据里的写法（`identity`）。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Known => "known",
            Self::Hidden => "hidden",
            Self::NameHidden => "name_hidden",
            Self::Bot => "bot",
        }
    }
}

/// 查战绩时的一个人。
#[derive(Debug, Clone, Default)]
pub struct ScoutPlayer {
    /// “疾风剑豪 亚索”。
    pub champion: String,
    /// 认人用的英文 id、短名。
    pub alias: String,
    pub short: String,
    pub champion_id: i64,
    /// 玩家名（`名字#编号` 或名字）。
    pub name: String,
    pub ally: bool,
    pub me: bool,
    /// 开黑分组（`teamParticipantId`）。
    pub party: Option<i64>,
    /// None = 还在查。
    pub history: Option<PlayerHistory>,
    /// 以下只给页面（给她的字里不用）：召唤师头像（`gameflow/session` 里的 `profileIconId`）、
    /// 召唤师技能（`playerChampionSelections` 的 `spell1Id`、`spell2Id`，不知道时 0）、
    /// 分路（`selectedPosition`：`TOP` / `JUNGLE` / `MIDDLE` / `BOTTOM` / `UTILITY`，没有时空）。
    pub profile_icon_id: Option<i64>,
    pub spells: [i64; 2],
    pub position: String,
    /// 看得到是谁没有（隐藏了身份、电脑的不查）。
    pub identity: Identity,
}

impl ScoutPlayer {
    /// 还在查（查得到的人、战绩还没回来）。隐藏了身份、电脑、选人时看不到名字的人不算。
    pub fn pending(&self) -> bool {
        self.identity == Identity::Known && self.history.is_none()
    }

    pub fn matches(&self, query: &str) -> bool {
        let q = crate::client::gamedata::normalize(query);
        !q.is_empty()
            && [&self.champion, &self.alias, &self.short, &self.name]
                .iter()
                .map(|s| crate::client::gamedata::normalize(s))
                .any(|s| !s.is_empty() && (s.contains(&q) || q.contains(&s)))
    }
}

fn scout_line(p: &ScoutPlayer, s: Option<&Summary>, mode: &str, random_pick: bool) -> String {
    let mut parts = vec![if p.me {
        format!("{}（你）", p.champion)
    } else {
        p.champion.clone()
    }];
    if !p.name.is_empty() {
        parts.push(format!("「{}」", p.name));
    }
    match p.identity {
        Identity::Known => {}
        Identity::Hidden => {
            parts.push("隐藏了身份（查不到战绩）".into());
            return parts.join(" · ");
        }
        Identity::NameHidden => {
            parts.push("选人时看不到名字（进了加载再查）".into());
            return parts.join(" · ");
        }
        Identity::Bot => {
            parts.push("电脑".into());
            return parts.join(" · ");
        }
    }
    let Some(h) = &p.history else {
        parts.push("还在查".into());
        return parts.join(" · ");
    };
    if let Some(rank) = &h.rank {
        parts.push(format!("单双排 {rank}"));
    }
    if h.hidden() {
        parts.push("隐藏战绩".into());
    }
    let Some(s) = s.filter(|s| s.games > 0) else {
        parts.push(match &h.error {
            Some(e) => format!("战绩查不到（{e}）"),
            None => "最近没有对局".into(),
        });
        return parts.join(" · ");
    };
    parts.push(format!(
        "近 {} 局 {} 胜 {} 负",
        s.games,
        s.wins,
        s.games - s.wins
    ));
    if s.mode_games > 0 {
        parts.push(format!(
            "{mode} {} 局 {} 胜，分均伤害 {:.0}",
            s.mode_games, s.mode_wins, s.mode_dpm
        ));
    }
    parts.push(format!("KDA {:.1}，场均死 {:.1}", s.kda, s.deaths));
    // 随机英雄的模式里“这个英雄 0 局”人人都是，不写
    if !random_pick || s.champ_games > 0 {
        parts.push(format!("这个英雄 {} 局 {} 胜", s.champ_games, s.champ_wins));
    }
    parts.extend(scout::tags(s, random_pick));
    parts.join(" · ")
}

/// 开黑的几组（同一边、同一个 `teamParticipantId`、至少两个人）。
pub(crate) fn parties(players: &[ScoutPlayer]) -> Vec<String> {
    let mut groups: BTreeMap<(bool, i64), Vec<&str>> = BTreeMap::new();
    for p in players {
        if let Some(id) = p.party.filter(|id| *id > 0) {
            groups.entry((!p.ally, id)).or_default().push(&p.short);
        }
    }
    groups
        .into_iter()
        .filter(|(_, names)| names.len() >= 2)
        .map(|((enemy, _), names)| {
            format!(
                "{} {}",
                if enemy { "对面" } else { "我方" },
                names.join(" + ")
            )
        })
        .collect()
}

/// `lol.scout` 的结果：每个人的事实（段位、最近的胜负、连胜连败、KDA、这个英雄打过几局）、谁和谁开黑、
/// 隐藏了身份的有几个。不评价人（ADR 043 D2：谁是突破口由她按这些数据说）。`only` = 只看这个人（英雄名或玩家名）；
/// `random_pick` = 随机英雄的模式（[`crate::client::gamedata::random_pick`]），不看这个英雄玩没玩过。
pub fn render_scout(
    players: &[ScoutPlayer],
    queue_id: i64,
    mode: &str,
    random_pick: bool,
    only: Option<&str>,
) -> String {
    let summaries: Vec<Option<Summary>> = players
        .iter()
        .map(|p| {
            p.history
                .as_ref()
                .map(|h| scout::summarize(&h.games, p.champion_id, queue_id))
        })
        .collect();
    let mut out = Vec::new();
    let wanted: Vec<usize> = match only.map(str::trim).filter(|q| !q.is_empty()) {
        Some(q) => (0..players.len())
            .filter(|i| players[*i].matches(q))
            .collect(),
        None => (0..players.len()).collect(),
    };
    if wanted.is_empty() {
        let names: Vec<&str> = players.iter().map(|p| p.short.as_str()).collect();
        return format!(
            "这局没有“{}”。这局的英雄：{}",
            only.unwrap_or(""),
            names.join("、")
        );
    }
    let random = if random_pick {
        "；英雄是随机的，不看玩没玩过这个英雄"
    } else {
        ""
    };
    out.push(format!(
        "每人最近 20 局（全部模式；“{mode}”是和这局同一个模式的{random}）："
    ));
    for (title, ally) in [("我方：", true), ("对面：", false)] {
        let lines: Vec<String> = wanted
            .iter()
            .filter(|i| players[**i].ally == ally)
            .map(|i| {
                format!(
                    "- {}",
                    scout_line(&players[*i], summaries[*i].as_ref(), mode, random_pick)
                )
            })
            .collect();
        if !lines.is_empty() {
            out.push(title.into());
            out.extend(lines);
        }
    }
    if only.is_some() {
        return out.join("\n");
    }
    let groups = parties(players);
    if !groups.is_empty() {
        out.push(format!("开黑：{}", groups.join("；")));
    }
    let hidden = |ally: bool| {
        players
            .iter()
            .filter(|p| p.ally == ally && p.identity == Identity::Hidden)
            .count()
    };
    let hidden: Vec<String> = [("我方", hidden(true)), ("对面", hidden(false))]
        .into_iter()
        .filter(|(_, n)| *n > 0)
        .map(|(side, n)| format!("{side} {n} 人"))
        .collect();
    if !hidden.is_empty() {
        out.push(format!(
            "隐藏了身份（客户端不给名字和战绩，不查）：{}",
            hidden.join("、")
        ));
    }
    let waiting = players.iter().filter(|p| p.pending()).count();
    if waiting > 0 {
        out.push(format!("还有 {waiting} 个人在查，过几秒再问会更全。"));
    }
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::history::{parse_match_list, tests as h};
    use crate::client::live::{parse_all_game_data, tests as l};
    use crate::client::spells::SpellUse;

    #[test]
    fn match_text_shows_score_flash_state_and_events() {
        let data = crate::client::gamedata::tests::sample();
        let game = parse_all_game_data(&l::sample_json()).unwrap();
        let mut tracker = SpellTracker::default();
        tracker.record(SpellUse {
            alias: "Yasuo".into(),
            champion: "亚索".into(),
            spell: "闪现".into(),
            used_at: 700.0,
            ready_at: 866.7,
            notes: vec![],
        });
        let text = render_match(&game, Some(&data), &tracker);
        assert!(
            text.starts_with("海克斯大乱斗 · 游戏时间 12:34 · 比分 我方 5 : 4 对面"),
            "{text}"
        );
        assert!(
            text.contains("对面的闪现：没闪：亚索 14:27 好（还有 1:53）；没带闪：阿狸"),
            "{text}"
        );
        assert!(
            text.contains("刀锋舞者 艾瑞莉娅（你） · 「玩家」"),
            "{text}"
        );
        assert!(
            text.contains("闪现（14:27 好）、引燃 · 装备：明朗之靴"),
            "{text}"
        );
        assert!(text.contains("最近：10:01 对面拿了火龙（抢的）"), "{text}");
    }

    fn scout_player(
        short: &str,
        ally: bool,
        party: Option<i64>,
        games: Vec<serde_json::Value>,
    ) -> ScoutPlayer {
        let list = h::list(games);
        ScoutPlayer {
            champion: format!("称号 {short}"),
            alias: short.into(),
            short: short.into(),
            champion_id: 39,
            name: format!("{short}的主人"),
            ally,
            me: false,
            party,
            history: Some(PlayerHistory {
                riot_id: String::new(),
                privacy: "PUBLIC".into(),
                games: parse_match_list(&list),
                rank: None,
                error: None,
                ..PlayerHistory::default()
            }),
            ..ScoutPlayer::default()
        }
    }

    /// ADR 043 D2：只给事实（最近的胜负、连胜连败、这个英雄几局、谁开黑），不说谁是突破口；
    /// D9：隐藏了身份的人照样列出来（只有英雄），电脑写“电脑”，都不算“还在查”。
    #[test]
    fn scout_text_states_facts_hidden_players_and_parties_only() {
        let good: Vec<_> = (0..10)
            .map(|i| h::game(i, 2400, 39, i % 3 != 0, (8, 3, 10), 30000))
            .collect();
        let bad: Vec<_> = (0..10)
            .map(|i| h::game(i, 2400, 157, i >= 5, (2, 9, 4), 12000))
            .collect();
        let mut players = vec![
            scout_player("甲", true, Some(1), good.clone()),
            scout_player("乙", false, Some(7), good.clone()),
            scout_player("丙", false, Some(7), good.clone()),
            scout_player("丁", false, Some(9), bad),
        ];
        players[3].history.as_mut().unwrap().privacy = "PRIVATE".into();
        let mut waiting = scout_player("戊", false, None, vec![]);
        waiting.history = None;
        players.push(waiting);
        for (short, identity) in [("己", Identity::Hidden), ("庚", Identity::Bot)] {
            let mut p = scout_player(short, true, None, vec![]);
            p.name = String::new();
            p.history = None;
            p.identity = identity;
            players.push(p);
        }
        let text = render_scout(&players, 2400, "海克斯大乱斗", true, None);
        assert!(text.contains("隐藏战绩"), "{text}");
        assert!(text.contains("开黑：对面 乙 + 丙"), "{text}");
        assert!(text.contains("5 连败"), "{text}");
        assert!(!text.contains("突破口"), "插件不推测突破口：{text}");
        assert!(text.contains("「戊的主人」 · 还在查"), "{text}");
        assert!(
            text.contains("- 称号 己 · 隐藏了身份（查不到战绩）"),
            "{text}"
        );
        assert!(text.contains("- 称号 庚 · 电脑"), "{text}");
        assert!(
            text.contains("隐藏了身份（客户端不给名字和战绩，不查）：我方 1 人"),
            "{text}"
        );
        assert!(text.contains("还有 1 个人在查"), "只数查得到的人：{text}");

        let one = render_scout(&players, 2400, "海克斯大乱斗", true, Some("丁"));
        assert!(one.contains("丁") && !one.contains("乙"), "{one}");
        assert!(
            render_scout(&players, 2400, "海克斯大乱斗", true, Some("不存在"))
                .starts_with("这局没有")
        );
    }

    #[test]
    fn random_pick_modes_drop_the_never_played_this_champion_noise() {
        // 海斗实测：英雄是随机的，几乎每个人这个英雄都是 0 局（测试数据里的人都玩英雄 39 / 157，这局是 103）
        let games: Vec<_> = (0..10)
            .map(|i| h::game(i, 2400, 39, i % 2 == 0, (6, 5, 10), 25000))
            .collect();
        let mut players: Vec<ScoutPlayer> = ["甲", "乙", "丙"]
            .iter()
            .map(|n| scout_player(n, false, None, games.clone()))
            .collect();
        for p in &mut players {
            p.champion_id = 103;
        }
        let aram = render_scout(&players, 2400, "海克斯大乱斗", true, None);
        assert!(
            aram.contains("英雄是随机的，不看玩没玩过这个英雄"),
            "{aram}"
        );
        assert!(
            !aram.contains("最近没玩过") && !aram.contains("局没玩过"),
            "{aram}"
        );
        assert!(!aram.contains("这个英雄 0 局"), "{aram}");
        assert!(!aram.contains("突破口"), "{aram}");

        // 峡谷：同样的数据照旧标出来
        let rift = render_scout(&players, 420, "单双排", false, None);
        assert!(
            rift.contains("这个英雄 0 局 0 胜 · 这个英雄最近没玩过"),
            "{rift}"
        );
        assert!(!rift.contains("英雄是随机的"), "{rift}");
    }
}
