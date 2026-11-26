//! 一键喊话（ADR 041 第 3 期）：`lol.shout` 按这局已经有的数据现生成几行字（不调 AI、不现查），经宿主交给本体
//! **唯一的发话入口**（[`Host::send_to_game`]）。插件只生成内容，不按键：选英雄、房间里入口交回给这个插件的客户端
//! 聊天接口（[`crate::chat`]），对局里是本体的通用按键发法（游戏配置里英雄联盟的聊天键和时序）。
//!
//! 照 LeagueAkari 的“游戏内发送”学做法（`dev` 分支 `src/main/shards/in-game-send/`：预设按下时从这局已经读好的数据
//! 现生成几行再发，`preset-controller.ts`；说的是我方 / 对面 / 全部这些人的情况，不是频道；发的那一步按阶段选路，
//! `send-executor.ts`），只学做法、不抄代码（ADR 039 D5）。四条预设登记在 `plugin.toml` 的 `[[commands]]`。
//!
//! - `flash`（喊对面闪现）：英雄联盟的接口里**没有**冷却，只有玩家报过的（`lol.spell` 记在 [`SpellTracker`]，这局结束清掉）：
//!   每个还没好的一行“亚索 闪 14:32好”（和 `lol.spell` 的 `chat` 同一个写法），再加一行“对面没带闪：阿狸”（局内接口里
//!   的召唤师技能）。都没有就不发，说为什么（没记过谁交了闪，要玩家先说“对面亚索闪了”）。
//! - `records`（喊对面 / 我方战绩）：后台进加载界面就查好的近 20 局（`lol.scout` 同一份）：段位、胜负、KDA、连胜连败、
//!   常玩 / 没玩过这个英雄，一人一行；还有人没查完就不发（团战等不起现查）。
//! - `premade`（喊开黑情况）：同一份数据里的开黑分组，一组一行；看不出开黑就不发。
//!
//! 叫人只用英雄（选英雄时还没选的用分路、几楼），**不发玩家名**（排位选英雄时名字可能是藏着的）。

use kotone_plugin_api::{Host, ToolResult};
use serde_json::{json, Value};

use crate::client::clock;
use crate::client::history::PlayerHistory;
use crate::client::live::LiveGame;
use crate::client::scout;
use crate::client::spells::{short_spell, SpellTracker};
use crate::client::text::{champion_names, Identity, ScoutPlayer};
use crate::client::GameData;
use crate::pages::MatchPhase;
use crate::LolPlugin;

/// 入口一次最多收几行（`kotone_core::send_to_game` 的规矩，超了整段不发）。
const MAX_LINES: usize = 10;

/// 喊什么。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum What {
    Flash,
    Records,
    Premade,
}

/// 说的是谁（不是频道）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Side {
    Enemy,
    Ally,
    All,
}

impl Side {
    fn covers(self, ally: bool) -> bool {
        match self {
            Side::Enemy => !ally,
            Side::Ally => ally,
            Side::All => true,
        }
    }
}

fn side_name(ally: bool) -> &'static str {
    if ally {
        "我方"
    } else {
        "对面"
    }
}

/// `lol.shout` 的参数。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Shout {
    pub what: What,
    pub side: Side,
    /// `team` / `all`；None = 游戏配置的默认频道（队伍）。
    pub channel: Option<String>,
    /// 只看会发什么，不发。
    pub preview: bool,
}

