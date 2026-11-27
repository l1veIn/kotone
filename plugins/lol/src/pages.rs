//! 页面的数据（ADR 038 D5 `structuredContent`）：这局、战绩、单局详情。她拿到的是 [`crate::client::text`]
//! 写的几行字；页面拿到的是这里的结构化数据，聊天卡片是它的快照。数字和她说的同一份（[`scout::summarize`]）。
//!
//! 照 LeagueAkari 的页面：这局 10 个人（段位、近况、标签、隐藏战绩、开黑；隐藏了身份的人、电脑只有英雄）；
//! 点一个人看他最近的战绩；点一局看 10 个人的 KDA、伤害、经济、装备。只给事实，不评价人（ADR 043 D2）。
//!
//! **图**（ADR 039 D6，调研 G0.1）：数据里只放 id 和客户端的资源路径（`icon`、`profileIcon`、`crest`……，
//! 例如 `/lol-game-data/assets/v1/champion-icons/103.png`），不放完整网址、不放图片本身。页面自己决定从哪取：
//! Kotone 里经回环代理（素材层 [`crate::client::assets`]），别的宿主换成 CommunityDragon 的地址。
//! 拿不到路径时是空字符串（页面显示占位）。给她的字（[`player_text`]、`text::render_scout`）不受影响。

use serde_json::{json, Value};

use crate::client::assets;
use crate::client::history::{GameRecord, PlayerHistory, Ranked};
use crate::client::scout::{self, Summary};
use crate::client::text::ScoutPlayer;
use crate::client::GameData;

pub const PAGE_HOME: &str = "ui://kotone/lol/home";
pub const PAGE_MATCH: &str = "ui://kotone/lol/match";
pub const PAGE_PLAYER: &str = "ui://kotone/lol/player";
pub const PAGE_GAME: &str = "ui://kotone/lol/game";
/// 今天的状态（只在推送里出现的卡片：进房间或排队时，`watcher/auto_cards/today.rs`）。
pub const PAGE_TODAY: &str = "ui://kotone/lol/today";
pub const PAGE_SETTINGS: &str = "ui://kotone/lol/settings";

/// 页面里每个人最多带几局近况（这局页面上一排小格子）。
const RECENT: usize = 10;

pub(crate) fn champion(data: Option<&GameData>, id: i64) -> Value {
    let icon = assets::champion_icon(id);
    match data.and_then(|d| d.champion(id)) {
        Some(c) => {
            json!({"id": id, "label": c.label(), "short": c.short(), "alias": c.alias, "icon": icon})
        }
        None => {
            json!({"id": id, "label": format!("英雄 {id}"), "short": format!("英雄 {id}"), "alias": "", "icon": icon})
        }
    }
}

/// 召唤师技能 `{id, name, icon}`（0 = 没有，不列）。
fn spells(data: Option<&GameData>, ids: &[i64; 2]) -> Vec<Value> {
    ids.iter()
        .filter(|id| **id > 0)
        .map(|id| {
            let spell = data.and_then(|d| d.spell_by_id(*id));
            json!({
                "id": id,
                "name": spell.map(|s| s.name.as_str()).unwrap_or(""),
                "icon": spell.map(|s| s.icon.as_str()).unwrap_or(""),
            })
        })
        .collect()
}

/// 装备 `{id, name, icon, slot}`，按格子（`slot` 0–6，6 是饰品），空格不列。
fn items(data: Option<&GameData>, slots: &[i64]) -> Vec<Value> {
    slots
        .iter()
        .enumerate()
        .filter(|(_, id)| **id > 0)
        .map(|(slot, id)| {
            json!({
                "id": id,
                "name": data.and_then(|d| d.items.get(id)).cloned().unwrap_or_default(),
                "icon": data.and_then(|d| d.item_icons.get(id)).cloned().unwrap_or_default(),
                "slot": slot,
            })
        })
        .collect()
}

/// 海克斯 `{id, name, rarity, icon}`（`rarity`：`kSilver` / `kGold` / `kPrismatic`，不知道时空）。
fn augments(data: Option<&GameData>, ids: &[i64]) -> Vec<Value> {
    ids.iter()
        .map(|id| {
            let a = data.and_then(|d| d.augments.get(id));
            json!({
                "id": id,
                "name": a.map(|a| a.name.as_str()).unwrap_or(""),
                "rarity": a.map(|a| a.rarity.as_str()).unwrap_or(""),
                "icon": a.map(|a| a.icon.as_str()).unwrap_or(""),
            })
        })
        .collect()
}

/// 符文：`{keystone, primaryStyle, subStyle}`，各是 `{id, name, icon}` 或 null；三个都没有（海斗、斗魂竞技场）时整个 null。
fn perks(data: Option<&GameData>, keystone: i64, primary: i64, sub: i64) -> Value {
    if keystone <= 0 && primary <= 0 && sub <= 0 {
        return Value::Null;
    }
    let one = |id: i64, style: bool| {
        (id > 0).then(|| {
            let p = data.and_then(|d| {
                if style {
                    d.perk_styles.get(&id)
                } else {
                    d.perks.get(&id)
                }
            });
            json!({
                "id": id,
                "name": p.map(|p| p.name.as_str()).unwrap_or(""),
                "icon": p.map(|p| p.icon.as_str()).unwrap_or(""),
            })
        })
    };
    json!({
        "keystone": one(keystone, false),
        "primaryStyle": one(primary, true),
        "subStyle": one(sub, true),
    })
}

/// 一个排位队列：`{queue, tier, division, lp, wins, losses, highestTier, label, crest}`；没查到是 null。
/// 没定级时 `tier` 空、`label` 是“未定级”、`crest` 是 `unranked` 的徽章。
fn ranked(r: Option<&Ranked>) -> Value {
    match r {
        None => Value::Null,
        Some(r) => json!({
            "queue": r.queue,
            "tier": r.tier,
            "division": r.division,
            "lp": r.lp,
            "wins": r.wins,
            "losses": r.losses,
            "highestTier": r.highest_tier,
            "label": r.label(),
            "crest": assets::ranked_crest(&r.tier),
        }),
    }
}

/// 单双排和灵活组排：`{solo, flex}`。
fn ranks(h: Option<&PlayerHistory>) -> Value {
    json!({
        "solo": ranked(h.and_then(|h| h.solo.as_ref())),
        "flex": ranked(h.and_then(|h| h.flex.as_ref())),
    })
}

pub(crate) fn queue(data: Option<&GameData>, id: i64, mode: &str) -> String {
    data.and_then(|d| d.queue_name(id))
        .map(str::to_string)
        .unwrap_or_else(|| crate::client::text::mode_name(mode).to_string())
}