impl Shout {
    pub(crate) fn parse(arguments: &Value) -> Result<Self, String> {
        let what = match arguments.get("what").and_then(Value::as_str) {
            Some("flash") => What::Flash,
            Some("records") => What::Records,
            Some("premade") => What::Premade,
            other => {
                return Err(format!(
                    "what 只能是 flash（对面闪现）、records（战绩）、premade（开黑），不是 {}",
                    other.unwrap_or("空的")
                ))
            }
        };
        let side = match arguments.get("side").and_then(Value::as_str) {
            None => match what {
                What::Premade => Side::All,
                _ => Side::Enemy,
            },
            Some("enemy") => Side::Enemy,
            Some("ally") => Side::Ally,
            Some("all") => Side::All,
            Some(other) => {
                return Err(format!(
                    "side 只能是 enemy（对面）、ally（我方）、all（两边），不是 {other}"
                ))
            }
        };
        let channel = match arguments.get("channel") {
            None | Some(Value::Null) => None,
            Some(Value::String(c)) if c == "team" || c == "all" => Some(c.clone()),
            Some(other) => {
                return Err(format!(
                    "channel 只能是 team（队伍）或 all（所有人），不是 {other}"
                ))
            }
        };
        let preview = arguments.get("preview").and_then(Value::as_bool) == Some(true);
        Ok(Self {
            what,
            side,
            channel,
            preview,
        })
    }
}

/// 闪现：没好的一人一行（先好的在前），没带闪的一行。都没有 → 不发的原因。
pub(crate) fn flash_lines(
    game: &LiveGame,
    data: Option<&GameData>,
    tracker: &SpellTracker,
    side: Side,
) -> Result<Vec<String>, String> {
    let now = game.time;
    let allies: Vec<String> = game.allies().map(|p| p.riot_id.clone()).collect();
    let mut down = Vec::new();
    let mut without: [Vec<String>; 2] = [Vec::new(), Vec::new()];
    for p in &game.players {
        let ally = allies.contains(&p.riot_id);
        if !side.covers(ally) {
            continue;
        }
        let short = champion_names(p, data).1;
        // 和 `lol.match` 同一个判断（不看显示的名字；认不出就不说没带，免得把错的信息发给队友）
        if p.carries_flash() == Some(false) {
            without[usize::from(ally)].push(short);
        } else if let Some(ready) = tracker.ready_at(&p.alias, "闪现", now) {
            let who = if ally {
                format!("我方 {short}")
            } else {
                short
            };
            down.push((
                ready,
                format!("{who} {} {}好", short_spell("闪现"), clock(ready)),
            ));
        }
    }
    down.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut lines: Vec<String> = down.into_iter().map(|(_, line)| line).collect();
    for (ally, names) in [(false, &without[0]), (true, &without[1])] {
        if !names.is_empty() {
            lines.push(format!("{}没带闪：{}", side_name(ally), names.join("、")));
        }
    }
    if lines.is_empty() {
        let whose = match side {
            Side::Ally => "我方",
            _ => "对面",
        };
        return Err(format!(
            "没记到{whose}谁交了闪（英雄联盟的接口里没有冷却，要你先说“对面亚索闪了”她才记得住），{whose}也都带着闪：这次没发"
        ));
    }
    lines.truncate(MAX_LINES);
    Ok(lines)
}

/// 叫这个人：英雄；选英雄时还没选的用分路，再不行用几楼（同一边里的第几个）。不用玩家名。
fn who(p: &ScoutPlayer, floor: usize) -> String {
    let name = if p.champion_id > 0 {
        p.short.clone()
    } else {
        match p.position.as_str() {
            "TOP" => "上单".into(),
            "JUNGLE" => "打野".into(),
            "MIDDLE" => "中单".into(),
            "BOTTOM" => "下路".into(),
            "UTILITY" => "辅助".into(),
            _ => format!("{floor}楼"),
        }
    };
    if p.me {
        format!("{name}（我）")
    } else {
        name
    }
}

/// 这一边的人，带“几楼”（同一边里的第几个，从 1 数）。
fn on_side(players: &[ScoutPlayer], side: Side) -> Vec<(&ScoutPlayer, usize)> {
    let mut floors = [0usize; 2];
    let mut out = Vec::new();
    for p in players {
        let floor = &mut floors[usize::from(p.ally)];
        *floor += 1;
        if side.covers(p.ally) {
            out.push((p, *floor));
        }
    }
    out
}

fn record_line(
    p: &ScoutPlayer,
    floor: usize,
    h: &PlayerHistory,
    queue_id: i64,
    random: bool,
) -> String {
    let mut parts = Vec::new();
    if let Some(rank) = &h.rank {
        parts.push(format!("单双排 {rank}"));
    }
    let s = scout::summarize(&h.games, p.champion_id, queue_id);
    if s.games == 0 {
        parts.push(if h.hidden() {
            "隐藏战绩".into()
        } else if h.error.is_some() {
            "战绩查不到".into()
        } else {
            "最近没有对局".into()
        });
    } else {
        if h.hidden() {
            parts.push("隐藏战绩".into());
        }
        parts.push(format!(
            "近 {} 局 {} 胜 {} 负",
            s.games,
            s.wins,
            s.games - s.wins
        ));
        parts.push(format!("KDA {:.1}", s.kda));
        parts.extend(scout::tags(&s, random));
    }
    format!(
        "{} {}：{}",
        side_name(p.ally),
        who(p, floor),
        parts.join(" · ")
    )
}

/// 战绩：一人一行。还有人没查完 → 不发的原因（不现查）。
pub(crate) fn record_lines(
    players: &[ScoutPlayer],
    queue_id: i64,
    random: bool,
    side: Side,
) -> Result<Vec<String>, String> {
    let wanted = on_side(players, side);
    if wanted.is_empty() {
        return Err("这局还没有这一边的人：这次没发".into());
    }
    // 隐藏了身份的、电脑不查，不算没查完（ADR 043 D9）
    let waiting = wanted.iter().filter(|(p, _)| p.pending()).count();
    if waiting > 0 {
        return Err(format!(
            "战绩还没查完（还差 {waiting} 个人），过几秒再按：这次没发"
        ));
    }
    let mut lines: Vec<String> = wanted
        .iter()
        .filter_map(|(p, floor)| match p.identity {
            Identity::Hidden => Some(format!(
                "{} {}：隐藏了身份",
                side_name(p.ally),
                who(p, *floor)
            )),
            Identity::Bot => Some(format!("{} {}：电脑", side_name(p.ally), who(p, *floor))),
            _ => p
                .history
                .as_ref()
                .map(|h| record_line(p, *floor, h, queue_id, random)),
        })
        .collect();
    lines.truncate(MAX_LINES);
    Ok(lines)
}

/// 开黑：一组一行（两个人以上同一个 `teamParticipantId`）。看不出开黑 → 不发的原因。
pub(crate) fn premade_lines(players: &[ScoutPlayer], side: Side) -> Result<Vec<String>, String> {
    let mut groups: Vec<(bool, i64, Vec<String>)> = Vec::new();
    for (p, floor) in on_side(players, side) {
        let Some(party) = p.party.filter(|id| *id > 0) else {
            continue;
        };
        match groups
            .iter_mut()
            .find(|(a, id, _)| *a == p.ally && *id == party)
        {
            Some((_, _, names)) => names.push(who(p, floor)),
            None => groups.push((p.ally, party, vec![who(p, floor)])),
        }
    }
    // 对面在前（照 `lol.scout`：先说对面）
    groups.sort_by_key(|(ally, _, _)| *ally);
    let lines: Vec<String> = groups
        .into_iter()
        .filter(|(_, _, names)| names.len() >= 2)
        .map(|(ally, _, names)| format!("{}开黑：{}", side_name(ally), names.join(" + ")))
        .take(MAX_LINES)
        .collect();
    if lines.is_empty() {
        return Err("这局没看出谁和谁开黑：这次没发".into());
    }
    Ok(lines)
}

/// 插件给的状态（`{"status", "text"}`）→ 给玩家的一句。
fn reason(value: &Value) -> String {
    let text = value
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or("现在不在英雄联盟的对局里");
    format!("{text}：这次没发")
}