pub fn summary_json(s: &Summary) -> Value {
    json!({
        "games": s.games,
        "wins": s.wins,
        "losses": s.games - s.wins,
        "streak": s.streak,
        "kda": (s.kda * 10.0).round() / 10.0,
        "deaths": (s.deaths * 10.0).round() / 10.0,
        "modeGames": s.mode_games,
        "modeWins": s.mode_wins,
        "modeDpm": s.mode_dpm.round(),
        "champGames": s.champ_games,
        "champWins": s.champ_wins,
    })
}

/// 战绩里的一局（这个人自己的数据）。
pub fn game_json(g: &GameRecord, data: Option<&GameData>) -> Value {
    json!({
        "gameId": g.game_id,
        "queueId": g.queue_id,
        "queue": queue(data, g.queue_id, &g.mode),
        "createdMs": g.created_ms,
        "durationS": g.duration_s,
        "champion": champion(data, g.champion_id),
        "win": g.win,
        "kills": g.kills,
        "deaths": g.deaths,
        "assists": g.assists,
        "damage": g.damage,
        "dpm": (g.damage as f64 / g.minutes()).round(),
        "gold": g.gold,
        "cs": g.cs,
        "augments": augments(data, &g.augments),
        "level": g.level,
        "spells": spells(data, &g.spells),
        "items": items(data, &g.items),
        "perks": perks(data, g.keystone, g.primary_style, g.sub_style),
    })
}

/// 这局的阶段（游戏中心的阶段，ADR 036；docs/design/lol-app.md 第二节 1 的 `phase`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchPhase {
    ChampSelect,
    Loading,
    InGame,
    PostMatch,
}

impl MatchPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ChampSelect => "champ_select",
            Self::Loading => "loading",
            Self::InGame => "in_game",
            Self::PostMatch => "post_match",
        }
    }
}

/// 选人时一个人选好了没有（`pick`）：锁了 / 预选 / 还没选。别的阶段都是锁了。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Pick {
    #[default]
    Locked,
    Intent,
    None,
}

impl Pick {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Locked => "locked",
            Self::Intent => "intent",
            Self::None => "none",
        }
    }
}

/// 结算后这局的结果（`result`，调研 G29）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MatchResult {
    pub win: bool,
    pub duration_s: i64,
    pub game_id: i64,
}

/// 这局现在在哪个阶段、只有那个阶段才有的东西（“这局”页面的四种状态，docs/design/lol-app.md 第三节 1）。
/// 阶段由后台（[`crate::watcher`]）知道；[`match_page`] 不带这些（阶段是 null，页面显示“在对局里”）。
#[derive(Debug, Clone, Default)]
pub struct MatchStage {
    pub phase: Option<MatchPhase>,
    /// 进这个阶段的时间（Unix 毫秒），页面算“进加载 N 秒”。
    pub since_ms: Option<i64>,
    /// 排位选人时名字被隐藏（调研 G1）。
    pub names_hidden: bool,
    /// 备选席的英雄 id（大乱斗、海斗选人）。
    pub bench: Vec<i64>,
    /// 选人时每个人选好了没有，和 `players` 一一对应；短了的算锁了。
    pub picks: Vec<Pick>,
    /// 结算后这局的结果；还没拿到是 None。
    pub result: Option<MatchResult>,
    /// 结算后还在等结果（最多 60 秒）。
    pub result_pending: bool,
}

/// 标签带种类（页面按种类上色）：`{kind, text}`，`text` 和给她的字里那句一样。
fn tags_json(s: &Summary, random_pick: bool) -> Vec<Value> {
    scout::tagged(s, random_pick)
        .into_iter()
        .map(|(kind, text)| json!({"kind": kind, "text": text}))
        .collect()
}

/// 这局（`lol.scout`）：两边各 5 个人、开黑。`puuids` 和 `players` 一一对应。不带阶段（见 [`match_page_in`]）。
pub fn match_page(
    players: &[ScoutPlayer],
    puuids: &[String],
    game_id: i64,
    queue_id: i64,
    mode: &str,
    random_pick: bool,
    data: Option<&GameData>,
) -> Value {
    match_page_in(
        players,
        puuids,
        game_id,
        queue_id,
        mode,
        random_pick,
        data,
        &MatchStage::default(),
    )
}

/// 同 [`match_page`]，再写上阶段和那个阶段才有的东西（选人的预选、备选席，结算的结果）。
#[allow(clippy::too_many_arguments)]
pub fn match_page_in(
    players: &[ScoutPlayer],
    puuids: &[String],
    game_id: i64,
    queue_id: i64,
    mode: &str,
    random_pick: bool,
    data: Option<&GameData>,
    stage: &MatchStage,
) -> Value {
    let summaries: Vec<Option<Summary>> = players
        .iter()
        .map(|p| {
            p.history
                .as_ref()
                .map(|h| scout::summarize(&h.games, p.champion_id, queue_id))
        })
        .collect();
    // 还在查：查得到的人（不是隐藏了身份、电脑、选人时看不到名字的）、有 puuid、战绩还没回来
    let pending = |i: usize| players[i].pending() && puuids.get(i).is_some_and(|p| !p.is_empty());
    let person = |i: usize| {
        let p = &players[i];
        let h = p.history.as_ref();
        let s = summaries[i].as_ref().filter(|s| s.games > 0);
        let icon_id = p.profile_icon_id.or(h.and_then(|h| h.profile_icon_id));
        json!({
            "puuid": puuids.get(i).cloned().unwrap_or_default(),
            "champion": champion(data, p.champion_id),
            "profileIconId": icon_id,
            "profileIcon": icon_id.map(assets::profile_icon).unwrap_or_default(),
            "summonerLevel": h.map(|h| h.summoner_level).filter(|l| *l > 0),
            "spells": spells(data, &p.spells),
            "position": p.position,
            "ranked": ranks(h),
            "name": p.name,
            "riotId": h.map(|h| h.riot_id.clone()).unwrap_or_default(),
            "me": p.me,
            "ally": p.ally,
            "party": p.party,
            // 没有 puuid 的（隐藏了身份、电脑、选人时的对面、排位里看不到名字的队友）不查，也不算“还在查”
            "pending": pending(i),
            // known / hidden（隐藏了身份）/ name_hidden（选人时看不到名字）/ bot（电脑），ADR 043 D9
            "identity": p.identity.as_str(),
            "hidden": h.is_some_and(PlayerHistory::hidden),
            "rank": h.and_then(|h| h.rank.clone()),
            "error": h.and_then(|h| h.error.clone()),
            "summary": s.map(summary_json),
            "tags": s.map(|s| tags_json(s, random_pick)).unwrap_or_default(),
            "recent": h.map(|h| h.games.iter().take(RECENT).map(|g| json!({
                "gameId": g.game_id,
                "win": g.win,
                "championId": g.champion_id,
                "championIcon": assets::champion_icon(g.champion_id),
                "kills": g.kills,
                "deaths": g.deaths,
                "assists": g.assists,
            })).collect::<Vec<_>>()).unwrap_or_default(),
            "pick": stage.picks.get(i).copied().unwrap_or_default().as_str(),
        })
    };
    let team = |ally: bool| -> Vec<Value> {
        (0..players.len())
            .filter(|i| players[*i].ally == ally)
            .map(person)
            .collect()
    };
    json!({
        "gameId": game_id,
        "queueId": queue_id,
        "mode": mode,
        "random": random_pick,
        "allies": team(true),
        "enemies": team(false),
        "waiting": (0..players.len()).filter(|i| pending(*i)).count(),
        "phase": stage.phase.map(MatchPhase::as_str),
        "phaseSince": stage.since_ms,
        "namesHidden": stage.names_hidden,
        "bench": stage.bench.iter().map(|id| champion(data, *id)).collect::<Vec<_>>(),
        "result": stage.result.map(|r| json!({"win": r.win, "durationS": r.duration_s, "gameId": r.game_id})),
        "resultPending": stage.result_pending,
    })
}