impl LolPlugin {
    /// `lol.shout`：生成几行（不发的说为什么，返回 Err，热键那边照“没成”说给玩家），`preview` 只给看，否则经入口发。
    pub(super) async fn shout_tool(
        &self,
        host: &dyn Host,
        arguments: &Value,
    ) -> Result<ToolResult, String> {
        let shout = Shout::parse(arguments)?;
        let lines = self.shout_lines(host, &shout).await?;
        let joined = lines.join(" / ");
        if shout.preview {
            return Ok(ToolResult::content(json!({
                "status": "ok",
                "preview": true,
                "lines": lines,
                "text": format!("会发：{joined}"),
            })));
        }
        // 锁都放开了才交给入口（客户端在前台时入口会回头调这个插件的发法）
        let mut sent = host
            .send_to_game(&lines.join("\n"), shout.channel.as_deref())
            .await?;
        if let Some(object) = sent.as_object_mut() {
            object.insert("status".into(), json!("ok"));
            object.insert("lines".into(), json!(lines));
            object.insert("text".into(), json!(format!("发了：{joined}")));
        }
        Ok(ToolResult::content(sent))
    }

    async fn shout_lines(&self, host: &dyn Host, shout: &Shout) -> Result<Vec<String>, String> {
        if shout.what == What::Flash {
            let game = self.live_game(host).await.map_err(|v| reason(&v))?;
            let data = self.ensure_data().await;
            let tracker = self.state().tracker.clone();
            return flash_lines(&game, data.as_deref(), &tracker, shout.side);
        }
        let data = self.ensure_data().await;
        let no_match = {
            let s = self.state();
            s.current.is_none() || s.stage.is_none()
        };
        if no_match {
            return Err(reason(&self.not_in_game(host)));
        }
        let view = self
            .match_view(data.as_deref())
            .ok_or("这局刚结束：这次没发")?;
        // 选英雄时对面只有英雄、查不到人（和 `lol.scout` 一样只说我方）
        let players: Vec<ScoutPlayer> = if view.stage.phase == Some(MatchPhase::ChampSelect) {
            if shout.side == Side::Enemy {
                return Err("还在选英雄，看不到对面是谁；进了加载界面再按：这次没发".into());
            }
            view.players.into_iter().filter(|p| p.ally).collect()
        } else {
            view.players
        };
        match shout.what {
            What::Records => record_lines(&players, view.queue_id, view.random, shout.side),
            _ => premade_lines(&players, shout.side),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::history::{parse_match_list, tests as h};
    use crate::client::live::{parse_all_game_data, tests as l};
    use crate::client::spells::SpellUse;
    use crate::tools::TOOL_SHOUT;
    use std::sync::Arc;

    fn flash(tracker: &mut SpellTracker, alias: &str, champion: &str, ready_at: f64) {
        tracker.record(SpellUse {
            alias: alias.into(),
            champion: champion.into(),
            spell: "闪现".into(),
            used_at: ready_at - 300.0,
            ready_at,
            notes: vec![],
        });
    }

    #[test]
    fn arguments_default_to_the_enemy_and_the_default_channel() {
        let flash = Shout::parse(&json!({"what": "flash"})).unwrap();
        assert_eq!(
            (flash.what, flash.side, flash.channel, flash.preview),
            (What::Flash, Side::Enemy, None, false)
        );
        let premade =
            Shout::parse(&json!({"what": "premade", "channel": "team", "preview": true})).unwrap();
        assert_eq!(premade.side, Side::All, "开黑默认两边都说");
        assert_eq!(premade.channel.as_deref(), Some("team"));
        assert!(premade.preview);
        for bad in [
            json!({}),
            json!({"what": "jungle"}),
            json!({"what": "records", "side": "mine"}),
            json!({"what": "records", "channel": "party"}),
        ] {
            assert!(Shout::parse(&bad).is_err(), "{bad}");
        }
    }

    /// 假局内接口（海斗：我方艾瑞莉娅，对面亚索带闪、阿狸没带闪），闪现只有玩家报过的才知道。
    #[test]
    fn flash_lines_come_from_what_the_player_reported() {
        let data = crate::client::gamedata::tests::sample();
        let game = parse_all_game_data(&l::sample_json()).unwrap();
        let mut tracker = SpellTracker::default();
        // 没记过：只有“没带闪”那一行
        assert_eq!(
            flash_lines(&game, Some(&data), &tracker, Side::Enemy).unwrap(),
            ["对面没带闪：阿狸"]
        );
        flash(&mut tracker, "Yasuo", "亚索", 866.7);
        assert_eq!(
            flash_lines(&game, Some(&data), &tracker, Side::Enemy).unwrap(),
            ["亚索 闪 14:27好", "对面没带闪：阿狸"],
            "和 lol.spell 的 chat 同一个写法"
        );
        // 已经好了的不喊
        flash(&mut tracker, "Yasuo", "亚索", 700.0);
        assert_eq!(
            flash_lines(&game, Some(&data), &tracker, Side::Enemy).unwrap(),
            ["对面没带闪：阿狸"]
        );
        // 我方：艾瑞莉娅带着闪、没记过 → 没东西可喊，说为什么
        let none = flash_lines(&game, Some(&data), &tracker, Side::Ally).unwrap_err();
        assert!(
            none.contains("没记到我方谁交了闪") && none.contains("这次没发"),
            "{none}"
        );
        // 对面都带着闪、谁也没记 → 不发
        let mut all_flash = l::sample_json();
        all_flash["allPlayers"][2]["summonerSpells"]["summonerSpellOne"] = json!({
            "displayName": "闪现",
            "rawDisplayName": "GeneratedTip_SummonerSpell_SummonerFlash_DisplayName"
        });
        let game = parse_all_game_data(&all_flash).unwrap();
        let none =
            flash_lines(&game, Some(&data), &SpellTracker::default(), Side::Enemy).unwrap_err();
        assert!(none.contains("对面亚索闪了"), "告诉玩家怎么让她记：{none}");
    }

    /// 带没带闪按不随语言变的 `rawDisplayName` 认：英文客户端（显示 Flash）不会被当成没带闪；字段缺、认不出时不喊
    /// “没带闪”（宁可不说，也不把错的信息发给队友）。
    #[test]
    fn no_flash_is_only_said_when_the_raw_names_say_so() {
        let data = crate::client::gamedata::tests::sample();
        let english = |v: &mut Value, i: usize, one: (&str, &str), two: (&str, &str)| {
            v["allPlayers"][i]["summonerSpells"] = json!({
                "summonerSpellOne": {"displayName": one.0, "rawDisplayName": format!("GeneratedTip_SummonerSpell_{}_DisplayName", one.1)},
                "summonerSpellTwo": {"displayName": two.0, "rawDisplayName": format!("GeneratedTip_SummonerSpell_{}_DisplayName", two.1)},
            });
        };
        // 英文客户端：亚索 Flash + Ignite、阿狸 Mark + Ignite → 只有阿狸没带闪
        let mut en = l::sample_json();
        english(
            &mut en,
            1,
            ("Flash", "SummonerFlash"),
            ("Ignite", "SummonerDot"),
        );
        english(
            &mut en,
            2,
            ("Mark", "SummonerSnowball"),
            ("Ignite", "SummonerDot"),
        );
        let game = parse_all_game_data(&en).unwrap();
        let tracker = SpellTracker::default();
        assert_eq!(
            flash_lines(&game, Some(&data), &tracker, Side::Enemy).unwrap(),
            ["对面没带闪：阿狸"]
        );
        let text = crate::client::text::enemy_flash_line(&game, Some(&data), &tracker);
        assert!(
            text.ends_with("没带闪：阿狸"),
            "lol.match 同一个判断：{text}"
        );
        // 字段缺（只有显示的名字）：不知道谁没带，不喊
        let mut missing = l::sample_json();
        for i in 0..3 {
            for slot in ["summonerSpellOne", "summonerSpellTwo"] {
                missing["allPlayers"][i]["summonerSpells"][slot]
                    .as_object_mut()
                    .unwrap()
                    .remove("rawDisplayName");
            }
        }
        let game = parse_all_game_data(&missing).unwrap();
        let none = flash_lines(&game, Some(&data), &tracker, Side::Enemy).unwrap_err();
        assert!(none.contains("这次没发"), "{none}");
        let text = crate::client::text::enemy_flash_line(&game, Some(&data), &tracker);
        assert!(!text.contains("没带闪"), "{text}");
        // 报过的照样喊
        let mut tracker = SpellTracker::default();
        flash(&mut tracker, "Yasuo", "亚索", 866.7);
        assert_eq!(
            flash_lines(&game, Some(&data), &tracker, Side::Enemy).unwrap(),
            ["亚索 闪 14:27好"]
        );
    }

    fn player(
        short: &str,
        ally: bool,
        party: Option<i64>,
        games: Option<Vec<Value>>,
    ) -> ScoutPlayer {
        ScoutPlayer {
            champion: format!("称号 {short}"),
            alias: short.into(),
            short: short.into(),
            champion_id: 39,
            name: format!("{short}的主人"),
            ally,
            party,
            history: games.map(|games| PlayerHistory {
                riot_id: format!("{short}的主人#1"),
                privacy: "PUBLIC".into(),
                games: parse_match_list(&h::list(games)),
                rank: Some("黄金 II".into()),
                ..PlayerHistory::default()
            }),
            ..ScoutPlayer::default()
        }
    }

    /// 假客户端的战绩（近 20 局）：一人一行、带段位胜负 KDA 和连败；不带玩家名；还有人没查完不发。
    #[test]
    fn record_lines_say_one_player_per_line_without_names() {
        let good: Vec<Value> = (0..10)
            .map(|i| h::game(i, 420, 39, i % 3 != 0, (8, 3, 10), 30000))
            .collect();
        let bad: Vec<Value> = (0..10)
            .map(|i| h::game(i, 420, 39, i >= 5, (2, 9, 4), 12000))
            .collect();
        let mut players = vec![
            player("甲", true, Some(1), Some(good.clone())),
            player("乙", false, Some(7), Some(good.clone())),
            player("丙", false, Some(7), Some(bad)),
        ];
        players[0].me = true;
        let enemy = record_lines(&players, 420, false, Side::Enemy).unwrap();
        assert_eq!(enemy.len(), 2);
        assert!(
            enemy[0].starts_with("对面 乙：单双排 黄金 II · 近 10 局 6 胜 4 负 · KDA"),
            "{}",
            enemy[0]
        );
        assert!(enemy[1].contains("5 连败"), "{}", enemy[1]);
        let ally = record_lines(&players, 420, false, Side::Ally).unwrap();
        assert_eq!(ally.len(), 1);
        assert!(ally[0].starts_with("我方 甲（我）："), "{}", ally[0]);
        assert_eq!(
            record_lines(&players, 420, false, Side::All).unwrap().len(),
            3
        );
        for line in enemy.iter().chain(&ally) {
            assert!(!line.contains("的主人"), "不发玩家名：{line}");
            assert!(line.chars().count() <= 200, "{line}");
        }
        // 隐藏战绩
        players[1].history.as_mut().unwrap().privacy = "PRIVATE".into();
        players[1].history.as_mut().unwrap().games.clear();
        assert!(record_lines(&players, 420, false, Side::Enemy).unwrap()[0]
            .ends_with("：单双排 黄金 II · 隐藏战绩"));
        // 还在查的：这边不发、说还差几个；另一边照发
        players[2].history = None;
        let waiting = record_lines(&players, 420, false, Side::Enemy).unwrap_err();
        assert!(waiting.contains("还差 1 个人"), "{waiting}");
        assert!(record_lines(&players, 420, false, Side::Ally).is_ok());
        // 隐藏了身份的、电脑（ADR 043 D9）：不算没查完，一行写明
        players[2].identity = Identity::Hidden;
        let lines = record_lines(&players, 420, false, Side::Enemy).unwrap();
        assert!(
            lines.iter().any(|l| l.ends_with("：隐藏了身份")),
            "{lines:?}"
        );
        players[2].identity = Identity::Bot;
        let lines = record_lines(&players, 420, false, Side::Enemy).unwrap();
        assert!(lines.iter().any(|l| l.ends_with("：电脑")), "{lines:?}");
    }

    /// 选英雄时还没选英雄的用分路、几楼叫；开黑一组一行，对面在前；看不出开黑不发。
    #[test]
    fn premade_lines_group_by_party_and_never_name_players() {
        let games = Some(vec![h::game(1, 420, 39, true, (5, 5, 5), 20000)]);
        let mut players = vec![
            player("甲", true, Some(1), games.clone()),
            player("乙", true, Some(1), games.clone()),
            player("丙", false, Some(7), games.clone()),
            player("丁", false, Some(7), games.clone()),
            player("戊", false, Some(9), games.clone()),
        ];
        assert_eq!(
            premade_lines(&players, Side::All).unwrap(),
            ["对面开黑：丙 + 丁", "我方开黑：甲 + 乙"]
        );
        assert_eq!(
            premade_lines(&players, Side::Ally).unwrap(),
            ["我方开黑：甲 + 乙"]
        );
        // 还没选英雄：分路，没分路用几楼
        players[1].champion_id = 0;
        players[1].position = "JUNGLE".into();
        players[0].champion_id = 0;
        assert_eq!(
            premade_lines(&players, Side::Ally).unwrap(),
            ["我方开黑：1楼 + 打野"]
        );
        for p in &mut players {
            p.party = None;
        }
        assert!(premade_lines(&players, Side::All)
            .unwrap_err()
            .contains("没看出谁和谁开黑"));
    }

    /// 宿主那一面：只记下交给入口的话和频道（入口的规矩不在插件里）；`refuse` = 入口不发（前台不是游戏之类）。
    #[derive(Default)]
    struct FakeHost {
        client_off: bool,
        refuse: Option<&'static str>,
        sent: std::sync::Mutex<Vec<(String, Option<String>)>>,
    }

    impl Host for FakeHost {
        fn plugin(&self) -> &str {
            "lol"
        }
        fn enabled(&self) -> bool {
            true
        }
        fn source_enabled(&self, _source: &str) -> bool {
            !self.client_off
        }
        fn settings(&self) -> Value {
            json!({})
        }
        fn data_dir(&self) -> std::path::PathBuf {
            std::path::PathBuf::new()
        }
        fn spawn(&self, _task: kotone_plugin_api::BoxFuture<'static, ()>) {}
        fn process_ids(&self, _exe: &str) -> Vec<u32> {
            Vec::new()
        }
        fn process_command_line(&self, _pid: u32) -> Option<String> {
            None
        }
        fn game_observe(&self, _game: &str, _observation: kotone_plugin_api::Observation) {}
        fn game_claim(&self, _game: &str, _claimed: bool) {}
        fn game_event(
            &self,
            _game: &str,
            _kind: &str,
            _source: &str,
            _data: serde_json::Map<String, Value>,
            _game_time: Option<f64>,
        ) {
        }
        fn send_to_game(
            &self,
            text: &str,
            channel: Option<&str>,
        ) -> kotone_plugin_api::BoxFuture<'static, Result<Value, String>> {
            self.sent
                .lock()
                .unwrap()
                .push((text.to_string(), channel.map(str::to_string)));
            let result = match self.refuse {
                Some(why) => Err(why.to_string()),
                None => Ok(
                    json!({"route": "keys", "channelName": "队伍", "sentLines": text.lines().count()}),
                ),
            };
            Box::pin(async move { result })
        }
    }

    /// 假客户端查好的这局（4 个人：我方甲乙开黑，对面丙丁开黑；丁 5 连败）放进插件的状态，再经工具走一遍：
    /// 交给宿主的是生成好的几行、预设的频道；只看不发；没东西可喊、宿主不发都是没成（Err），什么都没交 / 照实交。
    #[tokio::test]
    async fn the_tool_hands_the_lines_to_the_host_and_never_sends_on_its_own() {
        let good: Vec<Value> = (0..10)
            .map(|i| h::game(i, 420, 39, i % 3 != 0, (8, 3, 10), 30000))
            .collect();
        let bad: Vec<Value> = (0..10)
            .map(|i| h::game(i, 420, 39, i >= 5, (2, 9, 4), 12000))
            .collect();
        let plugin = LolPlugin::new();
        {
            let mut s = plugin.state();
            let member = |puuid: &str, team: u8, party: i64| super::super::Member {
                puuid: puuid.into(),
                champion_id: 39,
                team,
                party: Some(party),
                ..Default::default()
            };
            let history = |games: &[Value]| PlayerHistory {
                riot_id: "别人的名字#1".into(),
                privacy: "PUBLIC".into(),
                games: parse_match_list(&h::list(games.to_vec())),
                ..PlayerHistory::default()
            };
            s.current = Some(super::super::Match {
                queue_id: 420,
                mode: "CLASSIC".into(),
                members: vec![
                    member("a", 1, 1),
                    member("b", 1, 1),
                    member("c", 2, 7),
                    member("d", 2, 7),
                ],
                histories: [
                    ("a".to_string(), history(&good)),
                    ("b".to_string(), history(&good)),
                    ("c".to_string(), history(&good)),
                    ("d".to_string(), history(&bad)),
                ]
                .into_iter()
                .collect(),
                started: true,
                ..Default::default()
            });
            s.stage = Some((MatchPhase::InGame, 0));
        }
        let host = Arc::new(FakeHost::default());
        let dyn_host: Arc<dyn Host> = host.clone();
        let records = json!({"what": "records", "side": "enemy", "channel": "team"});
        let result = plugin.tool(&dyn_host, TOOL_SHOUT, &records).await.unwrap();
        let sent = host.sent.lock().unwrap().clone();
        assert_eq!(sent.len(), 1, "一次交给入口");
        let (text, channel) = &sent[0];
        assert_eq!(channel.as_deref(), Some("team"));
        assert_eq!(text.lines().count(), 2);
        assert!(text.lines().all(|l| l.starts_with("对面 ")), "{text}");
        assert!(
            text.contains("5 连败") && !text.contains("别人的名字"),
            "{text}"
        );
        assert_eq!(result.content["status"], "ok");
        assert_eq!(result.content["route"], "keys", "入口的结果原样带回");
        assert!(result.content["text"]
            .as_str()
            .unwrap()
            .starts_with("发了："));

        // 只看：不交给入口
        let premade = json!({"what": "premade", "preview": true});
        let preview = plugin.tool(&dyn_host, TOOL_SHOUT, &premade).await.unwrap();
        assert_eq!(
            preview.content["lines"],
            json!(["对面开黑：英雄 39 + 英雄 39", "我方开黑：英雄 39 + 英雄 39"])
        );
        assert_eq!(host.sent.lock().unwrap().len(), 1);

        // 入口不发（前台不是游戏……）：原因照实交回，算没成
        let refusing = Arc::new(FakeHost {
            refuse: Some("前台是 warp.exe，不是「League of Legends」：请先切到游戏"),
            ..FakeHost::default()
        });
        let refusing_host: Arc<dyn Host> = refusing.clone();
        let err = plugin
            .tool(&refusing_host, TOOL_SHOUT, &records)
            .await
            .unwrap_err();
        assert!(err.contains("请先切到游戏"), "{err}");

        // 没东西可喊 / 读客户端数据关着：什么都不交给入口
        let off = Arc::new(FakeHost {
            client_off: true,
            ..FakeHost::default()
        });
        let off_host: Arc<dyn Host> = off.clone();
        let err = plugin
            .tool(&off_host, TOOL_SHOUT, &records)
            .await
            .unwrap_err();
        assert!(
            err.contains("读客户端数据") && err.contains("没发"),
            "{err}"
        );
        plugin.state().current = None;
        let err = plugin
            .tool(&dyn_host, TOOL_SHOUT, &records)
            .await
            .unwrap_err();
        assert_eq!(err, "英雄联盟没开：这次没发");
        assert!(off.sent.lock().unwrap().is_empty());
        assert_eq!(host.sent.lock().unwrap().len(), 1);
    }
}