/// 一个人的战绩（`lol.player`）。`queue_id`：和这局同一个模式的单独算（不在对局里时 0）。
pub fn player_page(
    puuid: &str,
    history: &PlayerHistory,
    queue_id: i64,
    data: Option<&GameData>,
) -> Value {
    let all = scout::summarize(&history.games, 0, queue_id);
    // 常玩的英雄：按局数，最多 5 个
    let mut champions: Vec<(i64, usize, usize)> = Vec::new();
    for g in &history.games {
        match champions.iter_mut().find(|(id, _, _)| *id == g.champion_id) {
            Some(c) => {
                c.1 += 1;
                c.2 += usize::from(g.win);
            }
            None => champions.push((g.champion_id, 1, usize::from(g.win))),
        }
    }
    champions.sort_by_key(|(_, games, _)| std::cmp::Reverse(*games));
    champions.truncate(5);
    json!({
        "puuid": puuid,
        "riotId": history.riot_id,
        "profileIconId": history.profile_icon_id,
        "profileIcon": history.profile_icon_id.map(assets::profile_icon).unwrap_or_default(),
        "summonerLevel": (history.summoner_level > 0).then_some(history.summoner_level),
        "ranked": ranks(Some(history)),
        "hidden": history.hidden(),
        "rank": history.rank,
        "error": history.error,
        "summary": (all.games > 0).then(|| summary_json(&all)),
        "champions": champions.iter().map(|(id, games, wins)| json!({
            "champion": champion(data, *id), "games": games, "wins": wins,
        })).collect::<Vec<_>>(),
        "games": history.games.iter().map(|g| game_json(g, data)).collect::<Vec<_>>(),
    })
}

/// `lol.player` 给她的几行字：段位、近况、常玩、最近 5 局（页面上有全部）。
pub fn player_text(history: &PlayerHistory, data: Option<&GameData>) -> String {
    let name = if history.riot_id.is_empty() {
        "这个玩家".to_string()
    } else {
        format!("「{}」", history.riot_id)
    };
    let mut parts = vec![name];
    if let Some(rank) = &history.rank {
        parts.push(format!("单双排 {rank}"));
    }
    if history.hidden() {
        parts.push("隐藏战绩".into());
    }
    if history.games.is_empty() {
        parts.push(match &history.error {
            Some(e) => format!("战绩查不到（{e}）"),
            None => "最近没有对局".into(),
        });
        return parts.join(" · ");
    }
    let s = scout::summarize(&history.games, 0, 0);
    parts.push(format!(
        "近 {} 局 {} 胜 {} 负",
        s.games,
        s.wins,
        s.games - s.wins
    ));
    parts.push(format!("KDA {:.1}，场均死 {:.1}", s.kda, s.deaths));
    if s.streak >= 3 {
        parts.push(format!("{} 连胜", s.streak));
    } else if s.streak <= -3 {
        parts.push(format!("{} 连败", -s.streak));
    }
    let mut out = vec![parts.join(" · ")];
    let short = |id: i64| {
        data.and_then(|d| d.champion(id))
            .map(|c| c.short().to_string())
            .unwrap_or_else(|| format!("英雄 {id}"))
    };
    let recent: Vec<String> = history
        .games
        .iter()
        .take(5)
        .map(|g| {
            format!(
                "{} {} {}/{}/{}（{}）",
                short(g.champion_id),
                if g.win { "胜" } else { "负" },
                g.kills,
                g.deaths,
                g.assists,
                queue(data, g.queue_id, &g.mode)
            )
        })
        .collect();
    out.push(format!("最近 5 局：{}", recent.join("；")));
    out.join("\n")
}

fn i(v: &Value, key: &str) -> i64 {
    v.get(key).and_then(Value::as_i64).unwrap_or(0)
}

fn s(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

/// 单局详情（`/lol-match-history/v1/games/{gameId}`，`lol.game`）：两边 10 个人。不知道我是谁（见 [`game_page_for`]）。
pub fn game_page(detail: &Value, data: Option<&GameData>) -> Option<Value> {
    game_page_for(detail, data, None)
}

/// 同 [`game_page`]，`me` 是我的 puuid：我那一行 `me`、我在的那一队 `ally`（页面写“我方 / 对面”，不然是“胜方 / 负方”）。
pub fn game_page_for(detail: &Value, data: Option<&GameData>, me: Option<&str>) -> Option<Value> {
    let participants = detail.get("participants")?.as_array()?;
    let identities = detail
        .get("participantIdentities")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let duration = i(detail, "gameDuration");
    let minutes = (duration as f64 / 60.0).max(1.0);
    let mut teams: Vec<(i64, bool, Vec<Value>)> = Vec::new();
    for p in participants {
        let st = p.get("stats").cloned().unwrap_or(Value::Null);
        let id = i(p, "participantId");
        let player = identities
            .iter()
            .find(|x| i(x, "participantId") == id)
            .and_then(|x| x.get("player"))
            .cloned()
            .unwrap_or(Value::Null);
        let name = s(&player, "gameName");
        let tag = s(&player, "tagLine");
        let riot_id = match (name.is_empty(), tag.is_empty()) {
            (true, _) => s(&player, "summonerName"),
            (false, true) => name.clone(),
            (false, false) => format!("{name}#{tag}"),
        };
        let slots: Vec<i64> = (0..=6).map(|n| i(&st, &format!("item{n}"))).collect();
        let augment_ids: Vec<i64> = (1..=6)
            .map(|n| i(&st, &format!("playerAugment{n}")))
            .filter(|a| *a > 0)
            .collect();
        let icon_id = player.get("profileIcon").and_then(Value::as_i64);
        let damage = i(&st, "totalDamageDealtToChampions");
        let team_id = i(p, "teamId");
        let win = st.get("win").and_then(Value::as_bool).unwrap_or(false);
        let puuid = s(&player, "puuid");
        let is_me = me.is_some_and(|me| !me.is_empty() && me == puuid);
        let row = json!({
            "me": is_me,
            "puuid": puuid,
            "riotId": riot_id,
            "champion": champion(data, i(p, "championId")),
            "level": i(&st, "champLevel"),
            "kills": i(&st, "kills"),
            "deaths": i(&st, "deaths"),
            "assists": i(&st, "assists"),
            "damage": damage,
            "dpm": (damage as f64 / minutes).round(),
            "gold": i(&st, "goldEarned"),
            "cs": i(&st, "totalMinionsKilled") + i(&st, "neutralMinionsKilled"),
            "items": items(data, &slots),
            "augments": augments(data, &augment_ids),
            "profileIconId": icon_id,
            "profileIcon": icon_id.map(assets::profile_icon).unwrap_or_default(),
            "spells": spells(data, &[i(p, "spell1Id"), i(p, "spell2Id")]),
            "perks": perks(data, i(&st, "perk0"), i(&st, "perkPrimaryStyle"), i(&st, "perkSubStyle")),
        });
        match teams.iter_mut().find(|(t, _, _)| *t == team_id) {
            Some(team) => team.2.push(row),
            None => teams.push((team_id, win, vec![row])),
        }
    }
    teams.sort_by_key(|(t, _, _)| *t);
    let ally = |players: &[Value]| players.iter().any(|p| p["me"] == json!(true));
    Some(json!({
        "gameId": i(detail, "gameId"),
        "queueId": i(detail, "queueId"),
        "queue": queue(data, i(detail, "queueId"), &s(detail, "gameMode")),
        "createdMs": i(detail, "gameCreation"),
        "durationS": duration,
        "teams": teams.into_iter().map(|(id, win, players)| json!({
            "teamId": id, "win": win, "ally": ally(&players), "players": players,
        })).collect::<Vec<_>>(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::history::GameRecord;

    fn game(id: i64, win: bool, champion: i64) -> GameRecord {
        GameRecord {
            game_id: id,
            queue_id: 2400,
            mode: "KIWI".into(),
            created_ms: 1_000 - id,
            duration_s: 1200,
            champion_id: champion,
            win,
            kills: 5,
            deaths: 3,
            assists: 10,
            damage: 30_000,
            gold: 12_000,
            cs: 40,
            augments: vec![1, 2],
            ..GameRecord::default()
        }
    }

    fn history(wins: &[bool]) -> PlayerHistory {
        PlayerHistory {
            riot_id: "某人#123".into(),
            privacy: "PUBLIC".into(),
            games: wins
                .iter()
                .enumerate()
                .map(|(i, w)| game(i as i64 + 1, *w, if i < 3 { 39 } else { 157 }))
                .collect(),
            rank: Some("黄金 2".into()),
            error: None,
            ..PlayerHistory::default()
        }
    }

    #[test]
    fn player_page_has_summary_champions_and_games() {
        let data = crate::client::gamedata::tests::sample();
        let h = history(&[false, false, false, true, true]);
        let v = player_page("p1", &h, 2400, Some(&data));
        assert_eq!(v["riotId"], "某人#123");
        assert_eq!(v["summary"]["games"], 5);
        assert_eq!(v["summary"]["streak"], -3);
        assert_eq!(v["summary"]["modeGames"], 5);
        assert_eq!(v["champions"][0]["champion"]["short"], "艾瑞莉娅");
        assert_eq!(v["champions"][0]["games"], 3);
        assert_eq!(v["games"].as_array().unwrap().len(), 5);
        assert_eq!(v["games"][0]["dpm"], 1500.0);
        assert_eq!(v["games"][0]["kills"], 5);
    }

    #[test]
    fn match_page_splits_teams_and_states_facts_only() {
        let data = crate::client::gamedata::tests::sample();
        let mut players = Vec::new();
        let mut puuids = Vec::new();
        for n in 0..10 {
            let ally = n < 5;
            // 对面第一个连败、其余赢得多
            let wins: Vec<bool> = if n == 5 {
                vec![false; 10]
            } else {
                (0..10).map(|g| g % 3 != 0).collect()
            };
            players.push(ScoutPlayer {
                champion: "刀锋舞者 艾瑞莉娅".into(),
                short: "艾瑞莉娅".into(),
                champion_id: 39,
                name: format!("p{n}"),
                ally,
                me: n == 0,
                history: (n != 9).then(|| history(&wins)),
                ..ScoutPlayer::default()
            });
            puuids.push(format!("puuid-{n}"));
        }
        let v = match_page(
            &players,
            &puuids,
            77,
            2400,
            "海克斯大乱斗",
            true,
            Some(&data),
        );
        assert_eq!(v["allies"].as_array().unwrap().len(), 5);
        assert_eq!(v["enemies"].as_array().unwrap().len(), 5);
        assert_eq!(v["waiting"], 1);
        assert_eq!(v["allies"][0]["me"], true);
        assert_eq!(v["enemies"][0]["puuid"], "puuid-5");
        // ADR 043 D2：没有“突破口”（weak）了；连败照样是标签（事实）
        assert!(v["enemies"][0].get("weak").is_none());
        assert_eq!(v["enemies"][0]["identity"], "known");
        assert_eq!(v["enemies"][4]["pending"], true);
        assert_eq!(v["enemies"][0]["recent"].as_array().unwrap().len(), 10);
        // 标签带种类，字和给她的一样；没带阶段时阶段是 null、都算锁了
        assert_eq!(
            v["enemies"][0]["tags"],
            json!([{"kind": "loss_streak", "text": "10 连败"}])
        );
        assert_eq!(v["phase"], Value::Null);
        assert_eq!(v["result"], Value::Null);
        assert_eq!(v["bench"], json!([]));
        assert_eq!(v["allies"][1]["pick"], "locked");

        // 选人：预选、还没选、备选席、名字被隐藏
        let mut picks = vec![Pick::Locked; 10];
        picks[1] = Pick::Intent;
        picks[2] = Pick::None;
        let stage = MatchStage {
            phase: Some(MatchPhase::ChampSelect),
            since_ms: Some(1_700_000_000_000),
            names_hidden: true,
            bench: vec![157],
            picks,
            ..MatchStage::default()
        };
        let v = match_page_in(
            &players,
            &puuids,
            77,
            420,
            "单双排",
            false,
            Some(&data),
            &stage,
        );
        assert_eq!(v["phase"], "champ_select");
        assert_eq!(v["phaseSince"], 1_700_000_000_000_i64);
        assert_eq!(v["namesHidden"], true);
        assert_eq!(v["bench"][0]["short"], "亚索");
        assert_eq!(
            v["bench"][0]["icon"],
            "/lol-game-data/assets/v1/champion-icons/157.png"
        );
        assert_eq!(
            [
                &v["allies"][0]["pick"],
                &v["allies"][1]["pick"],
                &v["allies"][2]["pick"]
            ],
            [&json!("locked"), &json!("intent"), &json!("none")]
        );
        assert_eq!(v["resultPending"], false);
        // 没有 puuid 的人（选人时的对面、看不到名字的队友）不查，也不算“还在查”
        let mut anonymous = players.clone();
        anonymous[9].history = None;
        // 隐藏了身份（ADR 043 D9）：照样一行，只有英雄，不算“还在查”
        anonymous[8].history = None;
        anonymous[8].identity = crate::client::text::Identity::Hidden;
        let mut ids = puuids.clone();
        ids[9] = String::new();
        ids[8] = String::new();
        let v = match_page_in(
            &anonymous,
            &ids,
            77,
            420,
            "单双排",
            false,
            Some(&data),
            &stage,
        );
        assert_eq!(v["enemies"][4]["pending"], false);
        assert_eq!(v["enemies"][3]["pending"], false);
        assert_eq!(v["enemies"][3]["identity"], "hidden");
        assert_eq!(v["enemies"].as_array().unwrap().len(), 5);
        assert_eq!(v["waiting"], 0);

        // 结算：结果
        let stage = MatchStage {
            phase: Some(MatchPhase::PostMatch),
            result: Some(MatchResult {
                win: true,
                duration_s: 1112,
                game_id: 77,
            }),
            ..MatchStage::default()
        };
        let v = match_page_in(
            &players,
            &puuids,
            77,
            2400,
            "海克斯大乱斗",
            true,
            Some(&data),
            &stage,
        );
        assert_eq!(v["phase"], "post_match");
        assert_eq!(
            v["result"],
            json!({"win": true, "durationS": 1112, "gameId": 77})
        );
    }

    /// 标签的种类和给她的那句话是同一份（`scout::tags` 就是 `tagged` 去掉种类）。
    #[test]
    fn tags_carry_kinds_and_the_same_words() {
        let mut s = Summary {
            games: 20,
            wins: 12,
            streak: 4,
            champ_games: 6,
            champ_wins: 4,
            ..Summary::default()
        };
        let kinds = |s: &Summary, random: bool| -> Vec<String> {
            tags_json(s, random)
                .iter()
                .map(|t| t["kind"].as_str().unwrap().to_string())
                .collect()
        };
        assert_eq!(kinds(&s, false), ["win_streak", "main"]);
        assert_eq!(kinds(&s, true), ["win_streak"]);
        s.streak = -3;
        s.champ_games = 0;
        assert_eq!(kinds(&s, false), ["loss_streak", "first_time"]);
        let texts: Vec<Value> = tags_json(&s, false)
            .iter()
            .map(|t| t["text"].clone())
            .collect();
        assert_eq!(
            texts,
            scout::tags(&s, false)
                .iter()
                .map(|t| json!(t))
                .collect::<Vec<_>>()
        );
    }

    /// 给琴音的文字：同一份录到的样子（带页面才用的字段）→ 一字不差的那几行（2026-11-25 加页面字段前录的；
    /// 2026-11-26 照 ADR 043 D2 去掉了最后那行“突破口”，别的一字没改）。
    fn text_for_her() -> String {
        use crate::client::history::{self as h, tests as ht};
        let data = crate::client::gamedata::tests::sample();
        // 一局，带召唤师技能、装备、符文、海克斯、等级（页面用，给她的字里不出现）
        let rich = |ago: i64, champion: i64, win: bool, kda: (i64, i64, i64)| {
            let mut g = ht::game(ago, 2400, champion, win, kda, 24_000);
            let p = &mut g["participants"][0];
            p["spell1Id"] = json!(4);
            p["spell2Id"] = json!(32);
            let st = &mut p["stats"];
            st["item0"] = json!(3158);
            st["item6"] = json!(3340);
            st["perk0"] = json!(8369);
            st["perkPrimaryStyle"] = json!(8300);
            st["perkSubStyle"] = json!(8000);
            st["champLevel"] = json!(15);
            g
        };
        let ranked = json!({"queueMap": {
            "RANKED_SOLO_5x5": {"queueType": "RANKED_SOLO_5x5", "tier": "GOLD", "division": "II",
                                "leaguePoints": 45, "wins": 10, "losses": 8, "highestTier": "PLATINUM"},
            "RANKED_FLEX_SR": {"queueType": "RANKED_FLEX_SR", "tier": "", "division": "NA"}
        }});
        let person = |short: &str, id: i64, ally: bool, me: bool, games: Vec<Value>| {
            let list = ht::list(games);
            ScoutPlayer {
                champion: format!("称号 {short}"),
                alias: short.into(),
                short: short.into(),
                champion_id: id,
                name: format!("{short}的主人"),
                ally,
                me,
                party: ally.then_some(1),
                history: Some(PlayerHistory {
                    riot_id: format!("{short}的主人#1"),
                    privacy: "PUBLIC".into(),
                    games: h::parse_match_list(&list),
                    rank: h::parse_solo_rank(&ranked),
                    error: None,
                    // 页面才用的（给她的字里不出现）
                    profile_icon_id: Some(7335),
                    summoner_level: 321,
                    solo: h::parse_ranked(&ranked, h::SOLO),
                    flex: h::parse_ranked(&ranked, h::FLEX),
                }),
                profile_icon_id: Some(29),
                spells: [4, 32],
                position: "MIDDLE".into(),
                identity: crate::client::text::Identity::Known,
            }
        };
        let good: Vec<Value> = (0..10)
            .map(|i| rich(i, 39, i % 3 != 0, (8, 3, 10)))
            .collect();
        let bad: Vec<Value> = (0..10).map(|i| rich(i, 157, i >= 6, (2, 9, 4))).collect();
        let mut players = vec![
            person("甲", 39, true, true, good.clone()),
            person("乙", 157, true, false, good.clone()),
            person("丙", 103, false, false, good),
            person("丁", 157, false, false, bad),
        ];
        players[3].history.as_mut().unwrap().privacy = "PRIVATE".into();
        let mut out = vec![
            crate::client::text::render_scout(&players, 2400, "海克斯大乱斗", true, None),
            crate::client::text::render_scout(&players, 420, "单双排", false, Some("丁")),
        ];
        for p in &players {
            out.push(player_text(p.history.as_ref().unwrap(), Some(&data)));
        }
        out.join("\n---\n")
    }

    #[test]
    fn text_for_her_is_unchanged() {
        const EXPECTED: &[&str] = &[
            "每人最近 20 局（全部模式；“海克斯大乱斗”是和这局同一个模式的；英雄是随机的，不看玩没玩过这个英雄）：",
            "我方：",
            "- 称号 甲（你） · 「甲的主人」 · 单双排 黄金 2 · 近 10 局 6 胜 4 负 · 海克斯大乱斗 10 局 6 胜，分均伤害 1600 · KDA 6.0，场均死 3.0 · 这个英雄 10 局 6 胜",
            "- 称号 乙 · 「乙的主人」 · 单双排 黄金 2 · 近 10 局 6 胜 4 负 · 海克斯大乱斗 10 局 6 胜，分均伤害 1600 · KDA 6.0，场均死 3.0",
            "对面：",
            "- 称号 丙 · 「丙的主人」 · 单双排 黄金 2 · 近 10 局 6 胜 4 负 · 海克斯大乱斗 10 局 6 胜，分均伤害 1600 · KDA 6.0，场均死 3.0",
            "- 称号 丁 · 「丁的主人」 · 单双排 黄金 2 · 隐藏战绩 · 近 10 局 4 胜 6 负 · 海克斯大乱斗 10 局 4 胜，分均伤害 1600 · KDA 0.7，场均死 9.0 · 这个英雄 10 局 4 胜 · 6 连败",
            "开黑：我方 甲 + 乙",
            "---",
            "每人最近 20 局（全部模式；“单双排”是和这局同一个模式的）：",
            "对面：",
            "- 称号 丁 · 「丁的主人」 · 单双排 黄金 2 · 隐藏战绩 · 近 10 局 4 胜 6 负 · KDA 0.7，场均死 9.0 · 这个英雄 10 局 4 胜 · 6 连败 · 常玩这个英雄（10 局 4 胜）",
            "---",
            "「甲的主人#1」 · 单双排 黄金 2 · 近 10 局 6 胜 4 负 · KDA 6.0，场均死 3.0",
            "最近 5 局：艾瑞莉娅 负 8/3/10（海克斯大乱斗）；艾瑞莉娅 胜 8/3/10（海克斯大乱斗）；艾瑞莉娅 胜 8/3/10（海克斯大乱斗）；艾瑞莉娅 负 8/3/10（海克斯大乱斗）；艾瑞莉娅 胜 8/3/10（海克斯大乱斗）",
            "---",
            "「乙的主人#1」 · 单双排 黄金 2 · 近 10 局 6 胜 4 负 · KDA 6.0，场均死 3.0",
            "最近 5 局：艾瑞莉娅 负 8/3/10（海克斯大乱斗）；艾瑞莉娅 胜 8/3/10（海克斯大乱斗）；艾瑞莉娅 胜 8/3/10（海克斯大乱斗）；艾瑞莉娅 负 8/3/10（海克斯大乱斗）；艾瑞莉娅 胜 8/3/10（海克斯大乱斗）",
            "---",
            "「丙的主人#1」 · 单双排 黄金 2 · 近 10 局 6 胜 4 负 · KDA 6.0，场均死 3.0",
            "最近 5 局：艾瑞莉娅 负 8/3/10（海克斯大乱斗）；艾瑞莉娅 胜 8/3/10（海克斯大乱斗）；艾瑞莉娅 胜 8/3/10（海克斯大乱斗）；艾瑞莉娅 负 8/3/10（海克斯大乱斗）；艾瑞莉娅 胜 8/3/10（海克斯大乱斗）",
            "---",
            "「丁的主人#1」 · 单双排 黄金 2 · 隐藏战绩 · 近 10 局 4 胜 6 负 · KDA 0.7，场均死 9.0 · 6 连败",
            "最近 5 局：亚索 负 2/9/4（海克斯大乱斗）；亚索 负 2/9/4（海克斯大乱斗）；亚索 负 2/9/4（海克斯大乱斗）；亚索 负 2/9/4（海克斯大乱斗）；亚索 负 2/9/4（海克斯大乱斗）",
        ];
        let out = text_for_her();
        assert_eq!(out, EXPECTED.join("\n"), "\n{out}");
    }

    /// G0.1：页面要的图都有 id 和客户端路径（素材层认得的那种），段位是结构化的。
    #[test]
    fn page_fields_carry_ids_and_client_paths() {
        use crate::client::history::{self as h, tests as ht};
        let mut data = crate::client::gamedata::tests::sample();
        data.add_perks(
            &json!([{"id": 8369, "name": "先攻", "iconPath": "/lol-game-data/assets/v1/perk-images/Styles/Inspiration/FirstStrike/FirstStrike.png"}]),
            &json!({"styles": [{"id": 8300, "name": "启迪", "iconPath": "/lol-game-data/assets/v1/perk-images/Styles/7203_Whimsy.png"}]}),
        );
        let mut g = ht::game(0, 2400, 39, true, (6, 4, 12), 24_000);
        g["participants"][0]["spell1Id"] = json!(4);
        g["participants"][0]["spell2Id"] = json!(32);
        let st = &mut g["participants"][0]["stats"];
        st["item0"] = json!(3158);
        st["item6"] = json!(3340);
        st["perk0"] = json!(8369);
        st["perkPrimaryStyle"] = json!(8300);
        st["perkSubStyle"] = json!(8000);
        st["champLevel"] = json!(15);
        let ranked = json!({"queueMap": {"RANKED_SOLO_5x5": {"tier": "GOLD", "division": "II", "leaguePoints": 45, "wins": 10, "losses": 8}}});
        let history = PlayerHistory {
            riot_id: "某人#1".into(),
            privacy: "PUBLIC".into(),
            games: h::parse_match_list(&ht::list(vec![g])),
            profile_icon_id: Some(7335),
            summoner_level: 321,
            solo: h::parse_ranked(&ranked, h::SOLO),
            ..PlayerHistory::default()
        };

        // 战绩页
        let v = player_page("p", &history, 2400, Some(&data));
        assert_eq!(v["profileIconId"], 7335);
        assert_eq!(
            v["profileIcon"],
            "/lol-game-data/assets/v1/profile-icons/7335.jpg"
        );
        assert_eq!(v["summonerLevel"], 321);
        assert_eq!(
            v["ranked"]["solo"],
            json!({"queue": "RANKED_SOLO_5x5", "tier": "GOLD", "division": "II", "lp": 45, "wins": 10,
                   "losses": 8, "highestTier": "", "label": "黄金 2",
                   "crest": "/fe/lol-static-assets/images/ranked-mini-crests/gold.svg"})
        );
        assert_eq!(v["ranked"]["flex"], Value::Null);
        let game = &v["games"][0];
        assert_eq!(
            game["champion"]["icon"],
            "/lol-game-data/assets/v1/champion-icons/39.png"
        );
        assert_eq!(game["level"], 15);
        assert_eq!(game["spells"][0]["name"], "闪现");
        assert_eq!(game["spells"][1]["id"], 32);
        assert_eq!(
            game["spells"][1]["icon"], "",
            "资料里没有图标路径：空字符串"
        );
        assert_eq!(game["items"].as_array().unwrap().len(), 2);
        assert_eq!(game["items"][0]["slot"], 0);
        assert!(game["items"][0]["icon"]
            .as_str()
            .unwrap()
            .ends_with("IonianBootsofLucidity.png"));
        assert_eq!(
            (
                game["items"][1]["id"].clone(),
                game["items"][1]["slot"].clone()
            ),
            (json!(3340), json!(6))
        );
        assert_eq!(game["augments"][0]["rarity"], "kSilver");
        assert!(game["augments"][0]["icon"]
            .as_str()
            .unwrap()
            .ends_with("ADAPt_small.png"));
        assert_eq!(game["perks"]["keystone"]["name"], "先攻");
        assert!(game["perks"]["primaryStyle"]["icon"]
            .as_str()
            .unwrap()
            .ends_with("7203_Whimsy.png"));
        assert_eq!(game["perks"]["subStyle"]["id"], 8000);
        assert_eq!(game["perks"]["subStyle"]["name"], "");

        // 这局
        let players = vec![ScoutPlayer {
            champion_id: 103,
            me: true,
            ally: true,
            history: Some(history.clone()),
            profile_icon_id: Some(29),
            spells: [4, 0],
            position: "MIDDLE".into(),
            ..ScoutPlayer::default()
        }];
        let m = match_page(
            &players,
            &["p".into()],
            1,
            420,
            "单双排",
            false,
            Some(&data),
        );
        let me = &m["allies"][0];
        assert_eq!(me["profileIconId"], 29, "这局的头像以 session 为准");
        assert_eq!(
            me["profileIcon"],
            "/lol-game-data/assets/v1/profile-icons/29.jpg"
        );
        assert_eq!(me["summonerLevel"], 321);
        assert_eq!(me["spells"].as_array().unwrap().len(), 1);
        assert_eq!(me["position"], "MIDDLE");
        assert_eq!(me["ranked"]["solo"]["label"], "黄金 2");
        assert_eq!(
            me["champion"]["icon"],
            "/lol-game-data/assets/v1/champion-icons/103.png"
        );
        assert_eq!(
            me["recent"][0]["championIcon"],
            "/lol-game-data/assets/v1/champion-icons/39.png"
        );
        // 还在查的人：没有段位、等级，头像用 session 的
        let waiting = vec![ScoutPlayer {
            profile_icon_id: Some(1),
            ..ScoutPlayer::default()
        }];
        let m = match_page(&waiting, &["q".into()], 1, 420, "单双排", false, None);
        let w = &m["enemies"][0];
        assert_eq!(w["ranked"], json!({"solo": null, "flex": null}));
        assert_eq!(w["summonerLevel"], Value::Null);
        assert_eq!(
            w["profileIcon"],
            "/lol-game-data/assets/v1/profile-icons/1.jpg"
        );
        assert_eq!(w["champion"]["icon"], "");

        // 每个路径都过素材层的检查
        let mut paths = Vec::new();
        fn collect(v: &Value, out: &mut Vec<String>) {
            match v {
                Value::Object(o) => {
                    for (k, x) in o {
                        if matches!(
                            k.as_str(),
                            "icon" | "profileIcon" | "crest" | "championIcon"
                        ) {
                            if let Some(p) = x.as_str().filter(|p| !p.is_empty()) {
                                out.push(p.to_string());
                            }
                        }
                        collect(x, out);
                    }
                }
                Value::Array(a) => a.iter().for_each(|x| collect(x, out)),
                _ => {}
            }
        }
        collect(&v, &mut paths);
        collect(
            &match_page(
                &players,
                &["p".into()],
                1,
                420,
                "单双排",
                false,
                Some(&data),
            ),
            &mut paths,
        );
        assert!(paths.len() >= 10, "{paths:?}");
        for p in &paths {
            assert!(crate::client::assets::normalize(p).is_ok(), "{p}");
        }
    }

    #[test]
    fn game_page_reads_the_recorded_detail_shape() {
        let detail = json!({
            "gameId": 9, "queueId": 2400, "gameMode": "KIWI", "gameDuration": 600, "gameCreation": 5,
            "participantIdentities": [
                {"participantId": 1, "player": {"gameName": "甲", "tagLine": "1", "puuid": "a", "profileIcon": 29}},
                {"participantId": 2, "player": {"gameName": "乙", "tagLine": "", "puuid": "b"}}
            ],
            "participants": [
                {"participantId": 1, "teamId": 100, "championId": 39, "spell1Id": 4, "spell2Id": 0,
                 "stats": {"win": true, "kills": 3, "deaths": 1, "assists": 2, "totalDamageDealtToChampions": 6000,
                           "item0": 3031, "item1": 0, "playerAugment1": 7, "champLevel": 12}},
                {"participantId": 2, "teamId": 200, "championId": 157, "stats": {"win": false}}
            ]
        });
        let v = game_page(&detail, None).unwrap();
        assert_eq!(v["teams"][0]["teamId"], 100);
        assert_eq!(v["teams"][0]["win"], true);
        assert_eq!(v["teams"][0]["players"][0]["riotId"], "甲#1");
        assert_eq!(v["teams"][0]["players"][0]["dpm"], 600.0);
        assert_eq!(v["teams"][0]["players"][0]["items"][0]["id"], 3031);
        assert_eq!(v["teams"][0]["players"][0]["items"][0]["slot"], 0);
        assert_eq!(v["teams"][0]["players"][0]["augments"][0]["id"], 7);
        assert_eq!(v["teams"][0]["players"][0]["spells"][0]["id"], 4);
        assert_eq!(v["teams"][0]["players"][0]["profileIconId"], 29);
        assert_eq!(
            v["teams"][0]["players"][0]["profileIcon"],
            "/lol-game-data/assets/v1/profile-icons/29.jpg"
        );
        assert_eq!(v["teams"][0]["players"][0]["perks"], Value::Null);
        assert_eq!(v["teams"][1]["players"][0]["profileIconId"], Value::Null);
        assert_eq!(v["teams"][1]["players"][0]["spells"], json!([]));
        assert_eq!(v["teams"][1]["players"][0]["riotId"], "乙");
        assert!(game_page(&json!({}), None).is_none());
        // 不知道我是谁：都不是我、两队都不是我方
        assert_eq!(v["teams"][0]["ally"], false);
        assert_eq!(v["teams"][0]["players"][0]["me"], false);
        // 知道：我那一行、我那一队
        let v = game_page_for(&detail, None, Some("b")).unwrap();
        assert_eq!(v["teams"][1]["ally"], true);
        assert_eq!(v["teams"][1]["players"][0]["me"], true);
        assert_eq!(v["teams"][0]["ally"], false);
    }

    /// 客户端的海克斯资料（`cherry-augments.json` 的字段）：录的那一把里对面用到的几个，名字、稀有度、图标照录的。
    fn augment_data() -> GameData {
        let rows = [
            (
                1018,
                "巨像的勇气",
                "kPrismatic",
                "Cherry/Augments/Icons/CourageOfTheColossus_small.png",
            ),
            (
                1353,
                "坦克引擎",
                "kGold",
                "Cherry/Augments/Icons/Tank_Engine_Small.png",
            ),
            (
                1136,
                "扇巴掌",
                "kSilver",
                "Cherry/Augments/Icons/SlapAround_small.png",
            ),
            (
                1301,
                "神圣干预",
                "kGold",
                "Cherry/Augments/Icons/SlowActingPainkillers_small.png",
            ),
            (
                3078,
                "鼓声暴起 低音轰鸣",
                "kGold",
                "Kiwi/Augments/Icons/TheBoomAndTheBass_small.png",
            ),
        ];
        let augments: Vec<Value> = rows
            .iter()
            .map(|(id, name, rarity, icon)| {
                json!({"id": id, "nameTRA": name, "rarity": rarity,
                       "augmentSmallIconPath": format!("/lol-game-data/assets/ASSETS/UX/{icon}")})
            })
            .collect();
        GameData::from_json(
            &Value::Null,
            &Value::Null,
            &Value::Null,
            &Value::Array(augments),
            &Value::Null,
        )
    }

    /// 海斗、斗魂竞技场的单局详情里每个人都带海克斯（ADR 043 B 轮第 3 条：这局总结上列出对面每个人的）。字段和 id 照
    /// `target/lol-probe/20261123-210745` 录的那一把海斗（`stats.playerAugment1`–`6`，没选的格子是 0；名字换成编的）：
    /// 按选的顺序、空格不列，名字 / 稀有度 / 图标来自客户端的资料；资料里没有的照样列出 id。峡谷的详情没有这几个字段：空的。
    #[test]
    fn mayhem_details_carry_every_players_augments_in_pick_order() {
        let stats = |win: bool, augments: [i64; 6]| {
            let mut st = json!({"win": win, "kills": 3, "deaths": 4, "assists": 20});
            for (n, a) in augments.iter().enumerate() {
                st[format!("playerAugment{}", n + 1)] = json!(a);
            }
            st["playerSubteamId"] = json!(0);
            st
        };
        let detail = json!({
            "gameId": 7, "queueId": 2400, "gameMode": "KIWI", "gameDuration": 1014, "gameCreation": 5,
            "participantIdentities": [
                {"participantId": 1, "player": {"gameName": "我", "tagLine": "1", "puuid": "me"}},
                {"participantId": 6, "player": {"gameName": "对手甲", "tagLine": "2", "puuid": "e1"}},
                {"participantId": 7, "player": {"gameName": "对手乙", "tagLine": "3", "puuid": "e2"}}
            ],
            "participants": [
                {"participantId": 1, "teamId": 100, "championId": 68, "stats": stats(true, [1156, 2132, 1318, 1180, 0, 0])},
                {"participantId": 6, "teamId": 200, "championId": 412, "stats": stats(false, [1018, 1353, 1136, 1301, 0, 0])},
                {"participantId": 7, "teamId": 200, "championId": 432, "stats": stats(false, [1058, 3086, 1087, 3078, 0, 0])}
            ]
        });
        let data = augment_data();
        let v = game_page_for(&detail, Some(&data), Some("me")).unwrap();
        let enemies = &v["teams"][1];
        assert_eq!(enemies["ally"], false);
        let first = enemies["players"][0]["augments"].as_array().unwrap();
        assert_eq!(
            first
                .iter()
                .map(|a| a["id"].as_i64().unwrap())
                .collect::<Vec<_>>(),
            [1018, 1353, 1136, 1301],
            "按选的顺序，0 不列"
        );
        assert_eq!(first[0]["name"], "巨像的勇气");
        assert_eq!(first[0]["rarity"], "kPrismatic");
        assert_eq!(
            first[0]["icon"],
            "/lol-game-data/assets/ASSETS/UX/Cherry/Augments/Icons/CourageOfTheColossus_small.png"
        );
        assert!(crate::client::assets::normalize(first[0]["icon"].as_str().unwrap()).is_ok());
        let second = enemies["players"][1]["augments"].as_array().unwrap();
        assert_eq!(second.len(), 4);
        assert_eq!(second[3]["name"], "鼓声暴起 低音轰鸣");
        // 资料里没有的（这份测试资料只放了几个）：照样列出 id，名字空，页面显示 id
        assert_eq!(second[0]["id"], 1058);
        assert_eq!(second[0]["name"], "");
        // 我方（我）也有
        assert_eq!(
            v["teams"][0]["players"][0]["augments"]
                .as_array()
                .unwrap()
                .len(),
            4
        );
        // 峡谷：没有海克斯
        let rift = json!({"gameId": 8, "queueId": 420, "gameMode": "CLASSIC", "gameDuration": 1800,
            "participants": [{"participantId": 1, "teamId": 100, "championId": 157, "stats": {"win": true, "perk0": 8010}}]});
        let v = game_page_for(&rift, Some(&data), None).unwrap();
        assert_eq!(v["teams"][0]["players"][0]["augments"], json!([]));
    }

    /// 本机有录的那一把（`target/lol-probe/20261123-210745`，开发机上才有，CI 跳过）：两边 10 个人每人 4 个海克斯，
    /// 名字都从录的客户端资料里查得到。
    #[test]
    fn the_recorded_mayhem_detail_has_four_named_augments_per_player() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/lol-probe");
        let read = |p: &str| {
            std::fs::read_to_string(root.join(p))
                .ok()
                .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        };
        let (Some(detail), Some(augments)) = (
            read("20261123-210745/lcu-game-detail.json"),
            read("gamedata/cherry-augments.json"),
        ) else {
            return;
        };
        let data = GameData::from_json(
            &Value::Null,
            &Value::Null,
            &Value::Null,
            &augments,
            &Value::Null,
        );
        let v = game_page(&detail, Some(&data)).unwrap();
        let players: Vec<&Value> = v["teams"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|t| t["players"].as_array().unwrap())
            .collect();
        assert_eq!(players.len(), 10);
        for p in players {
            let list = p["augments"].as_array().unwrap();
            assert_eq!(list.len(), 4, "{}", p["champion"]["id"]);
            assert!(
                list.iter().all(|a| !a["name"].as_str().unwrap().is_empty()),
                "{list:?}"
            );
        }
    }
}
